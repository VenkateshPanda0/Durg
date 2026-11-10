//! System call handlers: what the kernel does when ring 3 asks.
//!
//! The architecture side (`arch::x86_64::syscall`) delivers the call number and
//! up to five arguments. The pure side (the `durg-syscall` crate) decodes them and
//! validates user pointers, with host tests. This module acts on a decoded call.
//!
//! ## Rules for touching user memory
//!
//! A user program controls every byte it passes, so the kernel **never
//! dereferences a user pointer**. To read a user buffer it:
//!
//! 1. validates the range is entirely in the lower half and short enough
//!    (`validate_user_range`: this alone refuses every kernel address and every
//!    wrapping range);
//! 2. splits it into pieces that each lie in one page (`split_by_page`);
//! 3. for each piece, asks the page tables of the *calling task's* address space
//!    where it is, requires the page to be mapped **and user-accessible**, and
//!    copies the bytes out through the kernel's direct map.
//!
//! Because the kernel reaches the bytes through its own mapping and never through
//! the user's pointer, an unmapped page is an error return and not a kernel page
//! fault, and SMAP (when enabled) never has to be switched off.
//!
//! ## What the console shows
//!
//! Text from a program is untrusted output. Control characters, escape
//! sequences and newlines are replaced before printing, and every line is tagged
//! with the task that wrote it, so a program cannot forge a kernel log line or
//! drive the host's terminal.

use durg_paging::flags;
use durg_syscall::{split_by_page, validate_user_range, Call, Error, MAX_PRINT};

use durg_cap::Rights;

use crate::cspace::CSpace;
use crate::mm::frame;
use crate::mm::paging::AddressSpace;
use crate::sched;

/// Vector number recorded for a task the kernel ended for a reason other than a
/// CPU exception.
pub const KILLED_BY_KERNEL: u64 = 0x100;

/// What a system call gives back: the value for `rax`, and for calls that
/// receive a message, the registers it arrives in (`rdi` = badge, `rdx`, `r10`,
/// `r8` = the three words). Every other register is left as the program had it.
pub struct Return {
    pub rax: u64,
    pub message_registers: Option<[u64; 4]>,
}

impl Return {
    fn value(rax: u64) -> Return {
        Return {
            rax,
            message_registers: None,
        }
    }

    fn error(error: Error) -> Return {
        Return::value(error.to_return_value())
    }

    fn received(r: sched::Received) -> Return {
        Return {
            rax: durg_syscall::received_label(r.message.label, r.got_cap),
            message_registers: Some([
                r.badge,
                r.message.words[0],
                r.message.words[1],
                r.message.words[2],
            ]),
        }
    }
}

/// Handles one system call and returns the value for the program's `rax`.
/// (Calls that return a message also set registers; see [`handle_full`].)
#[cfg_attr(not(feature = "selftest"), allow(dead_code))]
pub fn handle(number: u64, arguments: [u64; 5]) -> u64 {
    handle_full(number, arguments).rax
}

fn ipc_error(error: sched::IpcError) -> Error {
    match error {
        sched::IpcError::Aborted => Error::Aborted,
        sched::IpcError::NoReplyTarget => Error::NoReplyTarget,
        sched::IpcError::NoSuchThread => Error::NoSuchTask,
    }
}

// The self-test records whether a timer could preempt the capability-to-IPC
// handoff. Bits 0 and 1 record send/receive; bits 2 and 3 record an unsafe
// handoff with interrupts enabled. No extra work exists in release builds.
#[cfg(feature = "selftest")]
pub static IPC_HANDOFF_OBSERVED: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

#[cfg(feature = "selftest")]
fn observe_ipc_handoff(receive: bool) {
    use core::sync::atomic::Ordering;
    let bit = if receive { 2 } else { 1 };
    let enabled = if sched::interrupts_enabled() {
        bit << 2
    } else {
        0
    };
    IPC_HANDOFF_OBSERVED.fetch_or(bit | enabled, Ordering::Relaxed);
}

/// `send` and `call`: both need the write right on the endpoint capability, and a
/// message that carries a capability needs the grant right too.
fn send_message(cptr: u64, label: u64, words: [u64; 3], call: bool) -> Result<Return, Error> {
    // On the single-CPU kernel, no other task may revoke and reuse this
    // endpoint between resolving its capability and queuing the operation.
    sched::without_interrupts(|| {
        let space = sched::current_cspace().ok_or(Error::InvalidSlot)?;
        let (label, cap_slot) = durg_syscall::parse_message_label(label)?;
        let (endpoint, badge, rights) = space.endpoint(cptr, Rights::WRITE)?;
        #[cfg(feature = "selftest")]
        observe_ipc_handoff(false);
        if let Some(slot) = cap_slot {
            if !rights.contains(Rights::GRANT) {
                return Err(Error::NoRights);
            }
            space.check_sendable(slot)?;
        }
        let message = sched::Message {
            label,
            words,
            cap_slot,
        };
        match sched::ipc_send(endpoint, badge, message, call).map_err(ipc_error)? {
            Some(reply) => Ok(Return::received(reply)),
            None => Ok(Return::value(0)),
        }
    })
}

