# Changelog

## Unreleased

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
