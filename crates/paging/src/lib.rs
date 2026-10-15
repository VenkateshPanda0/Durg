//! # x86-64 4-level paging: entries, address splitting, and translation
//!
//! This crate is the pure half of Durg's paging code. It describes page table
//! entries, splits virtual addresses into table indices, and walks a table
//! hierarchy, but it never touches memory itself. The walk asks the caller for
//! each entry through a closure, so:
//!
//! - the kernel passes a reader that goes through the higher-half direct map,
//! - host unit tests pass a reader over a fake in-memory table tree.
//!
//! There is no `unsafe` code here (`#![forbid(unsafe_code)]`).
//!
//! References: Intel SDM Vol. 3 ch. 4 ("Paging"), in particular 4.5 (4-level
//! paging), table 4-15 to 4-19 (entry formats) and 4.5.4 (linear address
//! translation).

#![no_std]
#![forbid(unsafe_code)]

/// Bytes in a 4 KiB page.
pub const PAGE_SIZE: u64 = 4096;
/// Entries in one page table at any level.
pub const ENTRY_COUNT: usize = 512;
/// Bytes in a 2 MiB huge page.
pub const HUGE_2M: u64 = 2 * 1024 * 1024;
/// Bytes in a 1 GiB huge page.
pub const HUGE_1G: u64 = 1024 * 1024 * 1024;

/// Bits 12 to 51 of an entry hold the physical address (the architectural
/// maximum of 52 physical address bits).
pub const ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

/// Entry flag bits (Intel SDM Vol. 3 table 4-19 and neighbours).
pub mod flags {
    pub const PRESENT: u64 = 1 << 0;
    pub const WRITABLE: u64 = 1 << 1;
    pub const USER: u64 = 1 << 2;
    /// With `CACHE_DISABLE`, selects PAT entry 3 (uncacheable under the layout
    /// the bootloader guarantees) for a 4 KiB page.
    pub const WRITE_THROUGH: u64 = 1 << 3;
    pub const CACHE_DISABLE: u64 = 1 << 4;
    pub const ACCESSED: u64 = 1 << 5;
    pub const DIRTY: u64 = 1 << 6;
    /// In a PDPT or PD entry: this entry maps a 1 GiB or 2 MiB page directly
    /// instead of pointing at a lower table.
    pub const HUGE: u64 = 1 << 7;
    pub const GLOBAL: u64 = 1 << 8;
    /// Requires `EFER.NXE`. Setting it without that is a reserved-bit fault.
    pub const NO_EXECUTE: u64 = 1 << 63;
}

/// Why an entry or address could not be built or walked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagingError {
    /// A physical address is not 4 KiB aligned.
    UnalignedAddress,
    /// A physical address does not fit in 52 bits.
    AddressTooLarge,
    /// The flag bits given overlap the address field.
    FlagsOverlapAddress,
    /// A virtual address is not canonical (bits 63:48 do not repeat bit 47).
    NotCanonical,
}

/// `true` if `address` is a canonical 48-bit virtual address: bits 63 to 47 are
/// all equal.
pub fn is_canonical(address: u64) -> bool {
    let top = address >> 47;
    top == 0 || top == 0x1_ffff
}

/// One page table entry.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry(u64);

impl Entry {
    /// An empty (not present) entry.
    pub const EMPTY: Entry = Entry(0);

    /// Wraps a raw entry value read from memory, with no validation.
    pub const fn from_raw(raw: u64) -> Entry {
        Entry(raw)
    }

    /// The raw 64-bit value.
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Builds an entry pointing at `physical` with the given `flags`.
    ///
    /// `physical` must be 4 KiB aligned and fit in 52 bits, and `flags` must not
    /// touch the address bits.
    pub fn new(physical: u64, flags: u64) -> Result<Entry, PagingError> {
        if physical & (PAGE_SIZE - 1) != 0 {
            return Err(PagingError::UnalignedAddress);
        }
        if physical & !ADDRESS_MASK != 0 {
            return Err(PagingError::AddressTooLarge);
        }
        if flags & ADDRESS_MASK != 0 {
            return Err(PagingError::FlagsOverlapAddress);
        }
        Ok(Entry(physical | flags))
    }

    pub const fn is_present(self) -> bool {
        self.0 & flags::PRESENT != 0
    }

    pub const fn is_huge(self) -> bool {
        self.0 & flags::HUGE != 0
    }

    /// `true` if every bit of `mask` is set.
    pub const fn has(self, mask: u64) -> bool {
        self.0 & mask == mask
    }

