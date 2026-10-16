//! Kernel threads and the round-robin scheduler.
//!
//! ## The model
//!
//! A **thread** is a stack plus a saved stack pointer (see `arch::x86_64::context`
//! for what a switch does). All threads share the kernel's address space. There
//! is one CPU, so exactly one thread is *running*; the others are *ready* and wait
//! in a FIFO **run queue**. **Round robin** means: when the running thread's turn
//! ends it goes to the back of the queue and the thread at the front runs next.
//!
//! A turn ends in one of three ways:
//!
//! - the **timer** ticks (*preemption*: the thread did nothing wrong, it just
//!   used up its slice; this is what makes a thread that never yields share the
//!   CPU);
//! - the thread calls [`yield_now`];
//! - the thread calls [`exit`] (it never runs again).
//!
//! ## Switching inside the interrupt handler
//!
//! A timer interrupt arrives on the running thread's own stack: the CPU pushes
//! its return frame there and our stub saves the registers there too. The
//! handler then calls [`on_tick`], which may switch to another thread **from
//! inside the handler**. The first thread's whole interrupt frame stays on its
//! own stack, frozen. When it is picked again, execution resumes in that
//! handler, which returns, and the stub restores the registers and `iretq`s back
//! to wherever the thread was interrupted. One mechanism (the stack) carries
//! both kinds of saved state.
//!
//! ## Rules this module depends on
//!
//! - `schedule` runs with **interrupts off**, so nothing can start a second
//!   switch halfway through the first. Callers disable them; the timer
//!   interrupt arrives with them off already.
//! - The scheduler lock is **not** held across the switch. Holding it would
//!   leave a lock owned by a thread that is no longer running, and the next
//!   thread would deadlock on it. The lock only guards picking the next thread;
//!   interrupts staying off is what protects the switch itself.
//! - **Nothing in the timer path allocates.** The run queue is created with room
//!   for [`MAX_THREADS`] and spawning refuses to go past that, so a push never
//!   grows it.
//! - **The boot thread is the idle thread.** It is the context `kernel_main` runs
//!   on, it is never in the dead state, and when everyone else is finished it
//!   keeps running.
//!
//! ## Stacks
//!
//! Each thread gets its own 64 KiB stack in a dedicated virtual window (PML4
//! slot 507), with an unmapped **guard page** below it, the same trick as the
//! boot stack. Stacks of finished threads are unmapped and their frames returned
//! by the *next* thread to run, never by the dying thread itself, which is still
//! standing on its stack until the switch.
//!
//! ## What it does not do
//!
//! No priorities, no sleeping or blocking (a thread that must wait polls with
//! [`yield_now`]), no thread-local storage, no per-thread FPU state (the kernel
//! uses none), no SMP, and every tick is a switch if anyone else is ready, so
//! the slice length is whatever the timer period is. The timer is not calibrated
//! (see the roadmap), so a slice is a count of timer clocks, not a duration.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use core::mem;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use durg_paging::{flags, PAGE_SIZE};

use crate::arch::x86_64::context;
use crate::arch::x86_64::{lapic, IrqLock};
use crate::mm::frame;
use crate::mm::paging::{self, AddressSpace};

/// Start of the window thread stacks live in (PML4 slot 507).
pub const STACK_WINDOW: u64 = 0xffff_fd80_0000_0000;
/// Virtual space reserved per thread: guard page, stack, and unmapped slack.
const SLOT_BYTES: u64 = 128 * 1024;
/// Pages of stack per thread.
pub const STACK_PAGES: u64 = 16;
/// Most threads that can exist at once, the boot thread included. Also the
/// number of stack slots, and the run queue's fixed capacity.
pub const MAX_THREADS: usize = 64;
/// Timer period during scheduling, in timer clocks (the bus clock divided by
/// 16). Not calibrated: a count, not a duration.
pub const SLICE_COUNT: u32 = 200_000;

/// Identifies a thread. Never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ThreadId(pub u64);

impl core::fmt::Display for ThreadId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ready,
    Running,
    Dead,
}

/// Why [`spawn`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnError {
    /// The scheduler has not been initialised.
    NotStarted,
    /// [`MAX_THREADS`] threads exist already.
    TooManyThreads,
    /// No frames for the stack, or mapping it failed.
    OutOfMemory,
}

