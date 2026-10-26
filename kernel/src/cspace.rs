//! Capability spaces: where the kernel keeps what each task is allowed to touch.
//!
//! The rules (slots, derivation, revocation, retyping) are in the host-tested
//! `durg-cap` crate. This module supplies what only the kernel can: memory for
//! the slots, the lock, the task-facing `invoke` entry point, and returning
//! memory when a task goes.
//!
//! ## A task's capability space
//!
//! One 4 KiB frame holding [`SLOTS`] (128) slots of 32 bytes: a CNode. Slot 0
//! always holds a capability to the CNode itself. A task names a capability by
//! its slot number and by nothing else, and the number is checked against the
//! size of *its own* space, so it cannot name a slot in anyone else's. A task
//! with no capability space can name nothing at all.
//!
//! The slot lists (who was derived from whom) are linked through **physical
//! slot numbers** (`physical address / 32`), which is how [`PhysSlots`] reaches
//! slots through the kernel's direct map. A task cannot write those frames, so it
//! cannot forge a link.
//!
//! ## Where memory comes from
//!
//! The kernel does not allocate object memory for tasks. The root task is given
//! **untyped** regions, naturally aligned blocks taken out of the frame allocator
//! at boot ([`reserve_untyped`]); everything a task makes (frames, endpoints,
//! notifications) it carves out of those with `retype`, and the kernel zeroes
//! the new memory so nothing a previous owner left behind is readable.
//!
//! Only the blocks the kernel itself reserved go back to the frame allocator
//! when their last capability is deleted (the table in [`State`] says which).
//! A child region carved from one never does: its memory is still the parent's.
//!
//! ## What is not here yet
//!
//! Only the task's own CNode can be operated on (derive, move, delete, revoke
//! act on slots of the caller's own space), and only frames, endpoints,
//! notifications and smaller untyped regions can be created. A frame cannot be
//! mapped yet, so destroying one cannot leave a dangling mapping; when mapping
//! arrives, destroying a mapped frame must unmap it first. **Nothing here has
//! been reviewed by anyone but its author.**
//!
//! ## Locking
//!
//! One interrupt-safe lock, [`STATE`], covers every capability operation and the
//! table of reserved blocks. Order: scheduler, then this lock, then the frame
//! allocator (which never takes another). The console is a leaf, as always.

use durg_cap::{
    self as cap, Destroyed, Kind, Rights, Slot, SlotStore, MAX_RETYPE, NONE, SLOT_BYTES,
};
use durg_syscall::{label, Error};

use crate::arch::x86_64::IrqLock;
use crate::mm::frame;

/// log2 of the number of slots in a task's capability space.
pub const RADIX: u32 = 7;
/// Slots in a task's capability space (one 4 KiB frame).
pub const SLOTS: u32 = 1 << RADIX;

/// The most objects one retype may create when the kernel must zero them, and the
/// most bytes it will zero in one call (so the lock is not held for long).
const MAX_ZEROED_BYTES: u64 = 1 << 20;
/// Largest region size the kernel will reserve for untyped memory: 16 MiB.
const MAX_UNTYPED_BITS: u8 = 24;
/// Reserved blocks the table can track.
const MAX_BLOCKS: usize = 16;
/// Most steps along a derivation list before it is called corrupt. Far above any
/// real list (every task has at most [`SLOTS`] slots).
const WALK_LIMIT: u32 = 1 << 16;

struct State {
    /// Blocks reserved from the frame allocator and not yet returned:
    /// `(physical address, size in bits)`.
    blocks: [Option<(u64, u8)>; MAX_BLOCKS],
}

static STATE: IrqLock<State> = IrqLock::new(State {
    blocks: [None; MAX_BLOCKS],
});

/// Capability slots in physical memory, reached through the direct map. A slot
/// number is `physical address / 32`; that is how the derivation lists link
/// slots that live in different frames.
struct PhysSlots;

impl SlotStore for PhysSlots {
    fn len(&self) -> u32 {
        u32::MAX - 1
    }
    fn walk_limit(&self) -> u32 {
        WALK_LIMIT
    }
    fn load(&self, index: u32) -> Option<Slot> {
        let mut bytes = [0u8; SLOT_BYTES];
        frame::read_bytes(u64::from(index) * SLOT_BYTES as u64, &mut bytes)?;
        Slot::from_bytes(&bytes)
    }
    fn store(&mut self, index: u32, value: Slot) -> bool {
        frame::write_bytes(u64::from(index) * SLOT_BYTES as u64, &value.to_bytes()).is_some()
    }
}

