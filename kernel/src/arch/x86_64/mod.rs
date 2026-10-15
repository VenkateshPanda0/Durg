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

/// Reads a byte from I/O port `port`.
pub fn inb(port: u16) -> u8 {
    let value: u8;
    // SAFETY: `in` only reads a device register and writes the result to `al`.
    // We read only COM1 registers, which have no side effects that Rust code
    // depends on beyond the documented UART behaviour.
    unsafe {
        asm!(
            "in al, dx",
            in("dx") port,
            out("al") value,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}

/// Halts the CPU until the next interrupt.
pub fn hlt() {
    // SAFETY: `hlt` only pauses the CPU. Interrupts are not enabled yet, so
    // the CPU stays halted, which is the intent.
    unsafe {
        asm!("hlt", options(nomem, nostack, preserves_flags));
    }
}
