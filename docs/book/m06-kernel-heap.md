# M6: The kernel heap

**Goal:** kernel code can use `Box`, `Vec`, `String` and `BTreeMap`. The allocator behind them is our own, it grows when it runs dry, and running out of memory is a handled event, not a crash into the unknown.

## Why a heap at all

Until now the kernel used fixed-size statics and the stack. That works for a fixed set of structures, but a scheduler, a capability table or a message queue grows with the workload. Rust's `alloc` crate provides the collections, and needs one thing from us: a **global allocator**, a type implementing `GlobalAlloc` that hands out and takes back blocks of bytes.

Where it fits: the physical frame allocator (M4) hands out whole 4 KiB frames. The heap sits on top and slices pages into small pieces.

```
Vec / Box / String / BTreeMap        (alloc crate)
        |
KernelAllocator (GlobalAlloc)        kernel/src/mm/heap.rs
        |   grows by mapping pages
durg-heap  (free list, host-tested)  crates/heap
        |
frame allocator + page tables        M4, M5
```

## The algorithm: a free list

The heap keeps a singly linked list of **free blocks**, sorted by address. The list node lives *inside* the free block it describes (16 bytes: size and next pointer), so bookkeeping costs no extra memory, and a block's header vanishes when it is handed out.

- **Allocate** (*first fit*): walk the list to the first block that fits at the requested alignment, cut the request out, and put any leftover in front of or behind it back on the list.
- **Free:** insert the block at its sorted position and **merge** it with the neighbour before and after if they touch (*coalescing*). Without merging, a heap breaks into crumbs too small to use even though plenty of memory is free.

Every size and address is a multiple of 16. A free block must hold its own 16-byte node, so rounding to 16 guarantees a leftover is either zero or big enough to be a block. The price: a 1-byte request costs 16 bytes.

### Alternatives rejected

| Option | Why not (for now) |
|---|---|
| Bump allocator | Cannot free. Fine for boot, not for a running kernel. |
| Buddy allocator | Better worst-case fragmentation and speed, but rounds to powers of two and is more code. Revisit if the free list shows problems. |
| Slab / size classes | Fast for many same-sized objects. Worth it when we know which objects dominate (capability nodes, thread control blocks). |
| The `linked_list_allocator` crate | Forbidden in guest code by the originality policy, and the point of the project is writing it. |

The free list is O(number of free blocks) per operation. That is fine at this size and is visible in the `free_blocks` counter, so it will be obvious if it stops being fine.

## Detecting misuse

Freeing a block that overlaps free memory (a double free, a never-allocated pointer, a wrong size that reaches into a free block) is detected and refused. The global allocator turns that into a panic, because it means memory is corrupted or a bug exists, and continuing would hide it.

**Not detected:** freeing a pointer into the middle of a live allocation, or freeing with a smaller size than was allocated. Neither overlaps free memory. They need a bug elsewhere; the `GlobalAlloc` contract always hands back the original layout.

## Growing the heap

The heap owns a virtual window (`0xffff_fd00_0000_0000`, PML4 slot 506). At boot 512 KiB is mapped. When an allocation cannot be served:

1. Work out how much is needed, rounded up to pages, at least 256 KiB.
2. Refuse if the heap would pass **64 MiB**, or if fewer than **64 frames** would remain in the frame allocator. The reserve keeps page-table allocation and other frame users alive.
3. Take zeroed frames, map them read/write and **no-execute** at the end of the mapped part. If any step fails, unmap and return everything this attempt took.
4. Give the new region to the free list. It merges with a free block at the end of the old region.

The heap never shrinks: freed memory goes back to the free list, not to the frame allocator.

## Running out of memory

`GlobalAlloc::alloc` returns null when growth fails. What happens next is Rust's, not ours:

- **Fallible APIs** (`Vec::try_reserve`) return an error. The caller can cope. Tested: a 1 GiB request is refused, the heap's counters and free list are unchanged, and the next allocation works.
- **Infallible APIs** (`Vec::push`, `Box::new`, `Vec::with_capacity`) call the allocation-failure handler, which panics with `memory allocation of N bytes failed`. Our panic handler prints it and exits QEMU with the failure code. This is the `heap-oom` boot variant: it passes only if the exit status is *failure* and that message appears.

This matters for the thesis. Kernel code that must not die on a hostile request needs the fallible APIs, and **the kernel must never allocate on behalf of an untrusted task from this heap** (CLAUDE.md memory policy: tasks retype untyped memory they hold). M10 turns that into the design rule: the kernel heap holds the kernel's own bookkeeping only, and anything a task can make grow is paid for from the task's own untyped memory.

## Concurrency

None yet. One CPU, and interrupt handlers do not allocate. A re-entrancy flag turns accidental nested use into a panic. M7 adds a real lock before another thread can allocate.

## Tests

- **Host (`durg-heap`):** 12 tests including a model-based property test that performs random allocate/free sequences on a buffer, checks no two live blocks overlap, contents survive, the free-list invariants hold after every step, and freeing everything merges back to one block.
- **In QEMU (7 new self-tests, 52 total):** collections work; alignment from 1 to 4096; random alloc/free stress with a content pattern per block and a no-leak check; growth on demand; heap pages are read/write and not executable, and the page after the heap is unmapped; graceful refusal of an impossible request; checked detection of double and wild frees (on a private heap, because the global allocator panics on them).
- **Boot variant `heap-oom`:** the infallible out-of-memory path.

### Mutation tests and negative controls

Breaking the code on purpose, to see whether the tests notice:

| Break | Caught by |
|---|---|
| Heap crate: skip merging with the next block / the previous block / accept a double free / ignore the previous-block overlap / off-by-one in fit / ignore alignment / drop the front leftover / drop the back leftover | host tests (all 8 caught) |
| Kernel: `dealloc` does nothing | `heap_random_alloc_free_stress_keeps_contents_intact` (leak) |
| Kernel: map heap pages executable | `heap_pages_are_writable_and_not_executable` |
| Kernel: ignore the requested alignment | `heap_alignment_up_to_a_page_is_honoured` |

One test was too weak at first. With "ignore alignment" the alignment test still passed, because the heap's first block starts on a page boundary and every request was aligned by luck. The fix keeps a small block alive first, so the free memory starts at an address that is only 16-byte aligned. Only then did the break get caught. Lesson: a test that passes under the break is a test of nothing.

## What this milestone does not do

- **No lock.** Single CPU only.
- **The growth failure paths are tested only partly.** The size limit and the refusal of a huge request are tested. The "too few frames left" and "mapping failed, roll back" branches are written but not triggered by any test.
- **Allocated memory is not cleared** and freed memory is not poisoned. Anything a task can see must still come from zeroed frames.
- **No shrinking, no per-object caches, O(n) free list.**
- **No protection from overflow of one block into the next** (no red zones). Rust's checks cover safe code; `unsafe` code is on its own.

## Try it

```
cargo test -p durg-heap    # host unit and property tests
cargo xtask test           # six kernel variants, including heap-oom
cargo xtask run            # boot once and read the heap: lines
```
