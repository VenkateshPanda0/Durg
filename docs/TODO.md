# TODO: what to do next, in order

This is the working list: concrete tasks, in the order they should be done, each
with where the work lives and how to tell it is finished. The long-range plan
(all milestones, M0 to M29, and why they are ordered that way) is in
[ROADMAP.md](ROADMAP.md); how the system works today is in
[ARCHITECTURE.md](ARCHITECTURE.md). Update this file in the same pull request
that finishes or adds a task.

**How to take a task:** comment on its issue (or open one), then follow
[contributor-quickstart.md](contributor-quickstart.md). A task is done only when
its "done when" is shown by checks you actually ran, recorded in the pull request.

Status legend: `[ ]` open · `[~]` started · `[x]` done.

## Where we are

| Phase | Milestones | Status |
|---|---|---|
| Foundations: thesis, threat model, build, CI | M0–M1 | [x] done |
| A kernel that boots and survives: serial, traps, timer, memory, heap | M2–M6 | [x] done |
| Tasks and isolation: threads, ring 3, syscalls, ELF loader | M7–M9 | [x] done |
| Capabilities: derive, revoke, retype, IPC, root task | M10 | [~] in progress |
| Drivers, storage, trust services, agent demo | M11–M14 | [ ] not started (virtio validation groundwork exists) |
| Desktop/laptop expansion | M18–M29 | [ ] planning only (M18 gate) |

M10 so far: the `durg-cap` crate; per-task capability spaces and `invoke`;
untyped memory and retype (including empty CNodes); endpoint IPC with badges and
capability transfer; notifications; an interim root-task spawner and `join`;
cancellation of blocked user tasks; single-CPU hand-off hardening.

## 1. Finish M10: the capability and task lifecycle (current priority)

Read first: [design/m10-capability-core.md](design/m10-capability-core.md),
[ADR-0009](adr/0009-capability-object-model-and-syscall-shape.md),
[ADR-0011](adr/0011-spawner-capability-as-an-interim-step.md),
[book/m10-capability-core.md](book/m10-capability-core.md).

- [x] **1.1 Write the lifecycle rules down before more code.** Who owns each
  object, what a task's death wakes or frees, how blocked calls are cancelled,
  what a quota is, the order of teardown. *Where:* a new section in
  `docs/design/m10-capability-core.md`. *Done when:* every object kind in use has
  a create/use/destroy rule and a list of the tests that will check it.
  Done 2026-11-21: section 10 of the design doc.
- [x] **1.2 Attach and use a second CNode.** Done 2026-11-22: a task retypes a
  CNode (1 to 128 slots), puts capabilities in it (`retype`'s third argument names
  the CNode, 0 is its own space), and runs `derive`, `move`, `delete` and `revoke`
  on it; deleting it deletes what it holds. Tested from ring 3 (capdemo steps
  53 to 72); negative control: with the delete-contents step removed, step 72 fails.
- [x] **1.2b Move between two CNodes.** Done 2026-11-22: invoke label 10,
  `move_from`, on the destination CNode capability: arguments are the source
  CNode's slot in the caller's space, the slot to take, and the slot to fill. A
  capability to a CNode can only move into the task's own space. Tested from ring
  3 (capdemo steps 73 to 83: a notification stored in a second CNode is moved into
  the own space, signalled and waited on); negative control: with the CNode guard
  disabled, step 82 fails. No copy between CNodes yet (not needed by any task).
- [ ] **1.3 Thread objects.** Implement `Kind::Thread`: retype from untyped,
  configure (address space, capability space, entry, stack), start, stop, join.
  *Where:* `crates/cap`, `kernel/src/sched.rs`, `kernel/src/cspace.rs`,
  `crates/syscall` labels. *Done when:* the root task starts a thread it paid for
  from its own memory, and revoking the thread's memory stops it cleanly.
- [ ] **1.4 Address-space and page-table objects.** Map, unmap and protect frames
  by invocation; validate page tables a task supplies; enforce W^X and the user
  half; unmap a frame before its memory can be reused. *Where:*
  `kernel/src/mm/paging.rs`, `kernel/src/cspace.rs`, `crates/paging`. *Done when:*
  tests cover mapping, double mapping, revoking a mapped frame, and teardown with
  no stale mapping left.
- [ ] **1.5 Move task building into the root task; retire the spawner.** ELF
  loading and initial authority assembly happen in `user/src/bin/init.rs` using
  1.3 and 1.4. *Done when:* the kernel allocates nothing for tasks after boot
  (threat T9 can be demonstrated), ADR-0011 is marked superseded, and the boot demo
  still passes.
