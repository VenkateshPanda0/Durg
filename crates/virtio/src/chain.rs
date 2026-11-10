//! Pure validation of direct split-virtqueue descriptor chains.
//!
//! This does not read live DMA memory or synchronize with a device. The caller
//! must supply a stable snapshot and ensure buffers are authorized and mapped.
//! Indirect descriptors are deliberately unsupported until negotiated and
//! implemented. Reference: Virtio 1.2 sections 2.7.4–2.7.5.

use crate::MAX_QUEUE_SIZE;

/// Length of a Virtio split descriptor in bytes.
pub const DESCRIPTOR_BYTES: usize = 16;
const NEXT: u16 = 1;
const WRITE: u16 = 2;
const MAX_CHAIN_BYTES: u64 = 1u64 << 32;

/// Why a direct descriptor chain could not be accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainError {
    /// The snapshot is empty, too large or not a power-of-two queue size.
    InvalidTableSize,
    /// The head or a next index is outside the descriptor table.
    IndexOutOfRange,
    /// The chain requests indirect descriptors or has unknown flags.
    UnsupportedFlags,
    /// A buffer range wraps around the physical address space.
    BufferAddressOverflow,
    /// A device-readable descriptor follows a device-writable descriptor.
    ReadAfterWrite,
    /// The total chain length exceeds the Virtio 2^32-byte limit.
    TooManyBytes,
    /// The chain did not terminate within one queue's worth of descriptors.
    CycleOrTooLong,
}

/// Summary of one validated descriptor chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainInfo {
    descriptors: u16,
    total_bytes: u64,
    writable_bytes: u64,
}

impl ChainInfo {
    /// Number of descriptors in the chain.
    pub const fn descriptors(self) -> u16 {
        self.descriptors
    }
    /// Total length of all buffers, in bytes.
    pub const fn total_bytes(self) -> u64 {
        self.total_bytes
    }
    /// Total bytes made writable to the device.
    pub const fn writable_bytes(self) -> u64 {
        self.writable_bytes
    }
}