/// A task's capability space: the physical address of its CNode frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CSpace {
    base: u64,
}

impl CSpace {
    /// A new capability space whose only capability, in slot 0, names itself.
    /// `None` if there is no frame to spare.
    pub fn new() -> Option<CSpace> {
        let base = frame::alloc_zeroed_frame()?;
        // The slot number of the last slot must fit the list links.
        if (base / SLOT_BYTES as u64) + u64::from(SLOTS) >= u64::from(NONE) {
            let _ = frame::free_frame(base);
            return None;
        }
        let space = CSpace { base };
        let installed = {
            let _lock = STATE.lock();
            // Slot 0 is in range by construction.
            let first = (base / SLOT_BYTES as u64) as u32;
            cap::install_root(&mut PhysSlots, first, Kind::CNode, base, 12, Rights::ALL)
        };
        if installed.is_err() {
            let _ = frame::free_frame(base);
            return None;
        }
        Some(space)
    }

    /// Like [`new`](Self::new), but also puts an untyped capability for the
    /// block at `physical` (`1 << bits` bytes, reserved with
    /// [`reserve_untyped`]) in slot 1. This is how the root task gets its memory.
    pub fn with_untyped(physical: u64, bits: u8) -> Option<CSpace> {
        let space = CSpace::new()?;
        let installed = {
            let _lock = STATE.lock();
            let slot = space.slot_ref(1).ok()?;
            cap::install_root(
                &mut PhysSlots,
                slot,
                Kind::Untyped,
                physical,
                bits,
                Rights::ALL,
            )
        };
        if installed.is_err() {
            space.destroy();
            return None;
        }
        Some(space)
    }

