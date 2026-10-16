//! The kernel heap: `Box`, `Vec`, `String` and friends for kernel code.
//!
//! The allocation algorithm is in the host-tested `durg-heap` crate. This module
//! supplies what only the kernel can: the memory behind it, and the Rust
//! `#[global_allocator]` hook that routes `alloc` crate collections to it.
//!
//! ## Where the memory comes from
//!
//! The heap owns a virtual window (PML4 slot 506, [`HEAP_BASE`]). Nothing is
//! mapped there at boot. [`init`] maps [`INITIAL_BYTES`], and when an allocation
//! cannot be served the heap **grows**: it takes frames from the physical frame
//! allocator, maps them at the end of the mapped part of the window as
//! read/write, no-execute pages, and hands them to the free list.
//! Growth stops at [`MAX_BYTES`], and also whenever fewer than
//! [`FRAME_RESERVE`] frames would remain in the pool, so a runaway kernel
//! allocation cannot starve page-table allocation or later users of frames.
//! The heap never shrinks: freed memory returns to the free list, not to the
//! frame allocator.
//!
//! ## Out of memory
//!
//! `GlobalAlloc::alloc` returns null when it cannot grow. Fallible APIs
//! (`Vec::try_reserve`, `Box::try_new` where available) report that as an
//! error. Infallible ones (`Vec::push`, `Box::new`) reach Rust's allocation
//! failure handler, which panics with "memory allocation of N bytes failed";
//! our panic handler prints it and exits QEMU with the failure code. The
//! `oom-test` boot variant exercises exactly that path.
//!
//! ## Concurrency
//!
//! There is no lock yet: the kernel runs on one CPU and interrupt handlers do
//! not allocate. A re-entrancy guard turns accidental nested use (an interrupt
//! handler that allocates, or an allocation inside the allocator) into a panic
//! instead of silent corruption. M7 adds a real lock before a second thread of
//! execution can allocate.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use durg_heap::Heap;
use durg_paging::{flags, PAGE_SIZE};

use crate::arch::x86_64::BootCell;
use crate::mm::frame;
use crate::mm::paging::{self, AddressSpace};

/// Start of the heap's virtual window (PML4 slot 506).
pub const HEAP_BASE: u64 = 0xffff_fd00_0000_0000;
/// Mapped by [`init`].
pub const INITIAL_BYTES: u64 = 512 * 1024;
/// The least the heap grows by when it runs dry.
pub const GROW_BYTES: u64 = 256 * 1024;
/// The most the heap will ever map.
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Frames the heap leaves in the frame allocator when it grows.
pub const FRAME_RESERVE: u64 = 64;

static HEAP: BootCell<Heap> = BootCell::new(Heap::new());
static BUSY: AtomicBool = AtomicBool::new(false);
static READY: AtomicBool = AtomicBool::new(false);
/// Bytes of the window mapped so far; the next page to map is at
/// `HEAP_BASE + MAPPED`.
static MAPPED: AtomicU64 = AtomicU64::new(0);
static GROWTHS: AtomicU64 = AtomicU64::new(0);
static FAILED_GROWTHS: AtomicU64 = AtomicU64::new(0);

/// The kernel's global allocator.
pub struct KernelAllocator;

#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator;

/// Why the heap could not grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrowError {
    /// The heap is already at [`MAX_BYTES`], or the request would pass it.
    LimitReached,
    /// Too few free frames would remain (see [`FRAME_RESERVE`]).
    FramePoolLow,
    /// Mapping a page failed. Every page mapped by this attempt was undone.
    MapFailed,
    /// The heap's own list refused the new region. Indicates a bug.
    Rejected,
}

impl core::fmt::Display for GrowError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::LimitReached => f.write_str("heap size limit reached"),
            Self::FramePoolLow => f.write_str("too few free frames to grow the heap"),
            Self::MapFailed => f.write_str("mapping heap pages failed"),
            Self::Rejected => f.write_str("heap rejected the new region"),
        }
    }
}

/// What the heap looks like right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapStats {
    pub heap: durg_heap::Stats,
    /// Bytes of the window currently mapped.
    pub mapped_bytes: u64,
    /// Successful and failed growth attempts since boot.
    pub growths: u64,
    pub failed_growths: u64,
}

/// Takes the re-entrancy guard, runs `f` on the heap, releases the guard.
fn with_heap<R>(f: impl FnOnce(&mut Heap) -> R) -> R {
    assert!(
        !BUSY.swap(true, Ordering::Acquire),
        "heap: re-entrant use (an interrupt handler or the allocator itself allocated)"
    );
    // SAFETY: the guard above guarantees exclusive access: the kernel is
    // single-CPU and any nested use panicked before reaching this line.
    let result = f(unsafe { &mut *HEAP.get() });
    BUSY.store(false, Ordering::Release);
    result
}

/// Rounds up to a whole number of pages.
fn page_round_up(bytes: u64) -> Option<u64> {
    bytes
        .checked_add(PAGE_SIZE - 1)
        .map(|b| b & !(PAGE_SIZE - 1))
}

