# Requirement coverage register

Baseline: 2026-11-08. Source: [Universal Desktop & Laptop OS Requirements v1](universal-desktop-laptop-v1.md).
Execution order and acceptance gates: [integrated roadmap](../ROADMAP.md).

Each row identifies a primary delivery milestone, not an independent implementation order.
All milestones inherit their roadmap dependencies. The source's complete wording,
priorities, applicability, caveats and acceptance examples remain authoritative product
input; this register does not replace them with shorter titles. **Planned** means the
complete imported requirement is not signed off. Earlier kernel features may partially
contribute without completing it. P0 gates M28 stable on applicable hardware; a change
to that scope requires an explicit decision. P1 is tracked for delivery/expansion;
P2 is deferred where assigned to M29, and Experimental requires a separate evidence gate.

Cross-cutting contracts: M10 authority → M11 devices → M12 data integrity → M13 agent
trust; M18 scope/architecture → M19 kernel → M20 hardware → M21 identity/network →
M22 recovery/delivery → M23 UI/accessibility → M24 apps → M25 mobility → M26 beta →
M27 compatibility → M28 stable → M29 optional features. M16 supplies portable tools,
M15 is a VMM optimization side branch and M17 is a later architecture port.

**Coverage:** 235 unique numbered requirements, each assigned exactly once below.

## HW

Certify individual models and firmware; basic bring-up does not prove every driver or vendor works.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| HW-001 | P0 / Both | Standard platform support | M20 | Planned |
| HW-002 | P0 / Both | Device enumeration | M20 | Planned |
| HW-003 | P0 / Both | Driver lifecycle | M20 | Planned |
| HW-004 | P0 / Both | CPU and memory support | M20 | Planned |
| HW-005 | P1 / Both | Firmware integration | M20 | Planned |
| HW-006 | P1 / Both | Hardware health | M20 | Planned |
| HW-007 | P0 / Both | Storage controllers | M20 | Planned |
| HW-008 | P1 / Both | External storage | M20 | Planned |
| HW-009 | P1 / Desktop | Advanced storage | M29 | Planned |
| HW-010 | P0 / Both | Display output | M20 | Planned |
| HW-011 | P0 / Both | Multiple displays | M23 | Planned |
| HW-012 | P1 / Both | Modern panel controls | M23 | Planned |
| HW-013 | P1 / Laptop | Hybrid graphics | M25 | Planned |
| HW-014 | P2 / Desktop | High-end graphics | M29 | Planned |
| HW-015 | P1 / Optional hardware | eGPU/docks | M25 | Planned |
| HW-016 | P0 / Both | Essential input | M20 | Planned |
| HW-017 | P0 / Both | USB peripherals | M20 | Planned |
| HW-018 | P0 / Both | Audio | M20 | Planned |
| HW-019 | P1 / Laptop | Integrated hardware | M25 | Planned |
| HW-020 | P1 / Both | Bluetooth | M21 | Planned |
| HW-021 | P1 / Both | Printers and scanners | M26 | Planned |
| HW-022 | P2 / Optional hardware | Special devices | M29 | Planned |

## CORE

Existing M0–M9 evidence is partial foundation; full source contract still needs implementation and tests.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| CORE-001 | P0 | Verified boot path | M20 | Planned |
| CORE-002 | P0 | Boot diagnostics | M20 | Planned |
| CORE-003 | P0 | User session startup | M23 | Planned |
| CORE-004 | P1 | Multi-boot awareness | M22 | Planned |
| CORE-005 | P0 | Process isolation | M10 | Planned |
| CORE-006 | P0 | Scheduling | M19 | Planned |
| CORE-007 | P0 | Memory management | M19 | Planned |
| CORE-008 | P0 | System calls and IPC | M10 | Planned |
| CORE-009 | P0 | Interrupts and timers | M19 | Planned |
| CORE-010 | P0 | Filesystem and device I/O | M12 | Planned |
| CORE-011 | P1 | Resource controls | M19 | Planned |
| CORE-012 | P1 | Kernel observability | M19 | Planned |
| CORE-013 | P0 | Service manager | M11 | Planned |
| CORE-014 | P0 | Time and locale | M21 | Planned |
| CORE-015 | P0 | Local accounts | M21 | Planned |
| CORE-016 | P1 | Multi-seat/session support | M21 | Planned |
| CORE-017 | P1 | System diagnostics API | M21 | Planned |