impl core::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::NotStarted => "scheduler not initialised",
            Self::TooManyThreads => "too many threads",
            Self::OutOfMemory => "out of memory for the thread stack",
        })
    }
}

/// A thread's stack: a slot in the window, mapped when created and unmapped
/// when dropped.
struct Stack {
    slot: usize,
}

impl Stack {
    fn base(slot: usize) -> u64 {
        STACK_WINDOW + slot as u64 * SLOT_BYTES
    }

    /// Lowest mapped address (the guard page is the page below).
    fn bottom(&self) -> u64 {
        Self::base(self.slot) + PAGE_SIZE
    }

    fn top(&self) -> u64 {
        self.bottom() + STACK_PAGES * PAGE_SIZE
    }

    /// Maps the stack pages for `slot`, or maps nothing on failure.
    fn map(slot: usize) -> Result<Self, SpawnError> {
        let stack = Self { slot };
        let mut space = AddressSpace::active();
        let leaf = flags::PRESENT | flags::WRITABLE | paging::no_execute();
        for page in 0..STACK_PAGES {
            let address = stack.bottom() + page * PAGE_SIZE;
            let mapped = frame::alloc_zeroed_frame().and_then(|physical| {
                if space.map_page(address, physical, leaf).is_ok() {
                    Some(())
                } else {
                    let _ = frame::free_frame(physical);
                    None
                }
            });
            if mapped.is_none() {
                stack.unmap_first(&mut space, page);
                // The caller still owns the slot reservation. Skipping `Drop`
                // avoids a second unmap and a second release.
                mem::forget(stack);
                return Err(SpawnError::OutOfMemory);
            }
        }
        Ok(stack)
    }

    fn unmap_first(&self, space: &mut AddressSpace, pages: u64) {
        for page in 0..pages {
            if let Ok(unmapped) = space.unmap(self.bottom() + page * PAGE_SIZE) {
                let _ = frame::free_frame(unmapped.physical);
            }
        }
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        let mut space = AddressSpace::active();
        self.unmap_first(&mut space, STACK_PAGES);
        SCHED.lock().release_slot(self.slot);
    }
}

struct Thread {
    id: ThreadId,
    #[cfg_attr(not(feature = "selftest"), allow(dead_code))]
    name: &'static str,
    state: State,
    /// Where this thread's registers are saved while it is not running. The
    /// context switch writes it. The boxed thread never moves, so a raw pointer
    /// to this field stays valid for as long as the thread exists.
    saved_rsp: u64,
    /// `None` for the boot thread, which runs on the kernel's boot stack. Never
    /// read: it is here so dropping the thread unmaps the stack.
    _stack: Option<Stack>,
    /// Set when the thread finishes.
    done: Arc<AtomicBool>,
}

struct Scheduler {
    started: bool,
    current: Option<Box<Thread>>,
    run_queue: VecDeque<Box<Thread>>,
    /// A finished thread whose stack is released by the next thread to run.
    zombie: Option<Box<Thread>>,
    next_id: u64,
    /// Bit `i` set: stack slot `i` is in use.
    slots: u64,
    switches: u64,
}

impl Scheduler {
    const fn new() -> Self {
        Self {
            started: false,
            current: None,
            run_queue: VecDeque::new(),
            zombie: None,
            next_id: 0,
            slots: 0,
            switches: 0,
        }
    }

    fn take_slot(&mut self) -> Option<usize> {
        // All 64 slots in use makes `!slots` zero, whose trailing-zero count (64)
        // fails the bound check below.
        let free = (!self.slots).trailing_zeros() as usize;
        (free < MAX_THREADS).then(|| {
            self.slots |= 1 << free;
            free
        })
    }

    fn release_slot(&mut self, slot: usize) {
        self.slots &= !(1 << slot);
    }

    fn thread_count(&self) -> usize {
        usize::from(self.current.is_some()) + self.run_queue.len()
    }

