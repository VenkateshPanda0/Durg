# Teach-back questions for M7 to M9

**Purpose:** the project's value depends on the owner being able to explain this kernel (see the gate review, section 3). These questions are for talking through with the assistant, not a test with a pass mark. Answer in your own words, in whatever order you like; the assistant will correct, deepen, or admit where the code or chapter is unclear. "I don't know, show me" is a good answer.

Each question says where to look. The answers are deliberately **not** written here.

## M7: threads and scheduling (`docs/book/m07-threads-and-scheduling.md`, `kernel/src/sched.rs`, `crates/sync`)

1. The thread switch happens inside the timer interrupt handler, *after* the end-of-interrupt write to the LAPIC. What would go wrong if it happened before? What would go wrong if there were no EOI at all?
2. `IrqLock` restores the interrupt state it *found*, instead of always re-enabling interrupts on unlock. Construct a case where "always re-enable" is a bug.
3. The lock order is scheduler, then heap, then page tables, then frame allocator, with the console as a leaf. Why can an interrupt handler not allocate memory? Which two locks would deadlock, and how?
4. Each kernel thread stack has an unmapped guard page. What does that turn a stack overflow into, and why is that better than silent corruption?

## M8: user mode and syscalls (`docs/book/m08-user-mode-and-syscalls.md`, `kernel/src/arch/x86_64/syscall.rs`, `gdt.rs`)

5. The `syscall` instruction does not switch stacks. What does our entry stub do about that, why does it use global storage, and exactly which assumption makes that safe today and unsafe on a second CPU?
6. In the GDT, user data comes *before* user code. Why does `sysret` force that order? What does `STAR[63:48] = 0x10` mean in terms of the selectors it produces?
7. `FMASK` clears IF, TF, DF, NT and AC on `syscall` entry. Pick two of them and say what attack or bug appears if it is left set.
8. SMEP and SMAP both involve user pages. State in one sentence each what they stop, then explain why the kernel *still* never dereferences a user pointer directly even with SMAP on.
9. The release build once read user memory before `stac` ran. Why did the compiler reorder it, what did `nomem` have to do with it, and why did only the release build show it?
10. A new user task starts with every register zeroed. Name one specific leak this prevents.

## M9: ELF loader and isolation (`docs/book/m09-elf-loader-and-isolation.md`, `crates/elf`, `kernel/src/user.rs`)

11. Boot modules sit in memory the kernel's own tables do not map. Why, and what permissions does the module window give them, and why those?
12. The ELF parser refuses a segment that is writable and executable, and an entry point that is not inside executable file-backed bytes. For each, describe an attacker who benefits if the rule is missing.
13. Why must every frame given to a task be zeroed? The first version of the residue test passed even when the loader did not zero frames. Why? (Hint: look at `.got` and `memsz` versus `filesz`.) What does that teach you about trusting a green test?
14. `task_a` and `task_b` keep their secrets at the *same virtual address*. What would an isolation test that put them at different addresses fail to prove? What does the frame-disjointness assertion add that the secret-keeping programs do not?
15. The kernel's direct map still gives it a writable alias of every user page. Is that a hole in task-to-task isolation? In kernel-versus-task isolation? Who could exploit it?

## Prediction questions (answer *before* looking at the code)

16. If the kernel stack pointer in the TSS (`RSP0`) were not updated when switching to a different user thread, and a timer tick arrived in ring 3, whose stack would the CPU use?
17. If the loader mapped a data segment with the user bit set at the page but not at the page-table levels above it, what would a ring-3 read do?
18. If `task_b` were killed in the middle of the isolation test, which of its resources must be returned, and which test would notice if one was not?
