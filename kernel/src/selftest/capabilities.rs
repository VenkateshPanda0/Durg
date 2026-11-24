//! Capability space tests (M10).
//!
//! Part of the in-kernel self-tests; see [`super`] for how they are run.

use super::*;

// ---- capabilities (M10) -------------------------------------------------------

pub(super) fn describe(kind: durg_cap::Kind, rights: durg_cap::Rights, bits: u8) -> u64 {
    u64::from(kind as u8) | u64::from(rights.bits()) << 8 | u64::from(bits) << 16
}

pub(super) fn identify(cs: &CSpace, slot: u64) -> Result<u64, SysError> {
    cs.invoke(slot, label::IDENTIFY, [0; 3])
}

pub(super) fn cspace_starts_with_only_a_capability_to_itself() {
    use durg_cap::{Kind, Rights};
    let free = user_tests::free_frames();
    let cs = CSpace::new().expect("a capability space");
    assert_eq!(
        user_tests::free_frames(),
        free - 1,
        "one frame for the CNode"
    );
    assert_eq!(identify(&cs, 0), Ok(describe(Kind::CNode, Rights::ALL, 12)));
    for slot in [1, 2, 64, 127] {
        assert_eq!(identify(&cs, slot), Err(SysError::EmptySlot), "slot {slot}");
    }
    for slot in [128, 129, 1 << 40, u64::MAX] {
        assert_eq!(
            identify(&cs, slot),
            Err(SysError::InvalidSlot),
            "slot {slot}"
        );
    }
    cs.destroy();
    assert_eq!(user_tests::free_frames(), free, "the CNode frame came back");
}

pub(super) fn retyped_memory_is_zeroed() {
    use durg_cap::Kind;
    let block = cspace::reserve_untyped(16).expect("a 64 KiB block");
    // Fill the whole region with junk first, as a previous owner of this memory
    // might have left it.
    let junk = alloc::vec![0xa5u8; 4096];
    for page in 0..16 {
        frame::write_bytes(block + page * 4096, &junk).expect("scribble");
    }
    let cs = CSpace::with_untyped(block, 16).expect("a capability space");
    let args = [retype_arg(Kind::Frame as u8, 12, 4), 2, 0];
    assert_eq!(cs.invoke(1, label::RETYPE, args), Ok(0));
    let mut page = alloc::vec![0u8; 4096];
    for index in 0..4u64 {
        frame::read_bytes(block + index * 4096, &mut page).expect("read");
        assert!(page.iter().all(|&b| b == 0), "frame {index} was not zeroed");
    }
    // Memory that was not made into an object is left alone: the kernel zeroes
    // what it hands out and no more.
    frame::read_bytes(block + 4 * 4096, &mut page).expect("read");
    assert!(
        page.iter().all(|&b| b == 0xa5),
        "memory beyond the objects was touched"
    );
    cs.destroy();
}

