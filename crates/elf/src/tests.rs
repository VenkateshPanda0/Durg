use super::*;
use proptest::prelude::*;
use std::vec::Vec;

const POLICY: Policy = Policy {
    min_address: 0x1000,
    max_address: 0x0000_7fff_ffe0_0000,
    max_total_bytes: 16 << 20,
};

const R: u32 = 4;
const W: u32 = 2;
const X: u32 = 1;

#[derive(Clone, Debug)]
struct Seg {
    vaddr: u64,
    flags: u32,
    data: Vec<u8>,
    memsz: u64,
}

fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

/// Builds an ELF file: header, program header table, then each segment's bytes.
/// `extra` adds program headers of other types (type, flags).
fn build(segments: &[Seg], entry: u64, extra: &[(u32, u32)]) -> Vec<u8> {
    let count = segments.len() + extra.len();
    let data_start = ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE * count;
    let mut file = std::vec![0u8; data_start];
    file[0..4].copy_from_slice(b"\x7fELF");
    file[4] = 2;
    file[5] = 1;
    file[6] = 1;
    put16(&mut file, 16, ET_EXEC);
    put16(&mut file, 18, EM_X86_64);
    put32(&mut file, 20, 1);
    put64(&mut file, 24, entry);
    put64(&mut file, 32, ELF_HEADER_SIZE as u64);
    put16(&mut file, 52, ELF_HEADER_SIZE as u16);
    put16(&mut file, 54, PROGRAM_HEADER_SIZE as u16);
    put16(&mut file, 56, count as u16);
    let mut offset = data_start;
    for (index, segment) in segments.iter().enumerate() {
        let base = ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE * index;
        put32(&mut file, base, PT_LOAD);
        put32(&mut file, base + 4, segment.flags);
        put64(&mut file, base + 8, offset as u64);
        put64(&mut file, base + 16, segment.vaddr);
        put64(&mut file, base + 24, segment.vaddr);
        put64(&mut file, base + 32, segment.data.len() as u64);
        put64(&mut file, base + 40, segment.memsz);
        put64(&mut file, base + 48, PAGE_SIZE);
        file.extend_from_slice(&segment.data);
        offset += segment.data.len();
    }
    for (index, (kind, flags)) in extra.iter().enumerate() {
        let base = ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE * (segments.len() + index);
        put32(&mut file, base, *kind);
        put32(&mut file, base + 4, *flags);
    }
    file
}

fn text() -> Seg {
    Seg {
        vaddr: 0x40_0000,
        flags: R | X,
        data: std::vec![0x90; 32],
        memsz: 32,
    }
}

fn data() -> Seg {
    Seg {
        vaddr: 0x60_0000,
        flags: R | W,
        data: std::vec![1, 2, 3, 4],
        memsz: 0x2000,
    }
}

fn good() -> Vec<u8> {
    build(&[text(), data()], 0x40_0000, &[])
}

/// Offset of the second program header (the data segment), for the rejection tests.
const PH1: usize = ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE;

/// A change applied to a good file to make it bad.
type Change = fn(&mut Vec<u8>);

#[test]
fn a_good_two_segment_program_parses_exactly() {
    let file = good();
    let image = parse(&file, &POLICY).expect("should parse");
    assert_eq!(image.entry, 0x40_0000);
    let [code, data] = image.segments() else {
        panic!("expected two segments");
    };
    assert_eq!(code.vaddr, 0x40_0000);
    assert_eq!((code.filesz, code.memsz), (32, 32));
    assert_eq!(
        code.perms,
        Perms {
            read: true,
            write: false,
            execute: true
        }
    );
    assert_eq!(data.vaddr, 0x60_0000);
    assert_eq!((data.filesz, data.memsz), (4, 0x2000));
    assert_eq!(data.pages(), 2);
    assert!(data.perms.write && !data.perms.execute);
    // The loader copies `filesz` bytes from `file_offset`: that range is real.
    assert_eq!(
        &file[data.file_offset as usize..(data.file_offset + data.filesz) as usize],
        &[1, 2, 3, 4]
    );
}

