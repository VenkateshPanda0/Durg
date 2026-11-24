//! Kernel heap tests (M6).
//!
//! Part of the in-kernel self-tests; see [`super`] for how they are run.

use super::*;

// ---- kernel heap (M6) -------------------------------------------------------

/// A small deterministic random number generator (xorshift64), so a failing
/// run can be reproduced from its seed.
pub(super) struct Rng(pub(super) u64);

impl Rng {
    pub(super) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(super) fn below(&mut self, limit: u64) -> u64 {
        self.next() % limit
    }
}

pub(super) fn heap_collections_work() {
    use alloc::boxed::Box;
    use alloc::collections::BTreeMap;
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::fmt::Write;

    let boxed = Box::new([7u8; 300]);
    assert!(boxed.iter().all(|&b| b == 7));

    let mut numbers = Vec::new();
    for n in 0..5000u32 {
        numbers.push(n);
    }
    assert_eq!(
        numbers.iter().map(|&n| u64::from(n)).sum::<u64>(),
        12_497_500
    );

    let mut text = String::new();
    for n in 0..200 {
        write!(text, "{n},").expect("writing to a String cannot fail");
    }
    assert!(text.starts_with("0,1,2,") && text.ends_with("198,199,"));

    let mut map = BTreeMap::new();
    for n in 0..500u64 {
        map.insert(n * 7919 % 1000, n);
    }
    assert_eq!(map.len(), 500);
    assert_eq!(map.get(&(3 * 7919 % 1000)), Some(&3));
}

pub(super) fn heap_alignment_is_honoured() {
    use alloc::alloc::{alloc, dealloc};
    use core::alloc::Layout;

    // Keep a small block alive so the start of the free memory is only 16-byte
    // aligned. Without it, the heap's first block begins on a page boundary and
    // every alignment would be met by luck.
    let spacer_layout = Layout::from_size_align(16, 1).expect("valid layout");
    // SAFETY: the layout has a non-zero size.
    let spacer = unsafe { alloc(spacer_layout) };
    assert!(!spacer.is_null(), "spacer allocation failed");

    for shift in 0..=12 {
        let align = 1usize << shift;
        for size in [1usize, 24, 100, 4096] {
            let layout = Layout::from_size_align(size, align).expect("valid layout");
            // SAFETY: the layout has a non-zero size.
            let pointer = unsafe { alloc(layout) };
            assert!(!pointer.is_null(), "alloc({size}, {align}) failed");
            assert_eq!(
                pointer as usize % align,
                0,
                "alloc({size}, {align}) returned a misaligned pointer"
            );
            // SAFETY: `pointer` came from `alloc` with this layout and is freed once.
            unsafe { dealloc(pointer, layout) };
        }
    }
    // SAFETY: allocated above with this layout, freed once.
    unsafe { dealloc(spacer, spacer_layout) };
    mm::heap::check_invariants().expect("free list corrupted");
}

/// Many random allocations and frees, each block filled with a pattern that is
/// checked before it is freed. Overlapping blocks, or a free that damages a
/// neighbour, change a pattern and fail the test. At the end everything is
/// freed and the free memory must be exactly what it was before (no leak, no
/// loss to fragmentation).
pub(super) fn heap_stress_keeps_contents_intact() {
    use alloc::alloc::{alloc, dealloc};
    use alloc::vec::Vec;
    use core::alloc::Layout;

    struct Live {
        pointer: *mut u8,
        layout: Layout,
        pattern: u8,
    }

    fn check(live: &Live) {
        for offset in 0..live.layout.size() {
            // SAFETY: the block is live and `offset` is inside it.
            let byte = unsafe { *live.pointer.add(offset) };
            assert_eq!(byte, live.pattern, "heap block was overwritten");
        }
    }

    let before = mm::heap::stats();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut blocks: Vec<Live> = Vec::new();
    for round in 0..3000u32 {
        if blocks.len() < 200 && (blocks.is_empty() || rng.below(100) < 55) {
            let size = 1 + rng.below(1500) as usize;
            let align = 1usize << rng.below(7);
            let layout = Layout::from_size_align(size, align).expect("valid layout");
            // SAFETY: the layout has a non-zero size.
            let pointer = unsafe { alloc(layout) };
            assert!(!pointer.is_null(), "allocation failed in round {round}");
            let pattern = (round % 251) as u8 + 1;
            // SAFETY: the block is valid for `size` bytes and exclusively ours.
            unsafe { core::ptr::write_bytes(pointer, pattern, size) };
            blocks.push(Live {
                pointer,
                layout,
                pattern,
            });
        } else {
            let index = rng.below(blocks.len() as u64) as usize;
            let victim = blocks.swap_remove(index);
            check(&victim);
            // SAFETY: allocated above with this layout, freed once.
            unsafe { dealloc(victim.pointer, victim.layout) };
        }
    }
    for victim in blocks {
        check(&victim);
        // SAFETY: allocated above with this layout, freed once.
        unsafe { dealloc(victim.pointer, victim.layout) };
    }
    mm::heap::check_invariants().expect("free list corrupted");
    let after = mm::heap::stats();
    // Anything the heap grew by during the test is free again too.
    let grown = (after.mapped_bytes - before.mapped_bytes) as usize;
    assert_eq!(
        after.heap.free_bytes,
        before.heap.free_bytes + grown,
        "free memory changed: a block leaked"
    );
    // Other long-lived allocations (the scheduler's run queue, for one) may split
    // the free memory, so "one block" is not the property. Everything this test
    // took must merge back into the shape the free list had before it.
    assert_eq!(
        after.heap.free_blocks, before.heap.free_blocks,
        "free memory did not merge back"
    );
}

