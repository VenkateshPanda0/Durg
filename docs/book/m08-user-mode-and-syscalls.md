# M8: User mode and system calls

**Goal:** run a program the kernel does not trust. It lives in its own address space, it can call the kernel only through one narrow door, and whatever it does wrong ends *that program*, not the kernel.

This is the first milestone where the thesis has something to defend against: before it, every line of code was ours.

## Rings, segments and the GDT

The CPU has four privilege levels; we use two. **Ring 0** is the kernel. **Ring 3** is a user program. The level you are at is the low two bits of the code segment selector (CS), and it decides which instructions you may run (`cli`, `hlt`, `mov cr3`, port I/O are ring 0 only) and which pages you may touch.

Segmentation is mostly off in 64-bit mode, but the GDT still has to hold four descriptors that say "this selector means ring 0 or 3, code or data". The order is not arbitrary:

| Index | Selector | Descriptor |
|---|---|---|
| 1 | `0x08` | kernel code |
| 2 | `0x10` | kernel data |
| 3 | `0x1b` | user data (`0x18` with the requested privilege level 3) |
| 4 | `0x23` | user code (`0x20 | 3`) |
| 5-6 | `0x28` | TSS |

`sysret` computes the user selectors from a single field of the `IA32_STAR` register: `SS = base + 8`, `CS = base + 16`. So user data must come *before* user code, and the kernel pair (which `syscall` loads from the same register) must be code then data. Three `const` assertions tie the selectors to those formulas, so reordering the table fails to compile instead of faulting at the first system call. A test decodes every descriptor's privilege level, long-mode bit and type.

## Two doors between the rings

**Down (kernel to user): `iretq`.** `iretq` pops a return frame (RIP, CS, RFLAGS, RSP, SS) and, if CS names ring 3, switches to ring 3. To start a task the kernel pushes a hand-built frame and executes it. **First it zeroes every general-purpose register.** The thread trampoline keeps the address of the thread's entry function in `r13`, so without the zeroing a kernel code address would reach the program. A test makes a user program report the OR of all its registers, and a mutation that skipped zeroing `r13` is caught by it.

**Up (user to kernel): `syscall`.** In one instruction the CPU saves the return address in `rcx` and the flags in `r11`, loads the kernel CS/SS from `STAR`, clears the flags named in `IA32_FMASK`, and jumps to the address in `LSTAR`. What it does **not** do is touch the stack pointer: on entry `rsp` is still the *user's*. Everything dangerous about system calls is in the next few instructions.

### The entry stub

1. Interrupts are off (`FMASK` cleared `IF`), so nothing can intervene.
2. Park the user `rsp` in a scratch variable; load `rsp` from `KERNEL_STACK_TOP`, **the top of the current thread's kernel stack**, which the scheduler rewrites at every switch.
3. Push a frame (user `rsp`, flags, return address, all registers), turn interrupts back on (a long call must not stall preemption), and call Rust.
4. Interrupts off, restore registers, load the user `rsp` **last**, `sysretq`. No interrupt may arrive between loading the user stack pointer and `sysretq`, or it would run on the user's stack.

`FMASK` also clears `TF` (no single-stepping the stub), `DF` (the ABI wants it clear), `NT`, and `AC` (so a program cannot set the alignment-check flag to open a window for SMAP). A test reads the MSRs back.

One CPU means one global for the kernel stack and one for the scratch slot. SMP would make them per-CPU, reached through `swapgs`; that is on the roadmap.

### Interrupts from ring 3

A timer tick while a program runs in ring 3 changes privilege level, so the CPU must switch stacks, and it takes the new stack from **`TSS.RSP0`**. That field is the kernel stack of the thread that is running. The scheduler writes it, and the `syscall` global, at every switch. A test makes a program spin forever without ever calling the kernel and checks that another task still finishes, which only works if that path is right; a mutation that skipped the write ends in a double fault.

## Address spaces

Each task gets its own top-level page table. The lower half of the address space (entries 0-255) is the task's; the upper half (256-511) is the kernel's and **identical in every space**, so the kernel runs unchanged whichever space is loaded. Switching is a write to `CR3`, done before the stack switch (both spaces map both stacks).

### Sharing the kernel half

"Shared" means each space's top-level entries 256-511 are *copies of the kernel's*, pointing at the same second-level tables. That has a trap: if a kernel slot is empty when a task's space is created and the kernel fills it later (the heap grows into a fresh region, say), the new table exists in the kernel's root only, and every task created earlier cannot see the mapping. The fix is to **give every upper-half slot a table at boot** (254 extra frames, about 1 MiB), so a later mapping lands in a table every space already shares.