#[test]
fn header_checks_each_refuse_the_right_thing() {
    let cases: [(&str, Change, Error); 11] = [
        ("magic", |f| f[0] = 0, Error::BadMagic),
        ("32-bit", |f| f[4] = 1, Error::NotElf64),
        ("big-endian", |f| f[5] = 2, Error::NotLittleEndian),
        ("ident version", |f| f[6] = 2, Error::BadVersion),
        ("version field", |f| put32(f, 20, 2), Error::BadVersion),
        ("shared object", |f| put16(f, 16, 3), Error::NotExecutable),
        ("relocatable", |f| put16(f, 16, 1), Error::NotExecutable),
        ("other machine", |f| put16(f, 18, 0xb7), Error::WrongMachine),
        ("header size", |f| put16(f, 52, 65), Error::BadHeaderSize),
        ("ph size", |f| put16(f, 54, 57), Error::BadHeaderSize),
        ("no headers", |f| put16(f, 56, 0), Error::BadProgramHeaders),
    ];
    for (name, change, expected) in cases {
        let mut file = good();
        change(&mut file);
        assert_eq!(parse(&file, &POLICY), Err(expected), "{name}");
    }
}

#[test]
fn truncated_files_are_refused_not_read() {
    let file = good();
    for length in [0, 1, 16, 63] {
        assert_eq!(parse(&file[..length], &POLICY), Err(Error::TooShort));
    }
    // The header is intact but the program header table runs past the end.
    assert_eq!(
        parse(&file[..ELF_HEADER_SIZE + 10], &POLICY),
        Err(Error::BadProgramHeaders)
    );
}

#[test]
fn program_header_table_must_lie_inside_the_file() {
    let mut file = good();
    let length = file.len() as u64;
    put64(&mut file, 32, length - 8);
    assert_eq!(parse(&file, &POLICY), Err(Error::BadProgramHeaders));
    let mut file = good();
    put64(&mut file, 32, u64::MAX - 10); // offset + size overflows
    assert_eq!(parse(&file, &POLICY), Err(Error::BadProgramHeaders));
    let mut file = good();
    put16(&mut file, 56, 65); // more than the parser will read
    assert_eq!(parse(&file, &POLICY), Err(Error::BadProgramHeaders));
}

#[test]
fn segment_checks_each_refuse_the_right_thing() {
    // Changes to the data segment (program header 1) of a good file.
    let cases: [(&str, Change, Error); 12] = [
        ("empty", |f| put64(f, PH1 + 40, 0), Error::EmptySegment),
        (
            "file larger than memory",
            |f| put64(f, PH1 + 32, 0x3000),
            Error::SegmentFileLargerThanMemory,
        ),
        (
            "file range past the end",
            |f| put64(f, PH1 + 8, 1 << 40),
            Error::SegmentFileRange,
        ),
        (
            "file range wraps",
            |f| put64(f, PH1 + 8, u64::MAX),
            Error::SegmentFileRange,
        ),
        (
            "unaligned",
            |f| put64(f, PH1 + 16, 0x60_0008),
            Error::SegmentUnaligned,
        ),
        (
            "null page",
            |f| put64(f, PH1 + 16, 0),
            Error::SegmentOutOfRange,
        ),
        (
            "kernel half",
            |f| put64(f, PH1 + 16, 0xffff_ffff_8000_0000),
            Error::SegmentOutOfRange,
        ),
        (
            "top of user space",
            |f| put64(f, PH1 + 16, 0x0000_7fff_ffe0_0000),
            Error::SegmentOutOfRange,
        ),
        (
            "end wraps",
            |f| put64(f, PH1 + 40, u64::MAX),
            Error::SegmentOutOfRange,
        ),
        (
            "unknown flag bits",
            |f| put32(f, PH1 + 4, R | W | 0x10),
            Error::BadFlags,
        ),
        (
            "no permissions",
            |f| put32(f, PH1 + 4, 0),
            Error::NoPermissions,
        ),
        (
            "writable and executable",
            |f| put32(f, PH1 + 4, R | W | X),
            Error::WritableAndExecutable,
        ),
    ];
    for (name, change, expected) in cases {
        let mut file = good();
        change(&mut file);
        assert_eq!(parse(&file, &POLICY), Err(expected), "{name}");
    }
}

