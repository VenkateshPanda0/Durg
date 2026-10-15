//! Local APIC: the per-CPU interrupt controller and timer.
//!
//! The local APIC delivers interrupts to its CPU and contains a timer that can
//! fire once or periodically. Its registers are memory-mapped (the "xAPIC"
//! interface) at a physical address normally `0xfee00000`. That page is **not**
//! in the bootloader's memory map, so it is not mapped when the kernel starts.
//! [`init`] maps it uncached at a fixed kernel virtual address through the page
//! mapper in `mm::paging`, then switches the controller on.
//!
//! x2APIC (register access through MSRs, which would need no mapping) is not
//! used: QEMU's software emulation does not support it, so CI could not test it.
//!
//! At Limine base revision 4 the bootloader does not promise any local APIC
//! state (only revision 5 and later initialise it), so [`init`] sets everything
//! it relies on.
//!
//! References: Intel SDM Vol. 3 ch. 11 ("Advanced Programmable Interrupt
//! Controller"), in particular 11.4.1 (`IA32_APIC_BASE`), table 11-1 (register
//! offsets), 11.5.1 (local vector table), 11.5.4 (timer) and 11.9 (spurious
//! interrupt vector register).

use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::trap::{SPURIOUS_VECTOR, TIMER_VECTOR};
use super::{read_msr, IA32_APIC_BASE};
use crate::mm::paging::{self, MapError};

/// Fixed kernel virtual address of the local APIC register page. PML4 slot 510
/// is unused by the bootloader mappings (which use the HHDM and the -2 GiB
/// kernel window), so this window cannot collide with them.
pub const MMIO_BASE: u64 = 0xffff_ff00_0000_0000;

// Register offsets (Intel SDM Vol. 3 table 11-1).
const REG_ID: u64 = 0x20;
const REG_VERSION: u64 = 0x30;
const REG_TASK_PRIORITY: u64 = 0x80;
const REG_EOI: u64 = 0xb0;
const REG_SPURIOUS: u64 = 0xf0;
const REG_LVT_TIMER: u64 = 0x320;
const REG_TIMER_INITIAL: u64 = 0x380;
const REG_TIMER_DIVIDE: u64 = 0x3e0;

// `IA32_APIC_BASE` bits (Intel SDM Vol. 3 fig. 11-5).
const APIC_BASE_X2APIC: u64 = 1 << 10;
const APIC_BASE_ENABLE: u64 = 1 << 11;
const APIC_BASE_ADDRESS_MASK: u64 = 0x000f_ffff_ffff_f000;

const SPURIOUS_SOFTWARE_ENABLE: u32 = 1 << 8;
const LVT_MASKED: u32 = 1 << 16;
const LVT_TIMER_PERIODIC: u32 = 1 << 17;
/// Divide configuration value for "divide by 16" (Intel SDM fig. 11-10).
const DIVIDE_BY_16: u32 = 0b0011;

static INITIALISED: AtomicBool = AtomicBool::new(false);
static TICKS: AtomicU64 = AtomicU64::new(0);

/// Why the local APIC could not be brought up.
#[derive(Debug, Clone, Copy)]
pub enum InitError {
    /// `IA32_APIC_BASE` says the APIC is globally disabled.
    Disabled,
    /// The CPU is in x2APIC mode, where the memory interface is unavailable.
    X2ApicMode,
    /// Mapping the register page failed.
    Map(MapError),
    /// The page was mapped, but walking the page tables does not lead to the
    /// physical address that was meant (or the mapping is not uncached).
    MappingMismatch,
}

impl core::fmt::Display for InitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            InitError::Disabled => write!(f, "local APIC is disabled in IA32_APIC_BASE"),
            InitError::X2ApicMode => write!(f, "CPU is in x2APIC mode"),
            InitError::Map(error) => write!(f, "cannot map the register page: {error}"),
            InitError::MappingMismatch => write!(
                f,
                "the register page does not translate to the APIC base, or is cacheable"
            ),
        }
    }
}

/// Proof that the local APIC registers are mapped and the controller is
/// enabled. Only [`init`] and [`get`] create one, so holding it makes register
/// access sound.
#[derive(Clone, Copy)]
pub struct Lapic(());

/// What [`init`] found, for the boot log.
pub struct LapicInfo {
    pub physical_base: u64,
    pub id: u32,
    pub version: u32,
}

