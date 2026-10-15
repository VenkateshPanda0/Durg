//! ACPI discovery: find the RSDP, walk to the MADT, and parse it.
//!
//! The byte-level parsing lives in the host-testable `durg-acpi` crate. This
//! module is the part that touches memory: it turns the bootloader's RSDP
//! address into bytes, but only after checking that the memory really is ACPI
//! memory the bootloader mapped.
//!
//! Limine base revision 4 maps ACPI reclaimable, ACPI NVS and reserved-mapped
//! regions in the higher-half direct map and returns the RSDP as a virtual
//! address. (Base revision 3 maps none of those, so ACPI would be unreadable
//! without our own page tables.)

use durg_acpi::{AcpiError, Madt, Rsdp, Sdt, SdtHeader, SDT_HEADER_LEN};

use crate::limine::{self, MemoryKind};

/// Upper bound on tables inspected, so corrupt firmware data cannot loop us.
const MAX_TABLES: usize = 512;

/// Why ACPI discovery failed.
#[derive(Debug, Clone, Copy)]
pub enum Error {
    /// The bootloader gave no RSDP, or only a physical one we cannot read.
    NoRsdp,
    /// A table lies outside the memory regions the bootloader mapped for ACPI.
    OutsideMappedAcpiMemory(u64),
    /// A table failed validation.
    Parse(AcpiError),
    /// The RSDT/XSDT does not list a MADT.
    NoMadt,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::NoRsdp => write!(f, "no readable RSDP (base revision 4 required)"),
            Error::OutsideMappedAcpiMemory(address) => write!(
                f,
                "table at {address:#x} is outside the ACPI memory the bootloader mapped"
            ),
            Error::Parse(error) => write!(f, "table failed validation: {error:?}"),
            Error::NoMadt => write!(f, "no MADT listed in the root table"),
        }
    }
}

/// What discovery found.
pub struct AcpiInfo {
    pub rsdp_revision: u8,
    /// `true` if the XSDT was used, `false` for the older RSDT.
    pub used_xsdt: bool,
    pub madt: Madt,
}

/// Memory kinds the bootloader maps for ACPI under base revision 4.
fn is_mapped_acpi_kind(kind: MemoryKind) -> bool {
    matches!(
        kind,
        MemoryKind::AcpiReclaimable | MemoryKind::AcpiNvs | MemoryKind::ReservedMapped
    )
}

/// Bytes of the physical range `phys..phys + len`, if it lies inside a single
/// memory region the bootloader mapped for ACPI.
fn physical_bytes(phys: u64, len: usize) -> Result<&'static [u8], Error> {
    let outside = Error::OutsideMappedAcpiMemory(phys);
    let end = phys.checked_add(len as u64).ok_or(outside)?;
    let map = limine::memory_map().ok_or(outside)?;
    let inside = map
        .iter()
        .any(|r| is_mapped_acpi_kind(r.kind) && r.base <= phys && end <= r.end());
    if !inside {
        return Err(outside);
    }
    let virtual_address = phys
        .checked_add(limine::hhdm_offset().ok_or(outside)?)
        .ok_or(outside)?;
    // SAFETY: the range lies inside an ACPI region, which base revision 4 maps
    // in the HHDM (spec, "Base Revision 4"), so `virtual_address` is mapped and
    // readable for `len` bytes. The memory is firmware data that nothing else
    // in the kernel writes, so a shared slice is sound.
    Ok(unsafe { core::slice::from_raw_parts(virtual_address as *const u8, len) })
}

/// Reads a whole table at `phys`: the header first to learn the length, then
/// the full table, checksummed.
fn read_table(phys: u64) -> Result<Sdt<'static>, Error> {
    let header = SdtHeader::read(physical_bytes(phys, SDT_HEADER_LEN)?).map_err(Error::Parse)?;
    let bytes = physical_bytes(phys, header.length as usize)?;
    Sdt::parse(bytes).map_err(Error::Parse)
}

/// Finds and parses the MADT.
pub fn discover() -> Result<AcpiInfo, Error> {
    let rsdp_virtual = limine::rsdp_virtual_address().ok_or(Error::NoRsdp)?;
    let hhdm = limine::hhdm_offset().ok_or(Error::NoRsdp)?;
    let rsdp_physical = rsdp_virtual.checked_sub(hhdm).ok_or(Error::NoRsdp)?;

    // The full RSDP is 36 bytes. A shorter read is tried if the region ends
    // sooner, which is only valid for an ACPI 1.0 RSDP.
    let rsdp_bytes =
        physical_bytes(rsdp_physical, 36).or_else(|_| physical_bytes(rsdp_physical, 20))?;
    let rsdp = Rsdp::parse(rsdp_bytes).map_err(Error::Parse)?;

    let (root_address, wide) = if rsdp.revision >= 2 && rsdp.xsdt_address != 0 {
        (rsdp.xsdt_address, true)
    } else {
        (u64::from(rsdp.rsdt_address), false)
    };
    let root = read_table(root_address)?;

    for address in root.entries(wide).take(MAX_TABLES) {
        let header =
            SdtHeader::read(physical_bytes(address, SDT_HEADER_LEN)?).map_err(Error::Parse)?;
        if &header.signature == b"APIC" {
            let table = read_table(address)?;
            return Ok(AcpiInfo {
                rsdp_revision: rsdp.revision,
                used_xsdt: wide,
                madt: Madt::parse(&table).map_err(Error::Parse)?,
            });
        }
    }
    Err(Error::NoMadt)
}
