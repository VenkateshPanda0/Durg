//! Memory management: the physical frame allocator ([`frame`], M4), page tables
//! ([`paging`]: address spaces, mapping, unmapping, translation) and the kernel's
//! own address space ([`vm`]: W^X, a guarded stack, taking over from the
//! bootloader, M5). The heap (M6) comes later.

pub mod frame;
pub mod paging;
pub mod vm;
