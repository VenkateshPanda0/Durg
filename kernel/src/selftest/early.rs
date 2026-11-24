//! Early-boot, memory, paging, interrupt and timer tests (M1 to M5), and the deliberate-failure test.
//!
//! Part of the in-kernel self-tests; see [`super`] for how they are run.

use super::*;

pub(super) fn memory_map() -> limine::MemoryMap {
    limine::memory_map().expect("bootloader provided no memory map")
}

pub(super) fn serial_loopback() {
    assert!(serial::loopback_passed(), "UART loopback self-test failed");
}

pub(super) fn memory_map_has_usable_ram() {
    let map = memory_map();
    assert!(map.len() > 0, "memory map is empty");
    assert!(
        map.iter().any(|r| r.kind == MemoryKind::Usable),
        "no usable region in the memory map"
    );
}

/// Spec, "Memory Map Feature": entries are sorted by base, lowest to highest.
pub(super) fn memory_map_sorted_by_base() {
    let map = memory_map();
    let mut previous: Option<u64> = None;
    for region in map.iter() {
        if let Some(previous_base) = previous {
            assert!(
                region.base >= previous_base,
                "entries not sorted: {:#x} follows {:#x}",
                region.base,
                previous_base
            );
        }
        previous = Some(region.base);
    }
}

/// Spec, "Memory Map Feature": usable and bootloader-reclaimable entries are
/// 4096-byte aligned (base and length) and overlap no other entry.
pub(super) fn usable_regions_aligned_and_disjoint() {
    let map = memory_map();
    for (i, region) in map.iter().enumerate() {
        let must_be_clean =
            region.kind == MemoryKind::Usable || region.kind == MemoryKind::BootloaderReclaimable;
        if !must_be_clean {
            continue;
        }
        assert!(
            region.base.is_multiple_of(PAGE_SIZE) && region.length.is_multiple_of(PAGE_SIZE),
            "region {:#x}+{:#x} is not 4 KiB aligned",
            region.base,
            region.length
        );
        for (j, other) in map.iter().enumerate() {
            if i == j || region.length == 0 || other.length == 0 {
                continue;
            }
            let overlaps = region.base < other.end() && other.base < region.end();
            assert!(
                !overlaps,
                "region {:#x}..{:#x} overlaps {:#x}..{:#x}",
                region.base,
                region.end(),
                other.base,
                other.end()
            );
        }
    }
}

pub(super) fn hhdm_offset_in_higher_half() {
    let offset = limine::hhdm_offset().expect("no HHDM offset provided");
    assert!(
        offset >= 0xffff_8000_0000_0000,
        "HHDM offset {offset:#x} is not in the higher half"
    );
    assert!(
        offset.is_multiple_of(PAGE_SIZE),
        "HHDM offset is not page aligned"
    );
}

/// Checks the assumption that the direct map covers usable RAM, by reading a
/// byte from every usable region through it. A wrong assumption faults here
/// (and the fault handler arrives in M3), so this is a fail-fast probe.
pub(super) fn usable_ram_readable_through_hhdm() {
    let map = memory_map();
    let mut probed = 0;
    for region in map.iter().filter(|r| r.kind == MemoryKind::Usable) {
        assert!(
            limine::read_usable_byte(&region).is_some(),
            "could not read usable region {:#x}",
            region.base
        );
        probed += 1;
    }
    assert!(probed > 0, "no usable region to probe");
}

/// The bootloader must honour the revision we ask for, or ACPI is unreadable.
pub(super) fn base_revision_4_loaded() {
    assert_eq!(
        limine::loaded_base_revision(),
        4,
        "bootloader loaded us under a base revision other than 4"
    );
}

pub(super) fn rsdp_provided_and_virtual() {
    let address = limine::rsdp_virtual_address().expect("no readable RSDP provided");
    let hhdm = limine::hhdm_offset().expect("no HHDM offset provided");
    assert!(
        address >= hhdm,
        "RSDP {address:#x} is below the HHDM offset {hhdm:#x}, so it is not an HHDM address"
    );
}

/// Discovery walks RSDP, then XSDT/RSDT, then finds and parses the MADT. QEMU's
/// q35 machine has at least one CPU and one I/O APIC.
pub(super) fn madt_discovered() {
    let info = acpi::discover().expect("ACPI discovery failed");
    let madt = &info.madt;
    assert!(madt.cpu_count >= 1, "MADT lists no CPU");
    assert!(madt.io_apic_count >= 1, "MADT lists no I/O APIC");
    assert!(
        madt.local_apic_address != 0 && madt.local_apic_address.is_multiple_of(PAGE_SIZE),
        "local APIC address {:#x} is zero or not page aligned",
        madt.local_apic_address
    );
}

