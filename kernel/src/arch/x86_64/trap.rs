//! CPU exception entry and diagnostics.
//!
//! When an exception happens, the CPU looks up the vector in the IDT and jumps
//! to our stub for it. The stub makes every exception look the same to Rust:
//! it pushes a dummy error code if the CPU did not push a real one, pushes the
//! vector number, saves every general-purpose register, and calls
//! [`exception_dispatch`] with a pointer to the saved state ([`TrapFrame`]).
//! When the handler returns, the stub restores the registers and executes
//! `iretq`.
//!
//! Stable Rust has no `extern "x86-interrupt"` ABI (ADR-0002 keeps us on
//! stable), so the entry code is a few lines of assembly.
//!
//! Which exceptions push an error code is fixed by the architecture (Intel SDM
//! Vol. 3 table 6-1): vectors 8, 10, 11, 12, 13, 14, 17, 21, 29 and 30.
//!
//! This module also supports the self-tests: a page-fault probe that survives
//! the fault, a deliberate breakpoint, and a deliberate double fault.

#[cfg(any(feature = "selftest", feature = "df-test"))]
use core::arch::asm;
use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::gdt;
use super::read_cr2;
use crate::qemu::{self, ExitCode};

/// Vectors 0-31 are CPU exceptions.
pub const EXCEPTION_VECTORS: usize = 32;

/// Interrupt vector the local APIC timer raises. Well above the 32 exception
/// vectors and the legacy PIC range the PICs were moved to (0x20-0x2f).
pub const TIMER_VECTOR: u8 = 64;
/// Interrupt vector for spurious local APIC interrupts.
pub const SPURIOUS_VECTOR: u8 = 255;

const VECTOR_BREAKPOINT: u64 = 3;
const VECTOR_DOUBLE_FAULT: u64 = 8;
const VECTOR_PAGE_FAULT: u64 = 14;

/// Everything the stub saves, in the order it sits on the stack (lowest address
/// first). The field order is a contract with the assembly below: change one
/// and you must change the other.
#[repr(C)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    /// Pushed by the stub.
    pub vector: u64,
    /// Pushed by the CPU, or a zero the stub pushed for exceptions without one.
    pub error_code: u64,
    // Pushed by the CPU on every exception.
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

// Entry stubs. One per exception vector, then a table of their addresses.
//
// Stack alignment: the CPU aligns RSP to 16 bytes before pushing its 5-word
// frame (40 bytes), then we push an error code (8) and a vector (8), then 15
// registers (120). That is 176 bytes, a multiple of 16, so RSP is 16-byte
// aligned at the `call`, as the System V ABI requires.
global_asm!(
    r#"
    .macro isr_noerr n
    .global isr_stub_\n
    isr_stub_\n:
        push 0
        push \n
        jmp isr_common
    .endm

    .macro isr_err n
    .global isr_stub_\n
    isr_stub_\n:
        push \n
        jmp isr_common
    .endm

    .section .text.isr,"ax"
    isr_noerr 0
    isr_noerr 1
    isr_noerr 2
    isr_noerr 3
    isr_noerr 4
    isr_noerr 5
    isr_noerr 6
    isr_noerr 7
    isr_err   8
    isr_noerr 9
    isr_err   10
    isr_err   11
    isr_err   12
    isr_err   13
    isr_err   14
    isr_noerr 15
    isr_noerr 16
    isr_err   17
    isr_noerr 18
    isr_noerr 19
    isr_noerr 20
    isr_err   21
    isr_noerr 22
    isr_noerr 23
    isr_noerr 24
    isr_noerr 25
    isr_noerr 26
    isr_noerr 27
    isr_noerr 28
    isr_err   29
    isr_err   30
    isr_noerr 31

    // Hardware interrupt vectors (no error code). The numbers must match
    // `TIMER_VECTOR` and `SPURIOUS_VECTOR` in this file.
    isr_noerr 64
    isr_noerr 255

    isr_common:
        push rax
        push rbx
        push rcx
        push rdx
        push rsi
        push rdi
        push rbp
        push r8
        push r9
        push r10
        push r11
        push r12
        push r13
        push r14
        push r15
        mov rdi, rsp
        cld
        call {dispatch}
        pop r15
        pop r14
        pop r13
        pop r12
        pop r11
        pop r10
        pop r9
        pop r8
        pop rbp
        pop rdi
        pop rsi
        pop rdx
        pop rcx
        pop rbx
        pop rax
        add rsp, 16
        iretq

    .section .rodata.isr,"a"
    .global isr_stub_table
    .balign 8
    isr_stub_table:
        .quad isr_stub_0,  isr_stub_1,  isr_stub_2,  isr_stub_3
        .quad isr_stub_4,  isr_stub_5,  isr_stub_6,  isr_stub_7
        .quad isr_stub_8,  isr_stub_9,  isr_stub_10, isr_stub_11
        .quad isr_stub_12, isr_stub_13, isr_stub_14, isr_stub_15
        .quad isr_stub_16, isr_stub_17, isr_stub_18, isr_stub_19
        .quad isr_stub_20, isr_stub_21, isr_stub_22, isr_stub_23
        .quad isr_stub_24, isr_stub_25, isr_stub_26, isr_stub_27
        .quad isr_stub_28, isr_stub_29, isr_stub_30, isr_stub_31
    .section .text
    "#,
    dispatch = sym exception_dispatch,
);

