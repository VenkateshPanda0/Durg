//! QEMU test support: leaving the VM with a status code.
//!
//! QEMU's `isa-debug-exit` device (`-device isa-debug-exit,iobase=0xf4,iosize=0x04`)
//! exits QEMU when a value is written to its port. The process exit status is
//! `(value << 1) | 1`, so a test harness can tell pass from fail. Values are
//! chosen to avoid QEMU's own codes (0 and 1).
//!
//! On real hardware or without the device, the write is harmless and we fall
//! through to halting.

use crate::arch;

/// I/O port of the `isa-debug-exit` device, matching the QEMU command line.
const DEBUG_EXIT_PORT: u16 = 0xf4;

/// Status the kernel reports to the test harness.
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum ExitCode {
    /// QEMU exits with status 33.
    Success = 0x10,
    /// QEMU exits with status 35.
    Failure = 0x11,
}

/// Exits QEMU with `code`, or halts if the device is absent.
pub fn exit(code: ExitCode) -> ! {
    arch::x86_64::outb(DEBUG_EXIT_PORT, code as u8);
    arch::halt()
}