pub(super) fn heap_grows_on_demand() {
    use alloc::vec::Vec;

    let before = mm::heap::stats();
    let want = before.heap.largest_free_block + 1024 * 1024;
    let mut big: Vec<u8> = Vec::new();
    big.try_reserve_exact(want)
        .expect("the heap should have grown to serve this");
    big.resize(want, 0xa5);
    let during = mm::heap::stats();
    assert!(
        during.mapped_bytes > before.mapped_bytes,
        "no pages were mapped"
    );
    assert_eq!(during.growths, before.growths + 1, "expected one growth");
    assert!(big.iter().all(|&b| b == 0xa5));
    drop(big);
    mm::heap::check_invariants().expect("free list corrupted");
    let after = mm::heap::stats();
    assert_eq!(
        after.mapped_bytes, during.mapped_bytes,
        "the heap never shrinks"
    );
    assert!(
        after.heap.largest_free_block >= want,
        "freed memory is available again"
    );
}

pub(super) fn heap_pages_are_rw_nx() {
    let mapping = paging::translate(mm::heap::HEAP_BASE).expect("heap base is not mapped");
    let raw = mapping.entry.raw();
    assert_ne!(raw & flags::WRITABLE, 0, "heap page is not writable");
    assert_ne!(raw & flags::NO_EXECUTE, 0, "heap page is executable");
    assert_eq!(raw & flags::USER, 0, "heap page is user-accessible");
    // The page just past the mapped part of the window must not be mapped.
    let edge = mm::heap::HEAP_BASE + mm::heap::stats().mapped_bytes;
    assert!(
        paging::translate(edge).is_err(),
        "memory past the end of the heap is mapped"
    );
}

pub(super) fn heap_oom_is_graceful() {
    use alloc::vec::Vec;

    let before = mm::heap::stats();
    let mut huge: Vec<u8> = Vec::new();
    // Far past the 64 MiB limit.
    assert!(
        huge.try_reserve(1 << 30).is_err(),
        "a 1 GiB request must be refused"
    );
    let after = mm::heap::stats();
    assert!(after.failed_growths > before.failed_growths);
    assert_eq!(
        after.mapped_bytes, before.mapped_bytes,
        "failed growth leaked pages"
    );
    assert_eq!(after.heap.free_bytes, before.heap.free_bytes);
    mm::heap::check_invariants().expect("free list corrupted");
    // And the heap still works.
    let mut small: Vec<u8> = Vec::new();
    small
        .try_reserve(4096)
        .expect("heap unusable after a refused request");
}

/// The global allocator panics on a bad free (a bug elsewhere), so the checked
/// path is tested on a private heap built over one frame from the frame
/// allocator.
pub(super) fn heap_detects_bad_frees() {
    use core::ptr::NonNull;
    use durg_heap::{Heap, HeapError};

    let physical = frame::alloc_zeroed_frame().expect("no frame for the test heap");
    let base = (physical + limine::hhdm_offset().expect("no HHDM")) as *mut u8;
    let mut heap = Heap::new();
    // SAFETY: one whole frame reachable through the HHDM, exclusively ours until
    // it is freed at the end of this test, 4096 bytes being a multiple of 16.
    unsafe { heap.add_region(base, 4096) }.expect("add_region failed");

    let block = heap.alloc(100, 8).expect("allocation failed");
    // SAFETY: allocated just above with this layout.
    assert_eq!(unsafe { heap.dealloc(block, 100, 8) }, Ok(()));
    assert_eq!(
        // SAFETY: deliberately wrong (a second free of the same block); the heap
        // must refuse it without touching anything.
        unsafe { heap.dealloc(block, 100, 8) },
        Err(HeapError::OverlapsFreeMemory),
        "double free was accepted"
    );
    let wild = NonNull::new(base.wrapping_add(2048)).expect("non-null");
    assert_eq!(
        // SAFETY: deliberately a pointer that was never allocated, inside free memory.
        unsafe { heap.dealloc(wild, 64, 8) },
        Err(HeapError::OverlapsFreeMemory),
        "free of a never-allocated block was accepted"
    );
    heap.check_invariants().expect("bad frees damaged the list");
    assert_eq!(heap.stats().free_bytes, 4096);
    frame::free_frame(physical).expect("could not return the frame");
}
