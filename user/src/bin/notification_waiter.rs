//! Ring-3 notification waiter used to test delivery and cancellation.
#![no_std]
#![no_main]

use durg_user::{exit, notification_wait};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    match notification_wait(2) {
        Ok(badge) if badge == 1 << 63 => exit(0),
        Err(durg_syscall::Error::Aborted) => exit(1),
        _ => exit(2),
    }
}
