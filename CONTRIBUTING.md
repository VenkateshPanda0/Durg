# Contributing to Durg

Thanks for your interest. Durg is in the **design phase**: there is no bootable kernel yet, so most useful contributions right now are **review, critique, research and documentation**. Code contributions open up as the first milestones land.

By participating you agree to keep discussion respectful and focused on the work.

## Ways to contribute right now

| You enjoy... | Good fit |
|---|---|
| Finding holes in designs | Review the [five promises](README.md#the-idea-five-promises), then open an issue describing how you would defeat one |
| Security research | Look for trust-handoff paths the design might miss |
| Reading papers and specs | Summarise prior art (seL4, Asterinas, Genode, Zircon) or compare design options |
| Writing | Glossary entries, reading lists, proofreading, plain-language explanations |
| Newcomer eyes | Read the README cold and tell us what was confusing |

Issues labelled **`good first issue`** are scoped so you can finish them without prior knowledge of the project. Issues labelled **`help wanted`** are open design or research questions where outside input is especially welcome.

## Before you start

1. **Search existing issues** to avoid duplicates.
2. **Open an issue first** for anything larger than a typo or small fix, and say what you plan to do. We would rather talk for five minutes than have you redo work.
3. **Keep the scope in mind.** Some things are deliberately out of scope (see [What Durg is not](README.md#what-durg-is-not)): a GUI, real-hardware drivers, Linux ABI compatibility, formal verification claims. Proposals in those areas will likely be declined, with an explanation.

## Reporting security concerns

Durg has no released code, so a design flaw is a normal issue. If you find something you believe should not be disclosed publicly once code exists, say so in an issue without including the details, and we will arrange a private channel.

## Design decisions

Significant decisions are written down as short **Architecture Decision Records** (ADRs) in `docs/adr/`, with context, options considered, the decision and its consequences. If you disagree with a recorded decision, open an issue that engages with its stated reasoning. New evidence is the best argument.

## Claims must be honest

Every security claim in the code or docs must be backed by a test or by a written argument in the threat model. Please do not add wording like "secure", "safe" or "verified" without that backing. We would rather under-promise.

## When code contributions open up

These conventions apply once the workspace exists. Details will be filled in as the build lands.

- **Language:** Rust. Guest crates are `#![no_std]`.
- **`unsafe`:** confined to designated low-level modules. Every `unsafe` block needs a `// SAFETY:` comment stating the invariant it relies on.
- **Style:** `cargo fmt` and clippy clean. Prefer clear code over clever code, because the codebase doubles as teaching material. Comments explain *why*, not what.
- **Dependencies:** the guest kernel and services follow a strict originality policy. Adding a third-party crate to guest code requires an ADR. Host-side tooling is not subject to this.
- **Tests:** each milestone ships with automated tests that run headless in QEMU. Pure-logic code should also be unit-tested on the host.
- **Commits:** one logical change per commit with a meaningful message, for example `kernel/mm: add bitmap physical frame allocator`.

### Sign-off (DCO)

Contributions use the [Developer Certificate of Origin](https://developercertificate.org/) rather than a CLA. Sign each commit with `git commit -s`, which adds a `Signed-off-by:` line using your own name and email.

## Questions about tooling or process

Open an issue and tag it `question`. A separate policy for AI-assisted contributions is still being decided, so if that affects you, please ask in an issue before contributing.

## License

By contributing, you agree that your contributions are licensed under the [Apache License 2.0](LICENSE).
