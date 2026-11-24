//! Messages between threads and ring-3 tasks, notifications and join (M10).
//!
//! Part of the in-kernel self-tests; see [`super`] for how they are run.

use super::*;

// ---- messages between threads (M10) -----------------------------------------
//
// These test the scheduler's message passing with kernel threads and a made-up
// endpoint address (the layer below knows endpoints only by address). The
// capability checks on top of it are tested with ring-3 programs further down.

pub(super) mod ipc_threads {
    use crate::sched::{self, IpcError, Message};
    use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering::SeqCst};

    pub const EP: u64 = 0xdead_0000;
    pub const OTHER_EP: u64 = 0xdead_1000;

    pub static LABEL: AtomicU64 = AtomicU64::new(0);
    pub static WORD: AtomicU64 = AtomicU64::new(0);
    pub static BADGE: AtomicU64 = AtomicU64::new(0);
    pub static ERROR: AtomicU64 = AtomicU64::new(0);
    pub static SERVED: AtomicUsize = AtomicUsize::new(0);
    pub static ORDER: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    pub static NEXT: AtomicUsize = AtomicUsize::new(0);

    pub fn reset() {
        for a in [&LABEL, &WORD, &BADGE, &ERROR] {
            a.store(0, SeqCst);
        }
        SERVED.store(0, SeqCst);
        NEXT.store(0, SeqCst);
        for a in &ORDER {
            a.store(0, SeqCst);
        }
    }

    pub fn wait_until_blocked(n: usize) {
        for _ in 0..100_000 {
            if sched::blocked_count() >= n {
                return;
            }
            sched::yield_now();
        }
        panic!("{n} threads never blocked");
    }

    pub fn message(label: u32, w0: u64) -> Message {
        Message {
            label,
            words: [w0, w0 + 1, w0 + 2],
            cap_slot: None,
        }
    }

    /// Receives one message, immediately.
    pub fn receive_once() {
        let r = sched::ipc_recv(EP, None).expect("receive");
        LABEL.store(u64::from(r.message.label), SeqCst);
        WORD.store(r.message.words[0], SeqCst);
        BADGE.store(r.badge, SeqCst);
        SERVED.fetch_add(1, SeqCst);
    }

    /// Waits until someone is blocked (as a sender), then receives.
    pub fn receive_once_late() {
        wait_until_blocked(1);
        receive_once();
    }

    /// Receives one message and records which waiter it was.
    pub fn receive_and_record_order() {
        let me = NEXT.fetch_add(1, SeqCst);
        let r = sched::ipc_recv(EP, None).expect("receive");
        ORDER[me].store(u64::from(r.message.label), SeqCst);
    }

    /// Echo server: replies with the label plus one and every word doubled.
    pub fn echo_once() {
        let r = sched::ipc_recv(EP, None).expect("receive");
        BADGE.store(r.badge, SeqCst);
        sched::ipc_reply(Message {
            label: r.message.label + 1,
            words: r.message.words.map(|w| w * 2),
            cap_slot: None,
        })
        .expect("reply");
    }

    /// Takes a call and exits without answering it.
    pub fn take_call_and_die() {
        let _ = sched::ipc_recv(EP, None).expect("receive");
    }

    /// Waits for a message and records whether it was woken with an error.
    pub fn receive_expecting_abort() {
        let aborted = matches!(sched::ipc_recv(EP, None), Err(IpcError::Aborted));
        ERROR.store(if aborted { 1 } else { 100 }, SeqCst);
    }
}

pub(super) mod notification_threads {
    use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering::SeqCst};
    pub static OBJECT: AtomicU64 = AtomicU64::new(0);
    pub static NEXT: AtomicUsize = AtomicUsize::new(0);
    pub static ORDER: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    pub static ABORTED: AtomicU64 = AtomicU64::new(0);
    pub fn distinct_waiter() {
        let index = NEXT.fetch_add(1, SeqCst);
        let object = OBJECT.load(SeqCst) + index as u64 * 16;
        match crate::sched::notification_wait(object) {
            Ok(badge) => ORDER[index].store(badge, SeqCst),
            Err(durg_syscall::Error::Aborted) => {
                ABORTED.fetch_add(1, SeqCst);
            }
            other => panic!("unexpected notification result: {:?}", other),
        }
    }
    pub fn waiter() {
        let index = NEXT.fetch_add(1, SeqCst);
        let result = crate::sched::notification_wait(OBJECT.load(SeqCst));
        match result {
            Ok(badge) => ORDER[index].store(badge, SeqCst),
            Err(durg_syscall::Error::Aborted) => {
                ABORTED.fetch_add(1, SeqCst);
            }
            other => panic!("unexpected notification result: {:?}", other),
        }
    }
}

