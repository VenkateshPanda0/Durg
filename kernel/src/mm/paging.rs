//! Page table access: translate addresses and map pages.
//!
//! This is the kernel half of the paging code. The pure logic (entry formats,
//! splitting addresses, the table walk) lives in the host-tested `durg-paging`
//! crate. This module supplies what that crate cannot: reading and writing the
//! actual tables in memory.
//!
//! **Scope, deliberately small.** Until M5 the kernel runs on the page tables
//! the bootloader built. They sit in bootloader-reclaimable memory, their
//! layout is unspecified beyond "the required regions are mapped", and they are
//! reachable through the higher-half direct map (HHDM). This module can:
//!
//! - translate a virtual address by walking those tables ([`translate`]);
//! - add a 4 KiB mapping ([`map_page`]), creating any missing intermediate
//!   tables from a small pool reserved inside the kernel image, because there
//!   is no physical memory manager yet (M4).
//!
//! It exists so the kernel can reach the local APIC's registers, which are not
//! in the bootloader's mappings. M5 replaces the pool with real frame
//! allocation and takes over the page tables entirely.

use core::mem::size_of;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};

use durg_paging::{
    flags, split, walk, Entry, Mapping, PagingError, WalkError, ADDRESS_MASK, ENTRY_COUNT,
    PAGE_SIZE,
};

use crate::arch::x86_64::{self, BootCell};
use crate::limine;

/// One page table (a page of 512 entries) at any level.
#[repr(C, align(4096))]
struct PageTable([Entry; ENTRY_COUNT]);

const _: () = assert!(size_of::<PageTable>() == PAGE_SIZE as usize);

const EMPTY_TABLE: PageTable = PageTable([Entry::EMPTY; ENTRY_COUNT]);

/// Page-table pages reserved in the kernel image for tables added before a
/// physical memory manager exists. Mapping one MMIO page far from existing
/// mappings can need up to three (PDPT, PD, PT).
const POOL_TABLES: usize = 4;

static BOOT_TABLE_POOL: BootCell<[PageTable; POOL_TABLES]> =
    BootCell::new([EMPTY_TABLE; POOL_TABLES]);
static POOL_USED: AtomicUsize = AtomicUsize::new(0);

/// Why a mapping could not be created.
#[derive(Debug, Clone, Copy)]
pub enum MapError {
    /// The virtual or physical address or the flags were invalid.
    Invalid(PagingError),
    /// A table on the path is not in memory the kernel can reach.
    UnreadableTable { level: u8 },
    /// The path runs through a huge page, so there is no table to extend.
    HugePageInTheWay { level: u8 },
    /// The page is already mapped.
    AlreadyMapped,
    /// All reserved page-table pages are used.
    PoolExhausted,
    /// The address translation of a pool table failed.
    PoolUntranslatable(WalkError),
}

impl core::fmt::Display for MapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MapError::Invalid(error) => write!(f, "invalid address or flags: {error:?}"),
            MapError::UnreadableTable { level } => {
                write!(f, "level {level} page table is not in reachable memory")
            }
            MapError::HugePageInTheWay { level } => {
                write!(f, "a level {level} huge page covers the address")
            }
            MapError::AlreadyMapped => write!(f, "page is already mapped"),
            MapError::PoolExhausted => write!(f, "boot page-table pool is exhausted"),
            MapError::PoolUntranslatable(error) => {
                write!(
                    f,
                    "cannot find the physical address of a pool table: {error:?}"
                )
            }
        }
    }
}

impl From<PagingError> for MapError {
    fn from(error: PagingError) -> Self {
        MapError::Invalid(error)
    }
}

/// Physical address of the root (PML4) of the active page tables.
pub fn root_physical() -> u64 {
    x86_64::read_cr3() & ADDRESS_MASK
}

/// Virtual address (through the HHDM) of the page table at `physical`, if the
/// kernel can safely reach it.
fn table_pointer(physical: u64) -> Option<*mut Entry> {
    if !physical.is_multiple_of(PAGE_SIZE) || !limine::is_hhdm_mapped(physical, PAGE_SIZE) {
        return None;
    }
    let virtual_address = physical.checked_add(limine::hhdm_offset()?)?;
    Some(virtual_address as *mut Entry)
}

