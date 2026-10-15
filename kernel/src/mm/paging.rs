//! Page tables: address spaces, mapping, unmapping and translation.
//!
//! This is the kernel half of the paging code. The pure logic (entry formats,
//! splitting addresses, the table walk) lives in the host-tested `durg-paging`
//! crate. This module supplies what that crate cannot: reading and writing the
//! actual tables in memory, and loading them into the CPU.
//!
//! An [`AddressSpace`] is a root page table (a PML4) plus methods that edit the
//! tables below it. Any root works, not only the active one: the kernel builds
//! its own address space while still running on the bootloader's, checks it, and
//! only then switches ([`AddressSpace::activate`]).
//!
//! Page-table frames come from the physical frame allocator
//! ([`crate::mm::frame`]) and are zeroed before use. Tables are reached through
//! the higher-half direct map, so a table frame must be inside memory the
//! current page tables map there (see [`hhdm_reachable`]).
//!
//! What it deliberately does not do yet:
//!
//! - It never frees page-table frames, not even when a table becomes empty.
//!   Unmapping leaves the empty tables in place.
//! - It cannot split a huge page, so unmapping part of one is refused.
//! - It does not change the permissions of an existing mapping. To change them,
//!   unmap and map again.
//! - No locking: single CPU, and interrupt handlers do not edit page tables.

use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use durg_paging::{
    flags, split, walk, Entry, Mapping, PageSize, PagingError, WalkError, ADDRESS_MASK,
    ENTRY_COUNT, HUGE_2M, PAGE_SIZE,
};

use crate::arch::x86_64;
use crate::limine::{self, MemoryKind};
use crate::mm::frame;

/// Set once the kernel has switched to page tables it built itself. From then on
/// the direct map covers only the regions the kernel chose to map, which is
/// fewer than the bootloader's.
static KERNEL_TABLES_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Records that the kernel's own page tables are now the active ones.
pub fn mark_kernel_tables_active() {
    KERNEL_TABLES_ACTIVE.store(true, Ordering::Release);
}

/// `true` if memory of this kind is mapped in the direct map of the kernel's own
/// page tables. The kernel does **not** map its own image there (that would be a
/// writable alias of its code), nor the framebuffer.
pub fn kernel_maps_in_hhdm(kind: MemoryKind) -> bool {
    matches!(
        kind,
        MemoryKind::Usable
            | MemoryKind::BootloaderReclaimable
            | MemoryKind::AcpiReclaimable
            | MemoryKind::AcpiNvs
            | MemoryKind::ReservedMapped
    )
}

/// `true` if the physical range `start..start + length` can be read through the
/// direct map under the *active* page tables: before the kernel takes over, that
/// is what the bootloader mapped, and afterwards what the kernel mapped.
pub fn hhdm_reachable(start: u64, length: u64) -> bool {
    if !KERNEL_TABLES_ACTIVE.load(Ordering::Acquire) {
        return limine::is_hhdm_mapped(start, length);
    }
    let Some(end) = start.checked_add(length) else {
        return false;
    };
    let Some(map) = limine::memory_map() else {
        return false;
    };
    let covered = map.iter().any(|region| {
        kernel_maps_in_hhdm(region.kind) && region.base <= start && end <= region.end()
    });
    covered
}

/// The no-execute flag if the CPU has it switched on, otherwise zero. Setting the
/// bit without `EFER.NXE` is a reserved-bit fault, so it must be conditional.
pub fn no_execute() -> u64 {
    if x86_64::nx_enabled() {
        flags::NO_EXECUTE
    } else {
        0
    }
}

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
    /// The frame allocator has no frame for a new page table.
    OutOfFrames,
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
            MapError::OutOfFrames => write!(f, "no free frame for a new page table"),
        }
    }
}

impl From<PagingError> for MapError {
    fn from(error: PagingError) -> Self {
        MapError::Invalid(error)
    }
}

/// Why a page could not be unmapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnmapError {
    /// The address is not canonical.
    Invalid,
    /// Nothing is mapped there: the entry at `level` is not present.
    NotMapped { level: u8 },
    /// A table on the path is not in memory the kernel can reach.
    UnreadableTable { level: u8 },
    /// The address lies inside a huge page that it does not start, so unmapping
    /// would need to split the page. Or it is a 1 GiB page, which is not
    /// supported.
    HugePageInTheWay { level: u8 },
}

/// A page that was removed by [`AddressSpace::unmap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unmapped {
    /// Physical address the page was mapped to.
    pub physical: u64,
    pub size: PageSize,
}

/// A set of page tables, identified by the physical address of its root (PML4).
pub struct AddressSpace {
    root: u64,
}

impl AddressSpace {
    /// An empty address space: a fresh zeroed root table with nothing mapped.
    pub fn new() -> Result<Self, MapError> {
        Ok(Self {
            root: allocate_table()?,
        })
    }

