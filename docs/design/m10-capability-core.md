# M10 design draft: the capability core

**Status:** DRAFT, written ahead of the owner's decision. It assumes ADR-0009 **option C** (single-level CSpace first), which is *Proposed, not accepted*. If the owner picks another option or the teaching fallback, this file is discarded or rewritten. **No M10 code exists.** CLAUDE.md §10 asks for the design before the code; this is that design, for review.

Modelled on the seL4 object model (reference manual: capability spaces, untyped memory, IPC). Not verified, and nothing here is a security claim until a test backs it.

## 1. The slot

A CNode is `2^n` slots of 32 bytes. A slot holds a capability and its place in the derivation list:

| Field | Size | Meaning |
|---|---|---|
| `kind` + `rights` + flags | 8 | object type; rights bits (read, write, grant, grant-reply); is-original flag |
| `object` | 8 | physical address of the object (or untyped region) |
| `badge` / `extra` | 8 | endpoint badge, untyped free index, frame size, and so on |
| `prev`, `next` | 8 (two 4-byte slot references) | neighbours in the derivation list |

(The exact packing is left to the implementation; the point is that **everything lives inside the slot**, so deriving or revoking never allocates.)

A task names a capability by its slot index (CPtr) in its own CNode. Lookups fail closed: out of range, empty slot, wrong type or missing right all return an error and change nothing.

## 2. Derivation and revocation

Slots form a **list ordered by derivation**: a capability's descendants follow it contiguously.
- **Derive (copy/mint):** insert the new slot immediately after its parent; rights may only shrink; an endpoint mint may set a badge once.
- **Delete:** unlink one slot. If it was the last capability to an object, the object is destroyed (frames returned to its untyped).
- **Revoke:** delete every descendant of a slot, leaving the slot itself.
- **Move:** relink the slot without changing its place in the order.

Invariants the host-side property tests must hold after *any* sequence of operations (they are the testable form of the claim that authority cannot be forged or kept after revocation):
1. The list is consistent (`next(prev(s)) = s`), and acyclic.
2. A derived capability's rights are a subset of its parent's.
3. After `revoke(s)`, no slot other than `s` refers to anything derived from `s`.
4. No two untyped-derived objects overlap, and live objects lie inside their untyped.
5. Deleting the last capability to an object frees it exactly once.
6. A badge, once set, never changes.

## 3. Untyped memory and retype

An untyped capability covers an aligned physical region and a **free index** (a watermark). Retype creates `k` objects of one type and size at the next aligned offset above the watermark, advances the watermark, and places a new capability for each in empty slots named by the caller. It fails if the region is too small or a target slot is occupied. The watermark returns to zero only when `revoke` removes every child (so memory is never reused while a stale capability to it exists). The kernel allocates nothing after boot: all object memory comes from untyped regions the root task was given.

## 4. IPC

Synchronous rendezvous on **endpoints**; asynchronous signalling on **notifications**.
- Blocked senders/receivers queue through links inside each thread control block (no allocation).
- `Call` = send plus a one-shot **reply capability** created for the callee; `Reply` consumes it. `Recv` on an endpoint capability needs the read right, `Send` needs write.
- Message: a small number of registers and a label; capability transfer needs the grant right on the endpoint, and moves or copies slots named in the message.
- Badge: an endpoint capability minted with a badge identifies the *sender* to the receiver, so a service can tell its clients apart without any global name.

## 5. Boot and the root task

At boot the kernel creates exactly one task, the **root task**, and gives it: its own CNode, thread and address-space capabilities, a capability to every remaining untyped region, and the initial boot modules as frames. It builds everything else (services, address spaces, threads) by retyping and invoking. Spawning a program is therefore a sequence of invocations by user space, not a kernel call, which replaces today's kernel-side `spawn_user`.

## 6. Syscalls (ADR-0009, option C)

`Send`, `Recv`, `Call`, `Reply`, `ReplyRecv`, `Yield`, `Invoke(cptr, label, args)`; `debug_print` in debug builds only. Every object operation (retype, derive, revoke, delete, map, configure thread) is an `Invoke` on the relevant capability, so the whole kernel API is a handful of entry points with one validator in front of each.

## 7. Plan, in the order I would build it (each step ends with tests before the next starts)

1. **`durg-cap` crate, no kernel:** slots, derive, revoke, delete, retype accounting. Property tests for invariants 1 to 6 first, including adversarial sequences; mutation checks on the revoke logic; a libFuzzer target over random operation sequences.
2. **CNode and untyped in the kernel** with the capability-lookup path; fuzz the lookup.
3. **Threads and address spaces as capabilities;** the boot code builds only the root task.
4. **Endpoints and `Call`/`Reply`;** a two-task ping-pong test; capability transfer test.
5. **Notifications;** then the M10 acceptance test (below).

**Acceptance (my proposal, for the owner to amend):** a task with no capability to an object cannot name it; a derived capability cannot exceed its parent's rights; revocation removes access in a running task (the next use fails); retype cannot overlap memory; and the property tests in step 1 pass with revocation mutations caught.

## 8. What this design deliberately leaves out (so nobody reads more into it)

Multi-level CSpaces; scheduling contexts and time budgets; interrupt and I/O objects (M11); labels, journal, approval (M13). It makes no claim about the threat model's P2 to P5; it is the substrate for P1 only, and untested until step 5.

## 9. Risks

- Revocation is where capability systems go wrong; hence the tests-first order in step 1.
- Reply-capability and IPC edge cases (a task dying mid-call) need an explicit "what happens to the peer" rule before step 4.
- Single CPU is assumed throughout; SMP is a later redesign of the locking, not a bolt-on.
