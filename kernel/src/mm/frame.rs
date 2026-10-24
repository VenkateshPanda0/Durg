//! Physical frame allocator: the kernel side of `durg-pmm`.
//!
//! The pure allocator logic (bitmaps, the allocated/free/not-ours states, the
//! search) lives in the host-tested `durg-pmm` crate. This module decides what
//! the allocator manages, finds memory for its bitmaps, and turns frame numbers
//! into memory the kernel can touch.
//!
//! ## What is managed
//!
//! Only **usable** RAM from the bootloader's memory map, and only above 1 MiB:
//!
//! - *Bootloader-reclaimable* memory is left alone, because it holds the page
//!   tables, the memory map itself and the boot stack. It can be added once the
//!   kernel has copied out what it needs and taken over the page tables (M5).
//! - *Kernel and modules*, framebuffer, ACPI, reserved and everything else is
//!   never managed, so freeing such a frame is an error and not a way to put it
//!   up for allocation.
//! - The first MiB is kept out of the pool. It holds firmware and BIOS data and
//!   is the traditional home of the real-mode trampoline for starting other
//!   CPUs. Keeping it reserved is a policy choice, easy to relax later.
//!
//! ## Where the bitmaps live
//!
//! In the first usable region (above 1 MiB) with room for them, reached through
//! the higher-half direct map. They cover frames up to the highest *usable*
//! address, not the highest address in the map, because the map contains
//! reserved ranges near 1 TiB whose bitmap would waste tens of megabytes.
//!
//! ## Concurrency
//!
//! The allocator sits behind an interrupt-safe lock (`IrqLock`): interrupts stay
//! off while it is held, so the scheduler cannot switch threads in the middle
//! of an allocation. Taking it while it is already held (a bug) panics instead
//! of hanging. It is the innermost lock: it never takes another one.

use core::ptr;
use core::slice;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use durg_pmm::{words_needed, BitmapAllocator, Frame, FreeError, FRAME_SIZE};

use crate::arch::x86_64::IrqLock;
use crate::limine::{self, MemoryKind};

/// Memory below this physical address is never handed out.
const LOW_MEMORY_LIMIT: u64 = 1 << 20;

static ALLOCATOR: IrqLock<Option<BitmapAllocator<'static>>> = IrqLock::new(None);

/// Where the bitmaps live and how big they are. Written once by [`init`].
static BITMAP_PHYSICAL: AtomicU64 = AtomicU64::new(0);
static BITMAP_BYTES: AtomicU64 = AtomicU64::new(0);

/// What [`init`] set up, for the boot log and the tests.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Stats {
    /// Frames the bitmaps can describe (frame numbers `0..frame_count`).
    pub frame_count: u64,
    /// Frames in the pool, free or allocated.
    pub managed: u64,
    /// Frames currently free.
    pub free: u64,
    /// Physical address of the bitmap storage.
    pub bitmap_physical: u64,
    /// Size of the bitmap storage in bytes.
    pub bitmap_bytes: u64,
}

/// Why the allocator could not be set up.
#[derive(Debug, Clone, Copy)]
pub enum InitError {
    AlreadyInitialised,
    NoMemoryMap,
    NoHhdm,
    NoUsableMemory,
    /// No usable region above 1 MiB is big enough for the bitmaps.
    NoRoomForBitmaps,
    Allocator(durg_pmm::InitError),
    Range(durg_pmm::RangeError),
}

impl core::fmt::Display for InitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            InitError::AlreadyInitialised => write!(f, "frame allocator is already initialised"),
            InitError::NoMemoryMap => write!(f, "the bootloader gave no memory map"),
            InitError::NoHhdm => write!(f, "the bootloader gave no HHDM offset"),
            InitError::NoUsableMemory => write!(f, "the memory map has no usable RAM"),
            InitError::NoRoomForBitmaps => {
                write!(f, "no usable region above 1 MiB can hold the bitmaps")
            }
            InitError::Allocator(error) => write!(f, "allocator setup failed: {error:?}"),
            InitError::Range(error) => write!(f, "range setup failed: {error:?}"),
        }
    }
}

/// Why a frame could not be returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeFrameError {
    NotInitialised,
    /// The physical address is not a multiple of the frame size.
    Unaligned,
    /// The allocator refused it: out of range, never owned, or already free.
    Refused(FreeError),
}

