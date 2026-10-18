//! Global Descriptor Table and Task State Segment.
//!
//! In 64-bit mode segmentation is mostly switched off: base and limit of code
//! and data segments are ignored. A GDT is still required for the code segment
//! (which says "64-bit, ring 0"), and for a Task State Segment, whose
//! **Interrupt Stack Table** lets specific exceptions run on a known-good stack.
//!
//! Why this matters: a double fault usually means the normal stack is broken
//! (for example a kernel stack overflow). If the CPU tried to deliver the
//! double fault on that same stack, it would fault again and the machine would
//! triple-fault and reset with no message. Giving the double fault its own
//! stack through IST1 lets us print a diagnostic instead.
//!
//! References: Intel SDM Vol. 3 ch. 3.4.5 (segment descriptors), ch. 7.7 (TSS in
//! 64-bit mode), ch. 6.14.5 (IST).

use core::arch::asm;
use core::mem::size_of;

use super::{BootCell, DescriptorTablePointer};

/// Selector of the kernel code segment (GDT index 1, ring 0).
pub const KERNEL_CODE: u16 = 0x08;
/// Selector of the kernel data segment (GDT index 2, ring 0).
pub const KERNEL_DATA: u16 = 0x10;
/// Selector of the user data segment (GDT index 3, ring 3), with the requested
/// privilege level (the low two bits) set to 3.
pub const USER_DATA: u16 = 0x18 | 3;
/// Selector of the user code segment (GDT index 4, ring 3), RPL 3.
pub const USER_CODE: u16 = 0x20 | 3;
/// Selector of the TSS (GDT index 5, ring 0). The descriptor takes two slots.
pub const TSS_SELECTOR: u16 = 0x28;

/// The value for `IA32_STAR` bits 63:48, the base `sysret` computes the user
/// selectors from. In 64-bit mode `sysret` loads `SS = base + 8` and
/// `CS = base + 16`, both with RPL 3 (Intel SDM Vol. 2, `SYSRET`). That is why
/// user data comes *before* user code in the table, and why the two assertions
/// below exist: reordering the descriptors would make `sysret` load the wrong
/// segments.
pub const STAR_SYSRET_BASE: u16 = 0x10;
/// `syscall` loads `CS = STAR[47:32]` and `SS = that + 8` (Intel SDM Vol. 2,
/// `SYSCALL`): kernel code and data, in that order.
pub const STAR_SYSCALL_BASE: u16 = KERNEL_CODE;

const _: () = assert!(KERNEL_DATA == STAR_SYSCALL_BASE + 8);
const _: () = assert!(USER_DATA == (STAR_SYSRET_BASE + 8) | 3);
const _: () = assert!(USER_CODE == (STAR_SYSRET_BASE + 16) | 3);

/// Index (1-based, as the IDT gate field counts) of the IST entry used for the
/// double fault handler.
pub const DOUBLE_FAULT_IST: u8 = 1;

/// Size of the dedicated double-fault stack.
const IST_STACK_SIZE: usize = 16 * 1024;

#[repr(C, align(16))]
struct IstStack([u8; IST_STACK_SIZE]);

static DOUBLE_FAULT_STACK: BootCell<IstStack> = BootCell::new(IstStack([0; IST_STACK_SIZE]));

/// The 64-bit Task State Segment (Intel SDM Vol. 3 fig. 7-11). Packed to 4-byte
/// alignment because the layout places 64-bit fields at 4-byte offsets.
#[repr(C, packed(4))]
struct Tss {
    _reserved0: u32,
    /// Stack pointers for privilege levels 0-2. `rsp[0]` is where the CPU
    /// switches to when an interrupt or exception arrives from ring 3.
    rsp: [u64; 3],
    _reserved1: u64,
    /// Interrupt Stack Table. `ist[0]` is IST1.
    ist: [u64; 7],
    _reserved2: u64,
    _reserved3: u16,
    /// Offset of the I/O permission bitmap. Pointing it at the end of the TSS
    /// means "no bitmap".
    iomap_base: u16,
}

// The hardware layout is exactly 104 bytes. A wrong size would silently corrupt
// the descriptor limit.
const _: () = assert!(size_of::<Tss>() == 104);

static TSS: BootCell<Tss> = BootCell::new(Tss {
    _reserved0: 0,
    rsp: [0; 3],
    _reserved1: 0,
    ist: [0; 7],
    _reserved2: 0,
    _reserved3: 0,
    iomap_base: 0,
});

/// Slots in the table: null, kernel code, kernel data, user data, user code, and
/// a 16-byte TSS descriptor (two slots).
const GDT_SLOTS: usize = 7;

static GDT: BootCell<[u64; GDT_SLOTS]> = BootCell::new([0; GDT_SLOTS]);