/// The local APIC, if [`init`] has succeeded.
pub fn get() -> Option<Lapic> {
    INITIALISED.load(Ordering::Acquire).then_some(Lapic(()))
}

/// Maps the register page and enables the local APIC with the timer masked.
pub fn init() -> Result<(Lapic, LapicInfo), InitError> {
    let apic_base = read_msr(IA32_APIC_BASE);
    if apic_base & APIC_BASE_ENABLE == 0 {
        return Err(InitError::Disabled);
    }
    if apic_base & APIC_BASE_X2APIC != 0 {
        return Err(InitError::X2ApicMode);
    }
    let physical_base = apic_base & APIC_BASE_ADDRESS_MASK;
    paging::map_mmio_page(MMIO_BASE, physical_base).map_err(InitError::Map)?;

    // Do not trust the write: walk the page tables back and confirm the page
    // reaches the APIC and is uncached, before any register is touched.
    let mapping = paging::translate(MMIO_BASE).map_err(|_| InitError::MappingMismatch)?;
    let uncached = mapping
        .entry
        .has(durg_paging::flags::CACHE_DISABLE | durg_paging::flags::WRITE_THROUGH);
    if mapping.physical != physical_base || !uncached {
        return Err(InitError::MappingMismatch);
    }

    let lapic = Lapic(());
    // The page is mapped, so register access is sound from here on.
    lapic.write(REG_TASK_PRIORITY, 0); // accept every priority
    lapic.write(
        REG_SPURIOUS,
        SPURIOUS_SOFTWARE_ENABLE | u32::from(SPURIOUS_VECTOR),
    );
    lapic.write(REG_LVT_TIMER, LVT_MASKED | u32::from(TIMER_VECTOR));
    INITIALISED.store(true, Ordering::Release);

    let info = LapicInfo {
        physical_base,
        id: lapic.read(REG_ID) >> 24,
        version: lapic.read(REG_VERSION),
    };
    Ok((lapic, info))
}

impl Lapic {
    fn read(self, register: u64) -> u32 {
        // SAFETY: a `Lapic` only exists after `init` mapped the register page
        // at `MMIO_BASE`, and every offset used is a 16-byte-aligned register
        // inside that page. Device registers need volatile access.
        unsafe { ptr::read_volatile((MMIO_BASE + register) as *const u32) }
    }

    fn write(self, register: u64, value: u32) {
        // SAFETY: as in `read`. Writing these registers only configures the
        // local APIC.
        unsafe { ptr::write_volatile((MMIO_BASE + register) as *mut u32, value) }
    }

    /// Starts the timer in periodic mode: it raises [`TIMER_VECTOR`] every
    /// `initial_count` timer clocks (the bus clock divided by 16).
    ///
    /// The clock rate is not calibrated yet, so a tick is not a fixed number of
    /// milliseconds. Calibration comes with the scheduler (M7).
    pub fn start_periodic_timer(self, initial_count: u32) {
        self.write(REG_TIMER_DIVIDE, DIVIDE_BY_16);
        self.write(REG_LVT_TIMER, LVT_TIMER_PERIODIC | u32::from(TIMER_VECTOR));
        // Writing the initial count is what starts the countdown.
        self.write(REG_TIMER_INITIAL, initial_count);
    }

    /// Stops the timer and masks its interrupt.
    pub fn stop_timer(self) {
        self.write(REG_LVT_TIMER, LVT_MASKED | u32::from(TIMER_VECTOR));
        self.write(REG_TIMER_INITIAL, 0);
    }

    /// Signals that the interrupt being handled is finished, so the controller
    /// can deliver the next one.
    pub fn end_of_interrupt(self) {
        self.write(REG_EOI, 0);
    }

    /// The raw version register (low byte is the version, bits 16 to 23 the
    /// highest local vector table entry).
    #[cfg(feature = "selftest")]
    pub fn version(self) -> u32 {
        self.read(REG_VERSION)
    }
}

/// Called from the interrupt dispatcher for every timer interrupt.
pub fn on_timer_interrupt() {
    TICKS.fetch_add(1, Ordering::Relaxed);
    if let Some(lapic) = get() {
        lapic.end_of_interrupt();
    }
}

/// Timer interrupts handled since boot.
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}
