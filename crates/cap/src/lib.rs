//! # Capability core: slots, derivation, revocation, retyping
//!
//! A **capability** is an unforgeable token that names one kernel object and
//! says what the holder may do with it. A program can only act on objects it
//! holds capabilities for; there is no other way to name anything (promise 1 in
//! the README). This crate is the pure logic of that idea. It has no `unsafe`
//! code and never touches real memory, so every rule can be property-tested on
//! the host before the kernel relies on it (design: `docs/design/m10-capability-core.md`,
//! ADR-0009).
//!
//! ## The slot
//!
//! Capabilities live in **slots**. A slot is 32 bytes: the capability (kind,
//! rights, size, object address, an `extra` word) plus its place in the
//! **derivation list** (`prev`, `next`, `depth`). Everything is inside the slot
//! so that deriving or revoking never allocates, which keeps the locked rule
//! "the kernel never allocates on behalf of tasks after boot".
//!
//! ## The derivation list
//!
//! Slots that descend from each other form a doubly linked list in which a
//! slot's descendants follow it *contiguously*. `depth` makes this checkable:
//! the descendants of slot `s` are the slots after it whose depth is greater
//! than `s`'s, up to the first slot that is not. The operations keep three
//! facts true (see [`check_invariants`], which the tests run after every step):
//!
//! * **A capability is never stronger than its parent** (rights only shrink).
//! * **After `revoke(s)` nothing derived from `s` remains.**
//! * **Objects carved out of one untyped region never overlap.**
//!
//! ## Untyped memory
//!
//! Object memory is never allocated by the kernel. A task holds an **untyped**
//! capability for a region and *retypes* part of it into objects. The region has
//! a watermark (`extra`, the "free index"): retype hands out memory above it and
//! raises it, and it drops back to zero only when no descendant is left, so
//! memory is never reused while a stale capability to it exists. Untyped
//! capabilities cannot be copied (only subdivided), because two untyped
//! capabilities with retype authority over the same memory would overlap.
//!
//! ## What this crate does not do
//!
//! It does not zero object memory, find the object behind a capability, run
//! IPC, or look anything up by a task's index; the kernel does those. It
//! reports the objects whose last capability disappeared (see [`Destroyed`]) so
//! the kernel can free them. It is not formally verified.

#![no_std]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

/// "No slot": the end of a chain, or no parent.
pub const NONE: u32 = u32::MAX;

/// The most objects one retype call may create.
pub const MAX_RETYPE: usize = 64;

/// Smallest object size, as a power of two (16 bytes).
pub const MIN_OBJECT_BITS: u8 = 4;

/// Largest region or object size, as a power of two. Kept below 63 so that
/// `1 << bits` and sums of sizes cannot overflow a `u64`.
pub const MAX_BITS: u8 = 62;

/// What kind of object a capability names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// An unused slot.
    Empty = 0,
    /// A region of raw memory that can be retyped.
    Untyped = 1,
    /// An array of slots (a capability space).
    CNode = 2,
    /// A synchronous message rendezvous point.
    Endpoint = 3,
    /// An asynchronous signal.
    Notification = 4,
    /// A schedulable thread.
    Thread = 5,
    /// A page of memory that can be mapped.
    Frame = 6,
    /// A page table (any level).
    PageTable = 7,
}

impl Kind {
    /// The kind with this numeric value, or `None`.
    pub const fn from_u8(n: u8) -> Option<Kind> {
        Some(match n {
            0 => Kind::Empty,
            1 => Kind::Untyped,
            2 => Kind::CNode,
            3 => Kind::Endpoint,
            4 => Kind::Notification,
            5 => Kind::Thread,
            6 => Kind::Frame,
            7 => Kind::PageTable,
            _ => return None,
        })
    }
}

/// What the holder may do. Rights only ever shrink along a derivation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rights(u8);

