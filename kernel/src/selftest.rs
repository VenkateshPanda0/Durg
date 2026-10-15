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
use crate::mm::paging;
use crate::qemu::{self, ExitCode};

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
