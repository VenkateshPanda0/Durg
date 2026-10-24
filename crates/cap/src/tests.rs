//! Tests for the capability core. Two kinds: handwritten scenarios that pin each
//! rule down, and a property test that runs random operation sequences and
//! checks every invariant after every step.

extern crate std;

use super::*;
use proptest::prelude::*;
use std::collections::BTreeSet;
use std::vec::Vec;

const ROOT_OBJECT: u64 = 0x10_0000;

fn store(n: usize) -> Vec<Slot> {
    std::vec![Slot::EMPTY; n]
}

fn with_root(n: usize, bits: u8) -> Vec<Slot> {
    let mut s = store(n);
    install_root(&mut s[..], 0, Kind::Untyped, ROOT_OBJECT, bits, Rights::ALL).unwrap();
    s
}

fn ok(s: &Result<(), Error>) {
    assert_eq!(*s, Ok(()));
}

fn no_destroy() -> impl FnMut(Destroyed) {
    |d| panic!("nothing should be destroyed, but {d:?} was")
}

// ---- derive ---------------------------------------------------------------

#[test]
fn derive_copies_with_fewer_rights_and_refuses_more() {
    let mut s = with_root(8, 16);
    ok(&retype(&mut s[..], 0, Kind::Endpoint, 4, &[1]));
    ok(&derive(&mut s[..], 1, 2, Rights::READ, None));
    assert_eq!(s[2].rights(), Rights::READ);
    assert_eq!(s[2].depth(), 2);
    assert_eq!(s[2].identity(), s[1].identity());
    // READ is all slot 2 has; it cannot hand out WRITE, nor re-derive more.
    assert_eq!(
        derive(&mut s[..], 2, 3, Rights::WRITE, None),
        Err(Error::RightsEscalation)
    );
    assert_eq!(
        derive(&mut s[..], 2, 3, Rights::ALL, None),
        Err(Error::RightsEscalation)
    );
    assert!(s[3].is_empty(), "a refused derive changes nothing");
    check_invariants(&s[..]).unwrap();
}

#[test]
fn badges_are_set_once_and_only_on_endpoints_and_notifications() {
    let mut s = with_root(8, 16);
    ok(&retype(&mut s[..], 0, Kind::Endpoint, 4, &[1]));
    ok(&retype(&mut s[..], 0, Kind::Frame, 12, &[2]));
    ok(&derive(&mut s[..], 1, 3, Rights::ALL, Some(7)));
    assert_eq!(s[3].extra(), 7);
    assert_eq!(s[1].extra(), 0, "the original stays unbadged");
    assert_eq!(
        derive(&mut s[..], 3, 4, Rights::ALL, Some(9)),
        Err(Error::AlreadyBadged)
    );
    // A copy without a badge argument keeps the badge it was given.
    ok(&derive(&mut s[..], 3, 4, Rights::ALL, None));
    assert_eq!(s[4].extra(), 7);
    assert_eq!(
        derive(&mut s[..], 1, 5, Rights::ALL, Some(0)),
        Err(Error::ZeroBadge)
    );
    assert_eq!(
        derive(&mut s[..], 2, 5, Rights::ALL, Some(1)),
        Err(Error::CannotBadge)
    );
    check_invariants(&s[..]).unwrap();
}

#[test]
fn untyped_cannot_be_copied() {
    let mut s = with_root(4, 16);
    assert_eq!(
        derive(&mut s[..], 0, 1, Rights::ALL, None),
        Err(Error::CannotDeriveUntyped)
    );
}

#[test]
fn basic_argument_errors_change_nothing() {
    let mut s = with_root(4, 16);
    ok(&retype(&mut s[..], 0, Kind::Endpoint, 4, &[1]));
    let before = s.clone();
    assert_eq!(
        derive(&mut s[..], 9, 2, Rights::ALL, None),
        Err(Error::InvalidSlot)
    );
    assert_eq!(
        derive(&mut s[..], 1, 9, Rights::ALL, None),
        Err(Error::InvalidSlot)
    );
    assert_eq!(
        derive(&mut s[..], 2, 3, Rights::ALL, None),
        Err(Error::EmptySlot)
    );
    assert_eq!(
        derive(&mut s[..], 1, 1, Rights::ALL, None),
        Err(Error::SameSlot)
    );
    assert_eq!(
        derive(&mut s[..], 1, 0, Rights::ALL, None),
        Err(Error::DestinationOccupied)
    );
    assert_eq!(s, before);
}

