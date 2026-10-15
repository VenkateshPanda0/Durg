//! # ACPI table parsing (RSDP, RSDT/XSDT, MADT)
//!
//! This crate only interprets bytes. It never touches memory it was not handed,
//! never allocates, and compiles for both the bare-metal kernel and the host, so
//! its logic is unit-tested on the host (`cargo test -p durg-acpi`).
//!
//! The kernel is responsible for getting the bytes safely (checking that a
//! physical range really is ACPI memory the bootloader mapped) and passing them
//! in as slices.
//!
//! Every structure layout below is taken from the ACPI specification:
//!
//! - RSDP: section 5.2.5.3 ("Root System Description Pointer")
//! - System description table header: section 5.2.6
//! - RSDT and XSDT: sections 5.2.7 and 5.2.8
//! - MADT and its entries: section 5.2.12
//!
//! All multi-byte fields are little endian.
//!
//! Input is untrusted-ish firmware data, so every length is bounds checked and
//! a zero-length MADT entry is an error (it would otherwise loop forever).

#![no_std]

/// Maximum I/O APICs recorded. Real machines have one or a few.
pub const MAX_IO_APICS: usize = 8;
/// Maximum interrupt source overrides recorded.
pub const MAX_OVERRIDES: usize = 16;

const RSDP_SIGNATURE: &[u8; 8] = b"RSD PTR ";
/// Size of the ACPI 1.0 RSDP (the part covered by the first checksum).
const RSDP_V1_LEN: usize = 20;
/// Size of the ACPI 2.0+ RSDP (the part covered by the extended checksum).
const RSDP_V2_LEN: usize = 36;
/// Size of the common header of every system description table.
pub const SDT_HEADER_LEN: usize = 36;
const MADT_FIXED_LEN: usize = SDT_HEADER_LEN + 8;

/// Why a table could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpiError {
    /// The slice is shorter than the structure needs.
    TooShort,
    /// A signature did not match what was expected.
    BadSignature,
    /// A checksum did not sum to zero.
    BadChecksum,
    /// A length field is inconsistent (smaller than the header, larger than the
    /// slice, or zero where it must not be).
    BadLength,
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, AcpiError> {
    let raw = bytes.get(offset..offset + 2).ok_or(AcpiError::TooShort)?;
    Ok(u16::from_le_bytes([raw[0], raw[1]]))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, AcpiError> {
    let raw = bytes.get(offset..offset + 4).ok_or(AcpiError::TooShort)?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, AcpiError> {
    let raw = bytes.get(offset..offset + 8).ok_or(AcpiError::TooShort)?;
    let mut array = [0u8; 8];
    array.copy_from_slice(raw);
    Ok(u64::from_le_bytes(array))
}

/// ACPI checksums: all bytes of the structure sum to zero modulo 256.
fn checksum_ok(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

/// The root pointer, reduced to what we use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rsdp {
    /// 0 for ACPI 1.0 (RSDT only), 2 or more for ACPI 2.0+ (XSDT available).
    pub revision: u8,
    /// Physical address of the RSDT (32-bit).
    pub rsdt_address: u32,
    /// Physical address of the XSDT. Zero when `revision` is below 2.
    pub xsdt_address: u64,
}

impl Rsdp {
    /// Parses and checks the RSDP. `bytes` must start at the RSDP.
    pub fn parse(bytes: &[u8]) -> Result<Self, AcpiError> {
        if bytes.len() < RSDP_V1_LEN {
            return Err(AcpiError::TooShort);
        }
        if &bytes[0..8] != RSDP_SIGNATURE {
            return Err(AcpiError::BadSignature);
        }
        if !checksum_ok(&bytes[..RSDP_V1_LEN]) {
            return Err(AcpiError::BadChecksum);
        }
        let revision = bytes[15];
        let rsdt_address = u32_at(bytes, 16)?;
        if revision < 2 {
            return Ok(Self {
                revision,
                rsdt_address,
                xsdt_address: 0,
            });
        }
        // ACPI 2.0+: the `length` field covers the whole structure and a second
        // checksum covers all of it.
        let length = u32_at(bytes, 20)? as usize;
        if length < RSDP_V2_LEN || length > bytes.len() {
            return Err(AcpiError::BadLength);
        }
        if !checksum_ok(&bytes[..length]) {
            return Err(AcpiError::BadChecksum);
        }
        Ok(Self {
            revision,
            rsdt_address,
            xsdt_address: u64_at(bytes, 24)?,
        })
    }
}

/// The header common to every system description table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SdtHeader {
    pub signature: [u8; 4],
    /// Length of the whole table, header included.
    pub length: u32,
    pub revision: u8,
}

impl SdtHeader {
    /// Reads the header without checking the table checksum. Use this first to
    /// learn the table's length, then fetch that many bytes for [`Sdt::parse`].
    pub fn read(bytes: &[u8]) -> Result<Self, AcpiError> {
        if bytes.len() < SDT_HEADER_LEN {
            return Err(AcpiError::TooShort);
        }
        let mut signature = [0u8; 4];
        signature.copy_from_slice(&bytes[0..4]);
        Ok(Self {
            signature,
            length: u32_at(bytes, 4)?,
            revision: bytes[8],
        })
    }
}

/// A checked system description table: header plus the full table bytes.
#[derive(Debug, Clone, Copy)]
pub struct Sdt<'a> {
    pub header: SdtHeader,
    bytes: &'a [u8],
}

