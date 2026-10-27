//! A task that holds no endpoint capability. It tries to send a message through
//! every slot of its capability space, and to receive on every slot: nothing it
//! does can reach another task, because it was given nothing to name one with.
//! Exits 0 if every attempt was refused, otherwise 1 plus the slot number.

#![no_std]
#![no_main]

use durg_user::{error_of, exit, message_label, recv, send};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    for slot in 0..140u64 {
        if error_of(send(slot, message_label(1, None), [slot; 3])).is_none() {
            exit(1 + slot)
        }
        if recv(slot, None).is_ok() {
            exit(1000 + slot)
        }
    }
    exit(0)
}
