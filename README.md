# Durg (दुर्ग)

**A fortress for untrusted agents.**

Durg is an open-source, capability-based operating system written in Rust. It is designed to run AI agents and other untrusted automation inside a virtual machine, where the agent can only touch what it has been explicitly handed, and everything it does can be recorded and undone.

> **Status: pre-alpha. There is no bootable kernel yet.** This repository currently holds the project's license and the start of its design. Nothing here is a security claim until it is backed by a test or a written argument in the threat model.

## Why

AI coding agents now run with real power over real machines, and the failures are public: deleted home directories, leaked tokens, prompt injection in the wild. A recurring pattern is the "trust handoff": the agent stays inside its sandbox but writes a file (a git hook, an editor task) that a more trusted tool later executes.

That is an **ambient authority + missing provenance** problem. Most sandboxes run a full Linux guest, so the agent still has a large, ambient-authority environment inside it, and there is no built-in provenance or undo. Capability systems were designed for exactly this, and Durg applies that model to the agent use case.

## The idea: five promises

Each of these is meant to become a testable property, not a slogan.

1. **No ambient authority.** A process can only name and act on objects it holds capabilities for. No global path namespace, no root user, no inherited permissions.
2. **Provenance labels.** Everything an agent session creates carries that session's label. A more trusted principal cannot consume a labeled object without an explicit, journaled acceptance.
3. **Effect journal and session undo.** Side effects are appended to a hash-chained log on copy-on-write storage, so a whole session can be rolled back.
4. **Approval as a capability.** Destructive or external actions need a one-time capability that only the human-controlled console can mint.
5. **Brokered egress and secrets.** No raw network access. Egress goes through a host-side broker over vsock, which applies policy and injects credentials, so the agent never holds secret values.

### Target demo

A host-side agent is told to "clean up the project". It tries the equivalent of `rm -rf ~/` and fails, because nothing nameable exists to delete. It writes a git hook, and the hook is labeled and refused until the human accepts it. It tries to read a secret and only sees a broker handle. The human runs *undo session* and everything reverts, and the journal shows every attempt.

## Design in brief

- **Language:** Rust, with minimal assembly for entry, context switch and traps. `unsafe` is confined to a small audited core, and every block carries a `// SAFETY:` comment.
- **Kernel:** a small capability microkernel modeled on seL4's object model (untyped memory retyping, CSpaces, endpoints, notifications). It is **not** formally verified.
- **Runs on virtual hardware only:** x86-64 under QEMU/KVM first. Drivers are user-space virtio (console, block, vsock), so there is no real-hardware driver problem.
- **Services in user space:** copy-on-write object store, effect journal, label service, approval gate, session manager.
- **Models stay outside:** the LLM runs on the host or behind an API. The OS only executes tool calls under capabilities.

```
Host (Linux + QEMU/KVM)
 ├─ Agent loop + LLM          (not part of the OS)
 ├─ Controller / egress broker / human console   ── vsock ──┐
Guest OS ───────────────────────────────────────────────────┘
 ├─ Kernel: capabilities, IPC, memory, threads, traps
 └─ User space: drivers, object store, journal, labels, approval gate, tools
```

## What Durg is not

Stated up front so nobody is surprised:

- Not a desktop OS: no GUI, window system or browser.
- No real hardware drivers (virtio only).
- No Linux binary or ABI compatibility.
- No formal verification claims.
- No AI model inside the kernel.
- No claim of being "more secure than Linux". Claims will map to the threat model.

## Roadmap

Work proceeds one milestone at a time, each with headless QEMU tests.

| Stage | Goal |
|---|---|
| M0 | Threat model, ADRs, documentation (in progress) |
| M1 | Toolchain, build, CI, boot under QEMU |
| M2–M3 | Kernel entry, serial output, CPU structures, interrupts, timer |
| M4–M6 | Physical memory, paging, kernel heap |
| M7–M9 | Threads, user mode and syscalls, ELF loader, isolated tasks |
| M10+ | Capability core, user-space virtio, object store, journal, approval gate, demo |

## Get involved

The project is at the stage where **design feedback is the most valuable contribution**:

- Poke holes in the five promises and the threat model once it lands.
- Open an issue with prior art, or with a capability-system pitfall you have seen.
- Once the M1 build exists, try booting it from a clean clone and report what broke. That is the most useful early signal.
- If you know x86-64, seL4, virtio or Rust `no_std` kernels, review the design notes as they appear.

Please open an issue before starting a large change. Contribution guidelines (`CONTRIBUTING.md`) will be added with the first code.

## About the name

*Durg* (दुर्ग) means *fortress* in Sanskrit and Hindi: a small, walled, hardened kernel that an untrusted agent can work inside but cannot break out of. It is also the name of a city in Chhattisgarh, India.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
