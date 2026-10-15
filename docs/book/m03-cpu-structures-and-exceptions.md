# M3: CPU structures, exceptions, ACPI and the local APIC timer

**Goal:** the kernel owns its CPU setup. When something goes wrong, it prints what happened instead of the machine silently resetting.

This chapter covers descriptor tables, exception handling, the legacy PIC, reading the ACPI tables, a first slice of paging code, and the local APIC timer that delivers the kernel's first hardware interrupts.

## Why this comes first

Until now a bug in the kernel meant a **triple fault**: the CPU hits an exception, cannot find a handler, hits a second exception while trying, then a third, and gives up and resets. There is no message. Every later milestone (paging, scheduling, user mode) will crash in new ways, so the first job is to turn crashes into diagnostics.

## How x86-64 interrupt delivery works

An *exception* is the CPU reporting a problem with the instruction it just ran (divide by zero, bad memory access). An *interrupt* is an outside event. Both are delivered the same way:

1. The CPU assigns a **vector number**, 0 to 255. Vectors 0-31 are reserved for exceptions: 0 is divide error, 3 is breakpoint, 8 is double fault, 13 is general protection, 14 is page fault.
2. It looks up that vector in the **Interrupt Descriptor Table (IDT)**, a 256-entry table whose address is in the `IDTR` register. Each 16-byte entry (a *gate*) holds the handler's address, the code segment to run it in, and some flags.
3. It pushes a small **frame** onto the stack: the old `SS`, `RSP`, `RFLAGS`, `CS` and `RIP`. For some exceptions it also pushes an **error code** (page fault, general protection, double fault and a few more) that says why.
4. It jumps to the handler. The handler ends with `iretq`, which pops the frame and resumes the interrupted code.

Two details matter:

- **The handler runs with whatever stack the CPU had.** If the stack itself is the problem, delivery fails. That is what the Interrupt Stack Table is for (below).
- **Registers are the handler's problem.** The CPU saves only five values. If the handler is Rust code, we must save every other register first and restore it afterwards, or the interrupted program breaks.

### Entry stubs

Stable Rust has no special calling convention for interrupt handlers, so each of the 32 vectors has a tiny assembly stub. All 32 do the same thing: push a dummy error code if the CPU did not push a real one (so every frame looks identical), push the vector number, jump to common code that saves all registers, call `exception_dispatch` in Rust with a pointer to the saved state, restore, and `iretq`.

Which vectors push an error code is fixed by the architecture: 8, 10, 11, 12, 13, 14, 17, 21, 29 and 30. Getting that list wrong shifts every field in the frame by eight bytes, so a stub gets it wrong silently. The page fault test checks the error code and the faulting address, which would catch a shifted frame.

Alignment is a trap too. The System V calling convention wants the stack 16-byte aligned at a `call`. The CPU aligns before pushing, and we push 176 bytes in total (5-word frame, error code, vector, 15 registers), which is a multiple of 16, so alignment works out. The arithmetic is in a comment next to the assembly.

## The GDT and TSS

In 64-bit mode, segmentation is mostly switched off: base and limit are ignored. We still need a **Global Descriptor Table** for two reasons: the code segment descriptor is what says "this is 64-bit code, ring 0", and the **Task State Segment (TSS)** is found through it.

The TSS is no longer used for task switching. In 64-bit mode its useful part is the **Interrupt Stack Table**: seven stack pointers that individual IDT gates can request. We give the **double fault** gate its own 16 KiB stack (IST1).

Why: a double fault happens when the CPU fails to deliver an exception, often because the stack is broken (an overflow, a corrupt `RSP`). Delivering the double fault onto that same broken stack would fail again and cause the triple fault we are trying to avoid. With IST1, the CPU switches to a known-good stack before running the handler, so we can print a diagnostic.

### Tested, including a negative control

