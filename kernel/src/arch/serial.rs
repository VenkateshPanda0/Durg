//! 16550 UART driver for COM1.
//!
//! Register layout and bit meanings follow the National Semiconductor
//! PC16550D datasheet (register map and the line/modem/FIFO control register
//! descriptions). QEMU emulates this chip, and the initialisation below is
//! checked at boot by the UART's own loopback mode rather than assumed.
//!
//! Output is polled: the kernel waits until the transmit holding register is
//! empty before writing each byte. This module does no locking itself: the
//! console (`console.rs`) takes an interrupt-safe lock around each message, and
//! everything that prints goes through the console.

use core::sync::atomic::{AtomicBool, Ordering};

use super::x86_64::{inb, outb};

/// I/O base of COM1.
const COM1: u16 = 0x3f8;

// Register offsets from the base. Offsets 0 and 1 mean something else while the
// divisor latch access bit (DLAB) in the line control register is set.
const DATA: u16 = 0; // receive / transmit data, or divisor latch low
const INTERRUPT_ENABLE: u16 = 1; // or divisor latch high
const FIFO_CONTROL: u16 = 2;
const LINE_CONTROL: u16 = 3;
const MODEM_CONTROL: u16 = 4;
const LINE_STATUS: u16 = 5;

const LCR_DLAB: u8 = 1 << 7;
const LCR_8_BITS_NO_PARITY_1_STOP: u8 = 0b0000_0011;
const LSR_TRANSMIT_HOLDING_EMPTY: u8 = 1 << 5;
const FCR_ENABLE_AND_CLEAR_14_BYTE: u8 = 0xc7;
const MCR_LOOPBACK_TEST: u8 = 0x1e;
const MCR_NORMAL: u8 = 0x0f; // DTR, RTS, OUT1, OUT2

/// Divisor 1 gives 115200 baud (the UART's clock is 115200 x 16 Hz).
const BAUD_DIVISOR: u16 = 1;

/// Byte sent through the UART's internal loopback during self-test.
const LOOPBACK_BYTE: u8 = 0xae;

/// Upper bound on waiting for the transmitter, so an absent or stuck UART
/// cannot hang the kernel forever.
const TRANSMIT_SPIN_LIMIT: u32 = 100_000;

static LOOPBACK_PASSED: AtomicBool = AtomicBool::new(false);

/// Programs the UART: 115200 baud, 8 data bits, no parity, 1 stop bit, FIFOs on,
/// interrupts off. Then runs the loopback self-test.
///
/// Returns whether the self-test passed. Output is still attempted if it did
/// not, since the failure may be in the emulation of the test and not the
/// transmitter.
pub fn init() -> bool {
    outb(COM1 + INTERRUPT_ENABLE, 0x00); // no UART interrupts
    outb(COM1 + LINE_CONTROL, LCR_DLAB); // expose the baud divisor
    outb(COM1 + DATA, (BAUD_DIVISOR & 0xff) as u8);
    outb(COM1 + INTERRUPT_ENABLE, (BAUD_DIVISOR >> 8) as u8);
    outb(COM1 + LINE_CONTROL, LCR_8_BITS_NO_PARITY_1_STOP); // also clears DLAB
    outb(COM1 + FIFO_CONTROL, FCR_ENABLE_AND_CLEAR_14_BYTE);

    // Loopback: the transmitter is wired back to the receiver inside the chip,
    // so a byte we write must come straight back.
    outb(COM1 + MODEM_CONTROL, MCR_LOOPBACK_TEST);
    outb(COM1 + DATA, LOOPBACK_BYTE);
    let passed = inb(COM1 + DATA) == LOOPBACK_BYTE;

    outb(COM1 + MODEM_CONTROL, MCR_NORMAL);
    LOOPBACK_PASSED.store(passed, Ordering::Relaxed);
    passed
}

/// Whether the loopback self-test in [`init`] passed.
#[cfg(feature = "selftest")]
pub fn loopback_passed() -> bool {
    LOOPBACK_PASSED.load(Ordering::Relaxed)
}

fn write_byte(byte: u8) {
    for _ in 0..TRANSMIT_SPIN_LIMIT {
        if inb(COM1 + LINE_STATUS) & LSR_TRANSMIT_HOLDING_EMPTY != 0 {
            break;
        }
        core::hint::spin_loop();
    }
    outb(COM1 + DATA, byte);
}

/// Writes every byte of `s` to COM1.
pub fn write_str(s: &str) {
    for byte in s.bytes() {
        write_byte(byte);
    }
}