    /// Checks every capability-structure rule on this space's slots. For tests.
    /// Copies the slots out once first, so the check runs on ordinary memory.
    #[cfg(feature = "selftest")]
    pub fn check_invariants(&self) -> Result<(), &'static str> {
        struct Copy {
            first: u32,
            slots: [Slot; SLOTS as usize],
        }
        impl SlotStore for Copy {
            fn len(&self) -> u32 {
                self.first + SLOTS
            }
            fn load(&self, index: u32) -> Option<Slot> {
                let at = index.checked_sub(self.first)? as usize;
                self.slots.get(at).copied()
            }
            fn store(&mut self, _: u32, _: Slot) -> bool {
                false
            }
        }
        let first = (self.base / SLOT_BYTES as u64) as u32;
        let copy = {
            let _lock = STATE.lock();
            let mut slots = [Slot::EMPTY; SLOTS as usize];
            for (i, slot) in slots.iter_mut().enumerate() {
                *slot = PhysSlots
                    .load(first + i as u32)
                    .ok_or("a slot could not be read back")?;
            }
            Copy { first, slots }
        };
        cap::check_invariants_in(&copy, first, SLOTS)
    }

    /// The list-link number of this space's slot `index`.
    fn slot_ref(&self, index: u64) -> Result<u32, Error> {
        if index >= u64::from(SLOTS) {
            return Err(Error::InvalidSlot);
        }
        Ok((self.base / SLOT_BYTES as u64) as u32 + index as u32)
    }

    /// Deletes every capability in the space, returns the blocks it owned, and
    /// frees the CNode frame. Called when the task is reaped.
    pub fn destroy(self) {
        let mut state = STATE.lock();
        let mut store = PhysSlots;
        // Regions first, with everything carved from them, so that the memory
        // of a reserved block is returned by its own capability being deleted.
        for index in 0..SLOTS {
            let Ok(r) = self.slot_ref(u64::from(index)) else {
                continue;
            };
            if let Some(slot) = store.load(r) {
                if slot.kind() == Kind::Untyped && slot.depth() == 0 {
                    let _ = cap::revoke(&mut store, r, &mut |d| state.on_destroyed(d));
                }
            }
        }
        for index in 0..SLOTS {
            let Ok(r) = self.slot_ref(u64::from(index)) else {
                continue;
            };
            if store.load(r).is_some_and(|s| !s.is_empty()) {
                // Cannot fail: the slot is occupied and in range.
                let _ = cap::delete(&mut store, r, &mut |d| state.on_destroyed(d));
            }
        }
        // Everything must be gone, or a live slot would still link into this
        // frame once it is reused.
        for index in 0..SLOTS {
            let r = (self.base / SLOT_BYTES as u64) as u32 + index;
            assert!(
                store.load(r).is_some_and(|s| s.is_empty()),
                "capability space torn down with a capability left in slot {index}"
            );
        }
        drop(state);
        let _ = frame::free_frame(self.base);
    }

    /// What `invoke` does on behalf of the calling task. See the labels in
    /// `durg-syscall`. Returns the value for the task's `rax`, or why not.
    pub fn invoke(&self, cptr: u64, op: u64, args: [u64; 3]) -> Result<u64, Error> {
        let mut state = STATE.lock();
        let mut store = PhysSlots;
        let target = self.slot_ref(cptr)?;
        let slot = store.load(target).ok_or(Error::BadArgument)?;
        if slot.is_empty() {
            return Err(Error::EmptySlot);
        }
        match durg_syscall::operation(op) {
            label::IDENTIFY => {
                durg_syscall::require_plain_label(op)?;
                Ok(u64::from(slot.kind() as u8)
                    | u64::from(slot.rights().bits()) << 8
                    | u64::from(slot.bits()) << 16)
            }
            label::RETYPE => {
                durg_syscall::require_plain_label(op)?;
                self.retype(&mut store, target, &slot, args)
            }
            label::DERIVE => {
                let (rights, with_badge) = durg_syscall::parse_derive_label(op)?;
                self.require_own_cnode(&slot)?;
                let src = self.slot_ref(args[0])?;
                let dst = self.slot_ref(args[1])?;
                let badge = with_badge.then_some(args[2]);
                cap::derive(
                    &mut store,
                    src,
                    dst,
                    Rights::from_bits_truncate(rights),
                    badge,
                )
                .map_err(map_error)?;
                Ok(0)
            }
            label::MOVE => {
                durg_syscall::require_plain_label(op)?;
                self.require_own_cnode(&slot)?;
                let src = self.slot_ref(args[0])?;
                let dst = self.slot_ref(args[1])?;
                cap::move_cap(&mut store, src, dst).map_err(map_error)?;
                Ok(0)
            }
            label::DELETE => {
                durg_syscall::require_plain_label(op)?;
                self.require_own_cnode(&slot)?;
                let victim = self.slot_ref(args[0])?;
                // A task may not delete the last capability to its own space:
                // it would be left unable to name anything, and the frame would
                // be freed under it.
                let v = store.load(victim).ok_or(Error::BadArgument)?;
                if v.kind() == Kind::CNode
                    && v.object() == self.base
                    && cap::is_final(&store, victim).map_err(map_error)?
                {
                    return Err(Error::BadArgument);
                }
                cap::delete(&mut store, victim, &mut |d| state.on_destroyed(d))
                    .map_err(map_error)?;
                Ok(0)
            }
            label::REVOKE => {
                durg_syscall::require_plain_label(op)?;
                self.require_own_cnode(&slot)?;
                let victim = self.slot_ref(args[0])?;
                let n = cap::revoke(&mut store, victim, &mut |d| state.on_destroyed(d))
                    .map_err(map_error)?;
                Ok(u64::from(n))
            }
            _ => Err(Error::BadArgument),
        }
    }

    /// The caller must be invoking a capability to its own CNode, with the right
    /// to write it.
    fn require_own_cnode(&self, slot: &Slot) -> Result<(), Error> {
        if slot.kind() != Kind::CNode || slot.object() != self.base {
            return Err(Error::WrongKind);
        }
        if !slot.rights().contains(Rights::WRITE) {
            return Err(Error::NoRights);
        }
        Ok(())
    }

    fn retype(
        &self,
        store: &mut PhysSlots,
        untyped: u32,
        region: &Slot,
        args: [u64; 3],
    ) -> Result<u64, Error> {
        if region.kind() != Kind::Untyped {
            return Err(Error::WrongKind);
        }
        if !region.rights().contains(Rights::WRITE) {
            return Err(Error::NoRights);
        }
        let (kind, bits, count) = durg_syscall::parse_retype_arg(args[0])?;
        let kind = Kind::from_u8(kind).ok_or(Error::BadArgument)?;
        // Only these can exist so far. (A CNode, a thread or a page table needs
        // initialisation and teardown this module does not have yet.)
        if !matches!(
            kind,
            Kind::Untyped | Kind::Frame | Kind::Endpoint | Kind::Notification
        ) {
            return Err(Error::BadArgument);
        }
        let count = usize::from(count);
        if count == 0 || count > MAX_RETYPE {
            return Err(Error::BadArgument);
        }
        let size = 1u64
            .checked_shl(u32::from(bits))
            .ok_or(Error::BadArgument)?;
        let zeroed = kind != Kind::Untyped;
        if zeroed && size.saturating_mul(count as u64) > MAX_ZEROED_BYTES {
            return Err(Error::BadArgument);
        }
        let mut into = [0u32; MAX_RETYPE];
        for (i, slot) in into.iter_mut().take(count).enumerate() {
            *slot = self.slot_ref(args[1].checked_add(i as u64).ok_or(Error::InvalidSlot)?)?;
        }
        cap::retype(store, untyped, kind, bits, &into[..count]).map_err(map_error)?;
        if zeroed {
            for &r in &into[..count] {
                let object = store.load(r).ok_or(Error::BadArgument)?.object();
                zero(object, size);
            }
        }
        Ok(0)
    }
}

