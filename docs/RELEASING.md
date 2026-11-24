# Releasing

A release is a **pre-release** of an unreviewed research kernel. Follow every step;
do not skip one because a check "should" pass. The owner decides when to release
(the maintainer of record).

## Before tagging

1. **Choose the version** with [VERSIONING.md](VERSIONING.md). The owner approves it.
   Set `[workspace.package] version` in the root `Cargo.toml`; run `cargo metadata`
   once so `Cargo.lock` updates.
2. **Read the CI log, do not just see a green tick.** On the commit to be released,
   open the `ci-ok` run and read the jobs: the boot tests in both profiles (note
   whether they ran under KVM or software emulation, the log says), the fuzz smoke
   runs, Miri, coverage, `deny`, and the `reproducible` job. A skipped or cancelled
   job is not a pass.
3. **Update the evidence.** Record in [THREAT_MODEL.md](THREAT_MODEL.md) any property
   that gained a passing test, with a "what this does not show" note; extend the
   relevant [book](book/) chapter; add a [devlog](devlog/) entry with the observed
   results (quote the lines, for example `selftest: all N tests passed`).
4. **Check the wording.** The README, SECURITY.md and the release notes must still
   say that Durg is pre-alpha research software and is not for production use or as
   a security boundary. Every number in the README is current or dated.
5. **Finish the CHANGELOG.** Rename `## Unreleased` to `## vX.Y.Z-pre — YYYY-MM-DD`,
   add a fresh empty `## Unreleased`, and list breaking changes under "Breaking".
   The release notes are cut from this section; the workflow fails if it is missing.
6. **Update `docs/TODO.md`** if the release finishes or adds tasks.
7. **Check the reproducibility record** ([REPRODUCIBLE_BUILDS.md](REPRODUCIBLE_BUILDS.md))
   still describes what the `reproducible` job shows.

## Tagging

8. Commit the version and CHANGELOG change to `main` (through the usual checks).
9. Show the draft release (title, notes, file list below) to the owner and wait for a
   yes. A tag is public and should not be moved.
10. Create and push the tag: `git tag -a vX.Y.Z-pre -m "vX.Y.Z-pre"` then
    `git push origin vX.Y.Z-pre`.

## What the release workflow does

`.github/workflows/release.yml` runs the whole CI (including the reproducibility
check), then in release mode builds and boots every variant, and attaches:

- `durg-<tag>-plain.iso` and `durg-<tag>-selftest.iso`: the bootable images
- `durg-kernel-<tag>.elf`: the kernel
- `durg-sbom-<tag>.tar.gz`: a CycloneDX SBOM per workspace crate
- `SHA256SUMS`

and a build-provenance attestation for all of them. It creates a GitHub Release
marked **pre-release** with notes from the CHANGELOG section.

## After the workflow

11. Read the workflow log. Check the files: `sha256sum -c SHA256SUMS`, and
    `gh attestation verify <file> --repo VenkateshPanda0/Durg`.
12. If the run failed after the tag exists, **do not move the tag**. Fix forward with
    the next pre-release number, and say so in the devlog.
13. Record the release in the devlog.

## Version approved for the next release

None yet. The owner has not approved a version; the suggestion is `0.1.0-alpha.2`.
