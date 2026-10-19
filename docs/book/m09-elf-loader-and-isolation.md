# M9: The ELF loader and isolated tasks

**Goal:** stop hand-assembling user programs. Load real, separately built programs from the boot image, run several of them, and show that one cannot read another's memory.

M8 gave us ring 3 and a syscall door, but the "programs" were byte blobs written in assembly inside the kernel's tests. M9 replaces them with ordinary Rust programs (`user/`), packaged as **Limine modules** and loaded by an **ELF loader**.

## Pieces

1. **Boot modules.** `boot/limine.conf` lists five `module_path` lines. Limine loads each file into memory and passes a list (module request, `kernel/src/limine.rs`). Each module's `address` is a virtual pointer in *Limine's* direct map; the physical address is that minus the HHDM offset. The module memory sits in the "kernel and modules" region, which our own page tables do not map in the direct map, so `mm/vm.rs` maps each module read-only and no-execute in a dedicated window (`MODULE_WINDOW`, PML4 slot 504). A test checks the window is not writable, executable or user-accessible.
2. **A hostile-input ELF parser** (`crates/elf`, host-tested and fuzzed). An ELF file is data chosen by someone else, so the parser fails closed. It rejects: segments that are both writable and executable, unaligned or overlapping segments, segments outside the policy's address range or outside the file, dynamic/interpreter/TLS/unknown segment types, an executable stack, and an entry point that is not inside executable, file-backed bytes. It returns at most 8 segments. `Policy` carries the allowed address range and a total-size cap.
3. **The loader** (`kernel/src/user.rs`). For each segment and page it takes a **zeroed** frame, copies the file's bytes (the part beyond `filesz` stays zero: that is bss), and maps the page with exactly its segment's permissions (user, write only if writable, no-execute unless executable). Then four stack pages, read/write, no-execute. On any failure every frame taken so far is returned.
4. **The programs** (`user/`): `init` (the root task; the kernel boots it and expects exit status 0), `task_a` and `task_b`, plus two helpers for the zeroing test, `dirty` and `probe`. The linker script puts every program's data at the **same virtual address** (0x600000), so the isolation test cannot be passed by accident of layout.

## The isolation test

`task_a` and `task_b` each hold a secret (`SECRET-OF-TASK-A/B`) at the same virtual address, and an 8 KiB scratch area. They run interleaved, yielding eight times, and each checks that its own secret and scratch are unchanged, and that the other task's secret is not visible in its own memory. Exit status: 0 ok, 1 own secret changed, 2 scratch changed, 3 the other's secret found.

Because a behavioural test can pass for the wrong reason, a second, structural check asserts that the two address spaces own **disjoint physical frames**, and that every path entry has the permission bits the loader intended.

## Zeroing

A frame given to a task may have held another task's data (or the bootloader's). `dirty` fills its bss and stack with junk and exits; `probe` counts non-zero bytes in its bss and lower stack pages. The test checks that every frame the probe was given previously held junk, then requires the count to be zero.

## Bugs and surprises

- **A weak residue test.** Mutation testing (the loader switched from `alloc_zeroed_frame` to `alloc_frame`) *survived*. Two causes: the linker placed `.got` after `.bss`, so the segment's file size equalled its memory size and the file itself supplied the zeros (the zero-fill path never ran); and the probe's pages did not necessarily reuse the dirty task's frames. Fixed by ordering `.got` before `.bss`, poisoning a block of free frames, and asserting both premises. The mutation is now caught.
- **The isolation program scanned an unmapped gap** between rodata and 0x600000 and faulted. It now scans the ranges given by linker symbols.
- **Test bait:** a first positive control for the "other task's secret found" check failed because the task overwrote the planted secret with its own scratch fill; the secret is planted at offset 0x100.
- **Release-only differences:** `init` has no data segment, and release builds may have no rodata, so the "every module parses" test is property-based over segment permissions instead of expecting a fixed layout.

## Not done (see ROADMAP "Later")

- The direct map still maps all RAM read-write, so every loaded user page has a writable **kernel** alias. Only kernel code can use it, but it is an alias an attacker who gains kernel write could use. Not addressed in M9.
- Programs are not relocated (fixed addresses, no PIE, no dynamic linking), and have no capabilities: the syscall surface is still debug print, yield and exit.
- The fuzz target covers the ELF *parser*, not the loader's page-building.
