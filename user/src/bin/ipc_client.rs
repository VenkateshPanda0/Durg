//! A client for the message tests. Slot 2 holds a send capability (badged by the
//! test) for the server's endpoint, slot 3 a frame capability it may offer, slot 4
//! is empty. What it expects depends on whether its endpoint capability has the
//! grant right (bit 2 of the rights): with it, a capability sent in a message
//! arrives; without it, sending one is refused. The exit code is the number of the
//! first step that went wrong, 0 if none.

#![no_std]
#![no_main]

use durg_cap::{Kind, Rights};
use durg_syscall::{label, Error};
use durg_user::{call, error_of, exit, invoke, message_label, recv, send};

const ENDPOINT: u64 = 2;
const FRAME: u64 = 3;
const EMPTY: u64 = 4;

fn check(step: u64, ok: bool) {
    if !ok {
        exit(step)
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let ep = invoke(ENDPOINT, label::IDENTIFY, 0, 0, 0);
    check(1, ep & 0xff == Kind::Endpoint as u64);
    let rights = ((ep >> 8) & 0xff) as u8;
    let can_grant = rights & Rights::GRANT.bits() != 0;

    // A plain call: label + 1, first word doubled, and the badge the test chose.
    let Ok(r) = call(ENDPOINT, message_label(5, None), [10, 0, 0]) else {
        exit(2)
    };
    check(3, r.label == 6 && r.words[0] == 20);
    check(4, r.words[1] == 77);
    check(5, r.words[2] == 0 && !r.got_cap);

    // A call that carries the frame capability.
    let carrying = call(ENDPOINT, message_label(8, Some(FRAME as u16)), [1, 0, 0]);
    if can_grant {
        let Ok(r) = carrying else { exit(6) };
        // The server describes what it got (as `identify` does): a 4 KiB Frame, with
        // all rights.
        let expected = Kind::Frame as u64 | u64::from(Rights::ALL.bits()) << 8 | 12 << 16;
        check(7, r.label == 9 && r.words[2] == expected);
    } else {
        check(8, carrying.err() == Some(Error::NoRights));
    }

    // Refusals: an empty slot, a slot out of range, a capability of the wrong
    // kind, a slot to send that is empty.
    check(
        9,
        error_of(send(EMPTY, message_label(1, None), [0; 3])) == Some(Error::EmptySlot),
    );
    check(
        10,
        error_of(send(500, message_label(1, None), [0; 3])) == Some(Error::InvalidSlot),
    );
    check(
        11,
        error_of(send(FRAME, message_label(1, None), [0; 3])) == Some(Error::WrongKind),
    );
    if can_grant {
        check(
            12,
            call(ENDPOINT, message_label(1, Some(EMPTY as u16)), [0; 3]).err()
                == Some(Error::EmptySlot),
        );
    }
    // Offering this task's own capability space is accepted as a message but the
    // capability is not delivered: a capability space stays with its owner.
    if can_grant {
        let Ok(r) = call(ENDPOINT, message_label(2, Some(0)), [0; 3]) else {
            exit(17)
        };
        check(18, r.label == 3 && r.words[2] == 0);
    }
    // This capability can send but not receive.
    check(13, recv(ENDPOINT, None).err() == Some(Error::NoRights));
    // A label with stray bits is refused rather than masked.
    check(
        14,
        error_of(send(ENDPOINT, 1 << 50, [0; 3])) == Some(Error::BadArgument),
    );

    // Ask the server to stop; it says how many messages it served: the plain call,
    // and (if granting was allowed) the one with a frame and the one offering the
    // task's own space.
    let Ok(r) = call(ENDPOINT, message_label(0xdead, None), [0; 3]) else {
        exit(15)
    };
    let served = if can_grant { 3 } else { 1 };
    check(16, r.label == 0xdead && r.words[0] == served);
    exit(0)
}