pub(super) fn notification_ring3_blocked_delivery_and_revocation() {
    use durg_cap::{Kind, Rights};
    sched::yield_now();
    let free = user_tests::free_frames();
    let blocks = cspace::reserved_blocks();
    for mode in 0..3 {
        let block = cspace::reserve_untyped(16).expect("untyped");
        let root = CSpace::with_untyped(block, 16).expect("root space");
        assert_eq!(
            root.invoke(
                1,
                label::RETYPE,
                [retype_arg(Kind::Notification as u8, 4, 1), 2, 0]
            ),
            Ok(0)
        );
        assert_eq!(
            root.invoke(
                0,
                durg_syscall::derive_label(Rights::WRITE.bits(), true),
                [2, 3, 1 << 63]
            ),
            Ok(0)
        );
        let child = CSpace::new().expect("child space");
        assert!(root.grant(2, &child, 2, Rights::READ, None));
        let waiter = user::spawn_elf_with_cspace(
            "notification-waiter",
            module("notification_waiter.elf"),
            child,
        )
        .expect("spawn waiter");
        ipc_threads::wait_until_blocked(1);
        let expected = match mode {
            0 => {
                let (object, badge) = root
                    .notification(3, Rights::WRITE)
                    .expect("signal capability");
                sched::notification_signal(object, badge).expect("signal");
                user_status::Exited(0)
            }
            1 => {
                root.invoke(0, label::REVOKE, [1, 0, 0]).expect("revoke");
                user_status::Exited(1)
            }
            _ => {
                waiter.kill();
                user_status::Killed
            }
        };
        assert_eq!(user_tests::wait(&waiter), expected);
        root.destroy();
        sched::yield_now();
        assert_eq!(
            user_tests::free_frames(),
            free,
            "notification scenario leaked frames"
        );
        assert_eq!(
            cspace::reserved_blocks(),
            blocks,
            "notification scenario leaked blocks"
        );
        assert_eq!(
            sched::blocked_count(),
            0,
            "notification waiter was not reclaimed"
        );
    }
}

