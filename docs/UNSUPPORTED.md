# Current limits and planned expansion

Durg currently runs a pre-alpha research kernel in QEMU. The [roadmap](ROADMAP.md)
now includes desktop and laptop use under [ADR-0012](adr/0012-desktop-laptop-roadmap.md).
Planned support is not implemented or certified support.

## Not supported today, but planned

- Desktop GUI, window manager, accessible shell and maintained browser/apps: M23–M24.
- Real-hardware NVMe/SATA, USB, GPU, audio and network drivers: M20 onward, named devices only.
- General IP networking: M21. Untrusted-agent egress remains subject to the M13 broker.
- Offline user accounts, installation, encryption, authenticated updates and recovery: M21–M22.
- Laptop power, safe suspend/hibernate, docking and battery diagnostics: M25.
- Linux/Windows application compatibility, gaming and workstations: M27, subject to feasible runtimes, ports, vendor constraints and tested matrices.
- Optional local intelligence: M29, outside the privileged kernel, with scoped permissions and a disable switch.
- ARM64: M17 after x86-64 stability and a named hardware/test commitment.

## Not promised

- Formal verification or “more secure than Linux” without evidence.
- Every GPU, laptop, Windows game, anti-cheat system, codec or peripheral working.
- Native macOS app compatibility, an original browser engine or original AI model training in the first stable release.
- Unknown ARM platforms, RISC-V or specialist medical/industrial controllers in the first release.
- Protection from every side channel or a malicious VM host under the existing threat model.

The [threat model](THREAT_MODEL.md) still governs the implemented agent research
scope. M18 expands it before new hardware, accounts or desktop security claims.
Limine remains the boot path; a custom bootloader is not required by this plan.
