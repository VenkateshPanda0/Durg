//! Memory management: the physical frame allocator ([`frame`], M4) and the first
//! piece of paging ([`paging`]: reading and editing the page tables the
//! bootloader built). Full address-space control (M5) and the heap (M6) come
//! later.

pub mod frame;
pub mod paging;
