# ADR-0008: A nightly toolchain, for host-side fuzzing only

- **Status:** Accepted (2026-10-19). **Decided by the assistant under the owner's delegation** ("Do whatever you think is best", in reply to a message that listed this question). The owner may reverse it; see "How to undo".
- **Needed by:** M8 (the acceptance criteria ask for a first fuzz target for the syscall entry)

## Context

CLAUDE.md §9.5 asks for fuzzing of the syscall interface from M8. The standard Rust tool, `cargo-fuzz` (libFuzzer), instruments code with sanitizer flags that only nightly `rustc` accepts. ADR-0002 keeps the **kernel** on stable and says each nightly use is recorded. Until now the syscall entry was covered by host property tests of the pure validator and by 22,000 randomised calls into the real handler inside a QEMU self-test. Neither is coverage-guided.

## Options

1. **Nightly for the fuzz crate only.** `fuzz/` is its own Cargo workspace, built with `cargo +nightly fuzz`. The kernel, CI, `rust-toolchain.toml` and every other crate stay on stable.
   - Pro: the standard, well-supported tool; fast (about 500,000 executions per second here); finds bugs in seconds (see below).
   - Con: a second toolchain on the developer's machine; the fuzzer is not in CI.
2. **A stable-compatible fuzzer** (`cargo-afl`, `honggfuzz`).
   - Pro: no nightly.
   - Con: needs AFL++ or honggfuzz built from source with extra system dependencies; slower to set up and to run; less commonly used by the Rust community.
3. **No coverage-guided fuzzing.** Keep property tests and the in-kernel randomised test.
   - Pro: nothing to install.
   - Con: does not meet the M8 acceptance criterion; property tests do not use coverage feedback.

## Decision

**Option 1.** The nightly toolchain is installed with `rustup toolchain install nightly --profile minimal` and used only as `cargo +nightly fuzz ...` inside `fuzz/`. It is free, installed in the user's home directory, and not pinned (the fuzzer needs no specific nightly). The repository's pinned stable toolchain is unchanged.

## Consequences

- `fuzz/` holds two targets over the pure `durg-syscall` crate. `syscall_validate` checks decoding, the error codec, splitting, and compares validation against an **independent oracle** (128-bit arithmetic, the boundary as a literal), on both random and boundary-biased inputs. `split_pages` fuzzes `split_by_page` on ranges that were never validated.
- **The targets were tested the way tests are:** by breaking the validator on purpose. All 7 mutations tried were found, each within about a second. The first version of the target missed two (it compared against the constant under test, and checked only soundness, not completeness); it was strengthened and re-run. That is recorded in the M8 chapter.
- **Scope:** the fuzzer covers the pure decoding, validation and splitting logic. It does **not** cover the kernel's page-table walk, the copy through the direct map, the entry stub, or anything that needs the kernel running. Those are covered by the QEMU self-tests only.
- **Not in CI**: it needs nightly and a time budget. Run it by hand (`fuzz/README.md`). Putting a short run in CI is a possible later step, and would make nightly part of CI, which is a separate decision.
- ADR-0002 is unchanged in substance: the kernel stays on stable. Its rule that nightly uses are recorded is met by this ADR.

## How to undo

`rustup toolchain uninstall nightly`, delete `fuzz/`, and remove the `exclude = ["fuzz"]` line from the root `Cargo.toml`. Nothing else depends on it.
