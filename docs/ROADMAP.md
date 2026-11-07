# Durg roadmap: capability foundation to desktop and laptop OS

Replanned 2026-11-08. This is the execution plan; dates are not delivery promises.

The goal now includes the original agent execution environment **and** a usable,
recoverable desktop/laptop OS on a named, tested hardware set. The custom Rust
capability kernel remains the current implementation. Expanding scope does not
make a GUI, hardware driver, Linux ABI or security guarantee exist today.

## Reading this plan

- [Original desktop/laptop requirements](requirements/universal-desktop-laptop-v1.md): complete owner-supplied specification, preserved verbatim.
- [Requirement coverage](requirements/coverage.md): every numbered requirement retains its original priority and is assigned a delivery milestone; non-numbered targets and journeys are mapped too.
- [Scope decision](adr/0012-desktop-laptop-roadmap.md): how the expanded goals change previous non-goals and which architecture choices remain open.
- [Previous roadmap](ROADMAP-before-desktop-expansion.md): historical snapshot, including all old backlog entries. It is not the current task order.

M0–M17 keep their historical IDs. New milestones M18–M29 are inserted by dependency,
not by numeric order. Source priorities **P0/P1/P2/Experimental are not phase numbers**.
P0 means required before the first desktop/laptop stable release, not the next commit.
Every imported requirement is planned, even when earlier kernel work supplies part
of its foundation. P1 remains in the backlog; P2 and Experimental require evidence
and an explicit release decision before implementation.

## Current baseline

| Existing milestone | Status and evidence boundary |
|---|---|
| M0–M1: threat model, ADRs, build, CI, first boot | Recorded complete for the original VM scope; the expanded threat model is new work |
| M2–M6: serial, traps, timer ticks, physical/virtual memory, heap | Recorded complete in QEMU; timer ticks are not yet a calibrated wall clock or hardware certification |
| M7–M9: threads, preemption, ring 3, syscalls, isolated ELF tasks | Recorded complete on the single-CPU VM target; not SMP, an app platform or general hardware support |
| M10 steps 1–4 | Capability logic, per-task spaces, endpoint IPC, badges/transfers and interim root-task spawner implemented |
| M10 remainder | In progress: task-owned objects/mapping, notifications, capability-space growth and blocked-thread lifecycle |
| Latest hardening | Spawner-retype refusal and checker-range fixes pushed; malformed-untyped-range work is local and not signed off |
| M11 onward and all desktop expansion | Planned; no implementation completion is implied |

The published `v0.1.0-alpha.1` is a **research pre-release**, not the desktop Alpha
release described by the imported specification. M1–M8 historical sign-off is the
owner's; later sign-off follows ADR-0010. New milestone closure needs test evidence,
review, threat-model updates where relevant, and a design-log entry.

## Dependency order

```text
M18 scope, hardware and architecture decisions (start now)
  + finish current M10 hardening
  -> M10 object lifecycle and complete capabilities
  -> M11 isolated VM drivers -> M12 durable storage
  -> M13 trust services -> M14 agent-sandbox demonstration
  -> M19 general-purpose kernel and service foundations
  -> M20 certified hardware bring-up
  -> M21 network, identity and system-service contracts
  -> M22 install, authenticated delivery, backup and recovery
  -> M23 accessible desktop infrastructure
  -> M16 WASI + M24 application platform and daily-use apps
  -> M25 laptop mobility -> M26 integrated desktop/laptop beta
  -> M27 compatibility, gaming, creator and workstation expansion
  -> M28 first stable release and maintenance
  -> M29 advanced and optional experiences

Optional side branches: M15 after M14; M17 after the x86-64 M28 gate.
```

This is the main integration order. Design work can overlap, but a feature cannot
ship before all its dependencies pass. For example, an accessibility API is designed
with the compositor, encryption before hibernation, and recovery before testing
an installer on a disk with valuable data. Finish one reviewable implementation slice
at a time; create tracked issues with owners and acceptance cases when work begins.

