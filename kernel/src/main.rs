//! # Durg kernel entry (M2: serial console and boot information)
//!
//! The bootloader loads this ELF and jumps to [`_start`]. The kernel brings up
//! the serial console, reads what the bootloader reports (memory map, HHDM
//! offset, RSDP address), prints it, and exits QEMU through the `isa-debug-exit`
//! device so a test can read pass or fail from the exit code.
//!
//! With the `selftest` feature it also runs the in-kernel tests. Nothing here
//! is a security property yet.

#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

#[macro_use]
mod console;

mod arch;
mod boot;
mod limine;
mod qemu;
#[cfg(feature = "selftest")]
mod selftest;

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

/// Kernel entry point, called by the bootloader.
///
/// The bootloader has already switched to 64-bit mode and set up a stack, so
/// plain Rust can run from the first instruction.
#[no_mangle]
extern "C" fn _start() -> ! {
    let uart_ok = arch::serial::init();

    if !limine::base_revision_supported() {
        // The bootloader did not recognise the kernel or speaks a protocol
        // revision we do not. Nothing else can be trusted.
        error!("Limine base revision not supported");
        qemu::exit(qemu::ExitCode::Failure);
    }

    println!("Durg kernel: boot ok");
    info!(
        "serial: COM1 16550 loopback self-test {}",
        if uart_ok { "passed" } else { "FAILED" }
    );
    boot::print_boot_info();
    after_boot()
}

/// What happens once boot information is printed.
#[cfg(feature = "selftest")]
fn after_boot() -> ! {
    selftest::run_all()
}

/// What happens once boot information is printed.
#[cfg(not(feature = "selftest"))]
fn after_boot() -> ! {
    qemu::exit(qemu::ExitCode::Success)
}

/// Set once a panic starts, so a panic while panicking cannot loop.
static PANICKING: AtomicBool = AtomicBool::new(false);

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    if PANICKING.swap(true, Ordering::SeqCst) {
        qemu::exit(qemu::ExitCode::Failure);
    }
    // `PanicInfo` prints the source location and the message.
    println!("PANIC: {info}");
    qemu::exit(qemu::ExitCode::Failure)
}
