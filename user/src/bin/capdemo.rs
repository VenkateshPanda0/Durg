//! Exercises the capability system from ring 3. The kernel's self-test gives this
//! task a capability space with its own CNode in slot 0 and a 1 MiB untyped
//! region in slot 1, runs it, and expects exit code 0. The exit code is the
//! number of the first step that did not behave as expected, so a failure says
//! where. If the task has no capability space at all, it exits with 100: that
//! task can name nothing.

#![no_std]
#![no_main]

use durg_cap::{Kind, Rights};
use durg_syscall::{derive_label, label, retype_arg, Error};
use durg_user::{error_of, exit, invoke, print};

const CNODE: u64 = 0;
const UNTYPED: u64 = 1;

fn fail(step: u64) -> ! {
    print("capdemo: a step failed");
    exit(step)
}

fn check(step: u64, condition: bool) {
    if !condition {
        fail(step)
    }
}

fn identify(cptr: u64) -> u64 {
    invoke(cptr, label::IDENTIFY, 0, 0, 0)
}

fn describe(kind: Kind, rights: Rights, bits: u8) -> u64 {
    u64::from(kind as u8) | u64::from(rights.bits()) << 8 | u64::from(bits) << 16
}

fn retype(untyped: u64, kind: Kind, bits: u8, count: u8, first_slot: u64) -> u64 {
    invoke(
        untyped,
        label::RETYPE,
        retype_arg(kind as u8, bits, count),
        first_slot,
        0,
    )
}

fn derive(src: u64, dst: u64, rights: Rights, badge: Option<u64>) -> u64 {
    invoke(
        CNODE,
        derive_label(rights.bits(), badge.is_some()),
        src,
        dst,
        badge.unwrap_or(0),
    )
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // No capability space, nothing to name.
    if error_of(identify(CNODE)) == Some(Error::InvalidSlot) {
        print("capdemo: no capability space");
        exit(100)
    }

    // 1-4: what the task was given, and what it was not.
    check(1, identify(CNODE) == describe(Kind::CNode, Rights::ALL, 12));
    check(
        2,
        identify(UNTYPED) == describe(Kind::Untyped, Rights::ALL, 20),
    );
    check(3, error_of(identify(2)) == Some(Error::EmptySlot));
    check(4, error_of(identify(128)) == Some(Error::InvalidSlot));
    check(5, error_of(identify(u64::MAX)) == Some(Error::InvalidSlot));

    // 6-8: carve three frames out of the region, into slots 2, 3 and 4.
    check(6, retype(UNTYPED, Kind::Frame, 12, 3, 2) == 0);
    check(7, identify(3) == describe(Kind::Frame, Rights::ALL, 12));
    check(
        8,
        error_of(retype(UNTYPED, Kind::Frame, 12, 1, 3)) == Some(Error::SlotOccupied),
    );
    check(9, error_of(identify(5)) == Some(Error::EmptySlot));

    // 10-12: copies get fewer rights, never more.
    check(10, derive(2, 5, Rights::READ, None) == 0);
    check(11, identify(5) == describe(Kind::Frame, Rights::READ, 12));
    check(
        12,
        error_of(derive(5, 6, Rights::ALL, None)) == Some(Error::NoRights),
    );
    check(13, error_of(identify(6)) == Some(Error::EmptySlot));

    // 14-17: an operation needs the right kind of capability.
    check(
        14,
        error_of(invoke(2, derive_label(1, false), 2, 6, 0)) == Some(Error::WrongKind),
    );
    check(
        15,
        error_of(retype(CNODE, Kind::Frame, 12, 1, 6)) == Some(Error::WrongKind),
    );
    check(
        16,
        error_of(retype(2, Kind::Frame, 12, 1, 6)) == Some(Error::WrongKind),
    );
    check(
        17,
        error_of(invoke(CNODE, 99, 0, 0, 0)) == Some(Error::BadArgument),
    );

    // 18-20: refused requests leave nothing behind.
    check(
        18,
        error_of(retype(UNTYPED, Kind::CNode, 12, 1, 6)) == Some(Error::BadArgument),
    );
    check(
        19,
        error_of(retype(UNTYPED, Kind::Frame, 12, 0, 6)) == Some(Error::BadArgument),
    );
    check(
        20,
        error_of(retype(UNTYPED, Kind::Frame, 19, 2, 6)) == Some(Error::OutOfMemory),
    );
    check(21, error_of(identify(6)) == Some(Error::EmptySlot));
    check(22, error_of(identify(7)) == Some(Error::EmptySlot));

    // 23-26: revoking the region removes everything carved from it, copies too.
    check(23, invoke(CNODE, label::REVOKE, UNTYPED, 0, 0) == 4);
    check(24, error_of(identify(2)) == Some(Error::EmptySlot));
    check(25, error_of(identify(5)) == Some(Error::EmptySlot));
    check(
        26,
        identify(UNTYPED) == describe(Kind::Untyped, Rights::ALL, 20),
    );

    // 27-29: the memory is usable again.
    check(27, retype(UNTYPED, Kind::Frame, 12, 1, 2) == 0);
    check(28, invoke(CNODE, label::DELETE, 2, 0, 0) == 0);
    check(
        29,
        error_of(invoke(CNODE, label::DELETE, 2, 0, 0)) == Some(Error::EmptySlot),
    );

    // 30-34: a task cannot delete the last capability to its own space.
    check(
        30,
        error_of(invoke(CNODE, label::DELETE, CNODE, 0, 0)) == Some(Error::BadArgument),
    );
    check(31, derive(CNODE, 7, Rights::ALL, None) == 0);
    check(32, invoke(CNODE, label::DELETE, CNODE, 0, 0) == 0);
    check(
        33,
        error_of(invoke(7, label::DELETE, 7, 0, 0)) == Some(Error::BadArgument),
    );
    check(34, invoke(7, label::MOVE, 7, 0, 0) == 0);
    check(
        35,
        identify(CNODE) == describe(Kind::CNode, Rights::ALL, 12),
    );

    // 36-39: badges are set once.
    check(36, retype(UNTYPED, Kind::Endpoint, 4, 1, 2) == 0);
    check(37, derive(2, 3, Rights::ALL, Some(42)) == 0);
    check(
        38,
        error_of(derive(3, 4, Rights::ALL, Some(7))) == Some(Error::BadArgument),
    );
    check(39, derive(3, 4, Rights::READ, None) == 0);

    print("capdemo: all capability steps behaved");
    exit(0)
}