#[test]
fn the_end_of_the_file_is_an_exact_boundary() {
    // The data segment's last file byte is the file's last byte: fine. Cut one
    // byte off the file and its range reaches one past the end: refused.
    let file = good();
    assert!(parse(&file, &POLICY).is_ok());
    assert_eq!(
        parse(&file[..file.len() - 1], &POLICY),
        Err(Error::SegmentFileRange)
    );
}

#[test]
fn the_end_of_the_program_header_table_is_an_exact_boundary() {
    // Make the table the last thing in the file: the code segment takes its bytes
    // from the ELF header itself (offset 0), so nothing follows the table. Then
    // cut one byte off the table. (The last 8 bytes of a header, the alignment
    // field, are never read, so only an explicit bound can notice.)
    let mut file = build(&[text()], 0x40_0000, &[]);
    put64(&mut file, ELF_HEADER_SIZE + 8, 0); // p_offset
    put64(&mut file, ELF_HEADER_SIZE + 32, 32); // p_filesz, inside the header
    let table_only = ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE;
    assert!(parse(&file[..table_only], &POLICY).is_ok());
    assert_eq!(
        parse(&file[..table_only - 1], &POLICY),
        Err(Error::BadProgramHeaders)
    );
}

#[test]
fn the_lowest_allowed_address_is_an_exact_boundary() {
    let mut at_minimum = text();
    at_minimum.vaddr = POLICY.min_address;
    assert!(parse(&build(&[at_minimum], POLICY.min_address, &[]), &POLICY).is_ok());
    // One page below it is refused.
    let mut below = text();
    below.vaddr = POLICY.min_address - PAGE_SIZE;
    assert_eq!(
        parse(
            &build(&[below], POLICY.min_address - PAGE_SIZE, &[]),
            &POLICY
        ),
        Err(Error::SegmentOutOfRange)
    );
}

#[test]
fn a_huge_program_header_table_is_refused_even_when_it_fits_in_the_file() {
    // 65 program headers (the parser reads at most 64) in a file that really
    // contains them all: the cap bounds the work, not just the file size.
    let extra = std::vec![(PT_NULL, 0); 65];
    assert_eq!(
        parse(&build(&[text()], 0x40_0000, &extra), &POLICY),
        Err(Error::BadProgramHeaders)
    );
    // 64 is the most accepted: one load segment and 63 harmless headers.
    let extra = std::vec![(PT_NULL, 0); 63];
    assert!(parse(&build(&[text()], 0x40_0000, &extra), &POLICY).is_ok());
}

#[test]
fn a_segment_that_runs_into_the_stack_is_out_of_range() {
    // Ends exactly at the policy limit: fine. One byte over: refused.
    let mut fits = data();
    fits.vaddr = POLICY.max_address - 0x2000;
    assert!(parse(&build(&[text(), fits], 0x40_0000, &[]), &POLICY).is_ok());
    let mut over = data();
    over.vaddr = POLICY.max_address - 0x2000;
    over.memsz = 0x2001;
    assert_eq!(
        parse(&build(&[text(), over], 0x40_0000, &[]), &POLICY),
        Err(Error::SegmentOutOfRange)
    );
}

#[test]
fn segments_may_not_share_a_page() {
    // data starts in the page the code's memory ends in.
    let mut code = text();
    code.memsz = 0x1800;
    let mut overlapping = data();
    overlapping.vaddr = 0x40_1000;
    assert_eq!(
        parse(
            &build(&[code.clone(), overlapping], 0x40_0000, &[]),
            &POLICY
        ),
        Err(Error::SegmentsOverlap)
    );
    // The next page along is fine, and so is the order the file lists them in.
    let mut next = data();
    next.vaddr = 0x40_2000;
    assert!(parse(
        &build(&[code.clone(), next.clone()], 0x40_0000, &[]),
        &POLICY
    )
    .is_ok());
    assert!(parse(&build(&[next, code], 0x40_0000, &[]), &POLICY).is_ok());
}

