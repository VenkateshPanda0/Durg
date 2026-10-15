# M1: Toolchain, build and first boot

**Goal:** a fresh clone plus one command boots a kernel under QEMU and prints a line on the serial port, and CI does the same.

This chapter explains how the pieces fit, and the bug we hit on the way.

## The moving parts

```
cargo xtask test
   │
   ├─ cargo build  ──►  durg-kernel (ELF, x86_64-unknown-none)
   ├─ git clone Limine (pinned tag) ──► bootloader files
   ├─ xorriso      ──►  durg.iso   (kernel + Limine + config)
   └─ QEMU + OVMF  ──►  boots the ISO, serial captured, exit code checked
```

- **Target `x86_64-unknown-none`.** A bare-metal target with no operating system underneath. rustup ships a prebuilt `core` for it, so stable Rust is enough (ADR-0002).
- **Linker script.** The kernel is linked at `0xffffffff80000000` in the higher half, which is where Limine expects a kernel linked at -2 GiB. Segments have explicit permissions, so no page is both writable and executable.
- **Limine.** A third-party bootloader (a tool, outside the project's originality core). It loads our ELF, switches the CPU to 64-bit mode, builds page tables that include a direct map of physical memory, and jumps to `_start`.
- **Limine protocol, our bindings.** The kernel embeds a tagged structure that tells Limine which protocol revision it expects. M1 needs only that tag (ADR-0003). The bootloader zeroes a word in it to say "supported".
- **UEFI via OVMF.** QEMU runs open-source UEFI firmware, which boots Limine from the ISO.
- **`isa-debug-exit`.** A QEMU device. Writing a byte to port `0xf4` makes QEMU exit with status `(byte << 1) | 1`. We use `0x10`, so a pass is status 33 and a failure (`0x11`) is 35. That is how a test tells pass from fail without a human.

## The bug we hit

The first boot printed nothing and the test timed out. Firmware output on the serial port showed Limine starting, then silence, which says nothing about why.

Enabling `serial: yes` and `verbose: yes` in the Limine config made it speak (the repository keeps `serial: yes` and now sets `verbose: no`, to keep logs short. Turn verbose back on first when a boot gives no output):

```
PANIC: elf: ELF file type is ET_DYN, but PT_DYNAMIC segment missing
```

Rust builds position-independent executables by default for this target. A PIE has ELF type `ET_DYN`, and Limine expects such a file to carry a dynamic section for relocations, which ours does not have. We link at a fixed address, so we do not want a PIE at all. The fix is to build with `relocation-model=static` and link with `--no-pie`. `readelf -h` then reports `EXEC`.

**Lesson:** when a boot gives no output, make the *bootloader* talk before suspecting the kernel. A hang with no output is the most expensive kind of bug in OS work, so turn on every log you can early.

## What this milestone does not do

- It does not initialise the UART. It writes bytes to COM1 and relies on QEMU. M2 adds a real 16550 driver.
- It does not read the memory map or any other boot information. M2 does.
- It has no security properties. M1 is plumbing.

## How to run it

```
rustup show                       # installs the pinned toolchain on first use
sudo apt install qemu-system-x86 xorriso ovmf
cargo xtask test                  # builds, boots headless, checks output and exit code
cargo xtask run                   # boots and shows serial on your terminal
```

Without KVM, QEMU falls back to software emulation. That is slower, and the boot test still takes a few seconds.
