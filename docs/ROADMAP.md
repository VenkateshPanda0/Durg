# Roadmap

Work proceeds **one milestone at a time**. A milestone is done when its acceptance criteria pass with real, observed output, a design-log chapter is written, and the owner signs off. Dates are intentionally absent. The project is part-time, and slipping is reported honestly rather than hidden.

## Where we are, by phase (2026-11-06)

"Done" below means: the code works, its tests pass in CI on a hosted runner (log read), its negative controls were run, and its chapter and devlog exist. Since 23 October the assistant has signed milestones off under [ADR-0010](adr/0010-delegated-project-authority.md); the owner signed off M1 to M8.

| Phase | Milestones | Status |
|---|---|---|
| **0. Foundations** (thesis, threat model, ADRs, build, CI) | M0, M1 | **done** |
| **1. A kernel that boots and survives** (serial, exceptions, timer, memory, heap) | M2 to M6 | **done** |
| **2. Tasks and isolation** (threads, preemption, ring 3, syscalls, ELF loader, address spaces) | M7 to M9 | **done** |
| **3. Capabilities** (the heart: derive, revoke, retype, endpoints, root task) | M10 | **in progress, about 75%**: capability core, capability spaces, memory retyping, messages, root task via the interim spawner are done; thread and address-space objects (the proper replacement for the spawner) and notifications are left |
| **4. Drivers and storage** (user-space virtio console, block, vsock; copy-on-write store) | M11, M12 | not started |
| **5. Trust services** (effect journal, labels, approval gate, egress broker) | M13 | not started |
| **6. The product** (shell, tools, host controller, flagship demo) | M14 | not started |
| **7. Reach** (Firecracker-class boot, WASI runtime, ARM64) | M15 to M17 | not started, may not happen |

**Phase 3 in detail (M10):**

| Step | What | Status |
|---|---|---|
| 1 | `durg-cap` crate: slots, derive, revoke, retype, invariants (host-tested, fuzzed) | done |
| 2 | Per-task capability spaces, untyped memory from the frame allocator, `invoke` | done |
| 3 | Endpoints, blocking `send`/`recv`/`call`/`reply`, badges, capability transfer | done |
| 4 | Root task builds the system: spawner capability and `join` (**interim**, ADR-0011) | done in this commit series |
| 5 | Thread, address-space and page-table objects, mapping by invocation (replaces the spawner; restores "the kernel never allocates for tasks") | **left** |
| 6 | Notifications | **left** |
| 7 | CNode retype, capability-space growth, multi-level lookup if needed | **left** |
| 8 | `reply_recv`, replies that carry capabilities, killable blocked threads | **left** |

**After M10**, in order: M11 (virtio drivers in user space; needs contiguous frames and interrupt delivery through notifications), M12 (object store), M13 (journal, labels, approval gate, broker), M14 (the demo from the README). The capability work is what the rest depends on; the thesis is only really tested once M13 exists.

## Milestones

| # | Milestone | Acceptance (summary) | Status |
|---|---|---|---|
| **M0** | Thesis, threat model, decision records | Owner approves the threat model. No kernel code before that. | threat model approved, ADRs 0004-0005 pending |
| **M1** | Toolchain, build, CI | A fresh clone plus one documented command boots under QEMU and prints a line on serial. CI shows the same. | **done**, signed off by the owner 2026-10-15 |
| **M2** | Kernel entry and serial output | Kernel prints the memory map. A deliberate test failure exits QEMU with a failure code. | **done**, signed off by the owner 2026-10-15 |
| **M3** | CPU structures, exceptions, interrupts, timer | Breakpoint, page fault and double fault are handled in tests. Timer ticks are counted. | **done**, CI green, signed off by the owner 2026-10-15 |
| **M4** | Physical memory manager | Allocate/free stress test in QEMU. Host property tests show no double allocation. | **done**, CI green, signed off by the owner 2026-10-15 |
| **M5** | Virtual memory and paging | Map, unmap, translate, page-fault and guard-page tests. | **done**, CI green (run 11), signed off by the owner 2026-10-16 |
| **M6** | Kernel heap | Heap stress tests. An out-of-memory path is handled. | **done**, CI green (run 12), signed off by the owner 2026-10-16 |
| **M7** | Threads and scheduling | Multiple threads interleave under preemption, shown by a test. | **done**, CI green (run 13), signed off by the owner 2026-10-18 |
| **M8** | User mode and syscalls | A user task makes a syscall and returns. A task touching kernel memory is killed, not the kernel. | **done**, CI green (run 14), signed off by the owner 2026-10-19 |
| **M9** | ELF loader and init task | A CI test proves task A cannot read task B's memory. **Decision gate:** review progress before continuing to the capability work. | **done**, CI green (runs 17, 19), signed off by the assistant 2026-10-23 under ADR-0010; gate decision: continue (`docs/reviews/m09-decision-gate.md`) |

## After M9 (outline only, no fresh plan yet)

| # | Milestone |
|---|---|
| M10 | Capability core: CSpace/CNodes, untyped retype, derive, revoke, endpoints, notifications. Property tests for revocation. **In progress**: design in `docs/design/m10-capability-core.md`; see the phase table above for steps 1 to 8. |
| M11 | User-space virtio: console, block, vsock. Restartable drivers. |
| M12 | Copy-on-write object store with snapshots. Crash-consistency tests. |
| M13 | Effect journal (hash-chained), provenance labels, approval gate, host egress broker. |
| M14 | Shell, native tools, host controller. The flagship demo from the README. |
| M15 | Firecracker-class VMM boot path and boot-time benchmark. |
| M16 | WASI-component runtime. |
| M17 | ARM64 port. |

