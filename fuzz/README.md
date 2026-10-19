# Fuzzing the syscall door

Coverage-guided fuzz targets (libFuzzer, through `cargo-fuzz`) for the pure part of the syscall entry: `durg-syscall`. Host-only. This crate is its own workspace and needs a **nightly** toolchain; the kernel stays on stable ([ADR-0008](../docs/adr/0008-nightly-for-host-side-fuzzing.md)).

## One-time setup

```bash
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
```

## Run

```bash
cd fuzz
cargo +nightly fuzz run syscall_validate -- -max_total_time=60
cargo +nightly fuzz run split_pages      -- -max_total_time=60
```

A finding is written to `fuzz/artifacts/<target>/crash-...`; replay it with `cargo +nightly fuzz run <target> <that file>`. `corpus/` and `artifacts/` are not committed.

## What each target checks

- **`syscall_validate`:** decoding is total and exact; validation agrees with an independent oracle (128-bit arithmetic, the boundary as a literal number) on random inputs and on inputs biased to the edge of user space; refusals carry the right error; a validated range splits into contiguous single-page pieces that add up to its length; the error return values round-trip.
- **`split_pages`:** `split_by_page` on ranges that were **never validated** neither panics nor loops, and still yields sane pieces.

## What it does not cover

The kernel's page-table walk, the copy through the direct map, and the entry stub. Those need the kernel running and are covered by the QEMU self-tests, not by this fuzzer.

## Is the fuzzer any good?

It was tested by breaking the validator on purpose (wraparound unchecked, boundary moved, off-by-one on the boundary and on the limit, an unknown call accepted, error values not negated, a chunk allowed to cross a page). All were found within about a second. Keep doing that when the targets change: a target that cannot find a known bug proves nothing.
