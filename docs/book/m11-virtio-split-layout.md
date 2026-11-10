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

## Direct descriptor-chain validation

A second pure function validates a stable snapshot of a direct descriptor table.
It bounds traversal by the queue size, rejects cycles and invalid links, unsupported
flags (including indirect descriptors), wrapping buffer ranges, read-after-write
ordering and chains longer than 2^32 bytes. It reports descriptor, total-byte and
writable-byte counts. Unused `next` fields are ignored as required for terminal
descriptors. Indirect descriptors, optional negotiated ordering and concurrent
device access are not implemented.

Five unit tests include an independent exhaustive four-entry graph oracle; six
negative controls deliberately broke flag, address, ordering, length, index and
walk-bound checks and each was detected. A `virtio_chain` libFuzzer target compares
results to a separate visited-set/128-bit oracle; a 60-second local smoke run
completed 12,173,648 executions with no finding. This is not proof of DMA isolation
or of correctness with concurrent devices.
