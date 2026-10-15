//! # Durg kernel entry (M1: boot smoke test)
//!
//! Milestone M1 only proves that the toolchain, linker script, boot image and
//! CI hang together: the bootloader loads this ELF, jumps to [`_start`], and the
//! kernel prints one line to the serial port and exits QEMU through the
//! `isa-debug-exit` device so a test can read pass or fail from the exit code.
//!
//! Real console output (a proper 16550 driver), the memory map and the panic
//! diagnostics arrive in M2. Nothing here is a security property.

#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

mod arch;
mod limine;
mod qemu;

use core::panic::PanicInfo;

/// Line the boot test looks for on the serial port.
const BOOT_MARKER: &str = "Durg kernel: boot ok (M1)\n";

/// Kernel entry point, called by the bootloader.
///
/// The bootloader has already switched to 64-bit mode and set up a stack, so
/// plain Rust can run from the first instruction.
#[no_mangle]
extern "C" fn _start() -> ! {
    if !limine::base_revision_supported() {
        // The bootloader did not recognise the kernel or speaks a protocol
        // revision we do not. There is nothing safe to do except stop.
        arch::halt();
    }

    arch::serial::write_str(BOOT_MARKER);
    qemu::exit(qemu::ExitCode::Success)
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    // M1 keeps the panic path trivial. M2 adds the message and location.
    arch::serial::write_str("Durg kernel: PANIC\n");
    qemu::exit(qemu::ExitCode::Failure)
}