The `double-fault` boot variant sets `RSP` to an unmapped address and executes `int3`. The CPU cannot push the breakpoint frame, which raises a page fault. Delivering that page fault needs the same broken stack and fails too. Two page faults in a row become a double fault, which runs on IST1. The handler checks that its own frame address lies inside the IST1 stack range, prints the diagnostics, and exits QEMU with the success code.

To check the test can fail, we removed the IST entry from the double fault gate and ran it again. The result:

```
QEMU exit status: Some(0)
variant double-fault: FAIL (serial output did not contain "EXCEPTION: Double Fault (vector 8)")
```

The double fault escalated to a triple fault, QEMU shut down silently, and the variant failed. With the IST entry restored it passes. That is the difference the IST stack makes.

**What this does not test:** a real stack overflow. That needs a *guard page* (an unmapped page just below the stack) so the overflow faults cleanly, and we do not control page tables until M5. Setting `RSP` to a bad value exercises the same CPU path, but the genuine overflow test belongs to M5.

## The legacy PIC

Firmware leaves two old 8259 interrupt controllers mapped onto vectors 8-15, which collide with CPU exceptions (vector 8 is the double fault). A stray 8259 interrupt would look like an exception. We remap them to vectors 0x20-0x2f and mask every input. Modern interrupts use the local APIC and I/O APIC instead. A test reads both mask registers back and checks they are `0xff`.

## ACPI and the MADT

The **MADT** (Multiple APIC Description Table) tells the OS where the local APIC and I/O APICs are, how many CPUs there are, and which ISA interrupts are rewired. Getting there means walking: RSDP (a pointer found by the bootloader) to the XSDT or RSDT (a list of tables) to the MADT.

The parsing logic is in its own crate, `durg-acpi`, which compiles on the host. Its 17 unit tests build synthetic tables, including broken ones: wrong checksums, truncated entries, and a zero-length entry that would otherwise make the parser loop forever. The kernel side only fetches bytes, after checking that the memory really is ACPI memory the bootloader mapped.

On QEMU it reports: one CPU, local APIC at `0xfee00000`, one I/O APIC at `0xfec00000`, and five interrupt overrides (for example, ISA IRQ 0, the timer, is wired to GSI 2).

### Two spec surprises and a mistake

1. **ACPI memory is not mapped under base revision 3.** M2 found that. **Base revision 4** maps ACPI tables in the higher-half direct map, and returns the RSDP as a virtual address. So the kernel now asks for revision 4.
2. **The pinned bootloader did not support revision 4.** Limine v9.6.7 only goes up to revision 3. It loaded our kernel anyway, as the spec requires (a bootloader must fall back to the highest revision it knows), but ACPI was then unreadable. v10.8.5 supports revision 5, so we bumped the pin to that release. It is a one-line change to a build tool, and all variants pass with it.
3. **Our own mistake:** the first version treated the fallback as a fatal error. The spec's test is different: the bootloader overwrites the tag's second word with the revision it used, so if the magic number is still there, nobody processed the tag. A lower revision than requested is a legitimate fallback, not a failure. The kernel now only stops when the tag was untouched.

## Bugs hit

- A host test failed because our test helper computed the wrong checksum: the RSDP has two checksums covering different lengths (the first 20 bytes and all 36), and the helper summed the whole slice for both. The parser was right and the test was wrong.
- A clean-looking failure hid behind our own `grep` filter while debugging: the kernel's output shares a line with the bootloader's, so a filter on lines starting with `limine:` hid everything. Read raw output before trusting a filter.

## The local APIC and its timer

The local APIC is the interrupt controller built into each CPU. It receives interrupts, delivers them to its CPU, and contains a **timer**: a counter that counts down and raises an interrupt when it reaches zero, either once or repeatedly (*periodic* mode).

### The problem: its registers are not mapped

The xAPIC interface is a page of memory-mapped registers at a physical address, normally `0xfee00000`. The bootloader maps only regions in its memory map, and that page is in none of them. Reading it would page-fault.

Two ways out:

