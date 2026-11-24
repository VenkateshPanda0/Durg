# Reproducible builds

**Goal:** two clean builds of the same commit give a byte-identical boot image, so
anyone can rebuild a release and compare its SHA-256 with the published one.

## Status (read this before relying on it)

- **A first local test proved less than it seemed, and the first hosted run caught it.**
  On 2026-11-24 two release ISOs built from two directories had identical hashes, but
  the two builds ran at the same moment, so a wall-clock timestamp in the image could
  not show. The first run of the `reproducible` CI job (builds one after the other)
  failed: the two kernels were byte-identical (`27a9e668…a262a`, the same hash as the
  local build, so the kernel ELF also matched across two different machines) but the
  two ISOs differed. A sequential local test reproduced it: 162 bytes differed, the
  seconds field of directory timestamps.
- **Cause:** `xorriso` records each file's modification time, and the build copies
  files into a staging directory, which gives them the current time.
  `SOURCE_DATE_EPOCH` alone only fixed the volume dates.
- **Fix:** with `SOURCE_DATE_EPOCH` set, `cargo xtask` sets the modification time of
  every staged file and directory to it before building the image (`pin::set_tree_mtime`,
  covered by a unit test).
- **Observed locally after the fix, 2026-11-24:** three sequential release builds, in two
  different directories, each from a fresh `target/`, 5 to 10 seconds apart, all gave ISO
  `0bec3b9a4b9340da19c7a1cf2e917fe8bcc06a83ce36b3611abce86de6e0750e` and kernel
  `27a9e66858bf7a6a9511d3223fd9fa18b531627609ea20653413579329d7262a`. Same toolchain
  (Rust 1.97.0) and `xorriso 1.5.6`.
- **In CI:** the `reproducible` job in `.github/workflows/ci.yml` repeats the sequential
  two-build comparison on a hosted runner. **This page does not claim the build is
  reproducible in CI until that job's log shows two equal hashes**; the result is
  recorded under "CI result" below once it has been read.

## How it is done

- The toolchain is pinned (`rust-toolchain.toml`, 1.97.0) and `Cargo.lock` is
  committed.
- Limine is pinned by commit and its files by SHA-256 ([ADR-0014](adr/0014-pin-the-bootloader-by-commit-and-digest.md)).
- Source paths: cargo passes workspace crates to `rustc` by relative path, so the
  kernel and user ELFs do not contain the checkout directory (the two builds above
  were in different directories). No `--remap-path-prefix` was needed.
- `SOURCE_DATE_EPOCH` is set to the commit time (`git log -1 --format=%ct`). `xorriso`
  uses it for the volume dates; `xtask` sets every staged file's time to it (see above).
- No build-time timestamps, hostnames or random values are embedded by `xtask`.

## What is not covered

- **Other `xorriso` versions** or other hosts: the check uses one version at a time.
  A different `xorriso` may lay out the image differently.
- **Other toolchain versions.** The pin makes this a non-issue until it is bumped.
- **The Limine binaries are not built from source** here: they are fetched at the
  pinned commit and checked by digest. Rebuilding Limine reproducibly is future work.
- The `selftest` and other variant images are built the same way but are only
  compared in the release workflow's log, not by a dedicated comparison job.

## Checking a release yourself

```bash
git clone https://github.com/VenkateshPanda0/Durg && cd Durg
git checkout vX.Y.Z-pre
SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)" cargo xtask iso --release
sha256sum target/durg-default.iso      # compare with SHA256SUMS in the release
```

## CI result

*Not yet recorded for the fixed build.* The first run, before the fix, failed as described
under Status. (To be filled in from the first `reproducible` job log after the fix.)
