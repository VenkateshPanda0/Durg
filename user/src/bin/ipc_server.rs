//! A server for the message tests. It holds a receive capability for an endpoint
//! in slot 2 and answers every call: label + 1, first word doubled, the badge of
//! the capability the caller used, and a description of any capability that came
//! with the message (it asks for it in slot 5). A message with label 0xdead ends
//! the loop; the reply says how many messages it served.

#![no_std]
#![no_main]

use durg_syscall::{label, Error};
use durg_user::{error_of, exit, invoke, message_label, recv, reply, send};

const ENDPOINT: u64 = 2;
const RECEIVED_CAP: u64 = 5;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut served: u64 = 0;
    // This capability can receive but not send.
    if error_of(send(ENDPOINT, message_label(1, None), [0; 3])) != Some(Error::NoRights) {
        exit(102)
    }
    loop {
        let Ok(message) = recv(ENDPOINT, Some(RECEIVED_CAP)) else {
            exit(101)
        };
        if message.label == 0xdead {
            reply(0xdead, [served, 0, 0]);
            exit(0)
        }
        served += 1;
        // What arrived with the message, if anything, as `identify` describes it.
        let arrived = if message.got_cap {
            invoke(RECEIVED_CAP, label::IDENTIFY, 0, 0, 0)
        } else {
            0
        };
        reply(
            message.label + 1,
            [message.words[0] * 2, message.badge, arrived],
        );
        // Whatever arrived is not kept: delete it so the next message starts clean.
        if message.got_cap {
            let _ = invoke(0, label::DELETE, RECEIVED_CAP, 0, 0);
        }
    }
}
