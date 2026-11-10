//! Pure validation for Virtio 1.2 split virtqueue memory layouts.
//!
//! The descriptor table, driver area and device area are distinct ranges. This
//! crate checks their required size, alignment and arithmetic before a caller
//! configures a queue. It does not access shared memory, negotiate features,
//! provide memory barriers, prove physical contiguity or grant DMA authority.
//! The driver and memory manager must establish those properties separately.
//!
//! Layout requirements: OASIS Virtio 1.2, section 2.7, Split Virtqueues:
//! <https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html>.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod chain;
pub use chain::{validate_direct_chain, ChainError, ChainInfo, DESCRIPTOR_BYTES};

/// Largest split virtqueue size permitted by Virtio 1.2.
pub const MAX_QUEUE_SIZE: u16 = 32768;

/// Why a requested split virtqueue layout was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// Queue size is zero, exceeds the specification, or is not a power of two.
    InvalidQueueSize,
    /// The requested queue size exceeds the device's advertised maximum.
    ExceedsDeviceMaximum,
    /// A region's physical address does not meet its required alignment.
    Misaligned,
    /// A region's exclusive end cannot be represented as a `u64`.
    AddressOverflow,
    /// Two queue regions overlap.
    Overlap,
}

/// A half-open physical address range `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    start: u64,
    end: u64,
}

impl Region {
    /// The first physical byte of the region.
    pub const fn start(self) -> u64 {
        self.start
    }
    /// One past the last physical byte of the region.
    pub const fn end(self) -> u64 {
        self.end
    }
    /// Number of bytes the region occupies.
    pub const fn len(self) -> u64 {
        self.end - self.start
    }
    /// Whether the region has zero length.
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    fn new(start: u64, length: u64, alignment: u64) -> Result<Self, LayoutError> {
        if start & (alignment - 1) != 0 {
            return Err(LayoutError::Misaligned);
        }
        let end = start
            .checked_add(length)
            .ok_or(LayoutError::AddressOverflow)?;
        Ok(Self { start, end })
    }

    fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// Validated memory ranges for one Virtio split virtqueue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitQueueLayout {
    size: u16,
    descriptor: Region,
    driver: Region,
    device: Region,
}

impl SplitQueueLayout {
    /// Validates requested queue size, device maximum and physical addresses.
    ///
    /// Each supplied address is the start of a separately backed region; this
    /// function cannot establish that backing is allocated, contiguous, mapped,
    /// owned by the caller, or visible to the device. Device maximum zero means
    /// the queue is unavailable. No memory is accessed or modified.
    pub fn new(
        size: u16,
        device_max: u16,
        descriptor: u64,
        driver: u64,
        device: u64,
    ) -> Result<Self, LayoutError> {
        if size == 0 || size > MAX_QUEUE_SIZE || !size.is_power_of_two() {
            return Err(LayoutError::InvalidQueueSize);
        }
        if size > device_max {
            return Err(LayoutError::ExceedsDeviceMaximum);
        }
        let n = u64::from(size);
        let descriptor = Region::new(descriptor, 16 * n, 16)?;
        let driver = Region::new(driver, 6 + 2 * n, 2)?;
        let device = Region::new(device, 6 + 8 * n, 4)?;
        if descriptor.overlaps(driver) || descriptor.overlaps(device) || driver.overlaps(device) {
            return Err(LayoutError::Overlap);
        }
        Ok(Self {
            size,
            descriptor,
            driver,
            device,
        })
    }

    /// Number of descriptors in the queue.
    pub const fn size(self) -> u16 {
        self.size
    }
    /// Descriptor table range (16-byte alignment).
    pub const fn descriptor(self) -> Region {
        self.descriptor
    }
    /// Driver/available ring range (2-byte alignment).
    pub const fn driver(self) -> Region {
        self.driver
    }
    /// Device/used ring range (4-byte alignment).
    pub const fn device(self) -> Region {
        self.device
    }
}

#[cfg(test)]
mod tests;
