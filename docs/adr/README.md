# Architecture Decision Records

Significant decisions are recorded here as short documents: **context, options, decision, consequences**. If you disagree with a decision, open an issue that engages with its stated reasoning.

**Statuses:** `Proposed` (options and a recommendation, awaiting the owner's decision), `Accepted`, `Superseded by NNNN`.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-locked-decisions.md) | Locked foundational decisions | Accepted |
| [0002](0002-rust-toolchain-channel.md) | Stable vs nightly Rust | Accepted |
| [0003](0003-limine-bindings.md) | Limine bindings: own vs `limine` crate | Accepted |
| [0004](0004-ai-assisted-contributions.md) | Policy on AI-assisted contributions | Proposed |
| [0005](0005-repository-visibility.md) | Repository visibility | Proposed |
| [0006](0006-minimal-page-mapper-before-m5.md) | Minimal page mapper before M5, to reach the local APIC | Accepted |
| [0007](0007-kernel-owns-its-page-tables.md) | The kernel owns its page tables: W^X, minimal direct map, guarded stack | Accepted |
| [0008](0008-nightly-for-host-side-fuzzing.md) | A nightly toolchain, for host-side fuzzing only | Accepted (decided under the owner's delegation; reversible) |

New ADRs use the next free number and the same four sections.
