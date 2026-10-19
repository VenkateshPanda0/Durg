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
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};

use crate::arch::x86_64::BootCell;

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

/// The part of `struct limine_file` this kernel reads. The structure has more
/// fields after these (media type, partition and disk identifiers); they are not
/// needed and are not declared.
#[repr(C)]
struct FileInfo {
    _revision: u64,
    /// Virtual address of the file, in the bootloader's direct map.
    address: u64,
    size: u64,
    /// Zero-terminated path of the file, with a leading slash.
    path: *const u8,
}

#[repr(C)]
struct ModuleResponse {
    _revision: u64,
    module_count: u64,
    /// Pointer to an array of `module_count` pointers to files.
    modules: *const *const FileInfo,
}

#[used]
#[link_section = ".limine_requests"]
static MODULE_REQUEST: Request<ModuleResponse> =
    Request::new([0x3e7e_2797_02be_32af, 0xca1c_4f3b_d128_0cee]);

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

/// Most memory map entries kept. Real machines report tens, so 128 leaves room.
pub const MAX_REGIONS: usize = 128;

/// A region value to fill the capture array with before it is written.
const EMPTY_REGION: MemoryRegion = MemoryRegion {
    base: 0,
    length: 0,
    kind: MemoryKind::Reserved,
};

/// Everything the kernel needs from the bootloader's responses, copied into
/// kernel memory once by [`capture`].
///
/// The responses themselves live in bootloader-reclaimable memory, which the
/// kernel later gives back to the frame allocator. Keeping a copy means no code
/// reads bootloader memory after boot has started.
struct Captured {
    regions: [MemoryRegion; MAX_REGIONS],
    region_count: usize,
    hhdm_offset: u64,
    rsdp: Option<u64>,
    modules: [RawModule; MAX_MODULES],
    module_count: usize,
}

/// Most boot modules kept.
pub const MAX_MODULES: usize = 8;
/// Longest module file name kept (the part of the path after the last slash).
const MAX_MODULE_NAME: usize = 31;
/// Largest module accepted. Keeps a corrupt size from describing absurd memory.
pub const MAX_MODULE_BYTES: u64 = 16 * 1024 * 1024;

/// A module as captured: where it is in physical memory and what it is called.
#[derive(Clone, Copy)]
struct RawModule {
    physical: u64,
    size: u64,
    name: [u8; MAX_MODULE_NAME],
    name_len: usize,
}

const EMPTY_MODULE: RawModule = RawModule {
    physical: 0,
    size: 0,
    name: [0; MAX_MODULE_NAME],
    name_len: 0,
};

static CAPTURED: BootCell<Captured> = BootCell::new(Captured {
    regions: [EMPTY_REGION; MAX_REGIONS],
    region_count: 0,
    hhdm_offset: 0,
    rsdp: None,
    modules: [EMPTY_MODULE; MAX_MODULES],
    module_count: 0,
});
static CAPTURE_DONE: AtomicBool = AtomicBool::new(false);

/// Why [`capture`] failed.
#[derive(Debug, Clone, Copy)]
pub enum CaptureError {
    AlreadyCaptured,
    NoMemoryMap,
    NoHhdm,
    /// The memory map has more entries than [`MAX_REGIONS`].
    TooManyRegions(usize),
    /// More modules than [`MAX_MODULES`], or one larger than [`MAX_MODULE_BYTES`],
    /// or one whose address is not in the bootloader's direct map.
    BadModule,
}

impl core::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CaptureError::AlreadyCaptured => write!(f, "boot information was already captured"),
            CaptureError::NoMemoryMap => write!(f, "the bootloader gave no memory map"),
            CaptureError::NoHhdm => write!(f, "the bootloader gave no HHDM offset"),
            CaptureError::TooManyRegions(count) => {
                write!(
                    f,
                    "memory map has {count} entries, more than the {MAX_REGIONS} kept"
                )
            }
            CaptureError::BadModule => write!(f, "a boot module is missing, too big or misplaced"),
        }
    }
}

/// Copies the memory map, the HHDM offset and the RSDP address out of the
/// bootloader's responses into kernel memory. Call once, first thing after the
/// protocol tag is checked. Every accessor below reads the copy.
pub fn capture() -> Result<(), CaptureError> {
    if CAPTURE_DONE.load(Ordering::Acquire) {
        return Err(CaptureError::AlreadyCaptured);
    }
    let memmap = MEMMAP_REQUEST.response().ok_or(CaptureError::NoMemoryMap)?;
    let hhdm = HHDM_REQUEST.response().ok_or(CaptureError::NoHhdm)?;
    // A count that does not fit `usize` cannot describe real memory.
    let count = usize::try_from(memmap.entry_count).unwrap_or(usize::MAX);
    if count > MAX_REGIONS {
        return Err(CaptureError::TooManyRegions(count));
    }

    // SAFETY: nothing reads `CAPTURED` until `CAPTURE_DONE` is set at the end of
    // this function, and the kernel runs on one CPU at this point, so this is
    // the only access.
    let captured = unsafe { &mut *CAPTURED.get() };
    for index in 0..count {
        // SAFETY: the spec says `entries` points to an array of `entry_count`
        // pointers to valid entries, and `index` is below `entry_count`. The
        // bootloader memory holding them is still mapped and unmodified, because
        // this runs before the kernel reuses any of it.
        let entry = unsafe { &**memmap.entries.add(index) };
        captured.regions[index] = MemoryRegion {
            base: entry.base,
            length: entry.length,
            kind: MemoryKind::from_raw(entry.kind),
        };
    }
    captured.region_count = count;
    captured.hhdm_offset = hhdm.offset;
    captured.rsdp = RSDP_REQUEST.response().map(|response| response.address);
    // No response means no modules, which is not an error here: the caller
    // decides whether it needs any.
    if let Some(modules) = MODULE_REQUEST.response() {
        let count = usize::try_from(modules.module_count).unwrap_or(usize::MAX);
        if count > MAX_MODULES {
            return Err(CaptureError::BadModule);
        }
        for index in 0..count {
            // SAFETY: the spec says `modules` points to an array of
            // `module_count` pointers to valid file structures, and `index` is
            // below `module_count`. They are in bootloader memory that is still
            // mapped and unmodified (this runs before the kernel reuses it).
            let file = unsafe { &**modules.modules.add(index) };
            captured.modules[index] = capture_module(file, hhdm.offset)?;
        }
        captured.module_count = count;
    }
    CAPTURE_DONE.store(true, Ordering::Release);
    Ok(())
}