impl Rights {
    /// No rights.
    pub const NONE: Rights = Rights(0);
    /// Read, receive, or map readable.
    pub const READ: Rights = Rights(1);
    /// Write, send, or map writable.
    pub const WRITE: Rights = Rights(2);
    /// Transfer capabilities through this one.
    pub const GRANT: Rights = Rights(4);
    /// Transfer only a reply capability.
    pub const GRANT_REPLY: Rights = Rights(8);
    /// Every right.
    pub const ALL: Rights = Rights(15);

    /// Builds rights from bits, dropping any bit that is not a right.
    pub const fn from_bits_truncate(bits: u8) -> Rights {
        Rights(bits & Rights::ALL.0)
    }

    /// The raw bits.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// True if every right in `other` is also in `self`.
    pub const fn contains(self, other: Rights) -> bool {
        self.0 & other.0 == other.0
    }
}

/// One capability slot: exactly 32 bytes, and all-zero bytes mean "empty".
///
/// The zero rule matters: the kernel hands out *zeroed* memory when it retypes
/// untyped memory into a capability space, and that memory must already be a
/// valid space of empty slots. So the links are stored as `index + 1`, with 0
/// meaning "none" (see [`Slot::prev`] and [`Slot::next`]), and [`Kind::Empty`]
/// is 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Slot {
    object: u64,
    extra: u64,
    prev_raw: u32,
    next_raw: u32,
    depth: u16,
    kind: Kind,
    rights: Rights,
    bits: u8,
}

/// Size of a slot in memory, in bytes.
pub const SLOT_BYTES: usize = 32;
const _: () = assert!(core::mem::size_of::<Slot>() == SLOT_BYTES);

impl Slot {
    /// An unused slot (all zero bytes).
    pub const EMPTY: Slot = Slot {
        object: 0,
        extra: 0,
        prev_raw: 0,
        next_raw: 0,
        depth: 0,
        kind: Kind::Empty,
        rights: Rights::NONE,
        bits: 0,
    };

    /// What the capability names.
    pub fn kind(&self) -> Kind {
        self.kind
    }
    /// What the holder may do.
    pub fn rights(&self) -> Rights {
        self.rights
    }
    /// Size of the object (or region) as a power of two.
    pub fn bits(&self) -> u8 {
        self.bits
    }
    /// Depth in the derivation list (0 for a root).
    pub fn depth(&self) -> u16 {
        self.depth
    }
    /// Physical address of the object (or region).
    pub fn object(&self) -> u64 {
        self.object
    }
    /// Endpoint/notification: the badge. Untyped: the free index.
    pub fn extra(&self) -> u64 {
        self.extra
    }
    /// Previous slot in the derivation list, or [`NONE`].
    pub fn prev(&self) -> u32 {
        self.prev_raw.wrapping_sub(1)
    }
    /// Next slot in the derivation list, or [`NONE`].
    pub fn next(&self) -> u32 {
        self.next_raw.wrapping_sub(1)
    }
    fn set_prev(&mut self, index: u32) {
        self.prev_raw = index.wrapping_add(1);
    }
    fn set_next(&mut self, index: u32) {
        self.next_raw = index.wrapping_add(1);
    }

    /// The slot as 32 little-endian bytes: the form it has in memory.
    pub fn to_bytes(&self) -> [u8; SLOT_BYTES] {
        let mut b = [0u8; SLOT_BYTES];
        b[0..8].copy_from_slice(&self.object.to_le_bytes());
        b[8..16].copy_from_slice(&self.extra.to_le_bytes());
        b[16..20].copy_from_slice(&self.prev_raw.to_le_bytes());
        b[20..24].copy_from_slice(&self.next_raw.to_le_bytes());
        b[24..26].copy_from_slice(&self.depth.to_le_bytes());
        b[26] = self.kind as u8;
        b[27] = self.rights.0;
        b[28] = self.bits;
        b
    }