impl State {
    /// An object has lost its last capability. Blocks the kernel reserved go back
    /// to the frame allocator; nothing else needs doing yet.
    fn on_destroyed(&mut self, d: Destroyed) {
        if d.kind != Kind::Untyped {
            return;
        }
        for entry in self.blocks.iter_mut() {
            if *entry == Some((d.object, d.bits)) {
                assert!(
                    frame::release_block(d.object, u32::from(d.bits) - 12),
                    "a reserved block could not be returned whole to the frame allocator"
                );
                *entry = None;
            }
        }
    }
}

/// Fills `length` bytes of physical memory at `physical` with zeros.
fn zero(physical: u64, length: u64) {
    const ZEROS: [u8; 4096] = [0; 4096];
    let mut done = 0;
    while done < length {
        let n = (length - done).min(ZEROS.len() as u64) as usize;
        // The memory is inside a region the kernel reserved for this purpose, so
        // the direct map covers it; a refusal means the bookkeeping is wrong.
        assert!(
            frame::write_bytes(physical + done, &ZEROS[..n]).is_some(),
            "could not zero object memory at {physical:#x}"
        );
        done += n as u64;
    }
}

fn map_error(error: cap::Error) -> Error {
    use cap::Error as C;
    match error {
        C::InvalidSlot => Error::InvalidSlot,
        C::EmptySlot => Error::EmptySlot,
        C::DestinationOccupied => Error::SlotOccupied,
        C::RightsEscalation => Error::NoRights,
        C::NotUntyped => Error::WrongKind,
        C::OutOfMemory => Error::OutOfMemory,
        C::SameSlot
        | C::CannotBadge
        | C::AlreadyBadged
        | C::ZeroBadge
        | C::CannotDeriveUntyped
        | C::BadSize
        | C::BadCount
        | C::DepthLimit => Error::BadArgument,
        // A damaged list: a kernel bug or memory damage, not something the task
        // did. Refuse; the task learns only that the call failed.
        C::Corrupt => Error::BadArgument,
    }
}

/// Reserves a naturally aligned block of `1 << bits` bytes (4 KiB to 16 MiB) for
/// use as an untyped region and records it so that it returns to the frame
/// allocator when its capability is deleted. Returns the physical address.
pub fn reserve_untyped(bits: u8) -> Option<u64> {
    if !(12..=MAX_UNTYPED_BITS).contains(&bits) {
        return None;
    }
    let mut state = STATE.lock();
    let slot = state.blocks.iter().position(Option::is_none)?;
    let physical = frame::reserve_aligned_block(u32::from(bits) - 12)?;
    state.blocks[slot] = Some((physical, bits));
    Some(physical)
}

/// Blocks reserved and not yet returned, for tests.
#[cfg(feature = "selftest")]
pub fn reserved_blocks() -> usize {
    STATE.lock().blocks.iter().flatten().count()
}

/// Frames in the blocks reserved and not yet returned, for tests that account
/// for every frame.
#[cfg(feature = "selftest")]
pub fn reserved_frames() -> u64 {
    STATE
        .lock()
        .blocks
        .iter()
        .flatten()
        .map(|&(_, bits)| 1u64 << (bits - 12))
        .sum()
}
