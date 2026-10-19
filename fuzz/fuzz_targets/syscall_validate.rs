//! Fuzzes everything a program controls at the syscall door: the call number,
//! the three argument registers, a pointer and a length. Every claim here is one
//! the kernel relies on:
//!
//! - decoding never panics and returns exactly the call the number names;
//! - validation agrees with an **independent oracle** on every input: a range is
//!   accepted exactly when it is short enough and lies entirely below the
//!   user-space boundary. The oracle uses 128-bit arithmetic (so it cannot wrap)
//!   and the boundary as a literal number (so it notices if the constant moves),
//!   which makes the check complete as well as sound;
//! - refusals carry the right error;
//! - a validated range splits into contiguous, non-empty, single-page pieces that
//!   add up to its length;
//! - the error return values round-trip and no other value decodes as an error.

#![no_main]

use durg_syscall::{
    decode, split_by_page, validate_user_range, Call, Error, MAX_PRINT, PAGE_SIZE, USER_SPACE_END,
};
use libfuzzer_sys::fuzz_target;

/// The first address that is not user memory, as a literal: the hardware's
/// 48-bit canonical lower half. Deliberately not `USER_SPACE_END`, so a change to
/// that constant is caught.
const BOUNDARY: u128 = 0x0000_8000_0000_0000;

/// Whether a range should be accepted, computed without any of the code under
/// test.
fn oracle(pointer: u64, length: u64, limit: u64) -> bool {
    length <= limit && (length == 0 || u128::from(pointer) + u128::from(length) <= BOUNDARY)
}

fuzz_target!(|input: (u64, [u64; 3], u64, u64, u32, u16)| {
    let (number, arguments, pointer, length, back, short) = input;
    assert_eq!(
        u128::from(USER_SPACE_END),
        BOUNDARY,
        "the boundary constant moved"
    );

    match decode(number, arguments) {
        Ok(Call::DebugPrint {
            pointer: p,
            length: l,
        }) => {
            assert_eq!(number, 0);
            assert_eq!((p, l), (arguments[0], arguments[1]));
        }
        Ok(Call::Yield) => assert_eq!(number, 1),
        Ok(Call::Exit { code }) => {
            assert_eq!(number, 2);
            assert_eq!(code, arguments[0]);
        }
        Err(error) => {
            assert_eq!(error, Error::NoSuchCall);
            assert!(number > 2);
        }
    }

    // Pointers on the interesting side of the boundary as well as anywhere: random
    // 64-bit values almost never land within a few bytes of it.
    let near = USER_SPACE_END.wrapping_sub(u64::from(back));
    let short = u64::from(short);

    // The kernel's limit, a limit equal to the length, and no limit at all.
    for (pointer, length, limit) in [
        (pointer, length, MAX_PRINT),
        (pointer, length, length),
        (pointer, length, u64::MAX),
        (near, short, MAX_PRINT),
        (near, short, u64::MAX),
        (near, u64::from(back), u64::MAX),
        (near, u64::from(back) + 1, u64::MAX),
    ] {
        assert_eq!(
            validate_user_range(pointer, length, limit).is_ok(),
            oracle(pointer, length, limit),
            "disagrees with the oracle: pointer {pointer:#x}, length {length:#x}, limit {limit:#x}"
        );
        match validate_user_range(pointer, length, limit) {
            Ok(()) if length > 0 => {
                let end = pointer
                    .checked_add(length)
                    .expect("accepted a wrapping range");
                assert!(
                    pointer < USER_SPACE_END,
                    "accepted a start in the kernel half"
                );
                assert!(end <= USER_SPACE_END, "accepted an end past the boundary");
                assert!(length <= limit);
            }
            Ok(()) => {}
            Err(Error::TooLong) => assert!(length > limit),
            Err(Error::BadAddress) => assert!(length > 0 && length <= limit),
            Err(Error::NoSuchCall) => panic!("validation reported an unknown call"),
        }
    }

    // Splitting, only for ranges the kernel would ever split: validated with its
    // limit. (An unbounded limit would make a range of 2^47 bytes, which would
    // take hours to walk and is not something the kernel does.)
    if validate_user_range(pointer, length, MAX_PRINT).is_ok() {
        let mut next = pointer;
        let mut total = 0u64;
        for chunk in split_by_page(pointer, length) {
            assert!(chunk.length > 0, "empty chunk");
            assert_eq!(chunk.address, next, "gap or overlap");
            assert_eq!(
                chunk.address / PAGE_SIZE,
                (chunk.address + chunk.length - 1) / PAGE_SIZE,
                "chunk crosses a page boundary"
            );
            assert!(chunk.address + chunk.length <= USER_SPACE_END);
            next = chunk.address + chunk.length;
            total += chunk.length;
        }
        assert_eq!(total, length);
    }

    // The error codec, on the fuzzed value as a would-be return value.
    if let Some(error) = Error::from_return_value(number) {
        assert_eq!(error.to_return_value(), number);
    }
    for error in [Error::NoSuchCall, Error::BadAddress, Error::TooLong] {
        assert_eq!(
            Error::from_return_value(error.to_return_value()),
            Some(error)
        );
    }
});
