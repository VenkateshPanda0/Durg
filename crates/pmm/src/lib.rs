//! # Bitmap physical frame allocator
//!
//! A *frame* is one 4 KiB page of physical memory. This crate hands frames out
//! and takes them back. It is the pure half of Durg's physical memory manager:
//! it never touches memory itself, never allocates, and contains no `unsafe`
//! code (`#![forbid(unsafe_code)]`). The kernel gives it a slice of `u64` words
//! to use as bitmap storage, and the host unit and property tests give it a
//! `Vec`.
//!
//! ## Two bitmaps, three states
//!
//! Every frame number below `frame_count` is in one of three states, kept in two
//! bitmaps (one bit per frame each):
//!
//! | `managed` | `free` | meaning |
//! |---|---|---|
//! | 0 | 0 | **not ours**: reserved, firmware, kernel, or unknown. Never handed out, never accepted back. |
//! | 1 | 1 | **free**: in the pool and available. |
//! | 1 | 0 | **allocated**: in the pool and handed out. |
//!
//! The state `managed = 0, free = 1` cannot occur. Keeping *managed* separate
//! from *free* is what lets [`BitmapAllocator::free`] refuse a frame the
//! allocator never owned. With a single bitmap, freeing a reserved frame (for
//! example one holding the kernel) would just clear a bit and silently put it up
//! for allocation.
//!
//! ## Invariants
//!
//! 1. `free` is a subset of `managed`.
//! 2. `free_count` equals the number of set bits in `free`.
//! 3. `managed_count` equals the number of set bits in `managed`.
//! 4. [`BitmapAllocator::alloc`] returns only frames that were free, and each
//!    frame at most once until it is freed.
//! 5. Bits at positions at or beyond `frame_count` are never set.
//!
//! [`BitmapAllocator::check_invariants`] verifies all of them, and the tests call
//! it after every operation.
//!
//! ## What it does not do
//!
//! No locking (the caller serialises access), no contiguous allocation, no
//! per-region zones, no huge-frame support. Those are added when something needs
//! them.

#![no_std]
#![forbid(unsafe_code)]

/// Bytes in one frame.
pub const FRAME_SIZE: u64 = 4096;

/// A physical frame number: the physical address divided by [`FRAME_SIZE`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct Frame(pub u64);

impl Frame {
    /// The frame containing physical address `address`.
    pub const fn containing(address: u64) -> Frame {
        Frame(address / FRAME_SIZE)
    }

    /// Physical address of the first byte of the frame.
    pub const fn start_address(self) -> u64 {
        self.0 * FRAME_SIZE
    }
}

/// Why the allocator could not be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitError {
    /// The storage slice is shorter than [`words_needed`] says.
    StorageTooSmall,
    /// `frame_count` is zero, so there is nothing to manage.
    NoFrames,
}

/// Why a range operation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeError {
    /// The range extends past `frame_count`.
    OutOfRange,
}

/// Why a frame could not be returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeError {
    /// The frame number is at or beyond `frame_count`.
    OutOfRange,
    /// The allocator never owned this frame (reserved, kernel, firmware, ...).
    NotManaged,
    /// The frame is already free: a double free.
    AlreadyFree,
}

/// Number of `u64` words of storage needed to manage `frame_count` frames: two
/// bitmaps of one bit per frame, each rounded up to whole words.
pub const fn words_needed(frame_count: u64) -> usize {
    2 * frame_count.div_ceil(64) as usize
}

/// The allocator. See the crate docs for the model.
pub struct BitmapAllocator<'a> {
    free: &'a mut [u64],
    managed: &'a mut [u64],
    frame_count: u64,
    free_count: u64,
    managed_count: u64,
    /// Word index where the next search starts. A hint only: the search wraps
    /// around, so it never causes a free frame to be missed.
    hint: usize,
}

