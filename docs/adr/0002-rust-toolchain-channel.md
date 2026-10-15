# ADR-0002: Stable vs nightly Rust

- **Status:** Proposed (awaiting owner's decision)
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

## Recommendation

**Option 3.** Try stable first at M1. Verify by actually building, not from memory, which features are still nightly-only. Whatever is chosen is pinned in `rust-toolchain.toml` and recorded here as the decision.

## Consequences

To be filled in once decided.