pub(super) fn cpu_has_local_apic() {
    assert!(
        x86_64::apic_support().local_apic,
        "CPUID reports no on-chip local APIC"
    );
}

pub(super) fn gdt_and_tss_loaded() {
    assert_eq!(
        x86_64::read_cs(),
        gdt::KERNEL_CODE,
        "CS is not our code selector"
    );
    assert_eq!(
        x86_64::read_tr(),
        gdt::TSS_SELECTOR,
        "TR is not our TSS selector"
    );
}

/// Decodes the access byte and flags of the user descriptors. The numbers come
/// from Intel SDM Vol. 3 fig. 3-8: bit 7 present, bits 6:5 DPL, bit 4 "code or
/// data" (S), bit 3 executable, bit 1 readable/writable; in the flags nibble, bit
/// 1 is L (64-bit code) and bit 2 is D (must be 0 when L is set).
pub(super) fn gdt_has_correct_user_segments() {
    fn access(descriptor: u64) -> u64 {
        (descriptor >> 40) & 0xff
    }
    fn flags(descriptor: u64) -> u64 {
        (descriptor >> 52) & 0xf
    }
    let kernel_code = gdt::descriptor(1);
    let kernel_data = gdt::descriptor(2);
    let user_data = gdt::descriptor(3);
    let user_code = gdt::descriptor(4);

    for (name, descriptor, executable, dpl) in [
        ("kernel code", kernel_code, true, 0),
        ("kernel data", kernel_data, false, 0),
        ("user data", user_data, false, 3),
        ("user code", user_code, true, 3),
    ] {
        let access = access(descriptor);
        assert_ne!(access & 0x80, 0, "{name}: not present");
        assert_eq!((access >> 5) & 3, dpl, "{name}: wrong privilege level");
        assert_ne!(access & 0x10, 0, "{name}: not a code/data descriptor");
        assert_eq!(
            access & 0x08 != 0,
            executable,
            "{name}: wrong executable bit"
        );
        assert_ne!(access & 0x02, 0, "{name}: not readable/writable");
        if executable {
            assert_ne!(flags(descriptor) & 0x2, 0, "{name}: L (64-bit) not set");
            assert_eq!(flags(descriptor) & 0x4, 0, "{name}: D set together with L");
        }
    }
    // The selectors the CPU will load on `sysret`.
    assert_eq!(gdt::USER_DATA, 0x1b);
    assert_eq!(gdt::USER_CODE, 0x23);
    assert_eq!(gdt::USER_DATA & 3, 3);
    assert_eq!(gdt::USER_CODE & 3, 3);
}

pub(super) fn idt_loaded() {
    let idtr = x86_64::read_idtr();
    // Copy out of the packed struct before comparing (no references to packed fields).
    let (limit, base) = (idtr.limit, idtr.base);
    assert_eq!(limit, idt::LIMIT, "IDT limit differs");
    assert_eq!(base, idt::base(), "IDT base differs");
}

pub(super) fn legacy_pic_masked() {
    assert_eq!(
        pic::masks(),
        (0xff, 0xff),
        "legacy PIC inputs are not all masked"
    );
}

pub(super) fn breakpoint_exception_returns() {
    let before = trap::breakpoints_handled();
    trap::breakpoint();
    assert_eq!(
        trap::breakpoints_handled(),
        before + 1,
        "breakpoint handler did not run exactly once"
    );
}

/// Reads an address nothing maps and checks the page fault carries the right
/// diagnostics: CR2 holds the address, and the error code says "not present,
/// read, supervisor".
pub(super) fn page_fault_is_caught() {
    // Canonical, in the lower half, where the bootloader maps nothing.
    const UNMAPPED: u64 = 0x0000_0000_dead_0000;
    let fault = trap::probe_read(UNMAPPED).expect_err("reading an unmapped address succeeded");
    assert_eq!(
        fault.address, UNMAPPED,
        "CR2 does not hold the faulting address"
    );
    assert_eq!(
        fault.error_code & 0b10111,
        0,
        "error code {:#x} should mean not-present, read, supervisor, data access",
        fault.error_code
    );
}

/// The same probe on memory that is mapped must succeed, so a passing page
/// fault test above is not just "everything faults".
pub(super) fn hhdm_probe_does_not_fault() {
    let hhdm = limine::hhdm_offset().expect("no HHDM offset provided");
    let map = memory_map();
    let region = map
        .iter()
        .find(|r| r.kind == MemoryKind::Usable && r.length > 0)
        .expect("no usable region");
    trap::probe_read(region.base + hhdm).expect("a mapped usable address faulted");
}