impl<'a> BitmapAllocator<'a> {
    /// Creates an allocator for frame numbers `0..frame_count` using `storage`
    /// for its bitmaps. Every frame starts as "not ours": nothing can be
    /// allocated until a range is added with [`add_range`](Self::add_range).
    ///
    /// The storage is zeroed here, so it may hold garbage.
    pub fn new(storage: &'a mut [u64], frame_count: u64) -> Result<Self, InitError> {
        if frame_count == 0 {
            return Err(InitError::NoFrames);
        }
        let words = frame_count.div_ceil(64) as usize;
        if storage.len() < 2 * words {
            return Err(InitError::StorageTooSmall);
        }
        let (free, rest) = storage.split_at_mut(words);
        let managed = &mut rest[..words];
        free.fill(0);
        managed.fill(0);
        Ok(Self {
            free,
            managed,
            frame_count,
            free_count: 0,
            managed_count: 0,
            hint: 0,
        })
    }

    /// Number of frames the allocator can describe.
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Frames currently free.
    pub fn free_count(&self) -> u64 {
        self.free_count
    }

    /// Frames in the pool, free or allocated.
    pub fn managed_count(&self) -> u64 {
        self.managed_count
    }

    /// `true` if `frame` is in the pool (free or allocated).
    pub fn is_managed(&self, frame: Frame) -> bool {
        frame.0 < self.frame_count && get_bit(self.managed, frame.0)
    }

    /// `true` if `frame` is in the pool and free.
    pub fn is_free(&self, frame: Frame) -> bool {
        frame.0 < self.frame_count && get_bit(self.free, frame.0)
    }

    fn check_range(&self, start: Frame, count: u64) -> Result<u64, RangeError> {
        let end = start.0.checked_add(count).ok_or(RangeError::OutOfRange)?;
        if end > self.frame_count {
            return Err(RangeError::OutOfRange);
        }
        Ok(end)
    }

    /// Adds `count` frames starting at `start` to the pool as free. Frames
    /// already in the pool (free or allocated) are left alone, so overlapping
    /// calls are harmless. Returns how many frames were newly added.
    pub fn add_range(&mut self, start: Frame, count: u64) -> Result<u64, RangeError> {
        let end = self.check_range(start, count)?;
        let mut added = 0;
        for frame in start.0..end {
            if !get_bit(self.managed, frame) {
                set_bit(self.managed, frame);
                set_bit(self.free, frame);
                added += 1;
            }
        }
        self.managed_count += added;
        self.free_count += added;
        Ok(added)
    }

    /// Removes the **free** frames in the range from the pool for good: they
    /// become "not ours". Frames that are currently allocated are left alone
    /// (the owner must free them first). Returns how many frames were removed.
    pub fn reserve_range(&mut self, start: Frame, count: u64) -> Result<u64, RangeError> {
        let end = self.check_range(start, count)?;
        let mut removed = 0;
        for frame in start.0..end {
            if get_bit(self.free, frame) {
                clear_bit(self.free, frame);
                clear_bit(self.managed, frame);
                removed += 1;
            }
        }
        self.managed_count -= removed;
        self.free_count -= removed;
        Ok(removed)
    }

    /// The first run of `1 << log2_frames` consecutive free frames whose start
    /// is a multiple of its own length (a *naturally aligned block*), or `None`.
    /// Nothing is taken: pair it with [`reserve_range`](Self::reserve_range) to
    /// remove the block from the pool. The capability system uses this to find
    /// untyped regions, which must be aligned to their size.
    pub fn find_free_aligned_run(&self, log2_frames: u32) -> Option<Frame> {
        if log2_frames >= 63 {
            return None;
        }
        let len = 1u64 << log2_frames;
        let mut start = 0u64;
        while start.checked_add(len)? <= self.frame_count {
            // Find the last non-free frame in the block, if any, and skip past
            // it to the next aligned start.
            match (start..start + len).rev().find(|&f| !get_bit(self.free, f)) {
                None => return Some(Frame(start)),
                Some(blocked) => start = (blocked / len + 1) * len,
            }
        }
        None
    }

