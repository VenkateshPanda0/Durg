# Roadmap

Work proceeds **one milestone at a time**. A milestone is done when its acceptance criteria pass with real, observed output, a design-log chapter is written, and the owner signs off. Dates are intentionally absent. The project is part-time, and slipping is reported honestly rather than hidden.

## Milestones

| # | Milestone | Acceptance (summary) | Status |
|---|---|---|---|
| **M0** | Thesis, threat model, decision records | Owner approves the threat model. No kernel code before that. | threat model approved, ADRs 0004-0005 pending |
| **M1** | Toolchain, build, CI | A fresh clone plus one documented command boots under QEMU and prints a line on serial. CI shows the same. | implemented, CI green, awaiting owner sign-off |
| **M2** | Kernel entry and serial output | Kernel prints the memory map. A deliberate test failure exits QEMU with a failure code. | planned |
| **M3** | CPU structures, exceptions, interrupts, timer | Breakpoint, page fault and double fault are handled in tests. Timer ticks are counted. | planned |
| **M4** | Physical memory manager | Allocate/free stress test in QEMU. Host property tests show no double allocation. | planned |
| **M5** | Virtual memory and paging | Map, unmap, translate, page-fault and guard-page tests. | planned |
| **M6** | Kernel heap | Heap stress tests. An out-of-memory path is handled. | planned |
| **M7** | Threads and scheduling | Multiple threads interleave under preemption, shown by a test. | planned |
| **M8** | User mode and syscalls | A user task makes a syscall and returns. A task touching kernel memory is killed, not the kernel. | planned |
| **M9** | ELF loader and init task | A CI test proves task A cannot read task B's memory. **Decision gate:** review progress before continuing to the capability work. | planned |

## After M9 (outline only, no fresh plan yet)

| # | Milestone |
|---|---|
| M10 | Capability core: CSpace/CNodes, untyped retype, derive, revoke, endpoints, notifications. Property tests for revocation. |
| M11 | User-space virtio: console, block, vsock. Restartable drivers. |
| M12 | Copy-on-write object store with snapshots. Crash-consistency tests. |
| M13 | Effect journal (hash-chained), provenance labels, approval gate, host egress broker. |
| M14 | Shell, native tools, host controller. The flagship demo from the README. |
| M15 | Firecracker-class VMM boot path and boot-time benchmark. |
| M16 | WASI-component runtime. |
| M17 | ARM64 port. |

## Open decisions

Tracked as `Proposed` records in [docs/adr](adr/README.md) where options are written up. Others will be raised when they become relevant:

- Capability object list and syscall ABI shape (before M10)
- Label semantics (before M13)
- Journal tamper-evidence scheme (before M13)
- WASI runtime: port vs write (before M16)
- In-guest TCP/IP vs broker-only (after M14)

## Later

Ideas noticed along the way that do not belong to the current milestone are added here instead of being built early.

- (empty)

## Fallback

If progress stalls at the M9 decision gate, the project narrows into a **teaching capability OS** with a public book-style design log. Code and docs are written so that path stays viable: clear, explained and incremental.
