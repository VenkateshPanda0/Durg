# Desktop and laptop threat model — phase 0 working draft

Status: **draft, no desktop security claim**. This extends the existing
[agent VM threat model](../THREAT_MODEL.md), rather than replacing it. It is a
planning artifact for M18; no desktop subsystem is implied to exist. Each proposed
property requires design, negative tests and independent review before a release.

## Scope and trust boundaries

The target is an x86-64 UEFI desktop/laptop on an explicitly tested hardware
configuration, with multiple local accounts, applications, drivers, removable
media, updates and optional host or guest agents. Whether Durg itself owns the
hardware or runs as a guest of a Linux desktop remains an M18 decision.

| Boundary | Assets and intended control | Unresolved conditions |
|---|---|---|
| Firmware → bootloader → kernel | Boot integrity and recovery; authenticated components and owner-controlled recovery/development keys | Trust anchors, revocation, key rotation, rollback and firmware compromise policy |
| Devices / DMA → memory | Kernel, user data and secrets; restrict device DMA and MMIO through a validated isolation mechanism | IOMMU availability, device groups, hotplug and hardware without isolation |
| Drivers / services → kernel and other services | Contain faults and grant only necessary IRQ/device/memory capabilities | Driver reuse, reset, cancellation and resource accounting |
| Applications / users → data and devices | Per-user isolation; explicit capabilities for documents, mic, camera and screen | Identity, revocation, shared files and confused-deputy rules |
| Network / removable files → parsers | Protect user data and services from hostile packets, files and USB devices | Network stack, protocol choice, file parsing and update sources |
| Update signer / repository → installed system | Authenticate versioned updates and support safe recovery | Signing custody, compromised keys, revocation and downgrade policy |
| Disk / backups → owner and attacker | Confidentiality, integrity, restore and separation of backups from snapshots | Encryption mode, key recovery, hibernate, physical attacks and offline backup custody |
| Agent → tools / broker / human | Retain the original VM model: bounded authority, provenance, human approval and brokered egress | Guest/host handoff, approval scope, side effects that cannot be undone |
| Optional AI / telemetry → private data | Consent, least privilege, disablement, local-first processing and explicit remote disclosure | Model/runtime supply chain, retention, redaction and data export/deletion |

## Adversaries and assumptions

Consider malicious local applications and agents, remote peers, hostile files and
peripherals, a different unprivileged local account, accidental destructive actions,
and a stolen powered-off laptop. Administrator, recovery-key and firmware attacks
must be analyzed separately: an unlocked stolen device or a compromised trusted
signing key cannot be treated as safely protected merely by disk encryption.
The existing VM model trusts the host/VMM. For a Linux-hosted desktop, a compromised
host can compromise the Durg guest; for a native desktop, firmware, boot and kernel
become distinct trusted components. Neither design is assumed formally verified.

## Required evidence before claiming desktop support

1. Deny unauthorized cross-user/process access and capability escalation; prove
   revocation and task teardown under concurrency and low-memory conditions.
2. Exercise malformed USB/network/file inputs, driver crashes and hot-unplug; verify
   device DMA cannot access memory outside granted ranges on supported configurations.
3. Verify signed update and key rotation paths; inject power loss, corrupted images,
   failed boots and downgrade attempts. Recover without silently erasing user data.
4. Test offline local login, rate limits, encryption/recovery-key lifecycle and
   hibernate/suspend behavior on the named machine and firmware.
5. Test application permission denials and visible microphone/camera/screen access;
   isolate hostile previews and preserve accessible consent and revocation flows.
6. Validate agent-produced artifacts cannot cross into trusted execution without
   explicit acceptance; test broker bypass attempts and document irreversible effects.
7. Restore data from an independent backup, not merely an on-disk snapshot; test
   full disk, missing media and corrupted backup manifests.
8. Review boot, driver, parser, crypto, update and permission boundaries independently;
   document exclusions, residual risks, support ownership and incident response.

No claim of protection is made solely from this draft. Physical tampering, supply
chain compromise, side channels, vendor firmware and availability need explicit
scope and evidence; they must not silently inherit the original VM assumptions.

## Decisions to track

- Native Durg desktop versus Linux host + Durg guest; precise host/guest trust split.
- Supported device configurations and DMA/IOMMU fallback when hardware lacks protection.
- Cryptography, key ownership, update signing/revocation and offline recovery.
- App ABI, browser/compositor/driver reuse, licensing and provenance of dependencies.
- Backup encryption and key recovery, multi-user authority, permission revocation.
- Responsible maintainers, review capacity and severity-based response commitments.