    /// Chooses the next thread and makes it current. Returns the place to save
    /// the old thread's stack pointer and the new thread's stack pointer, or
    /// `None` if there is nobody else to run and the current thread can go on.
    fn pick_next(&mut self) -> Option<(*mut u64, u64)> {
        let Some(mut next) = self.run_queue.pop_front() else {
            let current = self
                .current
                .as_ref()
                .expect("scheduler has a current thread");
            assert!(
                current.state != State::Dead,
                "the last runnable thread exited: the boot thread must never exit"
            );
            return None;
        };
        next.state = State::Running;
        let load = next.saved_rsp;
        let mut old = self
            .current
            .replace(next)
            .expect("scheduler has a current thread");
        let save: *mut u64 = &mut old.saved_rsp;
        if old.state == State::Dead {
            debug_assert!(self.zombie.is_none(), "zombie not reaped");
            self.zombie = Some(old);
        } else {
            old.state = State::Ready;
            // Cannot grow: capacity was reserved for MAX_THREADS in `init`.
            self.run_queue.push_back(old);
        }
        self.switches += 1;
        CURRENT_ID.store(
            self.current.as_ref().map_or(0, |t| t.id.0),
            Ordering::Relaxed,
        );
        Some((save, load))
    }
}

static SCHED: IrqLock<Scheduler> = IrqLock::new(Scheduler::new());
static CURRENT_ID: AtomicU64 = AtomicU64::new(0);
/// Whether timer ticks may switch threads.
static PREEMPT: AtomicBool = AtomicBool::new(false);

/// Runs `f` with interrupts off, putting them back as they were.
fn without_interrupts<R>(f: impl FnOnce() -> R) -> R {
    use durg_sync::InterruptControl;
    let were_enabled = crate::arch::x86_64::Interrupts::disable();
    let result = f();
    if were_enabled {
        // SAFETY: undoes the `disable` above, which found interrupts on.
        unsafe { crate::arch::x86_64::Interrupts::enable() };
    }
    result
}

/// Makes the code that is running now (the boot thread) thread 0 and prepares
/// the run queue. Call once, after the heap.
pub fn init() {
    let mut scheduler = SCHED.lock();
    assert!(!scheduler.started, "scheduler initialised twice");
    scheduler.run_queue.reserve(MAX_THREADS);
    let id = ThreadId(scheduler.next_id);
    scheduler.next_id += 1;
    scheduler.current = Some(Box::new(Thread {
        id,
        name: "boot",
        state: State::Running,
        saved_rsp: 0,
        _stack: None,
        done: Arc::new(AtomicBool::new(false)),
    }));
    scheduler.started = true;
    CURRENT_ID.store(id.0, Ordering::Relaxed);
}

/// A handle to wait for a thread to finish.
pub struct JoinHandle {
    #[cfg_attr(not(feature = "selftest"), allow(dead_code))]
    id: ThreadId,
    done: Arc<AtomicBool>,
}

impl JoinHandle {
    #[cfg_attr(not(feature = "selftest"), allow(dead_code))]
    pub fn id(&self) -> ThreadId {
        self.id
    }

    pub fn is_finished(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }

    /// Lets other threads run until this one has finished.
    pub fn join(self) {
        while !self.is_finished() {
            yield_now();
        }
    }
}

/// What every new thread starts in: runs the entry function, then exits.
extern "C" fn thread_start(entry: u64) -> ! {
    // SAFETY: `entry` was made from a `fn()` in `spawn`; the value round-trips
    // through a register unchanged.
    let entry: fn() = unsafe { mem::transmute::<usize, fn()>(entry as usize) };
    entry();
    exit()
}

