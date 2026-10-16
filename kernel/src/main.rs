//! # Durg kernel entry (M5: the kernel owns its address space)
//!
//! Boot has two stages.
//!
//! **Stage 1, [`_start`], runs on the bootloader's stack and page tables.** It
//! brings up the serial console, copies what it needs out of the bootloader's
//! responses, loads its own GDT, TSS and IDT (so CPU exceptions print
//! diagnostics instead of triple-faulting), masks the legacy PIC, prints the
//! boot information, builds the physical frame allocator, then builds its own
//! page tables, verifies them and switches to them. Finally it moves to a kernel
//! stack with a guard page.
//!
//! **Stage 2, [`kernel_main`], runs on the kernel's own stack and page tables.**
//! It gives the bootloader's memory back to the frame allocator, maps and starts
//! the local APIC timer, then either runs the self-tests or exits QEMU through
//! the `isa-debug-exit` device so a test can read pass or fail from the exit
//! code.
//!
//! Nothing here is a security property yet.

#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

extern crate alloc;

#[macro_use]
mod console;

mod acpi;
mod arch;
mod boot;
mod limine;
mod mm;
mod qemu;
#[cfg(feature = "selftest")]
mod selftest;

use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

/// Kernel entry point, called by the bootloader.
///
/// The bootloader has already switched to 64-bit mode and set up a stack, so
/// plain Rust can run from the first instruction.
#[no_mangle]
extern "C" fn _start() -> ! {
    let uart_ok = arch::serial::init();

    if !limine::bootloader_processed_tag() {
        // No Limine-protocol bootloader touched our tag, so none of the boot
        // information can be trusted.
        error!("not booted by a Limine-protocol bootloader");
        qemu::exit(qemu::ExitCode::Failure);
    }
    // Copy what we need out of the bootloader's responses before anything else
    // can overwrite the memory they live in.
    if let Err(error) = limine::capture() {
        error!("cannot read the bootloader's boot information: {error}");
        qemu::exit(qemu::ExitCode::Failure);
    }
    if limine::loaded_base_revision() < 4 {
        // Legitimate fallback, but ACPI is unreadable without revision 4.
        warn!(
            "bootloader loaded us as base revision {}, not 4: ACPI will be unavailable",
            limine::loaded_base_revision()
        );
    }

    println!("Durg kernel: boot ok");
    info!(
        "serial: COM1 16550 loopback self-test {}",
        if uart_ok { "passed" } else { "FAILED" }
    );

    // Load our descriptor tables before anything else can go wrong, so a fault
    // during the rest of boot is reported instead of resetting the machine.
    arch::x86_64::init();
    info!("cpu: GDT, TSS and IDT loaded, double fault on IST1 stack, legacy PIC masked");

    boot::print_boot_info();
    start_frame_allocator();
    start_virtual_memory();
    switch_to_kernel_stack()
}

/// Builds the kernel's own page tables and switches to them. If the checks fail
/// the kernel keeps running on the bootloader's tables and says why.
fn start_virtual_memory() {
    match mm::vm::take_over() {
        Ok(info) => {
            println!(
                "VM: kernel page tables active (root {:#x}, {} table frames); \
                 W^X: code R+X, rodata R, data+bss RW, none writable and executable",
                info.root, info.table_frames
            );
            for section in mm::vm::sections() {
                println!(
                    "VM:   {:>8} {:#x}..{:#x} ({} KiB)",
                    section.name,
                    section.start,
                    section.end,
                    (section.end - section.start) / 1024
                );
            }
        }
        Err(error) => error!("VM: kept the bootloader's page tables: {error}"),
    }
}

/// Moves to a kernel stack with a guard page and continues in [`kernel_main`].
/// Never returns: the bootloader's stack is abandoned.
fn switch_to_kernel_stack() -> ! {
    let stack = match mm::vm::build_stack() {
        Ok(stack) => stack,
        Err(error) => {
            error!("VM: cannot build the kernel stack: {error}");
            qemu::exit(qemu::ExitCode::Failure);
        }
    };
    println!(
        "VM: kernel stack {:#x}..{:#x} ({} KiB), guard page {:#x}..{:#x} unmapped",
        stack.bottom,
        stack.top,
        (stack.top - stack.bottom) / 1024,
        stack.guard,
        stack.bottom
    );
    // SAFETY: `build_stack` mapped the stack writable, its top is page aligned
    // (so 16-byte aligned), nothing else uses it, and `kernel_main` never
    // returns and takes its data from statics, not from the old stack.
    unsafe { mm::vm::run_on_stack(stack.top, kernel_main) }
}