fn align_up(value: u64, alignment: u64) -> u64 {
    value.next_multiple_of(alignment)
}

/// Memory kinds that are, or can become, part of the pool.
fn is_poolable(kind: MemoryKind) -> bool {
    matches!(kind, MemoryKind::Usable | MemoryKind::BootloaderReclaimable)
}

/// Why bootloader memory could not be reclaimed.
#[derive(Debug, Clone, Copy)]
pub enum ReclaimError {
    NotInitialised,
    NoMemoryMap,
    /// The kernel is still running on the bootloader's page tables, which live
    /// in the memory that would be reclaimed.
    KernelTablesNotActive,
    AlreadyReclaimed,
    Range(durg_pmm::RangeError),
}

impl core::fmt::Display for ReclaimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReclaimError::NotInitialised => write!(f, "frame allocator is not initialised"),
            ReclaimError::NoMemoryMap => write!(f, "no memory map"),
            ReclaimError::KernelTablesNotActive => {
                write!(f, "the kernel is still on the bootloader's page tables")
            }
            ReclaimError::AlreadyReclaimed => write!(f, "bootloader memory was already reclaimed"),
            ReclaimError::Range(error) => write!(f, "range setup failed: {error:?}"),
        }
    }
}

static RECLAIMED: AtomicBool = AtomicBool::new(false);

/// Adds the bootloader-reclaimable memory above 1 MiB to the pool and returns the
/// number of frames added.
///
/// That memory holds the bootloader's page tables, its boot stack and its
/// response structures. It is safe to reuse only when none of those are in use:
/// the kernel must be on its own page tables (checked here), on its own stack,
/// and must have copied everything it needs from the responses (done once, by
/// `limine::capture`).
pub fn reclaim_bootloader_memory() -> Result<u64, ReclaimError> {
    if crate::mm::vm::kernel_root() == 0 {
        return Err(ReclaimError::KernelTablesNotActive);
    }
    if RECLAIMED.swap(true, Ordering::AcqRel) {
        return Err(ReclaimError::AlreadyReclaimed);
    }
    let map = limine::memory_map().ok_or(ReclaimError::NoMemoryMap)?;
    with_allocator(|allocator| {
        let mut added = 0;
        for region in map
            .iter()
            .filter(|r| r.kind == MemoryKind::BootloaderReclaimable)
        {
            let start = align_up(region.base.max(LOW_MEMORY_LIMIT), FRAME_SIZE);
            let end = region.end() / FRAME_SIZE * FRAME_SIZE;
            if start < end {
                added += allocator
                    .add_range(Frame::containing(start), (end - start) / FRAME_SIZE)
                    .map_err(ReclaimError::Range)?;
            }
        }
        Ok(added)
    })
    .ok_or(ReclaimError::NotInitialised)?
}

