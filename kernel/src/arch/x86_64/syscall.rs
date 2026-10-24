//! The `syscall` instruction: how ring 3 calls the kernel.
//!
//! `syscall` is a fast alternative to a software interrupt. In one instruction
//! the CPU saves the return address in `rcx` and the flags in `r11`, loads the
//! kernel code and stack-segment selectors, masks the flags in `IA32_FMASK`, and
//! jumps to the address in `IA32_LSTAR`. It does **not** touch the stack pointer:
//! on entry `rsp` is still the *user's*. Switching to a kernel stack is our
//! first job, and it is the dangerous part. This module is that entry stub.
//!
//! ## Entry
//!
//! 1. Interrupts are off (`IA32_FMASK` cleared the IF bit), so nothing can
//!    intervene between `syscall` and the stack switch.
//! 2. The user `rsp` is parked in a scratch variable and `rsp` becomes the top of
//!    the **current thread's kernel stack**. That address lives in
//!    [`KERNEL_STACK_TOP`], which the scheduler rewrites each time it switches to
//!    a thread that can run user code. One CPU means one such variable suffices;
//!    SMP would make it per-CPU (reached through `swapgs`).
//! 3. The user's registers, return address, flags and stack pointer are pushed
//!    as a [`SyscallFrame`], interrupts are turned back on (a long call must not
//!    stall preemption), and Rust's [`dispatch`] runs.
//!
//! ## Exit
//!
//! Interrupts off again, registers restored from the frame, then `rsp` set back
//! to the user stack **last** and `sysretq`. Between loading the user `rsp` and
//! `sysretq` no interrupt may arrive: it would run on the user's stack.
//!
//! Known `sysret` hazards, and what is done about them:
//!
//! - `sysretq` with a non-canonical return address faults in ring 0 *on the user
//!   stack* on Intel CPUs. The return address is whatever the CPU saved, which is
//!   canonical, and the handler never changes it, but [`dispatch`] still refuses
//!   to return to anything at or above the user-space boundary.
//! - Registers other than `rax`, `rcx` and `r11` come back exactly as the user
//!   left them; `rcx` and `r11` are the saved return address and flags, as the
//!   instruction defines.
//!
//! The selectors come from the GDT layout (see `gdt`): `syscall` loads
//! `CS = STAR[47:32]` and `sysret` loads `CS = STAR[63:48] + 16`,
//! `SS = STAR[63:48] + 8`.
//!
//! References: Intel SDM Vol. 2 (`SYSCALL`, `SYSRET`), Vol. 3 ch. 5.8.8.

use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};

use super::{gdt, read_msr, write_msr};

const IA32_EFER: u32 = 0xc000_0080;
const IA32_STAR: u32 = 0xc000_0081;
const IA32_LSTAR: u32 = 0xc000_0082;
const IA32_FMASK: u32 = 0xc000_0084;
/// `EFER.SCE`: enables `syscall` and `sysret`.
const EFER_SYSCALL_ENABLE: u64 = 1 << 0;

/// Flags cleared on `syscall` entry: IF (no interrupts until the stack is ours),
/// TF (no single-stepping the stub), DF (the ABI wants it clear), NT, and AC.
const FMASK: u64 = (1 << 9) | (1 << 8) | (1 << 10) | (1 << 14) | (1 << 18);

/// Top of the kernel stack the entry stub switches to. Written by the scheduler.
static KERNEL_STACK_TOP: AtomicU64 = AtomicU64::new(0);
/// Where the stub parks the user stack pointer for the instant between loading
/// the kernel stack and pushing it. Interrupts are off for that whole window.
static USER_RSP_SCRATCH: AtomicU64 = AtomicU64::new(0);

/// What the stub saves, lowest address first. The order is a contract with the
/// assembly below.
#[repr(C)]
pub struct SyscallFrame {
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
    /// On entry the call number; on return the result.
    pub rax: u64,
    /// Where `sysretq` will resume (the saved `rcx`).
    pub user_rip: u64,
    /// The flags `sysretq` will restore (the saved `r11`).
    pub user_rflags: u64,
    pub user_rsp: u64,
}

