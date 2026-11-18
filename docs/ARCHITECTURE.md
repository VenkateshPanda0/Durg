# Durg architecture (as built)

This page describes the system **as it exists in the code today**, not as it is
planned. For the plan see [ROADMAP.md](ROADMAP.md); for the ordered list of
concrete next tasks see [TODO.md](TODO.md); for why things are the way they are,
see the [ADRs](adr/README.md) and the [book chapters](book/). When this page and
the code disagree, the code is right and this page is a bug.

Last checked against the code: 2026-11-18 (commit after `bb8b6d5`).

## 1. The big picture

```text
Host machine (Linux or WSL2)
 └─ cargo xtask ─ builds everything, packs a Limine ISO, boots QEMU (KVM if available)
                                   │
QEMU x86-64 VM ────────────────────┘
 ├─ Limine bootloader (third party, pinned v10.8.5) loads the kernel and the boot modules
 ├─ Kernel, ring 0 (kernel/): one CPU
 │    traps and interrupts · physical and virtual memory · heap · threads and
 │    preemptive scheduler · system calls · ELF loader · capability spaces ·
 │    endpoint IPC · notifications · interim spawner
 └─ User space, ring 3 (user/): every program in its own address space
      init (the root task) ── builds the rest of the system from its capabilities
      ipc_server · ipc_client · ipc_stranger · capdemo · crasher · …  (test programs)
```

What is **not** there yet (see the roadmap): device drivers, storage, the effect
journal, provenance labels, the approval gate, the host-side broker and
controller, a shell, SMP, real hardware. No security promise is delivered end to
end; [THREAT_MODEL.md](THREAT_MODEL.md) records what the tests do and do not show.

## 2. Repository layout

| Path | What it is | Builds for |
|---|---|---|
| `kernel/` | The kernel crate (`durg-kernel`), `no_std`, `no_main`. See [kernel/README.md](../kernel/README.md). | `x86_64-unknown-none` |
| `crates/` | Pure logic the kernel uses, factored out so it can be tested and fuzzed on the host. No `unsafe` in any of them. See [crates/README.md](../crates/README.md). | host and bare metal |
| `user/` | The ring-3 runtime (`durg-user`) and the boot programs. See [user/README.md](../user/README.md). | `x86_64-unknown-none` |
| `xtask/` | The build tool: `cargo xtask build \| iso \| run \| test [--release]`. | host |
| `boot/limine.conf` | Boot menu and the list of boot modules (the order is the module numbering). | n/a |
| `fuzz/` | libFuzzer targets for the pure crates; its own workspace, needs nightly (ADR-0008). | host |
| `scripts/setup.sh` | One-command setup and boot test for Debian/Ubuntu/WSL. | n/a |
| `.github/` | CI workflow, issue and pull-request templates. | n/a |
| `docs/` | Everything written: this page, roadmap, TODO, threat model, ADRs, book, devlog. See [docs/README.md](README.md). | n/a |
| `rust-toolchain.toml` | Pins Rust 1.97.0 with the bare-metal target, rustfmt and clippy (ADR-0002). | n/a |
| `.cargo/config.toml` | The `xtask` alias and static (non-PIE) linking for the kernel. | n/a |

## 3. How a build becomes a boot

1. `cargo xtask build` builds `durg-kernel` for `x86_64-unknown-none` (debug, or
   optimized with `--release`), linked at `0xffffffff80000000` by
   `kernel/linker.ld`.
2. `cargo xtask iso` also builds every program in `user/` (linked at `0x400000`
   by `user/user.ld`), clones the pinned Limine binary release (`v10.8.5-binary`)
   once into `target/limine/` (this needs network access the first time),
   copies the kernel to `/boot/durg-kernel` and the programs to
   `/boot/user/*.elf`, and builds a bootable ISO with `xorriso`.
3. `cargo xtask run` boots that ISO in QEMU (UEFI firmware from OVMF, serial on
   stdio, no display; KVM when `/dev/kvm` is usable, software emulation with
   SMEP, SMAP and UMIP enabled otherwise) and stops when the kernel exits
   through the `isa-debug-exit` device.
4. `cargo xtask test` builds and boots **six variants** and checks each one's
   serial output and exit code:

   | Variant | Kernel features | Expected |
   |---|---|---|
   | `default` | none | a normal boot: the serial log shows every boot stage (serial, CPU tables, memory map, frame allocator, page tables, timer ticks) and the root task's run, and QEMU exits with success |
   | `selftest` | `selftest` | every in-kernel self-test passes (`selftest: all N tests passed`) |
   | `double-fault` | `df-test` | a deliberate double fault is handled on its own stack |
   | `stack-overflow` | `so-test` | a kernel stack overflow hits the guard page and is reported |
   | `heap-oom` | `oom-test` | heap exhaustion ends in the allocation-failure panic (a *failure* exit, on purpose) |
   | `fail-demo` | `fail-demo` | a test that always fails makes the run fail (proves the harness can fail) |

## 4. Boot sequence (kernel/src/main.rs)

