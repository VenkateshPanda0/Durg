# Hardware qualification plan — phase 0

Status: **no hardware is certified**. Inventory is a candidate list, not a claim
that Durg boots on or supports any physical machine. Windows or Linux identifying a
component is not evidence that Durg has a driver for it.

## Before selecting targets

Select one x86-64 UEFI desktop and one laptop available for repeatable testing.
Record exact product/SKU, CPU, GPU(s), Wi-Fi/Ethernet, audio, NVMe/SATA, USB
controllers, firmware version, display, battery and dock components. Keep serial
numbers, account names, network identifiers and recovery keys out of public logs.
The current development computer can be considered as a laptop candidate, but its
exact configuration and availability for safe testing are not yet certified.

Use a disposable drive and verified independent backup. Do not repartition or
change firmware/boot security on a personal machine as part of detection. Initial
work uses QEMU; real-machine testing begins only after a documented recovery path,
explicit target selection and safe test setup exist.

## Coverage grades

| Grade | Meaning |
|---|---|
| Certified | Named configuration and release passed the full required matrix; maintainer and retest date recorded |
| Works with limitations | Named configuration tested with explicitly listed missing functions and risks |
| Community tested | External report identifies build and configuration; not independently reproduced |
| Unknown | No sufficient test evidence; no compatibility promise |

## Matrix and release evidence

For each target/build, track firmware/boot/recovery, CPU and RAM, storage/flush,
USB/input, graphics/multi-display, Ethernet/Wi-Fi/Bluetooth, audio/camera,
IOMMU, battery/thermal, suspend/hibernate, lid, dock/hotplug, installer,
authenticated update/rollback and independent backup restore. Mark each cell
**not tested / pass / fail / unsupported** with test IDs, logs and limitations.
Never infer success of one GPU, NIC or firmware revision from another.

Define repeatable VM smoke/fault tests first, then disposable hardware tests;
exercise power interruption, low disk/memory, faulty drivers, unplug, denied
permissions, offline setup and upgrade rollback. Establish baselines with exact
hardware, firmware, build, workloads and measurement method for boot, resume,
frame times, idle/standby power and input latency. Do not assert the proposal's
target numbers are achieved without measurements.

## M18 exit blockers

Named machines and access; driver and graphics feasibility evidence; app/browser
port route; boot/update/recovery key ownership; test and review maintainers; and
a maintainable security-support policy. Record an ADR if the native Durg route
is infeasible and the Linux-hosted alternative is chosen.