## M18 — Product scope and architecture gate (new; start now)

1. Preserve this plan and the requirement coverage; resolve contradictory scope statements.
2. Choose one named x86-64 UEFI desktop and one laptop, with exact GPU, NIC, storage,
   firmware and docking configurations. Models are **not selected yet**. Publish a
   compatibility table with Certified / Works with limitations / Community tested / Unknown.
3. Define initial users (developer/researcher first is the planning default), required
   browser/editor/toolchains and workloads; obtain real hardware and test-lab access.
4. Record ADRs for driver reuse, GPU/compositor, app ABI/ports, maintained browser,
   package/image layout, crypto libraries, signing-key ownership and support capacity.
5. Compare extending the Durg kernel with a Linux-based desktop hosting Durg as an
   isolated guest. Keep Durg-native as the planning baseline; do not silently migrate
   or claim that a host Linux feature is implemented by the Durg guest. If the native
   driver/browser feasibility spike fails, replan the desktop track through an ADR.
6. Expand the threat model to removable devices/DMA, hostile networks, multiple users,
   signed delivery, lost devices, recovery keys and optional AI. Preserve the original
   hostile-agent model, licensing and explicit authority boundaries.

**Exit:** approved architecture and hardware matrix, dependency/reuse choices,
benchmarks, feasibility evidence and a staffed maintenance plan. This roadmap
accepts the expanded goals; it does not pretend these technical decisions are settled.

## M10 — Finish capability and task lifecycle (current engineering priority)

1. Finish the local untyped-range regression and checker hardening; run host/property
   tests, negative controls and debug/release boot checks before committing it.
2. Specify ownership, quotas, capability revocation, blocked-call cancellation,
   exception delivery and object teardown. Decide what wakes peers when a task dies.
3. Implement CNode retype/growth, then thread, address-space and page-table objects
   funded by caller-owned untyped memory. Add multi-level lookup only if justified.
4. Implement map/unmap/protect and address-space teardown, validate user-supplied
   tables, reclaim empty tables, eliminate stale mappings before frame reuse, enforce
   W^X and per-task memory bounds. Account for kernel aliases and zero visible memory.
5. Configure/start/stop/join task objects; move ELF loading and initial authority
   assembly into the root-task services and remove or reduce the interim spawner.
6. Add notification signal/wait/badges, interrupt-ready delivery, reply capability
   transfer, `reply_recv`, killable blocked tasks and bounded resource exhaustion.
7. Test cross-task revocation, rights attenuation, cancellation, low-memory rollback,
   adversarial object sequences and eventual reclamation without authority resurrection.

**Exit:** root task builds child tasks using owned objects; no unaccounted task
allocation; unauthorized operations fail; teardown and revocation leave no usable
stale references. Notifications support the next driver milestone. M10 is not complete
merely because today's 108 kernel self-tests pass.

## M11 — Isolated drivers in the VM

Depends on M10. Define device/IRQ capabilities and DMA ownership, contiguous-frame
allocation, MMIO, rings, notification delivery, cancellation and device reset. Implement
virtio console, block and vsock in user space; add entropy with a reviewed trust model.
Bring up the service manager with dependency ordering, bounded restarts and health logs.
Calibrate monotonic time before timeouts; bound the boot-time timer check.

**Exit:** driver crash/restart and malformed device responses are contained; block
read/write/flush and broker transport pass integration tests; resources reclaim cleanly.

## M12 — Durable storage and file-service foundations

Depends on M11. Build the copy-on-write object store, object ownership, transactions,
flush ordering, snapshots, quotas and crash replay. Specify a filesystem-facing API
for later desktop paths: path lookup is scoped to directory capabilities, not ambient
access. Add metadata, large files and safe copy/move/delete semantics. Design backup
export/import now, keeping independent copies distinct from on-disk snapshots.

**Exit:** power-cut/fault injection, full disk, checksum errors, interrupted transfers,
snapshot restore and recovery tests pass. No silent truncation or reuse of live storage.

