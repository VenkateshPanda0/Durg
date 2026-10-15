# M3: CPU structures, exceptions and ACPI (part 1)

**Goal:** the kernel owns its CPU setup. When something goes wrong, it prints what happened instead of the machine silently resetting.

This chapter covers the first half of M3: descriptor tables, exception handling, the legacy PIC, and reading the ACPI tables. The local APIC timer is the second half and is waiting on a design decision (see the end).

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

## What this part does not do

- No timer interrupt yet, and interrupts remain disabled.
- No guard page, so no real stack-overflow test (M5).
- Only exceptions 0-31 have gates. Other vectors are not present, so using one raises "segment not present".
- No security properties yet.

## The open question: the local APIC timer

The timer lives in the local APIC. Two ways to reach it:

- **xAPIC:** registers in memory at physical `0xfee00000`. That address is not in the bootloader's memory map, so it is **not mapped**. Reaching it needs our own page-table code.
- **x2APIC:** registers through MSRs, with no memory mapping needed. We checked: the CPU model we boot with reports **no x2APIC**, and QEMU's software emulation **does not support it at all** (`TCG doesn't support requested feature: CPUID.01H:ECX.x2apic`). It would work under KVM but not in CI.

So the timer needs a way to map one MMIO page. That is part of M5 (paging), which is later in the order. The choice of how to proceed is for the project owner and is recorded in the devlog.

## Try it

```
cargo test -p durg-acpi        # host unit tests for the ACPI parser
cargo xtask test               # four kernel variants in QEMU
cargo xtask run                # boot once and read the output
```