    /// The address space the CPU is using right now.
    pub fn active() -> Self {
        Self {
            root: x86_64::read_cr3() & ADDRESS_MASK,
        }
    }

    /// Physical address of the root table.
    pub fn root(&self) -> u64 {
        self.root
    }

    /// `true` if this is the address space the CPU is using.
    pub fn is_active(&self) -> bool {
        x86_64::read_cr3() & ADDRESS_MASK == self.root
    }

    /// Translates `address` by walking this address space's tables.
    pub fn translate(&self, address: u64) -> Result<Mapping, WalkError> {
        walk(self.root, address, read_entry)
    }

    /// Follows `path` (a list of table indices from the root) down to the table
    /// it names, creating any missing table on the way. Returns that table's
    /// physical address. The level of the first index is 4 (the PML4).
    fn descend(&mut self, path: &[usize]) -> Result<u64, MapError> {
        let mut table = self.root;
        for (depth, &index) in path.iter().enumerate() {
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
        Ok(table)
    }

    /// Maps the 4 KiB page at `virtual_address` to `physical` with `leaf_flags`
    /// (which must include `PRESENT` and must not include `HUGE`).
    ///
    /// Missing intermediate tables are created and marked present and writable
    /// (permissions are decided by the leaf). Fails rather than replacing an
    /// existing mapping.
    pub fn map_page(
        &mut self,
        virtual_address: u64,
        physical: u64,
        leaf_flags: u64,
    ) -> Result<(), MapError> {
        let indices = split(virtual_address)?;
        if !virtual_address.is_multiple_of(PAGE_SIZE) {
            return Err(PagingError::UnalignedAddress.into());
        }
        if leaf_flags & flags::HUGE != 0 {
            return Err(PagingError::FlagsOverlapAddress.into());
        }
        let leaf = Entry::new(physical, leaf_flags)?;

        let table = self.descend(&[indices.pml4, indices.pdpt, indices.pd])?;
        let existing =
            read_entry(table, indices.pt).ok_or(MapError::UnreadableTable { level: 1 })?;
        if existing.is_present() {
            return Err(MapError::AlreadyMapped);
        }
        write_entry(table, indices.pt, leaf).ok_or(MapError::UnreadableTable { level: 1 })?;
        if self.is_active() {
            x86_64::invlpg(virtual_address);
        }
        Ok(())
    }

    /// Maps a 2 MiB huge page. Both addresses must be 2 MiB aligned. `leaf_flags`
    /// must include `PRESENT`. The `HUGE` bit is added here.
    pub fn map_2m(
        &mut self,
        virtual_address: u64,
        physical: u64,
        leaf_flags: u64,
    ) -> Result<(), MapError> {
        let indices = split(virtual_address)?;
        if !virtual_address.is_multiple_of(HUGE_2M) || !physical.is_multiple_of(HUGE_2M) {
            return Err(PagingError::UnalignedAddress.into());
        }
        let leaf = Entry::new(physical, leaf_flags | flags::HUGE)?;

        let table = self.descend(&[indices.pml4, indices.pdpt])?;
        let existing =
            read_entry(table, indices.pd).ok_or(MapError::UnreadableTable { level: 2 })?;
        if existing.is_present() {
            return Err(MapError::AlreadyMapped);
        }
        write_entry(table, indices.pd, leaf).ok_or(MapError::UnreadableTable { level: 2 })?;
        if self.is_active() {
            x86_64::invlpg(virtual_address);
        }
        Ok(())
    }

    /// Maps `length` bytes (a multiple of the page size) starting at
    /// `virtual_address` to `physical`, using 2 MiB pages wherever both
    /// addresses are 2 MiB aligned and at least 2 MiB remain, and 4 KiB pages
    /// elsewhere. Stops at the first error, leaving earlier pages mapped.
    pub fn map_range(
        &mut self,
        virtual_address: u64,
        physical: u64,
        length: u64,
        leaf_flags: u64,
    ) -> Result<(), MapError> {
        if !length.is_multiple_of(PAGE_SIZE) {
            return Err(PagingError::UnalignedAddress.into());
        }
        let mut offset = 0;
        while offset < length {
            let virt = virtual_address + offset;
            let phys = physical + offset;
            if virt.is_multiple_of(HUGE_2M)
                && phys.is_multiple_of(HUGE_2M)
                && length - offset >= HUGE_2M
            {
                self.map_2m(virt, phys, leaf_flags)?;
                offset += HUGE_2M;
            } else {
                self.map_page(virt, phys, leaf_flags)?;
                offset += PAGE_SIZE;
            }
        }
        Ok(())
    }

    /// Removes the page that starts at `virtual_address` and invalidates its TLB
    /// entry. For a 2 MiB page, `virtual_address` must be its start.
    ///
    /// Page-table frames are not freed, even if a table is now empty.
    pub fn unmap(&mut self, virtual_address: u64) -> Result<Unmapped, UnmapError> {
        let indices = split(virtual_address).map_err(|_| UnmapError::Invalid)?;

        let mut table = self.root;
        for (depth, index) in [indices.pml4, indices.pdpt, indices.pd]
            .into_iter()
            .enumerate()
        {
            let level = 4 - depth as u8;
            let entry = read_entry(table, index).ok_or(UnmapError::UnreadableTable { level })?;
            if !entry.is_present() {
                return Err(UnmapError::NotMapped { level });
            }
            if entry.is_huge() {
                // Only a 2 MiB page (level 2) that starts exactly here can be
                // removed whole. Anything else would mean splitting a huge page.
                if level == 2 && virtual_address.is_multiple_of(HUGE_2M) {
                    write_entry(table, index, Entry::EMPTY)
                        .ok_or(UnmapError::UnreadableTable { level })?;
                    if self.is_active() {
                        x86_64::invlpg(virtual_address);
                    }
                    return Ok(Unmapped {
                        physical: entry.address(),
                        size: PageSize::Size2M,
                    });
                }
                return Err(UnmapError::HugePageInTheWay { level });
            }
            table = entry.address();
        }

        let leaf = read_entry(table, indices.pt).ok_or(UnmapError::UnreadableTable { level: 1 })?;
        if !leaf.is_present() {
            return Err(UnmapError::NotMapped { level: 1 });
        }
        write_entry(table, indices.pt, Entry::EMPTY)
            .ok_or(UnmapError::UnreadableTable { level: 1 })?;
        // Without this the CPU could keep using its cached translation and the
        // page would stay reachable after being unmapped.
        if self.is_active() {
            x86_64::invlpg(virtual_address);
        }
        Ok(Unmapped {
            physical: leaf.address(),
            size: PageSize::Size4K,
        })
    }

    /// Makes this address space the active one by loading its root into CR3. The
    /// CPU flushes its (non-global) TLB entries as part of the switch.
    ///
    /// # Safety
    ///
    /// Everything the kernel uses after the switch must be mapped in this address
    /// space with the right permissions: the code that is running, the current
    /// stack, the GDT, IDT and TSS, every static the kernel touches, and the
    /// direct map regions it reads. If any is missing, the CPU faults on the next
    /// instruction or push, and since the fault handler is itself unmapped the
    /// machine resets.
    pub unsafe fn activate(&self) {
        // SAFETY: the caller guarantees this address space maps everything the
        // kernel needs. Loading CR3 does not touch memory, and the new root is a
        // page-aligned physical address, so the low flag bits are zero.
        unsafe {
            core::arch::asm!("mov cr3, {}", in(reg) self.root, options(nostack, preserves_flags));
        }
    }
}

/// Virtual address (through the direct map) of the page table at `physical`, if
/// the kernel can safely reach it.
fn table_pointer(physical: u64) -> Option<*mut Entry> {
    if !physical.is_multiple_of(PAGE_SIZE) || !hhdm_reachable(physical, PAGE_SIZE) {
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
    // active page tables map in the direct map and that the address is page
    // aligned, so the table is mapped and readable. `index` is below 512, inside
    // the page.
    Some(unsafe { ptr::read_volatile(table.add(index)) })
}

/// Writes entry `index` of the table at `physical`.
fn write_entry(physical: u64, index: usize, entry: Entry) -> Option<()> {
    debug_assert!(index < ENTRY_COUNT);
    let table = table_pointer(physical)?;
    // SAFETY: as in `read_entry`, and the region kinds the direct map covers are
    // mapped writable. The page is a page table the CPU may also read, and this
    // is the only CPU with interrupts disabled during edits, so nothing else
    // touches it during the write. Callers invalidate the TLB afterwards.
    unsafe { ptr::write_volatile(table.add(index), entry) };
    Some(())
}

/// A zeroed frame for a new page table, from the physical frame allocator.
/// Zeroing matters: stale bytes would be read as present entries.
fn allocate_table() -> Result<u64, MapError> {
    frame::alloc_zeroed_frame().ok_or(MapError::OutOfFrames)
}

/// Translates `address` in the active address space.
pub fn translate(address: u64) -> Result<Mapping, WalkError> {
    AddressSpace::active().translate(address)
}

/// Maps one page of memory-mapped device registers in the active address space:
/// uncacheable, writable, and non-executable when the CPU supports no-execute.
///
/// The cache bits (write-through plus cache-disable) select PAT entry 3, which
/// the bootloader guarantees is "uncacheable" (Limine protocol, "Caching").
pub fn map_mmio_page(virtual_address: u64, physical: u64) -> Result<(), MapError> {
    let mmio_flags = flags::PRESENT
        | flags::WRITABLE
        | flags::WRITE_THROUGH
        | flags::CACHE_DISABLE
        | no_execute();
    AddressSpace::active().map_page(virtual_address, physical, mmio_flags)
}
