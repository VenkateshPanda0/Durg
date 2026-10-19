//! The root task: the first program the kernel starts. Until the capability
//! system exists (M10) there is nothing for a root task to distribute, so it only
//! announces itself and exits.

#![no_std]
#![no_main]

use durg_user::{exit, print};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    print("init: root task running in ring 3");
    exit(0)
}