#[test]
fn derive_at_the_depth_limit_is_refused() {
    let mut s = with_root(4, 16);
    ok(&retype(&mut s[..], 0, Kind::Endpoint, 4, &[1]));
    s[1].depth = u16::MAX;
    assert_eq!(
        derive(&mut s[..], 1, 2, Rights::ALL, None),
        Err(Error::DepthLimit)
    );
}

// ---- retype ---------------------------------------------------------------

#[test]
fn retype_aligns_objects_and_advances_the_watermark() {
    let mut s = with_root(8, 12); // a 4 KiB region
    ok(&retype(&mut s[..], 0, Kind::Frame, 4, &[1])); // 16 bytes at 0
    assert_eq!(s[1].object(), ROOT_OBJECT);
    assert_eq!(s[0].extra(), 16);
    ok(&retype(&mut s[..], 0, Kind::Frame, 6, &[2, 3])); // two 64-byte objects
    assert_eq!(
        s[2].object(),
        ROOT_OBJECT + 64,
        "aligned up past the 16 used"
    );
    assert_eq!(s[3].object(), ROOT_OBJECT + 128);
    assert_eq!(s[0].extra(), 192);
    assert_eq!(s[1].depth(), 1);
    check_invariants(&s[..]).unwrap();
}

#[test]
fn retype_out_of_memory_changes_nothing() {
    let mut s = with_root(8, 8); // 256 bytes
    ok(&retype(&mut s[..], 0, Kind::Frame, 7, &[1])); // 128 bytes
    let before = s.clone();
    assert_eq!(
        retype(&mut s[..], 0, Kind::Frame, 7, &[2, 3]),
        Err(Error::OutOfMemory)
    );
    assert_eq!(s, before);
    // The last 128 bytes still fit exactly one.
    ok(&retype(&mut s[..], 0, Kind::Frame, 7, &[2]));
    assert_eq!(s[0].extra(), 256);
    assert_eq!(
        retype(&mut s[..], 0, Kind::Frame, 4, &[3]),
        Err(Error::OutOfMemory),
        "the region is full"
    );
}

#[test]
fn retype_rejects_bad_requests() {
    let mut s = with_root(8, 12);
    ok(&retype(&mut s[..], 0, Kind::Frame, 4, &[1]));
    let before = s.clone();
    assert_eq!(
        retype(&mut s[..], 1, Kind::Frame, 4, &[2]),
        Err(Error::NotUntyped)
    );
    assert_eq!(
        retype(&mut s[..], 5, Kind::Frame, 4, &[2]),
        Err(Error::EmptySlot)
    );
    assert_eq!(
        retype(&mut s[..], 0, Kind::Empty, 4, &[2]),
        Err(Error::BadSize)
    );
    assert_eq!(
        retype(&mut s[..], 0, Kind::Frame, 3, &[2]),
        Err(Error::BadSize)
    );
    assert_eq!(
        retype(&mut s[..], 0, Kind::Frame, 13, &[2]),
        Err(Error::BadSize)
    );
    // A child region as big as its parent would be mistaken for a copy of it.
    assert_eq!(
        retype(&mut s[..], 0, Kind::Untyped, 12, &[2]),
        Err(Error::BadSize)
    );
    assert_eq!(
        retype(&mut s[..], 0, Kind::Frame, 4, &[]),
        Err(Error::BadCount)
    );
    assert_eq!(
        retype(&mut s[..], 0, Kind::Frame, 4, &[2, 2]),
        Err(Error::SameSlot)
    );
    assert_eq!(
        retype(&mut s[..], 0, Kind::Frame, 4, &[0]),
        Err(Error::SameSlot)
    );
    assert_eq!(
        retype(&mut s[..], 0, Kind::Frame, 4, &[1]),
        Err(Error::DestinationOccupied)
    );
    assert_eq!(
        retype(&mut s[..], 0, Kind::Frame, 4, &[2, 99]),
        Err(Error::InvalidSlot)
    );
    assert_eq!(s, before, "every refusal leaves the store untouched");
}