## M13 — Provenance, journal, approval and broker

Depends on M12. Decide label propagation and acceptance, hash-chain/checkpoint trust
and journal recovery. Implement the label/store/loader checks, one-use approval
capabilities, session manager, reversible effects and credential-injecting egress broker.
Bind approvals to operation/arguments/session and prevent replay. Document irreversible
external effects: sending a request cannot in general be undone by restoring storage.

**Exit:** adversarial trust-handoff, approval forgery/replay, secret exposure and
session rollback tests; recovery preserves journal/label consistency. Ordinary future
apps use permissioned system networking; untrusted agents still get brokered egress.

## M14 — Original agent product demonstration

Depends on M13. Build host controller/console, native guest tools and a small shell.
Run the README scenario: denied destructive access, labeled malicious hook, secret
broker handle, explicitly accepted output, session undo and inspectable attempts.
Provide reproducible setup, fault reports, SDK examples and a threat-model-linked demo.

**Exit:** repeatable positive and negative scenarios plus documented limitations.
This is the first agent demonstrator, not a desktop release or proof of universal safety.

## M19 — General-purpose kernel and service foundations

Depends on M14 and M18 decisions. First audit existing mitigations and reclaim paths;
then implement missing mechanisms rather than duplicating completed work:

1. Robust interrupt/exception handling including NMI/machine checks, `sysret` negative
   tests, thread-stack overflow diagnosis, calibrated timers, clocks and timeouts.
2. Per-CPU data and syscall entry, SMP-safe locks, scheduler/priority/fairness and
   memory accounting; stress multi-core and variable-RAM configurations.
3. Async I/O, memory-mapped file contracts, cache pressure and optional swap/compression,
   explicit OOM selection and bounded CPU/memory/I/O budgets.
4. Stable service/IPC lifecycle, entropy/crypto integration, logs/traces/crash dumps,
   diagnostics/settings APIs and safe developer mode. Enable ASLR/stack protections
   and audit direct-map aliases before executing broader untrusted applications.

**Exit:** SMP contention/teardown/pressure tests, no monopolization by a task,
repeatable latency baselines and symbolized fault reports with sensitive data controlled.

## M20 — Real hardware and authenticated boot

Depends on M19 and the M18 certified-target decisions. Preserve the QEMU path.
Implement UEFI/ACPI/PCI discovery, boot/recovery framebuffer, verified component loading,
owner-controlled development/recovery keys, interrupt routing and IOMMU/DMA isolation
before granting bus-mastering devices. Enumerate devices and expose unsupported states.

Bring up selected NVMe/SATA, USB HID/storage/hubs, keyboard/mouse/touchpad, Ethernet/Wi-Fi,
audio and camera drivers. Add authenticated driver lifecycle, safe detach/reset/rollback
and version/firmware inventory. GPU acceleration requires a feasible reviewed driver/API
route; basic framebuffer output is only a recovery step. Do not claim all Intel/AMD/NVIDIA
models from a single working GPU. Add hardware-health and thermal safety before stress.

**Exit:** named machine configurations boot and recover; driver failures, DMA boundaries,
hotplug and storage flush are tested. Essential devices and accelerated graphics work
on the advertised configurations. Existing Linux driver binaries do not automatically
work on the Durg kernel.

## M21 — Networking, identity and system services

Depends on M20 (VM development can begin using M11 drivers after M19).
Build permissioned TCP/UDP IPv4/IPv6, DNS/DHCP/routes/socket APIs, firewall, Wi-Fi
reconnection/diagnostics and offline behavior. Add Bluetooth and public/private/metered
profiles; then VPN, tethering, local discovery and scoped shares.

Provide offline local accounts, password hashing/rate limits, credential vaults,
roles and device/session ownership, timezone/locale/time sync and settings/diagnostic
contracts. Represent account authority through capabilities. Add authentication/audit
records, app network controls and privacy retention/export rules. Design multi-seat
ownership and radio transitions before implementing their UI.

