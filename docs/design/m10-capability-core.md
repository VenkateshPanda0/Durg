# M10 design draft: the capability core

**Status:** M10 is in progress. ADR-0009 option C was accepted, and the capability core, per-task spaces, endpoint IPC and interim root-task spawner are implemented. Thread/address-space objects, mapping and notification behavior remain work to do. Sections below describe the target design; see the [roadmap](../ROADMAP.md) and [ADR-0011](../adr/0011-spawner-capability-as-an-interim-step.md) for current limitations.

Modelled on the seL4 object model (reference manual: capability spaces, untyped memory, IPC). Not verified, and nothing here is a security claim until a test backs it.

## 1. The slot

A CNode is `2^n` slots of 32 bytes. A slot holds a capability and its place in the derivation list:

| Field | Size | Meaning |
|---|---|---|
| `kind` + `rights` + flags | 8 | object type; rights bits (read, write, grant, grant-reply); is-original flag |
| `object` | 8 | physical address of the object (or untyped region) |
| `badge` / `extra` | 8 | endpoint badge, untyped free index, frame size, and so on |
| `prev`, `next` | 8 (two 4-byte slot references) | neighbours in the derivation list |

**Implemented layout (`durg-cap`):** `object` u64, `extra` u64, `prev` u32, `next` u32, `depth` u16, `kind` u8, `rights` u8, `bits` u8, then padding to 32 bytes. Links are stored as `index + 1` so that **all-zero bytes are a valid empty slot**: the kernel hands out zeroed memory on retype, and zeroed memory is therefore already a valid, empty capability space. `Slot::from_bytes` refuses bytes the crate could not have written, so the kernel reads capability-space memory without trusting it. Everything lives inside the slot, so deriving or revoking never allocates.

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

An untyped capability covers an aligned physical region and a **free index** (a watermark). Retype creates `k` objects of one type and size at the next aligned offset above the watermark, advances the watermark, and places a new capability for each in empty slots named by the caller. It fails if the region is too small or a target slot is occupied. The watermark returns to zero only when `revoke` removes every child (so memory is never reused while a stale capability to it exists). The target design allocates object memory from untyped regions held by the root task. The interim spawner still causes the kernel to allocate task memory; see ADR-0011.

## 4. IPC

Synchronous rendezvous on **endpoints**; asynchronous signalling on **notifications**.
- Blocked senders/receivers queue through links inside each thread control block (no allocation).
- `Call` = send plus a one-shot **reply capability** created for the callee; `Reply` consumes it. `Recv` on an endpoint capability needs the read right, `Send` needs write.
- Message: a small number of registers and a label; capability transfer needs the grant right on the endpoint, and moves or copies slots named in the message.
- Badge: an endpoint capability minted with a badge identifies the *sender* to the receiver, so a service can tell its clients apart without any global name.

### Current single-CPU IPC lifetime boundary (2026-11-10)

Endpoint capability resolution, rights checks and the scheduler handoff execute
with maskable interrupts disabled. When revocation or teardown destroys endpoints,
the capability lock is released and waiters are woken before interrupts are
restored. This prevents another scheduled task on the single CPU from revoking or
reusing an endpoint between those steps. Blocking IPC can still yield while
interrupts are disabled, as the scheduler already does; each resumed context
restores its own prior interrupt state.

This is not an SMP design: a second CPU would require object lifetime references
and a shared synchronization/lock-order argument. It does not address unhandled
NMI or machine-check paths, spawner authorization races, bounded interrupt
latency during large revocations, or every possible malicious peer. Notification signal and wait use the same lifetime discipline.

### Notification behavior (single CPU)

A retyped notification begins idle. A signal requires WRITE; if waiters exist,
the oldest receives the signaling capability's badge, otherwise badges coalesce
with bitwise OR until a waiter consumes them. A zero badge is still a signal.
A wait requires READ and returns its full 64-bit badge in `rdi`, with `rax`
reserved for success or an error. Destroying the last notification capability
wakes blocked waiters with `Aborted`. Revocation records exact waiting object
addresses, so objects between two destroyed addresses are not woken spuriously.
This does not yet implement IRQ capabilities, device delivery, SMP, or a general
thread/address-space capability lifecycle.

