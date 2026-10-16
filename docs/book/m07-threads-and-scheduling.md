# M7: Threads and scheduling

**Goal:** more than one thing can run. Kernel threads, a context switch written in assembly, a round-robin scheduler driven by the timer, and locks that stay correct once the scheduler can interrupt anything.

## What a thread is

To the CPU a thread is two things: a **stack** and the **registers**. We do not keep a register structure. A context switch *pushes the registers onto the thread's own stack* and remembers one number, the stack pointer (`saved_rsp`). Resuming loads that stack pointer, pops the registers and executes `ret`.

```
 thread A's stack                     thread B's stack
 ┌──────────────┐                     ┌──────────────┐
 │ ...          │                     │ ...          │
 │ return addr  │ ← from `call`       │ return addr  │
 │ rbp rbx r12  │ ← pushed by         │ rbp rbx r12  │
 │ r13 r14 r15  │   switch_context    │ r13 r14 r15  │
 └──────────────┘                     └──────────────┘
        ▲ saved_rsp(A)                       ▲ saved_rsp(B)   ← rsp loads this
```

### Which registers

`switch_context` looks like a normal function call to Rust. The System V ABI lets any callee change `rax`, `rcx`, `rdx`, `rsi`, `rdi` and `r8`-`r11`, so the compiler already assumes they are gone after a call. Only the **callee-saved** registers (`rbx`, `rbp`, `r12`-`r15`) must survive, so those six are all we save. The return address came from the `call`.

Two facts make this complete: the kernel target is built without SSE, MMX and x87 (`-sse,-mmx,+soft-float`, checked with `rustc --print target-spec-json`), so there is no floating-point state; and it has no red zone, so an interrupt cannot trample memory below `rsp`.

### Starting a thread that never ran

It has no saved registers, so we **fake** them: at the top of its new stack we write six zero-ish register slots and, where `ret` will pop, the address of a small trampoline. The first switch into it "returns" into the trampoline, which executes `sti` (a switch under a lock or inside an interrupt handler leaves interrupts off, and a fresh thread has no `iretq` to restore them) and calls the entry function. When the entry function returns, the thread exits.

## Preemption: switching inside the interrupt handler

A timer interrupt arrives **on the running thread's own stack**: the CPU pushes its frame there, and our stub saves the other registers there too. The handler calls the scheduler, which may switch to another thread *from inside the handler*. The first thread's whole interrupt frame stays on its stack, frozen. When the scheduler picks it again, execution resumes in the handler, which returns, and the stub restores registers and `iretq`s to the instruction that was interrupted. One mechanism, the stack, carries both the voluntary and the involuntary case.

Order matters: the handler sends the **end-of-interrupt** to the APIC *before* calling the scheduler. The scheduler may not return for a long time, and the interrupt controller must not wait for it.

## The scheduler

A FIFO **run queue** of ready threads. When the running thread's turn ends (timer tick, `yield_now`, or `exit`) it goes to the back (unless it is finished) and the front thread runs: **round robin**.

Rules the code depends on, each written down where it applies:

- `schedule` runs with **interrupts off**. Nothing may start a second switch halfway through the first.
- The scheduler **lock is not held across the switch**. A lock held across a switch is owned by a thread that is no longer running, and the next thread would deadlock on it. The lock only guards *choosing*; interrupts staying off protects the switch.
- **The timer path never allocates.** The run queue is created with room for 64 threads, and `spawn` refuses to go past that, so a push never grows it.
- **The boot thread is the idle thread.** It never exits. When everyone else is finished, it keeps running.
- A finished thread's stack is released by the **next thread to run**, never by the dying thread, which is still standing on that stack until the switch completes.

### Stacks

Each thread has 64 KiB in its own virtual window (PML4 slot 507), read/write, no-execute, with an **unmapped guard page** below it. Finished threads' stacks are unmapped and their frames returned.

## Locks (`durg-sync`)

Why a lock must care about interrupts: thread A holds a plain spinlock, the timer fires, the scheduler switches to B, B wants the same lock and spins, and A cannot run while B spins on the only CPU. Deadlock caused by *preemption*, not by a second CPU.

`IrqLock` fixes it by forbidding the interruption: disable interrupts, take the lock, work, release the lock, **then** restore interrupts. Three details, each with a test:

