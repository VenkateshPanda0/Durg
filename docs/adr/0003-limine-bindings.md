# ADR-0003: Limine bindings, own vs `limine` crate

- **Status:** Accepted (2026-10-15)
- **Needed by:** M1
- **Discussion:** [#8](https://github.com/VenkateshPanda0/Durg/issues/8)

## Context

The kernel boots via the Limine protocol, which hands over the memory map, higher-half direct-map offset and ACPI RSDP pointer through request and response structures. The originality policy allows a third-party crate in guest code only when it is a binding to a spec, and each one needs an ADR. The `limine` crate is such a binding.

## Options

1. **Write our own minimal bindings.** Only the requests we need (memory map, HHDM, RSDP, modules, perhaps the bootloader info).
   - Pro: consistent with the originality policy and a good learning exercise. Small surface.
   - Con: protocol revisions need tracking by hand, and a subtle layout bug is possible.
2. **Use the `limine` crate.**
   - Pro: less code, tracks the spec.
   - Con: an external dependency in the trusted kernel, and it hides details we want to understand.
3. **Start with the crate, replace with our own later.**
   - Pro: fastest path to a booting kernel.
   - Con: risks never replacing it.

## Decision

**Option 1.** Write our own minimal Limine bindings in the kernel crate, adding requests only as a milestone needs them.

## Consequences

- M1 needs only the base-revision tag, so the bootloader accepts the kernel. Memory map, HHDM and RSDP requests arrive with M2.
- We track the Limine protocol revision by hand. The protocol document is the source of truth, and each binding is tested by a QEMU boot.
- No third-party crate enters the kernel for boot handoff.
