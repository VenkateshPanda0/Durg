use super::*;

#[test]
fn sizes_and_alignment_match_split_queue_requirements() {
    for shift in 0..=15 {
        let n = 1u16 << shift;
        let layout = SplitQueueLayout::new(n, n, 0x10_0000, 0x20_0000, 0x30_0000).unwrap();
        assert_eq!(layout.size(), n);
        assert_eq!(layout.descriptor().len(), 16 * u64::from(n));
        assert_eq!(layout.driver().len(), 6 + 2 * u64::from(n));
        assert_eq!(layout.device().len(), 6 + 8 * u64::from(n));
        assert!(!layout.descriptor().is_empty());
    }
}

#[test]
fn rejects_invalid_size_and_device_limit() {
    for n in [0, 3, 6, 32769, u16::MAX] {
        assert_eq!(
            SplitQueueLayout::new(n, u16::MAX, 0, 0x1000, 0x2000),
            Err(LayoutError::InvalidQueueSize)
        );
    }
    assert_eq!(
        SplitQueueLayout::new(8, 0, 0, 0x1000, 0x2000),
        Err(LayoutError::ExceedsDeviceMaximum)
    );
    assert_eq!(
        SplitQueueLayout::new(8, 7, 0, 0x1000, 0x2000),
        Err(LayoutError::ExceedsDeviceMaximum)
    );
    assert!(SplitQueueLayout::new(8, 9, 0, 0x1000, 0x2000).is_ok());
}

#[test]
fn rejects_each_misaligned_or_wrapping_region() {
    assert_eq!(
        SplitQueueLayout::new(1, 1, 1, 0x1000, 0x2000),
        Err(LayoutError::Misaligned)
    );
    assert_eq!(
        SplitQueueLayout::new(1, 1, 0, 0x1001, 0x2000),
        Err(LayoutError::Misaligned)
    );
    assert_eq!(
        SplitQueueLayout::new(1, 1, 0, 0x1000, 0x2002),
        Err(LayoutError::Misaligned)
    );
    assert_eq!(
        SplitQueueLayout::new(1, 1, u64::MAX - 15, 0x1000, 0x2000),
        Err(LayoutError::AddressOverflow)
    );
    assert_eq!(
        SplitQueueLayout::new(1, 1, 0, u64::MAX - 1, 0x2000),
        Err(LayoutError::AddressOverflow)
    );
    assert_eq!(
        SplitQueueLayout::new(1, 1, 0, 0x1000, u64::MAX - 3),
        Err(LayoutError::AddressOverflow)
    );
}

#[test]
fn rejects_overlaps_in_either_direction_but_allows_touching() {
    assert_eq!(
        SplitQueueLayout::new(1, 1, 0x1000, 0x1008, 0x2000),
        Err(LayoutError::Overlap)
    );
    assert_eq!(
        SplitQueueLayout::new(1, 1, 0x1000, 0x2000, 0x100c),
        Err(LayoutError::Overlap)
    );
    assert_eq!(
        SplitQueueLayout::new(1, 1, 0x1000, 0x2000, 0x2004),
        Err(LayoutError::Overlap)
    );
    assert_eq!(
        SplitQueueLayout::new(1, 1, 0x1000, 0x2010, 0x200c),
        Err(LayoutError::Overlap)
    );
    assert!(SplitQueueLayout::new(1, 1, 0x1000, 0x1010, 0x1018).is_ok());
    assert!(SplitQueueLayout::new(1, 1, 0x1020, 0x1010, 0x1000).is_ok());
}

#[test]
fn exhaustive_small_offsets_match_independent_interval_oracle() {
    for size in [1u16, 2, 4, 8] {
        for d in (0..128u64).step_by(16) {
            for a in (0..128u64).step_by(2) {
                for u in (0..128u64).step_by(4) {
                    let n = u64::from(size);
                    let spans = [(d, d + 16 * n), (a, a + 6 + 2 * n), (u, u + 6 + 8 * n)];
                    let intersects = |x: (u64, u64), y: (u64, u64)| x.0 < y.1 && y.0 < x.1;
                    let overlap = intersects(spans[0], spans[1])
                        || intersects(spans[0], spans[2])
                        || intersects(spans[1], spans[2]);
                    assert_eq!(
                        SplitQueueLayout::new(size, size, d, a, u).is_err(),
                        overlap,
                        "size={size} addresses={d},{a},{u}"
                    );
                }
            }
        }
    }
}