impl<'a> Sdt<'a> {
    /// Checks length and checksum. `bytes` must hold at least the table's
    /// declared length (extra trailing bytes are ignored).
    pub fn parse(bytes: &'a [u8]) -> Result<Self, AcpiError> {
        let header = SdtHeader::read(bytes)?;
        let length = header.length as usize;
        if length < SDT_HEADER_LEN || length > bytes.len() {
            return Err(AcpiError::BadLength);
        }
        let table = &bytes[..length];
        if !checksum_ok(table) {
            return Err(AcpiError::BadChecksum);
        }
        Ok(Self {
            header,
            bytes: table,
        })
    }

    /// The table including its header.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// For an RSDT or XSDT: the physical addresses of the tables it lists.
    ///
    /// `wide` is `true` for an XSDT (64-bit entries) and `false` for an RSDT
    /// (32-bit entries). A trailing partial entry is ignored.
    pub fn entries(&self, wide: bool) -> impl Iterator<Item = u64> + 'a {
        let size = if wide { 8 } else { 4 };
        let body = &self.bytes[SDT_HEADER_LEN..];
        body.chunks_exact(size).map(move |chunk| {
            if wide {
                let mut array = [0u8; 8];
                array.copy_from_slice(chunk);
                u64::from_le_bytes(array)
            } else {
                u64::from(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            }
        })
    }
}

/// One I/O APIC.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IoApic {
    pub id: u8,
    /// Physical address of its registers.
    pub address: u32,
    /// First global system interrupt this I/O APIC handles.
    pub gsi_base: u32,
}

/// An interrupt source override: ISA interrupt `source` is wired to global
/// system interrupt `gsi` instead of the identity mapping.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InterruptOverride {
    pub bus: u8,
    pub source: u8,
    pub gsi: u32,
    /// MPS INTI flags (polarity and trigger mode).
    pub flags: u16,
}

/// What the MADT tells us about interrupt hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Madt {
    /// Physical address of the local APIC registers, after any override entry.
    pub local_apic_address: u64,
    /// MADT flags. Bit 0: a legacy dual-8259 PIC is also installed.
    pub flags: u32,
    /// Processors that are enabled or could be brought online (APIC and x2APIC entries).
    pub cpu_count: u32,
    pub io_apics: [IoApic; MAX_IO_APICS],
    pub io_apic_count: usize,
    pub overrides: [InterruptOverride; MAX_OVERRIDES],
    pub override_count: usize,
}

