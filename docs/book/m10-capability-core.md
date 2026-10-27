# M10 (steps 1 to 3): the capability core, the kernel that hosts it, and messages between tasks

**Goal:** the heart of the project. A program can only touch what it holds a capability for; a capability can be passed on with *fewer* rights, and taken back (revoked) along with everything derived from it. M10 builds this in steps; this chapter covers **step 1**, a pure crate (`crates/cap`, `durg-cap`) that has no kernel in it yet, so every rule can be attacked with property tests before anything depends on it. The design is in `docs/design/m10-capability-core.md`; the decision in ADR-0009.

## The idea in four sentences

A *capability* names one kernel object and says what you may do with it. It lives in a *slot* in a table you own; you refer to it by slot number and by nothing else, so you cannot name an object you were never given. You can *derive* a copy with fewer rights (never more), optionally stamped with a badge so a server can tell its clients apart. You (or whoever gave you the object) can *revoke* it, which deletes every copy derived from it.

## The data structures

A slot is 32 bytes: what it names (kind, address, size), the rights, an `extra` word (a badge for endpoints; the watermark for untyped memory), and its place in the **derivation list**: `prev`, `next` and `depth`. Everything is *inside the slot*, so deriving and revoking never allocate, which is the locked rule "the kernel does not allocate on behalf of tasks after boot".

The derivation list keeps descendants right after their ancestor. `depth` makes that checkable: the descendants of a slot are the slots after it with greater depth, up to the first slot that is not deeper. So `revoke(s)` is "delete the slot after `s` until the next slot is not deeper than `s`."

**Untyped memory.** The kernel never hands out object memory; a task holds an *untyped* capability for a region and *retypes* it into objects. The region has a **watermark**: new objects are carved above it (aligned to their own size) and it moves up. It returns to zero only when no descendant is left, so memory is never reused while a stale capability to it exists.

## Choices, and what was rejected

- **Untyped capabilities cannot be copied.** Two copies could each retype the same memory into overlapping objects. They can only be *subdivided*. (seL4 has a similar rule.)
- **Deleting a slot with descendants moves them up one level** to hang off its parent, instead of deleting them or orphaning them. Without that the depth chain would break.
- **"Is this the last capability to the object?"** is decided by looking at the two neighbours, not by scanning everything: copies of an object are inserted next to their parent, so they stay adjacent. A scan is far too slow inside a kernel. The property tests check the cheap rule against a scan on every slot after every operation.
- **A refused operation changes nothing.** Every operation validates first, then changes.
- **Zero bytes are an empty slot, and slots are copied, never borrowed.** Retype hands out zeroed memory, so zeroed memory must already be a valid capability space; links are stored as `index + 1` for that. The store trait loads and stores whole slots (`from_bytes` refuses damaged ones), so the kernel can keep its slots behind a physical-memory window without handing out references.
- **Flat slot arena for now.** Slots are named by `u32` indices into a `SlotStore`. The kernel will implement the trait over real capability-space memory; the single-level space is ADR-0009's option C.

## How it is tested