/// Maps `bytes` more of the window and gives them to `heap`.
fn grow(heap: &mut Heap, bytes: u64) -> Result<(), GrowError> {
    let result = try_grow(heap, bytes);
    match result {
        Ok(()) => GROWTHS.fetch_add(1, Ordering::Relaxed),
        Err(_) => FAILED_GROWTHS.fetch_add(1, Ordering::Relaxed),
    };
    result
}

fn try_grow(heap: &mut Heap, bytes: u64) -> Result<(), GrowError> {
    let mapped = MAPPED.load(Ordering::Relaxed);
    let new_mapped = mapped.checked_add(bytes).ok_or(GrowError::LimitReached)?;
    if new_mapped > MAX_BYTES {
        return Err(GrowError::LimitReached);
    }
    let pages = bytes / PAGE_SIZE;
    // Page tables for the window need a few frames too; the reserve covers them.
    let free = frame::stats().map_or(0, |s| s.free);
    if free < pages + FRAME_RESERVE {
        return Err(GrowError::FramePoolLow);
    }

    let mut space = AddressSpace::active();
    let leaf = flags::PRESENT | flags::WRITABLE | paging::no_execute();
    for page in 0..pages {
        let address = HEAP_BASE + mapped + page * PAGE_SIZE;
        let mapped_ok = frame::alloc_zeroed_frame().and_then(|physical| {
            match space.map_page(address, physical, leaf) {
                Ok(()) => Some(()),
                Err(_) => {
                    // Could not map: give this frame back too.
                    let _ = frame::free_frame(physical);
                    None
                }
            }
        });
        if mapped_ok.is_none() {
            rollback(&mut space, mapped, page);
            return Err(GrowError::MapFailed);
        }
    }

    let start = (HEAP_BASE + mapped) as *mut u8;
    // SAFETY: the `bytes` bytes at `start` were just mapped read/write to frames
    // that only this code holds, nothing else uses this part of the window, and
    // it never overlaps memory the heap handed out (it lies past all of it).
    // `start` is page aligned and `bytes` a multiple of the page size, so both
    // are multiples of 16.
    unsafe { heap.add_region(start, bytes as usize) }.map_err(|_| GrowError::Rejected)?;
    MAPPED.store(new_mapped, Ordering::Relaxed);
    Ok(())
}

/// Undoes the first `pages` pages mapped by a failed growth starting at
/// `HEAP_BASE + mapped`: unmaps them and returns their frames.
fn rollback(space: &mut AddressSpace, mapped: u64, pages: u64) {
    for page in 0..pages {
        if let Ok(unmapped) = space.unmap(HEAP_BASE + mapped + page * PAGE_SIZE) {
            let _ = frame::free_frame(unmapped.physical);
        }
    }
}

// SAFETY: `alloc` returns blocks that are valid for `layout.size()` bytes,
// aligned to `layout.align()` and not handed out again until freed, which is
// what `durg_heap::Heap::alloc` guarantees; `dealloc` passes the original
// layout back as `Heap::dealloc` requires. Access is serialised by `with_heap`.
unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !READY.load(Ordering::Acquire) {
            return ptr::null_mut();
        }
        with_heap(|heap| {
            if let Some(block) = heap.alloc(layout.size(), layout.align()) {
                return block.as_ptr();
            }
            // Out of free memory: grow by enough for this request, plus the
            // worst-case alignment padding, and try once more.
            let needed = layout.size().max(1) as u64 + layout.align() as u64;
            let Some(needed) = page_round_up(needed) else {
                return ptr::null_mut();
            };
            if grow(heap, needed.max(GROW_BYTES)).is_err() && grow(heap, needed).is_err() {
                return ptr::null_mut();
            }
            heap.alloc(layout.size(), layout.align())
                .map_or(ptr::null_mut(), NonNull::as_ptr)
        })
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let Some(pointer) = NonNull::new(pointer) else {
            return;
        };
        // SAFETY: the caller promises `pointer` came from `alloc` with this
        // layout and is not used again (the `GlobalAlloc` contract).
        let freed =
            with_heap(|heap| unsafe { heap.dealloc(pointer, layout.size(), layout.align()) });
        // Freeing something the heap does not recognise means memory is
        // corrupted or a pointer was freed twice. Carrying on would hide it.
        assert!(freed.is_ok(), "heap: invalid free: {freed:?}");
    }
}

/// Maps the initial heap. Call once, after the frame allocator and the kernel's
/// own page tables exist.
pub fn init() -> Result<HeapStats, GrowError> {
    with_heap(|heap| grow(heap, INITIAL_BYTES))?;
    READY.store(true, Ordering::Release);
    Ok(stats())
}

/// Current counters.
pub fn stats() -> HeapStats {
    with_heap(|heap| HeapStats {
        heap: heap.stats(),
        mapped_bytes: MAPPED.load(Ordering::Relaxed),
        growths: GROWTHS.load(Ordering::Relaxed),
        failed_growths: FAILED_GROWTHS.load(Ordering::Relaxed),
    })
}

/// Checks the free list's invariants (see `durg_heap::Heap::check_invariants`).
pub fn check_invariants() -> Result<(), &'static str> {
    with_heap(|heap| heap.check_invariants())
}