## PERF

Measure per device/workload; no improvement claim without comparable baselines.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| PERF-001 | P0 | Foreground responsiveness | M19 | Planned |
| PERF-002 | P0 | Efficient idle behavior | M19 | Planned |
| PERF-003 | P0 | Memory pressure handling | M19 | Planned |
| PERF-004 | P0 | Startup governance | M23 | Planned |
| PERF-005 | P1 | Adaptive scheduling | M19 | Planned |
| PERF-006 | P1 | Storage efficiency | M12 | Planned |
| PERF-007 | P1 | Performance dashboard | M23 | Planned |
| PERF-008 | P1 | Resource budgets | M19 | Planned |
| PERF-009 | P2 | Smart workload preloading | M29 | Planned |
| PERF-010 | P1 | Honest optimization | M18 | Planned |

## MOB

Named laptop/firmware evidence required; unsupported power states must be disabled.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| MOB-001 | P0 / Laptop | Accurate battery state | M25 | Planned |
| MOB-002 | P0 / Laptop | Power profiles | M25 | Planned |
| MOB-003 | P0 / Laptop | Sleep, wake, and hibernate | M25 | Planned |
| MOB-004 | P0 / Laptop | Lid and button actions | M25 | Planned |
| MOB-005 | P0 / Laptop | Thermal safeguards | M25 | Planned |
| MOB-006 | P1 / Laptop | Charging controls | M25 | Planned |
| MOB-007 | P1 / Laptop | Battery diagnostics | M25 | Planned |
| MOB-008 | P1 / Laptop | Docking transitions | M25 | Planned |
| MOB-009 | P1 / Laptop | Connectivity on the move | M25 | Planned |
| MOB-010 | P1 / Laptop | Safe sleep in bags | M25 | Planned |
| MOB-011 | P2 / Optional hardware | Tablet/convertible mode | M29 | Planned |

## DESK

Publish supported device and workload combinations; do not infer universal support.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| DESK-001 | P0 / Desktop | Upgrade friendliness | M20 | Planned |
| DESK-002 | P0 / Desktop | Multi-monitor productivity | M23 | Planned |
| DESK-003 | P1 / Desktop | Hardware performance modes | M26 | Planned |
| DESK-004 | P1 / Desktop | Advanced audio and input | M26 | Planned |
| DESK-005 | P1 / Desktop | Storage growth | M26 | Planned |
| DESK-006 | P1 / Workstation | High-memory and high-core workloads | M27 | Planned |
| DESK-007 | P1 / Workstation | Compute acceleration | M27 | Planned |
| DESK-008 | P2 / Workstation | Pro device ecosystem | M29 | Planned |
| DESK-009 | P2 / Desktop | UPS integration | M29 | Planned |
| DESK-010 | P1 / Desktop | Quiet vs. maximum performance | M26 | Planned |

## UI

Depends on M20 devices, M21 services and M22 recovery; accessibility is part of the UI contract.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| UI-001 | P0 | Clear desktop | M23 | Planned |
| UI-002 | P0 | Consistent window manager | M23 | Planned |
| UI-003 | P0 | Fast app discovery | M23 | Planned |
| UI-004 | P0 | Settings that make sense | M23 | Planned |
| UI-005 | P0 | Good visual fundamentals | M23 | Planned |
| UI-006 | P1 | Customization | M23 | Planned |
| UI-007 | P1 | Smart tiling | M23 | Planned |
| UI-008 | P1 | Per-display behavior | M23 | Planned |
| UI-009 | P1 | Session restore | M23 | Planned |
| UI-010 | P1 | Touch adaptations | M29 | Planned |
| UI-011 | P0 | First-run onboarding | M23 | Planned |
| UI-012 | P0 | Sensible notifications | M23 | Planned |
| UI-013 | P0 | Useful errors | M23 | Planned |
| UI-014 | P1 | Unified quick settings | M23 | Planned |
| UI-015 | P1 | Clipboard history | M23 | Planned |
| UI-016 | P1 | Keyboard-first workflows | M23 | Planned |
| UI-017 | P1 | Contextual help | M23 | Planned |
| UI-018 | P2 | Task-oriented workspaces | M29 | Planned |

