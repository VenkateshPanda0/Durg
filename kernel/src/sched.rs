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
use alloc::vec::Vec;
use core::mem;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use durg_paging::{flags, PAGE_SIZE};

use crate::arch::x86_64::{self, context, lapic, IrqLock};
use crate::cspace::CSpace;
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
    /// Waiting for another thread (see the IPC section below). Not in the run
    /// queue; held in `Scheduler::blocked` until woken.
    Blocked,
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

/// Where a user thread starts in ring 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserEntry {
    pub rip: u64,
    pub rsp: u64,
}

/// How a thread ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    /// It called `exit` (or returned) with this code.
    Exited(u64),
    /// The CPU raised this exception vector while it ran in ring 3, and the
    /// kernel ended the task. `error` is the exception's error code (a page
    /// fault's code says what kind of access failed).
    Faulted { vector: u64, error: u64, rip: u64 },
    /// Someone asked for the task to be ended (see [`JoinHandle::kill`]).
    Killed,
}

/// Stored in the `faulted` field when the task was killed on request.
const KILLED_MARK: u64 = u64::MAX;

/// What a thread leaves behind for whoever waits for it.
struct ExitInfo {
    done: AtomicBool,
    /// 0 while running or after a clean exit; [`KILLED_MARK`] if killed;
    /// otherwise the vector plus one.
    faulted: AtomicU64,
    /// The exit code, or the exception's error code for a faulted task.
    code: AtomicU64,
    /// Where the faulting instruction was, for a faulted task.
    fault_rip: AtomicU64,
    /// Set by [`JoinHandle::kill`]; acted on by the next timer tick that finds
    /// the task running in ring 3.
    kill_requested: AtomicBool,
}

impl ExitInfo {
    fn new() -> Self {
        Self {
            done: AtomicBool::new(false),
            faulted: AtomicU64::new(0),
            code: AtomicU64::new(0),
            fault_rip: AtomicU64::new(0),
            kill_requested: AtomicBool::new(false),
        }
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
    /// `None` for the boot thread, which runs on the kernel's boot stack. This
    /// is the thread's *kernel* stack; a user thread has a separate user stack
    /// in its own address space.
    stack: Option<Stack>,
    /// The thread's own address space, for a user thread. `None` means it runs
    /// in the kernel's tables.
    space: Option<AddressSpace>,
    /// Where to enter ring 3, for a user thread.
    user: Option<UserEntry>,
    /// The capabilities this thread holds. A thread with none can name nothing.
    cspace: Option<CSpace>,
    /// What this thread is waiting for, and the messages passing through it.
    ipc: IpcState,
    /// Filled in when the thread finishes.
    exit: Arc<ExitInfo>,
}

impl Drop for Thread {
    fn drop(&mut self) {
        // Runs when a finished thread is reaped, on another thread's stack. The
        // thread is no longer running, so its address space is not the active
        // one. Frees every user page and table (not the shared kernel half).
        if let Some(space) = self.space.take() {
            space.destroy_user_half();
        }
        // Delete every capability and return the memory the task owned.
        if let Some(cspace) = self.cspace.take() {
            cspace.destroy();
        }
    }
}

/// What the scheduler needs done for the switch it chose.
struct Switch {
    save: *mut u64,
    load: u64,
    /// Root of the address space the next thread runs in.
    cr3: u64,
    /// Where interrupts and `syscall` from ring 3 should find the next thread's
    /// kernel stack. `None` for the boot thread, which never enters ring 3.
    kernel_stack_top: Option<u64>,
}

struct Scheduler {
    started: bool,
    current: Option<Box<Thread>>,
    run_queue: VecDeque<Box<Thread>>,
    /// A finished thread whose stack is released by the next thread to run.
    zombie: Option<Box<Thread>>,
    /// Threads waiting for a message or a reply. Capacity is reserved in `init`,
    /// so blocking never allocates. Boxed on purpose, like the run queue: the
    /// context switch keeps a raw pointer into each thread, so a thread must never
    /// move, and shuffling this vector moves only the boxes.
    #[allow(clippy::vec_box)]
    blocked: Vec<Box<Thread>>,
    /// Orders the waiters on one endpoint: lowest first.
    ipc_seq: u64,
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
            blocked: Vec::new(),
            ipc_seq: 0,
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
        usize::from(self.current.is_some()) + self.run_queue.len() + self.blocked.len()
    }

