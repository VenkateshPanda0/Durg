# ADR-0015: Formal-verification options for `durg-cap`

- **Status:** Proposed (2026-11-24). A draft for the owner; **nothing is decided and no verification work has been done.** The project makes no verification claim anywhere and this ADR does not change that.
- **Needed by:** nothing is blocked on it. It is worth deciding before M13, when the capability rules start carrying approval and label semantics.

## Context

`crates/cap` holds the pure capability rules (slots, derivation lists, revocation, untyped retyping). It is `#![forbid(unsafe_code)]`, has no allocation and no I/O, and is already tested by host unit tests, property tests with proptest, a libFuzzer target (`cap_ops`) that checks the structural invariants after every operation, and mutation checks. The six invariants the design relies on are written down in [design/m10-capability-core.md](../design/m10-capability-core.md) section 2: list consistency, rights only shrink, nothing derived survives a revoke, no overlap between untyped-derived objects, the last capability frees an object exactly once, a badge never changes.

Tests show that those invariants hold on the inputs tried. A proof tool would show they hold for **all** inputs within its model. The kernel is explicitly "not formally verified" ([ADR-0001](0001-locked-decisions.md), [UNSUPPORTED.md](../UNSUPPORTED.md)). The question is whether a small, bounded proof effort on the crate that carries the thesis is worth its cost, and with which tool.

## Options

1. **Do nothing now.** Keep tests, property tests and fuzzing; revisit at M13. No cost; the thesis stays "tested", never "proved".
2. **Bounded model checking with Kani.** Write proof harnesses next to the existing property tests (symbolic slot arrays of a small fixed size, any sequence of up to N operations) and run `cargo kani`.
   - Pro: reuses the Rust code as it is; no new syntax; the harnesses read like the property tests. Fits a `no_std`, `forbid(unsafe_code)` crate.
   - Con: bounded (small arrays, short sequences), so it proves less than a deductive proof; needs the Kani toolchain, separate from the pinned rustc; loops over slot lists need unwinding bounds.
3. **Deductive verification (Verus, Prusti, Creusot).** Annotate the crate with specifications and prove the invariants for all sizes and sequences.
   - Pro: the strongest result; research projects have used this style for verified systems code.
   - Con: the crate must be rewritten in the tool's dialect or annotated heavily; the tools have their own toolchains (Verus pins its own rustc), maturity and maintenance differ and must be checked before relying on any of them; proof effort is large and ongoing as the crate changes.
4. **Prove a model, not the code.** Write the rules in TLA+ or Alloy and check the invariants there.
   - Pro: quick to find design errors in revocation.
   - Con: proves the model, not the Rust; a divergence between the two is the usual failure.

## Recommendation

**Option 1 until M13, then a time-boxed spike of Option 2** on two properties only: invariant 3 (after `revoke(s)` nothing else refers to anything derived from `s`) and invariant 5 (deleting the last capability frees an object exactly once). Keep the spike out of CI until it is stable; record in the threat model exactly what the bounds are and say "bounded model checking", never "verified". Option 3 is a research project and should be a separate decision with named people and time.

## What the owner decides

- Whether to run the spike, and who maintains it.
- The wording rule: nothing in the repository says "verified" or "proved" without an artifact (harness plus a run) it points to.

## Consequences if accepted

A `crates/cap/proofs/` directory, a documented command, a threat-model evidence note with a "does not show" paragraph (bounds, the unmodelled kernel glue in `kernel/src/cspace.rs`), and an ADR superseding this one if the spike is kept or dropped.