**Stage 1, `_start`, on the bootloader's stack and page tables:**
serial console (our own 16550 driver) → check that a Limine-protocol bootloader
ran and copy its responses (memory map, HHDM offset, RSDP, modules) → load our
GDT, TSS (double fault on IST1) and IDT, mask the legacy PIC → print boot
information → build the physical frame allocator → build the kernel's own page
tables, verify them against the live ones, switch → move to a kernel stack with
a guard page.

**Stage 2, `kernel_main`, on the kernel's own stack and tables:**
reclaim bootloader memory → a map/unmap self-check → kernel heap → local APIC
(found through the ACPI MADT) → a bounded timer-tick check → scheduler →
user mode (SMEP, SMAP and UMIP when the CPU has them; `syscall`/`sysret` MSRs;
map the boot modules; start the root task and wait for it) → depending on the
build features, run the self-tests or exit QEMU with success or failure.

## 5. Kernel subsystems

| File | Responsibility |
|---|---|
| `main.rs` | The two boot stages above, the panic handler, the per-variant ending |
| `boot.rs`, `console.rs` | Printing boot information; `print!`/`println!` and log macros over serial |
| `limine.rs` | Our own Limine protocol bindings (ADR-0003): requests, responses, captured copies, modules |
| `acpi.rs` | Finds the MADT through the RSDP, using the pure `durg-acpi` parser |
| `qemu.rs` | Exit codes through `isa-debug-exit` (33 success, 35 failure as QEMU reports them) |
| `arch/serial.rs` | 16550 UART on COM1 |
| `arch/x86_64/` | `gdt` (segments, TSS), `idt` and `trap` (exception and interrupt handlers, ring-3 fault handling), `pic`, `lapic` (timer), `context` (thread switch stub), `syscall` (entry stub, MSRs), `protection` (SMEP/SMAP/UMIP), `mod` (CPU helpers, `IrqLock`) |
| `mm/frame.rs` | Physical frame allocator (bitmap from `durg-pmm`), zeroed frames, aligned block reservation for untyped memory |
| `mm/paging.rs` | Page tables (`durg-paging`), per-task address spaces sharing the kernel half |
| `mm/vm.rs` | The kernel's own address space: W^X image, direct map of usable RAM only, guarded stacks, module window |
| `mm/heap.rs` | Kernel heap (`durg-heap`) behind `GlobalAlloc`, grows on demand up to 64 MiB |
| `sched.rs` | Threads, round-robin preemptive scheduling on the LAPIC timer, blocking, endpoint IPC, notifications, `join`, exit history |
| `syscall.rs` | System-call handlers: decode, check, act; never dereferences a user pointer |
| `cspace.rs` | Capability spaces in physical memory, `invoke`, untyped memory blocks, teardown |
| `user.rs` | ELF loading into a fresh address space, the root task's start, the boot-program table |
| `selftest.rs` | In-kernel tests (feature `selftest`), including ring-3 test programs |

**Lock order** (outermost first; never take an earlier lock while holding a
later one): scheduler → capability state → frame allocator, and scheduler →
heap → page tables → frame allocator. The console lock is a leaf. All locks
are interrupt-safe (`IrqLock`) and correct for one CPU only.

## 6. Memory layout

Kernel half (PML4 slots 256–511, shared by every address space):

| PML4 slot | Base address | Contents |
|---|---|---|
| from 256 | Limine's HHDM offset | Direct map: usable RAM (read/write, no-execute), bootloader memory until reclaimed, ACPI tables read-only. The kernel image is **not** in it. |
| 504 | `0xffff_fc00_0000_0000` | Boot-module window: each module read-only, no-execute, 16 MiB apart |
| 506 | `0xffff_fd00_0000_0000` | Kernel heap (512 KiB at boot, up to 64 MiB) |
| 507 | `0xffff_fd80_0000_0000` | Thread kernel stacks: 128 KiB per slot, 16 pages each, guard page below |
| 508 | `0xffff_fe00_0000_0000` | Boot kernel stack, 64 KiB, guard page below |
| 509 | `0xffff_fe80_0000_0000` | Scratch window for boot checks and tests |
| 510 | `0xffff_ff00_0000_0000` | Local APIC registers (uncached) |
| 511 | `0xffff_ffff_8000_0000` | Kernel image: code R+X, rodata R, data/bss R+W, nothing both writable and executable |

User half of each task (`0 .. 0x0000_8000_0000_0000`): code and read-only data
from `0x40_0000`, writable data and bss from `0x60_0000`, a 4-page stack just
below `0x0000_8000_0000_0000` with an unmapped guard page beneath it. Everything
else is unmapped. Frames given to a task are zeroed first.

## 7. The system-call interface

Registers: `rax` = call number; arguments in `rdi`, `rsi`, `rdx`, `r10`, `r8`;
the result in `rax`. An error is `-(code)` (top bit set). Calls that receive a
message also return the badge in `rdi` and three words in `rdx`, `r10`, `r8`.
Decoding and packing live in `crates/syscall` (host-tested and fuzzed); user
programs call them through `user/src/lib.rs`.