- **x2APIC** reads and writes the same registers as MSRs, needing no mapping. We checked: the CPU model we boot with reports no x2APIC, and QEMU's software emulation does not support it at all (`TCG doesn't support requested feature: CPUID.01H:ECX.x2apic`, even with `-cpu max`). CI could not test it.
- **Map the page ourselves**, which needs page-table code. That is M5 territory, but the timer is M3.

The project owner chose to build a small mapper now, as the real start of M5 and not as throwaway code (ADR-0006).

### A first slice of paging

Paging translates virtual addresses to physical ones through four levels of tables. A virtual address is split into four 9-bit indices (plus a 12-bit offset): the top bits pick an entry in the top table (PML4), which points at the next table, and so on down to a 4 KiB page. Larger pages end the walk early: a 1 GiB page at level 3, a 2 MiB page at level 2.

The pure logic is its own crate, `durg-paging`, with `#![forbid(unsafe_code)]`:

- **Entries:** a 64-bit value holding a physical address (bits 12 to 51) and flags (present, writable, cache-disable, no-execute, huge).
- **Splitting** a canonical address into its four indices.
- **A walker** that translates an address by asking a *reader closure* for each entry. In the kernel the reader goes through the higher-half direct map. In the host tests it reads a fake map of tables, so 11 unit tests check 4 KiB, 2 MiB and 1 GiB translation, a "not present" at every level, tables the reader cannot supply, non-canonical addresses, and malformed entries (the huge bit set on a PML4 entry, or a 2 MiB entry that is only 4 KiB aligned).

The kernel side, `mm::paging`, reads and writes the real tables and adds `map_page`. Missing intermediate tables come from a pool of four pages reserved in the kernel image, because there is no frame allocator yet (M4).

### Uncacheable memory

Device registers must not be cached: a read must reach the device, not return a stale copy. In a page table entry, the write-through and cache-disable bits together select PAT entry 3, and the bootloader guarantees that entry means "uncacheable". The kernel does not just trust that: a test reads the PAT register and checks entry 3.

### Interrupts end to end

With the page mapped, the kernel enables the controller (spurious-vector register), starts the timer in periodic mode, enables interrupts with `sti`, and halts. Each tick the CPU looks up our timer vector in the IDT, runs the stub and handler, which counts the tick and sends the *end of interrupt* signal, and resumes after `hlt`. `sti; hlt` is deliberately a pair: the `sti` takes effect only after the next instruction, so an interrupt that is already pending still wakes the `hlt`, and there is no window in which a wake-up can be lost.

The boot log shows:

```
LAPIC: id 0, version 0x50014, registers at physical 0xfee00000 mapped at 0xffffff0000000000 (uncached)
timer: 5 ticks counted from the local APIC periodic timer (initial count 1000000)
```

The timer is **counted, not timed**: its clock rate is not calibrated, so a tick is not a fixed number of milliseconds. Calibration against another clock is on the roadmap for the scheduler (M7).

### A negative control for the mapping

The mapping flags matter, but QEMU does not punish a mistake. We changed the mapping to cacheable and ran the tests. The kernel still booted and counted ticks, because QEMU does not emulate caches, but the self-test failed with `LAPIC page is not mapped uncached`. On real hardware that bug would show up as intermittent, unexplained device behaviour. Reading the page-table entry back and checking its flags is what catches it.

## What this milestone does not do

- No guard page, so no real stack-overflow test (M5). The double fault is tested by corrupting `RSP`.
- The mapper only adds 4 KiB mappings. It cannot unmap, change permissions, or split a huge page.
- It edits the bootloader's page tables in place. Nothing may reclaim that memory until M5 takes over.
- The timer is not calibrated, and the boot-time tick check would wait forever if no interrupt arrived.
- Only exceptions 0-31 and the timer and spurious vectors have IDT gates.
- No security properties yet.

## Try it

```
cargo test -p durg-acpi -p durg-paging   # host unit tests
cargo xtask test                         # four kernel variants in QEMU
cargo xtask run                          # boot once and read the output
```