extern "C" {
    /// Addresses of the 32 entry stubs, defined in the assembly above.
    static isr_stub_table: [u64; EXCEPTION_VECTORS];

    // Entry stubs for the hardware interrupt vectors. They are only ever jumped
    // to by the CPU through the IDT, never called from Rust.
    fn isr_stub_64();
    fn isr_stub_255();
}

/// Address of the entry stub for the local APIC timer interrupt.
pub fn timer_stub_address() -> u64 {
    let stub: unsafe extern "C" fn() = isr_stub_64;
    stub as usize as u64
}

/// Address of the entry stub for spurious local APIC interrupts.
pub fn spurious_stub_address() -> u64 {
    let stub: unsafe extern "C" fn() = isr_stub_255;
    stub as usize as u64
}

/// Address of the entry stub for exception `vector` (0-31).
pub fn stub_address(vector: usize) -> u64 {
    // SAFETY: the table is a read-only static defined by the assembly above
    // with exactly `EXCEPTION_VECTORS` entries, and indexing is bounds checked.
    unsafe { isr_stub_table[vector] }
}

/// Architectural names of the exception vectors (Intel SDM Vol. 3 table 6-1).
const EXCEPTION_NAMES: [&str; EXCEPTION_VECTORS] = [
    "Divide Error",
    "Debug",
    "Non-Maskable Interrupt",
    "Breakpoint",
    "Overflow",
    "BOUND Range Exceeded",
    "Invalid Opcode",
    "Device Not Available",
    "Double Fault",
    "Coprocessor Segment Overrun",
    "Invalid TSS",
    "Segment Not Present",
    "Stack-Segment Fault",
    "General Protection",
    "Page Fault",
    "Reserved",
    "x87 Floating-Point Error",
    "Alignment Check",
    "Machine Check",
    "SIMD Floating-Point Exception",
    "Virtualization Exception",
    "Control Protection",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Reserved",
    "Hypervisor Injection",
    "VMM Communication",
    "Security Exception",
    "Reserved",
];

/// Number of breakpoint exceptions handled. A test reads it to prove the
/// handler ran and returned.
static BREAKPOINTS: AtomicU64 = AtomicU64::new(0);

/// Where a page fault should resume, or 0 if page faults are not expected. Set
/// only by [`probe_read`] around one deliberately faulting instruction.
static RESUME_RIP: AtomicU64 = AtomicU64::new(0);
static FAULT_TAKEN: AtomicBool = AtomicBool::new(false);
static FAULT_ADDRESS: AtomicU64 = AtomicU64::new(0);
static FAULT_ERROR: AtomicU64 = AtomicU64::new(0);

/// Set by [`trigger_double_fault`] so the handler knows the fault is a test.
static DOUBLE_FAULT_EXPECTED: AtomicBool = AtomicBool::new(false);

/// Set by [`trigger_stack_overflow`]: the double fault is expected, and it must
/// be traceable to the kernel stack's guard page.
static STACK_OVERFLOW_EXPECTED: AtomicBool = AtomicBool::new(false);

