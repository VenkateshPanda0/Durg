//! x86-64 primitives and CPU setup: port I/O, descriptor tables, exceptions.
//!
//! Reference: Intel SDM Vol. 1 ch. 18 (I/O), Vol. 2 (instruction reference) and
//! Vol. 3 ch. 3 (segments), 6 (interrupts and exceptions), 7 (task management).

use core::arch::asm;
use core::cell::UnsafeCell;

pub mod gdt;
pub mod idt;
pub mod lapic;
pub mod pic;
pub mod trap;

/// `IA32_APIC_BASE`: local APIC enable bits and physical base address.
pub const IA32_APIC_BASE: u32 = 0x1b;
/// `IA32_PAT`: the eight page attribute table entries.
#[cfg(feature = "selftest")]
pub const IA32_PAT: u32 = 0x277;
/// `IA32_EFER`: long mode and no-execute enable bits.
const IA32_EFER: u32 = 0xc000_0080;
const EFER_NO_EXECUTE_ENABLE: u64 = 1 << 11;

/// Boot-time mutable data that the CPU itself reads (descriptor tables and the
/// TSS).
///
/// It is written only while the kernel runs on one CPU with interrupts
/// disabled, before the structure is loaded into the processor. After that the
/// hardware reads it, and Rust code only reads the address. `UnsafeCell`
/// makes the type honest about the memory changing underneath the compiler.
pub struct BootCell<T>(UnsafeCell<T>);

// SAFETY: see the type docs. Mutation is confined to single-CPU early boot, so
// no two threads of execution can access the cell at once.
unsafe impl<T> Sync for BootCell<T> {}

impl<T> BootCell<T> {
    pub const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    pub fn get(&self) -> *mut T {
        self.0.get()
    }
}

/// The 10-byte operand of `lgdt` and `lidt`: a limit and a linear base.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct DescriptorTablePointer {
    pub limit: u16,
    pub base: u64,
}

/// Brings up the CPU structures this milestone owns: masks the legacy PIC, then
/// loads our GDT/TSS and IDT. After this, CPU exceptions reach Rust handlers
/// that print diagnostics instead of triple-faulting.
pub fn init() {
    pic::disable();
    gdt::init();
    idt::init();
}

/// Writes `value` to I/O port `port`.
pub fn outb(port: u16, value: u8) {
    // SAFETY: `out` has no memory effects visible to Rust. It talks to a device
    // register, and the only ports we write are the QEMU debug-exit port, COM1
    // and the legacy PIC, all owned by this kernel. No Rust-visible state is
    // touched.
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
    // We read only COM1 and PIC registers, whose reads have no side effects
    // that Rust code depends on beyond the documented device behaviour.
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

/// Which interrupt-controller features the CPU reports (CPUID leaf 1).
///
/// Reference: Intel SDM Vol. 2 (`CPUID`, leaf 01H): EDX bit 9 is the on-chip
/// local APIC and ECX bit 21 is x2APIC support.
pub struct ApicSupport {
    pub local_apic: bool,
    pub x2apic: bool,
}

/// Asks the CPU which local APIC modes it supports.
pub fn apic_support() -> ApicSupport {
    // `cpuid` is always available in 64-bit mode. The standard library routine
    // takes care of the `rbx` register that the compiler reserves.
    let leaf1 = core::arch::x86_64::__cpuid(1);
    ApicSupport {
        local_apic: leaf1.edx & (1 << 9) != 0,
        x2apic: leaf1.ecx & (1 << 21) != 0,
    }
}

/// Reads a model-specific register.
///
/// Only MSRs that exist on every x86-64 CPU this kernel targets may be read:
/// reading an unsupported MSR raises a general protection fault.
pub fn read_msr(msr: u32) -> u64 {
    let (low, high): (u32, u32);
    // SAFETY: `rdmsr` reads a register into edx:eax and has no memory effects.
    // The callers pass architectural MSRs (`IA32_APIC_BASE`, `IA32_EFER`,
    // `IA32_PAT`) that exist on all 64-bit x86 CPUs.
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    (u64::from(high) << 32) | u64::from(low)
}

/// `true` if the CPU has no-execute enabled (`EFER.NXE`). Page table entries may
/// only set the no-execute bit when this is true.
pub fn nx_enabled() -> bool {
    read_msr(IA32_EFER) & EFER_NO_EXECUTE_ENABLE != 0
}

/// The page table root register (CR3).
pub fn read_cr3() -> u64 {
    let value: u64;
    // SAFETY: reading CR3 has no side effects.
    unsafe {
        asm!("mov {}, cr3", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

/// Invalidates the TLB entry for the page containing `address`, so a changed
/// page table entry takes effect.
pub fn invlpg(address: u64) {
    // SAFETY: `invlpg` only drops a cached translation. The CPU refetches it
    // from the page tables on the next access.
    unsafe {
        asm!("invlpg [{}]", in(reg) address, options(nostack, preserves_flags));
    }
}

/// Disables maskable interrupts (`cli`).
pub fn disable_interrupts() {
    // SAFETY: masking interrupts is always sound. It changes the interrupt flag,
    // so `preserves_flags` is not claimed.
    unsafe {
        asm!("cli", options(nomem, nostack));
    }
}

/// Enables interrupts and halts until one arrives (`sti; hlt`). The `sti`
/// interrupt shadow guarantees an interrupt that is already pending still wakes
/// the `hlt`, so there is no lost-wakeup window between the two instructions.
pub fn enable_and_halt() {
    // SAFETY: as for `enable_interrupts`. The CPU resumes after the handler
    // returns.
    unsafe {
        asm!("sti", "hlt", options(nomem, nostack));
    }
}

/// The current stack pointer.
pub fn read_rsp() -> u64 {
    let value: u64;
    // SAFETY: reading RSP has no side effects.
    unsafe {
        asm!("mov {}, rsp", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

/// The faulting linear address of the most recent page fault (CR2).
pub fn read_cr2() -> u64 {
    let value: u64;
    // SAFETY: reading CR2 has no side effects.
    unsafe {
        asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

/// The current code segment selector.
#[cfg(feature = "selftest")]
pub fn read_cs() -> u16 {
    let value: u16;
    // SAFETY: reading a segment register has no side effects.
    unsafe {
        asm!("mov {0:x}, cs", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

/// The current task register (the TSS selector).
#[cfg(feature = "selftest")]
pub fn read_tr() -> u16 {
    let value: u16;
    // SAFETY: `str` stores the task register and has no side effects.
    unsafe {
        asm!("str {0:x}", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

/// The current IDT register contents.
#[cfg(feature = "selftest")]
pub fn read_idtr() -> DescriptorTablePointer {
    let mut pointer = DescriptorTablePointer { limit: 0, base: 0 };
    // SAFETY: `sidt` stores 10 bytes to the given address, which points at a
    // properly sized local.
    unsafe {
        asm!("sidt [{}]", in(reg) &mut pointer, options(nostack, preserves_flags));
    }
    pointer
}
