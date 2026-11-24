# ADR-0016: Migrating to Rust edition 2024

- **Status:** Proposed (2026-11-24). A draft for the owner. **No migration has been done.** Every crate is on edition 2021 (now set once in `[workspace.package]`).
- **Needed by:** nothing. The pinned toolchain (1.97.0) supports edition 2024.

## Context

Edition 2024 changes several defaults that matter to a kernel: `unsafe_op_in_unsafe_fn` is a warning by default (the workspace already denies it), `static mut` references are denied, `#[no_mangle]` and similar attributes must be written `#[unsafe(no_mangle)]`, `extern` blocks need `unsafe extern`, the scope of temporaries in tail expressions and `if let` changes, return-position `impl Trait` captures more lifetimes, `gen` becomes a reserved keyword and Cargo's resolver 3 prefers dependency versions compatible with the declared `rust-version`. `cargo fix --edition` automates most of it.

Counts in this tree (2026-11-24): 14 `no_mangle` uses and 7 `static mut` uses in the kernel, user programs and crates, about 200 occurrences of `unsafe`.

## Options

1. **Stay on 2021.** No churn in the trusted code; the lints the project cares about are already set by `[workspace.lints]`.
2. **Migrate everything in one behaviour-neutral change, after M10 settles.** A mechanical `cargo fix --edition`, then review, then the full gate including both boot profiles.
   - Pro: stricter defaults around `unsafe` and `static mut`, which is the project's main risk surface; fewer reasons for a contributor's tooling to disagree with the pinned config.
   - Con: the diff touches security-critical files; temporary-scope and lifetime-capture changes can change behaviour silently in rare cases, which the boot tests and self-tests may not catch; the fuzz crate and any dependencies on nightly must follow.
3. **Migrate crate by crate**, pure crates first (`cap`, `syscall`, `elf`, `paging`, `pmm`, `virtio`, `acpi`), then `heap` and `sync`, last the kernel and user programs.
   - Pro: smaller reviews; the host-tested crates have the best test coverage to catch regressions.
   - Con: a period with mixed editions; shared macros and traits must work in both.

## Recommendation

**Option 3, not before the M10 capability and task lifecycle work lands**, so the migration does not conflict with it. Each step is a behaviour-neutral pull request with `cargo fix --edition`, a human read of every diff in a file that contains `unsafe`, and the full gate. Do not combine it with feature work.

## What the owner decides

Whether to migrate at all, and the order. Until then, edition 2021 stays and this ADR stays `Proposed`.
