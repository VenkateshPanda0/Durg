//! Fuzzes the capability core with random operation sequences. The input bytes
//! are decoded into operations (derive, move, delete, revoke, retype) on a small
//! slot arena that starts with one untyped region. After every operation the
//! whole structure must satisfy every invariant (`check_invariants`), a refused
//! operation must leave the arena untouched, and `revoke` must leave no
//! descendant. Slot numbers are deliberately sometimes out of range.

#![no_main]

use durg_cap::{
    check_invariants, delete, derive, has_descendants, install_root, move_cap, retype, revoke,
    Kind, Rights, Slot, SlotStore,
};
use libfuzzer_sys::fuzz_target;

const SLOTS: usize = 16;

fn kind(n: u8) -> Kind {
    match n % 7 {
        0 => Kind::Untyped,
        1 => Kind::CNode,
        2 => Kind::Endpoint,
        3 => Kind::Notification,
        4 => Kind::Thread,
        5 => Kind::Frame,
        _ => Kind::PageTable,
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&first, rest)) = data.split_first() else {
        return;
    };
    let mut arena = [Slot::EMPTY; SLOTS];
    // A region between 1 KiB and 1 MiB, at a fixed aligned address.
    install_root(
        &mut arena[..],
        0,
        Kind::Untyped,
        0x10_0000,
        10 + first % 11,
        Rights::ALL,
    )
    .unwrap();

    for op in rest.chunks(6) {
        let [code, a, b, c, d, e] = match *op {
            [code, a, b, c, d, e] => [code, a, b, c, d, e],
            _ => break,
        };
        // Mostly in range; sometimes just past the end.
        let ix = |x: u8| u32::from(x % (SLOTS as u8 + 3));
        let before = arena;
        let mut sink = |_| {};
        let result = match code % 5 {
            0 => derive(
                &mut arena[..],
                ix(a),
                ix(b),
                Rights::from_bits_truncate(c),
                if d % 3 == 0 { Some(u64::from(e % 4)) } else { None },
            ),
            1 => move_cap(&mut arena[..], ix(a), ix(b)),
            2 => delete(&mut arena[..], ix(a), &mut sink),
            3 => revoke(&mut arena[..], ix(a), &mut sink).map(|_| {
                assert!(!has_descendants(&arena[..], ix(a)).unwrap());
            }),
            _ => {
                let count = 1 + usize::from(d % 3);
                let into: [u32; 3] = [ix(b), ix(c), ix(e)];
                retype(&mut arena[..], ix(a), kind(code >> 3), 4 + d % 12, &into[..count])
            }
        };
        if result.is_err() {
            assert!(arena == before, "a refused operation changed the arena");
        }
        if let Err(rule) = check_invariants(&arena[..]) {
            panic!("invariant broken: {rule}");
        }
    }
    let _ = arena.len() as u32 == SlotStore::len(&arena[..]);
});
