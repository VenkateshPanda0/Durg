//! Isolation program A. Its secret ends in 'A'; `task_b` is identical apart from that and has its
//! secret at the very same virtual address. See `durg_user::isolation`.

#![no_std]
#![no_main]

use durg_user::isolation::{run, SCRATCH_BYTES};

#[no_mangle]
static mut SECRET: [u8; 16] = *b"SECRET-OF-TASK-A";
static mut SCRATCH: [u8; SCRATCH_BYTES] = [0; SCRATCH_BYTES];

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // SAFETY: both statics are this program's own writable memory, of the sizes
    // `run` expects, and nothing else in this single-threaded program uses them.
    unsafe {
        run(
            "task A: memory isolated from the other task",
            core::ptr::addr_of_mut!(SECRET),
            core::ptr::addr_of_mut!(SCRATCH),
            b'A',
            b'B',
        )
    }
}
