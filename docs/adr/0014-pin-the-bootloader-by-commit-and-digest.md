# ADR-0014: Pin the bootloader by commit and file digest

- **Status:** Accepted (2026-11-25), on the owner's instruction "You have all my permission" (2026-11-25), given in reply to the list of open decisions; the assistant applied its recommendation. The owner has not reviewed the text line by line, so this is delegated, not independently reviewed. Implemented since 2026-11-24.
- **Needed by:** any release; the bootloader is part of the trusted computing base.

## Context

Limine runs before the kernel and hands it the memory map, the direct-map offset and the boot modules. The build fetched it with `git clone --depth 1 --branch v10.8.5-binary`. A git tag can be deleted and recreated, so the tag does not say which bytes were booted, and nothing checked the files that went into the ISO. [ADR-0003](0003-limine-bindings.md) is about the kernel's own bindings to the Limine protocol, not about where the binaries come from.

## Options

1. **Keep the tag.** Simple; the bootloader is whatever the tag points at on the day.
2. **Pin the commit.** Fetch the commit itself, check `HEAD`, and treat the tag as a label only.
3. **Pin the commit and the SHA-256 of every file copied into the image** (`BOOTX64.EFI`, `limine-uefi-cd.bin`). Check on every build, including from the cache. Fail closed.
4. **Vendor the binaries in the repository.** Strongest, but adds binary blobs to history and a licence notice; a reproducible build from source would be better still and is a later step.

## Decision

**Option 3.** `xtask` fetches commit `7a9013f305de0bea1f6310e76e7baba30499fef0` (the commit `v10.8.5-binary` pointed at on 2026-11-24, confirmed with `git ls-remote` against the peeled tag), checks that `HEAD` is that commit, and compares the SHA-256 of each file it copies into the image with constants in `xtask/src/main.rs`. A missing or different file stops the build with a message naming the file, the expected digest and the found digest; it never falls back to fetching something else. The cache stamp records the commit, so changing the pin refetches.

### Bumping Limine

1. Choose the new release tag and read its release notes and diff for protocol changes the kernel's bindings depend on.
2. Resolve the commit: `git ls-remote https://github.com/limine-bootloader/limine.git 'refs/tags/<tag>' 'refs/tags/<tag>^{}'`; the `^{}` line, if present, is the commit.
3. Change `LIMINE_TAG` and `LIMINE_COMMIT` in `xtask/src/main.rs`.
4. Delete `target/limine`, run `cargo xtask iso`, which fails with the digests it found. Check them independently (for example `sha256sum` on a second machine) before copying them into `LIMINE_PINS`.
5. Run the full gate, including `cargo xtask test` in both profiles, and record the bump in the devlog and CHANGELOG.

## Consequences

- A moved tag, a corrupted download or a modified cache cannot reach the ISO unnoticed.
- The pin is only as trustworthy as the first fetch: the digests were taken from a clone whose commit matches the remote's tag, not from a signed release. Limine does not publish signatures we check. Building Limine from source reproducibly is future work.
- Bumping needs five deliberate steps instead of one constant.
