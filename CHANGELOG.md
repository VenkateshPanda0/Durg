# Changelog

## Unreleased

### Added
- CI rebuilt as parallel jobs behind one required check (`ci-ok`): fmt, clippy
  (host, user, virtio, six kernel feature sets), host tests, rustdoc with warnings as
  errors (host crates and the kernel), debug and release boot tests, a 60-second
  fuzz smoke run of all five targets, Miri on `durg-heap` and `durg-sync`, coverage
  with a floor, and `cargo deny`; read-only token permissions, concurrency
  cancellation, a Limine cache keyed by the pinned commit, a weekly 30-minute fuzz
  workflow. `cargo xtask` logs whether QEMU used KVM or software emulation.
- Workspace lint policy (`undocumented_unsafe_blocks`, `missing_safety_doc`,
  `unsafe_op_in_unsafe_fn`) and shared package metadata in the root `Cargo.toml`.
- Supply chain: Limine is fetched by commit and every Limine file copied into the
  image is checked against a pinned SHA-256 on each build, failing closed
  (ADR-0014); `deny.toml` and a `cargo deny` CI job (licences, advisories, sources);
  Dependabot for cargo and workflow actions; workflow actions pinned to commit SHAs.
- A task can use a second CNode it retyped: operate on it with `derive`, `move`,
  `delete` and `revoke`, fill it with `retype` (third argument), and deleting it
  deletes its contents; `move_from` moves a capability between two CNodes a task
  holds (TODO 1.2, 1.2b).
- M10 object lifecycle rules (ownership, teardown order, blocked-call
  cancellation, limits) in `docs/design/m10-capability-core.md` section 10
  (documentation only; TODO 1.1).
- Notifications: `signal` and `wait`, coalescing badges, FIFO waiters, wake-up
  with `Aborted` when revoked, tested from ring 3.
- Empty CNodes can be retyped from untyped memory (not yet attachable to a task).
- `durg-virtio`: split-queue layout and descriptor-chain validation, with a fuzz
  target (groundwork for M11; not a driver).
- The kernel's internal kill handle now wakes and stops blocked user tasks.
- Desktop/laptop scope: ADR-0012, the integrated roadmap (M18–M29), the supplied
  requirements and their coverage register, and draft desktop threat-model and
  hardware-qualification documents.
- Documentation structure: `docs/ARCHITECTURE.md` (the system as built),
  `docs/TODO.md` (ordered next tasks), a step-by-step getting-started guide in
  `docs/contributor-quickstart.md`, a full documentation index, and READMEs in
  `kernel/`, `crates/` and `user/`.

### Changed
- `kernel/src/selftest.rs` (4,560 lines) is now a module tree, `kernel/src/selftest/`,
  split along its existing groups (early boot and memory, heap, threads, user mode,
  ELF loading, capabilities, messaging). Same tests, same names, same order; no
  behaviour change.

### Fixed
- Four `unsafe` blocks had their justification written in a form the lint cannot
  see (a label variant or a comment above the statement); relabelled, no logic changed.
- Unresolved rustdoc links to feature-gated items in the kernel.
- Capability lookup and scheduler hand-off for `send`/`recv`, and capability
  mutation with waiter wake-up, run with interrupts masked, so another task cannot
  revoke and reuse an endpoint in between (single-CPU rule).
- Destroying an endpoint or notification wakes only its own waiters (previously a
  range of addresses could wake unrelated ones).
- Spawner authority is checked again before a child is published, and `join` is
  serialised with it; failed spawns release the scheduler lock before teardown.
- Untyped physical ranges are validated; reserved blocks are returned atomically;
  the invariant checker respects its slot range; spawners cannot be made by retype.
- The boot-time timer check now has a bound and reports failure instead of hanging.
- The syscall crate's documentation no longer refers to a private file.


## v0.1.0-alpha.1 — 2026-11-07

### Added
- Ubuntu/WSL setup script, boot-output GIF, social-preview asset, and issue templates.
- Clear project scope, validation instructions and next engineering priorities.

### Fixed
- Exit-history self-test warnings that prevented strict Clippy checks from passing.

### Current pre-alpha baseline
- x86-64 boot, physical/virtual memory, heap and preemptive scheduling.
- Isolated ELF user tasks, syscalls, capability derivation/revocation and endpoint IPC.
- An interim root-task spawner; M10 remains incomplete.
- Observed local validation on 2026-11-07: 136 host tests, 108 kernel tests in each
  of debug and release, six boot variants per profile, and four short fuzz runs.

No production release or end-to-end agent sandbox is claimed. Provenance,
undo, approval services, storage and the egress broker remain planned.
