//! The root task: the first program the kernel starts, and the only one that
//! starts with any authority. The kernel gives it its own capability space (slot
//! 0), 4 MiB of untyped memory (slot 1) and the authority to start tasks (slot 2),
//! and nothing else. Everything any other task can do, it can do because this
//! task handed it a capability, so this is where the system's authority is
//! decided.
//!
//! Here it builds a small system and checks it: it makes an endpoint and a frame
//! out of its memory, derives a receive-only copy for a server and a badged,
//! send-only copy (with the right to pass capabilities on) for a client, starts
//! the server, the client and a stranger that is given nothing, and waits for all
//! three. The exit code is the number of the first step that went wrong.

#![no_std]
#![no_main]

use durg_cap::{Kind, Rights};
use durg_syscall::{derive_label, label, module, retype_arg, Error};
use durg_user::{error_of, exit, invoke, join, print, spawn};

const OWN_SPACE: u64 = 0;
const REGION: u64 = 1;
const SPAWNER: u64 = 2;
const ENDPOINT: u64 = 3;
const FRAME: u64 = 4;
const FOR_SERVER: u64 = 5;
const FOR_CLIENT: u64 = 6;
const FRAME_FOR_CLIENT: u64 = 7;

fn describe(kind: Kind, bits: u8) -> u64 {
    u64::from(kind as u8) | u64::from(Rights::ALL.bits()) << 8 | u64::from(bits) << 16
}

fn check(step: u64, ok: bool) {
    if !ok {
        print("init: a step failed");
        exit(step)
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    print("init: root task running in ring 3");

    // What the kernel gave this task, and nothing more.
    check(
        1,
        invoke(OWN_SPACE, label::IDENTIFY, 0, 0, 0) == describe(Kind::CNode, 12),
    );
    check(
        2,
        invoke(REGION, label::IDENTIFY, 0, 0, 0) == describe(Kind::Untyped, 22),
    );
    check(
        3,
        invoke(SPAWNER, label::IDENTIFY, 0, 0, 0) == describe(Kind::Spawner, 0),
    );
    check(
        4,
        error_of(invoke(ENDPOINT, label::IDENTIFY, 0, 0, 0)) == Some(Error::EmptySlot),
    );

    // Make an endpoint and a frame out of the region.
    let make = |kind: Kind, bits: u8, slot: u64| {
        invoke(
            REGION,
            label::RETYPE,
            retype_arg(kind as u8, bits, 1),
            slot,
            0,
        )
    };
    check(5, make(Kind::Endpoint, 4, ENDPOINT) == 0);
    check(6, make(Kind::Frame, 12, FRAME) == 0);

    // Hand out restricted copies. The server may only receive. The client may only
    // send, may pass capabilities on, and is badged 77 in a way it cannot change.
    let derive = |src: u64, dst: u64, rights: Rights, badge: Option<u64>| {
        invoke(
            OWN_SPACE,
            derive_label(rights.bits(), badge.is_some()),
            src,
            dst,
            badge.unwrap_or(0),
        )
    };
    let write_grant = Rights::from_bits_truncate(Rights::WRITE.bits() | Rights::GRANT.bits());
    check(7, derive(ENDPOINT, FOR_SERVER, Rights::READ, None) == 0);
    check(8, derive(ENDPOINT, FOR_CLIENT, write_grant, Some(77)) == 0);
    check(9, derive(FRAME, FRAME_FOR_CLIENT, Rights::ALL, None) == 0);

    // Spawning has rules: it needs a spawner capability, a real module, and
    // capabilities that can be copied (not memory regions).
    check(
        10,
        error_of(spawn(REGION, module::IPC_SERVER, FOR_SERVER, 1)) == Some(Error::WrongKind),
    );
    check(
        11,
        error_of(spawn(SPAWNER, 99, FOR_SERVER, 1)) == Some(Error::BadArgument),
    );
    check(
        12,
        error_of(spawn(SPAWNER, module::IPC_SERVER, REGION, 1)) == Some(Error::BadArgument),
    );
    check(
        13,
        error_of(spawn(SPAWNER, module::IPC_SERVER, 9, 1)) == Some(Error::BadArgument),
    );
    check(
        14,
        error_of(spawn(SPAWNER, module::IPC_SERVER, 200, 1)) == Some(Error::InvalidSlot),
    );
    check(
        15,
        error_of(join(SPAWNER, 999_999)) == Some(Error::NoSuchTask),
    );

    // Start the system. The server gets the receive-only endpoint (its slot 2); the
    // client the badged send endpoint and the frame (its slots 2 and 3); the
    // stranger gets nothing.
    let server = spawn(SPAWNER, module::IPC_SERVER, FOR_SERVER, 1);
    let client = spawn(SPAWNER, module::IPC_CLIENT, FOR_CLIENT, 2);
    let stranger = spawn(SPAWNER, module::IPC_STRANGER, 0, 0);
    check(16, error_of(server).is_none());
    check(17, error_of(client).is_none());
    check(18, error_of(stranger).is_none());

    // Each one's exit code is the number of its first failed step, 0 if none: the
    // client checked the replies, the badge and the capability it sent; the
    // stranger tried every slot and reached nobody; the server counted exactly the
    // client's three messages.
    check(19, join(SPAWNER, client) == 0);
    check(20, join(SPAWNER, stranger) == 0);
    check(21, join(SPAWNER, server) == 0);
    // A finished task can still be asked about.
    check(22, join(SPAWNER, client) == 0);

    print("init: built the system: server, client and stranger ran; the stranger reached nobody");
    exit(0)
}
