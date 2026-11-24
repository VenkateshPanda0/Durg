# Versioning

Durg uses [Semantic Versioning](https://semver.org/) with pre-release identifiers,
and **one version for the whole workspace**.

## The rule

- The version lives in one place: `[workspace.package] version` in the root
  `Cargo.toml`. Every crate inherits it (`version.workspace = true`).
- A release tag is `v` plus that version, for example `v0.1.0-alpha.2`. The release
  workflow refuses a tag that does not equal the workspace version.
- Releases are **pre-releases** (`0.y.z-alpha.N`, later `-beta.N`) until the owner
  decides otherwise. GitHub marks them as pre-releases. Nothing is a production
  security boundary ([SECURITY.md](../SECURITY.md)).
- The crates are `publish = false`; nothing is published to crates.io.

## What counts as breaking

Durg has no stable interfaces yet, and while the major version is `0` any release may
break any of them. A change to one of the following is still **called out under a
"Breaking" heading in the CHANGELOG**, because it invalidates user programs, boot
images or recorded evidence:

1. **The system-call ABI**: call numbers, register conventions, `invoke` labels and
   their argument layout, error codes and exit-status encoding (`crates/syscall`,
   [ARCHITECTURE.md §7](ARCHITECTURE.md)).
2. **Capability semantics**: the rights bits, what derive, move, delete, revoke and
   retype may do, which kinds exist, what a task is given at start
   ([design/m10-capability-core.md](design/m10-capability-core.md)).
3. **The boot protocol and image layout**: the Limine requests the kernel makes, the
   boot-module order and names (`boot/limine.conf`, `durg_syscall::module`), the
   memory map the kernel assumes.

Increment the pre-release number (`alpha.2` to `alpha.3`) for any release; move to
`0.2.0-alpha.1` when one of the three areas above changes incompatibly in a way
worth a new line of development.

## Choosing the next version

| Change since the last tag | Next version |
|---|---|
| Fixes, docs, CI, tests, internal refactors | next pre-release number |
| Compatible additions (a new `invoke` label, a new kind) | next pre-release number |
| A breaking change in 1 to 3 above | next pre-release number, **and** a "Breaking" entry |
| A deliberate new line of development | bump the minor, restart at `-alpha.1` |

The version the owner has approved for the next release is recorded in
[RELEASING.md](RELEASING.md); until one is approved the workspace stays at `0.0.0`
and no tag is created.
