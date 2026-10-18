//! # Syscall decoding and user-pointer validation
//!
//! A **system call** is how a program in ring 3 asks the kernel to do something.
//! Everything a user program passes in is untrusted: the call number, the
//! arguments, and above all any *pointer*. This crate is the pure half of the
//! syscall entry. It never touches memory and has no `unsafe` code
//! (`#![forbid(unsafe_code)]`). It answers three questions, so that the kernel
//! half is small and the answers can be property-tested and fuzzed on the host:
//!
//! 1. **Which call is this, with which arguments?** [`decode`]
//! 2. **Is this range of user memory one the program may name?**
//!    [`validate_user_range`]
//! 3. **How is a user range split into pieces that each lie in one page?**
//!    [`split_by_page`]. The kernel copies user bytes one page at a time by
//!    translating each page itself, so it never dereferences a user pointer
//!    directly.
//!
//! ## The calling convention (provisional)
//!
//! `rax` holds the call number, `rdi`, `rsi` and `rdx` the arguments, and the
//! result comes back in `rax`. A result with the top bit set is an error:
//! `-(code)` as a signed number (see [`Error`]). The table below is **not**
//! the capability ABI. That is an open decision (CLAUDE.md §11 item 6) and is
//! designed before M10. These three calls exist so that M8 can prove the ring-3
//! round trip.
//!
//! | number | name | arguments | meaning |
//! |---|---|---|---|
//! | 0 | `debug_print` | pointer, length | write up to [`MAX_PRINT`] bytes to the console |
//! | 1 | `yield` | none | give up the rest of the turn |
//! | 2 | `exit` | code | end the calling task |
//!
//! ## What "a user address" means
//!
//! The lower half of the 48-bit address space, `0 .. USER_SPACE_END`. Everything
//! from `USER_SPACE_END` up (the non-canonical hole and the whole kernel half)
//! is the kernel's, and a range that touches it is refused outright: this is
//! checked here, before any page table is consulted.

#![no_std]
#![forbid(unsafe_code)]

/// First address that is *not* user memory: the start of the non-canonical hole
/// between the lower and upper halves of the address space.
pub const USER_SPACE_END: u64 = 0x0000_8000_0000_0000;

/// The most bytes one `debug_print` may name.
pub const MAX_PRINT: u64 = 256;

/// Page size, for [`split_by_page`].
pub const PAGE_SIZE: u64 = 4096;

/// Syscall numbers.
pub mod number {
    pub const DEBUG_PRINT: u64 = 0;
    pub const YIELD: u64 = 1;
    pub const EXIT: u64 = 2;
}

/// A decoded call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    DebugPrint { pointer: u64, length: u64 },
    Yield,
    Exit { code: u64 },
}

/// Why a call was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum Error {
    /// The call number is not one we have.
    NoSuchCall = 1,
    /// A pointer range is not entirely user memory, or wraps around.
    BadAddress = 2,
    /// A length is over the limit for the call.
    TooLong = 3,
}

impl Error {
    /// The value returned to the program in `rax`: `-(code)` as a 64-bit
    /// two's-complement number.
    pub const fn to_return_value(self) -> u64 {
        (self as u64).wrapping_neg()
    }

    /// Inverse of [`Error::to_return_value`], for tests and user-side stubs.
    pub fn from_return_value(value: u64) -> Option<Error> {
        match value.wrapping_neg() {
            1 => Some(Error::NoSuchCall),
            2 => Some(Error::BadAddress),
            3 => Some(Error::TooLong),
            _ => None,
        }
    }
}

/// Turns the raw register values of a `syscall` into a [`Call`].
///
/// Arguments a call does not use are ignored. Pointers are **not** checked here,
/// only the number: [`validate_user_range`] does that, and the kernel calls it
/// when it is about to use the pointer.
pub fn decode(number: u64, arguments: [u64; 3]) -> Result<Call, Error> {
    match number {
        number::DEBUG_PRINT => Ok(Call::DebugPrint {
            pointer: arguments[0],
            length: arguments[1],
        }),
        number::YIELD => Ok(Call::Yield),
        number::EXIT => Ok(Call::Exit { code: arguments[0] }),
        _ => Err(Error::NoSuchCall),
    }
}