#[test]
fn retype_can_subdivide_an_untyped_region() {
    let mut s = with_root(8, 12);
    ok(&retype(&mut s[..], 0, Kind::Untyped, 10, &[1])); // a 1 KiB child
    ok(&retype(&mut s[..], 1, Kind::Frame, 4, &[2]));
    assert_eq!(s[2].object(), ROOT_OBJECT);
    assert_eq!(s[2].depth(), 2);
    // The parent cannot hand out the child's memory a second time.
    ok(&retype(&mut s[..], 0, Kind::Frame, 4, &[3]));
    assert_eq!(s[3].object(), ROOT_OBJECT + 1024);
    check_invariants(&s[..]).unwrap();
}

// ---- delete and revoke -----------------------------------------------------

#[test]
fn deleting_the_last_capability_reports_the_object_once() {
    let mut s = with_root(8, 12);
    ok(&retype(&mut s[..], 0, Kind::Frame, 4, &[1]));
    ok(&derive(&mut s[..], 1, 2, Rights::READ, None));
    // The original goes; a copy remains, so the frame is not destroyed.
    delete(&mut s[..], 1, &mut no_destroy()).unwrap();
    // The copy was a child of the original; now it hangs off the region.
    assert_eq!(s[2].depth(), 1);
    check_invariants(&s[..]).unwrap();
    let mut log = Vec::new();
    delete(&mut s[..], 2, &mut |d| log.push(d)).unwrap();
    assert_eq!(
        log,
        [Destroyed {
            kind: Kind::Frame,
            object: ROOT_OBJECT,
            bits: 4
        }]
    );
    assert_eq!(
        s[0].extra(),
        0,
        "no descendants left, so the watermark resets"
    );
    assert_eq!(
        delete(&mut s[..], 2, &mut no_destroy()),
        Err(Error::EmptySlot)
    );
}

#[test]
fn delete_moves_descendants_up_one_level() {
    let mut s = with_root(8, 12);
    ok(&retype(&mut s[..], 0, Kind::Endpoint, 4, &[1]));
    ok(&derive(&mut s[..], 1, 2, Rights::ALL, None));
    ok(&derive(&mut s[..], 2, 3, Rights::READ, None));
    assert_eq!((s[1].depth(), s[2].depth(), s[3].depth()), (1, 2, 3));
    delete(&mut s[..], 2, &mut no_destroy()).unwrap();
    assert_eq!(s[3].depth(), 2);
    assert_eq!(parent_of(&s[..], 3), Ok(Some(1)));
    check_invariants(&s[..]).unwrap();
}

#[test]
fn deleting_one_of_several_children_keeps_the_watermark() {
    let mut s = with_root(8, 12);
    ok(&retype(&mut s[..], 0, Kind::Frame, 4, &[1, 2]));
    delete(&mut s[..], 1, &mut |_| {}).unwrap();
    assert_eq!(
        s[0].extra(),
        32,
        "memory is not reused while a sibling lives"
    );
    delete(&mut s[..], 2, &mut |_| {}).unwrap();
    assert_eq!(s[0].extra(), 0);
}

#[test]
fn revoke_removes_everything_derived_and_returns_the_memory() {
    let mut s = with_root(12, 12);
    ok(&retype(&mut s[..], 0, Kind::Frame, 4, &[1, 2]));
    ok(&retype(&mut s[..], 0, Kind::Endpoint, 4, &[3]));
    ok(&derive(&mut s[..], 3, 4, Rights::READ, Some(5)));
    ok(&derive(&mut s[..], 1, 5, Rights::READ, None));
    let mut log = Vec::new();
    let n = revoke(&mut s[..], 0, &mut |d| log.push(d)).unwrap();
    assert_eq!(n, 5);
    assert_eq!(log.len(), 3, "two frames and the endpoint, each once");
    assert!(s[1..].iter().all(Slot::is_empty));
    assert!(!s[0].is_empty());
    assert_eq!(s[0].extra(), 0);
    // The region is usable again from its start.
    ok(&retype(&mut s[..], 0, Kind::Frame, 4, &[1]));
    assert_eq!(s[1].object(), ROOT_OBJECT);
    check_invariants(&s[..]).unwrap();
}

#[test]
fn revoke_reaches_nested_regions() {
    let mut s = with_root(12, 14);
    ok(&retype(&mut s[..], 0, Kind::Untyped, 12, &[1]));
    ok(&retype(&mut s[..], 1, Kind::Frame, 6, &[2, 3]));
    ok(&derive(&mut s[..], 2, 4, Rights::READ, None));
    let n = revoke(&mut s[..], 0, &mut |_| {}).unwrap();
    assert_eq!(n, 4);
    assert!(s[1..].iter().all(Slot::is_empty));
    check_invariants(&s[..]).unwrap();
}

