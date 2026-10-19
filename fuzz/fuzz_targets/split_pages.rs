//! Fuzzes `split_by_page` on ranges that were **never validated**: the case where
//! a caller forgets to call `validate_user_range` first. It must not panic or
//! loop, and what it yields must still be sane: pieces in order, non-empty, never
//! crossing a page, never adding up to more than the length asked for.

#![no_main]

use durg_syscall::{split_by_page, PAGE_SIZE};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: (u64, u16)| {
    let (pointer, length) = input;
    let length = u64::from(length);
    let mut total = 0u64;
    let mut previous_end: Option<u64> = None;
    for chunk in split_by_page(pointer, length) {
        assert!(chunk.length > 0);
        assert!(chunk.length <= PAGE_SIZE);
        // Never crosses a page boundary, even at the very top of the address space.
        assert_eq!(
            chunk.address / PAGE_SIZE,
            (chunk.address + (chunk.length - 1)) / PAGE_SIZE
        );
        if let Some(end) = previous_end {
            assert_eq!(chunk.address, end, "gap or overlap");
        }
        previous_end = chunk.address.checked_add(chunk.length);
        total += chunk.length;
    }
    assert!(total <= length);
});