    /// Reads a slot from its 32 bytes, or `None` if they are not a slot this
    /// crate could have written (an unknown kind, a bit that is not a right,
    /// non-zero padding, an oversized region, or an "empty" slot with anything
    /// set). The kernel reads capability-space memory through this, so damaged
    /// memory is refused rather than trusted.
    pub fn from_bytes(b: &[u8; SLOT_BYTES]) -> Option<Slot> {
        let u64_at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap_or([0; 8]));
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap_or([0; 4]));
        let kind = Kind::from_u8(b[26])?;
        if b[27] & !Rights::ALL.0 != 0 || b[29..].iter().any(|&x| x != 0) || b[28] > MAX_BITS {
            return None;
        }
        let slot = Slot {
            object: u64_at(0),
            extra: u64_at(8),
            prev_raw: u32_at(16),
            next_raw: u32_at(20),
            depth: u16::from_le_bytes([b[24], b[25]]),
            kind,
            rights: Rights(b[27]),
            bits: b[28],
        };
        if kind == Kind::Empty && slot != Slot::EMPTY {
            return None;
        }
        Some(slot)
    }
    /// Which *object* this capability names: two capabilities with the same
    /// identity are copies of each other.
    pub fn identity(&self) -> (Kind, u64, u8) {
        (self.kind, self.object, self.bits)
    }
    /// True if the slot holds no capability.
    pub fn is_empty(&self) -> bool {
        self.kind == Kind::Empty
    }
}

/// Where slots live. The kernel implements this over its capability-space
/// memory; tests use a plain array. A slot is named by a `u32` index. Slots are
/// copied in and out (never borrowed), so a store may keep them anywhere,
/// including behind a physical-memory window.
pub trait SlotStore {
    /// Number of addressable slots (an upper bound on how far a walk can go).
    fn len(&self) -> u32;
    /// The slot at `index`, or `None` if out of range (or unreadable).
    fn load(&self, index: u32) -> Option<Slot>;
    /// Writes the slot at `index`. Returns false if `index` is out of range.
    fn store(&mut self, index: u32, value: Slot) -> bool;
    /// True if there are no slots.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl SlotStore for [Slot] {
    fn len(&self) -> u32 {
        <[Slot]>::len(self) as u32
    }
    fn load(&self, index: u32) -> Option<Slot> {
        self.get(index as usize).copied()
    }
    fn store(&mut self, index: u32, value: Slot) -> bool {
        match self.get_mut(index as usize) {
            Some(target) => {
                *target = value;
                true
            }
            None => false,
        }
    }
}

/// Why an operation was refused. Every refusal changes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A slot index is out of range.
    InvalidSlot,
    /// The slot that should hold a capability is empty.
    EmptySlot,
    /// The destination slot is already in use.
    DestinationOccupied,
    /// Source and destination (or two destinations) are the same slot.
    SameSlot,
    /// The requested rights are not a subset of the source's.
    RightsEscalation,
    /// Only endpoint and notification capabilities carry a badge.
    CannotBadge,
    /// The capability already has a badge, and a badge is set only once.
    AlreadyBadged,
    /// A badge of zero means "none" and cannot be set.
    ZeroBadge,
    /// Untyped capabilities cannot be copied, only retyped.
    CannotDeriveUntyped,
    /// The slot is not an untyped capability.
    NotUntyped,
    /// An object size is outside `MIN_OBJECT_BITS ..= MAX_BITS`, or larger than the region.
    BadSize,
    /// A count is zero, larger than [`MAX_RETYPE`], or does not match the slots given.
    BadCount,
    /// The region does not have enough free space above its watermark.
    OutOfMemory,
    /// The derivation depth would overflow.
    DepthLimit,
    /// The list is corrupt (a bug, or memory damage). The kernel should panic.
    Corrupt,
}

/// An object whose last capability has just gone: the kernel frees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Destroyed {
    /// What the object was.
    pub kind: Kind,
    /// Its physical address.
    pub object: u64,
    /// Its size as a power of two.
    pub bits: u8,
}