**Exit:** account separation, denied device/network access, offline sign-in, network
loss/rejoin and service restart tests. Hostile-agent egress cannot bypass M13.

## M22 — Installation, packages, recovery and maintenance foundations

Depends on M12, M20–M21. Implement trusted package verification and version policy,
image-based/A-B or equivalent fail-safe OS upgrades, separate user-data migrations,
recovery boot, staged channels and scheduled restarts. Add key rotation and rollback
policy that distinguishes recovery from replaying a vulnerable signed image.

Build guided disk selection, explicit destructive confirmation, multi-boot awareness,
encryption/recovery keys, offline install/update, backups and verified granular restore.
Add authenticated driver/firmware paths only where supported, repair tools and exported
privacy-conscious crash logs. Test on disposable images/disks before any personal data.

**Exit:** full-disk/low-memory/interrupted update, wrong-disk refusal, encryption recovery,
rollback without user-data loss, offline install and off-device restore drills pass.
Do not postpone data safety until after the desktop MVP.

## M23 — Accessible desktop infrastructure and shell

Depends on M20–M22; UI design starts in M18. Build graphics/compositor and input routing,
text/font/Unicode infrastructure, window focus/resize/snap, multiple monitors/scaling,
keyboard navigation and accessibility semantics before apps rely on them.

Add launcher/taskbar/tray, settings, unified quick controls, notifications/Do Not Disturb,
error help, privacy indicators and permissions UI, onboarding, clock/calendar and audio
routing. Add themes, reduced motion/high contrast, IMEs/locale, shortcuts, virtual
desktops, clipboard privacy and session restore. Include screen-reader integration from
the start; touch/convertible behavior is gated on M25/M29 hardware, not guessed sensors.

**Exit:** primary shell/installer/settings flows work by keyboard and screen reader;
monitor disconnect preserves windows, display modes recover, local accounts and core
controls work offline, interaction p95/p99 and idle measurements are recorded.

## M16 + M24 — Application platform and desktop Alpha

Depends on M23 and the M18 app strategy. Keep M16's WASI-component runtime as a
portable tool path; it is not a substitute for a full browser/desktop ABI. Decide port
versus reuse with licensing/security review, and document versioned native APIs, UI
and accessibility toolkit, packaging, permissions and dependency isolation.

Deliver app catalog/install/remove/update, file associations and standard directory
APIs. Port a maintained browser (no original web engine), terminal/shell, editor,
image/media/PDF viewers, calculator, archive utility and screenshot tool. Add a file
manager with cancelable safe operations, trash, isolated previews, tags, local search
and exclusions. Provide C/C++, Python, Rust, JS/Node, Java and Git through supported
ports or an explicitly identified subsystem; never describe host tools as native ports.

**Exit:** desktop Alpha in a VM/test machine: browse, open local files, edit/build/test,
install/remove a verified package, run denied-permission cases and recover a failed
update. Publish exact supported app versions. Missing P0 apps block this gate.

## M25 — Laptop mobility and device-aware power

Depends on M20–M24. Implement battery/charger reporting with uncertainty, thermal
limits, AC/battery/quiet/performance policy, CPU/device idle and wake sources, lid/buttons,
function keys/backlight, touchpad and integrated media. Provide suspend, secure wake
and supported hibernate with encryption keys handled correctly. Add vendor-supported
charge limits, radio reconnection/captive portals, hybrid GPUs and certified docks/eGPUs.

Test bag-safe sleep/overnight drain, hotplug during suspend, dock/undock window/audio
continuity and low-battery actions. Desktop energy policy controls monitors/USB wake
without showing fictional laptop battery values.

**Exit:** named laptop firmware/device combinations pass repeated suspend/wake,
hibernate, charging and dock cycles, with drain/heat reports. Unsupported states are
explicitly disabled rather than advertised. Tablet/pen/hinge modes remain M29 extensions.

## M26 — Integrated desktop/laptop beta and everyday workflows

