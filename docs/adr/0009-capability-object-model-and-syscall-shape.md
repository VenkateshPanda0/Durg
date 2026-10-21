# ADR-0009: Capability object list and syscall ABI shape for M10

- **Status:** Proposed. **Awaiting the owner's decision** (CLAUDE.md §11, open decision 6). No M10 code is written against this until it is accepted.
- **Needed by:** before M10 starts. Also needed first: threat-model question 5 (confused-deputy rules).

## Context

§5 locks the style: a capability microkernel modelled on seL4's object model, untyped memory that tasks retype, CNodes/CSpaces, and a kernel that **never allocates on behalf of tasks after boot**. What is open is the *shape*: which objects exist in v0, how a task names a capability, and how many system calls there are.

Where the code stands after M9: three calls exist (`debug_print`, `yield`, `exit`) and they carry no authority. `spawn_user` and the ELF loader take frames straight from the kernel's frame allocator. That is exactly what M10 must replace: the kernel hands the root task all untyped memory at boot, and everything else (address spaces, threads, tables) is built by the root task by retyping and invoking capabilities.

## What any option must provide

- A capability is a kernel-held (object reference, rights, badge) triple living in a **slot** of a CNode. A task names it by an index (a *CPtr*), never by a pointer or a global name.
- Derive (a copy with equal or fewer rights), **revoke** (delete everything derived from a capability), delete, and retype (turn untyped memory into typed objects, accounted so memory cannot be double-used).
- No kernel allocation after boot: the derivation-tree links live **inside the slots**, as in seL4, so deriving costs the caller a slot it already owns.

## Options

### A. seL4-faithful: multi-level CSpaces, full object list, one `invoke` call
- Pro: the most studied design; every question has a published answer; the strongest teaching value.
- Con: the multi-level lookup (guards, radix, depth) is the hardest part to get right, and v0 does not need it. Largest M10.

### B. Zircon-style flat handle table per process
- Pro: simple to explain and implement; one table, an integer handle.
- Con: the kernel allocates and grows the table, which breaks the locked "no kernel allocation" rule, and there is no natural revoke tree. **Not recommended: contradicts §5.**

### C. seL4-style objects with a single-level CSpace first (recommended)
- A task's CSpace is **one CNode**: a power-of-two array of 32-byte slots (16 bytes capability, 16 bytes for the derivation links), retyped from untyped memory. A CPtr is a plain slot index. Multi-level lookup is a later, compatible extension (a CNode capability in a slot is already the object that makes it possible).
- Pro: keeps the part that carries the thesis (derive, revoke, retype accounting, rights, badges) and defers the part that does not. Small enough to factor into a host-tested crate (`durg-cap`) with property tests of revocation, as §9.5 asks from M10.
- Con: one CNode per task limits how many capabilities a task can hold until multi-level lookup is added; fine for the demo.

## Proposed v0 object list (under option C)

| Object | Why it is in v0 |
|---|---|
| Untyped | the source of all kernel-object memory |
| CNode | holds capabilities |
| Endpoint | synchronous IPC (the only way services talk) |
| Notification | asynchronous signal (needed for interrupts in M11) |
| Thread (TCB) | a schedulable entity, bound to a CSpace and address space |
| Frame | a page of memory that can be mapped |
| PageTable levels and the address-space root | built by the root task, not the kernel |

Deferred to M11: interrupt handler and I/O-port objects and DMA-capable frames (needed only by drivers). Deferred indefinitely: scheduling contexts and time budgets.

## Proposed syscall surface (under option C): about 7 calls

`Send`, `Recv`, `Call`, `Reply`, `ReplyRecv`, `Yield`, and one generic `Invoke(cptr, label, args)` that carries every object operation (retype, derive, revoke, map, set-up-thread). The existing `exit` becomes an operation on a thread capability; `debug_print` stays, in debug builds only, as §7 sketches. Seven calls is inside the §7 target of 10 to 15 and keeps the fuzz surface small.

## Consequences if accepted

- M10 begins with a written design (this ADR plus a design document), a pure `durg-cap` crate with its property tests, and only then kernel integration.
- The kernel's present spawn path, which takes frames from the allocator itself, is replaced: the boot code builds only the root task and hands it the untyped memory.
- The root-task model becomes the **authority distributor**; the confused-deputy rules (threat-model question 5) must be agreed before it is written.
- Risks I see: revocation bugs (mitigated by property tests written *before* the kernel uses the crate), IPC fast-path temptation (not in v0), and single-CPU assumptions baked into IPC (acceptable until SMP, which is deferred).

## Where the detail lives

A draft design for option C (slot layout, derivation, retype, IPC, root task, build order, acceptance test) is in `docs/design/m10-capability-core.md`. It is a draft and is discarded if another option is chosen.

## What I need from the owner

Option A, B or C, or a variation. My recommendation is **C**.