/// Called by the assembly stub for every exception.
extern "C" fn exception_dispatch(frame: &mut TrapFrame) {
    // An exception raised by a program in ring 3 is that program's problem, never
    // the kernel's. (A double fault is an abort and stays fatal.)
    if frame.vector < EXCEPTION_VECTORS as u64
        && frame.vector != VECTOR_DOUBLE_FAULT
        && frame.cs & 3 == 3
    {
        user_fault(frame);
    }
    match frame.vector {
        VECTOR_BREAKPOINT => {
            BREAKPOINTS.fetch_add(1, Ordering::Relaxed);
            println!("EXCEPTION: Breakpoint at {:#x} (handled)", frame.rip);
        }
        VECTOR_PAGE_FAULT => {
            let address = read_cr2();
            let resume = RESUME_RIP.load(Ordering::Relaxed);
            if resume != 0 {
                // An expected fault from `probe_read`: record it and continue
                // at the instruction after the faulting one.
                FAULT_ADDRESS.store(address, Ordering::Relaxed);
                FAULT_ERROR.store(frame.error_code, Ordering::Relaxed);
                FAULT_TAKEN.store(true, Ordering::Relaxed);
                frame.rip = resume;
            } else {
                fatal(frame);
            }
        }
        VECTOR_DOUBLE_FAULT => double_fault(frame),
        v if v == u64::from(TIMER_VECTOR) => {
            // A task that was asked to stop is ended here if it was running in
            // ring 3 when the tick arrived. The interrupt must still be
            // acknowledged, since this handler will not return.
            if frame.cs & 3 == 3 && crate::sched::current_kill_requested() {
                if let Some(apic) = super::lapic::get() {
                    apic.end_of_interrupt();
                }
                crate::sched::exit_killed();
            }
            super::lapic::on_timer_interrupt();
        }
        // A spurious interrupt is not a real one, so it needs no end-of-interrupt
        // signal and no action.
        v if v == u64::from(SPURIOUS_VECTOR) => {}
        _ => fatal(frame),
    }
}

/// Ends the task that raised an exception in ring 3, and tells the console why.
/// Never returns: the faulting thread is finished, and the scheduler moves on.
fn user_fault(frame: &TrapFrame) -> ! {
    let name = EXCEPTION_NAMES
        .get(frame.vector as usize)
        .copied()
        .unwrap_or("Unknown");
    println!(
        "[user {} {}] killed: {name} (vector {}), error code {:#x}, rip {:#x}",
        crate::sched::current_id(),
        crate::sched::current_name(),
        frame.vector,
        frame.error_code,
        frame.rip
    );
    if frame.vector == VECTOR_PAGE_FAULT {
        let [presence, access, mode, kind] = describe_page_fault(frame.error_code);
        println!(
            "  cr2 {:#x} ({presence}, {access}, {mode}, {kind})",
            read_cr2()
        );
    }
    crate::sched::exit_faulted(frame.vector, frame.error_code)
}

/// Describes the page fault error code bits (Intel SDM Vol. 3 fig. 6-9).
fn describe_page_fault(error: u64) -> [&'static str; 4] {
    [
        if error & 1 != 0 {
            "protection violation"
        } else {
            "page not present"
        },
        if error & 2 != 0 { "write" } else { "read" },
        if error & 4 != 0 {
            "user mode"
        } else {
            "supervisor mode"
        },
        if error & 16 != 0 {
            "instruction fetch"
        } else {
            "data access"
        },
    ]
}

/// Prints everything useful about an unexpected exception and stops the kernel.
fn fatal(frame: &TrapFrame) -> ! {
    crate::console::enter_panic_mode();
    report(frame);
    qemu::exit(ExitCode::Failure)
}

fn report(frame: &TrapFrame) {
    let name = EXCEPTION_NAMES
        .get(frame.vector as usize)
        .copied()
        .unwrap_or("Unknown");
    println!("EXCEPTION: {name} (vector {})", frame.vector);
    println!("  error code: {:#x}", frame.error_code);
    println!(
        "  rip: {:#018x}  cs: {:#06x}  rflags: {:#x}",
        frame.rip, frame.cs, frame.rflags
    );
    println!("  rsp: {:#018x}  ss: {:#06x}", frame.rsp, frame.ss);
    println!(
        "  rax: {:#018x}  rbx: {:#018x}  rcx: {:#018x}  rdx: {:#018x}",
        frame.rax, frame.rbx, frame.rcx, frame.rdx
    );
    println!(
        "  rsi: {:#018x}  rdi: {:#018x}  rbp: {:#018x}",
        frame.rsi, frame.rdi, frame.rbp
    );
    if frame.vector == VECTOR_PAGE_FAULT {
        let [presence, access, mode, kind] = describe_page_fault(frame.error_code);
        println!(
            "  cr2 (faulting address): {:#018x}  ({presence}, {access}, {mode}, {kind})",
            read_cr2()
        );
    }
}

