//! # Locks for the kernel
//!
//! Two locks, both tiny:
//!
//! - [`SpinLock`]: mutual exclusion by spinning on an atomic flag.
//! - [`IrqLock`]: a spinlock that also **turns interrupts off** while it is
//!   held, and puts them back the way it found them afterwards.
//!
//! ## Why interrupts matter to a lock
//!
//! Suppose thread A holds a plain spinlock and the timer interrupt fires. The
//! scheduler switches to thread B, which wants the same lock. B spins, waiting
//! for A to release it, but A cannot run while B spins on the only CPU. That is
//! a deadlock caused by *preemption*, not by two CPUs. The same happens if an
//! interrupt handler wants a lock the interrupted code already holds.
//!
//! The fix for data that is shared with preemptible code or interrupt handlers
//! is to forbid the interruption while the lock is held: disable interrupts,
//! take the lock, do the work, release the lock, restore interrupts.
//!
//! ## Restore, do not enable
//!
//! A lock must put interrupts back in the state it *found*. If the caller had
//! already disabled them (inside another [`IrqLock`], say), enabling them on
//! release would end that outer critical section early. [`IrqLock`] therefore
//! remembers whether interrupts were on and only turns them back on if they were.
//!
//! ## One CPU only
//!
//! With one CPU and interrupts off, nothing can run between "I tried to take the
//! lock" and "someone else holds it" except code that already holds it, so a
//! failed attempt on an [`IrqLock`] is always a self-deadlock bug. [`IrqLock`]
//! turns it into a panic with a message instead of a silent hang. When SMP
//! arrives this must change to spinning, which is why the policy is documented
//! here and tested.
//!
//! The interrupt flag is behind the [`InterruptControl`] trait. The kernel
//! implements it with `cli`/`sti`; the host tests implement it with a variable,
//! so the nesting logic runs under `cargo test`.

#![no_std]
#![deny(unsafe_op_in_unsafe_fn)]

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// A mutual-exclusion lock that spins. Does **not** touch interrupts: use it
/// only for data that no interrupt handler or preemptible thread contends for,
/// or through [`IrqLock`].
pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the lock hands out `&mut T` to one holder at a time (the `locked`
// flag, acquired with `Acquire` and released with `Release`, orders the holders'
// accesses), so sharing the lock between threads is as safe as sending the value
// between them, which needs `T: Send`.
unsafe impl<T: Send> Sync for SpinLock<T> {}
// SAFETY: owning a `SpinLock<T>` is owning a `T`.
unsafe impl<T: Send> Send for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Takes the lock if it is free.
    pub fn try_lock(&self) -> Option<SpinGuard<'_, T>> {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| SpinGuard { lock: self })
    }

    /// Takes the lock, spinning until it is free.
    pub fn lock(&self) -> SpinGuard<'_, T> {
        loop {
            if let Some(guard) = self.try_lock() {
                return guard;
            }
            // Wait on a plain load so waiting cores do not hammer the cache line
            // with failed compare-exchanges.
            while self.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }
}

/// Proof that the lock is held. The lock is released when this is dropped.
pub struct SpinGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<T> Deref for SpinGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard exists only while its holder owns the lock, so no
        // one else has access to the value.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: as above, and `&mut self` makes this the only reference the
        // guard hands out right now.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

/// How the kernel turns interrupts off and on. Implemented with `cli`/`sti` in
/// the kernel and with a plain variable in tests.
pub trait InterruptControl {
    /// Disables interrupts and returns whether they were enabled before.
    fn disable() -> bool;

    /// Re-enables interrupts.
    ///
    /// # Safety
    ///
    /// Only call this to undo a [`InterruptControl::disable`] that returned
    /// `true`, so code that expects interrupts to be off keeps working.
    unsafe fn enable();
}

/// A spinlock that holds interrupts off while locked. See the crate docs.
pub struct IrqLock<T, C: InterruptControl> {
    inner: SpinLock<T>,
    _control: PhantomData<fn() -> C>,
}

impl<T, C: InterruptControl> IrqLock<T, C> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: SpinLock::new(value),
            _control: PhantomData,
        }
    }

    /// Disables interrupts and takes the lock.
    ///
    /// # Panics
    ///
    /// If the lock is already held. With one CPU and interrupts off the holder
    /// can only be the caller itself (or code it called), so waiting would hang
    /// forever; a panic with a message is more useful.
    pub fn lock(&self) -> IrqGuard<'_, T, C> {
        let were_enabled = C::disable();
        match self.inner.try_lock() {
            Some(guard) => IrqGuard {
                guard: Some(guard),
                were_enabled,
                _control: PhantomData,
            },
            None => {
                if were_enabled {
                    // SAFETY: undoes the `disable` just above, which returned
                    // true, so the panic message is not printed with a lock
                    // state change pending.
                    unsafe { C::enable() };
                }
                panic!("deadlock: IrqLock taken while already held (one CPU, interrupts off)");
            }
        }
    }
}

/// Proof that an [`IrqLock`] is held. Dropping it releases the lock first and
/// then restores the interrupt state.
pub struct IrqGuard<'a, T, C: InterruptControl> {
    guard: Option<SpinGuard<'a, T>>,
    were_enabled: bool,
    _control: PhantomData<fn() -> C>,
}

