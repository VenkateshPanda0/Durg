//! The kernel's own address space.
//!
//! Until this runs, the CPU uses page tables the bootloader built, whose layout
//! the kernel does not control. [`take_over`] builds a new address space from
//! scratch, checks it against the live one, and switches to it. The result:
//!
//! - **W^X for the kernel image.** Code is readable and executable but not
//!   writable. Read-only data is neither writable nor executable. Data and bss
//!   are writable but not executable. The bootloader maps the kernel with
//!   page-granular permissions too, but nothing guaranteed the kernel's view of
//!   them, and the direct map still held a writable alias of the code.
//! - **No writable alias of the kernel in the direct map.** The kernel image is
//!   not mapped in the direct map at all. It is reachable only at its own
//!   addresses, with the permissions above.
//! - **Only the memory the kernel uses is mapped:** usable RAM, bootloader memory
//!   (until it is reclaimed), and ACPI tables read-only. The framebuffer and
//!   reserved ranges are absent.
//! - **A kernel stack with a guard page.** The stack lives in its own virtual
//!   region with an unmapped page below it, so an overflow raises a page fault
//!   instead of silently corrupting whatever sits below.
//!
//! ## Switching safely
//!
//! A wrong switch is a triple fault with no output, so the new address space is
//! verified first: every page of the kernel image must translate to the same
//! physical frame under both the old and the new tables, the stack the kernel is
//! running on must be mapped, and every memory region the kernel will keep
//! reading through the direct map must be present. If any check fails the kernel
//! keeps the bootloader's tables and reports why.

use core::sync::atomic::{AtomicU64, Ordering};

use durg_paging::{flags, PAGE_SIZE};

use crate::arch::x86_64;
use crate::limine::{self, MemoryKind};
use crate::mm::frame;
use crate::mm::paging::{self, AddressSpace, MapError};

extern "C" {
    // Section boundaries from `linker.ld`. Only their addresses are used.
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __data_end: u8;
}

/// Virtual window for kernel stacks (PML4 slot 508). Each stack is an unmapped
/// guard page followed by its mapped pages.
pub const STACK_REGION_BASE: u64 = 0xffff_fe00_0000_0000;

/// Pages in the kernel stack, excluding the guard page (64 KiB).
pub const STACK_PAGES: u64 = 16;

static KERNEL_ROOT: AtomicU64 = AtomicU64::new(0);

/// A section of the kernel image and the permissions it gets.
#[derive(Clone, Copy)]
pub struct Section {
    pub name: &'static str,
    pub start: u64,
    pub end: u64,
    /// Leaf flags, before the no-execute bit is added where it applies.
    writable: bool,
    executable: bool,
}

impl Section {
    fn flags(self) -> u64 {
        let mut leaf = flags::PRESENT;
        if self.writable {
            leaf |= flags::WRITABLE;
        }
        if !self.executable {
            leaf |= paging::no_execute();
        }
        leaf
    }
}

/// The kernel image's three sections, from the linker script.
pub fn sections() -> [Section; 3] {
    // Only the addresses of the linker symbols are taken, never their contents.
    [
        Section {
            name: "text",
            start: core::ptr::addr_of!(__text_start) as u64,
            end: core::ptr::addr_of!(__text_end) as u64,
            writable: false,
            executable: true,
        },
        Section {
            name: "rodata",
            start: core::ptr::addr_of!(__rodata_start) as u64,
            end: core::ptr::addr_of!(__rodata_end) as u64,
            writable: false,
            executable: false,
        },
        Section {
            name: "data+bss",
            start: core::ptr::addr_of!(__data_start) as u64,
            end: core::ptr::addr_of!(__data_end) as u64,
            writable: true,
            executable: false,
        },
    ]
}

/// What went wrong while building or checking the new address space.
#[derive(Debug, Clone, Copy)]
pub enum TakeoverError {
    Map(MapError),
    NoMemoryMap,
    NoHhdm,
    /// A kernel image page does not translate in the bootloader's tables.
    KernelPageUnmapped(u64),
    /// A required address is not mapped in the new address space.
    Missing {
        what: &'static str,
        address: u64,
    },
    /// An address maps to a different physical frame in the new tables.
    Mismatch {
        what: &'static str,
        address: u64,
        expected: u64,
        found: u64,
    },
}

impl core::fmt::Display for TakeoverError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TakeoverError::Map(error) => write!(f, "mapping failed: {error}"),
            TakeoverError::NoMemoryMap => write!(f, "no memory map"),
            TakeoverError::NoHhdm => write!(f, "no HHDM offset"),
            TakeoverError::KernelPageUnmapped(address) => {
                write!(f, "kernel page {address:#x} is not mapped by the bootloader")
            }
            TakeoverError::Missing { what, address } => {
                write!(f, "{what} at {address:#x} is not mapped in the new tables")
            }
            TakeoverError::Mismatch {
                what,
                address,
                expected,
                found,
            } => write!(
                f,
                "{what} at {address:#x} maps to {found:#x} in the new tables, expected {expected:#x}"
            ),
        }
    }
}