pub(super) fn notifications_coalesce_wake_fifo_and_abort() {
    use core::sync::atomic::Ordering::SeqCst;
    use notification_threads::*;
    let object = frame::alloc_frame().expect("notification frame");
    frame::write_bytes(object, &[0; durg_cap::NOTIFICATION_BYTES]).expect("zero notification");
    OBJECT.store(object, SeqCst);
    NEXT.store(0, SeqCst);
    ABORTED.store(0, SeqCst);
    for slot in &ORDER {
        slot.store(0, SeqCst);
    }
    sched::notification_signal(object, 1 << 63).expect("signal");
    sched::notification_signal(object, 5).expect("signal");
    assert_eq!(sched::notification_wait(object), Ok((1 << 63) | 5));
    sched::notification_signal(object, 0).expect("zero signal");
    assert_eq!(sched::notification_wait(object), Ok(0));
    let mut handles = alloc::vec::Vec::new();
    for n in 1..=3 {
        handles.push(sched::spawn("notification-waiter", waiter).expect("spawn"));
        ipc_threads::wait_until_blocked(n);
    }
    for badge in 1..=3 {
        sched::notification_signal(object, badge).expect("wake");
    }
    for h in handles {
        h.join();
    }
    assert_eq!(ORDER.each_ref().map(|v| v.load(SeqCst)), [1, 2, 3]);
    NEXT.store(0, SeqCst);
    let aborted_waiter = sched::spawn("notification-abort", waiter).expect("spawn");
    ipc_threads::wait_until_blocked(1);
    sched::ipc_abort_range(object, object);
    aborted_waiter.join();
    assert_eq!(ABORTED.load(SeqCst), 1);
    // Destroying two objects must not abort a live waiter between their addresses.
    frame::write_bytes(object, &[0; 48]).expect("reset notifications");
    NEXT.store(0, SeqCst);
    ABORTED.store(0, SeqCst);
    let mut handles = alloc::vec::Vec::new();
    for n in 1..=3 {
        handles.push(sched::spawn("distinct-notification", distinct_waiter).expect("spawn"));
        ipc_threads::wait_until_blocked(n);
    }
    cspace::test_destroyed_notifications(&[object, object + 32]);
    assert_eq!(
        sched::blocked_count(),
        1,
        "unrelated notification was aborted"
    );
    sched::notification_signal(object + 16, 42).expect("signal surviving waiter");
    for h in handles {
        h.join();
    }
    assert_eq!(ABORTED.load(SeqCst), 2);
    assert_eq!(ORDER[1].load(SeqCst), 42);
    frame::free_frame(object).expect("free notification frame");

    // Revoke a real notification capability while a thread is blocked on it.
    let block = cspace::reserve_untyped(16).expect("untyped");
    let root = CSpace::with_untyped(block, 16).expect("space");
    assert_eq!(
        root.invoke(
            1,
            label::RETYPE,
            [retype_arg(durg_cap::Kind::Notification as u8, 4, 1), 2, 0]
        ),
        Ok(0)
    );
    let (object, _) = root
        .notification(2, durg_cap::Rights::READ)
        .expect("notification");
    OBJECT.store(object, SeqCst);
    NEXT.store(0, SeqCst);
    ABORTED.store(0, SeqCst);
    let waiter = sched::spawn("notification-revoke", waiter).expect("spawn");
    ipc_threads::wait_until_blocked(1);
    root.invoke(0, label::REVOKE, [1, 0, 0]).expect("revoke");
    waiter.join();
    assert_eq!(ABORTED.load(SeqCst), 1, "revocation must wake the waiter");
    root.destroy();
}

pub(super) fn ipc_rendezvous_in_both_orders() {
    use core::sync::atomic::Ordering::SeqCst;
    use ipc_threads::*;
    reset();
    // The sender gets there first: it blocks until the receiver arrives.
    let receiver = sched::spawn("receiver-late", receive_once_late).expect("spawn");
    assert_eq!(sched::ipc_send(EP, 7, message(11, 100), false), Ok(None));
    receiver.join();
    assert_eq!(LABEL.load(SeqCst), 11);
    assert_eq!(WORD.load(SeqCst), 100);
    assert_eq!(BADGE.load(SeqCst), 7, "the badge of the capability used");
    assert_eq!(sched::blocked_count(), 0);

    // The receiver gets there first: it blocks until the sender arrives.
    reset();
    let threads = sched::thread_count();
    let receiver = sched::spawn("receiver", receive_once).expect("spawn");
    wait_until_blocked(1);
    // A thread waiting for a message still exists, and is counted.
    assert_eq!(sched::thread_count(), threads + 1);
    assert_eq!(sched::ipc_send(EP, 9, message(22, 200), false), Ok(None));
    receiver.join();
    assert_eq!(
        (LABEL.load(SeqCst), WORD.load(SeqCst), BADGE.load(SeqCst)),
        (22, 200, 9)
    );
    assert_eq!(sched::blocked_count(), 0);

    // A message on one endpoint is not seen on another: a receiver of OTHER_EP
    // stays blocked while a sender on EP waits, until both are released.
    reset();
    let waiting = sched::spawn("other-ep", || {
        let _ = sched::ipc_recv(ipc_threads::OTHER_EP, None);
    })
    .expect("spawn");
    wait_until_blocked(1);
    let helper = sched::spawn("helper", receive_once_late).expect("spawn");
    // Our send on EP must pair with `helper`, not with the OTHER_EP receiver.
    assert_eq!(sched::ipc_send(EP, 1, message(33, 300), false), Ok(None));
    helper.join();
    assert_eq!(LABEL.load(SeqCst), 33);
    assert_eq!(
        sched::blocked_count(),
        1,
        "the other endpoint's receiver is still waiting"
    );
    sched::ipc_abort_range(OTHER_EP, OTHER_EP);
    waiting.join();
}

