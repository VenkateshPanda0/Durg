//! Locks, threads and scheduling tests (M7).
//!
//! Part of the in-kernel self-tests; see [`super`] for how they are run.

use super::*;

// ---- locks (M7) -------------------------------------------------------------

pub(super) fn interrupts_enabled() -> bool {
    let flags: u64;
    // SAFETY: `pushfq; pop` reads RFLAGS and has no other effect.
    unsafe { core::arch::asm!("pushfq", "pop {}", out(reg) flags, options(nomem)) };
    flags & (1 << 9) != 0
}

/// The host tests check the lock's logic against a fake flag. This checks it
/// against the real one: inside the lock the CPU's interrupt flag is off, and
/// afterwards it is back on if it was on and off if it was off.
pub(super) fn irq_lock_masks_real_interrupts() {
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

pub(super) mod sched_tests {
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

pub(super) fn sched_unstarted_spawn_releases_owned_spaces() {
    let before = sched_tests::free_frames();
    let space = AddressSpace::new_user().expect("user space");
    let caps = CSpace::new().expect("capability space");
    assert!(matches!(
        sched::test_spawn_not_started(space, caps),
        Err(sched::SpawnError::NotStarted)
    ));
    assert_eq!(
        sched_tests::free_frames(),
        before,
        "failed spawn leaked its spaces"
    );
}

pub(super) fn sched_spawn_join_returns_resources() {
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

pub(super) fn sched_yield_alternates() {
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

pub(super) fn sched_switch_preserves_registers() {
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

pub(super) fn sched_timer_preempts() {
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

pub(super) fn sched_lock_prevents_lost_updates() {
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

pub(super) fn sched_thread_stack_guard_pages() {
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

pub(super) fn sched_thread_limit() {
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
