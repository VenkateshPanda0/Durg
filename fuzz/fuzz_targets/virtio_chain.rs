//! Fuzz a stable descriptor snapshot against a separate visited-set oracle.
//! This tests parsing and graph rules, not DMA, device concurrency or barriers.
#![no_main]

use durg_virtio::{validate_direct_chain, DESCRIPTOR_BYTES};
use libfuzzer_sys::fuzz_target;

fn oracle(table: &[[u8; DESCRIPTOR_BYTES]], head: u16) -> Option<(u16, u64, u64)> {
    let size = table.len();
    if size == 0 || size > 32768 || !size.is_power_of_two() {
        return None;
    }
    let mut seen = vec![false; size];
    let mut index = usize::from(head);
    let mut count = 0u16;
    let mut total = 0u128;
    let mut writable = 0u128;
    let mut saw_write = false;
    loop {
        if index >= size || seen[index] {
            return None;
        }
        seen[index] = true;
        count += 1;
        let d = &table[index];
        let address = u64::from_le_bytes(d[0..8].try_into().unwrap()) as u128;
        let length = u32::from_le_bytes(d[8..12].try_into().unwrap()) as u128;
        let flags = u16::from_le_bytes(d[12..14].try_into().unwrap());
        let next = u16::from_le_bytes(d[14..16].try_into().unwrap());
        if flags & !3 != 0 || address + length > u64::MAX as u128 {
            return None;
        }
        let writes = flags & 2 != 0;
        if saw_write && !writes {
            return None;
        }
        saw_write |= writes;
        total += length;
        if writes {
            writable += length;
        }
        if total > (1u128 << 32) {
            return None;
        }
        if flags & 1 == 0 {
            return Some((count, total as u64, writable as u64));
        }
        index = usize::from(next);
    }
}

fuzz_target!(|data: &[u8]| {
    let Some(&first) = data.first() else {
        return;
    };
    let sizes = [0usize, 1, 2, 3, 4, 7, 8, 16, 32, 64];
    let size = sizes[usize::from(first) % sizes.len()];
    let mut table = vec![[0u8; DESCRIPTOR_BYTES]; size];
    for (i, descriptor) in table.iter_mut().enumerate() {
        for (j, byte) in descriptor.iter_mut().enumerate() {
            *byte = data[(2 + i * DESCRIPTOR_BYTES + j) % data.len()];
        }
        // Half the inputs exercise plausible flags and links rather than being
        // rejected immediately; the rest retain completely arbitrary bytes.
        if first & 1 == 0 {
            descriptor[12] &= 3;
            descriptor[13] = 0;
            descriptor[14] %= (size + 1) as u8;
            descriptor[15] = 0;
        }
    }
    let head = u16::from(*data.get(1).unwrap_or(&0));
    let expected = oracle(&table, head);
    let actual = validate_direct_chain(&table, head).ok().map(|info| {
        (
            info.descriptors(),
            info.total_bytes(),
            info.writable_bytes(),
        )
    });
    assert_eq!(actual, expected);
});