The test for this looks simple and was wrong the first time: it used the scratch window, whose slot was already populated, so it would have passed even without the fix. It now maps in slot 400, which was empty before the fix, and was isolated to prove it fails when the pre-population is removed (`NotMapped { level: 4 }`).

### The user bit

A page is reachable from ring 3 only if the **user** bit is set at *every level* of the walk. Tables created on the way to a lower-half address get it; tables in the kernel half never do, and a user-accessible leaf in the kernel half is refused outright. Destroying a space frees its user pages and its own tables and **not** the shared kernel half (a mutation that did crashed the kernel immediately).

### What a task gets

Code at `0x400000`, mapped read and execute, never write. A stack of four pages at the top of user space, read and write, never execute. Every frame is zeroed before use, and the whole space is freed when the task ends; a test compares the free-frame count exactly.

## The kernel never dereferences a user pointer

A program controls every byte it passes. For `debug_print(pointer, length)` the kernel:

1. validates the range lies entirely in the lower half and is at most 256 bytes (`validate_user_range`). That one comparison refuses every kernel address, the non-canonical hole, and every range that wraps past `u64::MAX`;
2. splits it into pieces that each stay inside one page;
3. for each piece, asks the page tables of the *calling task* where it is, requires the page to be mapped **and user-accessible**, and copies from there through the kernel's own mapping.

An unmapped page is therefore an error return, not a kernel page fault, and a page that exists in the task's tables but is kernel-only (the test builds one) is refused instead of being read on the program's behalf.

The validator lives in a pure crate, `durg-syscall`, with no `unsafe` code and host property tests. **Printed text is untrusted too**: control characters, the escape character and newlines become `?`, and every line is tagged `[user N name]`, so a program cannot forge a kernel log line or send escape sequences to the host's terminal.

### The calling convention is provisional

`rax` = number, `rdi`/`rsi`/`rdx` = arguments, result in `rax`; an error is `-(code)`. Three calls: `debug_print`, `yield`, `exit`. This is **not** the capability ABI, which is an open decision (CLAUDE.md §11 item 6) to be designed before M10.

## When a program misbehaves

An exception raised while the CPU is in ring 3 is the program's problem. The exception handler checks the saved CS: if it was ring 3, it prints the exception, records the vector, error code and instruction address in the task's exit status, and ends the thread; the scheduler moves on. A **kill request** (`JoinHandle::kill`) is acted on by the next timer tick that finds the task in ring 3, so a task that never calls the kernel can still be stopped.

The tests run a program for each of: read, write and jump into kernel memory; read an unmapped page; write to its own code page; run code from its own stack; `cli`; `hlt`; `mov cr3`; port I/O. Each is checked for the exact vector and, for page faults, the exact error code (present, write, user, instruction-fetch bits). For the two "jump to here" cases the fault must be **at the first instruction**; see "Mistakes".

## SMEP, SMAP and UMIP

Three CPU features, enabled in `CR4` when CPUID reports them:

- **SMEP**: ring 0 faults if it *fetches an instruction* from a user-accessible page. Stops a corrupted kernel pointer being steered into code an attacker wrote.
- **SMAP**: ring 0 faults if it *reads or writes* a user-accessible page (unless `AC` is set; `stac`/`clac` set and clear it). Makes "forgot to validate a user pointer" an immediate fault. This kernel never needs `stac` outside tests because it copies through the page tables.
- **UMIP**: ring 3 faults on `sgdt`, `sidt`, `sldt`, `smsw`, `str`. Without it a program can read the kernel addresses of the GDT and IDT.

The boot log says which are on, with three states, so "supported but not enabled" cannot be mistaken for "not supported". The TCG CPU model in the test harness adds `+smep,+smap,+umip` to plain `qemu64` (`-cpu max` would also add x2APIC, which changes how the interrupt controller is set up); KVM keeps `-cpu host`. Each test skips *visibly* if the CPU lacks the feature.

## Tests

- **Host (`durg-syscall`):** 16 tests including properties: decoding is total, an accepted range never touches an address at or above the boundary or wraps, chunks partition a range without crossing a page.
- **QEMU, 21 new self-tests (81 total):** GDT descriptors; per-task address spaces (six tests); the syscall MSRs; the ring-3 round trip; ten fault classes; five bad system calls; console sanitising; registers preserved across a system call; registers zeroed at entry; a task preempted in ring 3 and then killed; exact resource return; 22,000 random and boundary calls into the real handler from inside a user space; and the CPU-protection tests.
- **Boot-time line, every boot:** `user: the program ran in ring 3, called the kernel, and exited with code 7`.

