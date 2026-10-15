# ADR-0003: Limine bindings, own vs `limine` crate

- **Status:** Proposed (awaiting owner's decision)
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

## Recommendation

**Option 1.** The needed protocol surface is small, and writing it is a good first lesson in the boot handoff. Test it with a QEMU boot that prints the memory map (M2). Revisit if the maintenance cost proves high.

## Consequences

To be filled in once decided.