/// Builds the allocator from the bootloader's memory map.
pub fn init() -> Result<Stats, InitError> {
    if ALLOCATOR.lock().is_some() {
        return Err(InitError::AlreadyInitialised);
    }
    let map = limine::memory_map().ok_or(InitError::NoMemoryMap)?;
    let hhdm = limine::hhdm_offset().ok_or(InitError::NoHhdm)?;

    // Cover frames up to the highest address of any memory that is or will become
    // allocatable: usable RAM now, and bootloader-reclaimable memory once the
    // kernel has stopped using it. Not the highest address in the whole map,
    // which includes reserved ranges near 1 TiB.
    if !map.iter().any(|region| region.kind == MemoryKind::Usable) {
        return Err(InitError::NoUsableMemory);
    }
    let highest = map
        .iter()
        .filter(|region| is_poolable(region.kind))
        .map(|region| region.end())
        .max()
        .ok_or(InitError::NoUsableMemory)?;
    let frame_count = highest.div_ceil(FRAME_SIZE);

    let words = words_needed(frame_count);
    let bitmap_bytes = (words * core::mem::size_of::<u64>()) as u64;
    let bitmap_frames = bitmap_bytes.div_ceil(FRAME_SIZE);

    // The first usable region, above 1 MiB, with room for the bitmaps.
    let bitmap_physical = map
        .iter()
        .filter(|region| region.kind == MemoryKind::Usable)
        .find_map(|region| {
            let start = align_up(region.base.max(LOW_MEMORY_LIMIT), FRAME_SIZE);
            let end = start.checked_add(bitmap_frames * FRAME_SIZE)?;
            (end <= region.end()).then_some(start)
        })
        .ok_or(InitError::NoRoomForBitmaps)?;

    // SAFETY: the bitmaps go in a usable region the bootloader maps in the HHDM,
    // so `hhdm + bitmap_physical` is mapped, writable, and large enough for
    // `words` words. Nothing else uses this memory: it is free RAM, and the code
    // below removes it from the pool before anything can be allocated from it.
    // The slice is the only reference to that memory and is stored for the
    // lifetime of the kernel.
    let storage: &'static mut [u64] =
        unsafe { slice::from_raw_parts_mut((bitmap_physical + hhdm) as *mut u64, words) };

    let mut allocator = BitmapAllocator::new(storage, frame_count).map_err(InitError::Allocator)?;

    for region in map.iter().filter(|r| r.kind == MemoryKind::Usable) {
        let start = align_up(region.base.max(LOW_MEMORY_LIMIT), FRAME_SIZE);
        let end = region.end() / FRAME_SIZE * FRAME_SIZE;
        if start < end {
            allocator
                .add_range(Frame::containing(start), (end - start) / FRAME_SIZE)
                .map_err(InitError::Range)?;
        }
    }
    // The bitmaps live in memory that was just added, so take it back out.
    allocator
        .reserve_range(Frame::containing(bitmap_physical), bitmap_frames)
        .map_err(InitError::Range)?;

    let stats = Stats {
        frame_count,
        managed: allocator.managed_count(),
        free: allocator.free_count(),
        bitmap_physical,
        bitmap_bytes,
    };
    BITMAP_PHYSICAL.store(bitmap_physical, Ordering::Relaxed);
    BITMAP_BYTES.store(bitmap_bytes, Ordering::Relaxed);
    *ALLOCATOR.lock() = Some(allocator);
    Ok(stats)
}

/// Runs `f` on the allocator, or returns `None` if it is not initialised.
fn with_allocator<R>(f: impl FnOnce(&mut BitmapAllocator<'static>) -> R) -> Option<R> {
    // The lock keeps interrupts off, so a timer interrupt cannot switch to
    // another thread in the middle of an allocation.
    ALLOCATOR.lock().as_mut().map(f)
}

/// Current counters, or `None` before [`init`].
pub fn stats() -> Option<Stats> {
    with_allocator(|a| Stats {
        frame_count: a.frame_count(),
        managed: a.managed_count(),
        free: a.free_count(),
        bitmap_physical: BITMAP_PHYSICAL.load(Ordering::Relaxed),
        bitmap_bytes: BITMAP_BYTES.load(Ordering::Relaxed),
    })
}

/// Takes one frame and returns its physical address, or `None` if memory is
/// exhausted or the allocator is not initialised. The contents are not cleared.
pub fn alloc_frame() -> Option<u64> {
    with_allocator(|a| a.alloc())
        .flatten()
        .map(Frame::start_address)
}

/// Takes the first naturally aligned block of `1 << log2_frames` free frames
/// out of the pool for good and returns its physical address. The capability
/// system hands such blocks to the root task as untyped memory; they stay
/// "not ours" to this allocator until [`release_block`] puts them back.
pub fn reserve_aligned_block(log2_frames: u32) -> Option<u64> {
    with_allocator(|a| {
        let start = a.find_free_aligned_run(log2_frames)?;
        let removed = a.reserve_range(start, 1u64 << log2_frames).ok()?;
        // `find_free_aligned_run` only returns blocks that are entirely free, so
        // every frame is removed; anything else would be an allocator bug.
        assert_eq!(
            removed,
            1u64 << log2_frames,
            "reserved block was not all free"
        );
        Some(start.start_address())
    })
    .flatten()
}

/// Returns a block taken by [`reserve_aligned_block`] to the pool. `false` if
/// the allocator is not initialised or the range is out of bounds.
pub fn release_block(physical: u64, log2_frames: u32) -> bool {
    with_allocator(|a| {
        a.add_range(Frame::containing(physical), 1u64 << log2_frames)
            .is_ok()
    })
    .unwrap_or(false)
}

/// Like [`alloc_frame`], but the frame is filled with zeros first.
pub fn alloc_zeroed_frame() -> Option<u64> {
    let physical = alloc_frame()?;
    let virtual_address = physical.checked_add(limine::hhdm_offset()?)?;
    // SAFETY: the frame was just taken from the pool, which only holds usable
    // RAM that the bootloader maps in the HHDM, so the address is mapped and
    // writable for a whole frame. The caller owns the frame exclusively.
    unsafe { ptr::write_bytes(virtual_address as *mut u8, 0, FRAME_SIZE as usize) };
    Some(physical)
}

/// Returns a frame to the pool.
pub fn free_frame(physical: u64) -> Result<(), FreeFrameError> {
    if !physical.is_multiple_of(FRAME_SIZE) {
        return Err(FreeFrameError::Unaligned);
    }
    with_allocator(|a| a.free(Frame::containing(physical)))
        .ok_or(FreeFrameError::NotInitialised)?
        .map_err(FreeFrameError::Refused)
}

/// Verifies the allocator's internal invariants. Costs time proportional to the
/// bitmap size, so it is for tests and boot-time checks.
#[cfg(feature = "selftest")]
pub fn check_invariants() -> Result<(), &'static str> {
    with_allocator(|a| a.check_invariants()).unwrap_or(Err("frame allocator is not initialised"))
}