/// Walks the page tables for the direct map address of usable RAM and checks the
/// result is the physical address the region started at.
pub(super) fn paging_translates_hhdm_to_physical() {
    let hhdm = limine::hhdm_offset().expect("no HHDM offset provided");
    let map = memory_map();
    let region = map
        .iter()
        .find(|r| r.kind == MemoryKind::Usable && r.length >= 2 * 4096)
        .expect("no usable region of two pages");
    // An address inside the second page, to check the offset survives.
    let physical = region.base + 4096 + 0x123;
    let mapping = paging::translate(physical + hhdm).expect("HHDM address did not translate");
    assert_eq!(
        mapping.physical, physical,
        "translation gave the wrong physical address"
    );
}

/// A kernel function's address must translate to physical memory the bootloader
/// reported as "kernel and modules".
pub(super) fn paging_translates_kernel_code() {
    let code = run_all as *const () as usize as u64;
    let mapping = paging::translate(code).expect("kernel code address did not translate");
    let map = memory_map();
    let in_kernel_region = map.iter().any(|r| {
        r.kind == MemoryKind::ExecutableAndModules
            && r.base <= mapping.physical
            && mapping.physical < r.end()
    });
    assert!(
        in_kernel_region,
        "kernel code at {code:#x} translates to {:#x}, outside the kernel region",
        mapping.physical
    );
}

pub(super) fn paging_reports_unmapped_addresses() {
    let result = paging::translate(0x0000_0000_dead_0000);
    assert!(
        matches!(result, Err(WalkError::NotMapped { .. })),
        "an unmapped address translated or failed oddly: {result:?}"
    );
    assert!(
        matches!(
            paging::translate(0x0001_0000_0000_0000),
            Err(WalkError::NotCanonical)
        ),
        "a non-canonical address was not rejected"
    );
}

/// The MMIO mapping selects PAT entry 3 (write-through plus cache-disable) and
/// assumes it is uncacheable. Read the PAT to check, instead of trusting that.
pub(super) fn pat_entry_3_is_uncacheable() {
    let pat = x86_64::read_msr(x86_64::IA32_PAT);
    let entry_3 = (pat >> 24) & 0xff;
    // Memory type encoding 0x00 is uncacheable (Intel SDM Vol. 3 table 11-10).
    assert_eq!(
        entry_3, 0x00,
        "PAT entry 3 is {entry_3:#x}, not uncacheable (0x00)"
    );
}

/// The local APIC register page must translate to the physical base from
/// `IA32_APIC_BASE`, and its entry must be uncached.
pub(super) fn lapic_is_mapped_uncached() {
    let apic_base = x86_64::read_msr(x86_64::IA32_APIC_BASE) & 0x000f_ffff_ffff_f000;
    let mapping = paging::translate(lapic::MMIO_BASE).expect("LAPIC page is not mapped");
    assert_eq!(
        mapping.physical, apic_base,
        "LAPIC page maps the wrong physical address"
    );
    assert_eq!(
        mapping.page_size,
        PageSize::Size4K,
        "LAPIC page is not a 4 KiB page"
    );
    assert!(
        mapping
            .entry
            .has(flags::CACHE_DISABLE | flags::WRITE_THROUGH),
        "LAPIC page is not mapped uncached"
    );
    if x86_64::nx_enabled() {
        assert!(
            mapping.entry.has(flags::NO_EXECUTE),
            "LAPIC page is executable although NX is available"
        );
    }
}

pub(super) fn lapic_registers_read_back() {
    let apic = lapic::get().expect("local APIC is not initialised");
    let version = apic.version();
    assert!(
        version != 0 && version != 0xffff_ffff,
        "LAPIC version register reads {version:#x}, which looks like an unmapped or dead page"
    );
}

/// Firmware tells us the local APIC address twice (ACPI and `IA32_APIC_BASE`).
/// They should agree.
pub(super) fn lapic_base_matches_the_madt() {
    let info = acpi::discover().expect("ACPI discovery failed");
    let apic_base = x86_64::read_msr(x86_64::IA32_APIC_BASE) & 0x000f_ffff_ffff_f000;
    assert_eq!(
        info.madt.local_apic_address, apic_base,
        "MADT and IA32_APIC_BASE disagree about the local APIC address"
    );
}

/// Mapping over an existing mapping must fail instead of silently replacing it.
pub(super) fn map_page_refuses_to_replace_a_mapping() {
    let result = paging::map_mmio_page(lapic::MMIO_BASE, 0xfee0_0000);
    assert!(
        matches!(result, Err(paging::MapError::AlreadyMapped)),
        "remapping the LAPIC page returned {result:?}"
    );
}

pub(super) fn pmm_stats() -> frame::Stats {
    frame::stats().expect("frame allocator is not initialised")
}