### Negative controls

| Break | Result |
|---|---|
| User code pages writable | `write-own-code` no longer faults: failed |
| User stack executable | **survived at first**, then caught after the fix below |
| Copy path ignores the user bit | the random test accepted the kernel-only page: failed |
| Console output not sanitised | failed |
| `enter_user` leaves `r13` (a kernel function pointer) | zeroed-registers test failed |
| `TSS.RSP0` not updated on a switch | double fault |
| Syscall kernel stack not updated | boot itself failed |
| Ring-3 exceptions treated as kernel-fatal | failed |
| Kill requests ignored | hang, caught by the timeout |
| A reaped thread does not free its address space | exact frame count failed |
| `FMASK` does not clear `IF`; wrong `STAR` base | MSR test failed |
| Kernel half not pre-populated | the sharing test failed, and in isolation the later-mapping test did too |
| SMEP / SMAP / UMIP not enabled | each dedicated test failed *in isolation* (SMEP by crashing when the kernel executed the user page, the others by assertion) |

Not mutation-checked: the privileged-instruction cases and the reads, writes and jumps into kernel memory, which rest on hardware behaviour of the GDT and page-table bits.

## Mistakes this milestone

- **A test that could not tell.** With the user stack mapped executable, every test still passed. The task ran harmlessly through its zeroed stack (`add [rax], al`) until it fell off the end of the page and faulted *with the same error code* as a no-execute violation. The exit status now records the faulting instruction address and the two "run code from here" cases require it to be the jump target. Found only because I ran a negative control for the stack permission.
- **A test that passed without the mechanism**, twice. The later-mapping test used an already-populated slot; the three CPU-protection controls were first caught only by the aggregate CPUID test, which runs first and ends the suite, so the dedicated tests were unproven until isolated.
- **A bad isolation filter.** My first attempt filtered on a string that matched no test and "passed" with zero tests run. I noticed because no test line appeared, then reran with the right filter and checked that the test ran and failed.
- **Two real gaps in the validator's tests** (wraparound with a huge length from a valid pointer, and the boundary constant moving), found by mutation testing of `durg-syscall`, plus a test of my own with a length over its own limit.
- **A release-only miscompile, found by the last check I ran.** The test that reads a user page from ring 0 passed in debug and faulted in release: the optimiser moved the read *before* `stac`. I had marked the `stac`/`clac` blocks `nomem`, which tells the compiler they touch no memory, so it may reorder memory accesses around them. An access-window helper must be a compiler memory barrier. The same flaw existed in the `cli`, `sti` and `IrqLock` enable/disable blocks (an `IrqLock` needs interrupts off *before* it takes the lock and back on only *after* it releases it); it had not bitten, and is fixed too. CI ran only debug builds, so it now also runs the boot test in release mode.
- **A misleading boot message** ("the CPU does not report it") when the CPU did and I had not enabled it. Found by a mutation; the message now has three states.
- **`AddressSpace::from_root` was a safe function** that could be used to free arbitrary frames. It is now `unsafe` with a stated contract.
- One commit (`6265238`) does not pass `clippy -D warnings` with `--features selftest`, because the test programs it adds are used only by the next commit. The two are meant to be read together.

## What this milestone does not do

- **One CPU.** The syscall stack variables are global; SMP needs per-CPU data and `swapgs`.
- **No NMI or machine-check handling.** An NMI in the instructions between `syscall` and the stack switch would run on the user's stack.
- **The `sysret` return-address defence and the AC-flag behaviour on exception delivery are written but not exercised.** Nothing can currently produce a non-canonical return address.
- **No coverage-guided fuzzing of the syscall entry yet.** That needs `cargo-fuzz`, which needs a nightly toolchain on the host (the kernel stays on stable). It is the owner's call; until then the entry is covered by property tests of the validator and by randomised calls into the real handler.
- **Three system calls, no capabilities.** This proves the ring-3 round trip, nothing about authority.
- **Hardware and KVM are untested**; the protections were verified under QEMU's software emulation only.
- **A task can still be killed only from ring 3**; one running kernel code finishes it first.

## Try it

```
cargo test -p durg-syscall   # host: the validator
cargo xtask test             # six kernel variants, 81 self-tests
cargo xtask run              # boot once; read the cpu: and user: lines
```
