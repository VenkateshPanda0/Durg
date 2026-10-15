//! # Durg kernel entry (M3: CPU structures and exceptions)
//!
//! The bootloader loads this ELF and jumps to [`_start`]. The kernel brings up
//! the serial console, loads its own GDT, TSS and IDT (so CPU exceptions print
//! diagnostics instead of triple-faulting), masks the legacy PIC, reads what
//! the bootloader reports (memory map, HHDM offset, ACPI tables), prints it, and
//! exits QEMU through the `isa-debug-exit` device so a test can read pass or
//! fail from the exit code.
//!
//! With the `selftest` feature it also runs the in-kernel tests. Nothing here
//! is a security property yet.

#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

#[macro_use]
mod console;

mod acpi;
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

    if !limine::bootloader_processed_tag() {
        // No Limine-protocol bootloader touched our tag, so none of the boot
        // information can be trusted.
        error!("not booted by a Limine-protocol bootloader");
        qemu::exit(qemu::ExitCode::Failure);
    }
    if limine::loaded_base_revision() < 4 {
        // Legitimate fallback, but ACPI is unreadable without revision 4.
        warn!(
            "bootloader loaded us as base revision {}, not 4: ACPI will be unavailable",
            limine::loaded_base_revision()
        );
    }

    println!("Durg kernel: boot ok");
    info!(
        "serial: COM1 16550 loopback self-test {}",
        if uart_ok { "passed" } else { "FAILED" }
    );

    // Load our descriptor tables before anything else can go wrong, so a fault
    // during the rest of boot is reported instead of resetting the machine.
    arch::x86_64::init();
    info!("cpu: GDT, TSS and IDT loaded, double fault on IST1 stack, legacy PIC masked");

    boot::print_boot_info();
    after_boot()
}

/// Trigger a double fault on purpose (the `df-test` variant).
#[cfg(feature = "df-test")]
fn after_boot() -> ! {
    println!("df-test: triggering a double fault");
    arch::x86_64::trap::trigger_double_fault()
}

/// Run the self-tests.
#[cfg(all(feature = "selftest", not(feature = "df-test")))]
fn after_boot() -> ! {
    selftest::run_all()
}

/// Nothing more to do yet: exit.
#[cfg(not(any(feature = "selftest", feature = "df-test")))]
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
