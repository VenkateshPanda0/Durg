# ADR-0006: A minimal page mapper before M5, to reach the local APIC

- **Status:** Accepted (2026-10-15, owner chose option A)
- **Milestone:** M3

## Context

The local APIC timer (an M3 deliverable) needs access to the local APIC's registers. In the xAPIC interface they are memory-mapped at a physical address normally `0xfee00000`. The bootloader's page tables map only the regions in its memory map (usable RAM, bootloader-reclaimable, kernel, framebuffer, and with base revision 4 the ACPI regions). The APIC page is in none of them, so it is **not mapped** when the kernel starts. Page-table control is scheduled for M5, after the timer.

Evidence gathered before deciding:

- x2APIC would avoid the mapping (registers are MSRs), but the QEMU CPU model we boot with reports no x2APIC, and QEMU 8.2.2 software emulation does not support it at all (`TCG doesn't support requested feature: CPUID.01H:ECX.x2apic`, for both `-cpu max` and `-cpu Haswell`). CI cannot test it.
- The bootloader's page tables are in bootloader-reclaimable memory, reachable through the direct map, with an unspecified layout (Limine protocol, "Memory map").
- The bootloader guarantees the PAT layout, with PAT3 uncacheable (Limine protocol, "Caching"), so an uncached MMIO mapping is expressible with the cache-disable and write-through bits.

## Options

- **A. A tiny page mapper now.** Build the first real piece of M5 (entry formats, address splitting, a table walker, and mapping one 4 KiB page), scoped to what the timer needs, and finish M3 in order.
- **B. Move the timer after M5.** Reorder the milestones. M3 ends without a timer, M5 comes first.
- **C. x2APIC under KVM only.** No mapper, but CI cannot exercise it.

## Decision

**Option A.** The mapper is built as the start of M5, not as throwaway code:

- The pure logic (entries, address splitting, walking) is a host-tested crate, `durg-paging`, with `#![forbid(unsafe_code)]`. The walk takes a reader closure, so tests run it over fake page tables.
- The kernel module `mm::paging` supplies the reader (through the higher-half direct map, after checking the table is in memory the bootloader maps) and a `map_page` that edits the bootloader's tables in place.
- Intermediate tables come from a small pool of four page-table pages reserved in the kernel image, because there is no physical memory manager until M4. M4 and M5 replace the pool with real frame allocation.

  **Update (M4):** the pool is gone. New page tables come from the physical frame allocator (`alloc_zeroed_frame`).

## Consequences

- The local APIC timer is part of M3, and the kernel counts timer interrupts at boot.
- For now the kernel edits tables that the bootloader owns, in memory the bootloader marks reclaimable. Nothing may reclaim that memory until M5 takes over the page tables.
- A fixed virtual window (PML4 slot 510, `0xffff_ff00_0000_0000`) is reserved for device registers.
- The mapper can add 4 KiB mappings only. It cannot unmap, change permissions, or split huge pages. Those are M5.
- ~~The pool holds four tables.~~ Superseded by M4: a mapping that needs a new table fails with `OutOfFrames` only when physical memory is exhausted.