/// Stage 2 of boot, on the kernel's own stack.
extern "C" fn kernel_main() -> ! {
    // The bootloader's page tables, boot stack and response structures are no
    // longer in use, so their memory can join the pool.
    match mm::frame::reclaim_bootloader_memory() {
        Ok(frames) => println!(
            "PMM: reclaimed {frames} frames ({} MiB) of bootloader memory",
            frames * 4096 / (1024 * 1024)
        ),
        Err(error) => error!("PMM: could not reclaim bootloader memory: {error}"),
    }
    match mm::vm::map_unmap_round_trip() {
        Ok(()) => println!("VM: map/write/read/unmap round trip ok"),
        Err(why) => error!("VM: map/unmap round trip failed: {why}"),
    }
    start_heap();
    start_local_apic();
    count_timer_ticks();
    after_boot()
}

/// Builds the physical frame allocator and checks one allocate/free round trip.
fn start_frame_allocator() {
    use mm::frame;

    const MIB: u64 = 1024 * 1024;
    match frame::init() {
        Ok(pool) => println!(
            "PMM: {} frames ({} MiB) managed, {} free; bitmaps {} KiB at physical {:#x}; \
             memory below 1 MiB, the kernel and bootloader memory are not managed",
            pool.managed,
            pool.managed * 4096 / MIB,
            pool.free,
            pool.bitmap_bytes.div_ceil(1024),
            pool.bitmap_physical
        ),
        Err(error) => {
            error!("frame allocator init failed: {error}");
            return;
        }
    }

    // One allocation and release, with the counters checked, so a basic fault in
    // the allocator shows up on every boot and not only in the self-tests.
    let before = frame::stats().map(|s| s.free);
    let round_trip = frame::alloc_frame().map(|physical| {
        let during = frame::stats().map(|s| s.free);
        (physical, during, frame::free_frame(physical))
    });
    let after = frame::stats().map(|s| s.free);
    match (before, round_trip, after) {
        (Some(b), Some((physical, Some(d), Ok(()))), Some(a)) if d + 1 == b && a == b => {
            println!("PMM: allocate/free round trip ok (frame {physical:#x})");
        }
        other => error!("PMM: allocate/free round trip failed: {other:?}"),
    }
}

/// Maps the initial kernel heap and checks that `alloc` collections work.
fn start_heap() {
    use alloc::boxed::Box;
    use alloc::vec::Vec;

    match mm::heap::init() {
        Ok(stats) => println!(
            "heap: {} KiB mapped at {:#x}, {} KiB free; limit {} MiB, grows on demand",
            stats.mapped_bytes / 1024,
            mm::heap::HEAP_BASE,
            stats.heap.free_bytes / 1024,
            mm::heap::MAX_BYTES / (1024 * 1024)
        ),
        Err(error) => {
            error!("heap init failed: {error}");
            return;
        }
    }
    let boxed = Box::new(0x5eed_u64);
    let numbers: Vec<u64> = (0..100).collect();
    if *boxed == 0x5eed && numbers.iter().sum::<u64>() == 4950 {
        println!("heap: Box and Vec round trip ok");
    } else {
        error!("heap: Box/Vec round trip gave wrong values");
    }
    drop((boxed, numbers));
    match mm::heap::check_invariants() {
        Ok(()) => println!("heap: free list consistent"),
        Err(why) => error!("heap: free list corrupted: {why}"),
    }
}

/// Maps the local APIC registers and enables the controller (timer left off).
fn start_local_apic() {
    match arch::x86_64::lapic::init() {
        Ok((_, apic)) => println!(
            "LAPIC: id {}, version {:#x}, registers at physical {:#x} mapped at {:#x} (uncached)",
            apic.id,
            apic.version,
            apic.physical_base,
            arch::x86_64::lapic::MMIO_BASE
        ),
        Err(error) => error!("LAPIC init failed: {error}"),
    }
}

/// Timer interrupts the boot-time check waits for.
const TICKS_TO_COUNT: u64 = 5;