/// Copies one module's location and name out of the bootloader's file structure.
fn capture_module(file: &FileInfo, hhdm_offset: u64) -> Result<RawModule, CaptureError> {
    if file.size == 0 || file.size > MAX_MODULE_BYTES {
        return Err(CaptureError::BadModule);
    }
    // The address is virtual, in the bootloader's direct map. Subtracting the
    // direct-map offset gives the physical address, and a module below the offset
    // cannot be in the direct map at all.
    let physical = file
        .address
        .checked_sub(hhdm_offset)
        .ok_or(CaptureError::BadModule)?;
    if !physical.is_multiple_of(4096) {
        return Err(CaptureError::BadModule);
    }
    // The name is the part of the path after the last slash.
    let mut name = [0u8; MAX_MODULE_NAME];
    let mut length = 0;
    // SAFETY: `path` is a zero-terminated string in bootloader memory (the spec),
    // read byte by byte up to a fixed bound, so a missing terminator cannot run
    // past it.
    unsafe {
        let mut at = file.path;
        for _ in 0..256 {
            let byte = *at;
            if byte == 0 {
                break;
            }
            if byte == b'/' {
                length = 0;
            } else if length < MAX_MODULE_NAME && byte.is_ascii_graphic() {
                name[length] = byte;
                length += 1;
            }
            at = at.add(1);
        }
    }
    Ok(RawModule {
        physical,
        size: file.size,
        name,
        name_len: length,
    })
}

/// A boot module: a file the bootloader loaded into memory next to the kernel.
#[derive(Clone, Copy)]
pub struct Module {
    /// Where it sits in physical memory (page-aligned).
    pub physical: u64,
    /// Its size in bytes.
    pub size: u64,
    name: [u8; MAX_MODULE_NAME],
    name_len: usize,
}

impl Module {
    /// The file name, without the directory.
    pub fn name(&self) -> &str {
        // Only printable ASCII is stored, so this is always valid UTF-8; the
        // fallback keeps the kernel panic-free regardless.
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("?")
    }
}

/// The boot modules, in the order the bootloader gave them.
pub fn modules() -> impl Iterator<Item = Module> {
    let (modules, count) = captured().map_or((&[][..], 0), |c| (&c.modules[..], c.module_count));
    modules[..count].iter().map(|raw| Module {
        physical: raw.physical,
        size: raw.size,
        name: raw.name,
        name_len: raw.name_len,
    })
}

/// The captured data, or `None` before [`capture`].
fn captured() -> Option<&'static Captured> {
    if !CAPTURE_DONE.load(Ordering::Acquire) {
        return None;
    }
    // SAFETY: after `CAPTURE_DONE` is set the data is never written again, so a
    // shared reference for the rest of the kernel's life is sound.
    Some(unsafe { &*CAPTURED.get() })
}

/// The bootloader's physical memory map, as captured at boot.
pub struct MemoryMap {
    regions: &'static [MemoryRegion],
}

/// Returns the captured memory map, or `None` before [`capture`].
pub fn memory_map() -> Option<MemoryMap> {
    captured().map(|captured| MemoryMap {
        regions: &captured.regions[..captured.region_count],
    })
}

impl MemoryMap {
    pub fn len(&self) -> usize {
        self.regions.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = MemoryRegion> + '_ {
        self.regions.iter().copied()
    }
}

/// Offset of the higher-half direct map: virtual address = physical + offset.
///
/// Only regions the spec lists as mapped (usable, bootloader reclaimable,
/// executable/modules, framebuffer) can be reached this way under the
/// bootloader's page tables. The kernel's own page tables choose their own set.
pub fn hhdm_offset() -> Option<u64> {
    captured().map(|captured| captured.hhdm_offset)
}

/// `true` if the physical range `start..start + length` lies entirely inside a
/// single memory region that the bootloader maps in the HHDM, so the kernel may
/// read it through the direct map.
///
/// Mapped kinds (spec, "Base Revision 3" and "Base Revision 4"): usable,
/// bootloader reclaimable, executable and modules, framebuffer, and, for
/// revision 4, ACPI reclaimable, ACPI NVS and reserved-mapped.
pub fn is_hhdm_mapped(start: u64, length: u64) -> bool {
    let Some(end) = start.checked_add(length) else {
        return false;
    };
    let Some(map) = memory_map() else {
        return false;
    };
    let covered = map.iter().any(|region| {
        let mapped = match region.kind {
            MemoryKind::Usable
            | MemoryKind::BootloaderReclaimable
            | MemoryKind::ExecutableAndModules
            | MemoryKind::Framebuffer => true,
            MemoryKind::AcpiReclaimable | MemoryKind::AcpiNvs | MemoryKind::ReservedMapped => {
                loaded_base_revision() >= 4
            }
            _ => false,
        };
        mapped && region.base <= start && end <= region.end()
    });
    covered
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
    captured().and_then(|captured| captured.rsdp)
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
