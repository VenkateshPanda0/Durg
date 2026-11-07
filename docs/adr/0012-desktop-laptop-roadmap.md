# ADR-0012: Expand the roadmap to desktop and laptop use

- **Status:** Accepted for scope and planning, 2026-11-08, following the owner's request to merge the supplied desktop/laptop specification with the existing roadmap.
- **Technical platform selection:** still open at the M18 gate; this ADR does not select a Linux migration.

## Context

Durg currently implements a custom Rust capability kernel in QEMU. The owner has
requested the desktop, laptop, gaming, workstation, accessibility, maintenance and
optional intelligence requirements in `docs/requirements/universal-desktop-laptop-v1.md`.
That document offers both a Linux-based product route and a from-scratch research
route. Its embedded recommendations are source material, not evidence that a
platform switch was requested or already implemented.

## Options

1. Keep desktop use permanently out of scope: does not satisfy the requested expansion.
2. Replace the kernel immediately with a Linux distribution: discards working research
   and silently settles a major product decision without hardware/app feasibility work.
3. Preserve Durg and its agent demonstration, add a gated desktop/laptop roadmap,
   and explicitly compare native expansion with a Linux desktop hosting Durg: chosen.

## Decision

- The integrated roadmap retains M0–M17 and adds M18–M29 in dependency order.
- GUI, real hardware, general-purpose app compatibility and local optional AI are
  long-term planned work, not permanent exclusions or current capabilities.
- ADR-0001's virtio-only driver restriction is narrowed to the original VM milestones.
  Its early broker-only network and native/WASI ABI choices remain for that stage;
  M21 adds permissioned system networking without granting raw egress to agents.
- The current custom capability kernel remains the planning baseline. M18 must settle
  driver/compositor reuse, browser/application ports, hardware targets and support
  capacity. A Linux-hosted desktop alternative requires a separate ADR and explicit
  host/guest security and feature attribution; no migration occurs in this change.
- Core Rust/capability design, Apache-2.0, unsafe discipline, original research and
  honest claims remain. Third-party guest dependencies still need recorded review;
  the expanded goals do not imply writing a new browser engine or crypto primitives.
- Preserve every source requirement and priority in a traceable coverage register.
  P0 gates desktop stable; later/conditional requirements remain visible, not “done.”
- Safety, recovery and accessibility are dependencies, not final cosmetic tasks.
  Formal verification, universal compatibility and guaranteed hardware support are
  not promised. Local AI stays outside the privileged kernel and is optional.

## Consequences

The scope is a long-term engineering program requiring hardware access, specialists,
maintainers and review. Milestones have exit criteria rather than speculative dates.
The existing pre-alpha release does not become a desktop Alpha by renaming it.
Published historical ADRs and the previous roadmap remain available; README,
contributor guidance and unsupported-feature documentation now distinguish current
limitations from future goals. The current local kernel changes are separate from
this documentation revision and are not signed off by it.