pub(super) fn capability_errors_change_nothing() {
    use durg_cap::{Kind, Rights};
    let block = cspace::reserve_untyped(16).expect("block");
    let cs = CSpace::with_untyped(block, 16).expect("space");
    let frames = retype_arg(Kind::Frame as u8, 12, 2);
    assert_eq!(cs.invoke(1, label::RETYPE, [frames, 2, 0]), Ok(0));

    // Wrong kind for the operation.
    assert_eq!(
        cs.invoke(1, derive_label(1, false), [2, 5, 0]),
        Err(SysError::WrongKind),
        "derive is invoked on the CNode, not on an untyped region"
    );
    assert_eq!(
        cs.invoke(0, label::RETYPE, [frames, 5, 0]),
        Err(SysError::WrongKind)
    );
    assert_eq!(
        cs.invoke(2, label::RETYPE, [frames, 5, 0]),
        Err(SysError::WrongKind)
    );
    // Labels: unknown operations, and stray bits, are refused.
    assert_eq!(cs.invoke(0, 99, [0; 3]), Err(SysError::BadArgument));
    assert_eq!(
        cs.invoke(0, label::IDENTIFY | 0x100, [0; 3]),
        Err(SysError::BadArgument)
    );
    assert_eq!(
        cs.invoke(0, label::DELETE | 0x100, [2, 0, 0]),
        Err(SysError::BadArgument)
    );
    assert_eq!(
        cs.invoke(0, derive_label(16, false), [2, 5, 0]),
        Err(SysError::BadArgument),
        "16 is not a rights value"
    );
    // Slot arguments are checked against this task's own space.
    assert_eq!(
        cs.invoke(0, derive_label(1, false), [128, 5, 0]),
        Err(SysError::InvalidSlot)
    );
    assert_eq!(
        cs.invoke(0, derive_label(1, false), [2, 128, 0]),
        Err(SysError::InvalidSlot)
    );
    // A retype whose last slot is out of range creates nothing at all: slot 127
    // stays empty, and the region's memory is untouched (so 14 more frames, all
    // that is left after the first two, still fit).
    assert_eq!(
        cs.invoke(
            1,
            label::RETYPE,
            [retype_arg(Kind::Frame as u8, 12, 2), 127, 0]
        ),
        Err(SysError::InvalidSlot)
    );
    assert_eq!(identify(&cs, 127), Err(SysError::EmptySlot));
    assert_eq!(
        cs.invoke(
            1,
            label::RETYPE,
            [retype_arg(Kind::Frame as u8, 12, 14), 4, 0]
        ),
        Ok(0)
    );
    // The region is now full.
    assert_eq!(
        cs.invoke(
            1,
            label::RETYPE,
            [retype_arg(Kind::Frame as u8, 12, 1), 20, 0]
        ),
        Err(SysError::OutOfMemory)
    );
    assert_eq!(identify(&cs, 20), Err(SysError::EmptySlot));

    // Rights shrink along a derivation and never grow back.
    assert_eq!(cs.invoke(0, derive_label(1, false), [2, 30, 0]), Ok(0));
    assert_eq!(
        identify(&cs, 30),
        Ok(describe(Kind::Frame, Rights::READ, 12))
    );
    assert_eq!(
        cs.invoke(0, derive_label(15, false), [30, 31, 0]),
        Err(SysError::NoRights)
    );
    assert_eq!(
        cs.invoke(0, derive_label(2, false), [30, 31, 0]),
        Err(SysError::NoRights),
        "a read-only capability cannot hand out write"
    );
    assert_eq!(identify(&cs, 31), Err(SysError::EmptySlot));
    // A capability to the space with no write right cannot change it.
    assert_eq!(cs.invoke(0, derive_label(1, false), [0, 40, 0]), Ok(0));
    assert_eq!(
        cs.invoke(40, derive_label(1, false), [2, 41, 0]),
        Err(SysError::NoRights),
        "a read-only capability to the space cannot derive"
    );
    assert_eq!(identify(&cs, 41), Err(SysError::EmptySlot));
    // Badges are set once.
    assert_eq!(
        cs.invoke(1, label::RETYPE, [0, 0, 0]),
        Err(SysError::BadArgument),
        "kind 0 is not an object"
    );
    cs.destroy();
}