#[test]
fn the_entry_point_must_be_inside_executable_file_backed_bytes() {
    for (name, entry) in [
        ("zero", 0u64),
        ("before the code", 0x3f_ffff),
        ("one past the file bytes", 0x40_0020),
        ("in the data segment", 0x60_0000),
        ("in the kernel half", 0xffff_ffff_8000_0000),
    ] {
        assert_eq!(
            parse(&build(&[text(), data()], entry, &[]), &POLICY),
            Err(Error::EntryNotExecutable),
            "{name}"
        );
    }
    // The last byte is fine.
    assert!(parse(&build(&[text(), data()], 0x40_001f, &[]), &POLICY).is_ok());
}

#[test]
fn memory_is_bounded() {
    let mut huge = data();
    huge.memsz = 17 << 20;
    assert_eq!(
        parse(&build(&[text(), huge], 0x40_0000, &[]), &POLICY),
        Err(Error::TooMuchMemory)
    );
    // The sum counts, not just each segment.
    let policy = Policy {
        max_total_bytes: 0x2000 + 0x1000,
        ..POLICY
    };
    assert!(parse(&good(), &policy).is_ok());
    let policy = Policy {
        max_total_bytes: 0x2000 + 0x1000 - 1,
        ..POLICY
    };
    assert_eq!(parse(&good(), &policy), Err(Error::TooMuchMemory));
}

#[test]
fn too_many_segments_are_refused() {
    let many: Vec<Seg> = (0..=MAX_SEGMENTS as u64)
        .map(|i| Seg {
            vaddr: 0x40_0000 + i * 0x1000,
            flags: R | X,
            data: std::vec![0x90; 8],
            memsz: 8,
        })
        .collect();
    assert_eq!(
        parse(&build(&many, 0x40_0000, &[]), &POLICY),
        Err(Error::TooManySegments)
    );
    assert!(parse(&build(&many[..MAX_SEGMENTS], 0x40_0000, &[]), &POLICY).is_ok());
}

#[test]
fn features_the_loader_does_not_have_are_refused_not_ignored() {
    for (name, kind) in [
        ("interpreter", 3u32),
        ("dynamic", 2),
        ("shared library", 5),
        ("thread-local storage", 7),
        ("unknown", 0x1234_5678),
    ] {
        assert_eq!(
            parse(&build(&[text(), data()], 0x40_0000, &[(kind, R)]), &POLICY),
            Err(Error::UnsupportedSegment(kind)),
            "{name}"
        );
    }
}

#[test]
fn an_executable_stack_is_refused_and_a_plain_one_is_fine() {
    assert_eq!(
        parse(
            &build(&[text()], 0x40_0000, &[(PT_GNU_STACK, R | W | X)]),
            &POLICY
        ),
        Err(Error::ExecutableStack)
    );
    assert!(parse(
        &build(&[text()], 0x40_0000, &[(PT_GNU_STACK, R | W)]),
        &POLICY
    )
    .is_ok());
}

#[test]
fn harmless_header_types_are_skipped() {
    let extra = [
        (PT_NULL, 0),
        (PT_NOTE, R),
        (PT_PHDR, R),
        (PT_GNU_EH_FRAME, R),
        (PT_GNU_RELRO, R),
        (PT_GNU_PROPERTY, R),
    ];
    assert!(parse(&build(&[text(), data()], 0x40_0000, &extra), &POLICY).is_ok());
}

#[test]
fn a_file_with_no_loadable_segment_is_refused() {
    assert_eq!(
        parse(&build(&[], 0x40_0000, &[(PT_NOTE, R)]), &POLICY),
        Err(Error::NoLoadSegments)
    );
}

// ---- properties -------------------------------------------------------------