/// Handles the double fault. It cannot be resumed (it is an abort), so this
/// either reports a test success or stops the kernel.
fn double_fault(frame: &TrapFrame) -> ! {
    crate::console::enter_panic_mode();
    report(frame);
    let (stack_start, stack_end) = gdt::double_fault_stack_range();
    // The handler's own frame address tells us which stack we are running on.
    let here = frame as *const TrapFrame as u64;
    let on_ist_stack = (stack_start..stack_end).contains(&here);
    println!(
        "  handler stack: {here:#x} ({} the IST1 stack {stack_start:#x}..{stack_end:#x})",
        if on_ist_stack { "inside" } else { "OUTSIDE" }
    );

    // A page fault while pushing the first exception frame leaves the faulting
    // address in CR2 and the stack pointer where the push was attempted. If
    // either lies in the guard page below the kernel stack, this is an overflow.
    let cr2 = read_cr2();
    let guard_hit = crate::mm::vm::in_stack_guard(cr2) || crate::mm::vm::in_stack_guard(frame.rsp);
    if guard_hit {
        let stack = crate::mm::vm::kernel_stack();
        println!(
            "kernel stack overflow detected: CR2 {cr2:#x}, RSP {:#x}, guard page {:#x}..{:#x}",
            frame.rsp, stack.guard, stack.bottom
        );
    }

    if on_ist_stack {
        if STACK_OVERFLOW_EXPECTED.load(Ordering::Relaxed) && guard_hit {
            println!("double fault handled on the IST stack");
            qemu::exit(ExitCode::Success)
        }
        if DOUBLE_FAULT_EXPECTED.load(Ordering::Relaxed) {
            println!("double fault handled on the IST stack");
            qemu::exit(ExitCode::Success)
        }
    }
    qemu::exit(ExitCode::Failure)
}

/// Executes `int3`. The handler counts it and returns.
#[cfg(feature = "selftest")]
pub fn breakpoint() {
    // SAFETY: `int3` raises a breakpoint exception, which our handler services
    // and returns from. It touches no memory beyond the stack push done by the
    // CPU.
    unsafe {
        asm!("int3", options(nomem));
    }
}

/// Number of breakpoint exceptions handled so far.
#[cfg(feature = "selftest")]
pub fn breakpoints_handled() -> u64 {
    BREAKPOINTS.load(Ordering::Relaxed)
}

/// A page fault caught by [`probe_read`].
#[cfg(feature = "selftest")]
#[derive(Debug)]
pub struct PageFault {
    pub address: u64,
    pub error_code: u64,
}

/// Reads one byte at `address`, returning the fault instead of crashing if the
/// address is not mapped.
///
/// `address` must be canonical: a non-canonical address raises a general
/// protection fault, which is not recoverable here.
#[cfg(feature = "selftest")]
pub fn probe_read(address: u64) -> Result<u8, PageFault> {
    let value: u8;
    FAULT_TAKEN.store(false, Ordering::Relaxed);
    // SAFETY: the instruction reads one byte at `address`. If the address is
    // unmapped, the page fault handler sees RESUME_RIP set and continues at
    // label 2, so the read either completes or the program skips it. The slot
    // pointer comes from a live static and the registers used are declared.
    unsafe {
        asm!(
            "lea {tmp}, [rip + 2f]",
            "mov qword ptr [{slot}], {tmp}",
            "mov {value}, byte ptr [{address}]",
            "2:",
            tmp = out(reg) _,
            slot = in(reg) RESUME_RIP.as_ptr(),
            address = in(reg) address,
            value = out(reg_byte) value,
            options(nostack),
        );
    }
    RESUME_RIP.store(0, Ordering::Relaxed);
    if FAULT_TAKEN.swap(false, Ordering::Relaxed) {
        Err(PageFault {
            address: FAULT_ADDRESS.load(Ordering::Relaxed),
            error_code: FAULT_ERROR.load(Ordering::Relaxed),
        })
    } else {
        Ok(value)
    }
}