/// Checks a direct chain in a stable snapshot of little-endian descriptors.
///
/// Every descriptor is `[addr: le64, len: le32, flags: le16, next: le16]`.
/// The snapshot must contain exactly the negotiated queue size. A descriptor's
/// `next` field is ignored unless its NEXT flag is set. The walk is bounded by
/// the table length, so a cycle fails without an allocation or an infinite loop.
///
/// Successful validation does not prove ownership, DMA isolation, memory
/// ordering, negotiated optional features, or that a device will obey bounds.
pub fn validate_direct_chain(
    table: &[[u8; DESCRIPTOR_BYTES]],
    head: u16,
) -> Result<ChainInfo, ChainError> {
    let size = table.len();
    if size == 0 || size > usize::from(MAX_QUEUE_SIZE) || !size.is_power_of_two() {
        return Err(ChainError::InvalidTableSize);
    }
    let mut index = usize::from(head);
    let mut total = 0u64;
    let mut writable = 0u64;
    let mut saw_write = false;
    for count in 1..=size {
        let d = table.get(index).ok_or(ChainError::IndexOutOfRange)?;
        let addr = u64::from_le_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]);
        let len = u32::from_le_bytes([d[8], d[9], d[10], d[11]]);
        let flags = u16::from_le_bytes([d[12], d[13]]);
        let next = u16::from_le_bytes([d[14], d[15]]);
        if flags & !(NEXT | WRITE) != 0 {
            return Err(ChainError::UnsupportedFlags);
        }
        if addr.checked_add(u64::from(len)).is_none() {
            return Err(ChainError::BufferAddressOverflow);
        }
        if flags & WRITE != 0 {
            saw_write = true;
            writable += u64::from(len);
        } else if saw_write {
            return Err(ChainError::ReadAfterWrite);
        }
        total += u64::from(len);
        if total > MAX_CHAIN_BYTES {
            return Err(ChainError::TooManyBytes);
        }
        if flags & NEXT == 0 {
            return Ok(ChainInfo {
                descriptors: count as u16,
                total_bytes: total,
                writable_bytes: writable,
            });
        }
        index = usize::from(next);
        // Validate the next index even for a full-length chain: an invalid
        // index is not a cycle, and should be reported accurately.
        if index >= size {
            return Err(ChainError::IndexOutOfRange);
        }
    }
    Err(ChainError::CycleOrTooLong)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(addr: u64, len: u32, flags: u16, next: u16) -> [u8; DESCRIPTOR_BYTES] {
        let mut d = [0; DESCRIPTOR_BYTES];
        d[0..8].copy_from_slice(&addr.to_le_bytes());
        d[8..12].copy_from_slice(&len.to_le_bytes());
        d[12..14].copy_from_slice(&flags.to_le_bytes());
        d[14..16].copy_from_slice(&next.to_le_bytes());
        d
    }

    #[test]
    fn accepts_read_then_write_and_ignores_unused_next() {
        let table = [desc(0x1000, 8, NEXT, 1), desc(0x2000, 32, WRITE, u16::MAX)];
        let info = validate_direct_chain(&table, 0).unwrap();
        assert_eq!(
            (
                info.descriptors(),
                info.total_bytes(),
                info.writable_bytes()
            ),
            (2, 40, 32)
        );
        let info = validate_direct_chain(&table, 1).unwrap();
        assert_eq!(
            (
                info.descriptors(),
                info.total_bytes(),
                info.writable_bytes()
            ),
            (1, 32, 32)
        );
    }

    #[test]
    fn rejects_bad_table_head_next_and_cycles() {
        assert_eq!(
            validate_direct_chain(&[], 0),
            Err(ChainError::InvalidTableSize)
        );
        assert_eq!(
            validate_direct_chain(&[[0; 16]; 3], 0),
            Err(ChainError::InvalidTableSize)
        );
        assert_eq!(
            validate_direct_chain(&[[0; 16]; 1], 1),
            Err(ChainError::IndexOutOfRange)
        );
        assert_eq!(
            validate_direct_chain(&[desc(0, 0, NEXT, 1)], 0),
            Err(ChainError::IndexOutOfRange)
        );
        assert_eq!(
            validate_direct_chain(&[desc(0, 0, NEXT, 0)], 0),
            Err(ChainError::CycleOrTooLong)
        );
        let cycle = [desc(0, 1, NEXT, 1), desc(1, 1, NEXT, 0)];
        assert_eq!(
            validate_direct_chain(&cycle, 0),
            Err(ChainError::CycleOrTooLong)
        );
    }

    #[test]
    fn rejects_unsupported_flags_overflow_and_read_after_write() {
        assert_eq!(
            validate_direct_chain(&[desc(0, 1, 4, 0)], 0),
            Err(ChainError::UnsupportedFlags)
        );
        assert_eq!(
            validate_direct_chain(&[desc(0, 1, 0x8000, 0)], 0),
            Err(ChainError::UnsupportedFlags)
        );
        assert_eq!(
            validate_direct_chain(&[desc(u64::MAX, 1, 0, 0)], 0),
            Err(ChainError::BufferAddressOverflow)
        );
        let reversed = [desc(0, 2, NEXT | WRITE, 1), desc(2, 2, 0, 0)];
        assert_eq!(
            validate_direct_chain(&reversed, 0),
            Err(ChainError::ReadAfterWrite)
        );
    }

    #[test]
    fn accepts_exact_byte_limit_and_rejects_more() {
        let exact = [desc(0, u32::MAX, NEXT, 1), desc(0, 1, 0, 0)];
        assert_eq!(
            validate_direct_chain(&exact, 0).unwrap().total_bytes(),
            1u64 << 32
        );
        let too_large = [desc(0, u32::MAX, NEXT, 1), desc(0, 2, 0, 0)];
        assert_eq!(
            validate_direct_chain(&too_large, 0),
            Err(ChainError::TooManyBytes)
        );
    }

    #[test]
    fn bounded_walk_checks_all_small_link_patterns() {
        // Four entries, each either terminal or pointing at one of the four
        // entries. An independent visited-set oracle decides cycle vs success.
        for pattern in 0..625usize {
            let mut value = pattern;
            let mut table = [[0u8; 16]; 4];
            let mut links = [None; 4];
            for i in 0..4 {
                let choice = value % 5;
                value /= 5;
                if choice < 4 {
                    links[i] = Some(choice);
                    table[i] = desc(i as u64, 1, NEXT, choice as u16);
                } else {
                    table[i] = desc(i as u64, 1, 0, 0);
                }
            }
            for head in 0..4 {
                let mut visited = [false; 4];
                let mut cursor = head;
                let mut count = 0;
                let cycle = loop {
                    if visited[cursor] {
                        break true;
                    }
                    visited[cursor] = true;
                    count += 1;
                    match links[cursor] {
                        Some(next) => cursor = next,
                        None => break false,
                    }
                };
                let result = validate_direct_chain(&table, head as u16);
                if cycle {
                    assert_eq!(result, Err(ChainError::CycleOrTooLong));
                } else {
                    assert_eq!(result.unwrap().descriptors() as usize, count);
                }
            }
        }
    }
}
