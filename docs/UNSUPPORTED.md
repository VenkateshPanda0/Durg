# What Durg does not support

These are deliberate non-goals, listed so nobody is surprised. If you want one of these, open an issue to discuss it. Expect an explanation of the cost and a request to justify changing the scope.

- **Desktop GUI**, framebuffer graphics, window system or compositor.
- **Real hardware drivers** (no AHCI, NVMe, Wi-Fi, USB, audio or GPU). Virtio only.
- **Linux binary or ABI compatibility.** Other projects already own that space.
- **A web browser**, gaming, printing, power management, suspend and resume.
- **Formal verification claims.** Durg borrows seL4's object model, not its proofs, and will not imply verification.
- **An AI model inside the kernel or OS.** The model runs on the host or behind an API. The OS only executes tool calls under capabilities.
- **Multiple CPU architectures early on.** x86-64 first. An ARM64 port is a later possibility, and RISC-V is not planned.
- **A custom bootloader.** Deferred. Durg uses the Limine protocol at first.
- **In-guest TCP/IP** in phase 1. Egress goes through the host-side broker.
- **Marketing claims** such as "more secure than Linux." Security claims must map to the [threat model](THREAT_MODEL.md).

Also out of scope for the threat model, such as side channels and a malicious host, are listed in [THREAT_MODEL.md](THREAT_MODEL.md#42-out-of-scope-v0).