impl Madt {
    /// Parses a MADT. `sdt` must already have passed [`Sdt::parse`] and carry
    /// the signature `APIC`.
    pub fn parse(sdt: &Sdt) -> Result<Self, AcpiError> {
        if &sdt.header.signature != b"APIC" {
            return Err(AcpiError::BadSignature);
        }
        let bytes = sdt.bytes();
        if bytes.len() < MADT_FIXED_LEN {
            return Err(AcpiError::TooShort);
        }
        let mut madt = Madt {
            local_apic_address: u64::from(u32_at(bytes, SDT_HEADER_LEN)?),
            flags: u32_at(bytes, SDT_HEADER_LEN + 4)?,
            cpu_count: 0,
            io_apics: [IoApic::default(); MAX_IO_APICS],
            io_apic_count: 0,
            overrides: [InterruptOverride::default(); MAX_OVERRIDES],
            override_count: 0,
        };

        let mut offset = MADT_FIXED_LEN;
        while offset < bytes.len() {
            // Every entry starts with a type byte and a length byte.
            let entry_type = *bytes.get(offset).ok_or(AcpiError::TooShort)?;
            let entry_len = usize::from(*bytes.get(offset + 1).ok_or(AcpiError::TooShort)?);
            // A length below 2 cannot even hold the header, and zero would loop
            // forever. A length past the table is truncated data.
            if entry_len < 2 || offset + entry_len > bytes.len() {
                return Err(AcpiError::BadLength);
            }
            let entry = &bytes[offset..offset + entry_len];
            match entry_type {
                // Processor Local APIC: flags bit 0 enabled, bit 1 online capable.
                0 => {
                    if entry_len < 8 {
                        return Err(AcpiError::BadLength);
                    }
                    if u32_at(entry, 4)? & 0b11 != 0 {
                        madt.cpu_count += 1;
                    }
                }
                // I/O APIC.
                1 => {
                    if entry_len < 12 {
                        return Err(AcpiError::BadLength);
                    }
                    if madt.io_apic_count < MAX_IO_APICS {
                        madt.io_apics[madt.io_apic_count] = IoApic {
                            id: entry[2],
                            address: u32_at(entry, 4)?,
                            gsi_base: u32_at(entry, 8)?,
                        };
                        madt.io_apic_count += 1;
                    }
                }
                // Interrupt source override.
                2 => {
                    if entry_len < 10 {
                        return Err(AcpiError::BadLength);
                    }
                    if madt.override_count < MAX_OVERRIDES {
                        madt.overrides[madt.override_count] = InterruptOverride {
                            bus: entry[2],
                            source: entry[3],
                            gsi: u32_at(entry, 4)?,
                            flags: u16_at(entry, 8)?,
                        };
                        madt.override_count += 1;
                    }
                }
                // Local APIC address override: a 64-bit address that replaces
                // the 32-bit one in the fixed part.
                5 => {
                    if entry_len < 12 {
                        return Err(AcpiError::BadLength);
                    }
                    madt.local_apic_address = u64_at(entry, 4)?;
                }
                // Processor Local x2APIC: same flag meaning as type 0.
                9 => {
                    if entry_len < 16 {
                        return Err(AcpiError::BadLength);
                    }
                    if u32_at(entry, 8)? & 0b11 != 0 {
                        madt.cpu_count += 1;
                    }
                }
                // NMI sources, GIC entries and others are not needed yet.
                _ => {}
            }
            offset += entry_len;
        }
        Ok(madt)
    }

    /// The I/O APICs found.
    pub fn io_apics(&self) -> &[IoApic] {
        &self.io_apics[..self.io_apic_count]
    }