/// Recomputes, straight from the memory map, how many frames the allocator should
/// manage, and compares. The allocator and this test share no code, so a bug in
/// how `init` or the reclaim step reads the map would show up as a mismatch.
///
/// Tests run after boot has reclaimed the bootloader's memory, so that memory
/// counts too.
pub(super) fn pmm_totals_match_an_independent_calculation() {
    const ONE_MIB: u64 = 1 << 20;
    let stats = pmm_stats();
    let map = memory_map();
    let mut expected: u64 = 0;
    for region in map.iter().filter(|r| {
        matches!(
            r.kind,
            MemoryKind::Usable | MemoryKind::BootloaderReclaimable
        )
    }) {
        let start = region.base.max(ONE_MIB);
        let end = region.end();
        if start < end {
            expected += (end - start) / PAGE_SIZE;
        }
    }
    let bitmap_frames = stats.bitmap_bytes.div_ceil(PAGE_SIZE);
    // Blocks reserved as untyped memory are out of the pool until returned.
    let reserved = cspace::reserved_frames();
    assert_eq!(
        stats.managed,
        expected - bitmap_frames - reserved,
        "managed frames differ from the memory map ({expected} usable above 1 MiB, {bitmap_frames} for bitmaps, {reserved} reserved as untyped memory)"
    );
    assert!(
        frame::check_invariants().is_ok(),
        "{:?}",
        frame::check_invariants()
    );
}

/// Every frame handed out must be page aligned, above 1 MiB, inside a usable
/// region, outside the bitmaps, and different from every other frame.
pub(super) fn pmm_hands_out_only_usable_distinct_frames() {
    const COUNT: usize = 256;
    let stats = pmm_stats();
    let map = memory_map();
    let mut taken = [0u64; COUNT];
    for slot in 0..COUNT {
        let physical = frame::alloc_frame().expect("out of frames");
        assert!(
            physical.is_multiple_of(PAGE_SIZE),
            "{physical:#x} is not frame aligned"
        );
        assert!(physical >= 1 << 20, "{physical:#x} is below 1 MiB");
        assert!(
            map.iter().any(|r| r.kind == MemoryKind::Usable
                && r.base <= physical
                && physical + PAGE_SIZE <= r.end()),
            "{physical:#x} is not inside a usable region"
        );
        let bitmap_end = stats.bitmap_physical + stats.bitmap_bytes.div_ceil(PAGE_SIZE) * PAGE_SIZE;
        assert!(
            physical + PAGE_SIZE <= stats.bitmap_physical || physical >= bitmap_end,
            "{physical:#x} overlaps the allocator's own bitmaps"
        );
        assert!(
            !taken[..slot].contains(&physical),
            "{physical:#x} was handed out twice"
        );
        taken[slot] = physical;
    }
    for physical in taken {
        frame::free_frame(physical).expect("free failed");
    }
    assert_eq!(
        pmm_stats().free,
        stats.free,
        "free count did not return to its starting value"
    );
}

/// Freeing must be refused for: the same frame twice, memory that was never in
/// the pool (frame 0, the kernel, the bitmaps), addresses out of range, and
/// unaligned addresses. A refused free must change nothing.
pub(super) fn pmm_rejects_double_free_and_foreign_frames() {
    use durg_pmm::FreeError::{AlreadyFree, NotManaged, OutOfRange};
    use frame::FreeFrameError::{Refused, Unaligned};

    let before = pmm_stats();
    let physical = frame::alloc_frame().expect("out of frames");
    assert_eq!(frame::free_frame(physical), Ok(()));
    assert_eq!(
        frame::free_frame(physical),
        Err(Refused(AlreadyFree)),
        "double free was accepted"
    );

    assert_eq!(
        frame::free_frame(0),
        Err(Refused(NotManaged)),
        "frame 0 is not ours"
    );
    let kernel = paging::translate(run_all as *const () as usize as u64)
        .expect("kernel code did not translate")
        .physical
        & !(PAGE_SIZE - 1);
    assert_eq!(
        frame::free_frame(kernel),
        Err(Refused(NotManaged)),
        "a kernel frame was accepted"
    );
    assert_eq!(
        frame::free_frame(before.bitmap_physical),
        Err(Refused(NotManaged)),
        "the allocator's own bitmap frame was accepted"
    );
    assert_eq!(frame::free_frame(1 << 60), Err(Refused(OutOfRange)));
    assert_eq!(frame::free_frame(physical + 1), Err(Unaligned));

    assert_eq!(
        pmm_stats().free,
        before.free,
        "a refused free changed the free count"
    );
    assert!(frame::check_invariants().is_ok());
}

