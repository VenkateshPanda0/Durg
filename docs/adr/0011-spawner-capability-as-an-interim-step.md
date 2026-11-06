# ADR-0011: A spawner capability and `join`, as an interim step before thread and address-space objects

- **Status:** Accepted (2026-11-06), decided by the assistant under [ADR-0010](0010-delegated-project-authority.md). Meant to be replaced; reversible.
- **Needed by:** M10, to let a root task build the system instead of the kernel building it.

## Context

Until now the kernel created every task, so "which capabilities does a task start with" was decided by kernel code, not by a root task. The thesis needs the opposite: the kernel starts exactly one task, and that task decides everyone else's authority. The seL4-style way is for the root task to create thread, address-space, page-table and capability-space *objects* by retyping untyped memory and to map frames by invocation. That needs several new object kinds, mapping and teardown for each, and a way for the kernel to link user-supplied page tables safely. It is the right design and a large amount of work.

CLAUDE.md §7 also fixes a rule this interacts with: *the kernel never allocates on behalf of user tasks after boot*; tasks retype untyped memory they hold.

## Options

1. **Full objects first.** Honours the allocation rule. Delays everything that depends on a root task, including the flagship demo, by a long way, and the risky mapping code gets written before anything else exercises it.
2. **A spawner capability (chosen).** A new capability kind, `Spawner`, authorises `spawn(module, first slot, count)`: the kernel builds a task from a *boot module*, gives it a capability space with copies of the caller's chosen capabilities, and returns its id; `join(id)` waits for a task to finish and reports how it ended. The kernel allocates the new task's address space and one frame for its capability space.
3. **Keep the kernel creating all tasks.** Nothing to build, but the root task would distribute nothing and the thesis would stay untested at the level that matters.

## Decision

**Option 2**, with these limits so that it stays small and replaceable:

- Only programs that are **boot modules** can be started (named by number, never by a user pointer); a task cannot supply its own code.
- The kernel's allocation is **bounded by the existing 64-thread limit** and the frame pool. A task can spend kernel memory by spawning, so the spawner capability is a powerful authority and is held by the root task alone at boot.
- A new task starts with **only** its own capability space (slot 0) and copies of what the caller named, as *children* of the caller's capabilities (so the caller can revoke them), with the same rights and badges. Memory regions and capability spaces **cannot be copied**, so a new task gets no memory and no authority over any space. The copy is all-or-nothing: on any failure everything allocated is freed.
- A spawner capability is an ordinary capability: it can be copied with fewer rights (without the write right it cannot spawn) and passed on, which delegates the power.
- `join` is available to a holder of the spawner capability, so authority to *start* is also the authority to *wait*. The kernel remembers the last 64 exit statuses.

## Consequences

- **The promise "the kernel never allocates for tasks after boot" is not true yet.** `docs/THREAT_MODEL.md` says so (T9 stays "not demonstrated"), and the roadmap lists replacing this with thread, address-space and page-table objects.
- The root task now decides the capabilities of every other task, which is what lets the system be built and checked from ring 3 at every boot.
- The module numbers (`durg_syscall::module`) must match the order in `boot/limine.conf`; a self-test checks it.
- **When replaced:** the `Spawner` kind and the two operations are removed (or reduced to a loader that works on user-supplied objects), the module numbers go with them, and the tests that start tasks through `spawn` are rewritten against the object-based calls.