pub(super) fn ipc_waiters_are_served_in_order() {
    use core::sync::atomic::Ordering::SeqCst;
    use ipc_threads::*;
    reset();
    // Three receivers block one after another; three sends must reach them in the
    // same order.
    let mut handles = alloc::vec::Vec::new();
    for n in 1..=3 {
        handles.push(sched::spawn("fifo-receiver", receive_and_record_order).expect("spawn"));
        wait_until_blocked(n);
    }
    for n in 1..=3u32 {
        assert_eq!(sched::ipc_send(EP, 0, message(n * 10, 0), false), Ok(None));
    }
    for h in handles {
        h.join();
    }
    let got: alloc::vec::Vec<u64> = ORDER.iter().map(|a| a.load(SeqCst)).collect();
    assert_eq!(
        got,
        [10, 20, 30],
        "receivers were not served in the order they waited"
    );
}

pub(super) fn ipc_call_and_reply() {
    use core::sync::atomic::Ordering::SeqCst;
    use ipc_threads::*;
    reset();
    let server = sched::spawn("echo", echo_once).expect("spawn");
    wait_until_blocked(1);
    let reply = sched::ipc_send(EP, 42, message(5, 10), true)
        .expect("call")
        .expect("a call returns the reply");
    assert_eq!(reply.message.label, 6);
    assert_eq!(reply.message.words, [20, 22, 24]);
    assert_eq!(BADGE.load(SeqCst), 42, "the server saw the caller's badge");
    server.join();

    // A call made before the server is listening waits for it, then gets its reply.
    reset();
    let server = sched::spawn("echo-late", || {
        ipc_threads::wait_until_blocked(1);
        ipc_threads::echo_once();
    })
    .expect("spawn");
    let reply = sched::ipc_send(EP, 0, message(100, 1), true)
        .expect("call")
        .expect("reply");
    assert_eq!((reply.message.label, reply.message.words), (101, [2, 4, 6]));
    server.join();
    assert_eq!(sched::blocked_count(), 0);
}

pub(super) fn ipc_destroyed_endpoint_wakes_waiters() {
    use core::sync::atomic::Ordering::SeqCst;
    use ipc_threads::*;
    reset();
    let waiter = sched::spawn("abortee", receive_expecting_abort).expect("spawn");
    wait_until_blocked(1);
    // Destroying an endpoint outside the range does nothing to this waiter.
    sched::ipc_abort_range(EP + 1, EP + 100);
    assert_eq!(sched::blocked_count(), 1);
    sched::ipc_abort_range(EP, EP);
    waiter.join();
    assert_eq!(
        ERROR.load(SeqCst),
        1,
        "the receiver was not told the endpoint is gone"
    );

    // A blocked *sender* is woken with the error too.
    reset();
    let sender = sched::spawn("abort-sender", || {
        let r = sched::ipc_send(ipc_threads::EP, 0, ipc_threads::message(1, 1), false);
        ipc_threads::ERROR.store(
            if r == Err(sched::IpcError::Aborted) {
                1
            } else {
                100
            },
            SeqCst,
        );
    })
    .expect("spawn");
    wait_until_blocked(1);
    sched::ipc_abort_range(0, u64::MAX);
    sender.join();
    assert_eq!(ERROR.load(SeqCst), 1);
}

pub(super) fn ipc_caller_woken_when_server_exits() {
    use ipc_threads::*;
    reset();
    let server = sched::spawn("mute", take_call_and_die).expect("spawn");
    wait_until_blocked(1);
    let result = sched::ipc_send(EP, 0, message(1, 1), true);
    assert_eq!(
        result,
        Err(sched::IpcError::Aborted),
        "a caller must not wait forever"
    );
    server.join();
    assert_eq!(sched::blocked_count(), 0);
}

pub(super) fn ipc_reply_without_a_caller() {
    assert_eq!(
        sched::ipc_reply(ipc_threads::message(1, 1)),
        Err(sched::IpcError::NoReplyTarget)
    );
}

// ---- messages between ring-3 tasks, through capabilities (M10) ------------------

