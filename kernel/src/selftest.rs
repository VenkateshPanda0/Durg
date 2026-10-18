//! In-kernel self-tests, run inside QEMU with the `selftest` feature.
//!
//! Rust's built-in test framework needs nightly features for a bare-metal
//! target (ADR-0002 keeps us on stable), so this is a small registry of our
//! own: each test is a function that panics on failure. The panic handler
//! prints the message and exits QEMU with the failure code, so the host-side
//! harness sees both which test failed and a non-zero status. A test that
//! hangs is caught by the host-side timeout.
//!
//! The `fail-demo` feature adds a test that always fails. `cargo xtask test`
//! boots that variant too and requires it to be reported as a failure, which
//! proves the harness can fail.

use durg_paging::{flags, PageSize, WalkError};

use crate::acpi;
use crate::arch::serial;
use crate::arch::x86_64::{self, gdt, idt, lapic, pic, trap};
use crate::limine::{self, MemoryKind};
use crate::mm;
use crate::mm::frame;
use crate::mm::paging::{self, AddressSpace, UnmapError};
use crate::mm::vm;
use crate::qemu::{self, ExitCode};
use crate::sched::ExitStatus as user_status;
use crate::user;

struct Test {
    name: &'static str,
    run: fn(),
}

const PAGE_SIZE: u64 = 4096;

const TESTS: &[Test] = &[
    Test {
        name: "serial_loopback",
        run: serial_loopback,
    },
    Test {
        name: "memory_map_has_usable_ram",
        run: memory_map_has_usable_ram,
    },
    Test {
        name: "memory_map_sorted_by_base",
        run: memory_map_sorted_by_base,
    },
    Test {
        name: "usable_regions_aligned_and_disjoint",
        run: usable_regions_aligned_and_disjoint,
    },
    Test {
        name: "hhdm_offset_in_higher_half",
        run: hhdm_offset_in_higher_half,
    },
    Test {
        name: "usable_ram_readable_through_hhdm",
        run: usable_ram_readable_through_hhdm,
    },
    Test {
        name: "base_revision_4_loaded",
        run: base_revision_4_loaded,
    },
    Test {
        name: "rsdp_provided_and_virtual",
        run: rsdp_provided_and_virtual,
    },
    Test {
        name: "madt_discovered",
        run: madt_discovered,
    },
    Test {
        name: "cpu_has_local_apic",
        run: cpu_has_local_apic,
    },
    Test {
        name: "gdt_and_tss_loaded",
        run: gdt_and_tss_loaded,
    },
    Test {
        name: "gdt_has_correct_user_segments",
        run: gdt_has_correct_user_segments,
    },
    Test {
        name: "idt_loaded",
        run: idt_loaded,
    },
    Test {
        name: "legacy_pic_masked",
        run: legacy_pic_masked,
    },
    Test {
        name: "breakpoint_exception_returns",
        run: breakpoint_exception_returns,
    },
    Test {
        name: "page_fault_is_caught_with_cr2_and_error_code",
        run: page_fault_is_caught,
    },
    Test {
        name: "hhdm_probe_does_not_fault",
        run: hhdm_probe_does_not_fault,
    },
    Test {
        name: "paging_translates_hhdm_to_physical",
        run: paging_translates_hhdm_to_physical,
    },
    Test {
        name: "paging_translates_kernel_code_into_the_kernel_region",
        run: paging_translates_kernel_code,
    },
    Test {
        name: "paging_reports_unmapped_addresses",
        run: paging_reports_unmapped_addresses,
    },
    Test {
        name: "pat_entry_3_is_uncacheable",
        run: pat_entry_3_is_uncacheable,
    },
    Test {
        name: "lapic_is_mapped_uncached_at_its_physical_base",
        run: lapic_is_mapped_uncached,
    },
    Test {
        name: "lapic_registers_read_back_sane_values",
        run: lapic_registers_read_back,
    },
    Test {
        name: "lapic_base_matches_the_madt",
        run: lapic_base_matches_the_madt,
    },
    Test {
        name: "map_page_refuses_to_replace_a_mapping",
        run: map_page_refuses_to_replace_a_mapping,
    },
    Test {
        name: "pmm_totals_match_an_independent_calculation",
        run: pmm_totals_match_an_independent_calculation,
    },
    Test {
        name: "pmm_hands_out_only_usable_distinct_aligned_frames",
        run: pmm_hands_out_only_usable_distinct_frames,
    },
    Test {
        name: "pmm_rejects_double_free_and_foreign_frames",
        run: pmm_rejects_double_free_and_foreign_frames,
    },
    Test {
        name: "pmm_zeroed_frames_are_zero",
        run: pmm_zeroed_frames_are_zero,
    },
    Test {
        name: "pmm_stress_with_unique_patterns",
        run: pmm_stress_with_unique_patterns,
    },
    Test {
        name: "pmm_exhaustion_and_recovery",
        run: pmm_exhaustion_and_recovery,
    },
    Test {
        name: "map_page_takes_new_table_frames_from_the_pmm",
        run: map_page_takes_new_table_frames_from_the_pmm,
    },
    Test {
        name: "vm_kernel_page_tables_are_active",
        run: vm_kernel_page_tables_are_active,
    },
    Test {
        name: "vm_no_kernel_page_is_both_writable_and_executable",
        run: vm_no_page_is_writable_and_executable,
    },
    Test {
        name: "vm_sections_have_the_intended_permissions",
        run: vm_sections_have_the_intended_permissions,
    },
    Test {
        name: "vm_writing_read_only_data_faults",
        run: vm_writing_read_only_data_faults,
    },
    Test {
        name: "vm_executing_data_faults_on_an_instruction_fetch",
        run: vm_executing_data_faults,
    },
    Test {
        name: "vm_kernel_image_has_no_alias_in_the_direct_map",
        run: vm_kernel_image_has_no_alias_in_the_direct_map,
    },
    Test {
        name: "vm_framebuffer_and_reserved_memory_are_not_mapped",
        run: vm_framebuffer_and_reserved_memory_are_not_mapped,
    },
    Test {
        name: "vm_running_on_the_kernel_stack_with_a_guard_page",
        run: vm_running_on_the_kernel_stack_with_a_guard_page,
    },
    Test {
        name: "paging_map_and_unmap_round_trip",
        run: paging_map_and_unmap_round_trip,
    },
    Test {
        name: "paging_unmap_takes_effect_despite_the_tlb",
        run: paging_unmap_takes_effect_despite_the_tlb,
    },
    Test {
        name: "paging_unmap_errors",
        run: paging_unmap_errors,
    },
    Test {
        name: "paging_2m_page_maps_translates_and_unmaps",
        run: paging_2m_page_maps_translates_and_unmaps,
    },
    Test {
        name: "paging_map_range_mixes_page_sizes",
        run: paging_map_range_mixes_page_sizes,
    },
    Test {
        name: "heap_boxes_vecs_strings_and_maps_work",
        run: heap_collections_work,
    },
    Test {
        name: "heap_alignment_up_to_a_page_is_honoured",
        run: heap_alignment_is_honoured,
    },
    Test {
        name: "heap_random_alloc_free_stress_keeps_contents_intact",
        run: heap_stress_keeps_contents_intact,
    },
    Test {
        name: "heap_grows_on_demand_and_stays_consistent",
        run: heap_grows_on_demand,
    },
    Test {
        name: "heap_pages_are_writable_and_not_executable",
        run: heap_pages_are_rw_nx,
    },
    Test {
        name: "heap_refused_request_fails_gracefully_and_heap_stays_usable",
        run: heap_oom_is_graceful,
    },
    Test {
        name: "heap_detects_double_and_wild_frees",
        run: heap_detects_bad_frees,
    },
    Test {
        name: "irq_lock_masks_real_interrupts_and_restores_them",
        run: irq_lock_masks_real_interrupts,
    },
    Test {
        name: "sched_spawn_join_runs_a_thread_and_returns_its_resources",
        run: sched_spawn_join_returns_resources,
    },
    Test {
        name: "sched_yield_alternates_between_cooperating_threads",
        run: sched_yield_alternates,
    },
    Test {
        name: "sched_switch_preserves_callee_saved_registers",
        run: sched_switch_preserves_registers,
    },
    Test {
        name: "sched_timer_preempts_threads_that_never_yield",
        run: sched_timer_preempts,
    },
    Test {
        name: "sched_irq_lock_prevents_lost_updates_under_preemption",
        run: sched_lock_prevents_lost_updates,
    },
    Test {
        name: "sched_thread_stacks_have_unmapped_guard_pages",
        run: sched_thread_stack_guard_pages,
    },
    Test {
        name: "sched_refuses_threads_beyond_the_limit_and_recovers",
        run: sched_thread_limit,
    },
    Test {
        name: "user_space_shares_the_kernel_half_and_has_an_empty_lower_half",
        run: user_space_shares_kernel_half,
    },
    Test {
        name: "user_mappings_are_user_accessible_at_every_level_kernel_ones_never",
        run: user_bit_at_every_level,
    },
    Test {
        name: "user_accessible_page_in_the_kernel_half_is_refused",
        run: user_page_in_kernel_half_refused,
    },
    Test {
        name: "kernel_mappings_made_later_appear_in_existing_user_spaces",
        run: later_kernel_mappings_are_shared,
    },
    Test {
        name: "switching_to_a_user_space_keeps_the_kernel_running",
        run: switching_address_spaces_works,
    },
    Test {
        name: "destroying_a_user_space_returns_every_frame",
        run: destroy_user_space_returns_frames,
    },
    Test {
        name: "syscall_msrs_are_programmed",
        run: user_msrs_programmed,
    },
    Test {
        name: "user_program_runs_in_ring_3_and_calls_the_kernel",
        run: user_hello_round_trip,
    },
    Test {
        name: "user_faults_end_only_the_task",
        run: user_faults_end_only_the_task,
    },
    Test {
        name: "user_bad_system_calls_are_refused_and_the_task_goes_on",
        run: user_bad_system_calls,
    },
    Test {
        name: "user_output_is_sanitised_before_it_reaches_the_console",
        run: user_output_is_sanitised,
    },
    Test {
        name: "user_registers_survive_system_calls",
        run: user_registers_survive_syscalls,
    },
    Test {
        name: "user_task_starts_with_every_register_zeroed",
        run: user_starts_with_zeroed_registers,
    },
    Test {
        name: "user_task_is_preempted_in_ring_3_and_can_be_killed",
        run: user_preempted_and_killed,
    },
    Test {
        name: "user_tasks_return_all_their_resources",
        run: user_tasks_return_resources,
    },
    Test {
        name: "syscall_handler_survives_random_arguments",
        run: syscall_handler_survives_random_arguments,
    },
    Test {
        name: "cpu_protections_match_what_cpuid_reports",
        run: cpu_protections_match_cpuid,
    },
    Test {
        name: "smep_stops_the_kernel_executing_user_pages",
        run: smep_blocks_kernel_execute_of_user_pages,
    },
    Test {
        name: "smap_stops_the_kernel_touching_user_pages_by_accident",
        run: smap_blocks_accidental_user_access,
    },
    Test {
        name: "umip_stops_ring_3_reading_descriptor_table_addresses",
        run: umip_blocks_sidt_in_ring_3,
    },
    Test {
        name: "timer_interrupts_were_counted_during_boot",
        run: timer_interrupts_were_counted,
    },
    #[cfg(feature = "fail-demo")]
    Test {
        name: "deliberate_failure",
        run: deliberate_failure,
    },
];