// Stack alignment: three words (24 bytes) and fifteen registers (120 bytes) make
// 144, a multiple of 16, so `rsp` is 16-byte aligned at the `call`, as the System
// V ABI requires, given a 16-byte-aligned stack top.
global_asm!(
    r#"
    .section .text.syscall,"ax"
    .global durg_syscall_entry
    durg_syscall_entry:
        mov [rip + {user_rsp}], rsp
        mov rsp, [rip + {kernel_top}]
        push qword ptr [rip + {user_rsp}]
        push r11
        push rcx
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
        sti
        call {dispatch}
        cli
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
        pop rcx
        pop r11
        pop rsp
        sysretq
    .section .text
    "#,
    user_rsp = sym USER_RSP_SCRATCH,
    kernel_top = sym KERNEL_STACK_TOP,
    dispatch = sym dispatch,
);

extern "C" {
    fn durg_syscall_entry();
}

/// Called by the stub with a pointer to the saved state.
extern "C" fn dispatch(frame: &mut SyscallFrame) {
    // Refuse to resume the task anywhere that is not user memory.
    if frame.user_rip >= durg_syscall::USER_SPACE_END {
        crate::syscall::kill_current("system call from a non-user return address");
    }
    frame.rax = crate::syscall::handle(
        frame.rax,
        [frame.rdi, frame.rsi, frame.rdx, frame.r10, frame.r8],
    );
}

/// Sets where the entry stub finds the kernel stack. Called with interrupts off
/// by the scheduler each time it switches to a thread with a kernel stack.
pub fn set_kernel_stack(top: u64) {
    KERNEL_STACK_TOP.store(top, Ordering::Release);
}

/// Whether the CPU supports `syscall`/`sysret` in long mode: CPUID leaf
/// 0x8000_0001, EDX bit 11.
fn cpu_supports_syscall() -> bool {
    // `cpuid` is always available in 64-bit mode.
    let max = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    max >= 0x8000_0001 && core::arch::x86_64::__cpuid(0x8000_0001).edx & (1 << 11) != 0
}

/// Why [`init`] could not enable `syscall`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitError {
    /// CPUID does not report `syscall`/`sysret`.
    Unsupported,
    /// A register did not read back the value written.
    ReadBackMismatch,
}

impl core::fmt::Display for InitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "the CPU does not support syscall/sysret",
            Self::ReadBackMismatch => "a syscall MSR did not keep the value written",
        })
    }
}

/// Enables `syscall` and points it at the entry stub.
pub fn init() -> Result<(), InitError> {
    if !cpu_supports_syscall() {
        return Err(InitError::Unsupported);
    }
    let star = (u64::from(gdt::STAR_SYSRET_BASE) << 48) | (u64::from(gdt::STAR_SYSCALL_BASE) << 32);
    let entry: unsafe extern "C" fn() = durg_syscall_entry;
    let lstar = entry as usize as u64;
    // SAFETY: the CPU reports `syscall` support (checked above), so these MSRs
    // exist. The values are the GDT's selector layout, the address of our entry
    // stub (valid kernel text), and a flag mask that only clears bits. The entry
    // stub switches to a kernel stack before using the stack.
    unsafe {
        write_msr(IA32_EFER, read_msr(IA32_EFER) | EFER_SYSCALL_ENABLE);
        write_msr(IA32_STAR, star);
        write_msr(IA32_LSTAR, lstar);
        write_msr(IA32_FMASK, FMASK);
    }
    if read_msr(IA32_STAR) != star
        || read_msr(IA32_LSTAR) != lstar
        || read_msr(IA32_FMASK) != FMASK
        || read_msr(IA32_EFER) & EFER_SYSCALL_ENABLE == 0
    {
        return Err(InitError::ReadBackMismatch);
    }
    Ok(())
}
