# Security policy

Durg is pre-alpha research software and is not ready for protecting real
workloads. No release is supported as a production security boundary.

## Supported versions

Only the latest pre-release and the current `main` are looked at, on a best-effort
basis. Older pre-releases get no fixes.

| Version | Looked at |
|---|---|
| `main` | yes, best effort |
| latest pre-release (see [Releases](https://github.com/VenkateshPanda0/Durg/releases)) | yes, best effort |
| anything older | no |

## Reporting a vulnerability

For a suspected vulnerability that should not be public, use
[private vulnerability reporting](https://github.com/VenkateshPanda0/Durg/security/advisories/new).
Include the affected commit, reproduction steps, expected and actual behavior,
and any relevant impact. Please avoid including real secrets in a report.

## What to expect

- **Acknowledgement within 7 days.** A person reads every report.
- **No fix deadline while Durg is pre-alpha.** There is one maintainer, with an AI
  assistant, and no service-level promise. You will be told what is decided.
- **Coordinated disclosure, 90 days.** Please give 90 days from the report before
  publishing details, or until a fix ships, whichever is first. The maintainer may ask
  for more time for a serious issue and will say why; a shorter time is fine if the
  issue is already public or a fix is released sooner.
- Reporters are credited in the advisory unless they ask not to be.

## What is in scope

Reports about the code in this repository that affect the properties the
[threat model](docs/THREAT_MODEL.md) says Durg tries to provide: a task reaching memory
or authority it was not given, a capability that outlives its revocation, a way for
user-mode code to crash or hang the kernel, an error in the loader, the system-call
validation or the capability rules, and weaknesses in the build and release chain
(unpinned or unverified inputs, workflow permissions).

## What is out of scope

- Anything the threat model or [UNSUPPORTED.md](docs/UNSUPPORTED.md) says Durg does
  not claim, including side channels, a malicious host or hypervisor, physical access,
  and features that do not exist yet (drivers, networking, the desktop).
- Findings that need a modified kernel, a modified bootloader or root on the host.
- Denial of service by exhausting the host's own resources.
- Reports about third-party services (GitHub, QEMU, Limine) that are not specific to how
  Durg uses them.

For non-sensitive design questions and public discussion, open a regular issue. The
[threat model](docs/THREAT_MODEL.md) records the project's assumptions and the limits of
current evidence.