1. **Scenario tests** pin each rule down (rights, badges, alignment, out of memory, delete, revoke, move, nested regions, depth limit).
2. **A property test** runs random sequences of up to 60 operations (derive, move, delete, revoke, retype, with deliberately out-of-range slots) from a random-sized region, and after every step checks: `check_invariants` (links symmetric and acyclic, depth steps of one, a parent's depth exactly one less, a copy never stronger than its parent, badges never change, the cheap last-capability rule equals a scan, objects carved from a region lie inside the used part, aligned and disjoint, no watermark without descendants, one untyped capability per region); that a refused operation left the store identical; that the objects that vanished are exactly the objects reported destroyed, each once; that only retype can create objects; that `revoke` leaves no descendant and returns the right count.
3. **Mutation checks.** I broke the logic on purpose in 22 ways (the 15 below, plus 6 for the byte layout, all caught: unknown kind, padding, bad right bit, a dirty "empty" slot, rights not written, link encoding off by one). The first 15 were (drop the rights check, allow re-badging, allow copying untyped, skip moving descendants up, never reset the watermark, never report destruction, revoke only one child, no alignment, an off-by-a-bit memory limit, watermark not advanced, objects sharing an address, last-capability check ignoring a neighbour, move not fixing neighbour links, insertion not fixing the next link, a same-size child region, a wrong parent test). **All 15 are caught.** Two went uncaught on the first run: one was dead code (a watermark reset in `revoke` that `delete` already does; removed) and one was a real gap (a wrong parent lookup, caught only once the checker required a parent to be exactly one level up).

## Bugs and surprises

- **A real design flaw, found by the property test.** Retyping a region into a child untyped *as big as its parent* gave the child the parent's identity (kind, address, size), so the code treated it as a *copy* of the parent. A child region must now be strictly smaller. The scenario tests had not thought of it; random sequences found it in under a second.
- **My own test was wrong:** "a copy stronger than its parent" set the child to all rights, but the parent already had all rights. Fixed by deriving a read-only parent first.
- **A blind spot I did not fix, and wrote down.** Two sibling objects with identical kind, address and size cannot be told from reparented copies of one object, so the checker cannot detect that particular corruption. Retype never produces it; the test says so.

## What this does not show

Nothing here runs in the kernel yet, and no isolation property depends on it until step 2 puts it behind a lookup and step 5 runs a test through it. It is not formally verified. The tests exercise a flat arena of 12 slots; a bug that needs a much larger structure would not show. No fuzz target exists yet for this crate.


---

# Step 2: the kernel hosts capability spaces

**Goal:** a task can hold capabilities, name them by slot number, and call the kernel to use them; nothing it was not given is nameable. This is the first time the capability rules run inside the kernel.

## What was built

- **A capability space per task** (`kernel/src/cspace.rs`): one 4 KiB frame of 128 slots, with slot 0 a capability to the space itself. The kernel creates it zeroed, which is already a valid empty space because an all-zero slot is empty (step 1's byte layout). A task with *no* space can name nothing: every `invoke` returns "invalid slot".
- **Physical slot numbers.** The derivation list links slots by `physical address / 32`, and the kernel reaches them through the direct map (`PhysSlots`, which loads and stores whole slots and refuses damaged ones). A task cannot write those frames, so it cannot forge a link.
- **Untyped memory comes from the frame allocator, not from thin air.** At boot the kernel reserves a naturally aligned 4 MiB block (a new `find_free_aligned_run` in the pure `durg-pmm` crate, property-tested against a brute-force scan) and gives it to the root task as an untyped capability in slot 1. Only blocks the kernel reserved go back to the frame allocator when their last capability goes; a child region carved from one never does, because its memory is still its parent's. The kernel zeroes what retype hands out and nothing more.
- **One new system call, `invoke`**, with up to five arguments (`rdi, rsi, rdx, r10, r8`): `cptr`, a label, three arguments. Operations: identify, retype (on an untyped capability), and derive, move, delete, revoke (on the caller's own CNode capability, acting on slots of its own space). Labels with stray bits are refused, not masked. Pure decoding and packing live in `durg-syscall` with property tests; errors have documented codes.
- **Teardown.** When a task is reaped its space is destroyed: regions are revoked, every capability deleted, the reserved blocks returned, the frame freed; the test counts frames to prove nothing leaks.
- **The root task now does something.** `init` checks that it holds its own space and 4 MiB of untyped memory and nothing else, makes a frame from it, and exits. Started *without* its space, it says so and exits with an error instead.

## Choices

- **Refuse what is not built yet instead of half-building it.** Retyping a CNode, thread or page table is refused: each needs initialisation and teardown (and, for CNodes, a rule for the capabilities inside) that does not exist yet. A task cannot delete the last capability to its own space.
- **Own space only.** Derive, move, delete and revoke act on the caller's own CNode. The same operations on someone else's CNode need the right capability *and* a way to hold it, which is IPC.
- **At most 1 MiB of zeroed objects per retype**, because zeroing happens with interrupts off; larger carve-outs are made in pieces, and smaller *regions* are not zeroed (their contents are zeroed when objects are made inside them).
- **A stricter release.** `release_block` only succeeds if every frame of the block was newly added to the pool; a double release panics instead of silently returning memory someone else may be using.

## How it was tested

Kernel self-tests (96 in total now): a fresh space holds only itself; retyped memory is zeroed and the memory beside it is not touched (checked with junk planted first); every error path changes nothing; rights shrink and never grow; destroying a space returns every frame and block (counted); a ring-3 program (`capdemo`, 39 steps, exit code = first failing step) does all of it through the real system-call path; a task with no space exits 100; the root task starts with exactly its memory; and 4,000 random `invoke` calls (wild slot numbers, garbled labels, every operation) leave every structural invariant intact, with a floor on how many calls of each operation must have succeeded so the run cannot be vacuous. **Negative controls:** 15 deliberate breakages of the kernel code; 13 were caught at once. Two were not, and each taught something: a "child region at the block's own address" test did not actually place the child there (the frames were carved first), and a block release that returned twice the frames was invisible because the extra frames happened to be in the pool already, so `release_block` was made strict. Both are fixed and caught now.

## Bugs and surprises

- **The random test was vacuous at first.** With random slot numbers it deleted the one root region within a few steps and then could never retype again; and a random `move` shifted the CNode capability away from slot 0, after which every "own space" call failed. The generator now looks where things are and protects the root region from delete and move most of the time. The activity floor is what exposed it.
- **An existing test assumed the old world.** The frame-accounting test did not know about blocks reserved as untyped memory, and an M9 test ran `init` without a capability space and expected success.
- **A slow invariant check.** Reading 128 slots through physical memory for every check timed the emulator out; the check now copies the slots once.

## What this does not show

No capability is ever passed between tasks (no endpoints, no IPC), so P1 is **not** shown: tasks are isolated by address spaces, not yet by capabilities. Objects have no behaviour; a frame cannot be mapped, and when it can, destroying a mapped frame must unmap it first. The kernel still creates every task itself.


---

# Step 3: messages between tasks

**Goal:** two tasks can talk, and the *only* way one can reach another is a capability to an endpoint the other listens on. This is the first time the thesis, "you can only act on what you were handed", is visible between tasks.

## What was built

- **Endpoints and four calls.** `send`, `recv`, `call` (send, then wait for the answer) and `reply`. A message is a 32-bit label and three words, and may carry **one capability**. Sending needs the *write* right on the endpoint capability, receiving needs *read*, and attaching a capability needs *grant*. A capability sent arrives in a slot the receiver named, as a *child* of the sender's, with the same rights and badge; so revoking the original reaches into the receiver's space.
- **Badges.** The receiver learns the badge of the capability the sender used, and the sender cannot change it (a badge is set once, when a copy is made). That is how a server tells its clients apart without any global name.
- **Blocking without allocating** (`kernel/src/sched.rs`). A thread that cannot be served at once is marked blocked and parked in a fixed-capacity list. The "queue" of an endpoint is simply the set of blocked threads that name it, ordered by a sequence number, so there is no per-endpoint state in memory that a task could damage, and no kernel allocation. If every thread is blocked the kernel panics with "deadlock" instead of hanging silently.
- **When waiting ends badly.** Destroying an endpoint (by revoking or deleting its last capability) wakes everyone waiting on it with an error. A caller waiting for a reply is woken with an error if the thread that took its call exits.
- **A lock-order wrinkle, avoided.** Waking waiters needs the scheduler lock, which is taken *before* the capability lock, so the capability code records the range of endpoint addresses it destroyed and wakes the waiters only after releasing its own lock.

## How it was tested

- **Six scheduler-level tests with kernel threads** and a made-up endpoint address: rendezvous in both orders, waiters on another endpoint left alone, first-come-first-served, call and reply with words and badge, an endpoint destroyed under blocked senders and receivers, a server that exits without replying, a reply with no caller.
- **Four ring-3 scenarios** where the kernel plays the root task, makes an endpoint and a frame out of untyped memory, and hands out *restricted, badged copies* (the server may only receive, the client only send): a client with the grant right sends a frame capability and the server sees it arrive; a client without it is refused; a **stranger** task with an empty space tries to send and receive on every one of 140 slots and reaches nobody, which the server proves by counting exactly the client's three messages; and revoking the endpoint's region cuts the client off *in its own space* and wakes a server that was waiting. Every scenario counts frames and blocks to prove nothing leaks. The ring-3 client also checks refusals (empty slot, out of range, wrong kind, a label with stray bits, receive without the read right) and that offering one's own capability space is accepted as a message but never delivered.

## Negative controls

I broke the message code on purpose in 17 ways: send without the write right, receive without the read right, a capability sent without the grant right, the badge not delivered (at the syscall layer and again between sender and receiver), the endpoint kind not checked, a capability space that can be sent, revoke not waking waiters, destroyed endpoints not recorded, waiters not served in order, a server's exit not waking its callers, an abort range that excludes its end, a call that forgets who to reply to, a capability never delivered, other endpoints' waiters matching, blocked threads not counted, and the reply's server not checked. **16 were caught** (several by a hang that the harness's timeout turns into a failure). **One survived on the first run and was a real gap:** nothing checked that a blocked thread is still counted as existing; a test now does, and the mutation is caught. **One stays uncaught and is an equivalent mutant under today's rules:** a caller can only leave the reply wait by being answered or by the server exiting, so a stale reply target cannot arise; the check is kept as defence in depth against a future change.

## Bugs and surprises

- Two of my own test expectations were wrong (the described capability includes its size; and the free-frame baseline must be taken after the previous test's thread stack has been reaped, a 16-frame difference that looked like a leak).
- Clippy's `vec_box` lint wanted the blocked list unboxed. The `Box` is deliberate: the context switch holds a raw pointer into each thread, so threads must never move.

## What this does not show

Tasks are still created by the kernel, not by a root task (no thread or address-space capabilities). A reply cannot carry a capability. There is one outstanding call per server thread, no `reply_recv`, no timeouts, and a blocked thread cannot be killed (it is woken only by its partner or by the endpoint going away). Notifications do not exist yet. One CPU only. The capability-structure invariant checker covers a single capability space, so it is not run on scenarios that pass capabilities between spaces; those are covered by the behavioural tests above instead.