/// Initial count for the boot-time check, in timer clocks (the bus clock divided
/// by 16). The rate is not calibrated yet, so this is a count, not a duration.
const TIMER_INITIAL_COUNT: u32 = 1_000_000;

/// Starts the local APIC timer, waits for a few interrupts, then stops it.
///
/// This is the proof that interrupts work end to end: the timer fires, the CPU
/// looks up the vector in our IDT, our stub and handler run, the end-of-interrupt
/// signal is sent, and execution resumes. If no interrupt ever arrives this waits
/// forever, and the host-side boot test fails on its timeout.
fn count_timer_ticks() {
    use arch::x86_64::{disable_interrupts, enable_and_halt, lapic};

    let Some(local_apic) = lapic::get() else {
        warn!("timer: local APIC unavailable, skipping the tick check");
        return;
    };
    let start = lapic::ticks();
    local_apic.start_periodic_timer(TIMER_INITIAL_COUNT);
    while lapic::ticks() - start < TICKS_TO_COUNT {
        enable_and_halt();
    }
    disable_interrupts();
    local_apic.stop_timer();
    println!(
        "timer: {} ticks counted from the local APIC periodic timer (initial count {})",
        lapic::ticks() - start,
        TIMER_INITIAL_COUNT
    );
}

/// Trigger a double fault on purpose (the `df-test` variant).
#[cfg(feature = "df-test")]
fn after_boot() -> ! {
    println!("df-test: triggering a double fault");
    arch::x86_64::trap::trigger_double_fault()
}

/// Overflow the kernel stack on purpose (the `so-test` variant).
#[cfg(all(feature = "so-test", not(feature = "df-test")))]
fn after_boot() -> ! {
    println!("so-test: overflowing the kernel stack");
    arch::x86_64::trap::trigger_stack_overflow()
}

/// Run the heap out of memory on purpose (the `oom-test` variant).
///
/// A fallible request must fail without harming the heap. Then an infallible
/// one that cannot be served must end in the allocation-failure panic, which
/// the host-side test expects (exit status: failure, message: "memory
/// allocation of N bytes failed").
#[cfg(all(
    feature = "oom-test",
    not(feature = "df-test"),
    not(feature = "so-test")
))]
fn after_boot() -> ! {
    use alloc::vec::Vec;
    use core::hint::black_box;

    const HUGE: usize = 1 << 30;
    println!("oom-test: asking for {} MiB with try_reserve", HUGE >> 20);
    let mut fallible: Vec<u8> = Vec::new();
    match fallible.try_reserve(HUGE) {
        Err(_) => println!("oom-test: try_reserve failed gracefully"),
        Ok(()) => error!("oom-test: try_reserve unexpectedly succeeded"),
    }
    match mm::heap::check_invariants() {
        Ok(()) => println!("oom-test: heap intact after the failed request"),
        Err(why) => error!("oom-test: heap corrupted: {why}"),
    }
    println!(
        "oom-test: asking for {} MiB with no way to fail",
        HUGE >> 20
    );
    let doomed: Vec<u8> = black_box(Vec::with_capacity(black_box(HUGE)));
    // Unreachable: the allocation above panics. Keep the value alive so the
    // request cannot be optimised away.
    println!("oom-test: unexpectedly got {} bytes", doomed.capacity());
    qemu::exit(qemu::ExitCode::Success)
}

/// Run the self-tests.
#[cfg(all(
    feature = "selftest",
    not(feature = "df-test"),
    not(feature = "so-test"),
    not(feature = "oom-test")
))]
fn after_boot() -> ! {
    selftest::run_all()
}

/// Nothing more to do yet: exit.
#[cfg(not(any(
    feature = "selftest",
    feature = "df-test",
    feature = "so-test",
    feature = "oom-test"
)))]
fn after_boot() -> ! {
    qemu::exit(qemu::ExitCode::Success)
}

/// Set once a panic starts, so a panic while panicking cannot loop.
static PANICKING: AtomicBool = AtomicBool::new(false);

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    if PANICKING.swap(true, Ordering::SeqCst) {
        qemu::exit(qemu::ExitCode::Failure);
    }
    // `PanicInfo` prints the source location and the message.
    println!("PANIC: {info}");
    qemu::exit(qemu::ExitCode::Failure)
}