## Open decisions

Tracked as `Proposed` records in [docs/adr](adr/README.md) where options are written up. Others will be raised when they become relevant:

- Thread and address-space objects replacing the spawner (ADR-0011): when, and how page tables supplied by a task are validated
- Label semantics (before M13)
- Journal tamper-evidence scheme (before M13)
- WASI runtime: port vs write (before M16)
- In-guest TCP/IP vs broker-only (after M14)

## Later

Ideas noticed along the way that do not belong to the current milestone are added here instead of being built early.

- **Free page-table frames on unmap (M8):** unmapping leaves empty page-table frames in place. Free them once address spaces are created and destroyed per task.
- **Change permissions and split huge pages (M8/M9):** `mm::paging` can map and unmap but not change the permissions of an existing mapping or split a 2 MiB page.
- **SMEP, SMAP and UMIP (M8):** enable them when user mode exists, so the kernel cannot be tricked into executing or touching user memory.
- **Writable direct-map alias of loaded code (M9):** the direct map maps all RAM read-write, so a loaded user program has a writable kernel alias. Decide how the ELF loader handles that.
- **Always zero frames given to untrusted tasks:** reclaimed bootloader frames hold old data and `alloc_frame` does not clear them. Use `alloc_zeroed_frame` for anything a task can see (M8).
- **Kernel address randomisation:** the kernel is at a fixed address.
- **Contiguous frame allocation (M11):** virtio rings and DMA buffers will need physically contiguous runs. The allocator only returns single frames.
- **Calibrate the LAPIC timer (before anything sleeps for a duration):** the timer is counted, not timed. Its clock rate is unknown, so a tick is not a fixed number of milliseconds. Calibrate against another clock before the scheduler depends on it.
- **Timer check can hang boot:** the boot-time tick check waits for interrupts forever if none arrive. That is acceptable for QEMU and Firecracker-class VMMs, but add a bounded wait before relying on other environments.
- **Reclaiming bootloader memory (M4):** copy out the memory map and any boot data before reusing bootloader-reclaimable regions.
- ~~**Pin the Rust toolchain version.**~~ Done 2026-10-19: pinned to 1.97.0 (ADR-0002 addendum); bump deliberately.
- **Kernel heap is for kernel bookkeeping only (M10):** nothing an untrusted task can make grow may be paid for from the kernel heap. Tasks pay with their own untyped memory.
- **Heap hardening:** poison freed memory, add red zones, and consider clearing on allocation. Consider a size-class cache once the hot kernel objects are known.
- **Heap growth failure paths:** "too few frames left" and "mapping failed, roll back" are not triggered by any test. Add fault injection to the frame allocator.
- **Thread stack overflow diagnosis (M8):** a thread that overflows its stack hits its guard page and ends in a double fault on the IST stack, but the handler only recognises the boot stack's guard, so it reports a plain double fault. `sched::in_thread_stack_guard` exists for this and is not used yet; add a boot variant that overflows a thread stack on purpose.
- **Blocking and sleeping:** message passing now has a real blocked state with wake-ups (M10). Sleeping for a *duration* still needs the timer calibrated, and a blocked thread still cannot be killed.
- **Scheduler policy:** strict round robin, one tick per slice, no priorities. Revisit when services have different latency needs.
- **IrqLock under SMP:** it panics on contention, which is right for one CPU with interrupts off and wrong the moment a second CPU exists. SMP is deferred, and this is one of the things it will change.
- **Page-table locking is coarse:** one lock for every mapping edit.
- **Thread stack memory is mapped eagerly:** 64 KiB per thread, all committed.
- **Fuzzing beyond the pure validator (M9+):** `fuzz/` covers `durg-syscall` only (ADR-0008). The page-table walk and copy in `syscall.rs`, and later the ELF loader's parser, should be factored into pure crates so they can be fuzzed too. A short fuzz run in CI would make nightly part of CI, which is a separate decision.
- **NMI and machine-check handling (M9+):** the interrupt table has no handler or stack for them. A non-maskable interrupt in the few instructions between `syscall` and the kernel-stack switch would run on the user's stack.
- **Per-CPU data and `swapgs` (SMP):** the syscall entry keeps the kernel stack pointer and a scratch slot in two globals, which is right for one CPU only.
- **Untested `sysret` defences:** `dispatch` refuses a non-canonical or kernel-half return address, but nothing can currently produce one, so it is untested. Likewise whether exception delivery from ring 3 clears `AC` (the SDM statement was not checked); the syscall path clears it through `IA32_FMASK`.
- **Killing a task in the kernel:** `kill` acts on the next tick that finds the task in ring 3. A task blocked or looping in kernel code is not interrupted.
- **Per-task limits:** nothing bounds how much memory a task can map. Tasks cannot map memory yet, so it does not matter until M10.
- **User-space ASLR and a user stack guard gap:** the code and stack addresses are fixed.
- **CI action versions:** `actions/checkout` and `actions/cache` v4 run on a deprecated Node version and are being forced onto a newer one. Bump when updated majors are available and checked.

## Fallback

If progress stalls at the M9 decision gate, the project narrows into a **teaching capability OS** with a public book-style design log. Code and docs are written so that path stays viable: clear, explained and incremental.
