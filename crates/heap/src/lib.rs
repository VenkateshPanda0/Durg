//! # A linked-list heap allocator
//!
//! A heap hands out variable-sized pieces of memory and takes them back. This one
//! keeps a **free list**: a singly linked list of the free blocks, sorted by
//! address, with each list node stored *inside* the free block it describes. So
//! the bookkeeping costs no extra memory, and a block's header disappears when
//! the block is allocated.
//!
//! ## How it works
//!
//! - **Allocate:** walk the list for the first block that can hold the request at
//!   the right alignment (*first fit*). Cut the request out of it. Anything left
//!   in front of or behind the cut goes back on the list as smaller free blocks.
//! - **Free:** put the block back in its sorted position, and **merge** it with the
//!   free block just before it and the one just after it if they touch
//!   (*coalescing*). Without merging, a heap fragments into useless crumbs.
//! - **Grow:** [`Heap::add_region`] adds a new range of memory to the list. The
//!   kernel calls it when the heap runs dry and it can map more pages.
//!
//! ## Block sizes
//!
//! Every block, and every allocation, is a multiple of 16 bytes, at an address that
//! is a multiple of 16. A free block must hold its own 16-byte list node, so 16 is
//! the smallest block. Rounding everything to 16 means a leftover piece is never
//! smaller than a node: it is either zero or at least 16. (A request for 1 byte
//! therefore costs 16.)
//!
//! ## Detecting misuse
//!
//! Freeing a block that overlaps a block that is already free is detected and
//! returned as an error: a double free, a free of a pointer that was never
//! allocated, or a free with the wrong size all show up as overlap with the free
//! list. **Not** detected: freeing a pointer that points into the *middle* of a
//! live allocation (it overlaps nothing free), or freeing with a smaller size than
//! the allocation (the leftover is silently leaked from the caller's view and is
//! given back as free memory, which is also wrong). The kernel's global allocator
//! always passes the original layout, so these need a bug elsewhere.
//!
//! ## Safety
//!
//! This crate writes list nodes into the memory it manages, so it has `unsafe`
//! code, kept to a few small helpers. [`Heap::add_region`] and [`Heap::dealloc`]
//! are `unsafe` because the caller must vouch for the memory.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

use core::ptr::{self, NonNull};

/// Smallest block, and the granularity of every size and address: the size of a
/// list node on a 64-bit machine.
pub const GRANULE: usize = 16;

/// Why an operation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeapError {
    /// The address or size is not a multiple of [`GRANULE`], or the size is zero.
    Misaligned,
    /// The range overlaps a block that is already free.
    OverlapsFreeMemory,
    /// The range wraps around the end of the address space.
    OutOfRange,
}

/// A free-list node, stored at the start of every free block.
#[repr(C)]
struct Node {
    /// Size of this free block in bytes, including the node.
    size: usize,
    /// Next free block (higher address), or null.
    next: *mut Node,
}

const _: () = assert!(core::mem::size_of::<Node>() == GRANULE);

/// The heap. Create it empty, then give it memory with [`Heap::add_region`].
pub struct Heap {
    head: *mut Node,
    free_bytes: usize,
    free_blocks: usize,
    /// Total bytes ever added by `add_region`.
    region_bytes: usize,
}

// SAFETY: the raw pointers inside point into memory that the heap owns
// exclusively. Moving the `Heap` to another thread moves that ownership with it.
// It has no interior mutability, so it is not `Sync`; callers serialise access.
unsafe impl Send for Heap {}

/// Counters describing the heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// All memory ever added.
    pub total_bytes: usize,
    /// Memory currently on the free list.
    pub free_bytes: usize,
    /// Memory currently allocated (including 16-byte rounding).
    pub used_bytes: usize,
    /// Separate pieces on the free list. A rising count means fragmentation.
    pub free_blocks: usize,
    /// Size of the largest free block: the biggest request that can succeed.
    pub largest_free_block: usize,
}

/// Rounds `value` up to a multiple of `alignment` (a power of two), or `None` on
/// overflow.
fn align_up(value: usize, alignment: usize) -> Option<usize> {
    debug_assert!(alignment.is_power_of_two());
    value
        .checked_add(alignment - 1)
        .map(|rounded| rounded & !(alignment - 1))
}