pub(super) fn destroying_a_cspace_returns_everything() {
    use durg_cap::Kind;
    let free = user_tests::free_frames();
    let blocks = cspace::reserved_blocks();
    let block = cspace::reserve_untyped(20).expect("a 1 MiB block");
    assert_eq!(
        user_tests::free_frames(),
        free - 256,
        "the block left the pool"
    );
    assert_eq!(cspace::reserved_blocks(), blocks + 1);
    let cs = CSpace::with_untyped(block, 20).expect("space");

    // First two smaller regions (the first starts at the very start of the block,
    // so it has the block's own address and only its smaller size tells them
    // apart), with frames made inside the first; then frames, a copy of one, and
    // endpoints, straight from the block.
    assert_eq!(
        cs.invoke(
            1,
            label::RETYPE,
            [retype_arg(Kind::Untyped as u8, 16, 2), 30, 0]
        ),
        Ok(0)
    );
    assert_eq!(
        cs.invoke(
            30,
            label::RETYPE,
            [retype_arg(Kind::Frame as u8, 12, 4), 40, 0]
        ),
        Ok(0)
    );
    assert_eq!(
        cs.invoke(
            1,
            label::RETYPE,
            [retype_arg(Kind::Frame as u8, 12, 8), 2, 0]
        ),
        Ok(0)
    );
    assert_eq!(cs.invoke(0, derive_label(1, false), [2, 20, 0]), Ok(0));
    assert_eq!(
        cs.invoke(
            1,
            label::RETYPE,
            [retype_arg(Kind::Endpoint as u8, 4, 3), 50, 0]
        ),
        Ok(0)
    );

    // More than 1 MiB of zeroed objects in one call is refused up front (the
    // kernel zeroes while holding the lock), whatever the region could hold.
    assert_eq!(
        cs.invoke(
            1,
            label::RETYPE,
            [retype_arg(Kind::Frame as u8, 19, 3), 60, 0]
        ),
        Err(SysError::BadArgument)
    );
    assert_eq!(identify(&cs, 60), Err(SysError::EmptySlot));

    // Deleting a *child* region returns nothing to the frame allocator: its
    // memory is still the parent's.
    let before = (user_tests::free_frames(), cspace::reserved_blocks());
    // (Slot 30 is the child at the very start of the block.)
    assert_eq!(cs.invoke(0, label::DELETE, [30, 0, 0]), Ok(0));
    assert_eq!(
        (user_tests::free_frames(), cspace::reserved_blocks()),
        before,
        "a child region went back to the frame allocator"
    );

    cs.destroy();
    assert_eq!(user_tests::free_frames(), free, "frames leaked");
    assert_eq!(
        cspace::reserved_blocks(),
        blocks,
        "a block was not returned"
    );
}

pub(super) fn capabilities_work_from_ring_3() {
    let free = user_tests::free_frames();
    let blocks = cspace::reserved_blocks();
    let block = cspace::reserve_untyped(20).expect("a 1 MiB block");
    let cs = CSpace::with_untyped(block, 20).expect("space");
    let handle =
        user::spawn_elf_with_cspace("capdemo", module("capdemo.elf"), cs).expect("spawn capdemo");
    assert_eq!(
        user_tests::wait(&handle),
        user_status::Exited(0),
        "capdemo exits with the number of the first step that went wrong"
    );
    assert_eq!(user_tests::free_frames(), free, "frames leaked");
    assert_eq!(
        cspace::reserved_blocks(),
        blocks,
        "a block was not returned"
    );
}

pub(super) fn a_task_without_a_cspace_can_name_nothing() {
    let handle = user::spawn_module("capdemo", "capdemo.elf").expect("spawn capdemo");
    assert_eq!(
        user_tests::wait(&handle),
        user_status::Exited(100),
        "every invoke must fail with InvalidSlot when the task holds no capability space"
    );
    // The root task, started without its capability space, finds it holds
    // nothing and stops at its first check, instead of doing anything.
    let handle = user::spawn_module("init", "init.elf").expect("spawn init");
    assert_eq!(user_tests::wait(&handle), user_status::Exited(1));
    assert_eq!(user_tests::last_print(), "init: a step failed");
}

pub(super) fn the_root_task_starts_with_its_memory() {
    // The same path the boot uses: it reserves a block, builds the space, runs
    // `init` (which checks what it holds and makes a frame), and must clean up.
    let free = user_tests::free_frames();
    let blocks = cspace::reserved_blocks();
    user::boot_demo().expect("the root task did not exit cleanly");
    // The finished task's resources are released by the next thread to run.
    sched::yield_now();
    assert_eq!(user_tests::free_frames(), free, "frames leaked");
    assert_eq!(
        cspace::reserved_blocks(),
        blocks,
        "a block was not returned"
    );
}