/// Checks that `length` bytes starting at `pointer` lie entirely in user memory
/// and that `length` is at most `limit`. A zero length is accepted and names no
/// memory.
///
/// Refuses, with no exceptions: a range whose end wraps past `u64::MAX`; a range
/// that starts in or runs into the kernel half or the non-canonical hole.
pub fn validate_user_range(pointer: u64, length: u64, limit: u64) -> Result<(), Error> {
    if length > limit {
        return Err(Error::TooLong);
    }
    if length == 0 {
        return Ok(());
    }
    // A range that wraps past u64::MAX fails here. A range that starts at or
    // above the boundary ends above it too (the length is not zero), so one
    // comparison on the end covers it.
    let end = pointer.checked_add(length).ok_or(Error::BadAddress)?;
    if end > USER_SPACE_END {
        return Err(Error::BadAddress);
    }
    Ok(())
}

/// One piece of a user range that lies within a single page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    /// First user address of the piece.
    pub address: u64,
    /// Length in bytes. Never zero, and never reaches into the next page.
    pub length: u64,
}

/// Splits `length` bytes at `pointer` into pieces that each stay inside one
/// 4 KiB page. The pieces are in address order and add up to `length`.
///
/// The range must already have passed [`validate_user_range`]. On a range that
/// would wrap past `u64::MAX` this stops at the wrap instead of panicking, so a
/// caller that forgot to validate gets a short result and not undefined
/// behaviour.
pub fn split_by_page(pointer: u64, length: u64) -> impl Iterator<Item = Chunk> {
    let mut address = pointer;
    let mut remaining = length;
    core::iter::from_fn(move || {
        if remaining == 0 {
            return None;
        }
        let room_in_page = PAGE_SIZE - (address % PAGE_SIZE);
        let take = remaining.min(room_in_page);
        let chunk = Chunk {
            address,
            length: take,
        };
        remaining -= take;
        match address.checked_add(take) {
            Some(next) => address = next,
            None => remaining = 0,
        }
        Some(chunk)
    })
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::vec::Vec;

    #[test]
    fn the_three_calls_decode() {
        assert_eq!(
            decode(0, [0x1000, 5, 99]),
            Ok(Call::DebugPrint {
                pointer: 0x1000,
                length: 5
            })
        );
        assert_eq!(decode(1, [7, 8, 9]), Ok(Call::Yield));
        assert_eq!(decode(2, [42, 0, 0]), Ok(Call::Exit { code: 42 }));
    }

    #[test]
    fn unknown_numbers_are_refused() {
        for number in [3, 4, 100, 1 << 32, u64::MAX] {
            assert_eq!(decode(number, [0; 3]), Err(Error::NoSuchCall), "{number}");
        }
    }

    #[test]
    fn error_return_values_have_the_top_bit_set_and_round_trip() {
        for error in [Error::NoSuchCall, Error::BadAddress, Error::TooLong] {
            let value = error.to_return_value();
            assert!(
                value >> 63 == 1,
                "{error:?} is not recognisable as an error"
            );
            assert_eq!(Error::from_return_value(value), Some(error));
        }
        assert_eq!(Error::from_return_value(0), None);
        assert_eq!(Error::from_return_value(5), None);
    }

    #[test]
    fn user_ranges_are_accepted_up_to_the_boundary_and_not_a_byte_past() {
        let limit = MAX_PRINT;
        assert_eq!(validate_user_range(0x1000, 16, limit), Ok(()));
        assert_eq!(validate_user_range(0, 1, limit), Ok(()));
        // The last byte of user space.
        assert_eq!(validate_user_range(USER_SPACE_END - 1, 1, limit), Ok(()));
        assert_eq!(
            validate_user_range(USER_SPACE_END - 16, 16, limit),
            Ok(()),
            "a range ending exactly at the boundary is fine"
        );
        assert_eq!(
            validate_user_range(USER_SPACE_END - 16, 17, limit),
            Err(Error::BadAddress),
            "one byte into the hole"
        );
        assert_eq!(
            validate_user_range(USER_SPACE_END, 1, limit),
            Err(Error::BadAddress)
        );
    }

    #[test]
    fn kernel_addresses_and_wraparound_are_refused() {
        let limit = MAX_PRINT;
        for pointer in [
            0xffff_8000_0000_0000,
            0xffff_ffff_8000_0000,
            u64::MAX,
            u64::MAX - 7,
        ] {
            assert_eq!(
                validate_user_range(pointer, 8, limit),
                Err(Error::BadAddress),
                "{pointer:#x}"
            );
        }
        // Wraps past u64::MAX back into the low addresses.
        assert_eq!(
            validate_user_range(u64::MAX - 3, 8, limit),
            Err(Error::BadAddress)
        );
    }

    /// The boundary is a number the hardware defines (the 48-bit canonical lower
    /// half), so it is pinned here as a literal. Tests that compare against the
    /// constant alone could not notice it being moved.
    #[test]
    fn the_boundary_and_the_addresses_around_it_are_pinned_to_literals() {
        assert_eq!(USER_SPACE_END, 0x0000_8000_0000_0000);
        // A limit that never gets in the way, so only the address matters.
        let limit = u64::MAX;
        assert_eq!(
            validate_user_range(0x0000_7fff_ffff_f000, 0x1000, limit),
            Ok(())
        );
        assert_eq!(
            validate_user_range(0x0000_8000_0000_0000, 1, limit),
            Err(Error::BadAddress)
        );
        // Inside the non-canonical hole.
        assert_eq!(
            validate_user_range(0x0000_ffff_0000_0000, 8, limit),
            Err(Error::BadAddress)
        );
        // The first kernel-half address.
        assert_eq!(
            validate_user_range(0xffff_8000_0000_0000, 8, limit),
            Err(Error::BadAddress)
        );
    }

    /// With a limit that does not stop it, a length near `u64::MAX` from a
    /// perfectly good user pointer wraps around. Random inputs almost never
    /// produce this combination, so it is a test of its own.
    #[test]
    fn a_huge_length_from_a_valid_pointer_is_refused_as_wrapping() {
        for pointer in [0x1000, 0x7fff_ffff_f000, 1] {
            for length in [
                u64::MAX,
                u64::MAX - 1,
                u64::MAX - 0x1000,
                u64::MAX - pointer + 1,
            ] {
                assert_eq!(
                    validate_user_range(pointer, length, u64::MAX),
                    Err(Error::BadAddress),
                    "pointer {pointer:#x} length {length:#x}"
                );
            }
        }
    }

    #[test]
    fn length_over_the_limit_is_too_long_and_zero_is_fine() {
        assert_eq!(
            validate_user_range(0x1000, MAX_PRINT + 1, MAX_PRINT),
            Err(Error::TooLong)
        );
        assert_eq!(validate_user_range(0x1000, MAX_PRINT, MAX_PRINT), Ok(()));
        // A zero-length range names nothing, even with a wild pointer.
        assert_eq!(validate_user_range(u64::MAX, 0, MAX_PRINT), Ok(()));
    }

    #[test]
    fn splitting_stops_at_page_boundaries() {
        let chunks: Vec<_> = split_by_page(0x1ff0, 0x30).collect();
        assert_eq!(
            chunks,
            [
                Chunk {
                    address: 0x1ff0,
                    length: 0x10
                },
                Chunk {
                    address: 0x2000,
                    length: 0x20
                }
            ]
        );
        assert_eq!(split_by_page(0x1000, 0).count(), 0);
        let whole: Vec<_> = split_by_page(0x1000, 0x2000).collect();
        assert_eq!(whole.len(), 2);
    }

    #[test]
    fn splitting_an_unvalidated_wrapping_range_does_not_panic() {
        let total: u64 = split_by_page(u64::MAX - 5, 100).map(|c| c.length).sum();
        assert!(total <= 100);
    }

    proptest! {
        // Whatever a program puts in the registers, decoding never panics and
        // only ever yields one of the three calls or "no such call".
        #[test]
        fn decode_total_over_all_inputs(number: u64, a: u64, b: u64, c: u64) {
            match decode(number, [a, b, c]) {
                Ok(Call::DebugPrint { pointer, length }) => {
                    prop_assert_eq!(number, 0);
                    prop_assert_eq!((pointer, length), (a, b));
                }
                Ok(Call::Yield) => prop_assert_eq!(number, 1),
                Ok(Call::Exit { code }) => {
                    prop_assert_eq!(number, 2);
                    prop_assert_eq!(code, a);
                }
                Err(error) => {
                    prop_assert_eq!(error, Error::NoSuchCall);
                    prop_assert!(number > 2);
                }
            }
        }

        // The security property: a range that validates never touches an
        // address at or above USER_SPACE_END, and never wraps.
        #[test]
        fn accepted_ranges_stay_inside_user_space(pointer: u64, length: u64, limit: u64) {
            if validate_user_range(pointer, length, limit).is_ok() && length > 0 {
                let end = pointer.checked_add(length);
                prop_assert!(end.is_some(), "accepted a wrapping range");
                prop_assert!(pointer < USER_SPACE_END);
                prop_assert!(end.unwrap() <= USER_SPACE_END);
                prop_assert!(length <= limit);
            }
        }

        // And the converse for the interesting region: everything entirely
        // inside user space and under the limit is accepted (no false refusals).
        #[test]
        fn ranges_inside_user_space_are_accepted(
            pointer in 0..USER_SPACE_END,
            length in 0..=MAX_PRINT,
        ) {
            prop_assume!(pointer + length <= USER_SPACE_END);
            prop_assert_eq!(validate_user_range(pointer, length, MAX_PRINT), Ok(()));
        }

        // Splitting partitions the range: pieces are contiguous, in order, sum
        // to the length, are never empty, and never cross a page boundary.
        #[test]
        fn chunks_partition_the_range_without_crossing_pages(
            pointer in 0..USER_SPACE_END,
            length in 0..=(8 * PAGE_SIZE),
        ) {
            prop_assume!(pointer + length <= USER_SPACE_END);
            let chunks: Vec<Chunk> = split_by_page(pointer, length).collect();
            let mut next = pointer;
            let mut total = 0;
            for chunk in &chunks {
                prop_assert!(chunk.length > 0);
                prop_assert_eq!(chunk.address, next, "gap or overlap");
                prop_assert_eq!(
                    chunk.address / PAGE_SIZE,
                    (chunk.address + chunk.length - 1) / PAGE_SIZE,
                    "chunk crosses a page boundary"
                );
                next = chunk.address + chunk.length;
                total += chunk.length;
            }
            prop_assert_eq!(total, length);
        }

        // Wrapping, aimed: a user pointer, a length close to u64::MAX, and a limit
        // that lets it through to the address check.
        #[test]
        fn huge_lengths_never_validate(
            pointer in 0..USER_SPACE_END,
            back in 0u64..=(1 << 40),
        ) {
            let length = u64::MAX - back;
            prop_assert!(validate_user_range(pointer, length, u64::MAX).is_err());
        }

        // The two functions together, on arbitrary input: whatever validates can
        // be split without panicking and stays in user space.
        #[test]
        fn validated_ranges_split_inside_user_space(pointer: u64, length: u64) {
            if validate_user_range(pointer, length, MAX_PRINT).is_ok() {
                for chunk in split_by_page(pointer, length) {
                    prop_assert!(chunk.address + chunk.length <= USER_SPACE_END);
                }
            }
        }
    }
}