/// The capability spaces of one scenario. The kernel plays the root task: it makes
/// an endpoint and a frame out of an untyped region and hands out restricted,
/// badged copies, which is all the authority anyone else has.
pub(super) struct Scenario {
    root: CSpace,
    server: CSpace,
    client: CSpace,
    free: u64,
    blocks: usize,
}

impl Scenario {
    fn new(client_rights: durg_cap::Rights) -> Scenario {
        use durg_cap::{Kind, Rights};
        // A thread that finished just before is reaped by the next one to run;
        // let that happen first, or its stack would be counted as freed by this test.
        sched::yield_now();
        let free = user_tests::free_frames();
        let blocks = cspace::reserved_blocks();
        let block = cspace::reserve_untyped(16).expect("a 64 KiB block");
        let root = CSpace::with_untyped(block, 16).expect("root space");
        let make = |kind: Kind, bits: u8, slot: u64| {
            assert_eq!(
                root.invoke(1, label::RETYPE, [retype_arg(kind as u8, bits, 1), slot, 0]),
                Ok(0)
            );
        };
        make(Kind::Endpoint, 4, 2);
        make(Kind::Frame, 12, 3);
        let server = CSpace::new().expect("server space");
        let client = CSpace::new().expect("client space");
        // The server may only receive; the client only send (and perhaps grant), under
        // a badge the server can see and the client cannot change.
        assert!(root.grant(2, &server, 2, Rights::READ, None));
        assert!(root.grant(2, &client, 2, client_rights, Some(77)));
        assert!(root.grant(3, &client, 3, Rights::ALL, None));
        Scenario {
            root,
            server,
            client,
            free,
            blocks,
        }
    }

    /// Destroys the root space and checks that everything came back. The server and
    /// client spaces belong to their tasks and were released when those finished.
    fn finish(self) {
        self.root.destroy();
        // The finished tasks' spaces are released by the next thread to run.
        sched::yield_now();
        assert_eq!(user_tests::free_frames(), self.free, "frames leaked");
        assert_eq!(
            cspace::reserved_blocks(),
            self.blocks,
            "a block was not returned"
        );
        assert_eq!(sched::blocked_count(), 0, "a thread is still waiting");
    }
}

pub(super) fn run_server_and_client(client_rights: durg_cap::Rights) {
    let s = Scenario::new(client_rights);
    let server = user::spawn_elf_with_cspace("server", module("ipc_server.elf"), s.server)
        .expect("spawn server");
    let client = user::spawn_elf_with_cspace("client", module("ipc_client.elf"), s.client)
        .expect("spawn client");
    assert_eq!(
        user_tests::wait(&client),
        user_status::Exited(0),
        "the client exits with the number of the first step that went wrong"
    );
    assert_eq!(user_tests::wait(&server), user_status::Exited(0));
    s.finish();
}

pub(super) fn ipc_tasks_with_grant() {
    use core::sync::atomic::Ordering;
    use durg_cap::Rights;
    assert!(
        interrupts_enabled(),
        "the IPC handoff test must begin with interrupts enabled"
    );
    crate::syscall::IPC_HANDOFF_OBSERVED.store(0, Ordering::Relaxed);
    cspace::WAKE_HANDOFF_OBSERVED.store(0, Ordering::Relaxed);
    run_server_and_client(Rights::from_bits_truncate(
        Rights::WRITE.bits() | Rights::GRANT.bits(),
    ));
    assert_eq!(
        crate::syscall::IPC_HANDOFF_OBSERVED.load(Ordering::Relaxed),
        3,
        "send and receive must hand off to the scheduler with interrupts disabled"
    );
    assert_eq!(
        cspace::WAKE_HANDOFF_OBSERVED.load(Ordering::Relaxed),
        1,
        "capability mutation and waiter wake-up must not be preempted"
    );
}

pub(super) fn ipc_tasks_without_grant() {
    run_server_and_client(durg_cap::Rights::WRITE);
}