    /// Takes one free frame, or `None` if the pool is exhausted.
    pub fn alloc(&mut self) -> Option<Frame> {
        if self.free_count == 0 {
            return None;
        }
        let words = self.free.len();
        for step in 0..words {
            let index = (self.hint + step) % words;
            let word = self.free[index];
            if word != 0 {
                let bit = word.trailing_zeros() as u64;
                let frame = index as u64 * 64 + bit;
                self.free[index] &= !(1u64 << bit);
                self.free_count -= 1;
                self.hint = index;
                return Some(Frame(frame));
            }
        }
        // `free_count` is non-zero but no word has a set bit: the counters and
        // the bitmap disagree, which is a bug in this crate.
        unreachable!("free_count is non-zero but the free bitmap is empty");
    }

    /// Returns a frame to the pool.
    ///
    /// Fails, without changing anything, if the frame is out of range, was never
    /// in the pool, or is already free.
    pub fn free(&mut self, frame: Frame) -> Result<(), FreeError> {
        if frame.0 >= self.frame_count {
            return Err(FreeError::OutOfRange);
        }
        if !get_bit(self.managed, frame.0) {
            return Err(FreeError::NotManaged);
        }
        if get_bit(self.free, frame.0) {
            return Err(FreeError::AlreadyFree);
        }
        set_bit(self.free, frame.0);
        self.free_count += 1;
        Ok(())
    }

    /// Checks every invariant in the crate docs against the bitmaps. Costs time
    /// proportional to the storage, so it is for tests and boot-time checks.
    pub fn check_invariants(&self) -> Result<(), &'static str> {
        let mut free_bits = 0u64;
        let mut managed_bits = 0u64;
        for (index, (free, managed)) in self.free.iter().zip(self.managed.iter()).enumerate() {
            if free & !managed != 0 {
                return Err("a free frame is not managed");
            }
            free_bits += u64::from(free.count_ones());
            managed_bits += u64::from(managed.count_ones());

            // Bits past the last frame must never be set.
            let first = index as u64 * 64;
            if first + 64 > self.frame_count {
                let valid = self.frame_count.saturating_sub(first);
                let mask = if valid >= 64 {
                    u64::MAX
                } else {
                    (1u64 << valid) - 1
                };
                if (free | managed) & !mask != 0 {
                    return Err("a bit beyond frame_count is set");
                }
            }
        }
        if free_bits != self.free_count {
            return Err("free_count does not match the free bitmap");
        }
        if managed_bits != self.managed_count {
            return Err("managed_count does not match the managed bitmap");
        }
        Ok(())
    }
}

fn get_bit(bitmap: &[u64], frame: u64) -> bool {
    bitmap[(frame / 64) as usize] & (1u64 << (frame % 64)) != 0
}

fn set_bit(bitmap: &mut [u64], frame: u64) {
    bitmap[(frame / 64) as usize] |= 1u64 << (frame % 64);
}