#[test]
fn revoking_a_copy_leaves_the_original() {
    let mut s = with_root(8, 12);
    ok(&retype(&mut s[..], 0, Kind::Endpoint, 4, &[1]));
    ok(&derive(&mut s[..], 1, 2, Rights::ALL, None));
    ok(&derive(&mut s[..], 2, 3, Rights::READ, None));
    let n = revoke(&mut s[..], 2, &mut no_destroy()).unwrap();
    assert_eq!(n, 1);
    assert!(s[3].is_empty());
    assert!(!s[1].is_empty() && !s[2].is_empty());
}

#[test]
fn parent_of_skips_siblings() {
    let mut s = with_root(8, 12);
    ok(&retype(&mut s[..], 0, Kind::Frame, 4, &[1, 2, 3]));
    ok(&derive(&mut s[..], 2, 4, Rights::READ, None));
    // 1, 2 and 3 are siblings under the region; 4 hangs off 2.
    assert_eq!(parent_of(&s[..], 0), Ok(None));
    for child in [1, 2, 3] {
        assert_eq!(parent_of(&s[..], child), Ok(Some(0)), "slot {child}");
    }
    assert_eq!(parent_of(&s[..], 4), Ok(Some(2)));
}

// ---- move -----------------------------------------------------------------

#[test]
fn move_keeps_the_place_in_the_derivation() {
    let mut s = with_root(8, 12);
    ok(&retype(&mut s[..], 0, Kind::Endpoint, 4, &[1]));
    ok(&derive(&mut s[..], 1, 2, Rights::READ, None));
    ok(&move_cap(&mut s[..], 1, 5));
    assert!(s[1].is_empty());
    assert_eq!(s[5].depth(), 1);
    assert_eq!(s[2].prev(), 5);
    assert_eq!(parent_of(&s[..], 2), Ok(Some(5)));
    check_invariants(&s[..]).unwrap();
    assert_eq!(move_cap(&mut s[..], 5, 5), Err(Error::SameSlot));
    assert_eq!(move_cap(&mut s[..], 1, 6), Err(Error::EmptySlot));
    assert_eq!(move_cap(&mut s[..], 5, 2), Err(Error::DestinationOccupied));
}

// ---- rights ---------------------------------------------------------------

#[test]
fn rights_contains_is_a_subset_test() {
    let rw = Rights::from_bits_truncate(Rights::READ.bits() | Rights::WRITE.bits());
    assert!(rw.contains(Rights::READ));
    assert!(rw.contains(Rights::NONE));
    assert!(!Rights::READ.contains(rw));
    assert!(Rights::ALL.contains(rw));
    assert_eq!(Rights::from_bits_truncate(0xff), Rights::ALL);
}

// ---- the byte layout ---------------------------------------------------------

#[test]
fn a_slot_is_32_bytes_and_zero_bytes_are_empty() {
    assert_eq!(core::mem::size_of::<Slot>(), 32);
    assert_eq!(Slot::EMPTY.to_bytes(), [0u8; SLOT_BYTES]);
    assert_eq!(Slot::from_bytes(&[0u8; SLOT_BYTES]), Some(Slot::EMPTY));
    // So freshly zeroed memory is a valid space of empty slots, with every link "none".
    assert_eq!(Slot::EMPTY.prev(), NONE);
    assert_eq!(Slot::EMPTY.next(), NONE);
}

#[test]
fn slots_round_trip_through_bytes() {
    let mut s = with_root(8, 14);
    ok(&retype(&mut s[..], 0, Kind::Endpoint, 4, &[1, 2]));
    ok(&derive(&mut s[..], 1, 3, Rights::READ, Some(0xdead_beef)));
    for slot in &s {
        assert_eq!(Slot::from_bytes(&slot.to_bytes()), Some(*slot));
    }
}