/// Runs every test, then exits QEMU with success. A failing test never
/// returns: its panic exits QEMU with the failure code.
pub fn run_all() -> ! {
    println!("selftest: running {} tests", TESTS.len());
    for test in TESTS {
        print!("test {} ... ", test.name);
        (test.run)();
        println!("ok");
    }
    println!("selftest: all {} tests passed", TESTS.len());
    qemu::exit(ExitCode::Success)
}

fn memory_map() -> limine::MemoryMap {
    limine::memory_map().expect("bootloader provided no memory map")
}

fn serial_loopback() {
    assert!(serial::loopback_passed(), "UART loopback self-test failed");
}

fn memory_map_has_usable_ram() {
    let map = memory_map();
    assert!(map.len() > 0, "memory map is empty");
    assert!(
        map.iter().any(|r| r.kind == MemoryKind::Usable),
        "no usable region in the memory map"
    );
}

/// Spec, "Memory Map Feature": entries are sorted by base, lowest to highest.
fn memory_map_sorted_by_base() {
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
fn usable_regions_aligned_and_disjoint() {
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

fn hhdm_offset_in_higher_half() {
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
fn usable_ram_readable_through_hhdm() {
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
fn base_revision_4_loaded() {
    assert_eq!(
        limine::loaded_base_revision(),
        4,
        "bootloader loaded us under a base revision other than 4"
    );
}

fn rsdp_provided_and_virtual() {
    let address = limine::rsdp_virtual_address().expect("no readable RSDP provided");
    let hhdm = limine::hhdm_offset().expect("no HHDM offset provided");
    assert!(
        address >= hhdm,
        "RSDP {address:#x} is below the HHDM offset {hhdm:#x}, so it is not an HHDM address"
    );
}

/// Discovery walks RSDP, then XSDT/RSDT, then finds and parses the MADT. QEMU's
/// q35 machine has at least one CPU and one I/O APIC.
fn madt_discovered() {
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

fn cpu_has_local_apic() {
    assert!(
        x86_64::apic_support().local_apic,
        "CPUID reports no on-chip local APIC"
    );
}

fn gdt_and_tss_loaded() {
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
fn gdt_has_correct_user_segments() {
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

fn idt_loaded() {
    let idtr = x86_64::read_idtr();
    // Copy out of the packed struct before comparing (no references to packed fields).
    let (limit, base) = (idtr.limit, idtr.base);
    assert_eq!(limit, idt::LIMIT, "IDT limit differs");
    assert_eq!(base, idt::base(), "IDT base differs");
}

fn legacy_pic_masked() {
    assert_eq!(
        pic::masks(),
        (0xff, 0xff),
        "legacy PIC inputs are not all masked"
    );
}

fn breakpoint_exception_returns() {
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
fn page_fault_is_caught() {
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
fn hhdm_probe_does_not_fault() {
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
fn paging_translates_hhdm_to_physical() {
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
fn paging_translates_kernel_code() {
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

fn paging_reports_unmapped_addresses() {
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
fn pat_entry_3_is_uncacheable() {
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
fn lapic_is_mapped_uncached() {
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

fn lapic_registers_read_back() {
    let apic = lapic::get().expect("local APIC is not initialised");
    let version = apic.version();
    assert!(
        version != 0 && version != 0xffff_ffff,
        "LAPIC version register reads {version:#x}, which looks like an unmapped or dead page"
    );
}

/// Firmware tells us the local APIC address twice (ACPI and `IA32_APIC_BASE`).
/// They should agree.
fn lapic_base_matches_the_madt() {
    let info = acpi::discover().expect("ACPI discovery failed");
    let apic_base = x86_64::read_msr(x86_64::IA32_APIC_BASE) & 0x000f_ffff_ffff_f000;
    assert_eq!(
        info.madt.local_apic_address, apic_base,
        "MADT and IA32_APIC_BASE disagree about the local APIC address"
    );
}

/// Mapping over an existing mapping must fail instead of silently replacing it.
fn map_page_refuses_to_replace_a_mapping() {
    let result = paging::map_mmio_page(lapic::MMIO_BASE, 0xfee0_0000);
    assert!(
        matches!(result, Err(paging::MapError::AlreadyMapped)),
        "remapping the LAPIC page returned {result:?}"
    );
}

fn pmm_stats() -> frame::Stats {
    frame::stats().expect("frame allocator is not initialised")
}

/// Recomputes, straight from the memory map, how many frames the allocator should
/// manage, and compares. The allocator and this test share no code, so a bug in
/// how `init` or the reclaim step reads the map would show up as a mismatch.
///
/// Tests run after boot has reclaimed the bootloader's memory, so that memory
/// counts too.
fn pmm_totals_match_an_independent_calculation() {
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
    assert_eq!(
        stats.managed,
        expected - bitmap_frames,
        "managed frames differ from the memory map ({expected} usable above 1 MiB, {bitmap_frames} for bitmaps)"
    );
    assert!(
        frame::check_invariants().is_ok(),
        "{:?}",
        frame::check_invariants()
    );
}

/// Every frame handed out must be page aligned, above 1 MiB, inside a usable
/// region, outside the bitmaps, and different from every other frame.
fn pmm_hands_out_only_usable_distinct_frames() {
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
fn pmm_rejects_double_free_and_foreign_frames() {
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
fn pmm_zeroed_frames_are_zero() {
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

fn xorshift(mut state: u64) -> u64 {
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    state
}

/// Each live frame holds a pattern derived from its own address in three places.
/// If any two allocations overlapped, or a freed frame were handed out while
/// still in use, one owner's pattern would be overwritten and the check at free
/// time would catch it.
fn pmm_stress_with_unique_patterns() {
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
fn pmm_exhaustion_and_recovery() {
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
fn map_page_takes_new_table_frames_from_the_pmm() {
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
static RODATA_PROBE: u64 = 0x1234_5678_9abc_def0;
static DATA_PROBE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

fn address_of<T>(item: &T) -> u64 {
    item as *const T as u64
}

fn vm_kernel_page_tables_are_active() {
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
fn vm_no_page_is_writable_and_executable() {
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

fn vm_sections_have_the_intended_permissions() {
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
fn vm_writing_read_only_data_faults() {
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
fn vm_executing_data_faults() {
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
fn vm_kernel_image_has_no_alias_in_the_direct_map() {
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
fn vm_framebuffer_and_reserved_memory_are_not_mapped() {
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
fn vm_running_on_the_kernel_stack_with_a_guard_page() {
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

fn scratch_leaf_flags() -> u64 {
    flags::PRESENT | flags::WRITABLE | paging::no_execute()
}

fn paging_map_and_unmap_round_trip() {
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
fn paging_unmap_takes_effect_despite_the_tlb() {
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
fn huge_span(skip: usize) -> u64 {
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

fn paging_unmap_errors() {
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

fn paging_2m_page_maps_translates_and_unmaps() {
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
fn paging_map_range_mixes_page_sizes() {
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
fn timer_interrupts_were_counted() {
    assert!(
        lapic::ticks() >= 5,
        "only {} timer interrupts were handled",
        lapic::ticks()
    );
}

#[cfg(feature = "fail-demo")]
fn deliberate_failure() {
    panic!("this failure is deliberate (fail-demo feature)");
}

// ---- kernel heap (M6) -------------------------------------------------------

/// A small deterministic random number generator (xorshift64), so a failing
/// run can be reproduced from its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, limit: u64) -> u64 {
        self.next() % limit
    }
}

fn heap_collections_work() {
    use alloc::boxed::Box;
    use alloc::collections::BTreeMap;
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::fmt::Write;

    let boxed = Box::new([7u8; 300]);
    assert!(boxed.iter().all(|&b| b == 7));

    let mut numbers = Vec::new();
    for n in 0..5000u32 {
        numbers.push(n);
    }
    assert_eq!(
        numbers.iter().map(|&n| u64::from(n)).sum::<u64>(),
        12_497_500
    );

    let mut text = String::new();
    for n in 0..200 {
        write!(text, "{n},").expect("writing to a String cannot fail");
    }
    assert!(text.starts_with("0,1,2,") && text.ends_with("198,199,"));

    let mut map = BTreeMap::new();
    for n in 0..500u64 {
        map.insert(n * 7919 % 1000, n);
    }
    assert_eq!(map.len(), 500);
    assert_eq!(map.get(&(3 * 7919 % 1000)), Some(&3));
}

fn heap_alignment_is_honoured() {
    use alloc::alloc::{alloc, dealloc};
    use core::alloc::Layout;

    // Keep a small block alive so the start of the free memory is only 16-byte
    // aligned. Without it, the heap's first block begins on a page boundary and
    // every alignment would be met by luck.
    let spacer_layout = Layout::from_size_align(16, 1).expect("valid layout");
    // SAFETY: the layout has a non-zero size.
    let spacer = unsafe { alloc(spacer_layout) };
    assert!(!spacer.is_null(), "spacer allocation failed");

    for shift in 0..=12 {
        let align = 1usize << shift;
        for size in [1usize, 24, 100, 4096] {
            let layout = Layout::from_size_align(size, align).expect("valid layout");
            // SAFETY: the layout has a non-zero size.
            let pointer = unsafe { alloc(layout) };
            assert!(!pointer.is_null(), "alloc({size}, {align}) failed");
            assert_eq!(
                pointer as usize % align,
                0,
                "alloc({size}, {align}) returned a misaligned pointer"
            );
            // SAFETY: `pointer` came from `alloc` with this layout and is freed once.
            unsafe { dealloc(pointer, layout) };
        }
    }
    // SAFETY: allocated above with this layout, freed once.
    unsafe { dealloc(spacer, spacer_layout) };
    mm::heap::check_invariants().expect("free list corrupted");
}

/// Many random allocations and frees, each block filled with a pattern that is
/// checked before it is freed. Overlapping blocks, or a free that damages a
/// neighbour, change a pattern and fail the test. At the end everything is
/// freed and the free memory must be exactly what it was before (no leak, no
/// loss to fragmentation).
fn heap_stress_keeps_contents_intact() {
    use alloc::alloc::{alloc, dealloc};
    use alloc::vec::Vec;
    use core::alloc::Layout;

    struct Live {
        pointer: *mut u8,
        layout: Layout,
        pattern: u8,
    }

    fn check(live: &Live) {
        for offset in 0..live.layout.size() {
            // SAFETY: the block is live and `offset` is inside it.
            let byte = unsafe { *live.pointer.add(offset) };
            assert_eq!(byte, live.pattern, "heap block was overwritten");
        }
    }

    let before = mm::heap::stats();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut blocks: Vec<Live> = Vec::new();
    for round in 0..3000u32 {
        if blocks.len() < 200 && (blocks.is_empty() || rng.below(100) < 55) {
            let size = 1 + rng.below(1500) as usize;
            let align = 1usize << rng.below(7);
            let layout = Layout::from_size_align(size, align).expect("valid layout");
            // SAFETY: the layout has a non-zero size.
            let pointer = unsafe { alloc(layout) };
            assert!(!pointer.is_null(), "allocation failed in round {round}");
            let pattern = (round % 251) as u8 + 1;
            // SAFETY: the block is valid for `size` bytes and exclusively ours.
            unsafe { core::ptr::write_bytes(pointer, pattern, size) };
            blocks.push(Live {
                pointer,
                layout,
                pattern,
            });
        } else {
            let index = rng.below(blocks.len() as u64) as usize;
            let victim = blocks.swap_remove(index);
            check(&victim);
            // SAFETY: allocated above with this layout, freed once.
            unsafe { dealloc(victim.pointer, victim.layout) };
        }
    }
    for victim in blocks {
        check(&victim);
        // SAFETY: allocated above with this layout, freed once.
        unsafe { dealloc(victim.pointer, victim.layout) };
    }
    mm::heap::check_invariants().expect("free list corrupted");
    let after = mm::heap::stats();
    // Anything the heap grew by during the test is free again too.
    let grown = (after.mapped_bytes - before.mapped_bytes) as usize;
    assert_eq!(
        after.heap.free_bytes,
        before.heap.free_bytes + grown,
        "free memory changed: a block leaked"
    );
    // Other long-lived allocations (the scheduler's run queue, for one) may split
    // the free memory, so "one block" is not the property. Everything this test
    // took must merge back into the shape the free list had before it.
    assert_eq!(
        after.heap.free_blocks, before.heap.free_blocks,
        "free memory did not merge back"
    );
}

fn heap_grows_on_demand() {
    use alloc::vec::Vec;

    let before = mm::heap::stats();
    let want = before.heap.largest_free_block + 1024 * 1024;
    let mut big: Vec<u8> = Vec::new();
    big.try_reserve_exact(want)
        .expect("the heap should have grown to serve this");
    big.resize(want, 0xa5);
    let during = mm::heap::stats();
    assert!(
        during.mapped_bytes > before.mapped_bytes,
        "no pages were mapped"
    );
    assert_eq!(during.growths, before.growths + 1, "expected one growth");
    assert!(big.iter().all(|&b| b == 0xa5));
    drop(big);
    mm::heap::check_invariants().expect("free list corrupted");
    let after = mm::heap::stats();
    assert_eq!(
        after.mapped_bytes, during.mapped_bytes,
        "the heap never shrinks"
    );
    assert!(
        after.heap.largest_free_block >= want,
        "freed memory is available again"
    );
}

fn heap_pages_are_rw_nx() {
    let mapping = paging::translate(mm::heap::HEAP_BASE).expect("heap base is not mapped");
    let raw = mapping.entry.raw();
    assert_ne!(raw & flags::WRITABLE, 0, "heap page is not writable");
    assert_ne!(raw & flags::NO_EXECUTE, 0, "heap page is executable");
    assert_eq!(raw & flags::USER, 0, "heap page is user-accessible");
    // The page just past the mapped part of the window must not be mapped.
    let edge = mm::heap::HEAP_BASE + mm::heap::stats().mapped_bytes;
    assert!(
        paging::translate(edge).is_err(),
        "memory past the end of the heap is mapped"
    );
}

fn heap_oom_is_graceful() {
    use alloc::vec::Vec;

    let before = mm::heap::stats();
    let mut huge: Vec<u8> = Vec::new();
    // Far past the 64 MiB limit.
    assert!(
        huge.try_reserve(1 << 30).is_err(),
        "a 1 GiB request must be refused"
    );
    let after = mm::heap::stats();
    assert!(after.failed_growths > before.failed_growths);
    assert_eq!(
        after.mapped_bytes, before.mapped_bytes,
        "failed growth leaked pages"
    );
    assert_eq!(after.heap.free_bytes, before.heap.free_bytes);
    mm::heap::check_invariants().expect("free list corrupted");
    // And the heap still works.
    let mut small: Vec<u8> = Vec::new();
    small
        .try_reserve(4096)
        .expect("heap unusable after a refused request");
}

/// The global allocator panics on a bad free (a bug elsewhere), so the checked
/// path is tested on a private heap built over one frame from the frame
/// allocator.
fn heap_detects_bad_frees() {
    use core::ptr::NonNull;
    use durg_heap::{Heap, HeapError};

    let physical = frame::alloc_zeroed_frame().expect("no frame for the test heap");
    let base = (physical + limine::hhdm_offset().expect("no HHDM")) as *mut u8;
    let mut heap = Heap::new();
    // SAFETY: one whole frame reachable through the HHDM, exclusively ours until
    // it is freed at the end of this test, 4096 bytes being a multiple of 16.
    unsafe { heap.add_region(base, 4096) }.expect("add_region failed");

    let block = heap.alloc(100, 8).expect("allocation failed");
    // SAFETY: allocated just above with this layout.
    assert_eq!(unsafe { heap.dealloc(block, 100, 8) }, Ok(()));
    // SAFETY: deliberately wrong (a second free of the same block); the heap
    // must refuse it without touching anything.
    assert_eq!(
        unsafe { heap.dealloc(block, 100, 8) },
        Err(HeapError::OverlapsFreeMemory),
        "double free was accepted"
    );
    let wild = NonNull::new(base.wrapping_add(2048)).expect("non-null");
    // SAFETY: deliberately a pointer that was never allocated, inside free memory.
    assert_eq!(
        unsafe { heap.dealloc(wild, 64, 8) },
        Err(HeapError::OverlapsFreeMemory),
        "free of a never-allocated block was accepted"
    );
    heap.check_invariants().expect("bad frees damaged the list");
    assert_eq!(heap.stats().free_bytes, 4096);
    frame::free_frame(physical).expect("could not return the frame");
}

// ---- locks (M7) -------------------------------------------------------------

fn interrupts_enabled() -> bool {
    let flags: u64;
    // SAFETY: `pushfq; pop` reads RFLAGS and has no other effect.
    unsafe { core::arch::asm!("pushfq", "pop {}", out(reg) flags, options(nomem)) };
    flags & (1 << 9) != 0
}

/// The host tests check the lock's logic against a fake flag. This checks it
/// against the real one: inside the lock the CPU's interrupt flag is off, and
/// afterwards it is back on if it was on and off if it was off.
fn irq_lock_masks_real_interrupts() {
    use crate::arch::x86_64::{disable_interrupts, IrqLock};

    static LOCK: IrqLock<u32> = IrqLock::new(0);

    // SAFETY: no timer or other interrupt source is running during the
    // self-tests, so briefly allowing interrupts cannot run a handler.
    unsafe { core::arch::asm!("sti", options(nomem, nostack)) };
    assert!(interrupts_enabled(), "could not enable interrupts");
    {
        let mut guard = LOCK.lock();
        *guard += 1;
        assert!(!interrupts_enabled(), "interrupts on inside the lock");
    }
    assert!(interrupts_enabled(), "interrupts not restored to on");

    disable_interrupts();
    drop(LOCK.lock());
    assert!(
        !interrupts_enabled(),
        "the lock turned interrupts on that were off"
    );
    assert_eq!(*LOCK.lock(), 1);
}

// ---- threads and scheduling (M7) --------------------------------------------

mod sched_tests {
    use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    use crate::arch::x86_64::{self, IrqLock};
    use crate::mm::frame;
    use crate::sched::{self, SpawnError};

    use durg_paging::PAGE_SIZE;

    pub static FLAG: AtomicBool = AtomicBool::new(false);

    pub fn set_flag() {
        FLAG.store(true, Ordering::SeqCst);
    }

    /// Free frames, for leak checks.
    pub fn free_frames() -> u64 {
        frame::stats().expect("frame allocator").free
    }

    // -- yield alternation --------------------------------------------------

    pub static LOG: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];
    pub static LOG_LEN: AtomicUsize = AtomicUsize::new(0);

    fn log(id: u64) {
        let index = LOG_LEN.fetch_add(1, Ordering::SeqCst);
        LOG[index].store(id, Ordering::SeqCst);
    }

    pub fn yielder_a() {
        for _ in 0..5 {
            log(1);
            sched::yield_now();
        }
    }

    pub fn yielder_b() {
        for _ in 0..5 {
            log(2);
            sched::yield_now();
        }
    }

    // -- registers ------------------------------------------------------------

    pub static REGISTER_FAILURES: AtomicU64 = AtomicU64::new(0);

    extern "C" fn yield_shim() {
        sched::yield_now();
    }

    /// Loads distinct values into every callee-saved register, yields to the
    /// other thread (which loads *different* values), and reads them back.
    /// Returns the registers in the order rbx, rbp, r12, r13, r14, r15.
    fn round_trip(base: u64) -> [u64; 6] {
        let (mut rbx, mut rbp, mut r12, mut r13, mut r14, mut r15): (u64, u64, u64, u64, u64, u64);
        // SAFETY: the block saves and restores rbx and rbp itself (the compiler
        // owns them), changes the other callee-saved registers only as declared
        // outputs, and calls an `extern "C"` function that preserves them. Two
        // pushes keep the stack 16-byte aligned for the call. `clobber_abi`
        // covers everything the callee may change.
        unsafe {
            core::arch::asm!(
                "push rbx",
                "push rbp",
                "lea rbx, [rax + 1]",
                "lea rbp, [rax + 2]",
                "lea r12, [rax + 3]",
                "lea r13, [rax + 4]",
                "lea r14, [rax + 5]",
                "lea r15, [rax + 6]",
                "call {yield_shim}",
                "mov r8, rbx",
                "mov r9, rbp",
                "pop rbp",
                "pop rbx",
                yield_shim = sym yield_shim,
                inout("rax") base => _,
                lateout("r8") rbx,
                lateout("r9") rbp,
                lateout("r12") r12,
                lateout("r13") r13,
                lateout("r14") r14,
                lateout("r15") r15,
                clobber_abi("C"),
            );
        }
        [rbx, rbp, r12, r13, r14, r15]
    }

    fn register_thread(base: u64) {
        for round in 0..25u64 {
            let base = base + round * 0x100;
            let got = round_trip(base);
            for (index, value) in got.iter().enumerate() {
                if *value != base + 1 + index as u64 {
                    REGISTER_FAILURES.fetch_or(1 << index, Ordering::SeqCst);
                }
            }
        }
    }

    pub fn register_thread_a() {
        register_thread(0x1111_0000);
    }

    pub fn register_thread_b() {
        register_thread(0x2222_0000);
    }

    // -- the race -----------------------------------------------------------

    /// Flip to `false` to remove the lock: the updates below are then lost.
    pub const USE_LOCK: bool = true;
    pub const RACERS: usize = 3;
    pub const ROUNDS: u64 = 400;
    /// Busy-wait inside each read-modify-write, to give the timer a real chance
    /// to land in the middle of it.
    const WINDOW: u32 = 3_000;

    static LOCKED: IrqLock<u64> = IrqLock::new(0);
    pub static UNLOCKED: AtomicU64 = AtomicU64::new(0);

    fn pause() {
        for i in 0..WINDOW {
            core::hint::black_box(i);
        }
    }

    pub fn racer() {
        for _ in 0..ROUNDS {
            if USE_LOCK {
                let mut guard = LOCKED.lock();
                let before = *guard;
                pause();
                *guard = before + 1;
            } else {
                let before = UNLOCKED.load(Ordering::Relaxed);
                pause();
                UNLOCKED.store(before + 1, Ordering::Relaxed);
            }
        }
    }

    pub fn locked_total() -> u64 {
        *LOCKED.lock()
    }

    // -- guard pages ----------------------------------------------------------

    pub static OBSERVED_RSP: AtomicU64 = AtomicU64::new(0);
    pub static RELEASE: AtomicBool = AtomicBool::new(false);

    pub fn stack_reporter() {
        OBSERVED_RSP.store(x86_64::read_rsp(), Ordering::SeqCst);
        while !RELEASE.load(Ordering::SeqCst) {
            sched::yield_now();
        }
    }

    // -- limits ---------------------------------------------------------------

    pub fn parked() {
        while !RELEASE.load(Ordering::SeqCst) {
            sched::yield_now();
        }
    }

    pub fn page() -> u64 {
        PAGE_SIZE
    }

    pub fn too_many() -> SpawnError {
        SpawnError::TooManyThreads
    }
}

fn sched_spawn_join_returns_resources() {
    use core::sync::atomic::Ordering;
    use sched_tests::{free_frames, set_flag, FLAG};

    // One round to create the page tables the stack window needs (they are
    // never freed), so the second round can be compared exactly.
    for round in 0..2 {
        FLAG.store(false, Ordering::SeqCst);
        let before = free_frames();
        let handle = crate::sched::spawn("one-shot", set_flag).expect("spawn failed");
        let id = handle.id();
        assert_ne!(
            id,
            crate::sched::current_id(),
            "thread has the boot thread's id"
        );
        handle.join();
        assert!(FLAG.load(Ordering::SeqCst), "the thread did not run");
        // The finished thread's stack is released by the next thread to run.
        crate::sched::yield_now();
        assert_eq!(
            crate::sched::thread_count(),
            1,
            "a thread is still registered"
        );
        assert_eq!(
            crate::sched::free_stack_slots(),
            crate::sched::MAX_THREADS,
            "a stack slot leaked"
        );
        if round == 1 {
            assert_eq!(free_frames(), before, "thread stack frames leaked");
        }
    }
    assert_eq!(crate::sched::current_name(), "boot");
}

fn sched_yield_alternates() {
    use core::sync::atomic::Ordering;
    use sched_tests::{yielder_a, yielder_b, LOG, LOG_LEN};

    LOG_LEN.store(0, Ordering::SeqCst);
    let a = crate::sched::spawn("yield-a", yielder_a).expect("spawn a");
    let b = crate::sched::spawn("yield-b", yielder_b).expect("spawn b");
    a.join();
    b.join();
    let len = LOG_LEN.load(Ordering::SeqCst);
    assert_eq!(len, 10, "each thread logs five times");
    let log: [u64; 10] = core::array::from_fn(|i| LOG[i].load(Ordering::SeqCst));
    assert_eq!(
        log,
        [1, 2, 1, 2, 1, 2, 1, 2, 1, 2],
        "round robin did not alternate the two threads"
    );
}

fn sched_switch_preserves_registers() {
    use core::sync::atomic::Ordering;
    use sched_tests::{register_thread_a, register_thread_b, REGISTER_FAILURES};

    REGISTER_FAILURES.store(0, Ordering::SeqCst);
    let a = crate::sched::spawn("regs-a", register_thread_a).expect("spawn a");
    let b = crate::sched::spawn("regs-b", register_thread_b).expect("spawn b");
    a.join();
    b.join();
    let failed = REGISTER_FAILURES.load(Ordering::SeqCst);
    assert_eq!(
        failed, 0,
        "a switch changed callee-saved registers (bit i set = rbx, rbp, r12..r15 index i): {failed:#b}"
    );
}

fn sched_timer_preempts() {
    let report = crate::sched::preemption_check().expect("preemption check failed");
    assert!(report.counts.iter().all(|&c| c > 0));
    // Three workers plus the boot thread each need at least one turn, so a
    // rotation is several switches.
    assert!(
        report.switches >= 4,
        "only {} context switches",
        report.switches
    );
}

fn sched_lock_prevents_lost_updates() {
    use core::sync::atomic::Ordering;
    use sched_tests::{locked_total, racer, RACERS, ROUNDS, UNLOCKED};

    UNLOCKED.store(0, Ordering::SeqCst);
    let before = locked_total();
    let mut handles = alloc::vec::Vec::new();
    for _ in 0..RACERS {
        handles.push(crate::sched::spawn("racer", racer).expect("spawn racer"));
    }
    crate::sched::start_preemption();
    let switches_before = crate::sched::switches();
    for handle in handles {
        handle.join();
    }
    crate::sched::stop_preemption();
    let switches = crate::sched::switches() - switches_before;
    // Without preemption in the middle of the work the test proves nothing, so
    // insist the timer actually switched threads while the racers ran.
    assert!(
        switches > RACERS as u64,
        "no preemption during the race ({switches})"
    );
    let expected = RACERS as u64 * ROUNDS;
    let total = if sched_tests::USE_LOCK {
        locked_total() - before
    } else {
        UNLOCKED.load(Ordering::SeqCst)
    };
    assert_eq!(
        total, expected,
        "updates were lost: the critical section was interrupted"
    );
}

fn sched_thread_stack_guard_pages() {
    use core::sync::atomic::Ordering;
    use sched_tests::{page, stack_reporter, OBSERVED_RSP, RELEASE};

    RELEASE.store(false, Ordering::SeqCst);
    OBSERVED_RSP.store(0, Ordering::SeqCst);
    let handle = crate::sched::spawn("stack-reporter", stack_reporter).expect("spawn");
    while OBSERVED_RSP.load(Ordering::SeqCst) == 0 {
        crate::sched::yield_now();
    }
    let rsp = OBSERVED_RSP.load(Ordering::SeqCst);
    let (guard, bottom, top) =
        crate::sched::stack_bounds(rsp).expect("rsp is not in the thread window");
    assert!((bottom..top).contains(&rsp), "thread ran outside its stack");
    assert_eq!(bottom - guard, page());
    assert!(
        paging::translate(guard).is_err(),
        "the thread's guard page is mapped"
    );
    let entry = paging::translate(bottom)
        .expect("stack bottom unmapped")
        .entry
        .raw();
    assert_ne!(entry & flags::WRITABLE, 0);
    assert_ne!(entry & flags::NO_EXECUTE, 0, "thread stack is executable");
    assert!(crate::sched::in_thread_stack_guard(guard));
    assert!(!crate::sched::in_thread_stack_guard(bottom));
    RELEASE.store(true, Ordering::SeqCst);
    handle.join();
    crate::sched::yield_now();
    assert!(
        paging::translate(bottom).is_err(),
        "a finished thread's stack is still mapped"
    );
}

fn sched_thread_limit() {
    use core::sync::atomic::Ordering;
    use sched_tests::{free_frames, parked, too_many, RELEASE};

    RELEASE.store(false, Ordering::SeqCst);
    let before = free_frames();
    let mut handles = alloc::vec::Vec::new();
    // The boot thread counts towards the limit.
    for _ in 1..crate::sched::MAX_THREADS {
        handles.push(crate::sched::spawn("parked", parked).expect("spawn below the limit"));
    }
    assert_eq!(crate::sched::thread_count(), crate::sched::MAX_THREADS);
    assert_eq!(
        crate::sched::spawn("one-too-many", parked).err(),
        Some(too_many()),
        "the thread limit was not enforced"
    );
    RELEASE.store(true, Ordering::SeqCst);
    for handle in handles {
        handle.join();
    }
    crate::sched::yield_now();
    assert_eq!(crate::sched::thread_count(), 1);
    assert_eq!(crate::sched::free_stack_slots(), crate::sched::MAX_THREADS);
    // Page tables for the stack window may have been created, but stacks must
    // not leak: allow only a few table frames.
    let leaked = before.saturating_sub(free_frames());
    assert!(
        leaked <= 8,
        "{leaked} frames leaked after 63 threads came and went"
    );
}

// ---- user address spaces (M8) -----------------------------------------------

fn kernel_space() -> AddressSpace {
    // `AddressSpace` identifies a space by its root, so this is a handle to the
    // kernel's tables, not a copy.
    AddressSpace::active()
}

fn user_space_shares_kernel_half() {
    let kernel = kernel_space();
    let user = AddressSpace::new_user().expect("new_user failed");
    for index in paging::USER_HALF_ENTRIES..512 {
        let shared = user.root_entry(index).expect("readable").raw();
        let original = kernel.root_entry(index).expect("readable").raw();
        assert_eq!(shared, original, "upper-half slot {index} is not shared");
        assert_ne!(
            shared & flags::PRESENT,
            0,
            "upper-half slot {index} is empty"
        );
        assert_eq!(
            shared & flags::USER,
            0,
            "kernel slot {index} is user-accessible"
        );
    }
    for index in 0..paging::USER_HALF_ENTRIES {
        let entry = user.root_entry(index).expect("readable").raw();
        assert_eq!(
            entry & flags::PRESENT,
            0,
            "lower-half slot {index} is not empty"
        );
    }
    // Kernel code is reachable through the new space, at the same frame.
    let code = run_all as *const () as usize as u64;
    let expected = paging::translate(code)
        .expect("kernel code unmapped")
        .physical;
    let through_user = user
        .translate(code)
        .expect("kernel code unreachable in user space");
    assert_eq!(through_user.physical, expected);
    user.destroy_user_half();
}

fn user_bit_at_every_level() {
    const USER_PAGE: u64 = 0x0000_0000_0040_0000;
    let mut user = AddressSpace::new_user().expect("new_user failed");
    let physical = frame::alloc_zeroed_frame().expect("no frame");
    user.map_page(
        USER_PAGE,
        physical,
        flags::PRESENT | flags::WRITABLE | flags::USER | paging::no_execute(),
    )
    .expect("map failed");
    let path = user.path_entries(USER_PAGE);
    for (level, entry) in path.iter().enumerate() {
        let entry = entry.expect("path is shorter than four levels");
        assert_ne!(
            entry.raw() & flags::USER,
            0,
            "level {} entry on the way to a user page lacks the user bit",
            4 - level
        );
    }
    // A kernel-only page in the user half is allowed, and its leaf has no user bit.
    let kernel_only = frame::alloc_zeroed_frame().expect("no frame");
    user.map_page(
        USER_PAGE + 0x1000,
        kernel_only,
        flags::PRESENT | flags::WRITABLE,
    )
    .expect("map failed");
    let leaf = user.path_entries(USER_PAGE + 0x1000)[3].expect("leaf present");
    assert_eq!(leaf.raw() & flags::USER, 0);
    // Tables in the kernel half never carry the user bit, at any level.
    let kernel_path = kernel_space().path_entries(vm::kernel_stack().bottom);
    for (level, entry) in kernel_path.iter().enumerate() {
        let entry = entry.expect("kernel stack path");
        assert_eq!(
            entry.raw() & flags::USER,
            0,
            "kernel stack mapping has the user bit at level {}",
            4 - level
        );
    }
    user.destroy_user_half();
}

fn user_page_in_kernel_half_refused() {
    let mut user = AddressSpace::new_user().expect("new_user failed");
    let physical = frame::alloc_zeroed_frame().expect("no frame");
    let user_leaf = flags::PRESENT | flags::WRITABLE | flags::USER;
    for address in [
        0xffff_8000_1000_0000u64,
        vm::SCRATCH_WINDOW,
        0xffff_ffff_8000_0000,
    ] {
        assert!(
            matches!(
                user.map_page(address, physical, user_leaf),
                Err(paging::MapError::UserPageInKernelHalf)
            ),
            "a user page was accepted at {address:#x}"
        );
    }
    assert!(matches!(
        user.map_2m(0xffff_8000_2000_0000, 0x4000_0000, user_leaf),
        Err(paging::MapError::UserPageInKernelHalf)
    ));
    // The same refusal on the kernel's own tables.
    let mut kernel = kernel_space();
    assert!(matches!(
        kernel.map_page(vm::SCRATCH_WINDOW, physical, user_leaf),
        Err(paging::MapError::UserPageInKernelHalf)
    ));
    frame::free_frame(physical).expect("could not return the frame");
    user.destroy_user_half();
}

fn later_kernel_mappings_are_shared() {
    // Create the user space *first*, then change the kernel's mappings.
    let user = AddressSpace::new_user().expect("new_user failed");
    let mut kernel = kernel_space();
    // A top-level slot nothing uses (slot 400). Before `populate_kernel_half` it
    // was empty, so a table created here later would exist in the kernel's root
    // alone and be missing from a user space created earlier. A slot that is
    // already in use (the scratch window, say) would hide that.
    let address = 0xffff_c800_0000_0000u64;
    assert!(
        user.translate(address).is_err(),
        "the test address is unexpectedly mapped before the test"
    );
    let physical = frame::alloc_zeroed_frame().expect("no frame");
    kernel
        .map_page(
            address,
            physical,
            flags::PRESENT | flags::WRITABLE | paging::no_execute(),
        )
        .expect("map failed");
    let seen = user
        .translate(address)
        .expect("a kernel mapping made after the user space was created is missing from it");
    assert_eq!(seen.physical, physical);
    let removed = kernel.unmap(address).expect("unmap failed");
    frame::free_frame(removed.physical).expect("free failed");
    assert!(
        user.translate(address).is_err(),
        "unmap not visible in the user space"
    );
    user.destroy_user_half();
}

fn switching_address_spaces_works() {
    const USER_PAGE: u64 = 0x0000_0000_0040_0000;
    let kernel = kernel_space();
    let kernel_root = kernel.root();
    let mut user = AddressSpace::new_user().expect("new_user failed");
    let physical = frame::alloc_zeroed_frame().expect("no frame");
    user.map_page(
        USER_PAGE,
        physical,
        flags::PRESENT | flags::WRITABLE | flags::USER | paging::no_execute(),
    )
    .expect("map failed");
    frame::write_word(physical, 0, 0x5eed_cafe_dead_beef).expect("write failed");

    let before = user.translate(USER_PAGE);
    assert!(before.is_ok());
    // Not reachable in the kernel's own space.
    assert!(kernel.translate(USER_PAGE).is_err());

    // SAFETY: the user space shares the kernel's upper half, which holds the
    // code, stack, tables and direct map this function needs, and interrupts are
    // not enabled here. The kernel root is restored below.
    unsafe { user.activate() };
    // Reading a user page from ring 0 is refused while SMAP is on, so this test
    // opens an explicit access window (`stac`/`clac`) around the one read.
    // SAFETY: the page is mapped readable at this address in the active space.
    let seen = x86_64::protection::with_user_access(|| unsafe {
        core::ptr::read_volatile(USER_PAGE as *const u64)
    });
    println!("(inside the user address space; the kernel is still running)");
    // SAFETY: restores the tables that were active on entry.
    unsafe { kernel.activate() };

    assert_eq!(
        seen, 0x5eed_cafe_dead_beef,
        "the user page showed other contents"
    );
    assert_eq!(AddressSpace::active().root(), kernel_root);
    assert!(
        paging::translate(USER_PAGE).is_err(),
        "the user page is still visible after switching back"
    );
    user.destroy_user_half();
}

fn destroy_user_space_returns_frames() {
    let before = sched_tests::free_frames();
    let mut user = AddressSpace::new_user().expect("new_user failed");
    let leaf = flags::PRESENT | flags::WRITABLE | flags::USER | paging::no_execute();
    // Pages far apart, so several separate tables are created.
    let addresses = [
        0x0000_0000_0040_0000u64,
        0x0000_0000_0040_1000,
        0x0000_0000_8000_0000,
        0x0000_0080_0000_0000,
        0x0000_7fff_ffff_f000,
    ];
    for address in addresses {
        let physical = frame::alloc_zeroed_frame().expect("no frame");
        user.map_page(address, physical, leaf).expect("map failed");
    }
    let during = sched_tests::free_frames();
    // Five leaf pages, plus the root, plus the tables below it.
    assert!(before - during > addresses.len() as u64 + 1);
    let freed = user.destroy_user_half();
    assert_eq!(
        sched_tests::free_frames(),
        before,
        "destroying the space leaked frames"
    );
    assert_eq!(
        freed as u64,
        before - during,
        "freed count disagrees with the allocator"
    );
}

// ---- user mode and system calls (M8) ---------------------------------------

mod user_tests {
    use crate::mm::frame;
    use crate::sched::{self, ExitStatus, JoinHandle};
    use crate::user;

    /// Runs `code` as a ring-3 task to completion and returns how it ended.
    pub fn run(name: &'static str, code: &[u8]) -> ExitStatus {
        let handle = user::spawn_program(name, code).expect("could not start the program");
        wait(&handle)
    }

    pub fn wait(handle: &JoinHandle) -> ExitStatus {
        loop {
            if let Some(status) = handle.status() {
                // The finished thread's resources are released by the next thread
                // to run.
                sched::yield_now();
                return status;
            }
            sched::yield_now();
        }
    }

    pub fn free_frames() -> u64 {
        frame::stats().expect("frame allocator").free
    }

    pub fn last_print() -> alloc::string::String {
        let last = crate::syscall::LAST_PRINT.lock();
        alloc::string::String::from_utf8_lossy(&last.0[..last.1]).into_owned()
    }
}

fn user_msrs_programmed() {
    const EFER: u32 = 0xc000_0080;
    const STAR: u32 = 0xc000_0081;
    const LSTAR: u32 = 0xc000_0082;
    const FMASK: u32 = 0xc000_0084;
    assert_ne!(x86_64::read_msr(EFER) & 1, 0, "EFER.SCE is not set");
    // `syscall` loads CS from bits 47:32 and `sysret` derives the user selectors
    // from bits 63:48 (see the GDT module).
    let star = x86_64::read_msr(STAR);
    assert_eq!((star >> 32) & 0xffff, u64::from(gdt::KERNEL_CODE));
    assert_eq!(((star >> 48) & 0xffff) + 8, u64::from(gdt::USER_DATA & !3));
    assert_eq!(((star >> 48) & 0xffff) + 16, u64::from(gdt::USER_CODE & !3));
    // The entry point is in kernel text.
    let lstar = x86_64::read_msr(LSTAR);
    let text = vm::sections()[0];
    assert!(
        (text.start..text.end).contains(&lstar),
        "LSTAR {lstar:#x} is not in kernel text"
    );
    // Interrupts, single-stepping, direction flag, nested task and alignment
    // check are cleared on entry.
    let mask = x86_64::read_msr(FMASK);
    for (bit, name) in [(9, "IF"), (8, "TF"), (10, "DF"), (14, "NT"), (18, "AC")] {
        assert_ne!(mask & (1 << bit), 0, "FMASK does not clear {name}");
    }
}

fn user_hello_round_trip() {
    let status = user_tests::run("hello", user::hello());
    assert_eq!(status, user_status::Exited(7));
    assert_eq!(user_tests::last_print(), "hello from ring 3");
}

/// Every way a program in ring 3 can reach for something it must not have. Each
/// must end only that task, with the exception the hardware raised, and the
/// kernel must carry on to the next one.
fn user_faults_end_only_the_task() {
    use user::programs as p;
    // Page fault error codes (Intel SDM Vol. 3 fig. 6-9): bit 0 present (set =
    // protection violation, clear = page not present), bit 1 write, bit 2 user
    // mode, bit 4 instruction fetch.
    const PROTECTION_READ: u64 = 0b00101;
    const PROTECTION_WRITE: u64 = 0b00111;
    const PROTECTION_FETCH: u64 = 0b10101;
    const NOT_PRESENT_READ: u64 = 0b00100;
    const PAGE_FAULT: u64 = 14;
    const GENERAL_PROTECTION: u64 = 13;

    // The instruction address matters for the two "run code from here" cases: a
    // fetch that is refused must be refused at the *first* instruction. Without
    // this, a stack that were wrongly executable would run its zeroed bytes
    // harmlessly until it fell off the end of the page and faulted with the same
    // error code, and the test could not tell.
    let kernel_text = 0xffff_ffff_8000_0000;
    let stack_start = user::STACK_TOP - 16;
    struct Case {
        name: &'static str,
        code: &'static [u8],
        vector: u64,
        error: u64,
        /// Where the fault must have happened, when that is part of the claim.
        rip: Option<u64>,
    }
    let case = |name, code, vector, error, rip| Case {
        name,
        code,
        vector,
        error,
        rip,
    };
    let cases = [
        case(
            "read-kernel",
            p::read_kernel(),
            PAGE_FAULT,
            PROTECTION_READ,
            None,
        ),
        case(
            "write-kernel",
            p::write_kernel(),
            PAGE_FAULT,
            PROTECTION_WRITE,
            None,
        ),
        case(
            "exec-kernel",
            p::exec_kernel(),
            PAGE_FAULT,
            PROTECTION_FETCH,
            Some(kernel_text),
        ),
        case(
            "read-unmapped",
            p::read_unmapped(),
            PAGE_FAULT,
            NOT_PRESENT_READ,
            None,
        ),
        case(
            "write-own-code",
            p::write_own_code(),
            PAGE_FAULT,
            PROTECTION_WRITE,
            None,
        ),
        case(
            "exec-stack",
            p::exec_stack(),
            PAGE_FAULT,
            PROTECTION_FETCH,
            Some(stack_start),
        ),
        case("cli", p::disable_interrupts(), GENERAL_PROTECTION, 0, None),
        case("hlt", p::halt(), GENERAL_PROTECTION, 0, None),
        case("read-cr3", p::read_cr3(), GENERAL_PROTECTION, 0, None),
        case("port-io", p::port_io(), GENERAL_PROTECTION, 0, None),
    ];
    for case in cases {
        let status = user_tests::run("faulty", case.code);
        let user_status::Faulted { vector, error, rip } = status else {
            panic!("{}: the task ended some other way: {status:?}", case.name);
        };
        assert_eq!(
            (vector, error),
            (case.vector, case.error),
            "{}: wrong exception",
            case.name
        );
        if let Some(expected) = case.rip {
            assert_eq!(
                rip, expected,
                "{}: the fault was not at the first instruction",
                case.name
            );
        }
    }
    // And the kernel is still here, with nothing left over.
    assert_eq!(crate::sched::thread_count(), 1);
}

fn user_bad_system_calls() {
    use durg_syscall::Error;
    use user::programs as p;
    let cases: [(&str, &[u8], Error); 5] = [
        (
            "kernel pointer",
            p::print_kernel_pointer(),
            Error::BadAddress,
        ),
        (
            "unmapped pointer",
            p::print_unmapped_pointer(),
            Error::BadAddress,
        ),
        ("length over the limit", p::print_too_long(), Error::TooLong),
        (
            "wrapping range",
            p::print_wrapping_range(),
            Error::BadAddress,
        ),
        ("unknown call", p::unknown_call(), Error::NoSuchCall),
    ];
    for (name, code, error) in cases {
        // The task is refused, keeps running, and exits with the error value it
        // was handed.
        let status = user_tests::run("refused", code);
        assert_eq!(
            status,
            user_status::Exited(error.to_return_value()),
            "{name}: wrong result"
        );
    }
}

fn user_output_is_sanitised() {
    let status = user_tests::run("control", user::programs::print_control_characters());
    assert_eq!(status, user_status::Exited(0));
    // ESC [ 2 J LF [ i n f o ] CR: every control byte, including the escape and
    // the newline, becomes '?', so the text cannot clear the host's terminal or
    // start what looks like a kernel log line.
    assert_eq!(user_tests::last_print(), "?[2J?[info]?");
}

fn user_registers_survive_syscalls() {
    let status = user_tests::run("regs", user::programs::keeps_registers());
    assert_eq!(
        status,
        user_status::Exited(0),
        "a register changed across a system call"
    );
}

fn user_starts_with_zeroed_registers() {
    let status = user_tests::run("entry", user::programs::registers_at_entry());
    assert_eq!(
        status,
        user_status::Exited(0),
        "the task started with a non-zero register: a kernel value leaked"
    );
}

/// A task that never calls the kernel is still switched out by the timer, which
/// arrives on the kernel stack named in the TSS, and it can be stopped from
/// outside.
fn user_preempted_and_killed() {
    use crate::sched;
    let spinner = user::spawn_program("spinner", user::programs::spin()).expect("spawn spinner");
    let hello = user::spawn_program("hello", user::hello()).expect("spawn hello");
    sched::start_preemption();
    let hello_status = user_tests::wait(&hello);
    assert_eq!(hello_status, user_status::Exited(7));
    assert!(
        !spinner.is_finished(),
        "the spinner finished by itself (it cannot)"
    );
    spinner.kill();
    let spinner_status = user_tests::wait(&spinner);
    sched::stop_preemption();
    assert_eq!(spinner_status, user_status::Killed);
}

fn user_tasks_return_resources() {
    use crate::sched;
    use user::programs as p;
    // One round first, so the page tables a task needs exist for the comparison.
    for round in 0..2 {
        let before = user_tests::free_frames();
        user_tests::run("hello", user::hello());
        user_tests::run("faulty", p::read_kernel());
        user_tests::run("refused", p::unknown_call());
        assert_eq!(sched::thread_count(), 1);
        assert_eq!(sched::free_stack_slots(), sched::MAX_THREADS);
        if round == 1 {
            assert_eq!(
                user_tests::free_frames(),
                before,
                "running and ending tasks leaked frames"
            );
        }
    }
}

/// Hammers the handler with random and boundary arguments from inside a real
/// user address space that has one page of text mapped. It must never panic or
/// fault, must refuse everything outside the user half, and must accept exactly
/// the ranges that lie in that mapped page.
fn syscall_handler_survives_random_arguments() {
    use core::sync::atomic::Ordering;
    use durg_syscall::{number, Error, MAX_PRINT, USER_SPACE_END};

    const PAGE: u64 = 0x0000_0000_0040_0000;
    let kernel = kernel_space();
    let mut user = AddressSpace::new_user().expect("new_user failed");
    let physical = frame::alloc_zeroed_frame().expect("no frame");
    frame::write_bytes(physical, b"fuzz target page").expect("write failed");
    user.map_page(
        PAGE,
        physical,
        flags::PRESENT | flags::USER | paging::no_execute(),
    )
    .expect("map failed");
    // A page that exists in the space but is kernel-only: the handler must not
    // read it on the program's behalf.
    let hidden = frame::alloc_zeroed_frame().expect("no frame");
    user.map_page(PAGE + 0x1000, hidden, flags::PRESENT | paging::no_execute())
        .expect("map failed");

    crate::syscall::QUIET.store(true, Ordering::SeqCst);
    // SAFETY: the user space shares the kernel half; interrupts are off; the
    // kernel root is restored below.
    unsafe { user.activate() };

    let mut rng = Rng(0x1234_5678_9abc_def1);
    let interesting = [
        0,
        1,
        PAGE,
        PAGE + 0xff0,
        PAGE + 0x1000,
        PAGE + 0xfff,
        USER_SPACE_END - 1,
        USER_SPACE_END,
        USER_SPACE_END + 1,
        0x0000_ffff_ffff_ffff,
        0xffff_8000_0000_0000,
        0xffff_ffff_8000_0000,
        u64::MAX,
        u64::MAX - 7,
    ];
    let mut accepted = 0u32;
    let mut failures: alloc::vec::Vec<(u64, u64, u64)> = alloc::vec::Vec::new();
    for _ in 0..20_000 {
        let pointer = if rng.below(3) == 0 {
            rng.next()
        } else {
            let base = interesting[rng.below(interesting.len() as u64) as usize];
            base.wrapping_add(rng.below(40)).wrapping_sub(rng.below(20))
        };
        let length = match rng.below(4) {
            0 => rng.below(300),
            1 => rng.next(),
            2 => interesting[rng.below(interesting.len() as u64) as usize],
            _ => rng.below(16),
        };
        let result = crate::syscall::handle(number::DEBUG_PRINT, [pointer, length, rng.next()]);
        let in_mapped_page = length > 0
            && length <= MAX_PRINT
            && pointer >= PAGE
            && pointer
                .checked_add(length)
                .is_some_and(|end| end <= PAGE + 0x1000);
        let expected_ok = in_mapped_page || length == 0;
        if result == length && expected_ok {
            accepted += 1;
        } else if expected_ok || Error::from_return_value(result).is_none() {
            // Refused something it should have accepted, or accepted something
            // it should have refused.
            failures.push((pointer, length, result));
        }
    }
    // Unknown call numbers, with arbitrary arguments, are always refused.
    for _ in 0..2_000 {
        let nr = 3 + rng.below(1 << 20);
        assert_eq!(
            crate::syscall::handle(nr, [rng.next(), rng.next(), rng.next()]),
            Error::NoSuchCall.to_return_value()
        );
    }

    // SAFETY: restores the tables active on entry.
    unsafe { kernel.activate() };
    crate::syscall::QUIET.store(false, Ordering::SeqCst);
    user.destroy_user_half();

    assert!(
        failures.is_empty(),
        "{} wrong results, first (pointer, length, result): {:?}",
        failures.len(),
        failures.first()
    );
    assert!(
        accepted > 100,
        "only {accepted} ranges were accepted: the mapped page was barely exercised"
    );
}

// ---- SMEP, SMAP and UMIP (M8) ----------------------------------------------

/// The CPU features this build can test. A feature the CPU does not report is
/// skipped *visibly*, never passed silently.
fn protection_or_skip(name: &str, supported: bool) -> bool {
    if !supported {
        println!("(skipped: this CPU does not report {name})");
    }
    supported
}

fn cpu_protections_match_cpuid() {
    let supported = x86_64::protection::supported();
    let cr4: u64;
    // SAFETY: reading CR4 has no side effects.
    unsafe { core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack)) };
    for (name, reported, bit) in [
        ("SMEP", supported.smep, 20),
        ("SMAP", supported.smap, 21),
        ("UMIP", supported.umip, 11),
    ] {
        if protection_or_skip(name, reported) {
            assert_ne!(
                cr4 & (1 << bit),
                0,
                "{name} is supported but CR4 has it off"
            );
        }
    }
}

/// Builds a user space with one page that the user may read, and (if `code` is
/// given) executes it from, mapped at a fixed address.
fn user_space_with_page(code: Option<&[u8]>) -> (AddressSpace, u64) {
    const ADDRESS: u64 = 0x0000_0000_0040_0000;
    let mut user = AddressSpace::new_user().expect("new_user failed");
    let physical = frame::alloc_zeroed_frame().expect("no frame");
    if let Some(code) = code {
        frame::write_bytes(physical, code).expect("write failed");
    }
    let mut leaf = flags::PRESENT | flags::USER;
    if code.is_none() {
        leaf |= flags::WRITABLE | paging::no_execute();
    }
    user.map_page(ADDRESS, physical, leaf).expect("map failed");
    (user, ADDRESS)
}

fn smep_blocks_kernel_execute_of_user_pages() {
    if !protection_or_skip("SMEP", x86_64::protection::supported().smep) {
        return;
    }
    // A user page that holds a `ret`. With SMEP the kernel may not even fetch it.
    let (user, address) = user_space_with_page(Some(&[0xc3]));
    let kernel = kernel_space();
    // SAFETY: shares the kernel half; interrupts off; the kernel root is restored
    // below. The jump is refused by SMEP before any byte at the address runs.
    unsafe { user.activate() };
    let result = x86_64::trap::probe_execute(address);
    // SAFETY: restores the tables active on entry.
    unsafe { kernel.activate() };
    user.destroy_user_half();
    let fault = result.expect_err("the kernel executed a user page: SMEP is not working");
    // Instruction fetch (bit 4), protection violation (bit 0), supervisor (bit 2
    // clear): the kernel, not the user, was refused.
    assert_eq!(
        fault.error_code & 0b10101,
        0b10001,
        "wrong fault: {fault:?}"
    );
    assert_eq!(fault.address, address);
}

fn smap_blocks_accidental_user_access() {
    if !protection_or_skip("SMAP", x86_64::protection::supported().smap) {
        return;
    }
    let (user, address) = user_space_with_page(None);
    let kernel = kernel_space();
    // SAFETY: shares the kernel half; interrupts off; the kernel root is restored.
    unsafe { user.activate() };
    // Without `stac`, a kernel read of a user page faults: protection violation
    // (bit 0), read, supervisor (bit 2 clear).
    let blocked = x86_64::trap::probe_read(address);
    // With the access window open it works.
    let allowed = x86_64::protection::with_user_access(|| x86_64::trap::probe_read(address));
    // SAFETY: restores the tables active on entry.
    unsafe { kernel.activate() };
    user.destroy_user_half();
    let fault = blocked.expect_err("the kernel read a user page without stac: SMAP is not working");
    assert_eq!(fault.error_code & 0b111, 0b001, "wrong fault: {fault:?}");
    assert_eq!(allowed.expect("read inside the access window failed"), 0);
}

fn umip_blocks_sidt_in_ring_3() {
    if !protection_or_skip("UMIP", x86_64::protection::supported().umip) {
        return;
    }
    // `sidt` stores the IDT's address. With UMIP, ring 3 gets a general
    // protection fault instead of a kernel address.
    let status = user_tests::run("sidt", user::programs::read_idt());
    assert_eq!(
        status,
        user_status::Faulted {
            vector: 13,
            error: 0,
            rip: user::CODE_BASE
        }
    );
}