/// The size and alignment a request is really served with.
fn adjust(size: usize, align: usize) -> Option<(usize, usize)> {
    if !align.is_power_of_two() {
        return None;
    }
    let size = align_up(size.max(GRANULE), GRANULE)?;
    Some((size, align.max(GRANULE)))
}

impl Heap {
    /// An empty heap with no memory. Every allocation fails until a region is
    /// added.
    pub const fn new() -> Self {
        Self {
            head: ptr::null_mut(),
            free_bytes: 0,
            free_blocks: 0,
            region_bytes: 0,
        }
    }

    /// Adds `size` bytes starting at `start` to the heap as free memory.
    ///
    /// A region next to existing free memory is merged with it.
    ///
    /// # Safety
    ///
    /// The `size` bytes at `start` must be valid for reads and writes for as long
    /// as the heap lives, owned by nobody else, and not overlap memory that has
    /// been handed out by this heap. (Overlap with *free* memory is detected and
    /// refused.)
    pub unsafe fn add_region(&mut self, start: *mut u8, size: usize) -> Result<(), HeapError> {
        let address = start as usize;
        if !address.is_multiple_of(GRANULE) || size < GRANULE || !size.is_multiple_of(GRANULE) {
            return Err(HeapError::Misaligned);
        }
        // SAFETY: forwarded from this function's contract: the range is valid,
        // exclusively ours, and 16-byte aligned with a size that is a multiple
        // of 16 (checked above).
        unsafe { self.insert(address, size)? };
        self.region_bytes += size;
        Ok(())
    }

    /// Allocates `size` bytes aligned to `align` (a power of two). Returns `None`
    /// if no free block is big enough. The memory is not cleared.
    pub fn alloc(&mut self, size: usize, align: usize) -> Option<NonNull<u8>> {
        let (size, align) = adjust(size, align)?;

        let mut previous: *mut Node = ptr::null_mut();
        let mut current = self.head;
        // SAFETY: every node on the list lies in memory the heap owns, was
        // written by this crate, and is valid to read and write. The loop only
        // follows `next` pointers that this crate stored.
        unsafe {
            while !current.is_null() {
                let block_start = current as usize;
                let block_size = (*current).size;
                let next = (*current).next;
                let block_end = block_start + block_size;

                if let Some(start) = align_up(block_start, align) {
                    if let Some(end) = start.checked_add(size) {
                        if end <= block_end {
                            // Cut `start..end` out of this block. Because block
                            // starts, sizes and `align` are all multiples of 16,
                            // the front and back leftovers are each zero or at
                            // least 16 bytes: big enough to be a block.
                            let front = start - block_start;
                            let back = block_end - end;
                            debug_assert!(
                                front.is_multiple_of(GRANULE) && back.is_multiple_of(GRANULE)
                            );

                            // Rebuild the chain: previous -> [front] -> [back] -> next.
                            let mut tail = next;
                            let mut blocks_added = 0;
                            if back > 0 {
                                let node = end as *mut Node;
                                node.write(Node {
                                    size: back,
                                    next: tail,
                                });
                                tail = node;
                                blocks_added += 1;
                            }
                            let mut head_of_remainder = tail;
                            if front > 0 {
                                let node = block_start as *mut Node;
                                node.write(Node {
                                    size: front,
                                    next: tail,
                                });
                                head_of_remainder = node;
                                blocks_added += 1;
                            }
                            if previous.is_null() {
                                self.head = head_of_remainder;
                            } else {
                                (*previous).next = head_of_remainder;
                            }
                            self.free_bytes -= size;
                            self.free_blocks = self.free_blocks - 1 + blocks_added;
                            return NonNull::new(start as *mut u8);
                        }
                    }
                }
                previous = current;
                current = next;
            }
        }
        None
    }

    /// Returns a block to the heap.
    ///
    /// Refuses, without changing anything, a block that overlaps free memory (a
    /// double free, a never-allocated pointer, or a wrong size) or that is
    /// misaligned.
    ///
    /// # Safety
    ///
    /// `pointer` and `size`/`align` must be exactly what a previous successful
    /// [`Heap::alloc`] on this heap was given and returned, and the block must not
    /// be used afterwards.
    pub unsafe fn dealloc(
        &mut self,
        pointer: NonNull<u8>,
        size: usize,
        align: usize,
    ) -> Result<(), HeapError> {
        let (size, _) = adjust(size, align).ok_or(HeapError::Misaligned)?;
        let address = pointer.as_ptr() as usize;
        if !address.is_multiple_of(GRANULE) {
            return Err(HeapError::Misaligned);
        }
        // SAFETY: forwarded from this function's contract: the block came from
        // `alloc` with the same layout, so it is valid, ours, 16-byte aligned and
        // a multiple of 16 long.
        unsafe { self.insert(address, size) }
    }