    /// The interrupt source overrides found.
    pub fn overrides(&self) -> &[InterruptOverride] {
        &self.overrides[..self.override_count]
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// Sets the byte at `checksum_offset` so that `bytes[..covered]` sums to
    /// zero. The RSDP has two checksums covering different lengths, so the
    /// covered length matters.
    fn fix_checksum_over(bytes: &mut [u8], checksum_offset: usize, covered: usize) {
        bytes[checksum_offset] = 0;
        let sum = bytes[..covered]
            .iter()
            .fold(0u8, |sum, b| sum.wrapping_add(*b));
        bytes[checksum_offset] = 0u8.wrapping_sub(sum);
    }

    /// Sets the checksum byte so the whole slice sums to zero.
    fn fix_checksum(bytes: &mut [u8], checksum_offset: usize) {
        let len = bytes.len();
        fix_checksum_over(bytes, checksum_offset, len);
    }

    fn rsdp_v1(rsdt: u32) -> Vec<u8> {
        let mut bytes = Vec::from(*b"RSD PTR \0\0OEMID\0");
        bytes.extend_from_slice(&rsdt.to_le_bytes());
        assert_eq!(bytes.len(), 20);
        fix_checksum_over(&mut bytes, 8, RSDP_V1_LEN);
        bytes
    }

    fn rsdp_v2(rsdt: u32, xsdt: u64) -> Vec<u8> {
        let mut bytes = Vec::from(*b"RSD PTR \0\0OEMID\x02");
        bytes.extend_from_slice(&rsdt.to_le_bytes());
        bytes.extend_from_slice(&36u32.to_le_bytes());
        bytes.extend_from_slice(&xsdt.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0]); // extended checksum + reserved
        assert_eq!(bytes.len(), 36);
        fix_checksum_over(&mut bytes, 8, RSDP_V1_LEN); // covers the first 20 bytes
        fix_checksum_over(&mut bytes, 32, RSDP_V2_LEN); // covers all 36
        bytes
    }

    /// Builds a table with `signature` and `body`, checksum fixed.
    fn sdt(signature: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(signature);
        bytes.extend_from_slice(&((SDT_HEADER_LEN + body.len()) as u32).to_le_bytes());
        bytes.push(1); // revision
        bytes.push(0); // checksum, fixed below
        bytes.extend_from_slice(b"OEMID\0"); // 6
        bytes.extend_from_slice(b"OEMTABLE"); // 8
        bytes.extend_from_slice(&[0; 12]); // oem revision, creator id, creator revision
        assert_eq!(bytes.len(), SDT_HEADER_LEN);
        bytes.extend_from_slice(body);
        fix_checksum(&mut bytes, 9);
        bytes
    }

