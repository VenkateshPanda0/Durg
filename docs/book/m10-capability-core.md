# M10 (step 1): the capability core, before the kernel uses it

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