/// What every accepted image must satisfy, whatever the file looked like. These
/// are the loader's assumptions.
fn check_invariants(image: &Image, file_len: usize, policy: &Policy) {
    let segments = image.segments();
    assert!(!segments.is_empty() && segments.len() <= MAX_SEGMENTS);
    let mut total = 0u64;
    for (index, s) in segments.iter().enumerate() {
        assert_eq!(s.vaddr % PAGE_SIZE, 0, "unaligned segment");
        assert!(s.memsz > 0);
        assert!(s.filesz <= s.memsz);
        assert!(s.file_offset.checked_add(s.filesz).unwrap() <= file_len as u64);
        assert!(s.vaddr >= policy.min_address);
        let page_end = s.vaddr.checked_add(s.pages() * PAGE_SIZE).unwrap();
        assert!(page_end <= policy.max_address);
        assert!(s.perms.read || s.perms.write || s.perms.execute);
        assert!(!(s.perms.write && s.perms.execute), "W^X violated");
        total += s.pages() * PAGE_SIZE;
        for other in &segments[index + 1..] {
            let other_end = other.vaddr + other.pages() * PAGE_SIZE;
            assert!(page_end <= other.vaddr || other_end <= s.vaddr, "overlap");
        }
    }
    assert!(total <= policy.max_total_bytes);
    assert!(
        segments
            .iter()
            .any(|s| s.perms.execute && image.entry >= s.vaddr && image.entry - s.vaddr < s.filesz),
        "entry is not in executable file-backed bytes"
    );
}

fn arbitrary_segment() -> impl Strategy<Value = Seg> {
    (
        prop_oneof![
            (0u64..0x2000).prop_map(|p| p * 0x1000),
            any::<u64>().prop_map(|v| v & !0xfff),
            any::<u64>(),
        ],
        0u32..16,
        proptest::collection::vec(any::<u8>(), 0..64),
        prop_oneof![0u64..0x4000, any::<u64>()],
    )
        .prop_map(|(vaddr, flags, data, memsz)| Seg {
            vaddr,
            flags,
            data,
            memsz,
        })
}

proptest! {
    // The parser sees only hostile bytes in real use: it must never panic.
    #[test]
    fn arbitrary_bytes_never_panic_and_accepted_images_are_sound(
        file in proptest::collection::vec(any::<u8>(), 0..400),
    ) {
        if let Ok(image) = parse(&file, &POLICY) {
            check_invariants(&image, file.len(), &POLICY);
        }
    }

    // Hostile files are usually *almost* valid. Change one byte of a valid file
    // anywhere and the result must be a refusal or a sound image.
    #[test]
    fn one_changed_byte_in_a_valid_file_is_refused_or_sound(
        position in 0usize..400,
        value: u8,
    ) {
        let mut file = good();
        let position = position % file.len();
        file[position] = value;
        if let Ok(image) = parse(&file, &POLICY) {
            check_invariants(&image, file.len(), &POLICY);
        }
    }

    // Files built from random segment parameters, including wild addresses and
    // sizes.
    #[test]
    fn random_segment_layouts_are_refused_or_sound(
        segments in proptest::collection::vec(arbitrary_segment(), 0..10),
        entry: u64,
        near_entry in any::<bool>(),
    ) {
        let entry = if near_entry {
            segments.first().map_or(entry, |s| s.vaddr)
        } else {
            entry
        };
        let file = build(&segments, entry, &[]);
        if let Ok(image) = parse(&file, &POLICY) {
            check_invariants(&image, file.len(), &POLICY);
        }
    }

    // Valid layouts are accepted: pages apart, sensible permissions, entry at the
    // start of the first executable one. Guards against a parser that refuses
    // everything.
    #[test]
    fn well_formed_programs_are_accepted(
        count in 1usize..=MAX_SEGMENTS,
        sizes in proptest::collection::vec(1u64..0x3000, MAX_SEGMENTS),
    ) {
        let segments: Vec<Seg> = (0..count)
            .map(|i| Seg {
                vaddr: 0x10_0000 + (i as u64) * 0x10_0000,
                flags: if i == 0 { R | X } else { R | W },
                data: std::vec![0xcc; 16],
                memsz: sizes[i].max(16),
            })
            .collect();
        let file = build(&segments, 0x10_0000, &[]);
        let image = parse(&file, &POLICY).expect("a well-formed program was refused");
        prop_assert_eq!(image.segments().len(), count);
        check_invariants(&image, file.len(), &POLICY);
    }
}
