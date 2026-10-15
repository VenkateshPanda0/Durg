# ADR-0001: Locked foundational decisions

- **Status:** Accepted
- **Date:** 2026-10-15
- **Deciders:** project owner

## Context

Before any code is written, the project's foundations need to be written down so contributors can see what is settled and what is open. These decisions came out of the initial design and research phase, and they are implemented as written. Reopening one requires a new ADR that supersedes this one.

## Decisions

| Area | Decision | Rationale |
|---|---|---|
| Language | **Rust** for the kernel and all native user space. Minimal assembly for entry, context switch and trap stubs. | A security thesis written in C undermines itself. Rust confines memory-unsafety risk to a small, auditable `unsafe` core. |
| Kernel style | **Capability microkernel modeled on seL4's object model**: untyped memory retyping, CNodes/CSpaces, endpoints, notifications, threads, address spaces, frames, IRQ handlers. Not formally verified. | Smallest trusted base, and capabilities are the thesis. |
| `unsafe` policy | `unsafe` is confined to a small set of designated, audited modules. Every `unsafe` block has a `// SAFETY:` comment. The unsafe line count is tracked as a metric. | Keeps the trusted surface reviewable. |
| Drivers | **User-space virtio drivers only**: console, block and vsock first, rng and net later. | Removes the real-hardware problem. A driver crash stays isolated. |
| Architecture | **x86-64** first, under QEMU (KVM when available). | Matches cloud VMs and Firecracker-class VMMs, with the best tooling. |
| Boot | **Limine boot protocol** at first, with a direct-boot path for VMMs later. | Lets work start at the kernel, not the boot sector. |
| Networking | **Phase 1: no in-guest TCP/IP.** Egress only via vsock to a host-side broker. | Defers the largest subsystem. |
| Storage | Own **copy-on-write object store** over virtio-blk with snapshots. Objects are addressed by capability handles, not paths. | Required for undo and provenance. |
| Labels | Enforced by the store and loader services, **not the kernel**. | Keeps the kernel minimal. |
| App ABI | Phase 1: native Rust programs on our own capability API. Phase 2: a WASI-component runtime (port or write is still open). Never POSIX-first. | WASI already has no ambient authority. |
| Licensing | **Apache License 2.0** for the whole repository. | See "License" below. |
| Contributions | **Developer Certificate of Origin** sign-off (`Signed-off-by:`), no CLA. | Contributor-friendly. |
| Governance | Owner-led with written ADRs. Move to a maintainer group at three or more regular contributors. | Simple while the project is small. |
| Originality | The core (kernel, drivers, store, journal, services) is written in-house. Third-party crates in guest code need an ADR each. Host-side tooling is exempt. | The point of the project is to build it. |

### License

The initial design proposed MPL-2.0 for the kernel and services, with MIT OR Apache-2.0 for SDK crates and docs. The owner chose **Apache-2.0 for the whole repository** on 2026-10-15. This supersedes the earlier split.

Consequences of that choice:

- Apache-2.0 has no file-level copyleft. Anyone may fork and extend the kernel without publishing their changes.
- Apache-2.0 includes an explicit patent grant.
- One `LICENSE` file covers everything, which is simpler for contributors than a split.

## Consequences

- Contributors can treat the items above as settled and focus on open questions.
- Open decisions are tracked as separate ADRs with status `Proposed`. They are never decided silently.
- Changing any row above needs a new ADR that supersedes this one.
