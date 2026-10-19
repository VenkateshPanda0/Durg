# ADR-0002: Stable vs nightly Rust

- **Status:** Accepted (2026-10-15)
- **Needed by:** M1

## Context

A bare-metal Rust kernel often reaches for nightly-only features: `build-std` (to build `core` and `alloc` for a custom target), `abi_x86_interrupt` for interrupt handlers, and `naked` functions for entry stubs. Nightly makes builds less reproducible. Stable is easier for contributors and CI. What is actually required must be checked against the toolchain at the time M1 starts, because several of these features have been moving toward stable.

## Options

1. **Stable only.** Use the prebuilt `core` for a supported bare-metal target, write entry and trap stubs as assembly, avoid nightly features.
   - Pro: reproducible, easy for newcomers, no surprise breakage.
   - Con: may not be fully possible, and more hand-written assembly.
2. **Pinned nightly.** Pin an exact nightly date in `rust-toolchain.toml` and use `build-std` and other features freely.
   - Pro: most convenient for kernel work.
   - Con: contributors must install a specific nightly, and the pin must be bumped deliberately.
3. **Stable by default, nightly only where proven necessary.** Start on stable. Move to a pinned nightly only for a feature that cannot be avoided, and record it here.

## Decision

**Option 3.** Stable by default. Move to a pinned nightly only for a feature proven unavoidable, and record each such case in this ADR. The toolchain is pinned in `rust-toolchain.toml`.

## Consequences

- M1 starts on stable. Entry code uses `core::arch::asm!`, which is stable. No `build-std` is needed for `x86_64-unknown-none`, because rustup ships a prebuilt `core` for it.
- If a later milestone needs a nightly-only feature (for example custom interrupt ABIs or naked functions), it is added here with the reason, and the pin changes.

## Addendum (2026-10-19)

The one use of nightly so far is **host-side fuzzing** with `cargo-fuzz`, recorded in [ADR-0008](0008-nightly-for-host-side-fuzzing.md). It does not touch the kernel, CI, or `rust-toolchain.toml`.
