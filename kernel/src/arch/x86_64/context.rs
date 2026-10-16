//! Context switching: how the CPU moves from one thread to another.
//!
//! A *context switch* saves what the running thread needs to continue later,
//! then loads the same things for another thread. What a thread "is" to the CPU
//! is its stack and the registers. We do not copy registers into a structure.
//! We push them onto the thread's own stack and remember only where the stack
//! pointer ended up (`saved_rsp`). Resuming is the reverse: load that stack
//! pointer, pop the registers, `ret`.
//!
//! ## Which registers
//!
//! [`switch_context`] is an ordinary function call as far as Rust knows. The
//! System V ABI says a callee may freely change `rax`, `rcx`, `rdx`, `rsi`,
//! `rdi` and `r8`-`r11`, so the compiler already assumes they are gone after any
//! call. Only the **callee-saved** registers `rbx`, `rbp` and `r12`-`r15` must
//! survive, so those six are all the stub saves. The return address is already
//! on the stack from the `call`, and the stack pointer is the thing being saved.
//!
//! There is nothing else to save: the kernel target is compiled without SSE,
//! MMX or x87 (`-sse,-mmx,+soft-float`), so there is no floating-point state,
//! and it has no red zone (the area below `rsp` that leaf functions may use
//! without adjusting it), which an interrupt could otherwise overwrite.
//!
//! ## Starting a new thread
//!
//! A thread that has never run has no saved registers. [`prepare_stack`] builds
//! a fake one: the six register slots, then the address of
//! `thread_trampoline` where `switch_context`'s `ret` will go. The first switch
//! into the thread then "returns" into the trampoline, which enables interrupts
//! (a switch inside an interrupt handler or under a lock leaves them off, and a
//! fresh thread has no `iretq` to restore them) and calls the thread's entry
//! point with its argument.
//!
//! ## Stack alignment
//!
//! The ABI requires `rsp` to be a multiple of 16 immediately before a `call`
//! instruction. At the trampoline's first instruction we arrange `rsp` to be a
//! multiple of 16, so its `call` is aligned.

use core::arch::global_asm;

global_asm!(
    r#"
    // fn switch_context(save_rsp: *mut u64, new_rsp: u64)
    //   rdi = where to store the old stack pointer
    //   rsi = the stack pointer to load
    .section .text.context,"ax"
    .global durg_switch_context
    durg_switch_context:
        push rbp
        push rbx
        push r12
        push r13
        push r14
        push r15
        mov [rdi], rsp
        mov rsp, rsi
        pop r15
        pop r14
        pop r13
        pop r12
        pop rbx
        pop rbp
        ret

    // First code a new thread runs. Registers come from the fake frame built
    // by `prepare_stack`: r12 = argument, r13 = function to call.
    .global durg_thread_trampoline
    durg_thread_trampoline:
        sti
        mov rdi, r12
        call r13
        ud2
    .section .text
    "#
);

extern "C" {
    #[link_name = "durg_switch_context"]
    fn switch_context_asm(save_rsp: *mut u64, new_rsp: u64);
    fn durg_thread_trampoline();
}

/// Words the first switch into a new thread pops: six registers, then the
/// return address (the trampoline).
const INITIAL_WORDS: u64 = 7;

/// Saves the current context at `*save_rsp` and continues the context whose
/// stack pointer is `new_rsp`. Returns when something switches back.
///
/// # Safety
///
/// - Interrupts must be off, so no other switch can start before this one ends.
/// - `save_rsp` must stay valid and unmoved until the saved context is resumed.
/// - `new_rsp` must be a stack pointer saved by an earlier call to this
///   function, or one built by [`prepare_stack`], for a stack that is mapped.
pub unsafe fn switch_context(save_rsp: *mut u64, new_rsp: u64) {
    // SAFETY: forwarded from this function's contract.
    unsafe { switch_context_asm(save_rsp, new_rsp) }
}

/// Writes the fake saved context for a thread that has not run yet, at the top
/// of its stack, and returns the stack pointer to hand to [`switch_context`].
///
/// The thread starts by calling `function(argument)`. `function` must never
/// return.
///
/// # Safety
///
/// The 64 bytes below `stack_top` must be mapped, writable, and not in use by
/// anything else; `stack_top` must be a multiple of 16.
pub unsafe fn prepare_stack(
    stack_top: u64,
    function: extern "C" fn(u64) -> !,
    argument: u64,
) -> u64 {
    debug_assert!(stack_top.is_multiple_of(16));
    // After the six pops and the `ret`, rsp = initial + 8 * INITIAL_WORDS. We want
    // that to be 16 bytes below the top (alignment, plus a little slack), so
    // initial = top - 16 - 8 * INITIAL_WORDS.
    let initial = stack_top - 16 - 8 * INITIAL_WORDS;
    let trampoline: unsafe extern "C" fn() = durg_thread_trampoline;
    let words: [u64; INITIAL_WORDS as usize] = [
        0,                          // r15
        0,                          // r14
        function as usize as u64,   // r13: what the trampoline calls
        argument,                   // r12: its argument
        0,                          // rbx
        0,                          // rbp
        trampoline as usize as u64, // return address of switch_context
    ];
    for (index, word) in words.iter().enumerate() {
        let slot = (initial + 8 * index as u64) as *mut u64;
        // SAFETY: inside the 64 bytes below `stack_top` the caller vouched for.
        unsafe { slot.write(*word) };
    }
    initial
}
