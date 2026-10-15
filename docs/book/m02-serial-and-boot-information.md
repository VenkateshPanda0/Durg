# M2: Serial console and boot information

**Goal:** the kernel talks. It prints the memory map, and a failing test exits QEMU with a failure code.

## Why serial first

Early in OS development there is no screen, no debugger you trust, and no filesystem. A serial port is the simplest output a CPU can drive: write a byte to an I/O port, and a character appears on the other side. QEMU connects that port to your terminal, which also lets a test read the output. Everything you debug in the next ten milestones goes through this port, so it comes before everything else.

## The 16550 UART

COM1 is a chip (the 16550, emulated by QEMU) at I/O port `0x3f8`. We program it in a fixed sequence:

1. Turn its interrupts off. We poll instead.
2. Set the DLAB bit, write the baud divisor (1 gives 115200 baud), then clear DLAB with the line settings (8 data bits, no parity, 1 stop bit). The same two ports (offsets 0 and 1) mean different things depending on DLAB.
3. Enable and clear the FIFOs.
4. **Loopback self-test.** Put the chip in loopback mode, write a byte, and read it back. If it comes back, the transmitter and receiver registers work, and the test costs a few instructions.

To send a byte, wait until the "transmit holding register empty" bit in the line status register is set, then write the byte. The wait is bounded, so a missing UART cannot hang the kernel.

There is **no locking**, because there is one CPU, no interrupts and no threads. That stops being true in M7, which must add interrupt-safe locking before a second writer can exist.

## Reading what the bootloader found

Limine hands over information through *requests*: tagged structures embedded in the kernel image. The bootloader scans the image, fills in a response pointer for each request it understands, and jumps to the entry point. We ask for three things:

- **Memory map:** which physical ranges are RAM, firmware, ACPI, framebuffer, and so on.
- **HHDM offset:** the bootloader maps physical memory at a fixed offset in the higher half, so physical address `p` is readable at `p + offset`.
- **RSDP:** the physical address of the ACPI root table, which later milestones parse to find interrupt controllers.

The request IDs and struct layouts are copied from the protocol document and checked against the `limine.h` that ships with the bootloader release we pin. They are not typed from memory, because a wrong magic number fails silently: the bootloader simply never answers.

### Two things the specification told us

Reading the spec before writing code changed the design:

1. **The direct map is restrictive in base revision 3.** Only usable, bootloader-reclaimable, kernel/module and framebuffer regions are mapped. ACPI tables are **not**. Our first idea was to verify the RSDP by reading its `RSD PTR ` signature through the direct map. That would have page-faulted, and we have no fault handler until M3. So M2 records the RSDP's physical address and does not touch it.
2. **The memory map has guarantees:** entries are sorted by base address, and usable and bootloader-reclaimable entries are 4 KiB aligned and overlap nothing. We turned those sentences into tests, so a bootloader or QEMU change that breaks them is caught.

## What the memory map shows

Under QEMU with 256 MiB of RAM, the map has 31 entries and about 207 MiB of usable RAM. The rest is the interesting part:

- Several small `bootloader reclaimable` and `acpi nvs` regions in the first megabytes, left by the firmware.
- `kernel and modules`, a few tens of KiB: our own image.
- `framebuffer`: the firmware's graphics buffer. Graphics is a non-goal, so we ignore it, but it still takes address space.
- Large `reserved` ranges high in the address space (PCI hole, firmware flash, and others).

Notice that usable RAM is not one block. It is broken up by firmware regions. The physical memory manager (M4) has to work from these fragments, and `bootloader reclaimable` memory can only be reused after we copy out what we still need from it.

## Testing without the nightly test framework

Rust's built-in test harness does not work on a bare-metal target with stable Rust, and ADR-0002 keeps us on stable. So we wrote a tiny one: a list of `fn()` values that panic on failure. A feature flag, `selftest`, makes the kernel run them after boot.

The panic handler prints the message and source location, then writes the failure code to QEMU's debug-exit device. The host sees both the text and a non-zero exit status. A test that hangs is caught by a host-side timeout.

**A test harness that cannot fail proves nothing.** So a second feature, `fail-demo`, adds a test that always panics, and `cargo xtask test` boots that variant too. It passes only if the kernel reports the failure: exit status 35, and the panic text in the serial output.

`cargo xtask test` therefore boots three kernels:

| Variant | Expected exit status | Must print |
|---|---|---|
| default | 33 (success) | boot banner, memory map, HHDM offset, RSDP |
| `selftest` | 33 | `selftest: all 7 tests passed` |
| `fail-demo` | 35 (failure) | `PANIC:` and the deliberate failure message |

## What this milestone does not do

- No interrupt or exception handling. A page fault today is a triple fault. M3 fixes that.
- No memory management. We read the map and do nothing with it.
- Serial output is unlocked and polled.
- The UART driver is verified against QEMU's model of the chip only.
- No security properties. This is still plumbing.

## Try it

```
cargo xtask test          # three variants, headless
cargo xtask run           # boot once and read the memory map in your terminal
```
