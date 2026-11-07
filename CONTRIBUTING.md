# Contributing to Durg

Thanks for your interest. Durg is early. The kernel boots under QEMU, runs isolated user programs and has a test suite (see the README for the one-command build and test), but **the five core promises are not delivered end to end**: a partial capability system exists, while provenance, undo, approval and broker services remain planned. The most useful contributions right now are still **review, critique, research and documentation**; focused code contributions and adversarial tests can help complete M10.

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

1. **Find a task:** browse [good first issues](https://github.com/VenkateshPanda0/Durg/labels/good%20first%20issue) or [help wanted](https://github.com/VenkateshPanda0/Durg/labels/help%20wanted). Check whether someone is already working on it; leave a short comment if you want to take it on.
2. **Use the existing issue** for a scoped task; for a larger new change, open an issue describing the problem and approach before investing significant time.
3. **Keep the scope in mind.** Some things are deliberately out of scope (see [What Durg is not](README.md#what-durg-is-not)): a GUI, real-hardware drivers, Linux ABI compatibility, formal verification claims. Proposals in those areas will likely be declined, with an explanation.

## Your first pull request

1. Fork the repository and clone your fork. Create a branch for one focused change.
2. For documentation, edit the relevant Markdown and check links and spelling. For Rust logic, run the relevant host tests; see [the development guide](docs/contributor-quickstart.md) for commands and for running QEMU.
3. Push your branch to your fork and open a pull request against `main`. Link the issue using `Fixes #NUMBER` when it fully resolves the task.
4. Describe what changed, list checks you actually ran, and say what you could not run. Draft pull requests and questions are welcome.

You do not need to know kernel development to improve explanations, review designs, or report a reproducible build problem. Please keep contributions small enough to review.
## Reporting security concerns

Durg has a pre-alpha release and makes no production-security claims. Report sensitive vulnerabilities privately using [GitHub private vulnerability reporting](https://github.com/VenkateshPanda0/Durg/security/advisories/new). For non-sensitive design questions, use a public issue without including private details. See [SECURITY.md](SECURITY.md).

## Design decisions

Significant decisions are written down as short **Architecture Decision Records** (ADRs) in `docs/adr/`, with context, options considered, the decision and its consequences. If you disagree with a recorded decision, open an issue that engages with its stated reasoning. New evidence is the best argument.

## Claims must be honest

Every security claim in the code or docs must be backed by a test or by a written argument in the threat model. Please do not add wording like "secure", "safe" or "verified" without that backing. We would rather under-promise.

## Code conventions

These conventions apply to the existing Rust workspace.

- **Language:** Rust. Guest crates are `#![no_std]`.
- **`unsafe`:** confined to designated low-level modules. Every `unsafe` block needs a `// SAFETY:` comment stating the invariant it relies on.
- **Style:** `cargo fmt` and clippy clean. Prefer clear code over clever code, because the codebase doubles as teaching material. Comments explain *why*, not what.
- **Dependencies:** the guest kernel and services follow a strict originality policy. Adding a third-party crate to guest code requires an ADR. Host-side tooling is not subject to this.
- **Tests:** each milestone ships with automated tests that run headless in QEMU. Pure-logic code should also be unit-tested on the host.
- **Commits:** one logical change per commit with a meaningful message, for example `kernel/mm: add bitmap physical frame allocator`.

### Sign-off (DCO)

Contributions use the [Developer Certificate of Origin](https://developercertificate.org/) rather than a CLA. Sign each commit with `git commit -s`, which adds a `Signed-off-by:` line using your own name and email.

## Questions about tooling or process

Open an issue and tag it `question`.

## AI-assisted contributions and how this project is built

This project is built **substantially by an AI coding assistant working under delegated authority** from the repository owner (see [ADR-0010](docs/adr/0010-delegated-project-authority.md)). It is not independently reviewed by a human maintainer, and nothing here should be described as audited or verified; the evidence for each claim is in `docs/THREAT_MODEL.md` and the test suite.

For contributors ([ADR-0004](docs/adr/0004-ai-assisted-contributions.md)): AI tools are allowed. Say in the pull request that you used one. You remain fully responsible for correctness and licensing, you must be able to explain every line in review, every `unsafe` block needs a `// SAFETY:` argument, and the change needs tests that would fail if it were wrong. Changes the author cannot explain may be declined.

## License

By contributing, you agree that your contributions are licensed under the [Apache License 2.0](LICENSE).