| # | Call | Arguments | Does |
|---|---|---|---|
| 0 | `debug_print` | pointer, length (≤ 256) | print sanitised text, tagged with the task |
| 1 | `yield` | — | give up the rest of the time slice |
| 2 | `exit` | code | end the calling task |
| 3 | `invoke` | slot, label, 3 args | an operation on the capability in `slot` (below) |
| 4 | `send` | endpoint slot, label word, 3 words | deliver a message, waiting for a receiver |
| 5 | `recv` | endpoint slot, slot for a received capability | wait for a message |
| 6 | `call` | endpoint slot, label word, 3 words | send, then wait for the reply |
| 7 | `reply` | label, 3 words | answer the last caller |

`invoke` operations (the low byte of the label):

| Label | Operation | Invoked on |
|---|---|---|
| 0 | `identify`: returns `kind \| rights << 8 \| size_bits << 16` | any capability |
| 1 | `retype`: carve objects (untyped, frame, endpoint, notification, empty CNode) | untyped memory |
| 2 | `derive`: copy a slot with fewer rights, optionally badged | the caller's own CNode |
| 3 | `move` | the caller's own CNode |
| 4 | `delete` | the caller's own CNode |
| 5 | `revoke`: delete everything derived from a slot | the caller's own CNode |
| 6 | `spawn`: start a boot module with copies of some slots (interim, ADR-0011) | a spawner |
| 7 | `join`: wait for a task and get its exit status | a spawner |
| 8 | `signal` | a notification (needs write) |
| 9 | `wait`: returns the combined badge in `rdi` | a notification (needs read) |

Error codes: 1 no such call, 2 bad address, 3 too long, 4 invalid slot, 5 empty
slot, 6 slot occupied, 7 no rights, 8 bad argument, 9 out of memory, 10 wrong
kind, 11 aborted, 12 no reply target, 13 no such task, 14 no resources.

## 8. Capabilities

- **A capability** names one object and carries rights: read (1), write (2),
  grant (4), grant-reply (8). Rights only shrink along a derivation. Endpoint and
  notification capabilities can carry a badge, set once.
- **Kinds:** untyped memory, CNode, endpoint, notification, frame, spawner (in
  use); thread and page table (reserved, not implemented).
- **A slot** is 32 bytes; all-zero bytes are an empty slot. Slots are linked
  into derivation lists, so revocation finds every copy, in any task.
- **A task's capability space** is one 4 KiB CNode of 128 slots. Slot 0 is a
  capability to the space itself. A slot number is checked against the task's
  own space, so a task cannot name anything it was not given; a task with no
  space can name nothing.
- **Memory:** the kernel does not create objects for tasks. A task retypes
  untyped memory it holds; retyped objects are zeroed; revoking a region removes
  every object carved from it. **Exception:** the interim spawner makes the
  kernel allocate a new task's address space and capability-space frame
  (ADR-0011, to be replaced by task-owned thread and address-space objects).
- **The root task** (`user/src/bin/init.rs`) starts with slot 0 its own space,
  slot 1 a 4 MiB untyped region, slot 2 the spawner, and nothing else. At every
  boot it makes an endpoint and a frame, hands a server a receive-only copy and a
  client a badged send-only copy, starts them and a stranger that gets nothing,
  and checks the results. A spawned task gets its own space in slot 0, slot 1
  empty, and the copied capabilities from slot 2 on.
- **Boot modules** are numbered by their order in `boot/limine.conf`
  (`durg_syscall::module`); a self-test checks the numbering.

The pure rules (derive, revoke, retype, invariants) are in `crates/cap` with
property tests and a fuzz target; the kernel side is `kernel/src/cspace.rs`.

## 9. Messages and notifications

- **Endpoints:** synchronous rendezvous. `send` needs write, `recv` read, a
  capability in a message needs grant. The receiver learns the sender's badge.
  A transferred capability lands in a slot the receiver named, as a child of the
  sender's (revocable). Waiters are served first come, first served.
- **Notifications:** asynchronous; `signal` OR-combines badges; `wait` returns
  them or blocks.
- **Blocking** never allocates: blocked threads sit in a fixed-capacity list.
  Destroying an endpoint or notification wakes its waiters with *aborted*; a
  caller waiting for a reply is woken if the server exits; a blocked user task can
  be stopped by the kernel's internal kill handle.
- Capability lookup and the scheduler hand-off run with interrupts masked, so on
  this single-CPU kernel no other task can revoke the object in between.

## 10. How it is tested

| Layer | What | Where |
|---|---|---|
| Pure logic | Unit and property tests on the host | `cargo test` on the crates in `crates/` |
| Kernel | In-kernel self-tests, including ring-3 test programs, run in QEMU | `kernel/src/selftest.rs`, `cargo xtask test` |
| Boot paths | Six QEMU variants, debug and release | `cargo xtask test [--release]` |
| Hostile input | libFuzzer targets: `syscall_validate`, `split_pages`, `elf_parse`, `cap_ops`, `virtio_chain` | `fuzz/`, nightly only, not in CI |
| Whole matrix | fmt, clippy for every kernel feature set, host tests, both boot profiles | `.github/workflows/ci.yml` |

Negative controls (breaking the code on purpose to check that a test notices)
are run by hand and recorded in the book chapters and devlog.
