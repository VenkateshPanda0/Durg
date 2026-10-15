//! Boot information: what the bootloader told us about the machine.
//!
//! M2 only reports it. The memory manager (M4) will consume the memory map.

use crate::limine::{self, MemoryKind};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;

/// Prints the memory map, the HHDM offset and the RSDP address.
pub fn print_boot_info() {
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

    match limine::rsdp_physical_address() {
        Some(address) => println!("RSDP (physical): {address:#018x}"),
        None => println!("RSDP: not provided"),
    }
}
