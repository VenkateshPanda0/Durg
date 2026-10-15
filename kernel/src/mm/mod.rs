//! Memory management. Today that is only the first piece of paging: reading and
//! editing the page tables the bootloader built. The physical frame allocator
//! (M4), full address-space control (M5) and heap (M6) come later.

pub mod paging;
