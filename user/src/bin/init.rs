//! The root task: the first program the kernel starts. The kernel gives it a
//! capability space whose slot 0 names that space and whose slot 1 is an untyped
//! region of memory, and nothing else. It checks that it holds exactly that, makes
//! one object from the region to show the machinery works, and exits. (Later it
//! will build and distribute the capabilities of every other task.)

#![no_std]
#![no_main]

use durg_cap::{Kind, Rights};
use durg_syscall::{label, retype_arg};
use durg_user::{exit, invoke, print};

fn describe(kind: Kind, bits: u8) -> u64 {
    u64::from(kind as u8) | u64::from(Rights::ALL.bits()) << 8 | u64::from(bits) << 16
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    print("init: root task running in ring 3");
    let own_space = invoke(0, label::IDENTIFY, 0, 0, 0);
    let region = invoke(1, label::IDENTIFY, 0, 0, 0);
    let nothing_else = invoke(2, label::IDENTIFY, 0, 0, 0);
    if own_space != describe(Kind::CNode, 12) {
        print("init: slot 0 is not my capability space");
        exit(1)
    }
    if region != describe(Kind::Untyped, 22) {
        print("init: slot 1 is not a 4 MiB untyped region");
        exit(2)
    }
    if durg_user::error_of(nothing_else).is_none() {
        print("init: slot 2 should be empty");
        exit(3)
    }
    // One 4 KiB frame out of the region, into slot 2.
    if invoke(1, label::RETYPE, retype_arg(Kind::Frame as u8, 12, 1), 2, 0) != 0 {
        print("init: could not make a frame from my region");
        exit(4)
    }
    print("init: I hold my own space and 4 MiB of untyped memory; made a frame from it");
    exit(0)
}
