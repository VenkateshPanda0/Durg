# `kernel/`: the Durg kernel (`durg-kernel`)

The ring-0 kernel: a `no_std`, `no_main` Rust binary for `x86_64-unknown-none`,
booted by Limine. It is built and run through `cargo xtask` (see
[getting started](../docs/contributor-quickstart.md)), not with plain `cargo run`.

How it fits together: [docs/ARCHITECTURE.md](../docs/ARCHITECTURE.md), sections 4
to 9. Why it is built this way: the [book chapters](../docs/book/), one per milestone.

| Path | Contents |
|---|---|
| `src/main.rs` | Boot, in two stages; the panic handler; how each test variant ends |
| `src/arch/` | Serial port and x86-64 specifics: GDT/TSS, IDT and trap handlers, PIC, local APIC timer, context switch, `syscall` entry |
| `src/mm/` | Physical frames, page tables, the kernel's own address space, the heap |
| `src/sched.rs` | Threads, the scheduler, blocking, endpoint IPC, notifications, `join` |
| `src/syscall.rs` | System-call handlers |
| `src/cspace.rs` | Capability spaces, `invoke`, untyped memory |
| `src/user.rs` | The ELF loader and the root task's start |
| `src/limine.rs`, `src/acpi.rs`, `src/boot.rs`, `src/console.rs`, `src/qemu.rs` | Bootloader protocol, ACPI MADT, boot report, printing, QEMU exit codes |
| `src/selftest.rs` | In-kernel self-tests (feature `selftest`) |
| `linker.ld`, `build.rs` | Higher-half link layout and the build script |

**Features** (`Cargo.toml`): `selftest` runs the self-tests; `fail-demo`,
`df-test`, `so-test` and `oom-test` each trigger a deliberate failure that
`cargo xtask test` checks is handled. Rules: every `unsafe` block is small and
carries a `// SAFETY:` comment; logic that can be pure lives in `../crates/`.
