# M4: The physical memory manager

**Goal:** the kernel can hand out and take back physical memory, one 4 KiB frame at a time, and can prove it never gives the same frame to two owners.

## The problem

Everything in an operating system eventually needs memory it can own: page tables, kernel stacks, buffers, user programs. Before any of that can exist, something has to know which parts of physical RAM are free, and hand them out without ever handing out the same part twice. A mistake here is the worst kind of bug: two owners silently share memory, and corruption shows up far from its cause.

## What counts as free

The bootloader's memory map classifies every physical range. The allocator manages only:

- **Usable** RAM, and only **above 1 MiB**.

It deliberately leaves out:

- **Bootloader-reclaimable** memory (about 42.6 MiB on QEMU with 256 MiB, roughly 17% of RAM). It holds the page tables the CPU is using right now, the memory map itself, and the boot stack. It can join the pool once the kernel has taken over the page tables and copied out what it needs (M5). Until then it is unavailable, and that is a real cost, not an oversight.
- **Kernel and modules**, the framebuffer, ACPI, and reserved ranges.
- **The first MiB.** It holds firmware and BIOS data, and it is where the real-mode trampoline for starting other CPUs traditionally lives. This is a policy choice that is easy to relax later.

## Frames and the bitmap

A *frame* is one 4 KiB page of physical memory; its number is its address divided by 4096. The allocator keeps one bit per frame.

### Three states, two bitmaps

A single "free or used" bit has a dangerous flaw. If `free()` just sets a bit, then freeing a frame that holds the kernel, or a frame that was never ours, quietly puts it up for allocation. The next caller gets memory that is in use. So the allocator keeps two bitmaps:

| `managed` | `free` | meaning |
|---|---|---|
| 0 | 0 | **not ours**: kernel, firmware, reserved, below 1 MiB. Never handed out, never accepted back. |
| 1 | 1 | **free**: in the pool and available |
| 1 | 0 | **allocated**: in the pool and handed out |

`free()` refuses anything that is not "managed and allocated", and says which way it was wrong: out of range, never owned, or already free (a double free). A refused free changes nothing.

### Invariants

The crate documents five, and checks them all after every operation in its tests:

1. Every free frame is managed.
2. `free_count` equals the number of set bits in the free bitmap.
3. `managed_count` equals the number of set bits in the managed bitmap.
4. `alloc` returns only frames that were free, and each at most once until freed.
5. No bit at or beyond the last frame is ever set.

## Where the bitmaps live

The allocator needs memory for its own bitmaps before any allocator exists. The kernel finds a usable region above 1 MiB with room, puts them there (reached through the higher-half direct map), and then takes those frames back out of the pool.

**They cover only up to the highest usable address.** The memory map includes reserved ranges near 1 TiB. A bitmap spanning those would be 32 MiB, to describe memory that can never be allocated. Covering only usable addresses makes the bitmaps 15 KiB.

## Two halves: pure logic and kernel glue

As with ACPI and paging, the logic is a separate crate, `durg-pmm`, with `#![forbid(unsafe_code)]`. It works on a slice of `u64` that the caller provides, so the host tests give it a `Vec`. The kernel module `mm::frame` decides what to manage, finds the memory for the bitmaps, and zeroes frames when asked.

## How it was tested

### On the host: model-based property tests

A property test with `proptest` generates random sequences of up to 300 operations (add a range, reserve a range, allocate, free a live frame, free an arbitrary frame number, including bogus ones) on pools of sizes chosen to straddle the 64-frame word boundaries. After every step it compares the allocator against a plain `BTreeSet` model and checks all invariants. It ran 512 cases. At the end it returns everything and checks the pool comes back whole.

A property test that has never failed might be testing nothing, so we broke the allocator on purpose in four ways. Every mutant was caught:

| Mutation | Tests failing |
|---|---|
| `alloc` forgets to clear the free bit (double allocation) | 7 of 13 |
| `free` accepts a double free | 2 |
| `free` accepts frames the allocator never owned | 3 |
| `add_range` off by one at the end | 2 |

### In QEMU: against real memory

Eight self-tests run against the real memory map:

- **Totals match an independent calculation.** The test recomputes, straight from the map, how many frames should be managed, sharing no code with the allocator.
- **Only usable, distinct, aligned frames.** Every frame handed out is above 1 MiB, inside a usable region, outside the bitmaps, and different from all others.
- **Double frees and foreign frames are rejected:** the same frame twice, frame 0, a kernel frame, the allocator's own bitmap frame, an out-of-range address, an unaligned one. A refused free changes nothing.
- **Zeroed frames are zero.** The test dirties a batch, frees them, then allocates zeroed ones and requires some to be reused, so zeroing is really exercised.
- **Stress with unique patterns.** 20,000 random allocate/free operations over 256 slots. Each live frame holds a pattern derived from its own address in three places. If two allocations ever overlapped, one owner would overwrite the other's pattern and the check at free time would catch it.
- **Exhaustion and recovery.** It takes every free frame (about 52,700), threads them into a list stored inside the frames themselves, checks that the next allocation fails cleanly, and returns them all.
- **Page tables come from the allocator.** Mapping a page that needs one new table consumes exactly one frame.

We also injected two bugs into the real kernel. With the zeroing removed, `pmm_zeroed_frames_are_zero` failed with `frame 0x200000 was not zeroed at offset 0x0`. With the allocator forgetting to mark frames used, the first test to fail was not an allocator test at all: the same frame went to the page-table code twice and corrupted the local APIC mapping, so the APIC self-test failed. That is what a duplicate allocation looks like in a real kernel.

## Replacing the stopgap

M3 gave the page mapper a pool of four page-table pages baked into the kernel image, because there was no allocator (ADR-0006). That pool is gone. New page tables now come from `alloc_zeroed_frame`. Zeroing matters: stale bytes in a page table would be read as present entries.

## What this milestone does not do

- **No locking.** The kernel is single-CPU and interrupt handlers do not allocate. A re-entrancy guard turns accidental nested use into a panic. M7 adds real locks.
- **No contiguous allocation.** Some devices will need physically contiguous buffers (M11). It is not built yet.
- **Search is a linear scan** of the bitmap from a moving hint. That is fine at 53,000 frames. A buddy allocator or free lists would be faster for large memory, and nothing needs that yet.
- **No NUMA zones or huge frames.**
- **About 42.6 MiB of reclaimable memory is unused** until M5.
- **The allocator trusts the memory map.** A bootloader bug that marked in-use memory as usable would not be caught.

## Try it

```
cargo test -p durg-pmm        # unit and property tests on the host
cargo xtask test              # four kernel variants in QEMU
```
