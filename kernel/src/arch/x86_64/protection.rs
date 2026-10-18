//! CPU features that stop the kernel and user programs from misusing each other's
//! memory: SMEP, SMAP and UMIP. All three are switched on through bits in `CR4`
//! and all three only exist on newer CPUs, so each is enabled only if CPUID
//! reports it and the result is reported, never assumed.
//!
//! - **SMEP** (Supervisor Mode Execution Prevention): ring 0 faults if it tries to
//!   *fetch an instruction* from a page marked user-accessible. Without it, a
//!   kernel bug that jumps through a corrupted pointer can be steered into code
//!   the attacker wrote in their own address space.
//! - **SMAP** (Supervisor Mode Access Prevention): ring 0 faults if it *reads or
//!   writes* a user-accessible page, unless the `AC` flag is set (`stac`/`clac`
//!   set and clear it). It turns "the kernel forgot to validate a user pointer
//!   and dereferenced it" from a silent bug into an immediate fault. This kernel
//!   copies user data by translating the pages itself (see `syscall.rs`), so
//!   nothing in it needs `stac` outside the tests.
//! - **UMIP** (User-Mode Instruction Prevention): ring 3 faults on `sgdt`, `sidt`,
//!   `sldt`, `smsw` and `str`. Without it, a user program can read the *kernel
//!   addresses* of the GDT and IDT.
//!
//! References: Intel SDM Vol. 3 ch. 4.6 (SMEP/SMAP in paging), ch. 2.5 (CR4),
//! Vol. 2 (`STAC`, `CLAC`).

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

const CR4_UMIP: u64 = 1 << 11;
const CR4_SMEP: u64 = 1 << 20;
const CR4_SMAP: u64 = 1 << 21;

/// CPUID leaf 7, subleaf 0: EBX bit 7 is SMEP, EBX bit 20 is SMAP, ECX bit 2 is
/// UMIP.
const LEAF7_EBX_SMEP: u32 = 1 << 7;
const LEAF7_EBX_SMAP: u32 = 1 << 20;
const LEAF7_ECX_UMIP: u32 = 1 << 2;

static SMAP_ENABLED: AtomicBool = AtomicBool::new(false);

/// Which protections are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Protections {
    pub smep: bool,
    pub smap: bool,
    pub umip: bool,
}

/// What CPUID says this CPU supports.
pub fn supported() -> Protections {
    // Leaf 7 exists only if the highest standard leaf is at least 7.
    if core::arch::x86_64::__cpuid(0).eax < 7 {
        return Protections {
            smep: false,
            smap: false,
            umip: false,
        };
    }
    let leaf = core::arch::x86_64::__cpuid_count(7, 0);
    Protections {
        smep: leaf.ebx & LEAF7_EBX_SMEP != 0,
        smap: leaf.ebx & LEAF7_EBX_SMAP != 0,
        umip: leaf.ecx & LEAF7_ECX_UMIP != 0,
    }
}

fn read_cr4() -> u64 {
    let value: u64;
    // SAFETY: reading CR4 has no side effects.
    unsafe { asm!("mov {}, cr4", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

/// Turns on every protection the CPU supports and returns what is on now, read
/// back from CR4 so the answer reflects the hardware and not the request.
pub fn enable() -> Protections {
    let supported = supported();
    let mut cr4 = read_cr4();
    if supported.smep {
        cr4 |= CR4_SMEP;
    }
    if supported.smap {
        cr4 |= CR4_SMAP;
    }
    if supported.umip {
        cr4 |= CR4_UMIP;
    }
    // SAFETY: each bit is set only if CPUID reports the feature, so no reserved
    // bit is written. Enabling these restricts what ring 0 and ring 3 may do; the
    // kernel runs only code that respects them (it never fetches from or
    // dereferences user memory outside `with_user_access`).
    unsafe { asm!("mov cr4, {}", in(reg) cr4, options(nomem, nostack, preserves_flags)) };
    let now = read_cr4();
    let on = Protections {
        smep: now & CR4_SMEP != 0,
        smap: now & CR4_SMAP != 0,
        umip: now & CR4_UMIP != 0,
    };
    SMAP_ENABLED.store(on.smap, Ordering::Release);
    on
}

/// Runs `f` with supervisor access to user memory allowed (`stac` .. `clac`).
/// Does nothing special if SMAP is off. For tests that deliberately touch a user
/// page from ring 0.
#[cfg(feature = "selftest")]
pub fn with_user_access<R>(f: impl FnOnce() -> R) -> R {
    let smap = SMAP_ENABLED.load(Ordering::Acquire);
    if smap {
        // SAFETY: SMAP is enabled, so the CPU supports `stac`; it only sets AC.
        unsafe { asm!("stac", options(nomem, nostack)) };
    }
    let result = f();
    if smap {
        // SAFETY: as above; restores AC to clear.
        unsafe { asm!("clac", options(nomem, nostack)) };
    }
    result
}