/// Reads entry `index` of the table at `physical`.
fn read_entry(physical: u64, index: usize) -> Option<Entry> {
    debug_assert!(index < ENTRY_COUNT);
    let table = table_pointer(physical)?;
    // SAFETY: `table_pointer` checked that the page lies in a memory region the
    // bootloader maps in the HHDM and that the address is page aligned, so the
    // table is mapped and readable. `index` is below 512, inside the page.
    Some(unsafe { ptr::read_volatile(table.add(index)) })
}

/// Writes entry `index` of the table at `physical`.
fn write_entry(physical: u64, index: usize, entry: Entry) -> Option<()> {
    debug_assert!(index < ENTRY_COUNT);
    let table = table_pointer(physical)?;
    // SAFETY: as in `read_entry`. The page is a page table the CPU also reads,
    // and this is the only CPU, with interrupts disabled, so no other code
    // touches it during the write. The caller invalidates the TLB afterwards.
    unsafe { ptr::write_volatile(table.add(index), entry) };
    Some(())
}

/// Translates `address` by walking the active page tables.
pub fn translate(address: u64) -> Result<Mapping, WalkError> {
    walk(root_physical(), address, read_entry)
}

/// Takes the next unused pool table and returns its physical address.
fn allocate_table() -> Result<u64, MapError> {
    let index = POOL_USED.fetch_add(1, Ordering::Relaxed);
    if index >= POOL_TABLES {
        POOL_USED.store(POOL_TABLES, Ordering::Relaxed);
        return Err(MapError::PoolExhausted);
    }
    // The pool lives in the kernel image, so its virtual address is in the
    // higher half. Walk the page tables to find where it sits physically.
    let table = (BOOT_TABLE_POOL.get() as *mut PageTable).wrapping_add(index);
    let mapping = translate(table as u64).map_err(MapError::PoolUntranslatable)?;
    Ok(mapping.physical & ADDRESS_MASK)
}

/// Maps the 4 KiB page at `virtual_address` to `physical` with `leaf_flags`
/// (which must include `PRESENT`).
///
/// Missing intermediate tables are created from the boot pool and marked
/// present and writable. Fails rather than replacing an existing mapping.
pub fn map_page(virtual_address: u64, physical: u64, leaf_flags: u64) -> Result<(), MapError> {
    let indices = split(virtual_address)?;
    if !virtual_address.is_multiple_of(PAGE_SIZE) {
        return Err(PagingError::UnalignedAddress.into());
    }
    let leaf = Entry::new(physical, leaf_flags)?;

    let mut table = root_physical();
    for (depth, index) in [indices.pml4, indices.pdpt, indices.pd]
        .into_iter()
        .enumerate()
    {
        let level = 4 - depth as u8;
        let entry = read_entry(table, index).ok_or(MapError::UnreadableTable { level })?;
        table = if entry.is_present() {
            if entry.is_huge() {
                return Err(MapError::HugePageInTheWay { level });
            }
            entry.address()
        } else {
            let new_table = allocate_table()?;
            let link = Entry::new(new_table, flags::PRESENT | flags::WRITABLE)?;
            write_entry(table, index, link).ok_or(MapError::UnreadableTable { level })?;
            new_table
        };
    }

    let existing = read_entry(table, indices.pt).ok_or(MapError::UnreadableTable { level: 1 })?;
    if existing.is_present() {
        return Err(MapError::AlreadyMapped);
    }
    write_entry(table, indices.pt, leaf).ok_or(MapError::UnreadableTable { level: 1 })?;
    x86_64::invlpg(virtual_address);
    Ok(())
}

/// Maps one page of memory-mapped device registers: uncacheable, writable, and
/// non-executable when the CPU supports no-execute.
///
/// The cache bits (write-through plus cache-disable) select PAT entry 3, which
/// the bootloader guarantees is "uncacheable" (Limine protocol, "Caching").
pub fn map_mmio_page(virtual_address: u64, physical: u64) -> Result<(), MapError> {
    let mut mmio_flags =
        flags::PRESENT | flags::WRITABLE | flags::WRITE_THROUGH | flags::CACHE_DISABLE;
    // Setting the no-execute bit without EFER.NXE is a reserved-bit fault, so
    // only set it when the CPU has it switched on.
    if x86_64::nx_enabled() {
        mmio_flags |= flags::NO_EXECUTE;
    }
    map_page(virtual_address, physical, mmio_flags)
}
