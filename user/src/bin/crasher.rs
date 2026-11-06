//! Writes to a kernel address from ring 3, which the CPU refuses with a page
//! fault: the kernel ends this task and carries on. Used to check that `join`
//! reports a task ended by an exception differently from one that called `exit`.

#![no_std]
#![no_main]

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // SAFETY: deliberately not sound: this address is in the kernel half and the
    // write must fault. Nothing in this program depends on it succeeding.
    unsafe { core::ptr::write_volatile(0xffff_8000_0000_0000 as *mut u8, 1) };
    // Not reached; if the write somehow succeeded, say so with a distinctive code.
    durg_user::exit(77)
}