## FILE

Storage semantics start in M12; GUI, privacy and restore integration follows M22–M26.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| FILE-001 | P0 | Reliable file manager | M24 | Planned |
| FILE-002 | P0 | Safe file operations | M12 | Planned |
| FILE-003 | P0 | Standard filesystems | M12 | Planned |
| FILE-004 | P0 | Trash and recovery | M24 | Planned |
| FILE-005 | P1 | Content search | M24 | Planned |
| FILE-006 | P1 | Previews | M24 | Planned |
| FILE-007 | P1 | Metadata and tags | M24 | Planned |
| FILE-008 | P1 | Version history | M22 | Planned |
| FILE-009 | P1 | Disk management | M22 | Planned |
| FILE-010 | P1 | Encryption | M22 | Planned |
| FILE-011 | P1 | Network files | M21 | Planned |
| FILE-012 | P1 | Cloud-agnostic sync | M26 | Planned |
| FILE-013 | P2 | Deduplication and smart space analysis | M29 | Planned |
| FILE-014 | P1 | Large-file workflows | M12 | Planned |

## APP

M18 feasibility/reuse decision first; maintained ports and explicit compatibility evidence required.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| APP-001 | P0 | Install/uninstall | M22 | Planned |
| APP-002 | P0 | Trusted packages | M22 | Planned |
| APP-003 | P0 | Default applications | M24 | Planned |
| APP-004 | P0 | Stable native API | M24 | Planned |
| APP-005 | P1 | App permissions | M23 | Planned |
| APP-006 | P1 | Sandboxed app option | M23 | Planned |
| APP-007 | P1 | Browser-based apps | M24 | Planned |
| APP-008 | P1 | Cross-platform dev ecosystem | M24 | Planned |
| APP-009 | P1 | Legacy app options | M27 | Planned |
| APP-010 | P1 | Update integrity | M22 | Planned |
| APP-011 | P2 | App migration | M29 | Planned |
| APP-012 | P1 | File associations | M24 | Planned |

## GAME

M20/M23 accelerated graphics and M24 app foundations are prerequisites; vendor/anti-cheat constraints apply.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| GAME-001 | P0 | Graphics foundation | M20 | Planned |
| GAME-002 | P1 | Game storefronts and launchers | M27 | Planned |
| GAME-003 | P1 | Controller support | M27 | Planned |
| GAME-004 | P1 | Fullscreen and display modes | M27 | Planned |
| GAME-005 | P1 | Low-latency input/audio | M27 | Planned |
| GAME-006 | P1 | Frame pacing and monitoring | M27 | Planned |
| GAME-007 | P1 | Performance profiles | M27 | Planned |
| GAME-008 | P1 | Capture and streaming | M27 | Planned |
| GAME-009 | P1 | Audio communications | M27 | Planned |
| GAME-010 | P1 | Compatibility transparency | M27 | Planned |
| GAME-011 | P2 | HDR/VRR and advanced features | M29 | Planned |
| GAME-012 | P2 | Gaming workspace | M29 | Planned |

## NET

Permissioned desktop networking does not bypass the M13 agent broker.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| NET-001 | P0 | Wired Ethernet and Wi-Fi | M21 | Planned |
| NET-002 | P0 | Modern IP networking | M21 | Planned |
| NET-003 | P0 | Network diagnostics | M21 | Planned |
| NET-004 | P0 | Personal firewall | M21 | Planned |
| NET-005 | P1 | VPN support | M21 | Planned |
| NET-006 | P1 | Hotspot and tethering | M21 | Planned |
| NET-007 | P1 | Local discovery | M21 | Planned |
| NET-008 | P1 | Private network profiles | M21 | Planned |
| NET-009 | P1 | Network quality | M21 | Planned |
| NET-010 | P2 | Advanced routing | M27 | Planned |
| NET-011 | P1 | Reliable offline behavior | M21 | Planned |