- [ ] **1.6 Finish the IPC surface.** Replies that carry a capability,
  `reply_recv`, user-visible task control (stop, wait) as a capability, and
  notifications that can be bound to interrupts. *Done when:* each has ring-3
  tests including cancellation and revocation while blocked.
- [ ] **1.7 M10 exit tests.** Cross-task revocation, rights attenuation,
  low-memory rollback, adversarial object sequences, and a capability fuzz run in
  the CI budget decision. *Done when:* the M10 exit criteria in
  [ROADMAP.md](ROADMAP.md#m10--finish-capability-and-task-lifecycle-current-engineering-priority)
  are met and recorded in the book chapter and devlog.

## 2. In parallel: the M18 architecture and hardware gate (planning)

These are decisions, not code. Each ends in an ADR. See
[ROADMAP.md, M18](ROADMAP.md#m18--product-scope-and-architecture-gate-new-start-now),
[design/desktop-threat-model-draft.md](design/desktop-threat-model-draft.md) and
[design/hardware-qualification-plan.md](design/hardware-qualification-plan.md).

- [ ] **2.1** Name one desktop and one laptop to target, with exact components.
- [ ] **2.2** Decide driver reuse, the GPU/compositor route, the application
  ABI/porting route and the browser, each with feasibility evidence.
- [ ] **2.3** Compare extending the Durg kernel with a Linux desktop hosting Durg
  as a guest, and record the outcome.
- [ ] **2.4** Expand the threat model for devices, networks, multiple users,
  updates and recovery.
- [ ] **2.5** State the maintenance and support capacity honestly (people, time,
  hardware access).

## 3. Next milestone after M10: M11 drivers in the VM

- [ ] **3.1** Contiguous, DMA-owned frame allocation (reuse the aligned-run search
  in `crates/pmm`).
- [ ] **3.2** Device and interrupt capabilities; interrupt delivery through
  notifications (needs 1.6).
- [ ] **3.3** A user-space virtio console driver, then block, then vsock, using the
  validators in `crates/virtio`.
- [ ] **3.4** Calibrate the timer before anything waits for a duration.
- [ ] **3.5** Driver crash and restart tests.

## 4. Small tasks, good for a first contribution

- [ ] Write the glossary: [docs/glossary.md](glossary.md) ([#3](https://github.com/VenkateshPanda0/Durg/issues/3)).
- [ ] Write the prior-art reading list: [docs/reading-list.md](reading-list.md) ([#4](https://github.com/VenkateshPanda0/Durg/issues/4)).
- [ ] Read the README cold and report what confused you ([#5](https://github.com/VenkateshPanda0/Durg/issues/5)).
- [ ] Add host-side tests for serial output cleanup ([#9](https://github.com/VenkateshPanda0/Durg/issues/9)).
- [ ] Review the decision to write our own Limine bindings ([#8](https://github.com/VenkateshPanda0/Durg/issues/8)).
- [ ] Attack the five promises, list trust-handoff paths, compare journal schemes,
  review label semantics ([#1](https://github.com/VenkateshPanda0/Durg/issues/1),
  [#2](https://github.com/VenkateshPanda0/Durg/issues/2),
  [#6](https://github.com/VenkateshPanda0/Durg/issues/6),
  [#7](https://github.com/VenkateshPanda0/Durg/issues/7)).

## 5. Housekeeping (owner or maintainer)

- [ ] Upload `docs/assets/social-preview.png` in the repository settings.
- [x] Workflow actions are pinned to full commit SHAs (2026-11-24) and Dependabot
  proposes updates weekly; newer majors that run on the current Node runtime arrive
  as Dependabot pull requests to review.
- [ ] Decide whether a short fuzz run belongs in CI (it would bring nightly into CI).
- [ ] Keep the expected self-test count in the README in step with the code when
  tests are added.

- [ ] **Burn down `clippy::multiple_unsafe_ops_per_block`**, then enable it as a
  workspace lint. Measured on 2026-11-24: 18 blocks in the kernel (selftest build)
  and 6 in the host crates contain more than one unsafe operation. Fixing them
  means splitting `unsafe` blocks in security-critical code, so it is a reviewed
  change of its own, not part of the CI work.
- [ ] **Document the public items of the library crates**, then enable
  `missing_docs` as a warning. Counted on 2026-11-24: 67 undocumented items
  (acpi 11, paging 16, sync 3, syscall 33, elf 4; `cap` and `virtio` already deny it).
- [ ] **Propose a split of `kernel/src/sched.rs`** (1,421 lines) before anyone changes it:
  the scheduler is security-critical and in active M10 use, so the split is the
  owner's decision, not part of the self-test split. Candidate seams: the thread
  and stack types, the run queue and context switch, the blocking IPC and
  notification waits, and `join`/exit bookkeeping.
- [ ] Raise the coverage floor in `ci.yml` (86% lines on 2026-11-24) as coverage grows;
  `xtask/src/main.rs` is not unit-tested (0%).

## 6. Known debts (not scheduled yet)

- [ ] **The debug kernel faults under KVM on AMD hosts.** Found 2026-11-24, the first time
  CI ran under KVM. On hosted runners with an `AMD EPYC 9V74` the debug `selftest` (and
  `fail-demo`) variant died in `task_a_and_task_b_cannot_read_each_others_memory` with
  `EXCEPTION: General Protection (vector 13), error code 0x18, cs 0x0008, rsp
  0xfffffd8000030fd8, rflags 0x10086`; the same boot passed 24 of 24 times on runners with an
  `Intel Xeon 6975P-C`, the release build passed on both, and software emulation (TCG)
  always passes (locally and in CI). The code did not change between a passing and a
  failing run, so it depends on the host CPU. **Hypothesis, not established:** `rsp` is
  exactly 0x28 below the top of a thread's kernel stack slot, which is where the five-word
  `iretq` frame for a *new* user thread sits, and error code 0x18 is GDT index 3 (the user
  data segment), so the `iretq` that enters ring 3 for the first time in a fresh thread is
  rejected; something in that frame or in how the CPU treats it differs on AMD. Next
  steps: capture the fatal fault block and its disassembly on an AMD runner (the temporary
  `kvm-diagnose.yml` workflow does this), then fix the kernel in a reviewed change. Until
  then the required boot test runs under TCG and the KVM job (`boot-test-kvm`) is
  informational.

Recorded so they are not forgotten; each has a home in the roadmap's backlog table.
One CPU only (locks, syscall entry and scheduler are not SMP-safe); no NMI or
machine-check handling; the timer is counted, not calibrated; no kernel or user
address randomisation; the heap has no poisoning or red zones; some `sysret`
defences are untested; the direct map gives the kernel a writable alias of every
user page; empty page tables are not reclaimed on unmap.

## 7. Needs other people or decisions (written up, not built)

Each item says what it needs. None is started.

- **An external security review** of the kernel's `unsafe` code, the system-call
  entry and exit path (`kernel/src/arch/x86_64/syscall.rs`, `kernel/src/syscall.rs`),
  and capability handling (`kernel/src/cspace.rs`, `crates/cap/`). A reviewer needs:
  a **frozen commit** (a tag, not `main`); the [threat model](THREAT_MODEL.md) and
  [UNSUPPORTED.md](UNSUPPORTED.md) so scope is clear; the **test evidence** (the CI
  run for that commit, `cargo xtask test` output, fuzz and Miri logs, the coverage
  report); an inventory of `unsafe` blocks with their `// SAFETY:` text (about 200
  occurrences today, each justified; the `undocumented_unsafe_blocks` lint is on);
  [build and boot instructions](contributor-quickstart.md) and the pinned toolchain
  and bootloader ([ADR-0014](adr/0014-pin-the-bootloader-by-commit-and-digest.md));
  the list of known debts (section 6 above); and a named person to answer questions.
  The owner chooses the reviewer and funds it; the assistant cannot.
- **Formal verification of `durg-cap`:** options and a recommendation in
  [ADR-0015](adr/0015-formal-verification-options-for-durg-cap.md). Needs the owner to
  decide whether to run a time-boxed spike (suggested: Kani, after M12).
- **Rust edition 2024:** pros, cons and a staged plan in
  [ADR-0016](adr/0016-rust-edition-2024-migration.md). Needs the owner's decision;
  not before M10 settles.
- **Name and trademark check for "Durg"**, and a project domain. Needs the owner: a
  search of trademark registers in the countries that matter, and a registrar account.
- **Funding and organisation** (a GitHub organisation, sponsorship, who is
  accountable). Needs the owner; moving the repository to an organisation changes
  URLs, CODEOWNERS and the ruleset, so do it deliberately.
