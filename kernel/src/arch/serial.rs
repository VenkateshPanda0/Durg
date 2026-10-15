//! Minimal serial output on COM1, enough for the M1 boot test.
//!
//! This does **not** program the UART. It relies on the firmware and QEMU
//! leaving COM1 usable, which holds in the QEMU setup used here. M2 replaces it
//! with a proper 16550 driver (initialisation, line status polling).

use super::x86_64::outb;

/// I/O port of COM1's transmit register.
const COM1: u16 = 0x3f8;

/// Writes every byte of `s` to COM1.
pub fn write_str(s: &str) {
    for byte in s.bytes() {
        outb(COM1, byte);
    }
}
