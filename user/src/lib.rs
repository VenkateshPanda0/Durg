//! The small runtime every Durg user program links: the three system calls and a
//! panic handler. See `kernel/src/syscall.rs` and the `durg-syscall` crate for the
//! kernel side. The call table is provisional (not the M10 capability ABI).
//!
//!   rax = number; rdi, rsi, rdx = arguments; result in rax. An error is the
//!   negated code, so it has the top bit set.

#![no_std]

use core::arch::asm;
use core::panic::PanicInfo;

const DEBUG_PRINT: u64 = 0;
const YIELD: u64 = 1;
const EXIT: u64 = 2;

/// Most bytes one `print` may carry (the kernel refuses more).
pub const MAX_PRINT: usize = 256;

fn syscall(number: u64, a: u64, b: u64, c: u64) -> u64 {
    let result: u64;
    // SAFETY: `syscall` enters the kernel, which preserves every register except
    // rax (the result), rcx and r11 (the saved return address and flags); the
    // clobbers say so. The kernel validates every argument, so no choice of
    // arguments can harm this program's memory.
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") number => result,
            in("rdi") a,
            in("rsi") b,
            in("rdx") c,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    result
}

/// Writes `text` (at most [`MAX_PRINT`] bytes) to the console, tagged by the
/// kernel with this task's name. Returns the number of bytes written, or an error
/// value (top bit set).
pub fn print(text: &str) -> u64 {
    syscall(DEBUG_PRINT, text.as_ptr() as u64, text.len() as u64, 0)
}

/// Gives up the rest of this turn.
pub fn yield_now() {
    syscall(YIELD, 0, 0, 0);
}

/// Ends this task with `code`.
pub fn exit(code: u64) -> ! {
    syscall(EXIT, code, 0, 0);
    // The kernel never returns from `exit`. If it somehow did, do not run on.
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    print("user program panicked");
    exit(0xdead)
}

extern "C" {
    /// Linker-provided boundaries of the program's own loaded image. Code and
    /// read-only data are contiguous (`__image_start..__image_ro_end`); the data
    /// segment (`__image_data_start..__image_end`) starts at a fixed address
    /// further up, and **the gap between the two is not mapped**.
    static __image_start: u8;
    static __image_ro_end: u8;
    static __image_data_start: u8;
    static __image_end: u8;
}

/// The program's own loaded memory as two byte ranges: code plus constants, and
/// data plus bss. The stack is not included, and neither is the unmapped gap
/// between the two ranges.
pub fn image_ranges() -> [(usize, usize); 2] {
    [
        (
            core::ptr::addr_of!(__image_start) as usize,
            core::ptr::addr_of!(__image_ro_end) as usize,
        ),
        (
            core::ptr::addr_of!(__image_data_start) as usize,
            core::ptr::addr_of!(__image_end) as usize,
        ),
    ]
}

/// Whether `needle` occurs anywhere in `[start, end)`, reading it directly (the
/// range must be mapped, which the program's own image is).
///
/// # Safety
///
/// Every byte of `[start, end)` must be readable by this task.
pub unsafe fn contains(start: usize, end: usize, needle: &[u8]) -> bool {
    let mut at = start;
    while at + needle.len() <= end {
        let mut matches = true;
        for (offset, wanted) in needle.iter().enumerate() {
            // SAFETY: `at + offset < end`, and the caller vouched for the range.
            let byte = unsafe { core::ptr::read_volatile((at + offset) as *const u8) };
            if byte != *wanted {
                matches = false;
                break;
            }
        }
        if matches {
            return true;
        }
        at += 1;
    }
    false
}

/// What the two isolation programs do. They are the same code with different
/// secrets, which sit at the same virtual address in both.
pub mod isolation {
    use super::*;

    /// Bytes of private scratch each program fills with its own tag.
    pub const SCRATCH_BYTES: usize = 8192;

    /// Checks that nothing changes this program's memory while the other one runs,
    /// and that the other one's secret is nowhere in this program's memory.
    ///
    /// `secret` is this program's 16 bytes; the other program's differ only in the
    /// last byte, `other_tag`. Exit codes: 0 all good; 1 own secret changed; 2 own
    /// scratch changed; 3 the other's secret was found.
    ///
    /// # Safety
    ///
    /// `secret` and `scratch` must point to this program's own writable memory,
    /// `secret` 16 bytes and `scratch` [`SCRATCH_BYTES`] bytes.
    pub unsafe fn run(
        name: &str,
        secret: *mut [u8; 16],
        scratch: *mut [u8; SCRATCH_BYTES],
        own_tag: u8,
        other_tag: u8,
    ) -> ! {
        // SAFETY: the caller's contract.
        let original = unsafe { core::ptr::read_volatile(secret) };
        let scratch = scratch.cast::<u8>();
        for index in 0..SCRATCH_BYTES {
            // SAFETY: the caller's contract; `index` is inside this program's own
            // scratch. (A byte loop, not one big array write, which the compiler
            // would expand into thousands of instructions.)
            unsafe { core::ptr::write_volatile(scratch.add(index), own_tag) };
        }

        // Let the other program run, several times, over the same virtual
        // addresses. If the two address spaces overlapped anywhere, its writes
        // would land in this program's memory.
        for _ in 0..8 {
            yield_now();
        }

        // SAFETY: as above.
        let secret_now = unsafe { core::ptr::read_volatile(secret) };
        if secret_now != original {
            print("own secret was changed by someone else");
            exit(1);
        }
        for index in 0..SCRATCH_BYTES {
            // SAFETY: as above.
            if unsafe { core::ptr::read_volatile(scratch.add(index)) } != own_tag {
                print("own scratch was changed by someone else");
                exit(2);
            }
        }

        // Look for the other program's secret in everything this program can see.
        // The needle is built on the stack, outside the scanned image, from this
        // program's own secret with the last byte replaced.
        let mut needle = original;
        needle[15] = other_tag;
        for (start, end) in image_ranges() {
            // SAFETY: both ranges are mapped, readable parts of this program's
            // own image (the unmapped gap between them is not scanned).
            if unsafe { contains(start, end, &needle) } {
                print("found the other program's secret in my own memory");
                exit(3);
            }
        }

        print(name);
        exit(0)
    }
}