/// Dirty a batch of frames, return them, then allocate zeroed ones. The pool
/// will hand the dirty frames back, so this really checks that zeroing happens.
pub(super) fn pmm_zeroed_frames_are_zero() {
    const COUNT: usize = 32;
    let mut taken = [0u64; COUNT];
    for slot in taken.iter_mut() {
        let physical = frame::alloc_frame().expect("out of frames");
        for offset in (0..PAGE_SIZE).step_by(8) {
            frame::write_word(physical, offset, 0xdead_beef_dead_beef).expect("write failed");
        }
        *slot = physical;
    }
    for physical in taken {
        frame::free_frame(physical).expect("free failed");
    }
    let mut zeroed = [0u64; COUNT];
    let mut reused = 0;
    for slot in zeroed.iter_mut() {
        let physical = frame::alloc_zeroed_frame().expect("out of frames");
        if taken.contains(&physical) {
            reused += 1;
        }
        for offset in (0..PAGE_SIZE).step_by(8) {
            assert_eq!(
                frame::read_word(physical, offset),
                Some(0),
                "frame {physical:#x} was not zeroed at offset {offset:#x}"
            );
        }
        *slot = physical;
    }
    assert!(
        reused > 0,
        "no dirty frame was reused, so zeroing was not really exercised"
    );
    for physical in zeroed {
        frame::free_frame(physical).expect("free failed");
    }
}

pub(super) fn xorshift(mut state: u64) -> u64 {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    state
}

/// Each live frame holds a pattern derived from its own address in three places.
/// If any two allocations overlapped, or a freed frame were handed out while
/// still in use, one owner's pattern would be overwritten and the check at free
/// time would catch it.
pub(super) fn pmm_block_return_is_atomic() {
    let before = pmm_stats();
    let block = frame::reserve_aligned_block(2).expect("reserve four frames");
    assert_eq!(pmm_stats().free, before.free - 4);
    assert!(
        !frame::release_block(block + 1, 2),
        "unaligned block accepted"
    );
    assert!(
        !frame::release_block(block, u32::MAX),
        "overflowing size accepted"
    );
    assert!(frame::release_block(block, 1));
    let partly_returned = pmm_stats();
    assert!(
        !frame::release_block(block, 2),
        "overlapping return accepted"
    );
    assert_eq!(
        pmm_stats().free,
        partly_returned.free,
        "failed return changed free frames"
    );
    assert_eq!(
        pmm_stats().managed,
        partly_returned.managed,
        "failed return changed managed frames"
    );
    assert!(frame::release_block(block + 2 * PAGE_SIZE, 1));
    assert!(!frame::release_block(block, 2), "double return accepted");
    assert_eq!(pmm_stats().free, before.free);
    assert_eq!(pmm_stats().managed, before.managed);
    assert!(frame::check_invariants().is_ok());
}

pub(super) fn pmm_stress_with_unique_patterns() {
    const SLOTS: usize = 256;
    const OPERATIONS: u32 = 20_000;
    const OFFSETS: [u64; 3] = [0, PAGE_SIZE / 2, PAGE_SIZE - 8];

    let pattern = |physical: u64| physical ^ 0xa5a5_a5a5_5a5a_5a5a;
    let check = |physical: u64| {
        for offset in OFFSETS {
            assert_eq!(
                frame::read_word(physical, offset),
                Some(pattern(physical)),
                "frame {physical:#x} was overwritten by another owner (offset {offset:#x})"
            );
        }
    };

    let before = pmm_stats();
    let mut live = [0u64; SLOTS]; // 0 means empty: frame 0 is never allocated
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    for _ in 0..OPERATIONS {
        rng = xorshift(rng);
        let slot = (rng >> 33) as usize % SLOTS;
        if live[slot] == 0 {
            let physical = frame::alloc_frame().expect("out of frames during the stress run");
            assert!(
                !live.contains(&physical),
                "{physical:#x} handed out while still live"
            );
            for offset in OFFSETS {
                frame::write_word(physical, offset, pattern(physical)).expect("write failed");
            }
            live[slot] = physical;
        } else {
            check(live[slot]);
            frame::free_frame(live[slot]).expect("free of a live frame failed");
            live[slot] = 0;
        }
    }
    for physical in live.into_iter().filter(|p| *p != 0) {
        check(physical);
        frame::free_frame(physical).expect("final free failed");
    }
    assert_eq!(pmm_stats().free, before.free, "frames leaked or duplicated");
    assert!(
        frame::check_invariants().is_ok(),
        "{:?}",
        frame::check_invariants()
    );
}

