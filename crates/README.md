# `crates/`: pure logic, tested on the host

Each crate here is `no_std`, contains **no `unsafe` code**, and does no I/O, so it
can be unit-tested, property-tested and fuzzed on an ordinary machine and then
used by the kernel unchanged. When kernel logic can be made pure, it belongs here.

| Crate | What it does | Used by |
|---|---|---|
| `acpi` (`durg-acpi`) | Parses the ACPI tables the kernel needs (RSDP, RSDT/XSDT, MADT) | `kernel/src/acpi.rs` |
| `cap` (`durg-cap`) | Capability slots, derive, revoke, retype, invariant checker; notification state | `kernel/src/cspace.rs`, `sched.rs`, user programs |
| `elf` (`durg-elf`) | ELF64 parser that treats the file as hostile and fails closed | `kernel/src/user.rs` |
| `heap` (`durg-heap`) | Free-list heap allocator with coalescing and double-free detection | `kernel/src/mm/heap.rs` |
| `paging` (`durg-paging`) | x86-64 page-table entries, address splitting, a table walker | `kernel/src/mm/` |
| `pmm` (`durg-pmm`) | Bitmap physical frame allocator, aligned-block search, atomic block return | `kernel/src/mm/frame.rs` |
| `sync` (`durg-sync`) | Spinlock and interrupt-safe lock, with the interrupt control as a trait | `kernel/src/arch/x86_64/` |
| `syscall` (`durg-syscall`) | System-call numbers, decoding, labels and packing, error codes, user-range checks | kernel and `user/` |
| `virtio` (`durg-virtio`) | Virtio 1.2 split-queue layout and descriptor-chain validation (groundwork for M11; not a driver) | not yet used by the kernel |

Run the tests: `cargo test -p durg-cap` (one crate) or
`cargo test --workspace --exclude durg-kernel --exclude durg-user` (all of them).
Fuzz targets for several of these crates are in [../fuzz/](../fuzz/README.md).