fn receive_message(cptr: u64, slot: u64) -> Result<Return, Error> {
    sched::without_interrupts(|| {
        let space = sched::current_cspace().ok_or(Error::InvalidSlot)?;
        let want = durg_syscall::parse_receive_slot(slot)?;
        let (endpoint, _, _) = space.endpoint(cptr, Rights::READ)?;
        #[cfg(feature = "selftest")]
        observe_ipc_handoff(true);
        sched::ipc_recv(endpoint, want)
            .map(Return::received)
            .map_err(ipc_error)
    })
}

fn reply_message(label: u64, words: [u64; 3]) -> Result<Return, Error> {
    let label = u32::try_from(label).map_err(|_| Error::BadArgument)?;
    sched::ipc_reply(sched::Message {
        label,
        words,
        cap_slot: None,
    })
    .map_err(ipc_error)?;
    Ok(Return::value(0))
}

/// `spawn` and `join`, which need a spawner capability. They run here and not in
/// the capability code because they allocate and take the scheduler lock, which
/// must come before the capability lock.
fn spawn_or_join(space: &CSpace, cptr: u64, label: u64, args: [u64; 3]) -> Result<u64, Error> {
    use durg_syscall::label as op;
    durg_syscall::require_plain_label(label)?;
    space.require_spawner(cptr)?;
    if durg_syscall::operation(label) == op::JOIN {
        return sched::ipc_join(sched::ThreadId(args[0])).map_err(ipc_error);
    }
    let [module, first, count] = args;
    if module >= durg_syscall::module::COUNT || count > durg_syscall::MAX_SPAWN_CAPS {
        return Err(Error::BadArgument);
    }
    let index = module as usize;
    let file = crate::mm::vm::module_bytes(index).ok_or(Error::BadArgument)?;
    // The kernel builds the new task's address space: an interim arrangement
    // (ADR-0011) that lets the holder of a spawner capability cost the kernel
    // memory, bounded by the thread limit.
    let (address_space, entry) = crate::user::build_space(file).map_err(|_| Error::NoResources)?;
    let Some(child) = CSpace::new() else {
        address_space.destroy_user_half();
        return Err(Error::NoResources);
    };
    if let Err(error) = space.copy_to(first, count, &child) {
        child.destroy();
        address_space.destroy_user_half();
        return Err(error);
    }
    // If starting fails, the address space and capability space are freed.
    sched::spawn_user_with_cspace(
        crate::user::PROGRAM_NAMES[index],
        address_space,
        entry,
        child,
    )
    .map(|handle| handle.id().0)
    .map_err(|_| Error::NoResources)
}

/// Handles one system call.
pub fn handle_full(number: u64, arguments: [u64; 5]) -> Return {
    let result = |r: Result<Return, Error>| r.unwrap_or_else(Return::error);
    match durg_syscall::decode(number, arguments) {
        Ok(Call::Send { cptr, label, words }) => result(send_message(cptr, label, words, false)),
        Ok(Call::CallEndpoint { cptr, label, words }) => {
            result(send_message(cptr, label, words, true))
        }
        Ok(Call::Recv { cptr, slot }) => result(receive_message(cptr, slot)),
        Ok(Call::Reply { label, words }) => result(reply_message(label, words)),
        Ok(Call::Invoke { cptr, label, args })
            if matches!(
                durg_syscall::operation(label),
                durg_syscall::label::SIGNAL | durg_syscall::label::WAIT
            ) =>
        {
            result(sched::without_interrupts(|| {
                durg_syscall::require_plain_label(label)?;
                if args != [0; 3] {
                    return Err(Error::BadArgument);
                }
                let space = sched::current_cspace().ok_or(Error::InvalidSlot)?;
                if durg_syscall::operation(label) == durg_syscall::label::SIGNAL {
                    let (object, badge) = space.notification(cptr, Rights::WRITE)?;
                    sched::notification_signal(object, badge)?;
                    Ok(Return::value(0))
                } else {
                    let (object, _) = space.notification(cptr, Rights::READ)?;
                    let badge = sched::notification_wait(object)?;
                    Ok(Return {
                        rax: 0,
                        message_registers: Some([badge, 0, 0, 0]),
                    })
                }
            }))
        }
        Ok(Call::Invoke { cptr, label, args })
            if matches!(
                durg_syscall::operation(label),
                durg_syscall::label::SPAWN | durg_syscall::label::JOIN
            ) =>
        {
            // A task with no capability space can name nothing.
            result(match sched::current_cspace() {
                Some(space) => spawn_or_join(&space, cptr, label, args).map(Return::value),
                None => Err(Error::InvalidSlot),
            })
        }
        other => Return::value(handle_simple(other)),
    }
}

