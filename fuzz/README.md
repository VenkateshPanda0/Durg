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
mkdir -p corpus/elf_parse
cargo +nightly fuzz run elf_parse corpus/elf_parse seeds/elf_parse -- -max_total_time=60
```

The ELF target starts from the valid files in `seeds/elf_parse/` (random bytes almost never form a valid ELF header) and mutates them. The seeds are committed; `corpus/` is not.

A finding is written to `fuzz/artifacts/<target>/crash-...`; replay it with `cargo +nightly fuzz run <target> <that file>`. `corpus/` and `artifacts/` are not committed.

## What each target checks

- **`syscall_validate`:** decoding is total and exact; validation agrees with an independent oracle (128-bit arithmetic, the boundary as a literal number) on random inputs and on inputs biased to the edge of user space; refusals carry the right error; a validated range splits into contiguous single-page pieces that add up to its length; the error return values round-trip.
- **`elf_parse`:** hostile ELF files. `durg_elf::parse` must not panic or loop, and any image it accepts must satisfy every property the loader relies on: page-aligned segments, no segment both writable and executable, every segment inside the allowed address range, no two sharing a page, every file range inside the file, the entry point inside executable file-backed bytes, and a bounded memory total.
- **`split_pages`:** `split_by_page` on ranges that were **never validated** neither panics nor loops, and still yields sane pieces.

## What it does not cover

The kernel's page-table walk, the copy through the direct map, the entry stub, and the part of the loader that maps pages (only the parsing is pure). Those need the kernel running and are covered by the QEMU self-tests, not by this fuzzer.

## Is the fuzzer any good?

It was tested by breaking the code under test on purpose. For the validator: wraparound unchecked, boundary moved, off-by-one on the boundary and on the limit, an unknown call accepted, error values not negated, a chunk allowed to cross a page. For the ELF parser: W^X not enforced, overlap unchecked, a file range one byte too generous, an entry point one byte past the code, the address limit unchecked, alignment unchecked, a wrapping segment end, the memory budget unchecked, a segment with no permissions. All 16 were found within about a second or two. Keep doing that when the targets change: a target that cannot find a known bug proves nothing.
