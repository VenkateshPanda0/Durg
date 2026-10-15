<div align="center">

# Durg (दुर्ग)

### A fortress for untrusted agents.

A from-scratch, capability-based operating system in Rust that runs AI agents inside a VM, where they can only touch what they're handed, and everything they do can be recorded and undone.

![Status](https://img.shields.io/badge/status-pre--alpha-orange)
![Language](https://img.shields.io/badge/language-Rust-b7410e)
![Target](https://img.shields.io/badge/target-x86--64%20%7C%20QEMU%2FKVM-blue)
[![License](https://img.shields.io/badge/license-Apache--2.0-green)](LICENSE)
[![Issues](https://img.shields.io/github/issues/VenkateshPanda0/Durg)](https://github.com/VenkateshPanda0/Durg/issues)
![PRs welcome](https://img.shields.io/badge/PRs-welcome-brightgreen)

[Why](#the-problem) · [The idea](#the-idea-five-promises) · [Demo goal](#the-demo-we-are-building-towards) · [Design](#how-it-works) · [Roadmap](#roadmap) · [**Contribute**](#-contribute) · [FAQ](#faq)

</div>

> **Honest status:** Durg is in the **design phase**. The kernel boots under QEMU, prints the memory map and a few boot facts over serial, runs a handful of self-tests, and exits. That is all it does. No security claim here is verified. Each promise below becomes a claim only once a test or a written argument in the threat model backs it. You are early, which means your feedback can still shape the foundations.

---

## The problem

AI agents now run commands on real machines, and the failures are public: deleted home directories, leaked tokens, prompt injection in the wild.

A recurring pattern is the **trust handoff**. The agent stays inside its sandbox, but writes a file (a git hook, an editor task, an interpreter config) that a more trusted, unsandboxed tool later executes. The sandbox held, and the system was still compromised.

That is an **ambient authority + missing provenance** problem. Typical sandboxes run a full Linux guest, so the agent still sits in a large environment where it can name almost everything, and nothing tracks where a file came from or lets you undo a session. Capability systems were built for this, and Durg applies that model to the agent use case.

## The idea: five promises

Each one is designed to become a **testable property**, not a slogan.

| # | Promise | In plain words |
|---|---|---|
| 1 | **No ambient authority** | A process can only act on objects it holds a capability for. No global paths, no root user, no inherited permissions. |
| 2 | **Provenance labels** | Everything an agent session creates is tagged with that session. Trusted code can't consume it without an explicit, journaled acceptance. |
| 3 | **Effect journal + session undo** | Side effects go into a hash-chained log on copy-on-write storage, so a whole session can be rolled back. |
| 4 | **Approval as a capability** | Destructive or external actions need a one-time capability that only the human console can mint. No prompt or agent can create one. |
| 5 | **Brokered egress and secrets** | No raw network. Traffic goes through a host-side broker over vsock that applies policy and injects credentials, so the agent never holds a secret. |

## The demo we are building towards

A host-side agent is told to *"clean up the project."*

1. It tries the equivalent of `rm -rf ~/` and **fails**, because nothing nameable exists to delete.
2. It writes a git hook. The hook is **labeled**, and a trusted tool **refuses to run it** until the human accepts.
3. It tries to read a secret and only sees a **broker handle**.
4. The human runs **undo session**, and everything reverts.
5. The **journal** shows every attempt.

## How it works

```
Host (Linux + QEMU/KVM)
 ├─ Agent loop + LLM                  not part of the OS
 ├─ Controller · egress broker · human console ──── vsock ────┐
                                                              │
Guest OS ─────────────────────────────────────────────────────┘
 ├─ Kernel (Rust, ring 0): capabilities, IPC, memory, threads, traps
 │    no drivers, no filesystem, no network in the kernel
 └─ User space, each service in its own address space:
      virtio drivers · copy-on-write object store · effect journal
      label service · approval gate · session manager · tools
```

- **Capability microkernel** modeled on seL4's object model (untyped memory retyping, CSpaces, endpoints, notifications). It is **not formally verified**, and we won't imply it is.
- **Rust, with `unsafe` confined** to a small audited core. Every `unsafe` block carries a `// SAFETY:` comment, and we track the unsafe line count as a metric.
- **Virtual hardware only.** x86-64 under QEMU/KVM first, with user-space virtio drivers. That removes the real-hardware driver problem that stalls most hobby OSes.
- **Small syscall surface** (about 10–15 calls) so it can be fuzzed.
- **The model stays outside.** The LLM runs on the host or behind an API, and the OS only executes tool calls under capabilities.

### How Durg differs from what exists

| Approach | Typical shape | Where Durg is different |
|---|---|---|
| Containers / gVisor | Shared or emulated Linux kernel | We don't run Linux at all |
| Firecracker / microVM sandboxes | Full Linux guest in a microVM | Capabilities inside the guest, plus provenance and undo built in |
| Policy proxy + snapshots on Linux | Bolt-on controls around a general OS | Authority is absent by construction, not filtered afterwards |
| seL4 | Capability kernel, no drivers or FS | Same object model, but with the services an agent needs |

We take the strongest counter-argument seriously: a Linux microVM with a policy proxy and snapshots may deliver much of the value sooner. Durg is a bet that removing ambient authority at the OS level gives guarantees that bolt-on layers can't. Whether that bet pays off is exactly the question this project exists to answer.

## What Durg is not

Stated up front so nobody is surprised:

- Not a desktop OS: no GUI, window system or browser.
- No real-hardware drivers (virtio only).
- No Linux binary or ABI compatibility.
- No formal verification claims.
- No AI model inside the kernel.
- No claim of being "more secure than Linux." Claims must map to the threat model.

## Try the boot test

Needs Rust (the pinned toolchain installs itself), QEMU, xorriso and OVMF.

```bash
sudo apt install qemu-system-x86 xorriso ovmf   # Debian/Ubuntu
cargo xtask test                                 # build, boot headless, check serial output + exit code
cargo xtask run                                  # boot and watch the serial output
```

Expected: the serial output shows `Durg kernel: boot ok`, the memory map, and `selftest: all 7 tests passed`, and the run ends with `boot test: PASS (3 variants)`. One of the three variants fails on purpose, to prove the harness can report failure. If it does not work on your machine, please open an issue with your OS and the output. That report is the most useful early contribution.

## Roadmap

Work goes one milestone at a time, with headless QEMU tests for each.

| Stage | Goal | Status |
|---|---|---|
| **M0** | Threat model, ADRs, docs | done (two ADRs still open) |
| **M1** | Toolchain, build, CI, boot under QEMU | done |
| **M2** | Serial console, boot information, self-test harness | done |
| **M3** | GDT/IDT, exceptions, interrupts, timer | exceptions done, timer pending a decision |
| **M4–M6** | Physical memory, paging, kernel heap | planned |
| **M7–M9** | Threads, user mode and syscalls, ELF loader, isolated tasks | planned |
| **M10+** | Capability core, user-space virtio, object store, journal, approval gate, demo | planned |

---

## 🤝 Contribute

Durg is at the stage where **design review is the most valuable thing you can give**, and where the first code is about to be written. If you want to be part of an OS from its first commit, this is that window.

### Pick the lane that fits you

| If you know... | You could help with |
|---|---|
| **Security / threat modeling** | Attack the five promises. Find a trust-handoff path the design misses. Review the threat model as it lands. |
| **seL4 / capability systems** | Review the object model, revocation and delegation design before it's written. |
| **x86-64 / OS internals** | Help with GDT/IDT/TSS, APIC and paging as M3–M5 start. Sanity-check hardware assumptions against the Intel SDM. |
| **Rust (`no_std`, unsafe auditing)** | Shape the workspace, the lint policy and the `unsafe` audit tooling. |
| **Virtualization / virtio / VMMs** | Review the virtio driver plan and the host-side broker design. |
| **Docs / teaching** | Help turn each milestone into a readable design-log chapter. |
| **Testing / fuzzing** | Help design the QEMU test harness and the syscall fuzz targets. |

### Start in five minutes

1. **Star and watch** this repo to follow along.
2. **Open an issue** with a question, a critique, or prior art we should read. A sharp objection is a contribution.
3. **Try to break an idea.** Pick a promise and write down how you'd defeat it.
4. **Boot Durg from a clean clone** (see [Try the boot test](#try-the-boot-test)) and tell us what broke. That is the most useful early signal.

Please **open an issue before starting a large change**, and read [CONTRIBUTING.md](CONTRIBUTING.md). Look for issues labelled [`good first issue`](https://github.com/VenkateshPanda0/Durg/labels/good%20first%20issue) and [`help wanted`](https://github.com/VenkateshPanda0/Durg/labels/help%20wanted).

### What to expect from us

- Decisions are written down as short **Architecture Decision Records**, with context, options and consequences.
- The scope is deliberately small. We will say no to a GUI, hardware drivers and Linux compatibility, and explain why.
- Claims stay honest. We would rather under-promise than overclaim security.

## FAQ

**Why a new kernel instead of hardening Linux?**
Because ambient authority is built into the Linux model: global paths, users, inherited permissions. A capability OS removes it by construction. It's a trade-off, not a free win, and the roadmap is designed to test it.

**Is this formally verified like seL4?**
No. We borrow seL4's object model, not its proofs, and we won't claim verification.

**Why Rust?**
A security thesis written in C undermines itself. Rust confines memory-unsafety risk to a small, auditable `unsafe` core.

**Will it run my existing Linux tools?**
No. Phase 1 is native Rust programs on our own capability API. A WASI runtime is the planned path for portable tools, since WASI also has no ambient authority.

**Can I use it today?**
Not for anything useful. It boots in QEMU and prints boot information. See [Try the boot test](#try-the-boot-test), or watch the repo and help build it.

**Does an AI model run inside it?**
No. The agent and model run on the host or behind an API. Durg only executes the tool calls it allows.

## About the name

*Durg* (दुर्ग) means *fortress* in Sanskrit and Hindi: a small, walled, hardened kernel that an untrusted agent can work inside but cannot break out of. It is also the name of a city in Chhattisgarh, India.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
