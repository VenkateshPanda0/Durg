//! Boot information: what the bootloader told us about the machine.
//!
//! M2 only reports it. The memory manager (M4) will consume the memory map.

use crate::acpi::{self, AcpiInfo};
use crate::limine::{self, MemoryKind};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;

/// Prints the memory map, the HHDM offset, the RSDP address and the MADT.
pub fn print_boot_info() {
    println!(
        "Limine base revision: {} loaded",
        limine::loaded_base_revision()
    );
    match limine::memory_map() {
        None => println!("Memory map: not provided by the bootloader"),
        Some(map) => {
            println!("Memory map ({} entries):", map.len());
            let mut usable_bytes: u64 = 0;
            for region in map.iter() {
                println!(
                    "  {:#018x} - {:#018x}  {:>10} KiB  {}",
                    region.base,
                    region.end(),
                    region.length / KIB,
                    region.kind.name()
                );
                if region.kind == MemoryKind::Usable {
                    usable_bytes = usable_bytes.saturating_add(region.length);
                }
            }
            println!("Usable RAM: {} MiB", usable_bytes / MIB);
        }
    }

    match limine::hhdm_offset() {
        Some(offset) => println!("HHDM offset: {offset:#018x}"),
        None => println!("HHDM offset: not provided"),
    }

    match limine::rsdp_virtual_address() {
        Some(address) => println!("RSDP (virtual): {address:#018x}"),
        None => println!("RSDP: not available"),
    }

    let apic = crate::arch::x86_64::apic_support();
    println!(
        "CPU: local APIC {}, x2APIC {}",
        if apic.local_apic { "yes" } else { "no" },
        if apic.x2apic { "yes" } else { "no" }
    );

    match acpi::discover() {
        Ok(info) => print_acpi(&info),
        Err(error) => println!("ACPI: discovery failed: {error}"),
    }
}

fn print_acpi(info: &AcpiInfo) {
    let madt = &info.madt;
    println!(
        "ACPI: RSDP revision {}, root table {}",
        info.rsdp_revision,
        if info.used_xsdt { "XSDT" } else { "RSDT" }
    );
    println!(
        "MADT: local APIC at {:#x}, {} CPU(s), {} I/O APIC(s), {} interrupt override(s), legacy PIC {}",
        madt.local_apic_address,
        madt.cpu_count,
        madt.io_apic_count,
        madt.override_count,
        if madt.flags & 1 != 0 { "present" } else { "absent" }
    );
    for io_apic in madt.io_apics() {
        println!(
            "  I/O APIC id {} at {:#x}, GSI base {}",
            io_apic.id, io_apic.address, io_apic.gsi_base
        );
    }
    for over in madt.overrides() {
        println!(
            "  override: ISA IRQ {} -> GSI {} (flags {:#x})",
            over.source, over.gsi, over.flags
        );
    }
}
