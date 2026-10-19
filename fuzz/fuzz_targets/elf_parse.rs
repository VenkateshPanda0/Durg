//! Fuzzes the ELF parser with hostile files. Whatever the bytes are, `parse`
//! must not panic or loop, and any image it *accepts* must satisfy every
//! property the kernel's loader relies on (page alignment, W^X, range, no
//! overlap, file bounds, entry in executable file-backed bytes, bounded memory).
//!
//! Random bytes almost never form a valid ELF header, so the fuzzer starts from
//! the valid files in `fuzz/seeds/elf_parse/` (see `fuzz/README.md`) and mutates
//! them.

#![no_main]

use durg_elf::{parse, Policy, MAX_SEGMENTS, PAGE_SIZE};
use libfuzzer_sys::fuzz_target;

const POLICY: Policy = Policy {
    min_address: 0x1000,
    max_address: 0x0000_7fff_ffe0_0000,
    max_total_bytes: 16 << 20,
};

fuzz_target!(|file: &[u8]| {
    let Ok(image) = parse(file, &POLICY) else {
        return;
    };
    let segments = image.segments();
    assert!(!segments.is_empty() && segments.len() <= MAX_SEGMENTS);
    let mut total = 0u64;
    for (index, s) in segments.iter().enumerate() {
        assert_eq!(s.vaddr % PAGE_SIZE, 0, "unaligned segment");
        assert!(s.memsz > 0 && s.filesz <= s.memsz);
        let file_end = s
            .file_offset
            .checked_add(s.filesz)
            .expect("file range wraps");
        assert!(file_end <= file.len() as u64, "file range outside the file");
        assert!(s.vaddr >= POLICY.min_address, "below the minimum address");
        let page_end = s
            .vaddr
            .checked_add(s.pages() * PAGE_SIZE)
            .expect("segment end wraps");
        assert!(page_end <= POLICY.max_address, "past the maximum address");
        assert!(
            s.perms.read || s.perms.write || s.perms.execute,
            "segment with no permissions"
        );
        assert!(
            !(s.perms.write && s.perms.execute),
            "writable and executable"
        );
        total += s.pages() * PAGE_SIZE;
        for other in &segments[index + 1..] {
            let other_end = other.vaddr + other.pages() * PAGE_SIZE;
            assert!(
                page_end <= other.vaddr || other_end <= s.vaddr,
                "segments share a page"
            );
        }
    }
    assert!(total <= POLICY.max_total_bytes, "over the memory budget");
    assert!(
        segments
            .iter()
            .any(|s| s.perms.execute && image.entry >= s.vaddr && image.entry - s.vaddr < s.filesz),
        "entry point not in executable file-backed bytes"
    );
});
