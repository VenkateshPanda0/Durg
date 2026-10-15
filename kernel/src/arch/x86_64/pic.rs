//! Legacy 8259 programmable interrupt controller: remap, then mask everything.
//!
//! Firmware leaves the two cascaded 8259 PICs mapped onto vectors 0x08-0x0f,
//! which collide with CPU exception vectors. Even if we never want their
//! interrupts, a stray one (or a spurious one) would be misread as an
//! exception. The standard fix is to remap them to harmless vectors and mask all
//! their inputs. Modern interrupt delivery uses the local APIC and I/O APIC.
//!
//! Reference: Intel 8259A datasheet (initialisation command words ICW1-ICW4 and
//! operation command word OCW1, the mask register).

#[cfg(feature = "selftest")]
use super::inb;
use super::outb;

const MASTER_COMMAND: u16 = 0x20;
const MASTER_DATA: u16 = 0x21;
const SLAVE_COMMAND: u16 = 0xa0;
const SLAVE_DATA: u16 = 0xa1;

/// ICW1: start initialisation, and an ICW4 will follow.
const ICW1_INIT_WITH_ICW4: u8 = 0x11;
/// ICW4: 8086/88 mode.
const ICW4_8086: u8 = 0x01;

/// Vectors the PICs are moved to. Both are past the 32 exception vectors.
const MASTER_OFFSET: u8 = 0x20;
const SLAVE_OFFSET: u8 = 0x28;

/// Delays by writing to an unused port, so slow old chips see the commands
/// spaced out. Port 0x80 is the traditional POST-code port.
fn io_wait() {
    outb(0x80, 0);
}

/// Remaps the PICs away from the exception vectors and masks every input.
pub fn disable() {
    outb(MASTER_COMMAND, ICW1_INIT_WITH_ICW4);
    io_wait();
    outb(SLAVE_COMMAND, ICW1_INIT_WITH_ICW4);
    io_wait();
    outb(MASTER_DATA, MASTER_OFFSET);
    io_wait();
    outb(SLAVE_DATA, SLAVE_OFFSET);
    io_wait();
    outb(MASTER_DATA, 4); // ICW3: a slave is attached to input 2
    io_wait();
    outb(SLAVE_DATA, 2); // ICW3: this slave's cascade identity
    io_wait();
    outb(MASTER_DATA, ICW4_8086);
    io_wait();
    outb(SLAVE_DATA, ICW4_8086);
    io_wait();

    // OCW1: a set bit masks that input. 0xff masks all eight.
    outb(MASTER_DATA, 0xff);
    outb(SLAVE_DATA, 0xff);
}

/// The two mask registers, master first. Both read `0xff` once disabled.
#[cfg(feature = "selftest")]
pub fn masks() -> (u8, u8) {
    (inb(MASTER_DATA), inb(SLAVE_DATA))
}