#[test]
fn damaged_bytes_are_refused() {
    let mut s = with_root(4, 12);
    ok(&retype(&mut s[..], 0, Kind::Frame, 4, &[1]));
    let good = s[1].to_bytes();
    assert!(Slot::from_bytes(&good).is_some());
    let mut b = good;
    b[26] = 9; // not a kind
    assert!(Slot::from_bytes(&b).is_none());
    let mut b = good;
    b[27] = 0x80; // not a right
    assert!(Slot::from_bytes(&b).is_none());
    let mut b = good;
    b[30] = 1; // padding must be zero
    assert!(Slot::from_bytes(&b).is_none());
    let mut b = good;
    b[28] = 63; // larger than any region
    assert!(Slot::from_bytes(&b).is_none());
    // An "empty" slot with anything else set.
    let mut b = [0u8; SLOT_BYTES];
    b[8] = 1;
    assert!(Slot::from_bytes(&b).is_none());
    let mut b = [0u8; SLOT_BYTES];
    b[16] = 1;
    assert!(Slot::from_bytes(&b).is_none());
}

#[test]
fn kind_numbers_are_stable() {
    for n in 0..=7u8 {
        assert_eq!(Kind::from_u8(n).map(|k| k as u8), Some(n));
    }
    assert_eq!(Kind::from_u8(8), None);
}

// ---- a deliberately broken store is detected -------------------------------

#[test]
fn the_invariant_checker_notices_damage() {
    let mut s = with_root(8, 12);
    ok(&retype(&mut s[..], 0, Kind::Frame, 4, &[1, 2]));
    check_invariants(&s[..]).unwrap();

    // Break the links.
    let mut broken = s.clone();
    broken[1].set_next(7);
    assert!(check_invariants(&broken[..]).is_err());
    // A copy stronger than its parent.
    let mut stronger = with_root(8, 12);
    ok(&retype(&mut stronger[..], 0, Kind::Endpoint, 4, &[1]));
    ok(&derive(&mut stronger[..], 1, 2, Rights::READ, None));
    ok(&derive(&mut stronger[..], 2, 3, Rights::READ, None));
    stronger[3].rights = Rights::ALL;
    assert!(check_invariants(&stronger[..]).is_err());
    // Overlapping objects of different sizes. (Two siblings with the same kind,
    // address and size cannot be told from reparented copies of one object, so
    // the checker does not claim to detect that corruption; retype never makes it.)
    let mut overlap = s.clone();
    overlap[2].bits = 5;
    overlap[2].object = overlap[1].object;
    assert!(check_invariants(&overlap[..]).is_err());
    // A watermark with nothing under it.
    let mut ghost = with_root(4, 12);
    ghost[0].extra = 16;
    assert!(check_invariants(&ghost[..]).is_err());
    // An object outside the used part of the region.
    let mut outside = s.clone();
    outside[2].object = ROOT_OBJECT + 4000;
    assert!(check_invariants(&outside[..]).is_err());
}

// ---- the property test ------------------------------------------------------

const SLOTS: usize = 12;

#[derive(Clone, Debug)]
enum Op {
    Derive {
        src: u32,
        dst: u32,
        rights: u8,
        badge: Option<u64>,
    },
    Move {
        src: u32,
        dst: u32,
    },
    Delete(u32),
    Revoke(u32),
    Retype {
        ut: u32,
        kind: u8,
        bits: u8,
        into: Vec<u32>,
    },
}

fn slot_index() -> impl Strategy<Value = u32> {
    // Mostly in range, occasionally out of range to exercise refusals.
    prop_oneof![9 => 0u32..SLOTS as u32, 1 => SLOTS as u32..SLOTS as u32 + 3]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (slot_index(), slot_index(), 0u8..16, prop::option::of(0u64..4)).prop_map(
            |(src, dst, rights, badge)| Op::Derive { src, dst, rights, badge }
        ),
        1 => (slot_index(), slot_index()).prop_map(|(src, dst)| Op::Move { src, dst }),
        2 => slot_index().prop_map(Op::Delete),
        2 => slot_index().prop_map(Op::Revoke),
        4 => (slot_index(), 1u8..8, 3u8..16, prop::collection::vec(slot_index(), 0..4))
            .prop_map(|(ut, kind, bits, into)| Op::Retype { ut, kind, bits, into }),
    ]
}

fn kind_from(n: u8) -> Kind {
    match n {
        1 => Kind::Untyped,
        2 => Kind::CNode,
        3 => Kind::Endpoint,
        4 => Kind::Notification,
        5 => Kind::Thread,
        6 => Kind::Frame,
        _ => Kind::PageTable,
    }
}