Depends on M24–M25. Integrate settings and utility flows with backups, large files,
removable filesystems, printing/scanning and print-to-PDF, network shares, optional
cloud clients, contacts/calendar standards and app autosave/session restoration.
Finish localization, accessibility alternatives, consent-based screen sharing and
local encrypted file/clipboard transfer; preserve retention and permission controls.

Qualify desktop upgrades, mixed-DPI/refresh multi-monitor reconnect, audio peripherals,
storage growth and quiet/performance profiles. Repeat installer/rollback tests on
both target form factors; run user journeys A, B, E, F and basic G from the source.

**Exit:** public beta on the named matrix with no release-blocking data loss, current
known limitations, usable restore, accessibility review and opt-in diagnostic handling.

## M27 — Compatibility, gaming, creators and workstations

Depends on M26 and graphics/API feasibility. Integrate selected compatibility runtimes,
containers/equivalent isolation and hardware VMs with quotas and documented boundaries.
Linux/Windows apps require explicit ports/runtime/guest support, not an assumed ABI.

Add tested launchers/controllers, fullscreen/alt-tab, low-latency input/audio/voice,
frame-time overlays, safe game profiles, recording/encoding and streaming. Add sustained
high-core/high-memory compute, supported GPU acceleration, professional color/HDR/VRR,
storage throughput and optional advanced network/lab tools. Publish app/game matrices
including version, drivers, offline/online behavior, anti-cheat, DRM and known failures.

**Exit:** journeys C, D and full G on certified configurations; no promise that every
game or vendor stack works. GAME-001's P0 graphics foundation is established in M20/M23
and must remain validated here; advanced P2 devices can stay explicitly deferred.

## M28 — Stable release, fleet management and sustained support

Depends on M26, M27's P0 gates and all unresolved P0 coverage rows. Implement required
administrative role/policy configuration, repeatable provisioning, consent-based remote
support, directory integration options, configuration import/export and minimal fleet
health reporting. Personal installs remain local-account/offline capable.

Run the complete software and hardware matrix, security review, supply-chain/signing
and response drills, accessibility/localization review and upgrade/restore rehearsal.
Publish certified devices, supported versions, known limitations, maintenance owners,
severity-based response goals and support funding/capacity. Channels are contracts,
not just labels. Track unresolved P1/P2 work in the next release, never as done.

**Exit:** every P0 requirement has linked tests/evidence on its applicable hardware,
all release-blocking issues resolved, independent security/accessibility review,
reproducible install/upgrade/rollback and a viable support policy. If resources or
compatibility are insufficient, stay beta or narrow the advertised hardware set.

## M29 — Advanced and optional experiences

Depends on M28 plus the relevant subsystem gates. Deliver optional local intelligence
only after the M13 approval/journal model and M23 accessible UI: scoped local search,
evidence-based diagnosis, action preview, delegated capabilities, reversible actions,
explicit remote processing and a full disable switch. Original model training is not
part of this roadmap; any runtime/model reuse needs a documented supply-chain decision.

Add task-first workspaces/preloading, cross-device continuation, migration assistance,
space analysis/dedup suggestions, guest/disposable sessions, application attestation,
compliance controls, bounded self-healing, specialist assistive/pro media devices,
VR/convertibles and UPS support where certified. Adaptive optimization remains
Experimental until measured privacy/safety evidence supports it. No silent deletion,
privilege grants, recording or network disclosure by the assistant.

**Exit per feature:** explicit enablement, permission-denial and disable tests, offline
fallback, privacy/retention checks, supported-device/app evidence and reversible policy.

## Preserved optional milestones

- **M15 — Firecracker-class/direct VMM boot and boot-time benchmark:** after M14;
  do not delay safety or app support to optimize boot numbers. Boot handoff and
  device model must be retested; any implementation choice needs an ADR.