/// Creates a thread that will run `entry` and puts it at the back of the run
/// queue. It starts the next time the scheduler picks it.
pub fn spawn(name: &'static str, entry: fn()) -> Result<JoinHandle, SpawnError> {
    // Reserve the slot and map the stack first, outside the scheduler lock, so
    // the lock never covers page-table work. Lock order: scheduler before heap
    // before page tables before frames, but a short critical section is better
    // than a long one.
    let slot = {
        let mut scheduler = SCHED.lock();
        if !scheduler.started {
            return Err(SpawnError::NotStarted);
        }
        if scheduler.thread_count() + usize::from(scheduler.zombie.is_some()) >= MAX_THREADS {
            return Err(SpawnError::TooManyThreads);
        }
        scheduler.take_slot().ok_or(SpawnError::TooManyThreads)?
    };
    let stack = match Stack::map(slot) {
        Ok(stack) => stack,
        Err(error) => {
            SCHED.lock().release_slot(slot);
            return Err(error);
        }
    };
    // SAFETY: the stack was just mapped read/write, so the 64 bytes below its
    // top are valid, and nobody else uses it. The top is page aligned.
    let saved_rsp = unsafe {
        context::prepare_stack(
            stack.top(),
            thread_start,
            entry as *const () as usize as u64,
        )
    };
    let done = Arc::new(AtomicBool::new(false));
    let mut scheduler = SCHED.lock();
    let id = ThreadId(scheduler.next_id);
    scheduler.next_id += 1;
    scheduler.run_queue.push_back(Box::new(Thread {
        id,
        name,
        state: State::Ready,
        saved_rsp,
        _stack: Some(stack),
        done: Arc::clone(&done),
    }));
    Ok(JoinHandle { id, done })
}

/// Picks the next thread and switches to it. Interrupts must be off.
fn schedule() {
    debug_assert!(
        !interrupts_enabled(),
        "schedule() called with interrupts on"
    );
    // Release the stack of a thread that finished earlier. This thread is not
    // it, so its stack is not in use.
    let zombie = SCHED.lock().zombie.take();
    drop(zombie);

    let switch = SCHED.lock().pick_next();
    if let Some((save, load)) = switch {
        // SAFETY: interrupts are off (asserted above), `save` points into a
        // boxed thread that lives at least until it is resumed (a dead thread
        // is held as the zombie, released only after the next switch), and
        // `load` came from `prepare_stack` or an earlier switch away from that
        // thread, for a stack that is still mapped.
        unsafe { context::switch_context(save, load) };
    }
}

fn interrupts_enabled() -> bool {
    let flags: u64;
    // SAFETY: `pushfq; pop` reads RFLAGS and nothing else.
    unsafe { core::arch::asm!("pushfq", "pop {}", out(reg) flags, options(nomem)) };
    flags & (1 << 9) != 0
}

/// Gives up the rest of this thread's turn.
pub fn yield_now() {
    without_interrupts(schedule);
}

/// Ends the current thread. Never returns.
pub fn exit() -> ! {
    crate::arch::x86_64::disable_interrupts();
    {
        let mut scheduler = SCHED.lock();
        let current = scheduler
            .current
            .as_mut()
            .expect("scheduler has a current thread");
        current.state = State::Dead;
        current.done.store(true, Ordering::Release);
    }
    schedule();
    unreachable!("a dead thread was resumed");
}

/// Called by the timer interrupt handler, after the end-of-interrupt signal, with
/// interrupts off. Ends the running thread's turn if anyone else is ready.
pub fn on_tick() {
    if PREEMPT.load(Ordering::Relaxed) {
        schedule();
    }
}

/// Starts the periodic timer and allows ticks to switch threads.
pub fn start_preemption() {
    PREEMPT.store(true, Ordering::SeqCst);
    if let Some(apic) = lapic::get() {
        apic.start_periodic_timer(SLICE_COUNT);
    }
}

/// Stops the timer and forbids preemption.
pub fn stop_preemption() {
    if let Some(apic) = lapic::get() {
        apic.stop_timer();
    }
    PREEMPT.store(false, Ordering::SeqCst);
}

/// Identity of the running thread.
#[cfg_attr(not(feature = "selftest"), allow(dead_code))]
pub fn current_id() -> ThreadId {
    ThreadId(CURRENT_ID.load(Ordering::Relaxed))
}

/// Name of the running thread.
#[cfg_attr(not(feature = "selftest"), allow(dead_code))]
pub fn current_name() -> &'static str {
    SCHED.lock().current.as_ref().map_or("?", |t| t.name)
}

/// Context switches since boot.
pub fn switches() -> u64 {
    SCHED.lock().switches
}

/// Threads that exist now (the boot thread included, finished-but-unreaped
/// threads excluded).
#[cfg_attr(not(feature = "selftest"), allow(dead_code))]
pub fn thread_count() -> usize {
    SCHED.lock().thread_count()
}