    /// The physical address field (bits 12 to 51).
    pub const fn address(self) -> u64 {
        self.0 & ADDRESS_MASK
    }
}

/// The four table indices and the page offset of a virtual address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Indices {
    /// Level 4 (PML4), bits 47:39.
    pub pml4: usize,
    /// Level 3 (PDPT), bits 38:30.
    pub pdpt: usize,
    /// Level 2 (PD), bits 29:21.
    pub pd: usize,
    /// Level 1 (PT), bits 20:12.
    pub pt: usize,
    /// Byte offset within a 4 KiB page, bits 11:0.
    pub offset: u64,
}

/// Splits a canonical virtual address into its table indices.
pub fn split(address: u64) -> Result<Indices, PagingError> {
    if !is_canonical(address) {
        return Err(PagingError::NotCanonical);
    }
    Ok(Indices {
        pml4: ((address >> 39) & 0x1ff) as usize,
        pdpt: ((address >> 30) & 0x1ff) as usize,
        pd: ((address >> 21) & 0x1ff) as usize,
        pt: ((address >> 12) & 0x1ff) as usize,
        offset: address & (PAGE_SIZE - 1),
    })
}

/// Size of the page a translation ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageSize {
    Size4K,
    Size2M,
    Size1G,
}

/// The result of a successful walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapping {
    /// Physical address that the virtual address translates to (page base plus
    /// the offset within the page).
    pub physical: u64,
    pub page_size: PageSize,
    /// The leaf entry (the entry that maps the page itself).
    pub entry: Entry,
}

/// Why a walk failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalkError {
    /// The address is not canonical.
    NotCanonical,
    /// No mapping: an entry at `level` (4 = PML4 down to 1 = PT) is not present.
    NotMapped { level: u8 },
    /// The reader could not supply the table at `level`, so the walk could not
    /// continue (for example the table is not in memory the kernel can reach).
    Unreadable { level: u8 },
    /// An entry is malformed: the huge bit set where it is reserved (PML4), or
    /// a huge page whose address is not aligned to its size.
    Malformed { level: u8 },
}

/// Walks the table hierarchy rooted at `root` (the physical address of the
/// PML4) to translate `address`.
///
/// `read(table_physical, index)` must return the entry at `index` of the table
/// at `table_physical`, or `None` if that table cannot be read.
pub fn walk<F>(root: u64, address: u64, mut read: F) -> Result<Mapping, WalkError>
where
    F: FnMut(u64, usize) -> Option<Entry>,
{
    let indices = split(address).map_err(|_| WalkError::NotCanonical)?;

    // Level 4: the PML4. A huge page is not allowed here.
    let pml4 = read(root, indices.pml4).ok_or(WalkError::Unreadable { level: 4 })?;
    if !pml4.is_present() {
        return Err(WalkError::NotMapped { level: 4 });
    }
    if pml4.is_huge() {
        return Err(WalkError::Malformed { level: 4 });
    }

    // Level 3: the PDPT. The huge bit means a 1 GiB page.
    let pdpt = read(pml4.address(), indices.pdpt).ok_or(WalkError::Unreadable { level: 3 })?;
    if !pdpt.is_present() {
        return Err(WalkError::NotMapped { level: 3 });
    }
    if pdpt.is_huge() {
        return leaf(pdpt, address, HUGE_1G, PageSize::Size1G, 3);
    }

    // Level 2: the PD. The huge bit means a 2 MiB page.
    let pd = read(pdpt.address(), indices.pd).ok_or(WalkError::Unreadable { level: 2 })?;
    if !pd.is_present() {
        return Err(WalkError::NotMapped { level: 2 });
    }
    if pd.is_huge() {
        return leaf(pd, address, HUGE_2M, PageSize::Size2M, 2);
    }

    // Level 1: the PT, a 4 KiB page.
    let pt = read(pd.address(), indices.pt).ok_or(WalkError::Unreadable { level: 1 })?;
    if !pt.is_present() {
        return Err(WalkError::NotMapped { level: 1 });
    }
    Ok(Mapping {
        physical: pt.address() | indices.offset,
        page_size: PageSize::Size4K,
        entry: pt,
    })
}

