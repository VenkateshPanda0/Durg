//! Interrupt Descriptor Table.
//!
//! The IDT maps each of the 256 interrupt vectors to a handler. Vectors 0-31
//! are CPU exceptions (divide error, page fault, ...); the rest are free for
//! hardware interrupts. Each 16-byte gate holds the handler address, the code
//! segment to run it in, an optional IST stack index, and type bits.
//!
//! References: Intel SDM Vol. 3 ch. 6.10 (IDT), 6.14.1 (64-bit gates).

use core::mem::size_of;

use super::gdt::{DOUBLE_FAULT_IST, KERNEL_CODE};
use super::trap::{
    spurious_stub_address, stub_address, timer_stub_address, EXCEPTION_VECTORS, SPURIOUS_VECTOR,
    TIMER_VECTOR,
};
use super::{BootCell, DescriptorTablePointer};

use core::arch::asm;

const ENTRIES: usize = 256;

/// Present, ring 0, 64-bit interrupt gate. An interrupt gate clears IF on
/// entry, so a handler cannot be interrupted by a maskable interrupt.
const INTERRUPT_GATE: u8 = 0x8e;

/// The vector of the double fault exception.
const DOUBLE_FAULT_VECTOR: usize = 8;

#[repr(C)]
#[derive(Clone, Copy)]
struct Gate {
    offset_low: u16,
    selector: u16,
    /// Bits 0-2: IST index (0 means "do not switch stacks").
    ist: u8,
    /// Present, DPL and gate type.
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    _reserved: u32,
}

const _: () = assert!(size_of::<Gate>() == 16);

impl Gate {
    const MISSING: Gate = Gate {
        offset_low: 0,
        selector: 0,
        ist: 0,
        type_attr: 0, // present bit clear: using it raises #NP
        offset_mid: 0,
        offset_high: 0,
        _reserved: 0,
    };

    fn interrupt(handler: u64, ist: u8) -> Gate {
        Gate {
            offset_low: handler as u16,
            selector: KERNEL_CODE,
            ist,
            type_attr: INTERRUPT_GATE,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            _reserved: 0,
        }
    }
}

static IDT: BootCell<[Gate; ENTRIES]> = BootCell::new([Gate::MISSING; ENTRIES]);

/// Linear address of the IDT, for checking `sidt`.
#[cfg(feature = "selftest")]
pub fn base() -> u64 {
    IDT.get() as u64
}

/// Size limit of the IDT as `lidt` encodes it (size in bytes minus one).
#[cfg(feature = "selftest")]
pub const LIMIT: u16 = (ENTRIES * size_of::<Gate>() - 1) as u16;

/// Installs a handler stub for every CPU exception and loads the IDT.
pub fn init() {
    // SAFETY: runs once on the only CPU with interrupts disabled, so nothing
    // else touches the table while it is written. The stub addresses come from
    // the assembly table in `trap`, where each entry is a valid code address.
    unsafe {
        let idt = &mut *IDT.get();
        for (vector, gate) in idt.iter_mut().take(EXCEPTION_VECTORS).enumerate() {
            // The double fault gets its own stack so it can run even when the
            // normal stack is what broke.
            let ist = if vector == DOUBLE_FAULT_VECTOR {
                DOUBLE_FAULT_IST
            } else {
                0
            };
            *gate = Gate::interrupt(stub_address(vector), ist);
        }
        // Hardware interrupts: the local APIC timer and the spurious vector.
        idt[usize::from(TIMER_VECTOR)] = Gate::interrupt(timer_stub_address(), 0);
        idt[usize::from(SPURIOUS_VECTOR)] = Gate::interrupt(spurious_stub_address(), 0);

        let pointer = DescriptorTablePointer {
            limit: (ENTRIES * size_of::<Gate>() - 1) as u16,
            base: IDT.get() as u64,
        };
        // SAFETY: the pointer describes the table we just filled, which is a
        // static and never moves.
        asm!("lidt [{}]", in(reg) &pointer, options(readonly, nostack, preserves_flags));
    }
}