impl From<MapError> for TakeoverError {
    fn from(error: MapError) -> Self {
        TakeoverError::Map(error)
    }
}

/// What [`take_over`] did, for the boot log.
pub struct TakeoverInfo {
    pub root: u64,
    /// Page-table frames the new address space uses.
    pub table_frames: u64,
}

/// Physical address of the kernel's own root page table, or 0 before
/// [`take_over`].
pub fn kernel_root() -> u64 {
    KERNEL_ROOT.load(Ordering::Acquire)
}

fn align_down(value: u64) -> u64 {
    value & !(PAGE_SIZE - 1)
}

fn align_up(value: u64) -> u64 {
    value.next_multiple_of(PAGE_SIZE)
}

/// Leaf flags for the direct map of a region of this kind: RAM is writable,
/// ACPI and reserved-mapped memory is read-only, and nothing is executable.
fn hhdm_flags(kind: MemoryKind) -> u64 {
    let mut leaf = flags::PRESENT | paging::no_execute();
    if matches!(kind, MemoryKind::Usable | MemoryKind::BootloaderReclaimable) {
        leaf |= flags::WRITABLE;
    }
    leaf
}

/// Builds the kernel's address space, verifies it, and switches to it.
pub fn take_over() -> Result<TakeoverInfo, TakeoverError> {
    let map = limine::memory_map().ok_or(TakeoverError::NoMemoryMap)?;
    let hhdm = limine::hhdm_offset().ok_or(TakeoverError::NoHhdm)?;
    let frames_before = frame::stats().map(|s| s.free).unwrap_or(0);

    let bootloader_space = AddressSpace::active();
    let mut space = AddressSpace::new()?;

    // 1. The kernel image, section by section, onto the same physical frames the
    //    bootloader put it on.
    for section in sections() {
        let mut address = section.start;
        while address < section.end {
            let physical = bootloader_space
                .translate(address)
                .map_err(|_| TakeoverError::KernelPageUnmapped(address))?
                .physical;
            space.map_page(address, align_down(physical), section.flags())?;
            address += PAGE_SIZE;
        }
    }

    // 2. The direct map: only the kinds the kernel reads or writes. The memory
    //    map is sorted by base address, so `mapped_to` tracks what is done and
    //    lets regions share a boundary page (ACPI regions need not be page
    //    aligned).
    let mut mapped_to: u64 = 0;
    for region in map.iter().filter(|r| paging::kernel_maps_in_hhdm(r.kind)) {
        let start = align_down(region.base).max(mapped_to);
        let end = align_up(region.end());
        if start >= end {
            continue;
        }
        space.map_range(hhdm + start, start, end - start, hhdm_flags(region.kind))?;
        mapped_to = end;
    }

    // 3. Check before switching. A mistake past this point resets the machine.
    verify(&space, &bootloader_space, hhdm)?;

    // SAFETY: `verify` confirmed that the new address space maps every kernel
    // image page onto the same frame as the live tables, maps the stack the
    // kernel is running on, and maps the direct map regions the kernel reads. So
    // the next instruction fetch, push, and descriptor table access all work.
    unsafe { space.activate() };
    paging::mark_kernel_tables_active();
    KERNEL_ROOT.store(space.root(), Ordering::Release);

    let frames_after = frame::stats().map(|s| s.free).unwrap_or(0);
    Ok(TakeoverInfo {
        root: space.root(),
        table_frames: frames_before.saturating_sub(frames_after),
    })
}

/// Confirms that `new` covers what the kernel needs right now and after the
/// switch.
fn verify(new: &AddressSpace, old: &AddressSpace, hhdm: u64) -> Result<(), TakeoverError> {
    // Every kernel image page must reach the same physical frame.
    for section in sections() {
        let mut address = section.start;
        while address < section.end {
            let expected = align_down(
                old.translate(address)
                    .map_err(|_| TakeoverError::KernelPageUnmapped(address))?
                    .physical,
            );
            match new.translate(address) {
                Ok(mapping) if align_down(mapping.physical) == expected => {}
                Ok(mapping) => {
                    return Err(TakeoverError::Mismatch {
                        what: "kernel image page",
                        address,
                        expected,
                        found: align_down(mapping.physical),
                    })
                }
                Err(_) => {
                    return Err(TakeoverError::Missing {
                        what: "kernel image page",
                        address,
                    })
                }
            }
            address += PAGE_SIZE;
        }
    }

    // The stack the kernel is running on right now, and the page below the stack
    // pointer that the next pushes and calls will use.
    let rsp = x86_64::read_rsp();
    for (what, address) in [
        ("current stack", rsp),
        ("stack below the pointer", rsp - 256),
    ] {
        if new.translate(address).is_err() {
            return Err(TakeoverError::Missing { what, address });
        }
    }

    // The first and last page of every direct-map region the kernel reads.
    let map = limine::memory_map().ok_or(TakeoverError::NoMemoryMap)?;
    for region in map.iter().filter(|r| paging::kernel_maps_in_hhdm(r.kind)) {
        if region.length == 0 {
            continue;
        }
        let first = align_down(region.base);
        let last = align_down(region.end() - 1);
        for physical in [first, last] {
            match new.translate(hhdm + physical) {
                Ok(mapping) if align_down(mapping.physical) == physical => {}
                Ok(mapping) => {
                    return Err(TakeoverError::Mismatch {
                        what: "direct map page",
                        address: hhdm + physical,
                        expected: physical,
                        found: align_down(mapping.physical),
                    })
                }
                Err(_) => {
                    return Err(TakeoverError::Missing {
                        what: "direct map page",
                        address: hhdm + physical,
                    })
                }
            }
        }
    }
    Ok(())
}