/// Finishes a walk at a huge-page leaf.
fn leaf(
    entry: Entry,
    address: u64,
    size: u64,
    page_size: PageSize,
    level: u8,
) -> Result<Mapping, WalkError> {
    // The address field of a huge entry must be aligned to the page size. The
    // low bits of the field would otherwise overlap the in-page offset.
    if entry.address() & (size - 1) != 0 {
        return Err(WalkError::Malformed { level });
    }
    Ok(Mapping {
        physical: entry.address() | (address & (size - 1)),
        page_size,
        entry,
    })
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A fake memory of page tables: (table physical address, index) -> entry.
    #[derive(Default)]
    struct Tables(BTreeMap<(u64, usize), Entry>);

    impl Tables {
        fn set(&mut self, table: u64, index: usize, entry: Entry) {
            self.0.insert((table, index), entry);
        }
        fn read(&self) -> impl FnMut(u64, usize) -> Option<Entry> + '_ {
            |table, index| {
                // A table "exists" if any entry in it was set. Unset entries in an
                // existing table read as empty, like zeroed memory.
                if self.0.keys().any(|(t, _)| *t == table) {
                    Some(self.0.get(&(table, index)).copied().unwrap_or(Entry::EMPTY))
                } else {
                    None
                }
            }
        }
    }

    fn table_entry(phys: u64) -> Entry {
        Entry::new(phys, flags::PRESENT | flags::WRITABLE).unwrap()
    }

    const ROOT: u64 = 0x1000;
    const PDPT: u64 = 0x2000;
    const PD: u64 = 0x3000;
    const PT: u64 = 0x4000;

    /// Maps virtual 0xffff_ff00_0000_0000 + 0x5000 to physical 0xfee0_0000 with 4 KiB pages.
    fn four_k_tree() -> (Tables, u64) {
        let virt = 0xffff_ff00_0000_5000u64;
        let idx = split(virt).unwrap();
        let mut t = Tables::default();
        t.set(ROOT, idx.pml4, table_entry(PDPT));
        t.set(PDPT, idx.pdpt, table_entry(PD));
        t.set(PD, idx.pd, table_entry(PT));
        t.set(
            PT,
            idx.pt,
            Entry::new(
                0xfee0_0000,
                flags::PRESENT | flags::WRITABLE | flags::CACHE_DISABLE | flags::NO_EXECUTE,
            )
            .unwrap(),
        );
        (t, virt)
    }

    #[test]
    fn canonical_addresses() {
        assert!(is_canonical(0));
        assert!(is_canonical(0x0000_7fff_ffff_ffff));
        assert!(is_canonical(0xffff_8000_0000_0000));
        assert!(is_canonical(0xffff_ffff_8000_0000));
        assert!(!is_canonical(0x0000_8000_0000_0000));
        assert!(!is_canonical(0xffff_7fff_ffff_ffff));
        assert!(!is_canonical(0x1234_0000_0000_0000));
    }

    #[test]
    fn split_matches_known_kernel_address() {
        // The kernel is linked at -2 GiB: PML4 511, PDPT 510, PD 0, PT 0.
        let i = split(0xffff_ffff_8000_0000).unwrap();
        assert_eq!((i.pml4, i.pdpt, i.pd, i.pt, i.offset), (511, 510, 0, 0, 0));
    }

    #[test]
    fn split_keeps_the_page_offset_and_every_index() {
        // 1 + (2<<12) + (3<<21) + (4<<30) + (5<<39), plus offset 0x123.
        let address = (5u64 << 39) | (4 << 30) | (3 << 21) | (2 << 12) | 0x123;
        let i = split(address).unwrap();
        assert_eq!((i.pml4, i.pdpt, i.pd, i.pt, i.offset), (5, 4, 3, 2, 0x123));
    }

    #[test]
    fn split_rejects_non_canonical() {
        assert_eq!(split(0x0001_0000_0000_0000), Err(PagingError::NotCanonical));
    }

    #[test]
    fn entry_new_validates_its_inputs() {
        assert!(Entry::new(0x1000, flags::PRESENT).is_ok());
        assert_eq!(
            Entry::new(0x1001, flags::PRESENT),
            Err(PagingError::UnalignedAddress)
        );
        assert_eq!(
            Entry::new(1 << 52, flags::PRESENT),
            Err(PagingError::AddressTooLarge)
        );
        assert_eq!(
            Entry::new(0x1000, 0x2000),
            Err(PagingError::FlagsOverlapAddress)
        );
    }

    #[test]
    fn entry_accessors() {
        let e = Entry::new(0xabc_d000, flags::PRESENT | flags::HUGE | flags::NO_EXECUTE).unwrap();
        assert!(e.is_present() && e.is_huge());
        assert!(e.has(flags::NO_EXECUTE));
        assert!(!e.has(flags::WRITABLE));
        assert_eq!(e.address(), 0xabc_d000);
        assert!(!Entry::EMPTY.is_present());
    }

    #[test]
    fn walk_translates_a_4k_page_and_keeps_the_offset() {
        let (t, virt) = four_k_tree();
        let m = walk(ROOT, virt + 0x123, t.read()).unwrap();
        assert_eq!(m.physical, 0xfee0_0123);
        assert_eq!(m.page_size, PageSize::Size4K);
        assert!(m.entry.has(flags::CACHE_DISABLE));
        assert!(m.entry.has(flags::NO_EXECUTE));
    }

    #[test]
    fn walk_translates_2m_and_1g_pages() {
        let mut t = Tables::default();
        // 1 GiB page at PML4[0] / PDPT[1] -> physical 0x4000_0000.
        t.set(ROOT, 0, table_entry(PDPT));
        t.set(
            PDPT,
            1,
            Entry::new(0x4000_0000, flags::PRESENT | flags::HUGE).unwrap(),
        );
        let m = walk(ROOT, (1 << 30) + 0x12_3456, t.read()).unwrap();
        assert_eq!(m.physical, 0x4000_0000 + 0x12_3456);
        assert_eq!(m.page_size, PageSize::Size1G);

        // 2 MiB page at PD[3] under PDPT[2].
        t.set(PDPT, 2, table_entry(PD));
        t.set(
            PD,
            3,
            Entry::new(0x2000_0000, flags::PRESENT | flags::HUGE).unwrap(),
        );
        let m = walk(ROOT, (2 << 30) + (3 << 21) + 0x1_2345, t.read()).unwrap();
        assert_eq!(m.physical, 0x2000_0000 + 0x1_2345);
        assert_eq!(m.page_size, PageSize::Size2M);
    }

    #[test]
    fn walk_reports_the_level_that_was_not_present() {
        let (t, virt) = four_k_tree();
        // Same PML4/PDPT/PD, different PT slot: not present at level 1.
        assert_eq!(
            walk(ROOT, virt + 0x1000, t.read()),
            Err(WalkError::NotMapped { level: 1 })
        );
        // A different PD slot: not present at level 2.
        assert_eq!(
            walk(ROOT, virt + (1 << 21), t.read()),
            Err(WalkError::NotMapped { level: 2 })
        );
        // A different PDPT slot: not present at level 3.
        assert_eq!(
            walk(ROOT, virt + (1 << 30), t.read()),
            Err(WalkError::NotMapped { level: 3 })
        );
        // A different PML4 slot: not present at level 4.
        assert_eq!(
            walk(ROOT, virt ^ (1 << 39), t.read()),
            Err(WalkError::NotMapped { level: 4 })
        );
    }

    #[test]
    fn walk_reports_unreadable_tables_instead_of_guessing() {
        let (t, virt) = four_k_tree();
        // Point the PD entry at a table the reader does not have.
        let idx = split(virt).unwrap();
        let mut broken = Tables(t.0.clone());
        broken.set(PDPT, idx.pdpt, table_entry(0x9000));
        assert_eq!(
            walk(ROOT, virt, broken.read()),
            Err(WalkError::Unreadable { level: 2 })
        );
    }

    #[test]
    fn walk_rejects_non_canonical_and_malformed_entries() {
        let (t, _) = four_k_tree();
        assert_eq!(
            walk(ROOT, 0x0001_0000_0000_0000, t.read()),
            Err(WalkError::NotCanonical)
        );

        // Huge bit on a PML4 entry is reserved.
        let mut m = Tables::default();
        m.set(
            ROOT,
            0,
            Entry::new(PDPT, flags::PRESENT | flags::HUGE).unwrap(),
        );
        assert_eq!(
            walk(ROOT, 0, m.read()),
            Err(WalkError::Malformed { level: 4 })
        );

        // A 2 MiB entry whose address is only 4 KiB aligned.
        let mut m = Tables::default();
        m.set(ROOT, 0, table_entry(PDPT));
        m.set(PDPT, 0, table_entry(PD));
        m.set(
            PD,
            0,
            Entry::new(0x20_1000, flags::PRESENT | flags::HUGE).unwrap(),
        );
        assert_eq!(
            walk(ROOT, 0, m.read()),
            Err(WalkError::Malformed { level: 2 })
        );
    }
}