/// Copies `buffer.len()` bytes out of physical memory at `physical` into
/// `buffer`, through the direct map. `None` if any of it is not memory the kernel
/// maps there.
pub fn read_bytes(physical: u64, buffer: &mut [u8]) -> Option<()> {
    let source = hhdm_pointer(physical, buffer.len() as u64)?;
    // SAFETY: `hhdm_pointer` checked that the whole range lies in memory the
    // direct map covers (readable), and `buffer` is a distinct Rust slice, so the
    // two cannot overlap.
    unsafe { ptr::copy_nonoverlapping(source, buffer.as_mut_ptr(), buffer.len()) };
    Some(())
}

/// Copies `bytes` into physical memory at `physical`, through the direct map.
/// `None` if any of it is not memory the kernel maps writable there. The caller
/// must own the destination frames.
pub fn write_bytes(physical: u64, bytes: &[u8]) -> Option<()> {
    let destination = hhdm_pointer(physical, bytes.len() as u64)?;
    // SAFETY: as in `read_bytes`; the direct map of usable RAM is writable, and
    // the caller owns the frames being written.
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
    Some(())
}

/// Pointer into the direct map for `length` bytes at `physical`, if the kernel
/// maps all of them.
fn hhdm_pointer(physical: u64, length: u64) -> Option<*mut u8> {
    if length == 0 || !crate::mm::paging::hhdm_reachable(physical, length) {
        return None;
    }
    Some(physical.checked_add(limine::hhdm_offset()?)? as *mut u8)
}

/// Reads 8 bytes at `byte_offset` inside the frame at `physical`.
///
/// For tests that need to look at frame contents. Returns `None` if the frame is
/// not in memory the bootloader maps, or the offset does not leave room for 8
/// bytes. The caller must own the frame.
#[cfg(feature = "selftest")]
pub fn read_word(physical: u64, byte_offset: u64) -> Option<u64> {
    let address = word_address(physical, byte_offset)?;
    // SAFETY: `word_address` checked that the frame lies in HHDM-mapped memory
    // and the word lies inside it, so the read is to a mapped, aligned address.
    Some(unsafe { ptr::read_volatile(address as *const u64) })
}

/// Writes 8 bytes at `byte_offset` inside the frame at `physical`. See
/// [`read_word`].
#[cfg(feature = "selftest")]
pub fn write_word(physical: u64, byte_offset: u64, value: u64) -> Option<()> {
    let address = word_address(physical, byte_offset)?;
    // SAFETY: as in `read_word`. The caller owns the frame, so no other code
    // reads or writes it concurrently.
    unsafe { ptr::write_volatile(address as *mut u64, value) };
    Some(())
}

#[cfg(feature = "selftest")]
fn word_address(physical: u64, byte_offset: u64) -> Option<u64> {
    if !physical.is_multiple_of(FRAME_SIZE)
        || !byte_offset.is_multiple_of(8)
        || byte_offset + 8 > FRAME_SIZE
        || !crate::mm::paging::hhdm_reachable(physical, FRAME_SIZE)
    {
        return None;
    }
    physical
        .checked_add(byte_offset)?
        .checked_add(limine::hhdm_offset()?)
}
