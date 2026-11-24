# Reproducible builds

**Goal:** two clean builds of the same commit give a byte-identical boot image, so
anyone can rebuild a release and compare its SHA-256 with the published one.

## Status (read this before relying on it)

- **Observed locally, 2026-11-24:** two release ISOs (`cargo xtask iso --release`)
  built from two copies of commit `b7d734e`'s tree, in two different directories,
  each with a fresh `target/` and `SOURCE_DATE_EPOCH=1700000000`, were
  **byte-identical**: ISO `9d500e8c78ed97845f20966cc6d0b7f27b45d41f6d41a24c6d9fce296021970f`,
  kernel `f73f86dbaa42afda96d161a748b7187a4f8c332d60dd367fe55ab6ff5df7fad1`, and the user programs `init` and `capdemo` had equal hashes too. Same machine, same
  toolchain (Rust 1.97.0), `xorriso 1.5.6`.
- **Without `SOURCE_DATE_EPOCH`, the ISO differs on every build** (three builds gave
  three different hashes): xorriso stamps the current time into the image.
- **In CI:** the `reproducible` job in `.github/workflows/ci.yml` repeats the
  two-build comparison on a hosted runner. **This page does not claim the build is
  reproducible in CI until that job's log shows two equal hashes**; the result is
  recorded in the section "CI result" below once it has been read.

## How it is done

- The toolchain is pinned (`rust-toolchain.toml`, 1.97.0) and `Cargo.lock` is
  committed.
- Limine is pinned by commit and its files by SHA-256 ([ADR-0014](adr/0014-pin-the-bootloader-by-commit-and-digest.md)).
- Source paths: cargo passes workspace crates to `rustc` by relative path, so the
  kernel and user ELFs do not contain the checkout directory (the two builds above
  were in different directories). No `--remap-path-prefix` was needed.
- `SOURCE_DATE_EPOCH` is set to the commit time (`git log -1 --format=%ct`). `xorriso`
  honours it for every timestamp in the image.
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

*Not yet recorded.* (To be filled in from the first `reproducible` job log.)