## 5. Boot and the root task

At boot the kernel creates exactly one task, the **root task**, and gives it: its own CNode, thread and address-space capabilities, a capability to every remaining untyped region, and the initial boot modules as frames. It builds everything else (services, address spaces, threads) by retyping and invoking. Spawning a program is therefore a sequence of invocations by user space, not a kernel call, which replaces today's kernel-side `spawn_user`.

## 6. Syscalls (ADR-0009, option C)

`Send`, `Recv`, `Call`, `Reply`, `ReplyRecv`, `Yield`, `Invoke(cptr, label, args)`; `debug_print` in debug builds only. Every object operation (retype, derive, revoke, delete, map, configure thread) is an `Invoke` on the relevant capability, so the whole kernel API is a handful of entry points with one validator in front of each.

## 7. Plan, in the order I would build it (each step ends with tests before the next starts)

**Status (2026-10-26):** step 1 done; step 2 done in a reduced form (per-task single-CNode capability spaces, boot-reserved untyped blocks, `invoke` with identify/retype/derive/move/delete/revoke on the caller's own space; CNode, thread and page-table objects cannot yet be created by retype). Step 3 (as step 4 here: endpoints and `call`/`reply`) done in the kernel with capability transfer and badges; an interim **spawner capability** (ADR-0011) now lets the root task build the system; the original step 3 (threads and address spaces as capabilities, which replaces the spawner) and step 5 (notifications) was not started at that date. Notification signal/wait has since been implemented on the single-CPU kernel; see the current behavior above.


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

## 10. Object lifecycle rules (TODO 1.1, written 2026-11-21)

This section is the rule set that TODO items 1.2 to 1.7 are built and tested
against. Each rule carries a status: **built** (the kernel does this today and a
self-test named below shows it), **interim** (done, but by a mechanism that
ADR-0011 plans to retire) or **target** (not built; the item that builds it is
named). Nothing here is a security claim until the named test exists and passes.
Status was read from `kernel/src/cspace.rs`, `kernel/src/sched.rs`,
`kernel/src/syscall.rs` and the self-test table in `kernel/src/selftest.rs`.

### 10.1 General rules

1. **A capability is the only owner.** An object lives while at least one
   capability to it exists. Deleting the last capability destroys the object once
   (invariant 5, section 2). Nothing else keeps an object alive: not a task, not
   a blocked waiter, not a queue. *built* for endpoints, notifications, CNodes
   and untyped regions.
2. **Memory is owned by its untyped region.** Object memory comes from an untyped
   capability by retype and returns to that region when the objects are gone.
   The watermark returns to zero only when `revoke` has removed every child, so
   memory is never handed out again while a stale capability to it exists.
   *built* (`retyped_memory_is_zeroed_and_only_that_memory`,
   `capability_errors_change_nothing_and_rights_never_grow`).
3. **Creation is explicit and fails closed.** Retype names its target slots; an
   occupied slot, a region too small or a kind that may not be retyped changes
   nothing. Spawner capabilities cannot be made by retype (they exist only as the
   root task's boot capability). *built*.
4. **Destruction never fails halfway.** Teardown of a capability space asserts
   that every slot is empty before the space's own frame is freed; a live slot
   linking into a reused frame would be a use-after-free. *built*
   (`destroying_a_cspace_returns_every_frame_and_block`).
5. **A destroyed object wakes its waiters with an error.** Whoever is blocked on
   an object when it is destroyed is woken with `Aborted`, exactly those waiters
   and no others (exact object address, not an address range). *built* for
   endpoints and notifications
   (`ipc_waiters_are_woken_with_an_error_when_the_endpoint_goes`,
   `notifications_coalesce_wake_fifo_and_abort`).
6. **No allocation, no blocking inside capability mutation.** Derive, delete and
   revoke work inside the slot memory; they do not allocate. Lock order is
   scheduler, then capability state, then frame (ARCHITECTURE.md section 5).
   Waiters are woken after the capability lock is released and before interrupts
   are restored. *built*; single CPU only (section 4 boundary note).
7. **Resource limits are fixed, not negotiated.** There are no per-task quotas
   yet. Today the limits are global constants: 64 threads (`MAX_THREADS`), 16
   reserved untyped blocks (`MAX_BLOCKS`), untyped blocks of 4 KiB to 16 MiB
   (`MAX_UNTYPED_BITS` = 24), a 128-slot space, at most 32 capabilities passed at
   spawn. Exhaustion returns `NoResources` and leaves no partial state
   (`sched_unstarted_spawn_releases_owned_spaces`). *built, global only*.
   A **quota** is defined here as: a bound on the untyped memory a task may
   *hold*, which is already enforced by construction once task building moves to
   the root task (1.5), because every object is paid for from an untyped region
   the task was given. No separate quota counter is planned; one would need an
   ADR first.

### 10.2 Rules by object kind

| Kind | Created by | Used by | Destroyed when | Waiters / dependents | Status | Tests (existing, or required before the item is done) |
|---|---|---|---|---|---|---|
| **Untyped** | Boot reservation (`reserve_untyped`) for the root task; derive of a smaller untyped | Retype, derive | Last capability deleted: the block returns whole to the frame allocator | Children are destroyed first (revoke) | built | `destroying_a_cspace_returns_every_frame_and_block` |
| **CNode** | Retype from untyped (1 to 128 slots, 4 KiB at most); the kernel makes each task's first CNode | `derive`, `move`, `delete`, `revoke` on the CNode a capability names, slot numbers relative to it; `retype` can fill it | Last capability deleted, or the owning task is reaped | Every capability inside is deleted first, so objects they hold are destroyed. A CNode capability never leaves its task and may only be stored in the task's own space (no recursion) | built (2026-11-22, TODO 1.2); *using a capability stored in it needs a cross-CNode move: **1.2b*** | capdemo steps 53 to 72 (ring 3); negative control: step 72 fails if contents are not deleted |
| **Endpoint** | Retype | send, recv, call (rights READ/WRITE/GRANT) | Last capability deleted | Blocked senders, receivers and callers woken with `Aborted` | built | `ipc_revoking_an_endpoint_cuts_clients_off_and_wakes_waiters` |
| **Notification** | Retype | signal (WRITE), wait (READ) | Last capability deleted | Blocked waiters woken with `Aborted`; pending badge bits are discarded with the object | built | `notifications_coalesce_wake_fifo_and_abort`, `notification_ring3_blocked_delivery_and_revocation` |
| **Frame** | Retype (Kind exists; mapping by invocation is not built) | Map (**1.4**) | Last capability deleted, after every mapping of it is removed | A mapped frame must be unmapped before its memory can be reused | target: **1.4** | 1.4: mapping, double mapping, revoking a mapped frame, teardown with no stale mapping |
| **Spawner** | Boot only (root task slot 2); never by retype | `spawn`, `join` | Deleted with the root task's space | None | interim (ADR-0011), retired by **1.5** | `join_waits_for_a_thread_and_remembers_the_last_64_that_finished`; 1.5: the same programs start with no spawner |
| **Thread** | Reserved kind; not creatable | Configure, start, stop, join (**1.3**) | Revoking the memory the thread was retyped from stops it, then frees it | The thread's blocked IPC is cancelled (10.4); joiners are woken | target: **1.3** | 1.3: root task starts a thread it paid for; revoking its memory stops it cleanly |
| **PageTable / address space** | Reserved kind; the kernel builds each task's space today | Map, unmap (**1.4**) | After all frames it maps are unmapped and the owning thread has stopped | Table frames returned to their untyped | target: **1.4** | 1.4: user-half teardown leaves no stale mapping; kernel half is never writable from a task |
| **Reply** | Implicit: `call` records the caller as waiting for a reply from that server. It is not a slot today | `reply` (once) | Consumed by the reply, or by the server's exit | The caller is woken with an error if the server exits without replying | interim: a reply is thread state, not a capability; made a real GRANT_REPLY capability by **1.6** | `ipc_call_and_reply_carry_words_and_the_badge`, `ipc_a_caller_is_woken_when_the_server_exits_without_replying`, `ipc_reply_without_a_caller_is_refused` |

### 10.3 Teardown order of a dying task

A task ends by `exit`, by a fault in ring 3, or because it was killed. All three
go through one path. The order, and why:

1. **Mark dead and publish the result.** The thread's state becomes `Dead`, its
   exit code (or fault vector and address, or the killed mark) is stored and its
   `done` flag is set. *built.*
2. **Abort the callers waiting on it.** Every thread blocked waiting for a reply
   from this thread is woken with an error (`abort_reply_waiters`). *built.*
3. **Record the exit and wake joiners.** The result goes into the last-64 exit
   history; every thread blocked in `join` on this id is woken with it
   (`record_exit`). *built.* A thread joined after it fell out of the history gets
   an error, not a stale result.
4. **Switch away; the thread becomes the single zombie.** It is not reaped on its
   own stack. The next thread to run drops it. *built.*
5. **Reap, in `Drop for Thread`:** first the user half of the address space (user
   pages and tables, never the shared kernel half), then the capability space.
   *built.* Order matters: while the address space exists the kernel half is
   shared, and no capability destruction may depend on a user mapping of its
   memory.
6. **Destroy the capability space** (`CSpace::destroy`): first revoke every
   root untyped region, with everything carved from it, so each reserved block
   returns to the frame allocator through its own capability; then delete every
   remaining slot; then assert that all slots are empty; then free the space's
   frame. The lock is taken per slot, and waiters of destroyed endpoints and
   notifications are woken between regions. *built.*
7. **Finally the kernel stack slot is released.** *built* (the stack `Drop`).

When thread and address-space objects exist (1.3, 1.4), step 5 splits: a task's
address-space object is destroyed through its capability, not unconditionally by
the dying thread, and a thread revoked from under its owner goes through steps
1 to 4 as if it had been killed.

### 10.4 Cancellation of blocked calls

| A task is blocked in | It is woken with `Aborted` when | Status |
|---|---|---|
| `send` / `recv` on an endpoint | the endpoint is destroyed | built |
| `call`, waiting for the reply | the endpoint is destroyed, or the server exits without replying | built |
| `wait` on a notification | the notification is destroyed | built |
| `join` on a thread | the thread exits (the woken task gets its result, not an error) | built |
| any of the above, and a kill was requested | the kill request: a blocked **user** task is woken so it can leave the system call, then it exits | built (`blocked_user_ipc_tasks_can_be_killed`) |

Rules that follow:
- A woken waiter must re-check why it was woken; it must never assume the object
  still exists. *built.*
- A kill reaches a user task running in ring 3 at the next timer tick; a task
  running kernel code finishes that kernel work first. A kernel thread blocked in
  IPC is not killable. *built.*
- Timeouts, and cancelling a call whose server is alive but slow, are not part of
  M10. They need scheduling contexts (section 8) and are a later decision.

### 10.5 Tests the remaining items must add

The existing tests above cover the rows marked built. Items 1.2 to 1.7 each add
the tests named in their row of 10.2 and TODO.md. In addition, the M10 exit
tests (1.7) must show these lifecycle properties from ring 3, each with a
negative control (the check fails when the guarded behaviour is removed):
1. After a task is reaped, `reserved_blocks` and the frame count return to what
   they were before it was spawned (no leak) for a task that exited, one that
   faulted and one that was killed while blocked in IPC.
2. A capability held by a dead task's peer cannot be used to reach the dead
   task's endpoints: the next use fails.
3. Revoking an untyped region destroys every object retyped from it, in every
   task that holds a derived capability, and wakes their waiters.
4. No object memory is reusable until the last capability to the old object is
   gone (retype after partial revoke does not overlap).

### 10.6 What this section does not settle

Process or thread priorities, time budgets, timeouts, interrupt objects (M11),
more than one CPU, and the exact `ReplyRecv` and reply-capability wire format
(decided when 1.6 starts, as a short ADR if it changes ADR-0009). It states rules
for the code as it will be; it is not evidence that any property holds.
