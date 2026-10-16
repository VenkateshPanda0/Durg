//! Memory management: the physical frame allocator ([`frame`], M4), page tables
//! ([`paging`]: address spaces, mapping, unmapping, translation) and the kernel's
//! own address space ([`vm`]: W^X, a guarded stack, taking over from the
//! bootloader, M5) and the kernel heap ([`heap`], M6).

pub mod frame;
pub mod heap;
pub mod paging;
pub mod vm;