/// Takes every free frame (threading them into a list stored in the frames
/// themselves, since there is nowhere else to keep them), checks that the next
/// allocation fails cleanly, then returns them all.
pub(super) fn pmm_exhaustion_and_recovery() {
    let before = pmm_stats();
    let mut head = 0u64;
    let mut taken = 0u64;
    while let Some(physical) = frame::alloc_frame() {
        frame::write_word(physical, 0, head).expect("write failed");
        head = physical;
        taken += 1;
    }
    assert_eq!(
        taken, before.free,
        "allocated a different number of frames than were free"
    );
    assert_eq!(pmm_stats().free, 0);
    assert!(
        frame::alloc_frame().is_none(),
        "allocation succeeded with no frames left"
    );
    assert!(frame::alloc_zeroed_frame().is_none());

    let mut returned = 0u64;
    while head != 0 {
        let next = frame::read_word(head, 0).expect("read failed");
        frame::free_frame(head).expect("free failed");
        head = next;
        returned += 1;
    }
    assert_eq!(returned, taken, "list was cut short");
    assert_eq!(pmm_stats().free, before.free, "free count did not recover");
    assert!(
        frame::check_invariants().is_ok(),
        "{:?}",
        frame::check_invariants()
    );
}

/// Mapping a page that needs a new page table must take exactly one frame from
/// the allocator (no more boot pool). The new window is two MiB beyond the
/// LAPIC page, so it needs its own page table but shares the directory above.
pub(super) fn map_page_takes_new_table_frames_from_the_pmm() {
    let before = pmm_stats().free;
    let virtual_address = lapic::MMIO_BASE + (2 << 20);
    paging::map_mmio_page(virtual_address, 0xfee0_0000).expect("mapping failed");
    assert_eq!(
        pmm_stats().free,
        before - 1,
        "a mapping that needs one new table should consume exactly one frame"
    );
    let mapping = paging::translate(virtual_address).expect("new mapping does not translate");
    assert_eq!(mapping.physical, 0xfee0_0000);
}

/// Read-only data, and writable data, to aim the permission tests at.
pub(super) static RODATA_PROBE: u64 = 0x1234_5678_9abc_def0;
pub(super) static DATA_PROBE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

pub(super) fn address_of<T>(item: &T) -> u64 {
    item as *const T as u64
}

pub(super) fn vm_kernel_page_tables_are_active() {
    let root = vm::kernel_root();
    assert!(
        root != 0,
        "the kernel never switched to its own page tables"
    );
    assert_eq!(
        AddressSpace::active().root(),
        root,
        "the CPU is not using the kernel's page tables"
    );
}

/// No page of the kernel image may be writable and executable at once. Checked
/// page by page over the whole image, not just one sample.
pub(super) fn vm_no_page_is_writable_and_executable() {
    if !x86_64::nx_enabled() {
        return; // without NX nothing can be non-executable
    }
    for section in vm::sections() {
        let mut address = section.start;
        while address < section.end {
            let entry = paging::translate(address)
                .expect("kernel page not mapped")
                .entry;
            assert!(
                !(entry.has(flags::WRITABLE) && !entry.has(flags::NO_EXECUTE)),
                "page {address:#x} in {} is writable and executable",
                section.name
            );
            address += PAGE_SIZE;
        }
    }
}

pub(super) fn vm_sections_have_the_intended_permissions() {
    let nx = x86_64::nx_enabled();
    for section in vm::sections() {
        let entry = paging::translate(section.start)
            .expect("section not mapped")
            .entry;
        let (writable, executable) = match section.name {
            "text" => (false, true),
            "rodata" => (false, false),
            _ => (true, false),
        };
        assert_eq!(
            entry.has(flags::WRITABLE),
            writable,
            "{}: wrong writable bit",
            section.name
        );
        if nx {
            assert_eq!(
                !entry.has(flags::NO_EXECUTE),
                executable,
                "{}: wrong executable bit",
                section.name
            );
        }
    }
}

/// Writing to read-only data must fault with "protection violation, write".
pub(super) fn vm_writing_read_only_data_faults() {
    let fault = trap::probe_write(address_of(&RODATA_PROBE), 0)
        .expect_err("a write to read-only data succeeded");
    assert_eq!(
        fault.error_code & 0b11,
        0b11,
        "error code {:#x} should mean present page, write",
        fault.error_code
    );
    assert_eq!(
        RODATA_PROBE, 0x1234_5678_9abc_def0,
        "the data changed anyway"
    );
}

/// Jumping into writable data must fault as an instruction fetch on a present
/// page (the no-execute bit).
pub(super) fn vm_executing_data_faults() {
    if !x86_64::nx_enabled() {
        return;
    }
    let fault = trap::probe_execute(address_of(&DATA_PROBE))
        .expect_err("executing from a data page succeeded");
    assert_eq!(
        fault.error_code & 0b10001,
        0b10001,
        "error code {:#x} should mean present page, instruction fetch",
        fault.error_code
    );
}

/// The kernel image must not be reachable through the direct map, or its code
/// would have a writable alias.
pub(super) fn vm_kernel_image_has_no_alias_in_the_direct_map() {
    let hhdm = limine::hhdm_offset().expect("no HHDM offset provided");
    for section in vm::sections() {
        let physical = paging::translate(section.start)
            .expect("section not mapped")
            .physical;
        let alias = paging::translate(hhdm + physical);
        assert!(
            matches!(alias, Err(WalkError::NotMapped { .. })),
            "{} is reachable through the direct map: {alias:?}",
            section.name
        );
    }
}