/// The kernel stack: where it is and where its guard page is.
#[derive(Clone, Copy)]
pub struct KernelStack {
    /// Lowest address of the guard page (unmapped).
    pub guard: u64,
    /// First mapped address of the stack.
    pub bottom: u64,
    /// Address just past the stack. The stack pointer starts here.
    pub top: u64,
}

impl KernelStack {
    fn new() -> Self {
        let guard = STACK_REGION_BASE;
        let bottom = guard + PAGE_SIZE;
        Self {
            guard,
            bottom,
            top: bottom + STACK_PAGES * PAGE_SIZE,
        }
    }
}

/// The location of the kernel stack. Valid whether or not [`build_stack`] ran.
pub fn kernel_stack() -> KernelStack {
    KernelStack::new()
}

/// Allocates and maps the kernel stack in the active address space: one
/// unmapped guard page, then [`STACK_PAGES`] writable, non-executable pages.
pub fn build_stack() -> Result<KernelStack, MapError> {
    let stack = KernelStack::new();
    let mut space = AddressSpace::active();
    let leaf = flags::PRESENT | flags::WRITABLE | paging::no_execute();
    for page in 0..STACK_PAGES {
        let physical = frame::alloc_zeroed_frame().ok_or(MapError::OutOfFrames)?;
        space.map_page(stack.bottom + page * PAGE_SIZE, physical, leaf)?;
    }
    Ok(stack)
}

/// Virtual window for scratch mappings (PML4 slot 509): boot-time checks and
/// tests map and unmap pages here.
pub const SCRATCH_WINDOW: u64 = 0xffff_fe80_0000_0000;

/// Maps a fresh frame in the scratch window, writes and reads it through the
/// mapping, unmaps it, and checks the address no longer translates. A basic
/// fault in map, unmap or TLB invalidation shows up on every boot this way, and
/// not only in the self-tests.
pub fn map_unmap_round_trip() -> Result<(), &'static str> {
    const MAGIC: u64 = 0x0d0e_0a0d_beef_f00d;

    let physical = frame::alloc_zeroed_frame().ok_or("no frame to map")?;
    let mut space = AddressSpace::active();
    let leaf = flags::PRESENT | flags::WRITABLE | paging::no_execute();
    let result = (|| {
        space
            .map_page(SCRATCH_WINDOW, physical, leaf)
            .map_err(|_| "map_page failed")?;
        match space.translate(SCRATCH_WINDOW) {
            Ok(mapping) if mapping.physical == physical => {}
            _ => return Err("the new mapping does not translate to its frame"),
        }
        // SAFETY: the page was just mapped writable to a frame this function
        // owns, and the address is 8-byte aligned.
        let read_back = unsafe {
            core::ptr::write_volatile(SCRATCH_WINDOW as *mut u64, MAGIC);
            core::ptr::read_volatile(SCRATCH_WINDOW as *const u64)
        };
        if read_back != MAGIC {
            return Err("data written through the mapping did not read back");
        }
        match space.unmap(SCRATCH_WINDOW) {
            Ok(unmapped) if unmapped.physical == physical => {}
            _ => return Err("unmap did not return the mapped frame"),
        }
        if space.translate(SCRATCH_WINDOW).is_ok() {
            return Err("the page still translates after unmap");
        }
        Ok(())
    })();
    // Return the frame whether or not the checks passed.
    let _ = frame::free_frame(physical);
    result
}

/// `true` if `address` lies in the guard page below the kernel stack.
pub fn in_stack_guard(address: u64) -> bool {
    let stack = KernelStack::new();
    (stack.guard..stack.bottom).contains(&address)
}

/// Switches the stack pointer to `top` and calls `entry`, which must never
/// return. The old stack is abandoned.
///
/// # Safety
///
/// `top` must be the 16-byte-aligned top of a mapped, writable stack that nothing
/// else uses, and `entry` must not return and must not rely on anything stored on
/// the old stack.
pub unsafe fn run_on_stack(top: u64, entry: extern "C" fn() -> !) -> ! {
    // SAFETY: the caller guarantees the new stack is valid. `call` pushes a
    // return address on it, and `entry` never returns, so execution never
    // reaches the `ud2` after the call.
    unsafe {
        core::arch::asm!(
            "mov rsp, {top}",
            "xor ebp, ebp",
            "call {entry}",
            "ud2",
            top = in(reg) top,
            entry = in(reg) entry,
            options(noreturn),
        );
    }
}
