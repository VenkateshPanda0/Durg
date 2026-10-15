# M5: The kernel owns its address space

**Goal:** the kernel stops living on page tables someone else built. It builds its own, with W^X, only the memory it needs, and a kernel stack that cannot silently overflow. Then it gives the bootloader's memory back.

## Why take over

From the first instruction the CPU runs on page tables the bootloader built. They work, but:

- Their layout is unspecified, they live in memory the kernel wants back, and the kernel cannot change their permissions on its own terms.
- They map the kernel image **twice**: at its own address and again through the higher-half direct map (HHDM), where it is writable. Any bug that writes through the direct map can modify kernel code.
- They map the framebuffer and more than the kernel uses.
- The boot stack has no guard page below it.

## W^X: no page is both writable and executable

If a page is writable and executable, an attacker who can write bytes there can run them. The rule "write XOR execute" removes that. The page table has two relevant bits: **writable** and **no-execute (NX)**. NX only works if the CPU has it switched on (`EFER.NXE`), which the bootloader does, and the kernel checks before setting the bit, because setting NX without it is a reserved-bit fault. The **WP** bit in `CR0` makes the kernel itself obey read-only pages (without it, ring-0 code ignores them).

The linker script now exports where each section starts and ends, and each gets its own permissions:

| Section | Readable | Writable | Executable |
|---|---|---|---|
| text | yes | no | **yes** |
| rodata | yes | no | no |
| data + bss | yes | **yes** | no |

The kernel image is also **not mapped in the direct map at all**, so its code has no writable alias.

### Tested, not assumed

- Every page of the image is checked: none is writable and executable.
- Writing a byte into read-only data really faults, with error code "present page, write" (a protection violation), and the data is unchanged.
- Jumping into a data page really faults, with "present page, instruction fetch" (the NX bit at work).
- Looking up `direct-map address + physical address of each section` finds nothing: no alias.
- The framebuffer and reserved regions are not mapped.

## Taking over safely

The scariest line in the kernel is the one that loads `CR3`. If the new tables miss the code that is running, the stack, the descriptor tables, or anything the kernel touches next, the CPU faults on the very next instruction, the fault handler is also unmapped, and the machine resets with no output.

So the kernel verifies the new address space **before** switching:

1. Every page of the kernel image must translate to the same physical frame under the old and the new tables.
2. The stack the kernel is running on right now, and the memory just below the stack pointer, must be mapped.
3. The first and last page of every direct-map region the kernel will read must be present and correct.

If any check fails, the kernel keeps the bootloader's tables and reports exactly which address and why. On the first real boot, all checks passed and the switch worked.

## A stack with a guard page

The kernel stack is 16 pages (64 KiB) in its own virtual region, with one **unmapped** page directly below it. Without that, an overflow would silently write into whatever sits below the stack. With it:

1. The stack grows into the guard page. The next access is a **page fault**.
2. The CPU must push an exception frame to handle it, but the stack pointer is in the guard page, so that push faults too.
3. A page fault while delivering a page fault becomes a **double fault**, which uses the separate IST1 stack from M3, so it can run.
4. The handler sees that `CR2` (the faulting address) and `RSP` lie in the guard page and reports `kernel stack overflow detected`.

A real recursion overflows the stack in a dedicated boot variant, and the log shows:

```
kernel stack overflow detected: CR2 0xfffffe0000000e58, RSP 0xfffffe0000000e60, guard page 0xfffffe0000000000..0xfffffe0000001000
double fault handled on the IST stack
```

This is the genuine version of the double-fault-from-stack-overflow test that M3 could only approximate by corrupting `RSP`.

### Two-stage boot

The kernel cannot switch stacks in the middle of a function and keep going, so boot is split. Stage 1 runs on the bootloader's stack and tables and ends by switching `RSP` to the new stack and calling stage 2, which never returns to stage 1.

## The TLB

The CPU caches page table translations in the **TLB**. If you change a page table entry and do not tell the CPU, it keeps using the old translation. After unmapping a page, `invlpg` drops that one entry.

We checked it matters, not just that it is there. A test uses a page (so its translation is cached), unmaps it, and requires the next access to fault. With `invlpg` removed from `unmap`, that test fails: the stale entry survives and the unmapped page stays readable. QEMU's software TLB behaves like a real one in this respect.

## Mapping, unmapping and huge pages

An `AddressSpace` is a root table plus operations: translate, map a 4 KiB page, map a 2 MiB page, map a range (using 2 MiB pages where both addresses are aligned, 4 KiB pages elsewhere), and unmap. Any root works, not only the active one, which is what lets the kernel build its tables first and switch later.

Unmapping refuses to split a huge page: asking to unmap a 4 KiB piece in the middle of a 2 MiB page returns an error and changes nothing, and a test checks that. Unmapping does not free page-table frames, even when a table becomes empty. That is a known limitation.

## Giving the bootloader's memory back

About 41 MiB of RAM on QEMU with 256 MiB (roughly 16%) was marked bootloader-reclaimable and sat unused since M4. It holds the bootloader's page tables, boot stack and response structures. Reusing it is only safe when none of those are still needed:

- **Page tables:** the kernel is on its own. (Checked: the reclaim function refuses to run before the switch.)
- **Stack:** the kernel is on its own.
- **Response structures:** the memory map, HHDM offset and RSDP address were **copied** into kernel memory at the very start of boot, and everything reads the copy. Nothing reads bootloader memory afterwards.

The result: `PMM: reclaimed 10578 frames (41 MiB) of bootloader memory`. An independent test recomputes the expected total from the memory map and checks the allocator matches, and the exhaustion test from M4 writes into every frame including the reclaimed ones, which also proves they are mapped and writable in the new tables.

**A caution:** reclaimed frames still contain old bootloader data, and `alloc_frame` does not clear them. Anything given to an untrusted task must come from `alloc_zeroed_frame`.

## Negative controls

Each claim above was checked by breaking it on purpose:

| Break | Result |
|---|---|
| Map kernel text writable | `page 0xffffffff80000000 in text is writable and executable` |
| Map the guard page and the whole stack | `the guard page is mapped`, and the overflow is no longer attributed to the guard page, so the stack-overflow variant fails |
| Remove `invlpg` from `unmap` | `paging_unmap_takes_effect_despite_the_tlb` fails: the stale entry survived |

One control was badly designed on the first try: it left the top stack page unmapped, so the kernel crashed right after switching stacks. That failed loudly but tested the wrong thing, so it was redone to map the guard page *and* the full stack.

## What this milestone does not do

- **No user mode, so no isolation from an untrusted task is tested.** SMEP, SMAP and UMIP (which stop the kernel from executing or touching user memory) are not enabled yet, because nothing needs them until M8.
- **The direct map is writable for all RAM.** Memory that later holds a loaded program will have a writable kernel alias. That is fine while only trusted code runs, but the loader (M9) must account for it.
- **Page-table frames are never freed,** and permissions of an existing mapping cannot be changed. To change them, unmap and map again.
- **No address randomisation.** The kernel is at a fixed address.
- **No locking.** Single CPU, and interrupt handlers do not edit page tables.

## Try it

```
cargo xtask test      # five kernel variants, including the stack overflow
cargo xtask run       # boot once and read the VM lines
```