pub(super) fn invoke_survives_random_arguments() {
    use durg_cap::Kind;
    let free = user_tests::free_frames();
    let blocks = cspace::reserved_blocks();
    let block = cspace::reserve_untyped(18).expect("a 256 KiB block");
    let cs = CSpace::with_untyped(block, 18).expect("space");
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let mut ok_by_operation = [0u32; 6];
    let operations = [
        label::IDENTIFY,
        label::RETYPE,
        label::DERIVE,
        label::MOVE,
        label::DELETE,
        label::REVOKE,
    ];
    for step in 0..4_000u32 {
        // Which of the first 24 slots hold something now, and where the CNode
        // capability is (a random move can shift it away from slot 0).
        let mut occupied = alloc::vec::Vec::new();
        let mut empty = alloc::vec::Vec::new();
        let mut cnode_at = 0;
        let mut region_at = 1;
        let mut spare = alloc::vec::Vec::new(); // occupied, except the root region
        for s in 0..24u64 {
            match identify(&cs, s) {
                Ok(v) => {
                    occupied.push(s);
                    let is_root_region = v & 0xff == Kind::Untyped as u64 && (v >> 16) & 0xff == 18;
                    if is_root_region {
                        region_at = s;
                    } else {
                        spare.push(s);
                    }
                    if v & 0xff == Kind::CNode as u64 {
                        cnode_at = s;
                    }
                }
                Err(_) => empty.push(s),
            }
        }
        // Mostly sensible slot choices, now and then a wild one.
        let pick = |rng: &mut Rng, from: &alloc::vec::Vec<u64>| match rng.below(10) {
            0 => rng.next(),
            1 => 120 + rng.below(20),
            _ if from.is_empty() => rng.below(24),
            _ => from[rng.below(from.len() as u64) as usize],
        };
        let op_index = rng.below(operations.len() as u64) as usize;
        let mut op = operations[op_index];
        let mut args = match op {
            // Losing the one root region would end the run's retyping for good, so
            // delete and move leave it alone nine times in ten; revoke may hit it,
            // which is the interesting case (everything carved from it goes).
            label::DERIVE => [
                pick(&mut rng, &occupied),
                pick(&mut rng, &empty),
                rng.next(),
            ],
            label::MOVE => [pick(&mut rng, &spare), pick(&mut rng, &empty), rng.next()],
            label::DELETE => [pick(&mut rng, &spare), 0, 0],
            label::REVOKE => [pick(&mut rng, &occupied), 0, 0],
            _ => [
                pick(&mut rng, &occupied),
                pick(&mut rng, &empty),
                rng.next(),
            ],
        };
        // Derive, move, delete and revoke are invoked on the space's own CNode
        // capability; retype on a region. Either choice is wrong sometimes, on
        // purpose.
        let wanted = if op == label::RETYPE {
            region_at
        } else {
            cnode_at
        };
        let cptr = match rng.below(10) {
            0 => pick(&mut rng, &occupied),
            1 => rng.below(2),
            _ => wanted,
        };
        match op {
            label::RETYPE => {
                let kind = [
                    Kind::Untyped,
                    Kind::Frame,
                    Kind::Endpoint,
                    Kind::Notification,
                ][rng.below(4) as usize] as u8;
                let bits = 4 + rng.below(12) as u8;
                let count = 1 + rng.below(4) as u8;
                args = [retype_arg(kind, bits, count), pick(&mut rng, &empty), 0];
            }
            label::DERIVE => {
                // A badge only makes sense on an endpoint or notification, so ask
                // for one a third of the time and let it be refused elsewhere.
                op = derive_label(rng.below(16) as u8, rng.below(3) == 0);
            }
            _ => {}
        }
        // Occasionally garble the label, as a hostile program would.
        if rng.below(50) == 0 {
            op ^= rng.next();
        }
        let result = cs.invoke(cptr, op, args);
        if let Err(error) = result {
            assert!(
                SysError::from_return_value(error.to_return_value()).is_some(),
                "an undocumented error"
            );
        } else {
            ok_by_operation[op_index] += 1;
        }
        // Every few steps (the check is thorough and the emulator is slow).
        if step % 4 == 0 {
            if let Err(rule) = cs.check_invariants() {
                panic!("capability invariant broken after step {step}: {rule}");
            }
        }
    }
    if let Err(rule) = cs.check_invariants() {
        panic!("capability invariant broken at the end: {rule}");
    }
    // The run has to have done real work, or it proves nothing.
    for (index, &n) in ok_by_operation.iter().enumerate() {
        assert!(n > 10, "operation {index} succeeded only {n} times");
    }
    cs.destroy();
    assert_eq!(user_tests::free_frames(), free, "frames leaked");
    assert_eq!(
        cspace::reserved_blocks(),
        blocks,
        "a block was not returned"
    );
}