## SEC

Review and negative tests required; capability research is not a production security certification.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| SEC-001 | P0 | Least privilege | M10 | Planned |
| SEC-002 | P0 | Process boundaries | M10 | Planned |
| SEC-003 | P0 | Secure authentication | M21 | Planned |
| SEC-004 | P0 | Verified software delivery | M22 | Planned |
| SEC-005 | P0 | Patch response | M28 | Planned |
| SEC-006 | P1 | Disk encryption | M22 | Planned |
| SEC-007 | P1 | Strong second factors | M21 | Planned |
| SEC-008 | P0 | Network protection | M21 | Planned |
| SEC-009 | P1 | Sandboxing and capabilities | M10 | Planned |
| SEC-010 | P1 | Malware defenses | M19 | Planned |
| SEC-011 | P1 | Camera/microphone indicators | M23 | Planned |
| SEC-012 | P1 | External-device safety | M20 | Planned |
| SEC-013 | P1 | Memory exploit mitigations | M19 | Planned |
| SEC-014 | P2 | Application attestation | M29 | Planned |

## PRIV

Policy is set early and tested in each relevant feature; no hidden online dependency.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| PRIV-001 | P0 | Offline-first local account | M21 | Planned |
| PRIV-002 | P0 | Data minimization | M18 | Planned |
| PRIV-003 | P0 | Permission visibility | M23 | Planned |
| PRIV-004 | P0 | No forced advertisements | M18 | Planned |
| PRIV-005 | P0 | Delete and export | M23 | Planned |
| PRIV-006 | P1 | Private search | M24 | Planned |
| PRIV-007 | P1 | App network controls | M21 | Planned |
| PRIV-008 | P1 | Account and permission auditing | M21 | Planned |
| PRIV-009 | P1 | Device recovery options | M22 | Planned |
| PRIV-010 | P2 | Temporary/private sessions | M29 | Planned |
| PRIV-011 | P1 | Real user choice | M18 | Planned |

## REL

Verify restores and fault containment; snapshots and rollback do not replace off-device backups.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| REL-001 | P0 | Crash resilience | M19 | Planned |
| REL-002 | P0 | Power-loss safety | M12 | Planned |
| REL-003 | P0 | Recovery environment | M22 | Planned |
| REL-004 | P0 | Restore points / rollback | M22 | Planned |
| REL-005 | P0 | Backups | M22 | Planned |
| REL-006 | P1 | Backup separation | M22 | Planned |
| REL-007 | P1 | Silent fault detection | M26 | Planned |
| REL-008 | P1 | Application recovery | M24 | Planned |
| REL-009 | P1 | Restore granularity | M22 | Planned |
| REL-010 | P1 | Crash reports | M19 | Planned |
| REL-011 | P2 | Self-healing services | M29 | Planned |

## UPD

Test on disposable media; signed artifacts alone do not guarantee safe rollback or data migration.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| UPD-001 | P0 | Clear installer | M22 | Planned |
| UPD-002 | P0 | Reproducible install | M22 | Planned |
| UPD-003 | P0 | Authenticated updates | M22 | Planned |
| UPD-004 | P0 | No surprise restarts | M22 | Planned |
| UPD-005 | P0 | Atomic or fail-safe updates | M22 | Planned |
| UPD-006 | P0 | Separate user data | M22 | Planned |
| UPD-007 | P1 | Staged deployments | M22 | Planned |
| UPD-008 | P1 | Driver and app update channels | M22 | Planned |
| UPD-009 | P1 | Repair tools | M22 | Planned |
| UPD-010 | P1 | Release channels | M22 | Planned |
| UPD-011 | P1 | End-of-support clarity | M28 | Planned |
| UPD-012 | P2 | Migration assistant | M29 | Planned |

## DEV