fn slot<S: SlotStore + ?Sized>(store: &S, index: u32) -> Result<Slot, Error> {
    store.load(index).ok_or(Error::InvalidSlot)
}

fn put<S: SlotStore + ?Sized>(store: &mut S, index: u32, value: Slot) {
    // Every caller has just loaded `index`, so it is in range.
    let _ = store.store(index, value);
}

fn require_empty<S: SlotStore + ?Sized>(store: &S, index: u32) -> Result<(), Error> {
    if slot(store, index)?.is_empty() {
        Ok(())
    } else {
        Err(Error::DestinationOccupied)
    }
}

fn child_depth(parent: &Slot) -> Result<u16, Error> {
    parent.depth.checked_add(1).ok_or(Error::DepthLimit)
}

/// Inserts `new` (already filled in) at `at`, immediately after `parent`.
fn link_after<S: SlotStore + ?Sized>(
    store: &mut S,
    parent: u32,
    at: u32,
    mut new: Slot,
) -> Result<(), Error> {
    let mut p = slot(store, parent)?;
    let following = p.next();
    new.set_prev(parent);
    new.set_next(following);
    put(store, at, new);
    p.set_next(at);
    put(store, parent, p);
    if following != NONE {
        let mut f = slot(store, following)?;
        f.set_prev(at);
        put(store, following, f);
    }
    Ok(())
}

/// Removes the slot from the list (but does not reparent anything).
fn unlink<S: SlotStore + ?Sized>(store: &mut S, index: u32) -> Result<(), Error> {
    let s = slot(store, index)?;
    if s.prev() != NONE {
        let mut p = slot(store, s.prev())?;
        p.set_next(s.next());
        put(store, s.prev(), p);
    }
    if s.next() != NONE {
        let mut n = slot(store, s.next())?;
        n.set_prev(s.prev());
        put(store, s.next(), n);
    }
    put(store, index, Slot::EMPTY);
    Ok(())
}

/// Places a root capability (depth 0, no parent) in an empty slot. The kernel
/// uses this at boot to give the root task its initial authority; nothing a
/// task can invoke reaches it.
pub fn install_root<S: SlotStore + ?Sized>(
    store: &mut S,
    at: u32,
    kind: Kind,
    object: u64,
    bits: u8,
    rights: Rights,
) -> Result<(), Error> {
    require_empty(store, at)?;
    if kind == Kind::Empty || bits > MAX_BITS {
        return Err(Error::BadSize);
    }
    put(
        store,
        at,
        Slot {
            kind,
            rights,
            bits,
            depth: 0,
            object,
            extra: 0,
            prev_raw: 0,
            next_raw: 0,
        },
    );
    Ok(())
}

/// True if the slot after `index` descends from it.
pub fn has_descendants<S: SlotStore + ?Sized>(store: &S, index: u32) -> Result<bool, Error> {
    let s = slot(store, index)?;
    if s.next() == NONE {
        return Ok(false);
    }
    Ok(slot(store, s.next())?.depth > s.depth)
}

/// The slot this one was derived from: the nearest earlier slot with a smaller
/// depth, or `None` for a root.
pub fn parent_of<S: SlotStore + ?Sized>(store: &S, index: u32) -> Result<Option<u32>, Error> {
    let s = slot(store, index)?;
    if s.depth == 0 {
        return Ok(None);
    }
    let mut at = s.prev();
    // A well-formed list has the parent within `len` steps; the bound stops a
    // corrupt (cyclic) list from looping forever.
    for _ in 0..store.len() {
        if at == NONE {
            return Err(Error::Corrupt);
        }
        let p = slot(store, at)?;
        if p.depth < s.depth {
            return Ok(Some(at));
        }
        at = p.prev();
    }
    Err(Error::Corrupt)
}