fn handle_simple(decoded: Result<Call, Error>) -> u64 {
    match decoded {
        Ok(
            Call::Send { .. } | Call::Recv { .. } | Call::CallEndpoint { .. } | Call::Reply { .. },
        ) => {
            unreachable!("message calls are handled before this")
        }
        Err(error) => error.to_return_value(),
        Ok(Call::DebugPrint { pointer, length }) => match debug_print(pointer, length) {
            Ok(written) => written,
            Err(error) => error.to_return_value(),
        },
        Ok(Call::Yield) => {
            sched::yield_now();
            0
        }
        Ok(Call::Exit { code }) => sched::exit_with(code),
        Ok(Call::Invoke { cptr, label, args }) => {
            // A task with no capability space can name nothing.
            let result = match sched::current_cspace() {
                Some(space) => space.invoke(cptr, label, args),
                None => Err(Error::InvalidSlot),
            };
            match result {
                Ok(value) => value,
                Err(error) => error.to_return_value(),
            }
        }
    }
}

/// Copies up to `length` bytes of the calling task's memory at `pointer` into
/// `buffer`. Every byte must be in a page that is mapped and user-accessible.
pub fn copy_from_user(pointer: u64, length: u64, buffer: &mut [u8]) -> Result<(), Error> {
    validate_user_range(pointer, length, buffer.len() as u64)?;
    let space = AddressSpace::active();
    let mut filled = 0usize;
    for chunk in split_by_page(pointer, length) {
        let mapping = space
            .translate(chunk.address)
            .map_err(|_| Error::BadAddress)?;
        // A page the program could not itself read must not be readable on its
        // behalf, or a task could use the kernel to peek at kernel-only pages
        // that happen to be mapped in its own tables.
        if mapping.entry.raw() & flags::USER == 0 {
            return Err(Error::BadAddress);
        }
        let end = filled + chunk.length as usize;
        frame::read_bytes(mapping.physical, &mut buffer[filled..end]).ok_or(Error::BadAddress)?;
        filled = end;
    }
    Ok(())
}

/// The last line a task printed, for tests. (Sanitised text, at most
/// `MAX_PRINT` bytes.)
#[cfg(feature = "selftest")]
pub static LAST_PRINT: crate::arch::x86_64::IrqLock<([u8; MAX_PRINT as usize], usize)> =
    crate::arch::x86_64::IrqLock::new(([0; MAX_PRINT as usize], 0));

/// Tests set this to run the handler thousands of times without flooding the
/// serial port.
#[cfg(feature = "selftest")]
pub static QUIET: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn debug_print(pointer: u64, length: u64) -> Result<u64, Error> {
    let mut buffer = [0u8; MAX_PRINT as usize];
    copy_from_user(pointer, length, &mut buffer)?;
    let text = &mut buffer[..length as usize];
    for byte in text.iter_mut() {
        if !(0x20..=0x7e).contains(byte) {
            *byte = b'?';
        }
    }
    // Printable ASCII is valid UTF-8, so this cannot fail; the fallback keeps the
    // kernel panic-free regardless.
    #[cfg(feature = "selftest")]
    {
        let mut last = LAST_PRINT.lock();
        last.0[..text.len()].copy_from_slice(text);
        last.1 = text.len();
    }
    let text = core::str::from_utf8(text).unwrap_or("?");
    #[cfg(feature = "selftest")]
    if QUIET.load(core::sync::atomic::Ordering::Relaxed) {
        return Ok(length);
    }
    println!(
        "[user {} {}] {text}",
        sched::current_id(),
        sched::current_name()
    );
    Ok(length)
}

/// Ends the calling task because the kernel will not let it continue.
pub fn kill_current(reason: &str) -> ! {
    println!(
        "[user {} {}] killed by the kernel: {reason}",
        sched::current_id(),
        sched::current_name()
    );
    sched::exit_faulted(KILLED_BY_KERNEL, 0, 0)
}