- **M17 — ARM64:** after x86-64 M28 stability and a named hardware/test owner.
  Port interrupt, MMU, ABI, boot, drivers and power paths; repeat all applicable
  gates. Unsupported ARM devices, RISC-V and specialist industrial/medical hardware
  are not implied by the word “universal.”

## Engineering backlog carried forward

The previous roadmap's entire backlog is preserved in the historical snapshot.
These are the new destinations; old “later” entries are not automatically undone work.

| Existing item | Where to verify or implement it |
|---|---|
| Empty page-table reclamation; permission changes/huge-page splits; code aliases | M10 mapping/teardown and M19 review |
| SMEP/SMAP/UMIP; zero visible frames; copy boot information before reclaim | Audit existing implementation/evidence in M10; remaining gaps M19 |
| Kernel/user ASLR and guard gaps; heap poison/red zones; OOM rollback injection | M19 before broader untrusted apps |
| Contiguous frame allocation | M11 DMA ownership; reuse existing aligned-run logic where suitable |
| Calibrated timer; bounded tick wait; duration sleeping | M11 prerequisites, extended M19/M25 |
| No unaccounted task heap growth; per-task limits | M10 object ownership, M19 budgets |
| Thread-stack overflow diagnosis; NMI/machine checks; return-path negative tests | M19 |
| Blocked task cancellation and kill; IPC reply lifecycle | M10, with M19 scheduling policy |
| SMP-safe IrqLock; per-CPU syscall data/swapgs; coarse page-table locking | M19 before multicore claims |
| Eager 64 KiB stacks; scheduling policy and memory cost | Measure in M19; tune only with safety/performance tests |
| Fuzz page walks/copies; ELF, capability and syscall fuzzing | Retain existing targets; add missing parser/boundary coverage M10–M19; evaluate CI fuzz budget |
| Rust/tool versions and CI action runtimes | Maintain M18 onward with deliberately reviewed upgrades; do not assume old version warnings remain current |
| Label semantics; journal tamper evidence | M13 ADRs |
| WASI reuse; full networking vs broker | M16/M24 and M21 respectively; agent broker restriction remains |

## Measurement, acceptance and scope controls

Preserve source section 24 targets as **proposals**, not achieved benchmarks: 8 GB + SSD
recommended baseline to validate, about 15 s cold boot, about 3 s resume, a 16.7 ms
60 Hz frame budget with p95/p99, input latency and per-device idle/overnight power.
Measure timing boundaries, firmware, hardware, workload and versions; never improve
one number by silently sacrificing integrity, accessibility or battery safety.

At each gate: unit/property tests, debug/release integration, fuzzing and negative
controls for affected boundaries, fault injection, privacy/permission refusals and
actual logs. Add hardware-in-loop, long strings/bidi/IME, keyboard/screen reader,
network loss, full disk/RAM, unplugged media, encryption and interrupted-update cases
when those subsystems arrive. Tests and the source journeys A–G are release evidence,
not unchecked marketing boxes. Automated accessibility checks need human use testing.

No “all Windows/macOS apps,” “every laptop,” “no crashes,” “100% secure,” or guaranteed
battery/FPS claims. Snapshots, trash, redundancy, OS rollback and off-device backups
are distinct. Native macOS compatibility, a proprietary browser engine, every printer,
all codecs, unsupported architectures, full cloud office suites and original model
training remain outside the first stable scope. All source sections and checklist
items remain in the coverage plan even when conditional or explicitly deferred.

## Next five tasks

1. Finish and validate the existing local untyped-range change; do not count it as shipped.
2. Open M18 architecture/hardware/app decisions with feasibility evidence and named owners.
3. Specify M10 cancellation, mapping ownership and teardown invariants.
4. Implement/test CNode growth and task-owned thread/address-space/page-table objects.
5. Complete notifications and root-task migration, then begin M11 DMA/IRQ driver work.

If the desktop expansion proves infeasible, retain the working capability research
OS and teaching chapters, or propose the Linux-hosted desktop route explicitly.
The fallback is a conscious scope decision, not silent deletion of requirements.