/// The framebuffer and reserved regions are not mapped in the kernel's address
/// space, even though the bootloader's tables had the framebuffer.
pub(super) fn vm_framebuffer_and_reserved_memory_are_not_mapped() {
    let hhdm = limine::hhdm_offset().expect("no HHDM offset provided");
    let map = memory_map();
    let mut checked = 0;
    for region in map.iter().filter(|r| {
        matches!(r.kind, MemoryKind::Framebuffer | MemoryKind::Reserved) && r.length > 0
    }) {
        let result = paging::translate(hhdm + region.base);
        assert!(
            matches!(result, Err(WalkError::NotMapped { .. })),
            "{} region at {:#x} is mapped: {result:?}",
            region.kind.name(),
            region.base
        );
        checked += 1;
    }
    assert!(checked > 0, "no framebuffer or reserved region to check");
}

/// The kernel runs on the stack it built: inside its mapped pages, with an
/// unmapped guard page directly below them.
pub(super) fn vm_running_on_the_kernel_stack_with_a_guard_page() {
    let stack = vm::kernel_stack();
    let rsp = x86_64::read_rsp();
    assert!(
        (stack.bottom..stack.top).contains(&rsp),
        "RSP {rsp:#x} is not inside the kernel stack {:#x}..{:#x}",
        stack.bottom,
        stack.top
    );
    assert_eq!(
        stack.guard + PAGE_SIZE,
        stack.bottom,
        "guard page is not directly below the stack"
    );
    assert!(
        matches!(
            paging::translate(stack.guard),
            Err(WalkError::NotMapped { .. })
        ),
        "the guard page is mapped"
    );
    for address in [stack.bottom, stack.top - 1] {
        let entry = paging::translate(address)
            .expect("stack page not mapped")
            .entry;
        assert!(
            entry.has(flags::WRITABLE),
            "stack page {address:#x} is not writable"
        );
        if x86_64::nx_enabled() {
            assert!(
                entry.has(flags::NO_EXECUTE),
                "stack page {address:#x} is executable"
            );
        }
    }
    // Touching the guard page must fault, not corrupt anything.
    let fault = trap::probe_write(stack.guard + 8, 0).expect_err("the guard page accepted a write");
    assert_eq!(
        fault.error_code & 1,
        0,
        "guard page fault should be 'not present'"
    );
}

pub(super) fn scratch_leaf_flags() -> u64 {
    flags::PRESENT | flags::WRITABLE | paging::no_execute()
}

pub(super) fn paging_map_and_unmap_round_trip() {
    let mut space = AddressSpace::active();
    let physical = frame::alloc_zeroed_frame().expect("out of frames");
    space
        .map_page(vm::SCRATCH_WINDOW, physical, scratch_leaf_flags())
        .expect("map_page failed");

    let mapping = space
        .translate(vm::SCRATCH_WINDOW + 0x10)
        .expect("mapping does not translate");
    assert_eq!(mapping.physical, physical + 0x10);
    assert_eq!(mapping.page_size, PageSize::Size4K);

    // A write through the mapping must land in the frame, as seen via the direct map.
    trap::probe_write(vm::SCRATCH_WINDOW, 0x5a).expect("write through the mapping faulted");
    assert_eq!(frame::read_word(physical, 0).map(|w| w & 0xff), Some(0x5a));

    let unmapped = space.unmap(vm::SCRATCH_WINDOW).expect("unmap failed");
    assert_eq!(unmapped.physical, physical);
    assert_eq!(unmapped.size, PageSize::Size4K);
    assert!(
        space.translate(vm::SCRATCH_WINDOW).is_err(),
        "page translates after unmap"
    );
    frame::free_frame(physical).expect("free failed");
}

/// The CPU caches translations. After unmapping a page that was just used, the
/// next access must still fault, which only holds if the TLB entry was
/// invalidated.
pub(super) fn paging_unmap_takes_effect_despite_the_tlb() {
    let mut space = AddressSpace::active();
    let physical = frame::alloc_zeroed_frame().expect("out of frames");
    space
        .map_page(vm::SCRATCH_WINDOW, physical, scratch_leaf_flags())
        .expect("map_page failed");
    // Use the page so its translation is cached.
    trap::probe_write(vm::SCRATCH_WINDOW, 1).expect("first write faulted");
    trap::probe_read(vm::SCRATCH_WINDOW).expect("read faulted");

    space.unmap(vm::SCRATCH_WINDOW).expect("unmap failed");
    let after = trap::probe_read(vm::SCRATCH_WINDOW);
    frame::free_frame(physical).expect("free failed");
    assert!(
        after.is_err(),
        "the page was still readable after unmap: a stale TLB entry survived"
    );
}