    /// Chooses the next thread and makes it current. Returns the place to save
    /// the old thread's stack pointer and the new thread's stack pointer, or
    /// `None` if there is nobody else to run and the current thread can go on.
    fn pick_next(&mut self) -> Option<Switch> {
        let Some(mut next) = self.run_queue.pop_front() else {
            let current = self
                .current
                .as_ref()
                .expect("scheduler has a current thread");
            assert!(
                current.state != State::Dead,
                "the last runnable thread exited: the boot thread must never exit"
            );
            assert!(
                current.state != State::Blocked,
                "deadlock: every thread is waiting for another"
            );
            return None;
        };
        next.state = State::Running;
        let load = next.saved_rsp;
        let cr3 = next
            .space
            .as_ref()
            .map_or_else(crate::mm::vm::kernel_root, AddressSpace::root);
        let kernel_stack_top = next.stack.as_ref().map(Stack::top);
        let mut old = self
            .current
            .replace(next)
            .expect("scheduler has a current thread");
        let save: *mut u64 = &mut old.saved_rsp;
        if old.state == State::Dead {
            debug_assert!(self.zombie.is_none(), "zombie not reaped");
            self.zombie = Some(old);
        } else if old.state == State::Blocked {
            // Cannot grow: capacity was reserved for MAX_THREADS in `init`.
            self.blocked.push(old);
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
        Some(Switch {
            save,
            load,
            cr3,
            kernel_stack_top,
        })
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
    scheduler.blocked.reserve(MAX_THREADS);
    let id = ThreadId(scheduler.next_id);
    scheduler.next_id += 1;
    scheduler.current = Some(Box::new(Thread {
        id,
        name: "boot",
        state: State::Running,
        saved_rsp: 0,
        stack: None,
        space: None,
        user: None,
        exit: Arc::new(ExitInfo::new()),
        cspace: None,
        ipc: IpcState::default(),
    }));
    scheduler.started = true;
    CURRENT_ID.store(id.0, Ordering::Relaxed);
}

/// A handle to wait for a thread to finish.
pub struct JoinHandle {
    #[cfg_attr(not(feature = "selftest"), allow(dead_code))]
    id: ThreadId,
    exit: Arc<ExitInfo>,
}

impl JoinHandle {
    #[cfg_attr(not(feature = "selftest"), allow(dead_code))]
    pub fn id(&self) -> ThreadId {
        self.id
    }

    pub fn is_finished(&self) -> bool {
        self.exit.done.load(Ordering::Acquire)
    }

    /// How the thread ended, or `None` while it is still running.
    #[cfg_attr(not(feature = "selftest"), allow(dead_code))]
    pub fn status(&self) -> Option<ExitStatus> {
        if !self.is_finished() {
            return None;
        }
        Some(match self.exit.faulted.load(Ordering::Acquire) {
            0 => ExitStatus::Exited(self.exit.code.load(Ordering::Acquire)),
            KILLED_MARK => ExitStatus::Killed,
            vector_plus_one => ExitStatus::Faulted {
                vector: vector_plus_one - 1,
                error: self.exit.code.load(Ordering::Acquire),
                rip: self.exit.fault_rip.load(Ordering::Acquire),
            },
        })
    }

    /// Asks for the task to be ended. A user task is ended by the next timer tick
    /// that finds it in ring 3, so a task that never makes a system call and
    /// never returns to the kernel on its own can still be stopped. A task
    /// running kernel code is not interrupted: it finishes its kernel work first.
    #[cfg_attr(not(feature = "selftest"), allow(dead_code))]
    pub fn kill(&self) {
        self.exit.kill_requested.store(true, Ordering::Release);
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
    spawn_inner(name, entry, None, None, None)
}

#[cfg_attr(not(feature = "selftest"), allow(dead_code))]
/// Creates a thread that runs in its own address space and enters ring 3 at
/// `entry`. It starts in the kernel (on its own kernel stack, already in its
/// address space) and drops to user mode from there.
///
/// The space is destroyed, with every page it maps, when the thread is reaped.
pub fn spawn_user(
    name: &'static str,
    space: AddressSpace,
    entry: UserEntry,
) -> Result<JoinHandle, SpawnError> {
    spawn_inner(name, user_thread_main, Some(space), Some(entry), None)
}

/// Like [`spawn_user`], and the task holds the capabilities in `cspace`. If the
/// spawn fails, the capability space is destroyed too.
pub fn spawn_user_with_cspace(
    name: &'static str,
    space: AddressSpace,
    entry: UserEntry,
    cspace: CSpace,
) -> Result<JoinHandle, SpawnError> {
    spawn_inner(
        name,
        user_thread_main,
        Some(space),
        Some(entry),
        Some(cspace),
    )
}

/// The kernel half of a user thread: leaves for ring 3. Never returns.
fn user_thread_main() {
    let entry = SCHED
        .lock()
        .current
        .as_ref()
        .and_then(|thread| thread.user)
        .expect("user thread without an entry point");
    // SAFETY: the thread's address space (already loaded by the scheduler) maps
    // the entry point as user-executable and the stack as user-writable, which
    // the task loader checked when it built them.
    unsafe { crate::arch::x86_64::enter_user(entry.rip, entry.rsp) }
}

fn spawn_inner(
    name: &'static str,
    entry: fn(),
    space: Option<AddressSpace>,
    user: Option<UserEntry>,
    cspace: Option<CSpace>,
) -> Result<JoinHandle, SpawnError> {
    // Reserve the slot and map the stack first, outside the scheduler lock, so
    // the lock never covers page-table work. Lock order: scheduler before heap
    // before page tables before frames, but a short critical section is better
    // than a long one.
    let slot = {
        let mut scheduler = SCHED.lock();
        if !scheduler.started {
            discard(space, cspace);
            return Err(SpawnError::NotStarted);
        }
        if scheduler.thread_count() + usize::from(scheduler.zombie.is_some()) >= MAX_THREADS {
            drop(scheduler);
            discard(space, cspace);
            return Err(SpawnError::TooManyThreads);
        }
        match scheduler.take_slot() {
            Some(slot) => slot,
            None => {
                drop(scheduler);
                discard(space, cspace);
                return Err(SpawnError::TooManyThreads);
            }
        }
    };
    let stack = match Stack::map(slot) {
        Ok(stack) => stack,
        Err(error) => {
            SCHED.lock().release_slot(slot);
            discard(space, cspace);
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
    let exit = Arc::new(ExitInfo::new());
    let mut scheduler = SCHED.lock();
    let id = ThreadId(scheduler.next_id);
    scheduler.next_id += 1;
    scheduler.run_queue.push_back(Box::new(Thread {
        id,
        name,
        state: State::Ready,
        saved_rsp,
        stack: Some(stack),
        space,
        user,
        cspace,
        ipc: IpcState::default(),
        exit: Arc::clone(&exit),
    }));
    Ok(JoinHandle { id, exit })
}

/// Frees an address space and capability space that were handed to a spawn but
/// never became part of a thread, so a failed spawn does not leak the task's
/// pages or capabilities.
fn discard(space: Option<AddressSpace>, cspace: Option<CSpace>) {
    if let Some(space) = space {
        space.destroy_user_half();
    }
    if let Some(cspace) = cspace {
        cspace.destroy();
    }
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
    if let Some(switch) = switch {
        // Load the next thread's address space *before* switching stacks. Both
        // spaces share the kernel half, which holds this code, the scheduler's
        // data and both kernel stacks, so execution continues across the load.
        if x86_64::read_cr3() & !0xfff != switch.cr3 {
            // SAFETY: `switch.cr3` is the root of the kernel's own space or of the
            // space a live thread owns (built by `AddressSpace::new_user`, which
            // shares every kernel mapping), so it is a valid root; the handle is
            // used only to load it, and interrupts are off.
            unsafe { AddressSpace::from_root(switch.cr3).activate() };
        }
        if let Some(top) = switch.kernel_stack_top {
            // Interrupts, exceptions and `syscall` arriving from ring 3 must land
            // on the kernel stack of the thread that is about to run.
            x86_64::gdt::set_kernel_stack(top);
            x86_64::syscall::set_kernel_stack(top);
        }
        // SAFETY: interrupts are off (asserted above), `switch.save` points into
        // a boxed thread that lives at least until it is resumed (a dead thread
        // is held as the zombie, released only after the next switch), and
        // `switch.load` came from `prepare_stack` or an earlier switch away from
        // that thread, for a stack that is still mapped.
        unsafe { context::switch_context(switch.save, switch.load) };
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
    finish(0, 0)
}

/// Ends the current thread, recording `code` for whoever waits for it.
pub fn exit_with(code: u64) -> ! {
    finish(code, 0)
}

/// Ends the current thread because the CPU raised exception `vector` (with
/// `error` as its error code, at instruction address `rip`) while it ran in
/// ring 3. The kernel survives; only this task is gone.
pub fn exit_faulted(vector: u64, error: u64, rip: u64) -> ! {
    finish_at(error, vector + 1, rip)
}

/// Ends the current thread because it was asked to stop.
pub fn exit_killed() -> ! {
    finish(0, KILLED_MARK)
}

/// Whether the running thread has been asked to stop. Called from the timer
/// interrupt, with interrupts off.
pub fn current_kill_requested() -> bool {
    SCHED.lock().current.as_ref().is_some_and(|thread| {
        thread.user.is_some() && thread.exit.kill_requested.load(Ordering::Acquire)
    })
}

fn finish(code: u64, faulted: u64) -> ! {
    finish_at(code, faulted, 0)
}

fn finish_at(code: u64, faulted: u64, rip: u64) -> ! {
    crate::arch::x86_64::disable_interrupts();
    {
        let mut scheduler = SCHED.lock();
        let current = scheduler
            .current
            .as_mut()
            .expect("scheduler has a current thread");
        current.state = State::Dead;
        let my_id = current.id;
        current.exit.code.store(code, Ordering::Release);
        current.exit.fault_rip.store(rip, Ordering::Release);
        current.exit.faulted.store(faulted, Ordering::Release);
        current.exit.done.store(true, Ordering::Release);
        // Anyone waiting for this thread to answer a call is told it never will.
        scheduler.abort_reply_waiters(my_id);
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

/// The running thread's capability space, if it has one.
pub fn current_cspace() -> Option<CSpace> {
    SCHED.lock().current.as_ref().and_then(|t| t.cspace)
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

// ---- messages between threads ---------------------------------------------------
//
// Synchronous rendezvous through an *endpoint*, named here only by the address of
// its object (the capability layer in `cspace` and `ipc` decides who may use
// which). A thread that cannot be served at once blocks; the thread that serves
// it wakes it. Everything is allocation-free: waiting threads sit in
// `Scheduler::blocked` (capacity reserved at start), and the "queue" of an
// endpoint is the set of blocked threads that name it, ordered by a sequence
// number, so there is no per-endpoint state in memory a task could damage.
//
// * `send`: hand a message to a waiting receiver, or wait for one.
// * `recv`: take a message from a waiting sender, or wait for one.
// * `call`: `send`, then wait for the `reply` of whoever received it.
// * `reply`: answer the caller of the last message this thread received.
//
// A thread dies only when it runs, so a blocked thread cannot be killed; it is
// woken (with an error) if its endpoint is destroyed, or, when it waits for a
// reply, if the thread that took its call exits.

/// A message: a 32-bit label, three words, and optionally one of the sender's own
/// slots whose capability travels with it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Message {
    pub label: u32,
    pub words: [u64; 3],
    /// The sender's slot to send (sender side only; cleared on delivery).
    pub cap_slot: Option<u16>,
}

/// What a receiver (or a caller waiting for a reply) gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Received {
    pub message: Message,
    /// The badge on the capability the sender used (0 if none).
    pub badge: u64,
    /// A capability arrived in the slot the receiver asked for.
    pub got_cap: bool,
}

/// Why a message operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpcError {
    /// The endpoint was destroyed, or the thread being waited on exited.
    Aborted,
    /// `reply` with no caller waiting.
    NoReplyTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitKind {
    /// A sender waiting for a receiver.
    Send,
    /// A receiver waiting for a sender.
    Recv,
    /// A caller waiting for a receiver.
    Call,
    /// A caller whose message was taken, waiting for the reply.
    Reply,
}

#[derive(Clone, Copy, Debug)]
struct Wait {
    endpoint: u64,
    kind: WaitKind,
    seq: u64,
    /// For `Reply`: the thread that must answer.
    server: Option<ThreadId>,
}

#[derive(Default)]
struct IpcState {
    wait: Option<Wait>,
    /// A blocked sender's message and the badge it sends with.
    outgoing: Message,
    out_badge: u64,
    /// Where a capability sent to this thread should land, while it receives.
    recv_slot: Option<u16>,
    /// Set by whoever wakes this thread with something for it.
    inbox: Option<Received>,
    /// Set by whoever wakes this thread because what it waited for is gone.
    aborted: bool,
    /// The caller this thread has received a call from and not yet answered.
    reply_to: Option<ThreadId>,
}

impl Scheduler {
    fn next_seq(&mut self) -> u64 {
        self.ipc_seq += 1;
        self.ipc_seq
    }

    /// Index in `blocked` of the longest-waiting thread on `endpoint` with one of
    /// the `kinds`.
    fn find_waiter(&self, endpoint: u64, kinds: &[WaitKind]) -> Option<usize> {
        self.blocked
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                t.ipc
                    .wait
                    .is_some_and(|w| w.endpoint == endpoint && kinds.contains(&w.kind))
            })
            .min_by_key(|(_, t)| t.ipc.wait.map_or(u64::MAX, |w| w.seq))
            .map(|(i, _)| i)
    }

    /// Moves the blocked thread at `index` to the run queue.
    fn wake(&mut self, index: usize) {
        let mut thread = self.blocked.swap_remove(index);
        thread.state = State::Ready;
        thread.ipc.wait = None;
        // Cannot grow: capacity was reserved for MAX_THREADS in `init`.
        self.run_queue.push_back(thread);
    }

    /// Wakes, with an error, every caller waiting for `server` to answer.
    fn abort_reply_waiters(&mut self, server: ThreadId) {
        while let Some(i) = self.blocked.iter().position(|t| {
            t.ipc
                .wait
                .is_some_and(|w| w.kind == WaitKind::Reply && w.server == Some(server))
        }) {
            self.blocked[i].ipc.aborted = true;
            self.wake(i);
        }
    }

    fn current_mut(&mut self) -> &mut Thread {
        self.current
            .as_mut()
            .expect("scheduler has a current thread")
    }

    fn current_ref(&self) -> &Thread {
        self.current
            .as_ref()
            .expect("scheduler has a current thread")
    }

    /// Delivers a capability from the thread `from` (its slot `slot`) to the thread
    /// `to` (the slot it asked for). `false` if there was nothing to deliver or it
    /// could not be.
    fn deliver_capability(
        from: Option<CSpace>,
        slot: Option<u16>,
        to: Option<CSpace>,
        want: Option<u16>,
    ) -> bool {
        match (from, slot, to, want) {
            (Some(from), Some(slot), Some(to), Some(want)) => from.transfer(slot, &to, want),
            _ => false,
        }
    }
}

/// Sends `message` through the endpoint at `endpoint`, with `badge`. Returns once
/// a receiver has it. With `call`, then waits for the reply and returns it.
pub fn ipc_send(
    endpoint: u64,
    badge: u64,
    message: Message,
    call: bool,
) -> Result<Option<Received>, IpcError> {
    without_interrupts(|| {
        let mut s = SCHED.lock();
        let seq = s.next_seq();
        if let Some(i) = s.find_waiter(endpoint, &[WaitKind::Recv]) {
            let (my_id, my_space) = {
                let me = s.current_ref();
                (me.id, me.cspace)
            };
            let (receiver_id, receiver_space, want) = {
                let r = &s.blocked[i];
                (r.id, r.cspace, r.ipc.recv_slot)
            };
            let got_cap =
                Scheduler::deliver_capability(my_space, message.cap_slot, receiver_space, want);
            let r = &mut s.blocked[i];
            r.ipc.inbox = Some(Received {
                message: Message {
                    cap_slot: None,
                    ..message
                },
                badge,
                got_cap,
            });
            if call {
                r.ipc.reply_to = Some(my_id);
            }
            s.wake(i);
            if !call {
                return Ok(None);
            }
            let me = s.current_mut();
            me.ipc.wait = Some(Wait {
                endpoint,
                kind: WaitKind::Reply,
                seq,
                server: Some(receiver_id),
            });
            me.state = State::Blocked;
        } else {
            let me = s.current_mut();
            me.ipc.outgoing = message;
            me.ipc.out_badge = badge;
            me.ipc.wait = Some(Wait {
                endpoint,
                kind: if call { WaitKind::Call } else { WaitKind::Send },
                seq,
                server: None,
            });
            me.state = State::Blocked;
        }
        // Park this thread and run something else. Interrupts are off, so nothing
        // can intervene between recording the wait and switching away; the lock
        // is released first because `schedule` takes it.
        drop(s);
        schedule();
        // Woken: by a receiver (send), by the reply (call), or by an abort.
        let mut s = SCHED.lock();
        let me = s.current_mut();
        if me.ipc.aborted {
            me.ipc.aborted = false;
            return Err(IpcError::Aborted);
        }
        Ok(me.ipc.inbox.take())
    })
}

/// Waits for a message on the endpoint at `endpoint`. A capability sent with it
/// lands in this thread's slot `want`, if given.
pub fn ipc_recv(endpoint: u64, want: Option<u16>) -> Result<Received, IpcError> {
    without_interrupts(|| {
        let mut s = SCHED.lock();
        if let Some(i) = s.find_waiter(endpoint, &[WaitKind::Send, WaitKind::Call]) {
            let (my_id, my_space) = {
                let me = s.current_ref();
                (me.id, me.cspace)
            };
            let (sender_id, sender_space, message, badge, kind) = {
                let t = &s.blocked[i];
                let w = t.ipc.wait.expect("a blocked thread has a wait");
                (t.id, t.cspace, t.ipc.outgoing, t.ipc.out_badge, w.kind)
            };
            let got_cap =
                Scheduler::deliver_capability(sender_space, message.cap_slot, my_space, want);
            if kind == WaitKind::Call {
                // The caller stays blocked, now waiting for this thread's reply.
                let w = s.blocked[i].ipc.wait.as_mut().expect("wait");
                w.kind = WaitKind::Reply;
                w.server = Some(my_id);
                s.current_mut().ipc.reply_to = Some(sender_id);
            } else {
                s.wake(i);
            }
            return Ok(Received {
                message: Message {
                    cap_slot: None,
                    ..message
                },
                badge,
                got_cap,
            });
        }
        let seq = s.next_seq();
        let me = s.current_mut();
        me.ipc.recv_slot = want;
        me.ipc.wait = Some(Wait {
            endpoint,
            kind: WaitKind::Recv,
            seq,
            server: None,
        });
        me.state = State::Blocked;
        // Park this thread and run something else. Interrupts are off, so nothing
        // can intervene between recording the wait and switching away; the lock
        // is released first because `schedule` takes it.
        drop(s);
        schedule();
        let mut s = SCHED.lock();
        let me = s.current_mut();
        me.ipc.recv_slot = None;
        if me.ipc.aborted {
            me.ipc.aborted = false;
            return Err(IpcError::Aborted);
        }
        Ok(me.ipc.inbox.take().expect("a woken receiver has a message"))
    })
}

/// Answers the caller of the last message this thread received.
pub fn ipc_reply(message: Message) -> Result<(), IpcError> {
    without_interrupts(|| {
        let mut s = SCHED.lock();
        let my_id = s.current_ref().id;
        let caller = s
            .current_mut()
            .ipc
            .reply_to
            .take()
            .ok_or(IpcError::NoReplyTarget)?;
        let index = s
            .blocked
            .iter()
            .position(|t| {
                t.id == caller
                    && t.ipc
                        .wait
                        .is_some_and(|w| w.kind == WaitKind::Reply && w.server == Some(my_id))
            })
            .ok_or(IpcError::NoReplyTarget)?;
        s.blocked[index].ipc.inbox = Some(Received {
            message: Message {
                cap_slot: None,
                ..message
            },
            badge: 0,
            got_cap: false,
        });
        s.wake(index);
        Ok(())
    })
}

/// Wakes, with an error, every thread waiting to send to or receive on an
/// endpoint whose address is in `first..=last`. Called when endpoints are
/// destroyed.
pub fn ipc_abort_range(first: u64, last: u64) {
    without_interrupts(|| {
        let mut s = SCHED.lock();
        while let Some(i) = s.blocked.iter().position(|t| {
            t.ipc.wait.is_some_and(|w| {
                (first..=last).contains(&w.endpoint)
                    && matches!(w.kind, WaitKind::Send | WaitKind::Recv | WaitKind::Call)
            })
        }) {
            s.blocked[i].ipc.aborted = true;
            s.wake(i);
        }
    })
}

/// Threads waiting on messages now, for tests.
#[cfg(feature = "selftest")]
pub fn blocked_count() -> usize {
    SCHED.lock().blocked.len()
}