fn clear_bit(bitmap: &mut [u64], frame: u64) {
    bitmap[(frame / 64) as usize] &= !(1u64 << (frame % 64));
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeSet;
    use std::vec;
    use std::vec::Vec;

    fn storage(frame_count: u64) -> Vec<u64> {
        // Garbage on purpose: `new` must zero it.
        vec![u64::MAX; words_needed(frame_count)]
    }

    #[test]
    fn frame_address_conversions() {
        assert_eq!(Frame::containing(0), Frame(0));
        assert_eq!(Frame::containing(4095), Frame(0));
        assert_eq!(Frame::containing(4096), Frame(1));
        assert_eq!(Frame(3).start_address(), 3 * 4096);
    }

    #[test]
    fn aligned_run_search_finds_the_first_whole_block() {
        let mut st = storage(64);
        let mut a = BitmapAllocator::new(&mut st, 64).unwrap();
        a.add_range(Frame(0), 64).unwrap();
        assert_eq!(a.find_free_aligned_run(3), Some(Frame(0)));
        // Take frame 5: blocks of 8 now start at 8, blocks of 4 still at 0.
        a.reserve_range(Frame(5), 1).unwrap();
        assert_eq!(a.find_free_aligned_run(3), Some(Frame(8)));
        assert_eq!(a.find_free_aligned_run(2), Some(Frame(0)));
        assert_eq!(a.find_free_aligned_run(0), Some(Frame(0)));
        // A block that ends exactly at the last frame counts.
        assert_eq!(
            a.find_free_aligned_run(6),
            None,
            "frame 5 is gone, so no 64-frame block"
        );
        let mut st2 = storage(64);
        let mut b = BitmapAllocator::new(&mut st2, 64).unwrap();
        b.add_range(Frame(32), 32).unwrap();
        assert_eq!(b.find_free_aligned_run(5), Some(Frame(32)));
        assert_eq!(b.find_free_aligned_run(4), Some(Frame(32)));
        // Nothing of 128 frames fits in 64.
        assert_eq!(a.find_free_aligned_run(7), None);
        assert_eq!(a.find_free_aligned_run(63), None);
        assert_eq!(a.find_free_aligned_run(200), None);
    }

    #[test]
    fn aligned_run_search_ignores_unmanaged_and_allocated_frames() {
        let mut st = storage(64);
        let mut a = BitmapAllocator::new(&mut st, 64).unwrap();
        a.add_range(Frame(16), 16).unwrap(); // only 16..32 managed
        assert_eq!(a.find_free_aligned_run(4), Some(Frame(16)));
        let taken = a.alloc().unwrap();
        assert!(taken.0 >= 16);
        assert_eq!(
            a.find_free_aligned_run(4),
            None,
            "an allocated frame blocks the block"
        );
        assert_eq!(a.find_free_aligned_run(3), Some(Frame(24)));
    }

    #[test]
    fn words_needed_rounds_up_per_bitmap() {
        assert_eq!(words_needed(1), 2);
        assert_eq!(words_needed(64), 2);
        assert_eq!(words_needed(65), 4);
        assert_eq!(words_needed(128), 4);
    }

    #[test]
    fn new_rejects_empty_and_undersized_storage() {
        let mut s = storage(100);
        assert!(matches!(
            BitmapAllocator::new(&mut s, 0),
            Err(InitError::NoFrames)
        ));
        let mut small = vec![0u64; words_needed(100) - 1];
        assert!(matches!(
            BitmapAllocator::new(&mut small, 100),
            Err(InitError::StorageTooSmall)
        ));
    }

    #[test]
    fn new_zeroes_garbage_storage_and_starts_with_nothing_allocatable() {
        let mut s = storage(200);
        let mut a = BitmapAllocator::new(&mut s, 200).unwrap();
        assert_eq!(a.free_count(), 0);
        assert_eq!(a.managed_count(), 0);
        assert_eq!(a.alloc(), None);
        a.check_invariants().unwrap();
    }

    #[test]
    fn allocates_every_added_frame_exactly_once_then_runs_dry() {
        let mut s = storage(130);
        let mut a = BitmapAllocator::new(&mut s, 130).unwrap();
        assert_eq!(a.add_range(Frame(10), 100).unwrap(), 100);
        let mut seen = BTreeSet::new();
        while let Some(frame) = a.alloc() {
            assert!((10..110).contains(&frame.0), "{frame:?} outside the range");
            assert!(seen.insert(frame), "{frame:?} handed out twice");
            a.check_invariants().unwrap();
        }
        assert_eq!(seen.len(), 100);
        assert_eq!(a.free_count(), 0);
        assert_eq!(a.alloc(), None);
    }

    #[test]
    fn range_ends_exactly_on_a_word_boundary_and_on_the_last_frame() {
        let mut s = storage(128);
        let mut a = BitmapAllocator::new(&mut s, 128).unwrap();
        assert_eq!(a.add_range(Frame(0), 128).unwrap(), 128);
        a.check_invariants().unwrap();
        assert_eq!(a.free_count(), 128);
        // A frame count that is not a multiple of 64.
        let mut s = storage(65);
        let mut a = BitmapAllocator::new(&mut s, 65).unwrap();
        assert_eq!(a.add_range(Frame(0), 65).unwrap(), 65);
        a.check_invariants().unwrap();
        let mut count = 0;
        while a.alloc().is_some() {
            count += 1;
        }
        assert_eq!(count, 65);
    }

    #[test]
    fn add_range_rejects_out_of_range_and_overflow() {
        let mut s = storage(100);
        let mut a = BitmapAllocator::new(&mut s, 100).unwrap();
        assert_eq!(a.add_range(Frame(90), 11), Err(RangeError::OutOfRange));
        assert_eq!(a.add_range(Frame(u64::MAX), 2), Err(RangeError::OutOfRange));
        assert_eq!(a.add_range(Frame(100), 1), Err(RangeError::OutOfRange));
        assert_eq!(a.managed_count(), 0, "a refused range must change nothing");
        // A zero-length range at the very end is fine.
        assert_eq!(a.add_range(Frame(100), 0), Ok(0));
    }

    #[test]
    fn overlapping_add_range_does_not_free_allocated_frames() {
        let mut s = storage(64);
        let mut a = BitmapAllocator::new(&mut s, 64).unwrap();
        a.add_range(Frame(0), 10).unwrap();
        let taken = a.alloc().unwrap();
        // Adding the same range again must not make `taken` free again.
        assert_eq!(a.add_range(Frame(0), 10).unwrap(), 0);
        assert!(!a.is_free(taken));
        assert!(a.is_managed(taken));
        a.check_invariants().unwrap();
    }

    #[test]
    fn free_returns_a_frame_and_double_free_is_an_error() {
        let mut s = storage(64);
        let mut a = BitmapAllocator::new(&mut s, 64).unwrap();
        a.add_range(Frame(0), 64).unwrap();
        let f = a.alloc().unwrap();
        assert_eq!(a.free(f), Ok(()));
        assert_eq!(a.free(f), Err(FreeError::AlreadyFree));
        assert_eq!(a.free_count(), 64);
        a.check_invariants().unwrap();
    }

    #[test]
    fn free_refuses_frames_the_allocator_never_owned() {
        let mut s = storage(64);
        let mut a = BitmapAllocator::new(&mut s, 64).unwrap();
        a.add_range(Frame(10), 20).unwrap();
        // Never added: a reserved or kernel frame must not become allocatable.
        assert_eq!(a.free(Frame(5)), Err(FreeError::NotManaged));
        assert_eq!(a.free(Frame(40)), Err(FreeError::NotManaged));
        assert!(!a.is_free(Frame(5)));
        assert_eq!(a.alloc().map(|f| f.0 >= 10 && f.0 < 30), Some(true));
        assert_eq!(a.free(Frame(64)), Err(FreeError::OutOfRange));
        assert_eq!(a.free(Frame(u64::MAX)), Err(FreeError::OutOfRange));
        a.check_invariants().unwrap();
    }

    #[test]
    fn reserve_removes_only_free_frames_for_good() {
        let mut s = storage(64);
        let mut a = BitmapAllocator::new(&mut s, 64).unwrap();
        a.add_range(Frame(0), 20).unwrap();
        let taken = a.alloc().unwrap(); // frame 0, allocated
        assert_eq!(
            a.reserve_range(Frame(0), 10).unwrap(),
            9,
            "the allocated frame is left alone"
        );
        assert!(a.is_managed(taken));
        assert!(!a.is_managed(Frame(5)));
        assert_eq!(a.free(Frame(5)), Err(FreeError::NotManaged));
        assert_eq!(a.managed_count(), 11);
        assert_eq!(a.free_count(), 10);
        a.check_invariants().unwrap();
        // Reserved frames are never handed out.
        while let Some(f) = a.alloc() {
            assert!(f.0 >= 10, "{f:?} was reserved");
        }
    }

    #[test]
    fn search_wraps_around_to_find_frames_freed_behind_the_hint() {
        let mut s = storage(256);
        let mut a = BitmapAllocator::new(&mut s, 256).unwrap();
        a.add_range(Frame(0), 256).unwrap();
        let mut all = Vec::new();
        while let Some(f) = a.alloc() {
            all.push(f);
        }
        assert_eq!(all.len(), 256);
        // The hint now points at the last word. Free a frame in the first word.
        a.free(Frame(3)).unwrap();
        assert_eq!(a.alloc(), Some(Frame(3)));
        assert_eq!(a.alloc(), None);
    }

    /// One step of the model-based test.
    #[derive(Debug, Clone)]
    enum Op {
        Add {
            start: u64,
            count: u64,
        },
        Reserve {
            start: u64,
            count: u64,
        },
        Alloc,
        /// Free the n-th live (allocated) frame, if any.
        FreeLive(usize),
        /// Free an arbitrary frame number, which may be bogus.
        FreeAny(u64),
    }

    fn op_strategy(frame_count: u64) -> impl Strategy<Value = Op> {
        // Ranges may overshoot `frame_count` slightly so the error path is hit.
        let span = frame_count + 8;
        prop_oneof![
            3 => (0..span, 0..span).prop_map(|(start, count)| Op::Add { start, count }),
            1 => (0..span, 0..span).prop_map(|(start, count)| Op::Reserve { start, count }),
            6 => Just(Op::Alloc),
            4 => any::<usize>().prop_map(Op::FreeLive),
            2 => (0..span).prop_map(Op::FreeAny),
        ]
    }

    /// A frame count (chosen to straddle the 64-frame word boundaries) together
    /// with a sequence of operations sized for it.
    fn scenario() -> impl Strategy<Value = (u64, Vec<Op>)> {
        prop_oneof![1u64..=70, 120u64..=140, 190u64..=200].prop_flat_map(|frame_count| {
            (
                Just(frame_count),
                proptest::collection::vec(op_strategy(frame_count), 0..300),
            )
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        // The headline property for M4: whatever sequence of operations runs, the
        // allocator agrees with a simple reference model, never hands out a frame
        // twice, and keeps every invariant.
        #[test]
        fn matches_a_reference_model_and_never_double_allocates(
            (frame_count, ops) in scenario()
        ) {
            run_model(frame_count, &ops)?;
        }
    }

    /// Runs `ops` on the real allocator and on a `BTreeSet` model, comparing after
    /// every step.
    fn run_model(frame_count: u64, ops: &[Op]) -> Result<(), TestCaseError> {
        let mut s = storage(frame_count);
        let mut a = BitmapAllocator::new(&mut s, frame_count).unwrap();
        let mut managed: BTreeSet<u64> = BTreeSet::new();
        let mut free: BTreeSet<u64> = BTreeSet::new();
        let mut live: Vec<u64> = Vec::new();

        for op in ops {
            match *op {
                Op::Add { start, count } => {
                    let in_range = start.checked_add(count).is_some_and(|e| e <= frame_count);
                    let result = a.add_range(Frame(start), count);
                    if in_range {
                        let mut added = 0;
                        for f in start..start + count {
                            if managed.insert(f) {
                                free.insert(f);
                                added += 1;
                            }
                        }
                        prop_assert_eq!(result, Ok(added));
                    } else {
                        prop_assert_eq!(result, Err(RangeError::OutOfRange));
                    }
                }
                Op::Reserve { start, count } => {
                    let in_range = start.checked_add(count).is_some_and(|e| e <= frame_count);
                    let result = a.reserve_range(Frame(start), count);
                    if in_range {
                        let mut removed = 0;
                        for f in start..start + count {
                            if free.remove(&f) {
                                managed.remove(&f);
                                removed += 1;
                            }
                        }
                        prop_assert_eq!(result, Ok(removed));
                    } else {
                        prop_assert_eq!(result, Err(RangeError::OutOfRange));
                    }
                }
                Op::Alloc => match a.alloc() {
                    Some(frame) => {
                        prop_assert!(free.remove(&frame.0), "{frame:?} was not free in the model");
                        prop_assert!(!live.contains(&frame.0), "{frame:?} is already allocated");
                        live.push(frame.0);
                    }
                    None => prop_assert!(free.is_empty(), "alloc failed with free frames left"),
                },
                Op::FreeLive(n) => {
                    if !live.is_empty() {
                        let frame = live.swap_remove(n % live.len());
                        prop_assert_eq!(a.free(Frame(frame)), Ok(()));
                        free.insert(frame);
                    }
                }
                Op::FreeAny(frame) => {
                    let result = a.free(Frame(frame));
                    if frame >= frame_count {
                        prop_assert_eq!(result, Err(FreeError::OutOfRange));
                    } else if !managed.contains(&frame) {
                        prop_assert_eq!(result, Err(FreeError::NotManaged));
                    } else if free.contains(&frame) {
                        prop_assert_eq!(result, Err(FreeError::AlreadyFree));
                    } else {
                        // A genuinely allocated frame: this is a legal free.
                        prop_assert_eq!(result, Ok(()));
                        live.retain(|f| *f != frame);
                        free.insert(frame);
                    }
                }
            }
            prop_assert_eq!(a.free_count(), free.len() as u64);
            prop_assert_eq!(a.managed_count(), managed.len() as u64);
            prop_assert!(a.check_invariants().is_ok(), "{:?}", a.check_invariants());
        }

        // Return everything: the pool must come back whole.
        for frame in live.drain(..) {
            prop_assert_eq!(a.free(Frame(frame)), Ok(()));
            free.insert(frame);
        }
        prop_assert_eq!(a.free_count(), managed.len() as u64);
        prop_assert!(a.check_invariants().is_ok());
        Ok(())
    }

    proptest! {
        /// The search returns exactly the first aligned all-free block a brute
        /// force scan finds, and taking it leaves the allocator consistent.
        #[test]
        fn aligned_run_search_matches_a_brute_force_scan(
            managed in prop::collection::vec((0u64..200, 1u64..40), 0..6),
            taken in prop::collection::vec(0u64..200, 0..30),
            log2 in 0u32..7,
        ) {
            let mut st = storage(200);
            let mut a = BitmapAllocator::new(&mut st, 200).unwrap();
            for (start, count) in managed {
                let _ = a.add_range(Frame(start), count.min(200 - start));
            }
            for t in taken {
                // Allocate-then-keep: reserve the frame so it is "not ours".
                let _ = a.reserve_range(Frame(t), 1);
            }
            let len = 1u64 << log2;
            let expect = (0..200u64).step_by(len as usize).find(|&start| {
                start + len <= 200 && (start..start + len).all(|f| a.is_free(Frame(f)))
            });
            let found = a.find_free_aligned_run(log2);
            prop_assert_eq!(found.map(|f| f.0), expect);
            if let Some(f) = found {
                prop_assert_eq!(f.0 % len, 0);
                let before = a.free_count();
                prop_assert_eq!(a.reserve_range(f, len).unwrap(), len);
                prop_assert_eq!(a.free_count(), before - len);
                prop_assert_eq!(a.check_invariants(), Ok(()));
            }
        }
    }
}
