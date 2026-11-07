# M11: Virtio split-queue groundwork

**Status:** in progress, not a driver. M10 object, notification and DMA/IRQ
capabilities remain prerequisites before untrusted user-space drivers can run.

The `durg-virtio` crate provides pure `no_std`, no-unsafe layout validation for
the three memory areas of a Virtio 1.2 split virtqueue. It rejects invalid queue
sizes, sizes larger than the device maximum, misaligned addresses, address-range
overflow and overlapping areas. Size and alignment requirements are from the
[OASIS Virtio 1.2 specification, section 2.7](https://docs.oasis-open.org/virtio/virtio/v1.2/virtio-v1.2.html).
The queue memory lengths include space for the event fields; using extra space
when event index is not negotiated is conservative.

The pure validator does **not** allocate or map memory, prove physical contiguity
or device reachability, protect memory with an IOMMU, negotiate features, perform
MMIO, publish descriptors with ordering barriers, handle interrupts, or recover a
failed device. It is not integrated into a driver or kernel. Those are separate
M10/M11 implementation and security obligations.

Host tests cover the permitted sizes, device limits, each alignment and overflow
boundary, reversed and touching ranges, and an exhaustive small-address interval
oracle. Four negative controls (invalid size accepted, alignment omitted, wrong
driver-area length, overlap allowed) each caused a test failure before restoration.
The crate is included in workspace tests and CI; bare-metal `no_std` compilation is
checked separately. Passing these tests is evidence about layout arithmetic only.