impl<T, C: InterruptControl> Deref for IrqGuard<'_, T, C> {
    type Target = T;
    fn deref(&self) -> &T {
        self.guard.as_ref().expect("guard present until drop")
    }
}

impl<T, C: InterruptControl> DerefMut for IrqGuard<'_, T, C> {
    fn deref_mut(&mut self) -> &mut T {
        self.guard.as_mut().expect("guard present until drop")
    }
}

impl<T, C: InterruptControl> Drop for IrqGuard<'_, T, C> {
    fn drop(&mut self) {
        // Release the lock *before* interrupts come back on: if an interrupt
        // arrived while the lock was still held, its handler could not take it.
        self.guard = None;
        if self.were_enabled {
            // SAFETY: `were_enabled` is true only if the matching `disable`
            // reported interrupts were on, which is exactly what `enable` undoes.
            unsafe { C::enable() };
        }
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;
    use std::sync::Arc;
    use std::thread;

    // A fake interrupt flag per test thread.
    std::thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(true) };
    }

    struct Fake;
    impl InterruptControl for Fake {
        fn disable() -> bool {
            ENABLED.with(|e| e.replace(false))
        }
        unsafe fn enable() {
            ENABLED.with(|e| e.set(true));
        }
    }

    fn interrupts_on() -> bool {
        ENABLED.with(Cell::get)
    }

    fn set_interrupts(on: bool) {
        ENABLED.with(|e| e.set(on));
    }

    type Lock<T> = IrqLock<T, Fake>;

    #[test]
    fn spinlock_gives_exclusive_access_and_releases_on_drop() {
        let lock = SpinLock::new(5);
        {
            let mut guard = lock.lock();
            *guard += 1;
            assert!(lock.is_locked());
            assert!(lock.try_lock().is_none(), "second holder got in");
        }
        assert!(!lock.is_locked());
        assert_eq!(*lock.lock(), 6);
    }

    #[test]
    fn spinlock_loses_no_updates_under_real_contention() {
        // Read-modify-write on a plain integer: without mutual exclusion, some
        // increments would be lost.
        let lock = Arc::new(SpinLock::new(0u64));
        let handles: std::vec::Vec<_> = (0..8)
            .map(|_| {
                let lock = Arc::clone(&lock);
                thread::spawn(move || {
                    for _ in 0..20_000 {
                        let mut guard = lock.lock();
                        let current = *guard;
                        core::hint::black_box(&mut *guard);
                        *guard = current + 1;
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(*lock.lock(), 160_000);
    }

    #[test]
    fn irq_lock_disables_interrupts_while_held_and_restores_them() {
        set_interrupts(true);
        let lock = Lock::new(1);
        {
            let guard = lock.lock();
            assert!(!interrupts_on(), "interrupts still on inside the lock");
            assert_eq!(*guard, 1);
        }
        assert!(interrupts_on(), "interrupts not restored");
    }

    #[test]
    fn irq_lock_does_not_enable_interrupts_that_were_already_off() {
        set_interrupts(false);
        let lock = Lock::new(1);
        drop(lock.lock());
        assert!(!interrupts_on(), "the lock turned interrupts on");
    }

    #[test]
    fn nested_irq_locks_restore_in_order() {
        set_interrupts(true);
        let a = Lock::new(1);
        let b = Lock::new(2);
        {
            let ga = a.lock();
            {
                let gb = b.lock();
                assert_eq!(*ga + *gb, 3);
                assert!(!interrupts_on());
            }
            assert!(
                !interrupts_on(),
                "inner unlock ended the outer critical section"
            );
        }
        assert!(interrupts_on());
    }

    #[test]
    fn lock_is_released_before_interrupts_return() {
        // Check the order inside `drop`: the closure runs while "interrupts
        // come back" and records whether the lock was already free.
        use core::sync::atomic::AtomicBool;
        static FREE_AT_ENABLE: AtomicBool = AtomicBool::new(false);
        static LOCK: IrqLock<u8, Order> = IrqLock::new(0);

        struct Order;
        impl InterruptControl for Order {
            fn disable() -> bool {
                true
            }
            unsafe fn enable() {
                FREE_AT_ENABLE.store(!LOCK.inner.is_locked(), Ordering::SeqCst);
            }
        }
        drop(LOCK.lock());
        assert!(
            FREE_AT_ENABLE.load(Ordering::SeqCst),
            "interrupts came back while the lock was still held"
        );
    }

    #[test]
    #[should_panic(expected = "deadlock")]
    fn taking_an_irq_lock_twice_panics_instead_of_hanging() {
        set_interrupts(true);
        let lock = Lock::new(0);
        let _first = lock.lock();
        let _second = lock.lock();
    }

    #[test]
    fn a_failed_irq_lock_attempt_leaves_interrupts_as_found() {
        // Reach the failure path with interrupts ON: hold the inner spinlock
        // directly (as a bug elsewhere might), so `IrqLock::lock` disables
        // interrupts, fails to get the lock, and must turn them back on before
        // it panics.
        set_interrupts(true);
        let lock = Lock::new(0);
        let held = lock.inner.lock();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _second = lock.lock();
        }));
        assert!(result.is_err());
        assert!(
            interrupts_on(),
            "a failed attempt left interrupts off, which the caller never asked for"
        );
        drop(held);
    }
}
