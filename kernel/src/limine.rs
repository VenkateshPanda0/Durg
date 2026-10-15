//! Minimal Limine boot protocol bindings, written in-house (ADR-0003).
//!
//! The protocol works by the kernel embedding magic-number tagged structures in
//! its image. The bootloader scans the image for them, fills in a response for
//! each one it understands, and jumps to the entry point.
//!
//! Source of truth: the Limine boot protocol specification, `PROTOCOL.md` in
//! <https://codeberg.org/Limine/limine-protocol>. The request IDs and struct
//! layouts below were checked against that document and the `limine.h` header
//! shipped with the pinned bootloader release, not written from memory.
//!
//! Facts about base revisions 3 and 4 that this module relies on (spec, "Base
//! Revision 3", "Base Revision 4" and "Memory Map Feature"):
//!
//! - The HHDM (higher-half direct map) is **restrictive**: usable,
//!   bootloader-reclaimable, executable/modules and framebuffer regions are
//!   mapped. **Revision 4 additionally maps ACPI reclaimable, ACPI NVS and
//!   reserved-mapped regions**, which is why this kernel asks for revision 4:
//!   it can then read the ACPI tables without page tables of its own.
//! - The RSDP address is **physical** under revision 3 and **virtual** (an HHDM
//!   address) under revision 4.
//! - A bootloader that does not know revision 4 must still load us as revision
//!   3, and it records the revision it actually used in the tag, which
//!   [`loaded_base_revision`] reads.
//! - Memory map entries are sorted by base address. Usable and
//!   bootloader-reclaimable entries are 4096-byte aligned (base and length)
//!   and overlap no other entry.
//!
//! This is the one module outside `arch/` that contains `unsafe`. It reads
//! memory the bootloader owns, and the unsafe code is confined to a few small
//! accessors with their invariants stated.

use core::ptr;
use core::sync::atomic::{AtomicPtr, AtomicU64, Ordering};

/// Protocol revision this kernel asks for. See the module docs for why 4.
const BASE_REVISION: u64 = 4;

/// The second magic word of the base-revision tag. A bootloader that processed
/// the tag replaces it with the revision it actually used.
const BASE_REVISION_MAGIC_2: u64 = 0x6a7b_3849_4453_6bdc;

/// First two words shared by every request ID.
const COMMON_MAGIC: [u64; 2] = [0xc7b1_dd30_df4c_8b88, 0x0a82_e883_a194_f07b];

/// Marks the start of the area the bootloader scans for requests.
#[used]
#[link_section = ".limine_requests_start"]
static REQUESTS_START: [u64; 4] = [
    0xf6b8_f4b3_9de7_d1ae,
    0xfab9_1a69_40fc_b9cf,
    0x785c_6ed0_15d3_e316,
    0x181e_920a_7852_b9d9,
];

/// Marks the end of the area the bootloader scans for requests.
#[used]
#[link_section = ".limine_requests_end"]
static REQUESTS_END: [u64; 2] = [0xadc0_e053_1bb1_0d03, 0x9572_709f_3176_4c62];

/// The base-revision tag: two magic numbers, then the revision.
///
/// The bootloader overwrites the third word with `0` if it supports the
/// requested revision, which is why these are atomics (the memory changes
/// behind the compiler's back) and not plain constants.
#[used]
#[link_section = ".limine_requests"]
static BASE_REVISION_TAG: [AtomicU64; 3] = [
    AtomicU64::new(0xf956_2b2d_5c95_a6c8),
    AtomicU64::new(BASE_REVISION_MAGIC_2),
    AtomicU64::new(BASE_REVISION),
];

/// Returns `true` if a Limine-protocol bootloader processed the base-revision
/// tag at all. If the magic number is still in place, nothing touched it and
/// none of the boot information can be trusted.
pub fn bootloader_processed_tag() -> bool {
    BASE_REVISION_TAG[1].load(Ordering::Relaxed) != BASE_REVISION_MAGIC_2
}

/// The protocol revision the bootloader actually used. The spec requires it to
/// store this in the tag's second word, replacing the magic number. It may be
/// lower than the revision requested, because a bootloader that does not know
/// our revision must still load us with the highest one it supports. Only
/// meaningful if [`bootloader_processed_tag`] is `true`.
pub fn loaded_base_revision() -> u64 {
    BASE_REVISION_TAG[1].load(Ordering::Relaxed)
}

/// A request: four ID words, a request revision, and a response pointer the
/// bootloader fills in. `R` is the response type.
#[repr(C)]
struct Request<R> {
    id: [u64; 4],
    revision: u64,
    response: AtomicPtr<R>,
}

impl<R> Request<R> {
    const fn new(id_tail: [u64; 2]) -> Self {
        Self {
            id: [COMMON_MAGIC[0], COMMON_MAGIC[1], id_tail[0], id_tail[1]],
            revision: 0,
            response: AtomicPtr::new(ptr::null_mut()),
        }
    }

    /// The bootloader's answer, or `None` if it did not provide one.
    fn response(&self) -> Option<&'static R> {
        let pointer = self.response.load(Ordering::Relaxed);
        // SAFETY: the bootloader either leaves this null or points it at a
        // response structure it placed in memory that stays mapped and
        // unmodified for the lifetime of the kernel. The kernel must not
        // reclaim bootloader-reclaimable memory without copying what it needs
        // first (a rule for the memory manager, M4).
        unsafe { pointer.as_ref() }
    }
}

#[repr(C)]
struct MemmapEntry {
    base: u64,
    length: u64,
    kind: u64,
}