Native, subsystem and VM support must be distinguished; avoid insecure global developer defaults.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| DEV-001 | P0 | Terminal and shell | M24 | Planned |
| DEV-002 | P0 | Programming toolchains | M24 | Planned |
| DEV-003 | P0 | Debugging fundamentals | M19 | Planned |
| DEV-004 | P1 | Package and dependency tooling | M22 | Planned |
| DEV-005 | P1 | Containers | M27 | Planned |
| DEV-006 | P1 | Virtual machines | M27 | Planned |
| DEV-007 | P1 | Developer mode | M19 | Planned |
| DEV-008 | P1 | Observability | M19 | Planned |
| DEV-009 | P1 | Remote tools | M21 | Planned |
| DEV-010 | P1 | Documentation and SDK | M24 | Planned |
| DEV-011 | P1 | Automation interfaces | M21 | Planned |
| DEV-012 | P2 | Kernel experimentation | M15 | Planned |
| DEV-013 | P2 | Security-lab features | M27 | Planned |
| DEV-014 | P1 | Developer-friendly filesystem | M12 | Planned |

## ACC

Design begins at M18/M23; require human accessibility testing as well as automation.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| ACC-001 | P0 | Keyboard accessibility | M23 | Planned |
| ACC-002 | P0 | Screen-reader architecture | M23 | Planned |
| ACC-003 | P0 | Readability | M23 | Planned |
| ACC-004 | P1 | Input alternatives | M23 | Planned |
| ACC-005 | P1 | Sensory preferences | M23 | Planned |
| ACC-006 | P1 | Speech and captions | M29 | Planned |
| ACC-007 | P1 | Multilingual UI | M23 | Planned |
| ACC-008 | P1 | Regional formats | M23 | Planned |
| ACC-009 | P1 | Input methods | M23 | Planned |
| ACC-010 | P2 | Assistive device support | M29 | Planned |

## PROD

Uses M23 UI and M24 apps; cross-device features require consent, authentication and retention controls.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| PROD-001 | P0 | Core utilities | M24 | Planned |
| PROD-002 | P1 | Workspaces and virtual desktops | M23 | Planned |
| PROD-003 | P1 | Focus policies | M23 | Planned |
| PROD-004 | P1 | Cross-device file transfer | M26 | Planned |
| PROD-005 | P1 | Clipboard transfer | M26 | Planned |
| PROD-006 | P1 | Remote desktop/screen sharing | M26 | Planned |
| PROD-007 | P1 | Calendar/contacts integration | M26 | Planned |
| PROD-008 | P1 | Print to PDF and share | M26 | Planned |
| PROD-009 | P2 | Task-first launcher | M29 | Planned |
| PROD-010 | P2 | Multi-device continuation | M29 | Planned |

## AI

Optional, off unless enabled; depends on M13 authority/journal and M23 accessible UI; not privileged kernel code.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| AI-001 | P2 | Natural-language search | M29 | Planned |
| AI-002 | P2 | Explain system behavior | M29 | Planned |
| AI-003 | P2 | Proposed actions first | M29 | Planned |
| AI-004 | P2 | Reversible automation | M29 | Planned |
| AI-005 | P2 | Local-first inference | M29 | Planned |
| AI-006 | P2 | Narrow permissions | M29 | Planned |
| AI-007 | P2 | Accessible interface | M29 | Planned |
| AI-008 | Experimental | Adaptive task optimization | M29 | Planned |

## ADMIN

Keep personal offline accounts usable; fleet collection and remote control need explicit policy and consent.

| ID | Original priority / applicability | Requirement | Primary milestone | Status |
|---|---|---|---|---|
| ADMIN-001 | P1 | Multiple user roles | M28 | Planned |
| ADMIN-002 | P1 | Policy configuration | M28 | Planned |
| ADMIN-003 | P1 | Fleet provisioning | M28 | Planned |
| ADMIN-004 | P1 | Centralized diagnostics | M28 | Planned |
| ADMIN-005 | P1 | Remote support | M28 | Planned |
| ADMIN-006 | P1 | Directory integration | M28 | Planned |
| ADMIN-007 | P2 | Compliance features | M28 | Planned |
| ADMIN-008 | P1 | Configuration portability | M28 | Planned |

## Non-numbered source material and release evidence