    /// Inserts the free block `address..address + size` in address order, merging
    /// it with touching neighbours.
    ///
    /// # Safety
    ///
    /// The range must be valid, exclusively owned memory, with `address` and
    /// `size` multiples of [`GRANULE`] and `size` at least [`GRANULE`].
    unsafe fn insert(&mut self, address: usize, size: usize) -> Result<(), HeapError> {
        let end = address.checked_add(size).ok_or(HeapError::OutOfRange)?;

        // SAFETY: list nodes are valid, as in `alloc`. The new node is written
        // only after the overlap checks pass, into memory the caller vouched for.
        unsafe {
            // Find the last node before `address` and the first node at or after.
            let mut previous: *mut Node = ptr::null_mut();
            let mut next = self.head;
            while !next.is_null() && (next as usize) < address {
                previous = next;
                next = (*next).next;
            }

            let previous_end = if previous.is_null() {
                0
            } else {
                previous as usize + (*previous).size
            };
            if !previous.is_null() && previous_end > address {
                return Err(HeapError::OverlapsFreeMemory);
            }
            if !next.is_null() && end > next as usize {
                return Err(HeapError::OverlapsFreeMemory);
            }

            let node = address as *mut Node;
            node.write(Node { size, next });
            if previous.is_null() {
                self.head = node;
            } else {
                (*previous).next = node;
            }
            self.free_bytes += size;
            self.free_blocks += 1;

            // Merge with the block after, if they touch.
            if !next.is_null() && end == next as usize {
                (*node).size += (*next).size;
                (*node).next = (*next).next;
                self.free_blocks -= 1;
            }
            // Merge with the block before, if they touch.
            if !previous.is_null() && previous_end == address {
                (*previous).size += (*node).size;
                (*previous).next = (*node).next;
                self.free_blocks -= 1;
            }
        }
        Ok(())
    }

    /// Current counters.
    pub fn stats(&self) -> Stats {
        let mut largest = 0;
        let mut current = self.head;
        // SAFETY: as in `alloc`: the list contains only valid nodes.
        unsafe {
            while !current.is_null() {
                largest = largest.max((*current).size);
                current = (*current).next;
            }
        }
        Stats {
            total_bytes: self.region_bytes,
            free_bytes: self.free_bytes,
            used_bytes: self.region_bytes - self.free_bytes,
            free_blocks: self.free_blocks,
            largest_free_block: largest,
        }
    }