- It **restores** the state it found rather than enabling. Inside another `IrqLock`, enabling on release would end the outer critical section.
- It releases the lock *before* interrupts come back, so a handler that runs immediately after finds it free.
- With one CPU and interrupts off, failing to get the lock can only mean the caller already holds it, so it **panics with a message** instead of hanging. When SMP arrives this must change to spinning.

The interrupt flag sits behind a trait, so the nesting logic is tested on the host with a fake flag, and again in QEMU against the real `RFLAGS.IF`.

### What got locked

The console (so lines do not interleave and a tick cannot switch halfway through one), the frame allocator, the heap, and page-table edits. The old re-entrancy panics would have fired on any normal preemption inside an allocation. **Lock order, outermost first: scheduler, heap, page tables, frame allocator; the console is a leaf.** A dying kernel prints *without* the console lock, because a panic can happen while the dying thread holds it.

## Tests

- **Host (`durg-sync`):** 8 tests, including 8 real threads incrementing a plain integer 20,000 times each under the spinlock.
- **Boot-time check, every boot:** three threads that only count and never yield. All three must make progress. Only the timer can explain that. Example output: `sched: 3 threads ran under preemption without ever yielding (loop counts [36016, 43406, 41578]), 8 context switches`.
- **In QEMU (8 new self-tests, 60 total):** an `IrqLock` against the real interrupt flag; spawn/join with an exact frame-leak check; strict alternation of two yielding threads; the preemption check; a **read-modify-write race** (a load, a busy wait, a store) that only the lock prevents, with the test insisting that preemption really happened during it; callee-saved registers preserved across switches; guard pages unmapped and stacks no-execute; and the 64-thread limit with full recovery.

### Negative controls

Breaking the code on purpose:

| Break | Result |
|---|---|
| Remove the lock from the race | `sched_irq_lock_prevents_lost_updates_under_preemption` failed: updates were lost |
| Put the old thread at the *front* of the queue | `sched_yield_alternates_between_cooperating_threads` failed |
| Never reap finished threads | `sched_spawn_join_runs_a_thread_and_returns_its_resources` failed (slot leak) |
| Timer never calls the scheduler | `sched_timer_preempts_threads_that_never_yield` failed |
| Remove `sti` from the trampoline | every variant timed out: new threads ran with interrupts off and nothing could preempt them (a hang counts as failure) |
| Do not save `r12` / do not save `rbx` | caught, but see below |
| Remove `cli` from the interrupt-disable function | `irq_lock_masks_real_interrupts_and_restores_them` failed |

**The register controls need an honest note.** With `r12` or `rbx` unsaved, the first thing to fail was the boot-time preemption check, which crashed before the self-tests ran. To see whether the *register test itself* detects it, I skipped the boot check and ran only that test: it failed with a page fault. It detects the fault by crashing, not by a clean assertion, because the compiled test code itself keeps live values in `r12`, so corrupting it breaks the thread. A corrupted callee-saved register is hard to observe without also breaking the observer, which is a reason to keep that test but not to trust it for diagnostics.

### Mistakes this milestone

- The heap stress test asserted that, after everything is freed, the free list is one block. That silently assumed no other long-lived allocation. The scheduler's run queue now splits the free memory, and the test failed. The bug was in the test: the real property is that the free list returns to the shape it had before.
- A script I wrote to run mutations hung on the first one (a lock that never releases spins forever) and left the source mutated. I restored the file from a copy and rewrote the script to kill the whole process group on a timeout. Separately, `pkill -f` matched my own shell's command line and killed it.
- The first negative control for `IrqLock`'s failure path survived: the code branch is only reachable if interrupts were on while the lock was held, which my test never arranged. The test now holds the inner lock directly.

## What this milestone does not do

- **No blocking or sleeping.** A thread that must wait polls with `yield_now`.
- **No priorities.** Strict round robin; a slice is one timer period, and the timer is **not calibrated**, so it is a count of clocks, not milliseconds.
- **One CPU.** `IrqLock` would be wrong under SMP, as described above.
- **A thread stack overflow is not diagnosed.** The guard page exists and is tested as unmapped, but nothing yet overflows a thread stack to show what happens. It would end in a double fault on the IST stack, reported without the "guard page" explanation.
- **No user mode.** Every thread is trusted kernel code. Nothing here tests isolation.

## Try it

```
cargo test -p durg-sync    # host tests for the locks
cargo xtask test           # six kernel variants, 60 self-tests
cargo xtask run            # boot once and read the sched: line
```