pub(super) fn blocked_user_ipc_tasks_can_be_killed() {
    use durg_cap::Rights;

    // A receiver with no sender must leave its syscall and be reaped on kill.
    let s = Scenario::new(Rights::WRITE);
    let receiver = user::spawn_elf_with_cspace("kill-receiver", module("ipc_server.elf"), s.server)
        .expect("spawn receiver");
    ipc_threads::wait_until_blocked(1);
    receiver.kill();
    assert_eq!(sched::blocked_count(), 0, "receiver was not woken");
    assert_eq!(user_tests::wait(&receiver), user_status::Killed);
    s.client.destroy();
    s.finish();

    // A caller waiting for a receiver must be cancelled without delivering its
    // pending message; both capability spaces and all frames are reclaimed.
    let s = Scenario::new(Rights::WRITE);
    let caller = user::spawn_elf_with_cspace("kill-caller", module("ipc_client.elf"), s.client)
        .expect("spawn caller");
    ipc_threads::wait_until_blocked(1);
    caller.kill();
    assert_eq!(sched::blocked_count(), 0, "caller was not woken");
    assert_eq!(user_tests::wait(&caller), user_status::Killed);
    s.server.destroy();
    s.finish();

    // Once a server has accepted a call, killing its caller must invalidate
    // the pending reply as well as release the caller's resources.
    let s = Scenario::new(Rights::WRITE);
    let (endpoint, _, _) = s.root.endpoint(2, Rights::READ).expect("endpoint");
    let caller = user::spawn_elf_with_cspace("kill-reply", module("ipc_client.elf"), s.client)
        .expect("spawn caller");
    ipc_threads::wait_until_blocked(1);
    sched::ipc_recv(endpoint, None).expect("receive call");
    assert_eq!(sched::blocked_count(), 1, "caller must await the reply");
    caller.kill();
    assert_eq!(sched::blocked_count(), 0, "reply waiter was not woken");
    assert_eq!(user_tests::wait(&caller), user_status::Killed);
    assert_eq!(
        sched::ipc_reply(ipc_threads::message(1, 0)),
        Err(sched::IpcError::NoReplyTarget),
        "a killed caller must not accept a stale reply"
    );
    s.server.destroy();
    s.finish();
}

pub(super) fn ipc_stranger_reaches_nobody() {
    use durg_cap::Rights;
    let s = Scenario::new(Rights::from_bits_truncate(
        Rights::WRITE.bits() | Rights::GRANT.bits(),
    ));
    let stranger_space = CSpace::new().expect("stranger space");
    let server = user::spawn_elf_with_cspace("server", module("ipc_server.elf"), s.server)
        .expect("spawn server");
    let stranger =
        user::spawn_elf_with_cspace("stranger", module("ipc_stranger.elf"), stranger_space)
            .expect("spawn stranger");
    let client = user::spawn_elf_with_cspace("client", module("ipc_client.elf"), s.client)
        .expect("spawn client");
    assert_eq!(
        user_tests::wait(&stranger),
        user_status::Exited(0),
        "the stranger got a message through (exit code = 1 + slot, or 1000 + slot for a receive)"
    );
    // The client's exit code includes a check that the server served exactly the
    // client's own three messages: nothing from the stranger got in between.
    assert_eq!(user_tests::wait(&client), user_status::Exited(0));
    assert_eq!(user_tests::wait(&server), user_status::Exited(0));
    s.finish();
}

pub(super) fn ipc_revoke_cuts_off_and_wakes() {
    use durg_cap::Rights;
    let s = Scenario::new(Rights::WRITE);
    // The client's endpoint and frame capabilities are there (children of the
    // root's, in another space)...
    assert!(identify(&s.client, 2).is_ok());
    assert!(identify(&s.client, 3).is_ok());
    // ...a server waits on the endpoint...
    let server = user::spawn_elf_with_cspace("server", module("ipc_server.elf"), s.server)
        .expect("spawn server");
    ipc_threads::wait_until_blocked(1);
    // ...and the root revokes the region the endpoint was made from.
    assert!(s.root.invoke(0, label::REVOKE, [1, 0, 0]).is_ok());
    // The client's copies are gone, in *its* space, without it doing anything.
    assert_eq!(identify(&s.client, 2), Err(SysError::EmptySlot));
    assert_eq!(identify(&s.client, 3), Err(SysError::EmptySlot));
    // The waiting server was woken with an error (it exits 101 when its receive fails).
    assert_eq!(user_tests::wait(&server), user_status::Exited(101));
    // No task ever ran in the client's space, so it is destroyed here.
    s.client.destroy();
    s.finish();
}