    /// Walks the free list and checks its invariants:
    ///
    /// 1. blocks are in strictly ascending address order;
    /// 2. blocks do not overlap and never touch (touching blocks must be merged);
    /// 3. every block starts at a multiple of 16 and has a size that is a
    ///    non-zero multiple of 16;
    /// 4. the byte and block counters match what the walk finds.
    ///
    /// Costs time proportional to the number of free blocks.
    pub fn check_invariants(&self) -> Result<(), &'static str> {
        let mut bytes = 0usize;
        let mut blocks = 0usize;
        let mut previous_end: Option<usize> = None;
        let mut current = self.head;
        // SAFETY: as in `alloc`. The loop is bounded by `free_blocks + 1`, so a
        // corrupted cyclic list is reported instead of looping forever.
        unsafe {
            while !current.is_null() {
                if blocks > self.free_blocks {
                    return Err("free list is longer than the block counter, or cyclic");
                }
                let start = current as usize;
                let size = (*current).size;
                if !start.is_multiple_of(GRANULE) {
                    return Err("a free block is not 16-byte aligned");
                }
                if size < GRANULE || !size.is_multiple_of(GRANULE) {
                    return Err("a free block has an invalid size");
                }
                if let Some(end) = previous_end {
                    if start < end {
                        return Err("free blocks overlap or are out of order");
                    }
                    if start == end {
                        return Err("touching free blocks were not merged");
                    }
                }
                previous_end = Some(start + size);
                bytes += size;
                blocks += 1;
                current = (*current).next;
            }
        }
        if bytes != self.free_bytes {
            return Err("free_bytes does not match the free list");
        }
        if blocks != self.free_blocks {
            return Err("free_blocks does not match the free list");
        }
        Ok(())
    }
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::alloc::{alloc as std_alloc, dealloc as std_dealloc, Layout};
    use std::collections::BTreeMap;
    use std::vec::Vec;

    /// A page-aligned buffer from the host allocator, freed on drop.
    struct Buffer {
        pointer: *mut u8,
        layout: Layout,
    }

    impl Buffer {
        fn new(size: usize) -> Self {
            let layout = Layout::from_size_align(size, 4096).unwrap();
            // SAFETY: the layout has a non-zero size.
            let pointer = unsafe { std_alloc(layout) };
            assert!(!pointer.is_null());
            Self { pointer, layout }
        }
    }

    impl Drop for Buffer {
        fn drop(&mut self) {
            // SAFETY: allocated in `new` with the same layout.
            unsafe { std_dealloc(self.pointer, self.layout) };
        }
    }

    fn heap_over(buffer: &Buffer) -> Heap {
        let mut heap = Heap::new();
        // SAFETY: the buffer is valid, exclusively ours, 4096-aligned, and its
        // size is a multiple of 16.
        unsafe {
            heap.add_region(buffer.pointer, buffer.layout.size())
                .unwrap()
        };
        heap
    }

    fn alloc_ok(heap: &mut Heap, size: usize, align: usize) -> NonNull<u8> {
        let pointer = heap.alloc(size, align).expect("allocation failed");
        assert_eq!(
            pointer.as_ptr() as usize % align.max(GRANULE),
            0,
            "misaligned"
        );
        pointer
    }

    #[test]
    fn empty_heap_allocates_nothing() {
        let mut heap = Heap::new();
        assert!(heap.alloc(1, 1).is_none());
        assert_eq!(heap.stats().total_bytes, 0);
        heap.check_invariants().unwrap();
    }

    #[test]
    fn add_region_rejects_bad_ranges() {
        let buffer = Buffer::new(4096);
        let mut heap = Heap::new();
        // SAFETY (all calls below): they are refused before touching memory, or
        // point into the valid buffer.
        unsafe {
            assert_eq!(
                heap.add_region(buffer.pointer.add(8), 64),
                Err(HeapError::Misaligned)
            );
            assert_eq!(
                heap.add_region(buffer.pointer, 0),
                Err(HeapError::Misaligned)
            );
            assert_eq!(
                heap.add_region(buffer.pointer, 24),
                Err(HeapError::Misaligned)
            );
            assert_eq!(
                heap.add_region(buffer.pointer, 8),
                Err(HeapError::Misaligned)
            );
            heap.add_region(buffer.pointer, 1024).unwrap();
            // Overlaps the free region just added.
            assert_eq!(
                heap.add_region(buffer.pointer.add(512), 1024),
                Err(HeapError::OverlapsFreeMemory)
            );
        }
        assert_eq!(heap.stats().total_bytes, 1024);
        heap.check_invariants().unwrap();
    }

    #[test]
    fn small_requests_are_rounded_to_sixteen() {
        let buffer = Buffer::new(4096);
        let mut heap = heap_over(&buffer);
        let a = alloc_ok(&mut heap, 1, 1);
        let b = alloc_ok(&mut heap, 1, 1);
        assert_eq!(b.as_ptr() as usize - a.as_ptr() as usize, 16);
        assert_eq!(heap.stats().used_bytes, 32);
        heap.check_invariants().unwrap();
    }

    #[test]
    fn alignment_is_honoured_up_to_a_page() {
        let buffer = Buffer::new(64 * 1024);
        let mut heap = heap_over(&buffer);
        for align in [1, 2, 8, 16, 32, 64, 256, 4096] {
            let pointer = alloc_ok(&mut heap, 100, align);
            assert_eq!(pointer.as_ptr() as usize % align, 0);
            heap.check_invariants().unwrap();
        }
    }

    #[test]
    fn front_padding_becomes_a_free_block_not_a_leak() {
        let buffer = Buffer::new(16 * 1024);
        let mut heap = heap_over(&buffer);
        let _first = alloc_ok(&mut heap, 16, 16); // moves the free block off a page boundary
        let free_before = heap.stats().free_bytes;
        let aligned = alloc_ok(&mut heap, 64, 4096);
        // The padding in front of the aligned block must still be allocatable.
        assert_eq!(aligned.as_ptr() as usize % 4096, 0);
        let used = free_before - heap.stats().free_bytes;
        assert_eq!(
            used, 64,
            "only the request itself is used, padding stays free"
        );
        heap.check_invariants().unwrap();
    }

    #[test]
    fn freeing_everything_restores_one_whole_block() {
        let buffer = Buffer::new(32 * 1024);
        let mut heap = heap_over(&buffer);
        let mut live = Vec::new();
        for size in [10, 200, 31, 4000, 16, 1, 999, 64] {
            live.push((alloc_ok(&mut heap, size, 8), size));
        }
        // Free in an interleaved order to force merging in both directions.
        for index in [3, 0, 7, 1, 5, 2, 6, 4] {
            let (pointer, size) = live[index];
            // SAFETY: each pointer came from `alloc` with this size and is freed once.
            unsafe { heap.dealloc(pointer, size, 8).unwrap() };
            heap.check_invariants().unwrap();
        }
        let stats = heap.stats();
        assert_eq!(stats.free_bytes, stats.total_bytes);
        assert_eq!(stats.free_blocks, 1, "all free blocks should have merged");
        assert_eq!(stats.largest_free_block, stats.total_bytes);
    }

    #[test]
    fn double_free_is_detected_and_changes_nothing() {
        let buffer = Buffer::new(4096);
        let mut heap = heap_over(&buffer);
        let pointer = alloc_ok(&mut heap, 100, 8);
        // SAFETY: first free of a valid allocation.
        unsafe { heap.dealloc(pointer, 100, 8).unwrap() };
        let before = heap.stats();
        // SAFETY: deliberately wrong: the second free of the same block. The
        // heap must refuse it before writing anything.
        let result = unsafe { heap.dealloc(pointer, 100, 8) };
        assert_eq!(result, Err(HeapError::OverlapsFreeMemory));
        assert_eq!(heap.stats(), before);
        heap.check_invariants().unwrap();
    }

    #[test]
    fn freeing_with_a_size_that_reaches_into_free_memory_is_detected() {
        let buffer = Buffer::new(4096);
        let mut heap = heap_over(&buffer);
        let pointer = alloc_ok(&mut heap, 64, 8);
        // SAFETY: deliberately wrong size (too big): the block would extend into
        // the free block that follows. The heap must refuse it.
        let result = unsafe { heap.dealloc(pointer, 128, 8) };
        assert_eq!(result, Err(HeapError::OverlapsFreeMemory));
        heap.check_invariants().unwrap();
    }

    #[test]
    fn exhaustion_returns_none_and_recovers() {
        let buffer = Buffer::new(8 * 1024);
        let mut heap = heap_over(&buffer);
        let mut live = Vec::new();
        while let Some(pointer) = heap.alloc(48, 8) {
            live.push(pointer);
        }
        // Every byte is either handed out or still free, and what is left
        // is too small for another 48-byte request.
        let free = heap.stats().free_bytes;
        assert_eq!(free + live.len() * 48, 8 * 1024);
        assert!(free < 48);
        assert!(heap.alloc(48, 8).is_none());
        for pointer in live {
            // SAFETY: each came from `alloc(48, 8)` and is freed once.
            unsafe { heap.dealloc(pointer, 48, 8).unwrap() };
        }
        assert_eq!(heap.stats().free_blocks, 1);
        assert!(
            heap.alloc(8 * 1024, 8).is_some(),
            "the whole region is one block again"
        );
    }

    #[test]
    fn overflowing_requests_fail_cleanly() {
        let buffer = Buffer::new(4096);
        let mut heap = heap_over(&buffer);
        assert!(heap.alloc(usize::MAX, 8).is_none());
        assert!(heap.alloc(usize::MAX - 8, 16).is_none());
        assert!(
            heap.alloc(8, 3).is_none(),
            "alignment must be a power of two"
        );
        heap.check_invariants().unwrap();
    }

    #[test]
    fn a_region_added_next_to_free_memory_merges_with_it() {
        let buffer = Buffer::new(8192);
        let mut heap = Heap::new();
        // SAFETY: both halves lie inside the valid buffer and do not overlap.
        unsafe {
            heap.add_region(buffer.pointer, 4096).unwrap();
            heap.add_region(buffer.pointer.add(4096), 4096).unwrap();
        }
        assert_eq!(heap.stats().free_blocks, 1);
        assert!(
            heap.alloc(8000, 16).is_some(),
            "the two halves act as one block"
        );
    }

    /// A step of the model-based test.
    #[derive(Debug, Clone)]
    enum Op {
        Alloc { size: usize, align_shift: u32 },
        Free(usize),
    }

    fn op_strategy() -> impl Strategy<Value = Op> {
        prop_oneof![
            7 => (1usize..1500, 0u32..8).prop_map(|(size, align_shift)| Op::Alloc { size, align_shift }),
            5 => any::<usize>().prop_map(Op::Free),
        ]
    }

    /// One live allocation in the model.
    struct Live {
        pointer: NonNull<u8>,
        size: usize,
        align: usize,
        pattern: u8,
    }

    fn fill(live: &Live) {
        // SAFETY: the block is a live allocation of at least `size` bytes.
        unsafe { ptr::write_bytes(live.pointer.as_ptr(), live.pattern, live.size) };
    }

    fn intact(live: &Live) -> bool {
        // SAFETY: the block is a live allocation of at least `size` bytes.
        let bytes = unsafe { core::slice::from_raw_parts(live.pointer.as_ptr(), live.size) };
        bytes.iter().all(|b| *b == live.pattern)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        // The headline property: through any sequence of allocations and frees,
        // no two live allocations overlap, each one keeps its contents, every
        // pointer is aligned and inside the region, the invariants hold, and
        // freeing everything gives back one whole block.
        #[test]
        fn never_overlaps_never_corrupts_and_merges_back(
            ops in proptest::collection::vec(op_strategy(), 0..250)
        ) {
            const REGION: usize = 64 * 1024;
            let buffer = Buffer::new(REGION);
            let mut heap = heap_over(&buffer);
            let base = buffer.pointer as usize;
            let mut live: Vec<Live> = Vec::new();
            let mut next_pattern = 1u8;

            for op in ops {
                match op {
                    Op::Alloc { size, align_shift } => {
                        let align = 1usize << align_shift;
                        if let Some(pointer) = heap.alloc(size, align) {
                            let address = pointer.as_ptr() as usize;
                            prop_assert_eq!(address % align.max(GRANULE), 0);
                            let (rounded, _) = adjust(size, align).unwrap();
                            prop_assert!(address >= base && address + rounded <= base + REGION);
                            let item = Live { pointer, size, align, pattern: next_pattern };
                            next_pattern = next_pattern.wrapping_add(1).max(1);
                            fill(&item);
                            live.push(item);
                        }
                    }
                    Op::Free(index) => {
                        if !live.is_empty() {
                            let item = live.swap_remove(index % live.len());
                            prop_assert!(intact(&item), "contents of a live block were overwritten");
                            // SAFETY: allocated with this size and align, freed once.
                            prop_assert_eq!(unsafe { heap.dealloc(item.pointer, item.size, item.align) }, Ok(()));
                        }
                    }
                }
                prop_assert!(heap.check_invariants().is_ok(), "{:?}", heap.check_invariants());

                // No two live blocks overlap (compare neighbours in address order).
                let mut by_address: BTreeMap<usize, usize> = BTreeMap::new();
                for item in &live {
                    let rounded = adjust(item.size, item.align).unwrap().0;
                    by_address.insert(item.pointer.as_ptr() as usize, rounded);
                }
                let mut previous_end = 0usize;
                for (start, length) in &by_address {
                    prop_assert!(*start >= previous_end, "two live allocations overlap");
                    previous_end = start + length;
                }
                // Used bytes equal the sum of the live (rounded) blocks.
                let live_bytes: usize = by_address.values().sum();
                prop_assert_eq!(heap.stats().used_bytes, live_bytes);
            }

            // Every survivor is still intact, then everything is freed.
            for item in live.drain(..) {
                prop_assert!(intact(&item));
                // SAFETY: allocated with this size and align, freed once.
                prop_assert_eq!(unsafe { heap.dealloc(item.pointer, item.size, item.align) }, Ok(()));
            }
            let stats = heap.stats();
            prop_assert_eq!(stats.free_bytes, REGION);
            prop_assert_eq!(stats.free_blocks, 1);
            prop_assert!(heap.check_invariants().is_ok());
        }
    }
}