#[repr(C)]
struct MemmapResponse {
    _revision: u64,
    entry_count: u64,
    /// Pointer to an array of `entry_count` pointers to entries.
    entries: *const *const MemmapEntry,
}

#[repr(C)]
struct HhdmResponse {
    _revision: u64,
    offset: u64,
}

#[repr(C)]
struct RsdpResponse {
    _revision: u64,
    /// Physical for base revision 3, virtual (HHDM) for base revision 4.
    address: u64,
}

#[used]
#[link_section = ".limine_requests"]
static MEMMAP_REQUEST: Request<MemmapResponse> =
    Request::new([0x67cf_3d9d_378a_806f, 0xe304_acdf_c50c_3c62]);

#[used]
#[link_section = ".limine_requests"]
static HHDM_REQUEST: Request<HhdmResponse> =
    Request::new([0x48dc_f1cb_8ad2_b852, 0x6398_4e95_9a98_244b]);

#[used]
#[link_section = ".limine_requests"]
static RSDP_REQUEST: Request<RsdpResponse> =
    Request::new([0xc5e7_7b6b_397e_7b43, 0x2763_7845_accd_cf3c]);

/// What a memory map region is used for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    Usable,
    Reserved,
    AcpiReclaimable,
    AcpiNvs,
    BadMemory,
    BootloaderReclaimable,
    ExecutableAndModules,
    Framebuffer,
    ReservedMapped,
    /// A type this kernel does not know. Treated as unusable.
    Unknown(u64),
}

impl MemoryKind {
    fn from_raw(raw: u64) -> Self {
        match raw {
            0 => Self::Usable,
            1 => Self::Reserved,
            2 => Self::AcpiReclaimable,
            3 => Self::AcpiNvs,
            4 => Self::BadMemory,
            5 => Self::BootloaderReclaimable,
            6 => Self::ExecutableAndModules,
            7 => Self::Framebuffer,
            8 => Self::ReservedMapped,
            other => Self::Unknown(other),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Usable => "usable",
            Self::Reserved => "reserved",
            Self::AcpiReclaimable => "acpi reclaimable",
            Self::AcpiNvs => "acpi nvs",
            Self::BadMemory => "bad memory",
            Self::BootloaderReclaimable => "bootloader reclaimable",
            Self::ExecutableAndModules => "kernel and modules",
            Self::Framebuffer => "framebuffer",
            Self::ReservedMapped => "reserved (mapped)",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// One region of the physical address space, copied out of bootloader memory.
#[derive(Clone, Copy)]
pub struct MemoryRegion {
    pub base: u64,
    pub length: u64,
    pub kind: MemoryKind,
}

impl MemoryRegion {
    /// First address after the region. Saturates instead of wrapping so a
    /// malformed entry cannot produce an end below its base.
    pub fn end(&self) -> u64 {
        self.base.saturating_add(self.length)
    }
}

/// The bootloader's physical memory map.
pub struct MemoryMap {
    response: &'static MemmapResponse,
}

/// Returns the memory map, or `None` if the bootloader did not provide one.
pub fn memory_map() -> Option<MemoryMap> {
    MEMMAP_REQUEST
        .response()
        .map(|response| MemoryMap { response })
}

impl MemoryMap {
    pub fn len(&self) -> usize {
        // A count that does not fit `usize` cannot describe real memory.
        usize::try_from(self.response.entry_count).unwrap_or(0)
    }

    /// Returns region `index`, copied out of bootloader memory.
    pub fn get(&self, index: usize) -> Option<MemoryRegion> {
        if index >= self.len() {
            return None;
        }
        // SAFETY: the spec says `entries` points to an array of `entry_count`
        // pointers to valid entries, and `index` is below `entry_count`. Both
        // the array and the entries live in bootloader memory that stays
        // mapped (see `Request::response`).
        let entry = unsafe { &**self.response.entries.add(index) };
        Some(MemoryRegion {
            base: entry.base,
            length: entry.length,
            kind: MemoryKind::from_raw(entry.kind),
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = MemoryRegion> + '_ {
        (0..self.len()).filter_map(move |index| self.get(index))
    }
}

/// Offset of the higher-half direct map: virtual address = physical + offset.
///
/// Only regions the spec lists as mapped (usable, bootloader reclaimable,
/// executable/modules, framebuffer) can be reached this way.
pub fn hhdm_offset() -> Option<u64> {
    HHDM_REQUEST.response().map(|response| response.offset)
}

/// Virtual (HHDM) address of the ACPI RSDP.
///
/// Returns `None` if the bootloader gave no RSDP, or if it loaded us under
/// base revision 3, where the address is physical and ACPI memory is not
/// mapped. Only base revision 4 makes this readable.
pub fn rsdp_virtual_address() -> Option<u64> {
    if loaded_base_revision() < 4 {
        return None;
    }
    RSDP_REQUEST.response().map(|response| response.address)
}

/// Reads one byte of a *usable* region through the HHDM.
///
/// Returns `None` for any other kind, because only the kinds listed in the
/// module docs are guaranteed to be mapped. Used by the boot self-test to
/// check that the direct map really covers usable RAM.
#[cfg(feature = "selftest")]
pub fn read_usable_byte(region: &MemoryRegion) -> Option<u8> {
    if region.kind != MemoryKind::Usable || region.length == 0 {
        return None;
    }
    let virtual_address = region.base.checked_add(hhdm_offset()?)?;
    // SAFETY: the region is usable RAM, which the spec says is mapped in the
    // HHDM for base revision 3, so `base + offset` is a mapped, readable
    // address. It is free RAM the kernel does not own yet, and a volatile
    // read of a byte has no side effect.
    Some(unsafe { ptr::read_volatile(virtual_address as *const u8) })
}