/// `spawn` names a program by its position among the boot modules. The numbers
/// are constants in `durg-syscall` and the names a table in the kernel; both must
/// agree with the order of `boot/limine.conf`, or `spawn` would start the wrong
/// program.
pub(super) fn boot_module_numbers_match() {
    assert_eq!(
        user::PROGRAM_NAMES.len() as u64,
        durg_syscall::module::COUNT,
        "the module count and the name table disagree"
    );
    let found: alloc::vec::Vec<alloc::string::String> = crate::limine::modules()
        .map(|m| alloc::string::String::from(m.name()))
        .collect();
    assert_eq!(found.len(), user::PROGRAM_NAMES.len(), "{found:?}");
    for (index, name) in user::PROGRAM_NAMES.iter().enumerate() {
        assert_eq!(
            found[index],
            alloc::format!("{name}.elf"),
            "module {index} is not {name}"
        );
        assert!(
            vm::module_bytes(index).is_some(),
            "module {index} is not mapped"
        );
    }
    use durg_syscall::module as m;
    for (number, name) in [
        (m::INIT, "init"),
        (m::TASK_A, "task_a"),
        (m::TASK_B, "task_b"),
        (m::DIRTY, "dirty"),
        (m::PROBE, "probe"),
        (m::CAPDEMO, "capdemo"),
        (m::IPC_SERVER, "ipc_server"),
        (m::IPC_CLIENT, "ipc_client"),
        (m::IPC_STRANGER, "ipc_stranger"),
        (m::CRASHER, "crasher"),
        (m::NOTIFICATION_WAITER, "notification_waiter"),
    ] {
        assert_eq!(user::PROGRAM_NAMES[number as usize], name);
    }
}

pub(super) fn exit_after_yielding_with_five() {
    for _ in 0..5 {
        sched::yield_now();
    }
    sched::exit_with(5)
}

pub(super) fn exit_with_seven() {
    sched::exit_with(7)
}

pub(super) fn exit_with_two() {
    sched::exit_with(2)
}

pub(super) fn finish_and_wait(handle: &crate::sched::JoinHandle) {
    for _ in 0..100_000 {
        if handle.is_finished() {
            // Its resources are released by the next thread to run.
            sched::yield_now();
            return;
        }
        sched::yield_now();
    }
    panic!("a thread never finished");
}

pub(super) fn join_semantics() {
    use crate::sched::{ipc_join, IpcError, ThreadId};
    // A thread that has not finished: join waits, and is woken with its code.
    let running = sched::spawn("joined", exit_after_yielding_with_five).expect("spawn");
    assert_eq!(ipc_join(running.id()), Ok(5));
    finish_and_wait(&running);

    // One that has already finished, even if it has been reaped: remembered.
    let quick = sched::spawn("quick", exit_with_seven).expect("spawn");
    finish_and_wait(&quick);
    assert_eq!(ipc_join(quick.id()), Ok(7));
    assert_eq!(
        ipc_join(quick.id()),
        Ok(7),
        "asking twice gives the same answer"
    );

    // A thread that never existed, and the caller itself, are refused.
    assert_eq!(
        ipc_join(ThreadId(u64::MAX - 1)),
        Err(IpcError::NoSuchThread)
    );
    assert_eq!(ipc_join(sched::current_id()), Err(IpcError::NoSuchThread));

    // Only the last 64 threads to finish are remembered.
    let first = sched::spawn("first", exit_with_seven).expect("spawn");
    finish_and_wait(&first);
    assert_eq!(ipc_join(first.id()), Ok(7));
    let filler = || {
        let h = sched::spawn("filler", exit_with_two).expect("spawn");
        finish_and_wait(&h);
        h.id()
    };
    // After 63 more, the first is still remembered: the history holds 64.
    for _ in 0..63 {
        filler();
    }
    assert_eq!(
        ipc_join(first.id()),
        Ok(7),
        "the history must hold the last 64, counting the first"
    );
    // One more pushes it out.
    let last = filler();
    assert_eq!(
        ipc_join(first.id()),
        Err(IpcError::NoSuchThread),
        "a thread 64 exits ago is forgotten"
    );
    assert_eq!(ipc_join(last), Ok(2), "the most recent is remembered");
}
