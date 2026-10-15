# ADR-0007: The kernel owns its page tables, with W^X, a minimal direct map and a guarded stack

- **Status:** Accepted (2026-10-15). An implementation decision inside the M5 scope in the roadmap ("take over from the bootloader's tables with a kernel higher-half mapping we control; guard pages"). Flagged here because it fixes policy other milestones build on, and the owner can overturn it.
- **Milestone:** M5

## Context

At entry the CPU runs on page tables the bootloader built. The Limine protocol guarantees they map what the kernel needs, but leaves their layout unspecified, puts them in bootloader-reclaimable memory, and maps more than the kernel wants: the kernel image appears again through the higher-half direct map (HHDM) as a **writable alias of its own code**, and the framebuffer is mapped too. The boot stack lives in memory that should eventually return to the pool, and it has no guard page.

## Decision

1. **Build a new address space from scratch, verify it, then switch.** Every kernel image page must translate to the same frame under the old and new tables, the running stack must be mapped, and every direct-map region the kernel will read must be present. If a check fails, the kernel keeps the bootloader's tables and says why. A wrong switch is a triple fault with no output, so the checks are not optional.
2. **W^X for the image.** Section boundaries come from the linker script. Code is readable and executable but not writable. Read-only data is neither writable nor executable. Data and bss are writable and not executable. No page is both.
3. **The kernel image is not mapped in the direct map.** It is reachable only at its own addresses with the permissions above, so there is no writable alias of its code.
4. **The direct map covers only what the kernel uses:** usable RAM and bootloader-reclaimable memory (read-write, no-execute), and ACPI reclaimable, ACPI NVS and reserved-mapped memory (read-only, no-execute). The framebuffer and reserved ranges are not mapped.
5. **A kernel stack with a guard page.** 64 KiB in its own virtual region (PML4 slot 508), with an unmapped page directly below. An overflow is a page fault; because delivering that fault needs the exhausted stack, it escalates to a double fault, which runs on the dedicated IST1 stack and recognises the guard page by the faulting address.
6. **Copy the boot information, then reclaim.** Before anything reuses bootloader memory, the memory map, HHDM offset and RSDP address are copied into kernel memory (`limine::capture`). After the switch and the move to the new stack, the bootloader-reclaimable memory above 1 MiB (about 41 MiB on QEMU with 256 MiB) joins the frame pool. The reclaim step refuses to run before the kernel is on its own page tables.

## Consequences

- The kernel boots in two stages: stage 1 on the bootloader's stack and tables, stage 2 on its own.
- `mm::paging` is general: an `AddressSpace` can be built, edited and activated, with map, 2 MiB map, range map, unmap and translate. Unmapping invalidates the TLB entry (a test fails without it).
- **The direct map is writable for all RAM.** Memory that later holds code (for example loaded user programs, M9) will have a writable kernel alias through the direct map. That is acceptable while the kernel is trusted, but the loader must be written with it in mind.
- **Reclaimed frames contain old bootloader data.** `alloc_frame` does not clear them. Anything handed to an untrusted task must come from `alloc_zeroed_frame`.
- **Not done:** SMEP, SMAP and UMIP (they matter once user mode exists, M8), kernel address randomisation, freeing empty page-table frames on unmap, splitting huge pages, changing the permissions of an existing mapping.
- Revert path for the policy: each point is a few lines in `mm::vm`.