The source contains more than numbered requirements. All 30 sections are retained;
these mappings carry their policies, tables and examples into the execution plan.

| Source section | Destination and treatment |
|---|---|
| 1: vision, promises and success | M18 product definition; M28 validation across desktop/laptop users |
| 2: priority labels, rules, profiles | M18 governance; M23/M25/M27 transparent reversible profiles |
| 3: users and devices | M18 named desktop/laptop and required-app selection; M17/M29 later devices |
| 4: hardware and compatibility grades | M20 base matrix; M25 mobility; M27 advanced; M28 certification |
| 5: OS fundamentals | M10–M12, M19–M23; imported general-purpose requirements are not retroactively marked done |
| 6: performance and measurement caution | M18 benchmarks, M19 resource controls, M25 power, M27 workloads |
| 7: mobility and desktop equivalents | M25; UPS extension M29 |
| 8: desktops/workstations | M20/M23/M26 platform; M27 scaling; M29 specialist devices |
| 9: GUI and interaction principles | M23 from first UI implementation; M24 apps; M29 optional workspaces/touch |
| 10: files and data | M12 primitives, M22 recovery, M24/M26 workflows, M29 advanced analysis |
| 11: application compatibility table/rules | M18 route decision, M16/M24 native/WASI/web, M27 runtimes/VMs; no universal Windows/macOS claim |
| 12: games and vendor/DRM/anti-cheat risks | M20 graphics feasibility, M27 measured compatibility, M29 advanced modes |
| 13: networking | M21 with scoped permissions and agent-broker separation |
| 14: threat matrix and security engineering | M18 expanded threat model; M10–M28 applicable boundary tests, independent reviews, supply chain and response |
| 15: privacy principles | M18 policy; M21 identities; M23 permission UI; M28 release audit; M29 optional AI |
| 16: reliability and five distinct protections | M12/M22 design distinguishes trash, snapshots, backups, redundancy and OS rollback |
| 17: updates and user scheduling | M22 implementation; M28 supported releases and maintenance |
| 18: development and experimentation | M19 observability, M24 SDK/toolchains, M27 VMs/lab; M15 optional optimized VMM path |
| 19: accessibility/internationalization | M23 semantics and keyboard access, M26 usability/localization; M29 advanced devices/speech |
| 20: productivity and cross-device | M24 core apps; M26 consent-based sharing; M29 continuation/task-first features |
| 21: AI examples and prohibited silent actions | M29 after M13 trust services; search/diagnosis/workspaces/storage/microphone examples require permission tests |
| 22: administration | M28 policy/provisioning/support; M29 advanced compliance |
| 23: architecture alternatives, component table, decisions | M18 ADR gate; layered services M19–M24; Linux option remains a decision, not an implemented migration |
| 24: quantified targets | M18 baseline protocol; measure boot M20/M15, frames M23, resume/power M25, latency M27, all gates M28 |
| 25: journeys A–G | A/B/E/F at M26; C/D and full G at M27; repeat all at M28 |
| 26: automated, hardware, upgrade and release matrices | Per-milestone tests; M24 desktop Alpha, M26 beta, M28 stable; hardware preview only after recovery and named-device tests |
| 27: limitations, tradeoffs, first-release exclusions | ADR-0012 and roadmap scope controls; explicitly conditional/deferred, never universal-support promises |
| 28: paths A/B and solo order | Existing B0–B2 work preserved; M18 compares native route and Linux host; recovery and accessibility moved earlier than cosmetic polish |
| 29: master checklist | All categories roll up from numbered rows and M28 gates; check a box only with evidence, not from this plan |
| 30: glossary and next decisions | Reference terminology retained; hardware, apps, architecture, licensing and smallest supported release settled at M18 |

## Record completion

For each delivered row, add the implementation issue/PR, tested commit, relevant
host/VM/hardware results, failure cases, reviewer, known limitations and release.
Multi-device requirements need evidence for each advertised configuration. Do not
mark a broad row complete because one subfeature works. No percentages or release
dates are inferred from the number of rows. Deferred/conditional requirements remain
visible for future scope decisions.