    fn madt_body(lapic: u32, flags: u32, entries: &[Vec<u8>]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&lapic.to_le_bytes());
        body.extend_from_slice(&flags.to_le_bytes());
        for entry in entries {
            body.extend_from_slice(entry);
        }
        body
    }

    fn local_apic(id: u8, flags: u32) -> Vec<u8> {
        let mut e = Vec::from([0u8, 8, id, id]);
        e.extend_from_slice(&flags.to_le_bytes());
        e
    }

    fn io_apic(id: u8, address: u32, gsi_base: u32) -> Vec<u8> {
        let mut e = Vec::from([1u8, 12, id, 0]);
        e.extend_from_slice(&address.to_le_bytes());
        e.extend_from_slice(&gsi_base.to_le_bytes());
        e
    }

    fn irq_override(source: u8, gsi: u32, flags: u16) -> Vec<u8> {
        let mut e = Vec::from([2u8, 10, 0, source]);
        e.extend_from_slice(&gsi.to_le_bytes());
        e.extend_from_slice(&flags.to_le_bytes());
        e
    }

    #[test]
    fn rsdp_v1_parses() {
        let rsdp = Rsdp::parse(&rsdp_v1(0x1234_5000)).unwrap();
        assert_eq!(rsdp.revision, 0);
        assert_eq!(rsdp.rsdt_address, 0x1234_5000);
        assert_eq!(rsdp.xsdt_address, 0);
    }

    #[test]
    fn rsdp_v2_parses_and_exposes_xsdt() {
        let rsdp = Rsdp::parse(&rsdp_v2(0x1000, 0xdead_beef_0000)).unwrap();
        assert_eq!(rsdp.revision, 2);
        assert_eq!(rsdp.xsdt_address, 0xdead_beef_0000);
    }

    #[test]
    fn rsdp_rejects_bad_signature_checksum_and_truncation() {
        let mut bad_sig = rsdp_v1(0);
        bad_sig[0] = b'X';
        assert_eq!(Rsdp::parse(&bad_sig), Err(AcpiError::BadSignature));

        let mut bad_sum = rsdp_v1(0);
        bad_sum[16] ^= 0xff;
        assert_eq!(Rsdp::parse(&bad_sum), Err(AcpiError::BadChecksum));

        assert_eq!(Rsdp::parse(&rsdp_v1(0)[..10]), Err(AcpiError::TooShort));
    }

    #[test]
    fn rsdp_v2_extended_checksum_is_enforced() {
        let mut bytes = rsdp_v2(0x1000, 0x2000);
        bytes[30] ^= 0xff; // corrupt the xsdt address, first checksum still fine
        assert_eq!(Rsdp::parse(&bytes), Err(AcpiError::BadChecksum));
    }

    #[test]
    fn rsdp_v2_rejects_bad_length_field() {
        let mut bytes = rsdp_v2(0x1000, 0x2000);
        bytes[20..24].copy_from_slice(&1000u32.to_le_bytes()); // longer than the slice
        fix_checksum_over(&mut bytes, 8, RSDP_V1_LEN);
        assert_eq!(Rsdp::parse(&bytes), Err(AcpiError::BadLength));
    }

    #[test]
    fn sdt_checks_length_and_checksum() {
        let good = sdt(b"TEST", &[1, 2, 3]);
        assert!(Sdt::parse(&good).is_ok());

        let mut corrupt = good.clone();
        corrupt[SDT_HEADER_LEN] ^= 1;
        assert_eq!(Sdt::parse(&corrupt).unwrap_err(), AcpiError::BadChecksum);

        assert_eq!(
            Sdt::parse(&good[..good.len() - 1]).unwrap_err(),
            AcpiError::BadLength
        );
        assert_eq!(Sdt::parse(&good[..10]).unwrap_err(), AcpiError::TooShort);
    }

    #[test]
    fn sdt_length_smaller_than_header_is_rejected() {
        let mut bytes = sdt(b"TEST", &[]);
        bytes[4..8].copy_from_slice(&10u32.to_le_bytes());
        assert_eq!(Sdt::parse(&bytes).unwrap_err(), AcpiError::BadLength);
    }

    #[test]
    fn rsdt_and_xsdt_entries_are_listed() {
        let mut rsdt_body = Vec::new();
        rsdt_body.extend_from_slice(&0x1000u32.to_le_bytes());
        rsdt_body.extend_from_slice(&0x2000u32.to_le_bytes());
        rsdt_body.extend_from_slice(&[0xff, 0xff]); // trailing partial entry
        let rsdt = sdt(b"RSDT", &rsdt_body);
        let parsed = Sdt::parse(&rsdt).unwrap();
        assert_eq!(parsed.entries(false).collect::<Vec<_>>(), [0x1000, 0x2000]);

        let mut xsdt_body = Vec::new();
        xsdt_body.extend_from_slice(&0x1_0000_0000u64.to_le_bytes());
        let xsdt = sdt(b"XSDT", &xsdt_body);
        let parsed = Sdt::parse(&xsdt).unwrap();
        assert_eq!(parsed.entries(true).collect::<Vec<_>>(), [0x1_0000_0000]);
    }

    #[test]
    fn madt_counts_cpus_and_finds_io_apics_and_overrides() {
        let entries = [
            local_apic(0, 1),
            local_apic(1, 1),
            local_apic(2, 0), // disabled and not online capable: not counted
            local_apic(3, 2), // online capable: counted
            io_apic(4, 0xfec0_0000, 0),
            irq_override(0, 2, 0),
            irq_override(9, 9, 0xd),
        ];
        let table = sdt(b"APIC", &madt_body(0xfee0_0000, 1, &entries));
        let madt = Madt::parse(&Sdt::parse(&table).unwrap()).unwrap();
        assert_eq!(madt.local_apic_address, 0xfee0_0000);
        assert_eq!(madt.flags, 1);
        assert_eq!(madt.cpu_count, 3);
        assert_eq!(
            madt.io_apics(),
            [IoApic {
                id: 4,
                address: 0xfec0_0000,
                gsi_base: 0
            }]
        );
        assert_eq!(madt.overrides().len(), 2);
        assert_eq!(madt.overrides()[1].source, 9);
        assert_eq!(madt.overrides()[1].flags, 0xd);
    }

    #[test]
    fn madt_address_override_replaces_the_32_bit_address() {
        let mut over = Vec::from([5u8, 12, 0, 0]);
        over.extend_from_slice(&0x1_fee0_0000u64.to_le_bytes());
        let table = sdt(b"APIC", &madt_body(0xfee0_0000, 0, &[over]));
        let madt = Madt::parse(&Sdt::parse(&table).unwrap()).unwrap();
        assert_eq!(madt.local_apic_address, 0x1_fee0_0000);
    }

    #[test]
    fn madt_counts_x2apic_cpus() {
        let mut e = Vec::from([9u8, 16, 0, 0]);
        e.extend_from_slice(&7u32.to_le_bytes()); // x2apic id
        e.extend_from_slice(&1u32.to_le_bytes()); // flags: enabled
        e.extend_from_slice(&0u32.to_le_bytes()); // acpi uid
        let table = sdt(b"APIC", &madt_body(0xfee0_0000, 0, &[e]));
        let madt = Madt::parse(&Sdt::parse(&table).unwrap()).unwrap();
        assert_eq!(madt.cpu_count, 1);
    }

    #[test]
    fn madt_zero_length_entry_is_an_error_not_an_infinite_loop() {
        let table = sdt(
            b"APIC",
            &madt_body(0xfee0_0000, 0, &[Vec::from([0u8, 0, 0, 0])]),
        );
        let sdt = Sdt::parse(&table).unwrap();
        assert_eq!(Madt::parse(&sdt).unwrap_err(), AcpiError::BadLength);
    }

    #[test]
    fn madt_entry_running_past_the_table_is_an_error() {
        // Declares 12 bytes but only 4 are present.
        let truncated = Vec::from([1u8, 12, 0, 0]);
        let table = sdt(b"APIC", &madt_body(0xfee0_0000, 0, &[truncated]));
        let sdt = Sdt::parse(&table).unwrap();
        assert_eq!(Madt::parse(&sdt).unwrap_err(), AcpiError::BadLength);
    }

    #[test]
    fn madt_entry_shorter_than_its_type_requires_is_an_error() {
        // An I/O APIC entry that claims to be only 6 bytes long.
        let short = Vec::from([1u8, 6, 0, 0, 0, 0]);
        let table = sdt(b"APIC", &madt_body(0xfee0_0000, 0, &[short]));
        let sdt = Sdt::parse(&table).unwrap();
        assert_eq!(Madt::parse(&sdt).unwrap_err(), AcpiError::BadLength);
    }

    #[test]
    fn madt_rejects_wrong_signature() {
        let table = sdt(b"FACP", &madt_body(0, 0, &[]));
        let sdt = Sdt::parse(&table).unwrap();
        assert_eq!(Madt::parse(&sdt).unwrap_err(), AcpiError::BadSignature);
    }

    #[test]
    fn madt_unknown_entry_types_are_skipped() {
        let unknown = Vec::from([0x7fu8, 4, 0xaa, 0xbb]);
        let table = sdt(
            b"APIC",
            &madt_body(0xfee0_0000, 0, &[unknown, local_apic(0, 1)]),
        );
        let madt = Madt::parse(&Sdt::parse(&table).unwrap()).unwrap();
        assert_eq!(madt.cpu_count, 1);
    }

    #[test]
    fn madt_capacity_limits_are_enforced_without_panicking() {
        let entries: Vec<Vec<u8>> = (0..(MAX_IO_APICS + 5))
            .map(|i| io_apic(i as u8, 0xfec0_0000 + (i as u32) * 0x1000, i as u32 * 24))
            .collect();
        let table = sdt(b"APIC", &madt_body(0xfee0_0000, 0, &entries));
        let madt = Madt::parse(&Sdt::parse(&table).unwrap()).unwrap();
        assert_eq!(madt.io_apics().len(), MAX_IO_APICS);
    }
}