/// Sets the stack the CPU switches to when an interrupt or exception arrives
/// while running in ring 3 (`TSS.RSP0`). The scheduler calls this each time it
/// switches to a thread that runs user code, so a timer tick or fault in ring 3
/// lands on *that thread's* kernel stack.
#[expect(
    dead_code,
    reason = "first caller is the user-thread entry code, a later M8 step"
)]
pub fn set_kernel_stack(top: u64) {
    // SAFETY: `TSS` is a live static. The field is only 4-byte aligned (the
    // struct is packed to 4), so the write must be unaligned. One CPU, and callers
    // run with interrupts off, so the CPU cannot read the field half written.
    unsafe {
        let field = core::ptr::addr_of_mut!((*TSS.get()).rsp[0]);
        field.write_unaligned(top);
    }
}

/// Address range of the double-fault stack, for checking that the handler really
/// runs on it.
pub fn double_fault_stack_range() -> (u64, u64) {
    let start = DOUBLE_FAULT_STACK.get() as u64;
    (start, start + IST_STACK_SIZE as u64)
}

/// Builds the two-slot 64-bit TSS descriptor (Intel SDM Vol. 3 fig. 7-4).
fn tss_descriptor(base: u64, limit: u32) -> [u64; 2] {
    let low = u64::from(limit & 0xffff)
        | ((base & 0x00ff_ffff) << 16)
        | (0x89u64 << 40) // present, DPL 0, type 0x9 = available 64-bit TSS
        | (u64::from((limit >> 16) & 0xf) << 48)
        | (((base >> 24) & 0xff) << 56);
    let high = base >> 32;
    [low, high]
}

/// Reads descriptor `index` of the table that is loaded now, for tests.
#[cfg(feature = "selftest")]
pub fn descriptor(index: usize) -> u64 {
    assert!(index < GDT_SLOTS);
    // SAFETY: the table is a live static with `GDT_SLOTS` entries and `index` is
    // in range; nothing writes it after `init`.
    unsafe { (*GDT.get())[index] }
}

/// Loads the GDT, reloads the segment registers and loads the task register.
pub fn init() {
    let stack_top = double_fault_stack_range().1;
    let tss_address = TSS.get() as u64;
    let [tss_low, tss_high] = tss_descriptor(tss_address, (size_of::<Tss>() - 1) as u32);

    // SAFETY: runs once, on the only CPU, before interrupts are enabled, so
    // nothing else can access these statics while we write them. The raw
    // pointers come from `BootCell::get` and point at live statics.
    unsafe {
        let tss = &mut *TSS.get();
        tss.ist[usize::from(DOUBLE_FAULT_IST) - 1] = stack_top;
        tss.iomap_base = size_of::<Tss>() as u16;

        let gdt = &mut *GDT.get();
        gdt[0] = 0;
        // Kernel code: access 0x9a (present, ring 0, code, readable), flags 0xa
        // (granularity 4K, long mode). Limit and base are ignored in 64-bit mode.
        gdt[1] = 0x00af_9a00_0000_ffff;
        // Kernel data: access 0x92 (present, ring 0, data, writable).
        gdt[2] = 0x00cf_9200_0000_ffff;
        // User data: access 0xf2 (present, ring 3, data, writable).
        gdt[3] = 0x00cf_f200_0000_ffff;
        // User code: access 0xfa (present, ring 3, code, readable), long mode.
        gdt[4] = 0x00af_fa00_0000_ffff;
        gdt[5] = tss_low;
        gdt[6] = tss_high;

        let pointer = DescriptorTablePointer {
            limit: (size_of::<[u64; GDT_SLOTS]>() - 1) as u16,
            base: GDT.get() as u64,
        };
        load(&pointer);
    }
}

/// Loads the table described by `pointer` and switches to the new selectors.
///
/// # Safety
///
/// `pointer` must describe a valid GDT whose entries match the selector
/// constants in this module, and the GDT and TSS must stay alive and unmoved.
unsafe fn load(pointer: &DescriptorTablePointer) {
    // SAFETY (whole block): the caller guarantees the table is valid. `lgdt`
    // only reads the pointer. The far return reloads CS with a selector that
    // refers to our 64-bit code descriptor, then data segment registers get
    // our data selector, and `ltr` loads a TSS descriptor that is present and
    // marked available.
    unsafe {
        asm!("lgdt [{}]", in(reg) pointer, options(readonly, nostack, preserves_flags));
        // `retfq` pops RIP then CS, so push CS first and the return address
        // second. This is the way to change CS in 64-bit mode.
        asm!(
            "push {cs}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            cs = in(reg) u64::from(KERNEL_CODE),
            tmp = lateout(reg) _,
            options(preserves_flags),
        );
        asm!(
            "mov ds, {sel:x}",
            "mov es, {sel:x}",
            "mov ss, {sel:x}",
            sel = in(reg) KERNEL_DATA,
            options(nostack, preserves_flags),
        );
        asm!("ltr {sel:x}", sel = in(reg) TSS_SELECTOR, options(nostack, preserves_flags));
    }
}