fn objects(s: &[Slot]) -> BTreeSet<(u8, u64, u8)> {
    s.iter()
        .filter(|x| !x.is_empty())
        .map(|x| (x.kind() as u8, x.object(), x.bits()))
        .collect()
}

fn descendant_count(s: &[Slot], i: u32) -> u32 {
    let base = s[i as usize].depth();
    let mut n = 0;
    let mut at = s[i as usize].next();
    while at != NONE && s[at as usize].depth() > base {
        n += 1;
        at = s[at as usize].next();
    }
    n
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// After any sequence of operations, every invariant holds; a refused
    /// operation changes nothing; objects are reported destroyed exactly when
    /// their last capability goes; revoke leaves no descendant; and derive never
    /// grants more than the source has.
    #[test]
    fn random_operations_preserve_every_invariant(
        ops in prop::collection::vec(op(), 1..60),
        root_bits in 10u8..16,
    ) {
        let mut s = with_root(SLOTS, root_bits);
        for step in ops {
            let before = s.clone();
            let before_objects = objects(&before);
            let mut destroyed: Vec<Destroyed> = Vec::new();
            let mut sink = |d: Destroyed| destroyed.push(d);
            let result: Result<(), Error> = match &step {
                Op::Derive { src, dst, rights, badge } => {
                    let wanted = Rights::from_bits_truncate(*rights);
                    let r = derive(&mut s[..], *src, *dst, wanted, *badge);
                    if r.is_ok() {
                        let new = s[*dst as usize];
                        let from = s[*src as usize];
                        prop_assert!(from.rights().contains(new.rights()), "escalation");
                        prop_assert_eq!(new.identity(), from.identity());
                        prop_assert_eq!(new.depth(), from.depth() + 1);
                    }
                    r
                }
                Op::Move { src, dst } => move_cap(&mut s[..], *src, *dst),
                Op::Delete(i) => delete(&mut s[..], *i, &mut sink),
                Op::Revoke(i) => {
                    let expect = if (*i as usize) < SLOTS && !before[*i as usize].is_empty() {
                        Some(descendant_count(&before, *i))
                    } else {
                        None
                    };
                    match revoke(&mut s[..], *i, &mut sink) {
                        Ok(n) => {
                            prop_assert_eq!(Some(n), expect);
                            prop_assert!(!has_descendants(&s[..], *i).unwrap());
                            if s[*i as usize].kind() == Kind::Untyped {
                                prop_assert_eq!(s[*i as usize].extra(), 0);
                            }
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                }
                Op::Retype { ut, kind, bits, into } => {
                    let r = retype(&mut s[..], *ut, kind_from(*kind), *bits, into);
                    if r.is_ok() {
                        for &at in into {
                            let c = s[at as usize];
                            prop_assert_eq!(c.depth(), s[*ut as usize].depth() + 1);
                            prop_assert_eq!(c.kind(), kind_from(*kind));
                        }
                    }
                    r
                }
            };

            if result.is_err() {
                prop_assert_eq!(&s, &before, "a refused operation changed the store: {:?}", step);
                prop_assert!(destroyed.is_empty());
            }
            prop_assert_eq!(check_invariants(&s[..]), Ok(()), "after {:?}", step);
            for slot in &s {
                prop_assert_eq!(Slot::from_bytes(&slot.to_bytes()), Some(*slot));
            }

            // Identities that vanished are exactly the destroyed objects, each once.
            let after_objects = objects(&s);
            let vanished: BTreeSet<_> = before_objects.difference(&after_objects).cloned().collect();
            let reported: Vec<_> = destroyed
                .iter()
                .map(|d| (d.kind as u8, d.object, d.bits))
                .collect();
            let reported_set: BTreeSet<_> = reported.iter().cloned().collect();
            prop_assert_eq!(reported.len(), reported_set.len(), "an object was reported twice");
            prop_assert_eq!(&reported_set, &vanished, "destroyed set differs from vanished set");
            // Only retype can add objects.
            if !matches!(step, Op::Retype { .. }) {
                prop_assert!(after_objects.is_subset(&before_objects));
            }
        }

        // Finally, revoking the root and deleting it leaves nothing behind.
        let mut sink = |_: Destroyed| {};
        if !s[0].is_empty() {
            revoke(&mut s[..], 0, &mut sink).unwrap();
            prop_assert_eq!(check_invariants(&s[..]), Ok(()));
        }
    }
}