/// Writes one byte at `address`, returning the fault instead of crashing if the
/// write is not allowed (not mapped, or mapped read-only).
///
/// `address` must be canonical.
#[cfg(feature = "selftest")]
pub fn probe_write(address: u64, value: u8) -> Result<(), PageFault> {
    FAULT_TAKEN.store(false, Ordering::Relaxed);
    // SAFETY: the store writes one byte at `address`. If the address is not
    // writable, the page fault handler sees RESUME_RIP set and continues at label
    // 2, skipping the store. Callers pass only addresses where a successful write
    // is harmless. The slot pointer comes from a live static and the registers
    // used are declared.
    unsafe {
        asm!(
            "lea {tmp}, [rip + 2f]",
            "mov qword ptr [{slot}], {tmp}",
            "mov byte ptr [{address}], {value}",
            "2:",
            tmp = out(reg) _,
            slot = in(reg) RESUME_RIP.as_ptr(),
            address = in(reg) address,
            value = in(reg_byte) value,
            options(nostack),
        );
    }
    RESUME_RIP.store(0, Ordering::Relaxed);
    if FAULT_TAKEN.swap(false, Ordering::Relaxed) {
        Err(PageFault {
            address: FAULT_ADDRESS.load(Ordering::Relaxed),
            error_code: FAULT_ERROR.load(Ordering::Relaxed),
        })
    } else {
        Ok(())
    }
}

/// Tries to execute code at `address`, returning the fault if the page is not
/// executable. Meant for addresses that must **not** be executable: if the jump
/// succeeds, the CPU runs whatever bytes are there.
///
/// `address` must be canonical.
#[cfg(feature = "selftest")]
pub fn probe_execute(address: u64) -> Result<(), PageFault> {
    FAULT_TAKEN.store(false, Ordering::Relaxed);
    // SAFETY: `jmp` to a non-executable page raises a page fault with the
    // instruction-fetch bit set, before any byte at `address` runs. The handler
    // sees RESUME_RIP set and continues at label 2. `jmp` pushes nothing, so the
    // stack is unchanged either way. Callers pass only non-executable addresses.
    unsafe {
        asm!(
            "lea {tmp}, [rip + 2f]",
            "mov qword ptr [{slot}], {tmp}",
            "jmp {address}",
            "2:",
            tmp = out(reg) _,
            slot = in(reg) RESUME_RIP.as_ptr(),
            address = in(reg) address,
            options(nostack),
        );
    }
    RESUME_RIP.store(0, Ordering::Relaxed);
    if FAULT_TAKEN.swap(false, Ordering::Relaxed) {
        Err(PageFault {
            address: FAULT_ADDRESS.load(Ordering::Relaxed),
            error_code: FAULT_ERROR.load(Ordering::Relaxed),
        })
    } else {
        Ok(())
    }
}

/// Overflows the kernel stack with recursion until it runs into the guard page.
///
/// The first access below the stack raises a page fault. Delivering it needs a
/// push onto the same exhausted stack, which faults again, and that becomes a
/// double fault on the IST1 stack, whose handler recognises the guard page.
#[cfg(feature = "so-test")]
pub fn trigger_stack_overflow() -> ! {
    STACK_OVERFLOW_EXPECTED.store(true, Ordering::Relaxed);
    recurse(0);
    // `recurse` only returns if the guard page failed to stop it.
    println!("so-test: the stack overflowed without faulting");
    qemu::exit(ExitCode::Failure)
}

/// One level of recursion with a 512-byte frame that the optimiser cannot drop.
///
/// It recurses without end on purpose: the guard page is what stops it.
#[cfg(feature = "so-test")]
#[allow(unconditional_recursion)]
#[inline(never)]
fn recurse(depth: u64) -> u64 {
    let frame = [depth as u8; 512];
    core::hint::black_box(&frame);
    let below = recurse(depth + 1);
    core::hint::black_box(below) + u64::from(frame[0])
}

/// Causes a double fault on purpose, to show it is delivered on its own stack.
///
/// The stack pointer is set to an unmapped address and `int3` is executed. The
/// CPU cannot push the exception frame, which raises a page fault. Delivering
/// that page fault needs the same broken stack and faults again. Two page
/// faults in a row become a double fault, which runs on the IST1 stack.
///
/// This stands in for a kernel stack overflow. A real overflow needs a guard
/// page below the stack, which arrives with paging control in M5.
#[cfg(feature = "df-test")]
pub fn trigger_double_fault() -> ! {
    DOUBLE_FAULT_EXPECTED.store(true, Ordering::Relaxed);
    // SAFETY: this deliberately destroys the stack pointer and never returns.
    // The double fault handler exits the VM, so the broken stack is never used
    // for normal code.
    unsafe {
        asm!("mov rsp, 0", "int3", options(noreturn));
    }
}