/// Free stack slots: every slot is free when no thread but the boot thread
/// exists and no zombie is waiting.
#[cfg_attr(not(feature = "selftest"), allow(dead_code))]
pub fn free_stack_slots() -> usize {
    MAX_THREADS - SCHED.lock().slots.count_ones() as usize
}

/// Whether `address` is the guard page of some thread stack. For fault
/// diagnostics.
#[cfg_attr(not(feature = "selftest"), allow(dead_code))]
pub fn in_thread_stack_guard(address: u64) -> bool {
    if !(STACK_WINDOW..STACK_WINDOW + MAX_THREADS as u64 * SLOT_BYTES).contains(&address) {
        return false;
    }
    (address - STACK_WINDOW) % SLOT_BYTES < PAGE_SIZE
}

/// Where a thread stack's pieces are, for the thread stack that contains
/// `address` (any address in its slot): `(guard, bottom, top)`.
#[cfg_attr(not(feature = "selftest"), allow(dead_code))]
pub fn stack_bounds(address: u64) -> Option<(u64, u64, u64)> {
    if !(STACK_WINDOW..STACK_WINDOW + MAX_THREADS as u64 * SLOT_BYTES).contains(&address) {
        return None;
    }
    let base = STACK_WINDOW + (address - STACK_WINDOW) / SLOT_BYTES * SLOT_BYTES;
    let bottom = base + PAGE_SIZE;
    Some((base, bottom, bottom + STACK_PAGES * PAGE_SIZE))
}

/// What [`preemption_check`] measured.
pub struct PreemptionReport {
    /// How many times each worker ran its counter loop.
    pub counts: [u64; WORKERS],
    /// Context switches during the check.
    pub switches: u64,
}

/// Workers in the preemption check.
pub const WORKERS: usize = 3;

static WORKER_COUNT: [AtomicU64; WORKERS] = [const { AtomicU64::new(0) }; WORKERS];
static WORKER_STOP: AtomicBool = AtomicBool::new(false);

/// A thread that only counts. It never yields, never blocks and never calls
/// anything that could switch: if the other workers (and the thread that started
/// them) make progress while this one spins, only the timer interrupt can be
/// the reason.
fn worker<const INDEX: usize>() {
    while !WORKER_STOP.load(Ordering::Relaxed) {
        WORKER_COUNT[INDEX].fetch_add(1, Ordering::Relaxed);
    }
}

/// Timer ticks the check waits for before giving up.
const CHECK_TICK_BUDGET: u64 = 2_000;

/// Starts [`WORKERS`] threads that spin without ever yielding and waits until
/// each has made progress. Succeeds only if the timer preempted them.
///
/// Runs on the boot thread with preemption temporarily on.
pub fn preemption_check() -> Result<PreemptionReport, &'static str> {
    use crate::arch::x86_64::enable_and_halt;

    for count in &WORKER_COUNT {
        count.store(0, Ordering::SeqCst);
    }
    WORKER_STOP.store(false, Ordering::SeqCst);
    let entries: [fn(); WORKERS] = [worker::<0>, worker::<1>, worker::<2>];
    let mut handles = alloc::vec::Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let name = ["worker-0", "worker-1", "worker-2"][index];
        handles.push(spawn(name, entry).map_err(|_| "could not start a worker thread")?);
    }

    let switches_before = switches();
    let start_tick = lapic::ticks();
    start_preemption();
    let mut all_ran = false;
    while lapic::ticks() - start_tick < CHECK_TICK_BUDGET {
        if WORKER_COUNT.iter().all(|c| c.load(Ordering::Relaxed) > 0) {
            all_ran = true;
            break;
        }
        enable_and_halt();
    }
    WORKER_STOP.store(true, Ordering::SeqCst);
    for handle in handles {
        handle.join();
    }
    stop_preemption();

    if !all_ran {
        return Err("a worker never ran: the timer did not preempt the others");
    }
    Ok(PreemptionReport {
        counts: core::array::from_fn(|i| WORKER_COUNT[i].load(Ordering::Relaxed)),
        switches: switches() - switches_before,
    })
}
