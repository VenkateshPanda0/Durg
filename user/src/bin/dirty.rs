//! Leaves a mess: fills its whole bss and most of its stack with a recognisable
//! non-zero byte, then exits. The next task that is given the same frames must
//! find them zeroed (see `probe`).

#![no_std]
#![no_main]

use durg_user::exit;

/// 64 KiB of bss.
static mut JUNK: [u8; 65536] = [0; 65536];

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let junk = core::ptr::addr_of_mut!(JUNK).cast::<u8>();
    for index in 0..65536 {
        // SAFETY: JUNK is this program's own writable bss and `index` is in range.
        unsafe { core::ptr::write_volatile(junk.add(index), 0xaa) };
    }
    // Dirty about three stack pages with a local array bigger than two pages.
    let mut stack_junk = [0u8; 10_000];
    for index in 0..stack_junk.len() {
        // SAFETY: `index` is in range of the local array; volatile so the
        // compiler keeps the stores.
        unsafe { core::ptr::write_volatile(stack_junk.as_mut_ptr().add(index), 0xbb) };
    }
    // Read one back so the array is observably used.
    // SAFETY: in range, as above.
    let last = unsafe { core::ptr::read_volatile(stack_junk.as_ptr().add(9_999)) };
    core::hint::black_box(last);
    exit(0)
}
