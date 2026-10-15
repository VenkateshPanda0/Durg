//! x86-64 primitives: port I/O and `hlt`.
//!
//! Reference: Intel SDM Vol. 1 ch. 18 (I/O) and Vol. 2 (`IN`/`OUT`/`HLT`).

use core::arch::asm;

/// Writes `value` to I/O port `port`.
pub fn outb(port: u16, value: u8) {
    // SAFETY: `out` has no memory effects visible to Rust. It talks to a device
    // register, and the only ports we write are the QEMU debug-exit port and
    // COM1, both owned by this kernel. No Rust-visible state is touched.
    unsafe {
        asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Halts the CPU until the next interrupt.
pub fn hlt() {
    // SAFETY: `hlt` only pauses the CPU. Interrupts are not enabled in M1, so
    // the CPU stays halted, which is the intent.
    unsafe {
        asm!("hlt", options(nomem, nostack, preserves_flags));
    }
}
