//! Minimal Limine boot protocol bindings, written in-house (ADR-0003).
//!
//! The protocol works by the kernel embedding magic-number tagged structures in
//! its image. The bootloader scans the image for them, fills in answers, and
//! jumps to the entry point. M1 needs only the *base revision* tag, which tells
//! the bootloader which protocol revision the kernel expects. Further requests
//! (memory map, HHDM offset, RSDP) are added as milestones need them.
//!
//! Source of truth: the Limine boot protocol specification (`PROTOCOL.md` in
//! the Limine repository), section "Base Revision".

use core::sync::atomic::{AtomicU64, Ordering};

/// Protocol revision this kernel is written against.
const BASE_REVISION: u64 = 3;

/// The base-revision tag: two magic numbers, then the revision.
///
/// The bootloader overwrites the third word with `0` if it supports the
/// requested revision, which is why these are atomics (the memory changes
/// behind the compiler's back) and not plain constants.
#[used]
#[link_section = ".limine_requests"]
static BASE_REVISION_TAG: [AtomicU64; 3] = [
    AtomicU64::new(0xf956_2b2d_5c95_a6c8),
    AtomicU64::new(0x6a7b_3849_4453_6bdc),
    AtomicU64::new(BASE_REVISION),
];

/// Returns `true` if the bootloader accepted the requested protocol revision.
pub fn base_revision_supported() -> bool {
    BASE_REVISION_TAG[2].load(Ordering::Relaxed) == 0
}