/// A 2 MiB aligned, 2 MiB sized span inside usable memory, for tests that map
/// huge pages. They never touch the memory, only the translations.
pub(super) fn huge_span(skip: usize) -> u64 {
    const HUGE: u64 = 2 * 1024 * 1024;
    let map = memory_map();
    let mut found = 0;
    for region in map.iter().filter(|r| r.kind == MemoryKind::Usable) {
        let start = region.base.next_multiple_of(HUGE);
        let mut candidate = start;
        while candidate + HUGE <= region.end() {
            if found == skip {
                return candidate;
            }
            found += 1;
            candidate += HUGE;
        }
    }
    panic!("no 2 MiB aligned usable span number {skip}");
}

pub(super) fn paging_unmap_errors() {
    let mut space = AddressSpace::active();
    assert!(
        matches!(
            space.unmap(0x0000_0000_dead_0000),
            Err(UnmapError::NotMapped { .. })
        ),
        "unmapping nothing should say NotMapped"
    );
    assert_eq!(space.unmap(0x0001_0000_0000_0000), Err(UnmapError::Invalid));

    // The direct map uses 2 MiB pages where it can. Unmapping the middle of one
    // would need to split it, which is refused, and nothing may change.
    let hhdm = limine::hhdm_offset().expect("no HHDM offset provided");
    let span = huge_span(0);
    let inside = hhdm + span + PAGE_SIZE;
    assert_eq!(
        space.unmap(inside),
        Err(UnmapError::HugePageInTheWay { level: 2 })
    );
    assert!(
        space.translate(inside).is_ok(),
        "a refused unmap changed the mapping"
    );
}

pub(super) fn paging_2m_page_maps_translates_and_unmaps() {
    let mut space = AddressSpace::active();
    let physical = huge_span(1);
    let virt = vm::SCRATCH_WINDOW + 4 * 1024 * 1024;
    space
        .map_2m(virt, physical, flags::PRESENT | paging::no_execute())
        .expect("map_2m failed");
    let mapping = space
        .translate(virt + 0x1_2345)
        .expect("2 MiB page does not translate");
    assert_eq!(mapping.physical, physical + 0x1_2345);
    assert_eq!(mapping.page_size, PageSize::Size2M);
    // Unaligned addresses are refused.
    assert!(space
        .map_2m(virt + PAGE_SIZE, physical, flags::PRESENT)
        .is_err());
    let unmapped = space.unmap(virt).expect("unmap of the huge page failed");
    assert_eq!(unmapped.size, PageSize::Size2M);
    assert_eq!(unmapped.physical, physical);
    assert!(space.translate(virt).is_err());
}

/// `map_range` must use one huge page for the aligned 2 MiB and 4 KiB pages for
/// the rest.
pub(super) fn paging_map_range_mixes_page_sizes() {
    const HUGE: u64 = 2 * 1024 * 1024;
    let mut space = AddressSpace::active();
    let physical = huge_span(2);
    let virt = vm::SCRATCH_WINDOW + 8 * 1024 * 1024;
    let length = HUGE + 2 * PAGE_SIZE;
    // The physical span after the first 2 MiB may belong to anything, so only the
    // translations are checked and nothing is read or written.
    space
        .map_range(
            virt,
            physical,
            length,
            flags::PRESENT | paging::no_execute(),
        )
        .expect("map_range failed");
    assert_eq!(space.translate(virt).unwrap().page_size, PageSize::Size2M);
    assert_eq!(
        space.translate(virt + HUGE - 1).unwrap().page_size,
        PageSize::Size2M
    );
    for page in 0..2 {
        let at = virt + HUGE + page * PAGE_SIZE;
        let mapping = space.translate(at).expect("tail page not mapped");
        assert_eq!(mapping.page_size, PageSize::Size4K);
        assert_eq!(mapping.physical, physical + HUGE + page * PAGE_SIZE);
    }
    assert!(
        space.translate(virt + length).is_err(),
        "mapped past the end"
    );
    space.unmap(virt).expect("unmap huge");
    space.unmap(virt + HUGE).expect("unmap tail 0");
    space.unmap(virt + HUGE + PAGE_SIZE).expect("unmap tail 1");
}

/// `main` runs a boot-time timer check. By the time the tests run, interrupts
/// must have been delivered and handled, so the counter is at least that high.
pub(super) fn timer_interrupts_were_counted() {
    assert!(
        lapic::ticks() >= 5,
        "only {} timer interrupts were handled",
        lapic::ticks()
    );
}

#[cfg(feature = "fail-demo")]
pub(super) fn deliberate_failure() {
    panic!("this failure is deliberate (fail-demo feature)");
}
