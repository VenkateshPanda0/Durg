//! Reports how much of the memory it was given is NOT zero. A fresh task must
//! start with zeroed bss and zeroed stack pages, whatever the frames held before.
//! Exit code: the number of non-zero bytes found in 64 KiB of bss and in the
//! three lowest of its four stack pages (0 means clean).

#![no_std]
#![no_main]

use durg_user::exit;

static mut CLEAN: [u8; 65536] = [0; 65536];

/// One past the last byte of user space; the stack grows down from here.
const STACK_TOP: usize = 0x0000_8000_0000_0000;
const PAGE: usize = 4096;
const STACK_PAGES: usize = 4;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut nonzero = 0u64;
    for index in 0..65536 {
        // SAFETY: CLEAN is this program's own bss, and `index` is in range.
        let byte =
            unsafe { core::ptr::read_volatile(core::ptr::addr_of!(CLEAN).cast::<u8>().add(index)) };
        if byte != 0 {
            nonzero += 1;
        }
    }
    // The top stack page is the one this code runs on, so scan only the three
    // below it.
    let bottom = STACK_TOP - STACK_PAGES * PAGE;
    for address in bottom..bottom + (STACK_PAGES - 1) * PAGE {
        // SAFETY: these stack pages are mapped readable and writable for this task.
        let byte = unsafe { core::ptr::read_volatile(address as *const u8) };
        if byte != 0 {
            nonzero += 1;
        }
    }
    exit(nonzero)
}