/// True if no other capability names the same object. Copies of an object are
/// adjacent in the list (they are inserted next to their parent), so looking at
/// the two neighbours is enough; the property tests check this against a scan.
pub fn is_final<S: SlotStore + ?Sized>(store: &S, index: u32) -> Result<bool, Error> {
    let s = slot(store, index)?;
    if s.is_empty() {
        return Err(Error::EmptySlot);
    }
    for neighbour in [s.prev(), s.next()] {
        if neighbour != NONE && slot(store, neighbour)?.identity() == s.identity() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Copies the capability in `src` into the empty slot `dst`, with rights
/// restricted to `rights` and, for an endpoint or notification, optionally
/// stamped with a badge. The copy becomes a child of `src`.
pub fn derive<S: SlotStore + ?Sized>(
    store: &mut S,
    src: u32,
    dst: u32,
    rights: Rights,
    badge: Option<u64>,
) -> Result<(), Error> {
    let source = slot(store, src)?;
    if src == dst {
        return Err(Error::SameSlot);
    }
    if source.is_empty() {
        return Err(Error::EmptySlot);
    }
    require_empty(store, dst)?;
    if source.kind == Kind::Untyped {
        return Err(Error::CannotDeriveUntyped);
    }
    if !source.rights.contains(rights) {
        return Err(Error::RightsEscalation);
    }
    let mut extra = source.extra;
    if let Some(value) = badge {
        if !matches!(source.kind, Kind::Endpoint | Kind::Notification) {
            return Err(Error::CannotBadge);
        }
        if value == 0 {
            return Err(Error::ZeroBadge);
        }
        if source.extra != 0 {
            return Err(Error::AlreadyBadged);
        }
        extra = value;
    }
    let depth = child_depth(&source)?;
    let new = Slot {
        rights,
        depth,
        extra,
        ..source
    };
    link_after(store, src, dst, new)
}

/// Moves the capability from `src` to the empty slot `dst`, keeping its place in
/// the derivation list.
pub fn move_cap<S: SlotStore + ?Sized>(store: &mut S, src: u32, dst: u32) -> Result<(), Error> {
    let source = slot(store, src)?;
    if src == dst {
        return Err(Error::SameSlot);
    }
    if source.is_empty() {
        return Err(Error::EmptySlot);
    }
    require_empty(store, dst)?;
    put(store, dst, source);
    if source.prev() != NONE {
        let mut p = slot(store, source.prev())?;
        p.set_next(dst);
        put(store, source.prev(), p);
    }
    if source.next() != NONE {
        let mut n = slot(store, source.next())?;
        n.set_prev(dst);
        put(store, source.next(), n);
    }
    put(store, src, Slot::EMPTY);
    Ok(())
}

/// Deletes one capability. Its descendants are not deleted: they become
/// children of its parent. If it was the last capability to its object,
/// `destroyed` is told so the kernel can free the object. If the parent is an
/// untyped region left with no descendants, its watermark drops to zero.
pub fn delete<S: SlotStore + ?Sized>(
    store: &mut S,
    index: u32,
    destroyed: &mut dyn FnMut(Destroyed),
) -> Result<(), Error> {
    let s = slot(store, index)?;
    if s.is_empty() {
        return Err(Error::EmptySlot);
    }
    let last = is_final(store, index)?;
    let parent = parent_of(store, index)?;

    // Descendants move up one level so the depth chain stays unbroken.
    let mut at = s.next();
    while at != NONE {
        let mut d = slot(store, at)?;
        if d.depth <= s.depth {
            break;
        }
        let next = d.next();
        d.depth -= 1;
        put(store, at, d);
        at = next;
    }

    unlink(store, index)?;

    if let Some(p) = parent {
        let mut ps = slot(store, p)?;
        if ps.kind == Kind::Untyped && ps.extra != 0 && !has_descendants(store, p)? {
            ps.extra = 0;
            put(store, p, ps);
        }
    }
    if last {
        destroyed(Destroyed {
            kind: s.kind,
            object: s.object,
            bits: s.bits,
        });
    }
    Ok(())
}

/// Deletes everything derived from `index`, leaving `index` itself. Returns how
/// many capabilities were deleted. If `index` is an untyped region, its
/// watermark ends at zero (nothing is left to conflict with).
pub fn revoke<S: SlotStore + ?Sized>(
    store: &mut S,
    index: u32,
    destroyed: &mut dyn FnMut(Destroyed),
) -> Result<u32, Error> {
    let s = slot(store, index)?;
    if s.is_empty() {
        return Err(Error::EmptySlot);
    }
    let mut count = 0;
    while has_descendants(store, index)? {
        let next = slot(store, index)?.next();
        delete(store, next, destroyed)?;
        count += 1;
    }
    // If `index` is an untyped region its watermark is already back at zero:
    // `delete` resets it when the last descendant goes.
    Ok(count)
}

/// Carves `count` objects of one kind, each `1 << object_bits` bytes, out of the
/// untyped region in `untyped`, above its watermark and aligned to the object
/// size. The new capabilities go into the empty, distinct slots `into` and are
/// children of the region. The caller (the kernel) zeroes the new memory.
pub fn retype<S: SlotStore + ?Sized>(
    store: &mut S,
    untyped: u32,
    kind: Kind,
    object_bits: u8,
    into: &[u32],
) -> Result<(), Error> {
    let region = slot(store, untyped)?;
    if region.is_empty() {
        return Err(Error::EmptySlot);
    }
    if region.kind != Kind::Untyped {
        return Err(Error::NotUntyped);
    }
    if kind == Kind::Empty {
        return Err(Error::BadSize);
    }
    if into.is_empty() || into.len() > MAX_RETYPE {
        return Err(Error::BadCount);
    }
    if !(MIN_OBJECT_BITS..=MAX_BITS).contains(&object_bits) || object_bits > region.bits {
        return Err(Error::BadSize);
    }
    // A child region the size of its parent would have the parent's identity
    // (same kind, address and size) and be mistaken for a copy of it.
    if kind == Kind::Untyped && object_bits == region.bits {
        return Err(Error::BadSize);
    }
    for (i, &a) in into.iter().enumerate() {
        if a == untyped || into[..i].contains(&a) {
            return Err(Error::SameSlot);
        }
        require_empty(store, a)?;
    }
    let size = 1u64 << object_bits;
    let start = region
        .extra
        .checked_add(size - 1)
        .ok_or(Error::OutOfMemory)?
        & !(size - 1);
    let total = size
        .checked_mul(into.len() as u64)
        .ok_or(Error::OutOfMemory)?;
    let end = start.checked_add(total).ok_or(Error::OutOfMemory)?;
    if end > (1u64 << region.bits) {
        return Err(Error::OutOfMemory);
    }
    let depth = child_depth(&region)?;

    // Insert in reverse so the list ends up in address order.
    for (i, &at) in into.iter().enumerate().rev() {
        let new = Slot {
            kind,
            rights: Rights::ALL,
            bits: object_bits,
            depth,
            object: region.object + start + size * i as u64,
            extra: 0,
            prev_raw: 0,
            next_raw: 0,
        };
        link_after(store, untyped, at, new)?;
    }
    let mut region = slot(store, untyped)?;
    region.extra = end;
    put(store, untyped, region);
    Ok(())
}

/// Checks every structural rule on the whole store. The tests call this after
/// every operation; the kernel can call it in self-tests. `Err` names the first
/// rule that does not hold.
pub fn check_invariants<S: SlotStore + ?Sized>(store: &S) -> Result<(), &'static str> {
    let n = store.len();
    let get = |i: u32| store.load(i).ok_or("slot index out of range");

    for i in 0..n {
        let s = get(i)?;
        if s.is_empty() {
            if s.prev() != NONE || s.next() != NONE || s.depth != 0 || s.extra != 0 {
                return Err("an empty slot has leftover state");
            }
            continue;
        }
        if s.next() != NONE {
            let next = get(s.next()).map_err(|_| "next points outside the store")?;
            if next.is_empty() || next.prev() != i {
                return Err("list links are not symmetric (next)");
            }
            if next.depth > s.depth.saturating_add(1) {
                return Err("depth jumps by more than one");
            }
        }
        if s.prev() != NONE {
            let prev = get(s.prev()).map_err(|_| "prev points outside the store")?;
            if prev.is_empty() || prev.next() != i {
                return Err("list links are not symmetric (prev)");
            }
        } else if s.depth != 0 {
            return Err("a chain head must have depth 0");
        }
        // Following `next` must end within `n` steps (no cycle).
        let mut at = s.next();
        let mut steps = 0;
        while at != NONE {
            steps += 1;
            if steps > n {
                return Err("the list contains a cycle");
            }
            at = get(at)?.next();
        }
    }

    for i in 0..n {
        let s = get(i)?;
        if s.is_empty() {
            continue;
        }
        // A copy is never stronger than its parent, and a badge never changes.
        if let Some(p) = parent_of(store, i).map_err(|_| "no parent found for a non-root")? {
            let ps = get(p)?;
            if u32::from(ps.depth) + 1 != u32::from(s.depth) {
                return Err("a parent's depth is not exactly one less than its child's");
            }
            if ps.identity() == s.identity() {
                if !ps.rights.contains(s.rights) {
                    return Err("a copy has rights its parent lacks");
                }
                if s.kind != Kind::Untyped && s.extra != ps.extra && ps.extra != 0 {
                    return Err("a badge changed along a derivation");
                }
            }
        }
        // The neighbour test for "last capability" agrees with a full scan.
        let scan_final = (0..n).all(|j| {
            j == i
                || store
                    .load(j)
                    .is_none_or(|o| o.is_empty() || o.identity() != s.identity())
        });
        if is_final(store, i).map_err(|_| "is_final failed")? != scan_final {
            return Err("neighbour check for the last capability disagrees with a scan");
        }
    }

    for i in 0..n {
        let u = get(i)?;
        if u.kind != Kind::Untyped {
            continue;
        }
        let region_size = 1u64 << u.bits;
        if u.extra > region_size {
            return Err("an untyped watermark passes the end of its region");
        }
        if !has_descendants(store, i).map_err(|_| "has_descendants failed")? && u.extra != 0 {
            return Err("an untyped region has a watermark but no descendants");
        }
        // Direct children are objects carved from the region: inside the used
        // part, aligned, and pairwise disjoint unless they are copies.
        let mut at = u.next();
        let mut children = 0u32;
        while at != NONE {
            let c = get(at)?;
            if c.depth <= u.depth {
                break;
            }
            if c.depth == u.depth + 1 {
                children += 1;
                let size = 1u64 << c.bits;
                if c.object < u.object
                    || c.object.checked_add(size).is_none()
                    || c.object + size > u.object + u.extra
                {
                    return Err("an object lies outside the used part of its region");
                }
                if (c.object - u.object) % size != 0 {
                    return Err("an object is not aligned to its size");
                }
                let mut other = c.next();
                while other != NONE {
                    let o = get(other)?;
                    if o.depth <= u.depth {
                        break;
                    }
                    if o.depth == u.depth + 1 && o.identity() != c.identity() {
                        let osize = 1u64 << o.bits;
                        if c.object < o.object + osize && o.object < c.object + size {
                            return Err("two objects carved from one region overlap");
                        }
                    }
                    other = o.next();
                }
            }
            at = c.next();
        }
        let _ = children;
        // Two untyped capabilities for the same region would both be able to retype it.
        for j in (i + 1)..n {
            let o = get(j)?;
            if o.kind == Kind::Untyped && o.identity() == u.identity() {
                return Err("two untyped capabilities name the same region");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
