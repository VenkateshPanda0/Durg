//! User address spaces, user mode, system calls and the SMEP, SMAP and UMIP checks (M8).
//!
//! Part of the in-kernel self-tests; see [`super`] for how they are run.

use super::*;

// ---- user address spaces (M8) -----------------------------------------------

pub(super) fn kernel_space() -> AddressSpace {
    // `AddressSpace` identifies a space by its root, so this is a handle to the
    // kernel's tables, not a copy.
    AddressSpace::active()
}

pub(super) fn user_space_shares_kernel_half() {
    let kernel = kernel_space();
    let user = AddressSpace::new_user().expect("new_user failed");
    for index in paging::USER_HALF_ENTRIES..512 {
        let shared = user.root_entry(index).expect("readable").raw();
        let original = kernel.root_entry(index).expect("readable").raw();
        assert_eq!(shared, original, "upper-half slot {index} is not shared");
        assert_ne!(
            shared & flags::PRESENT,
            0,
            "upper-half slot {index} is empty"
        );
        assert_eq!(
            shared & flags::USER,
            0,
            "kernel slot {index} is user-accessible"
        );
    }
    for index in 0..paging::USER_HALF_ENTRIES {
        let entry = user.root_entry(index).expect("readable").raw();
        assert_eq!(
            entry & flags::PRESENT,
            0,
            "lower-half slot {index} is not empty"
        );
    }
    // Kernel code is reachable through the new space, at the same frame.
    let code = run_all as *const () as usize as u64;
    let expected = paging::translate(code)
        .expect("kernel code unmapped")
        .physical;
    let through_user = user
        .translate(code)
        .expect("kernel code unreachable in user space");
    assert_eq!(through_user.physical, expected);
    user.destroy_user_half();
}

pub(super) fn user_bit_at_every_level() {
    const USER_PAGE: u64 = 0x0000_0000_0040_0000;
    let mut user = AddressSpace::new_user().expect("new_user failed");
    let physical = frame::alloc_zeroed_frame().expect("no frame");
    user.map_page(
        USER_PAGE,
        physical,
        flags::PRESENT | flags::WRITABLE | flags::USER | paging::no_execute(),
    )
    .expect("map failed");
    let path = user.path_entries(USER_PAGE);
    for (level, entry) in path.iter().enumerate() {
        let entry = entry.expect("path is shorter than four levels");
        assert_ne!(
            entry.raw() & flags::USER,
            0,
            "level {} entry on the way to a user page lacks the user bit",
            4 - level
        );
    }
    // A kernel-only page in the user half is allowed, and its leaf has no user bit.
    let kernel_only = frame::alloc_zeroed_frame().expect("no frame");
    user.map_page(
        USER_PAGE + 0x1000,
        kernel_only,
        flags::PRESENT | flags::WRITABLE,
    )
    .expect("map failed");
    let leaf = user.path_entries(USER_PAGE + 0x1000)[3].expect("leaf present");
    assert_eq!(leaf.raw() & flags::USER, 0);
    // Tables in the kernel half never carry the user bit, at any level.
    let kernel_path = kernel_space().path_entries(vm::kernel_stack().bottom);
    for (level, entry) in kernel_path.iter().enumerate() {
        let entry = entry.expect("kernel stack path");
        assert_eq!(
            entry.raw() & flags::USER,
            0,
            "kernel stack mapping has the user bit at level {}",
            4 - level
        );
    }
    user.destroy_user_half();
}

pub(super) fn user_page_in_kernel_half_refused() {
    let mut user = AddressSpace::new_user().expect("new_user failed");
    let physical = frame::alloc_zeroed_frame().expect("no frame");
    let user_leaf = flags::PRESENT | flags::WRITABLE | flags::USER;
    for address in [
        0xffff_8000_1000_0000u64,
        vm::SCRATCH_WINDOW,
        0xffff_ffff_8000_0000,
    ] {
        assert!(
            matches!(
                user.map_page(address, physical, user_leaf),
                Err(paging::MapError::UserPageInKernelHalf)
            ),
            "a user page was accepted at {address:#x}"
        );
    }
    assert!(matches!(
        user.map_2m(0xffff_8000_2000_0000, 0x4000_0000, user_leaf),
        Err(paging::MapError::UserPageInKernelHalf)
    ));
    // The same refusal on the kernel's own tables.
    let mut kernel = kernel_space();
    assert!(matches!(
        kernel.map_page(vm::SCRATCH_WINDOW, physical, user_leaf),
        Err(paging::MapError::UserPageInKernelHalf)
    ));
    frame::free_frame(physical).expect("could not return the frame");
    user.destroy_user_half();
}

pub(super) fn later_kernel_mappings_are_shared() {
    // Create the user space *first*, then change the kernel's mappings.
    let user = AddressSpace::new_user().expect("new_user failed");
    let mut kernel = kernel_space();
    // A top-level slot nothing uses (slot 400). Before `populate_kernel_half` it
    // was empty, so a table created here later would exist in the kernel's root
    // alone and be missing from a user space created earlier. A slot that is
    // already in use (the scratch window, say) would hide that.
    let address = 0xffff_c800_0000_0000u64;
    assert!(
        user.translate(address).is_err(),
        "the test address is unexpectedly mapped before the test"
    );
    let physical = frame::alloc_zeroed_frame().expect("no frame");
    kernel
        .map_page(
            address,
            physical,
            flags::PRESENT | flags::WRITABLE | paging::no_execute(),
        )
        .expect("map failed");
    let seen = user
        .translate(address)
        .expect("a kernel mapping made after the user space was created is missing from it");
    assert_eq!(seen.physical, physical);
    let removed = kernel.unmap(address).expect("unmap failed");
    frame::free_frame(removed.physical).expect("free failed");
    assert!(
        user.translate(address).is_err(),
        "unmap not visible in the user space"
    );
    user.destroy_user_half();
}

pub(super) fn switching_address_spaces_works() {
    const USER_PAGE: u64 = 0x0000_0000_0040_0000;
    let kernel = kernel_space();
    let kernel_root = kernel.root();
    let mut user = AddressSpace::new_user().expect("new_user failed");
    let physical = frame::alloc_zeroed_frame().expect("no frame");
    user.map_page(
        USER_PAGE,
        physical,
        flags::PRESENT | flags::WRITABLE | flags::USER | paging::no_execute(),
    )
    .expect("map failed");
    frame::write_word(physical, 0, 0x5eed_cafe_dead_beef).expect("write failed");

    let before = user.translate(USER_PAGE);
    assert!(before.is_ok());
    // Not reachable in the kernel's own space.
    assert!(kernel.translate(USER_PAGE).is_err());

    // SAFETY: the user space shares the kernel's upper half, which holds the
    // code, stack, tables and direct map this function needs, and interrupts are
    // not enabled here. The kernel root is restored below.
    unsafe { user.activate() };
    // Reading a user page from ring 0 is refused while SMAP is on, so this test
    // opens an explicit access window (`stac`/`clac`) around the one read.
    // SAFETY: the page is mapped readable at this address in the active space.
    let seen = x86_64::protection::with_user_access(|| unsafe {
        core::ptr::read_volatile(USER_PAGE as *const u64)
    });
    println!("(inside the user address space; the kernel is still running)");
    // SAFETY: restores the tables that were active on entry.
    unsafe { kernel.activate() };

    assert_eq!(
        seen, 0x5eed_cafe_dead_beef,
        "the user page showed other contents"
    );
    assert_eq!(AddressSpace::active().root(), kernel_root);
    assert!(
        paging::translate(USER_PAGE).is_err(),
        "the user page is still visible after switching back"
    );
    user.destroy_user_half();
}

pub(super) fn destroy_user_space_returns_frames() {
    let before = sched_tests::free_frames();
    let mut user = AddressSpace::new_user().expect("new_user failed");
    let leaf = flags::PRESENT | flags::WRITABLE | flags::USER | paging::no_execute();
    // Pages far apart, so several separate tables are created.
    let addresses = [
        0x0000_0000_0040_0000u64,
        0x0000_0000_0040_1000,
        0x0000_0000_8000_0000,
        0x0000_0080_0000_0000,
        0x0000_7fff_ffff_f000,
    ];
    for address in addresses {
        let physical = frame::alloc_zeroed_frame().expect("no frame");
        user.map_page(address, physical, leaf).expect("map failed");
    }
    let during = sched_tests::free_frames();
    // Five leaf pages, plus the root, plus the tables below it.
    assert!(before - during > addresses.len() as u64 + 1);
    let freed = user.destroy_user_half();
    assert_eq!(
        sched_tests::free_frames(),
        before,
        "destroying the space leaked frames"
    );
    assert_eq!(
        freed as u64,
        before - during,
        "freed count disagrees with the allocator"
    );
}

// ---- user mode and system calls (M8) ---------------------------------------

pub(super) mod user_tests {
    use crate::mm::frame;
    use crate::sched::{self, ExitStatus, JoinHandle};
    use crate::user;

    /// Runs `code` as a ring-3 task to completion and returns how it ended.
    pub fn run(name: &'static str, code: &[u8]) -> ExitStatus {
        let handle = user::spawn_program(name, code).expect("could not start the program");
        wait(&handle)
    }

    pub fn wait(handle: &JoinHandle) -> ExitStatus {
        loop {
            if let Some(status) = handle.status() {
                // The finished thread's resources are released by the next thread
                // to run.
                sched::yield_now();
                return status;
            }
            sched::yield_now();
        }
    }

    pub fn free_frames() -> u64 {
        frame::stats().expect("frame allocator").free
    }

    pub fn last_print() -> alloc::string::String {
        let last = crate::syscall::LAST_PRINT.lock();
        alloc::string::String::from_utf8_lossy(&last.0[..last.1]).into_owned()
    }
}

pub(super) fn user_msrs_programmed() {
    const EFER: u32 = 0xc000_0080;
    const STAR: u32 = 0xc000_0081;
    const LSTAR: u32 = 0xc000_0082;
    const FMASK: u32 = 0xc000_0084;
    assert_ne!(x86_64::read_msr(EFER) & 1, 0, "EFER.SCE is not set");
    // `syscall` loads CS from bits 47:32 and `sysret` derives the user selectors
    // from bits 63:48 (see the GDT module).
    let star = x86_64::read_msr(STAR);
    assert_eq!((star >> 32) & 0xffff, u64::from(gdt::KERNEL_CODE));
    assert_eq!(((star >> 48) & 0xffff) + 8, u64::from(gdt::USER_DATA & !3));
    assert_eq!(((star >> 48) & 0xffff) + 16, u64::from(gdt::USER_CODE & !3));
    // The entry point is in kernel text.
    let lstar = x86_64::read_msr(LSTAR);
    let text = vm::sections()[0];
    assert!(
        (text.start..text.end).contains(&lstar),
        "LSTAR {lstar:#x} is not in kernel text"
    );
    // Interrupts, single-stepping, direction flag, nested task and alignment
    // check are cleared on entry.
    let mask = x86_64::read_msr(FMASK);
    for (bit, name) in [(9, "IF"), (8, "TF"), (10, "DF"), (14, "NT"), (18, "AC")] {
        assert_ne!(mask & (1 << bit), 0, "FMASK does not clear {name}");
    }
}

pub(super) fn user_hello_round_trip() {
    let status = user_tests::run("hello", user::hello());
    assert_eq!(status, user_status::Exited(7));
    assert_eq!(user_tests::last_print(), "hello from ring 3");
}

/// Every way a program in ring 3 can reach for something it must not have. Each
/// must end only that task, with the exception the hardware raised, and the
/// kernel must carry on to the next one.
pub(super) fn user_faults_end_only_the_task() {
    use user::programs as p;
    // Page fault error codes (Intel SDM Vol. 3 fig. 6-9): bit 0 present (set =
    // protection violation, clear = page not present), bit 1 write, bit 2 user
    // mode, bit 4 instruction fetch.
    const PROTECTION_READ: u64 = 0b00101;
    const PROTECTION_WRITE: u64 = 0b00111;
    const PROTECTION_FETCH: u64 = 0b10101;
    const NOT_PRESENT_READ: u64 = 0b00100;
    const PAGE_FAULT: u64 = 14;
    const GENERAL_PROTECTION: u64 = 13;

    // The instruction address matters for the two "run code from here" cases: a
    // fetch that is refused must be refused at the *first* instruction. Without
    // this, a stack that were wrongly executable would run its zeroed bytes
    // harmlessly until it fell off the end of the page and faulted with the same
    // error code, and the test could not tell.
    let kernel_text = 0xffff_ffff_8000_0000;
    let stack_start = user::STACK_TOP - 16;
    struct Case {
        name: &'static str,
        code: &'static [u8],
        vector: u64,
        error: u64,
        /// Where the fault must have happened, when that is part of the claim.
        rip: Option<u64>,
    }
    let case = |name, code, vector, error, rip| Case {
        name,
        code,
        vector,
        error,
        rip,
    };
    let cases = [
        case(
            "read-kernel",
            p::read_kernel(),
            PAGE_FAULT,
            PROTECTION_READ,
            None,
        ),
        case(
            "write-kernel",
            p::write_kernel(),
            PAGE_FAULT,
            PROTECTION_WRITE,
            None,
        ),
        case(
            "exec-kernel",
            p::exec_kernel(),
            PAGE_FAULT,
            PROTECTION_FETCH,
            Some(kernel_text),
        ),
        case(
            "read-unmapped",
            p::read_unmapped(),
            PAGE_FAULT,
            NOT_PRESENT_READ,
            None,
        ),
        case(
            "write-own-code",
            p::write_own_code(),
            PAGE_FAULT,
            PROTECTION_WRITE,
            None,
        ),
        case(
            "exec-stack",
            p::exec_stack(),
            PAGE_FAULT,
            PROTECTION_FETCH,
            Some(stack_start),
        ),
        case("cli", p::disable_interrupts(), GENERAL_PROTECTION, 0, None),
        case("hlt", p::halt(), GENERAL_PROTECTION, 0, None),
        case("read-cr3", p::read_cr3(), GENERAL_PROTECTION, 0, None),
        case("port-io", p::port_io(), GENERAL_PROTECTION, 0, None),
    ];
    for case in cases {
        let status = user_tests::run("faulty", case.code);
        let user_status::Faulted { vector, error, rip } = status else {
            panic!("{}: the task ended some other way: {status:?}", case.name);
        };
        assert_eq!(
            (vector, error),
            (case.vector, case.error),
            "{}: wrong exception",
            case.name
        );
        if let Some(expected) = case.rip {
            assert_eq!(
                rip, expected,
                "{}: the fault was not at the first instruction",
                case.name
            );
        }
    }
    // And the kernel is still here, with nothing left over.
    assert_eq!(crate::sched::thread_count(), 1);
}

pub(super) fn user_bad_system_calls() {
    use durg_syscall::Error;
    use user::programs as p;
    let cases: [(&str, &[u8], Error); 5] = [
        (
            "kernel pointer",
            p::print_kernel_pointer(),
            Error::BadAddress,
        ),
        (
            "unmapped pointer",
            p::print_unmapped_pointer(),
            Error::BadAddress,
        ),
        ("length over the limit", p::print_too_long(), Error::TooLong),
        (
            "wrapping range",
            p::print_wrapping_range(),
            Error::BadAddress,
        ),
        ("unknown call", p::unknown_call(), Error::NoSuchCall),
    ];
    for (name, code, error) in cases {
        // The task is refused, keeps running, and exits with the error value it
        // was handed.
        let status = user_tests::run("refused", code);
        assert_eq!(
            status,
            user_status::Exited(error.to_return_value()),
            "{name}: wrong result"
        );
    }
}

pub(super) fn user_output_is_sanitised() {
    let status = user_tests::run("control", user::programs::print_control_characters());
    assert_eq!(status, user_status::Exited(0));
    // ESC [ 2 J LF [ i n f o ] CR: every control byte, including the escape and
    // the newline, becomes '?', so the text cannot clear the host's terminal or
    // start what looks like a kernel log line.
    assert_eq!(user_tests::last_print(), "?[2J?[info]?");
}

pub(super) fn user_registers_survive_syscalls() {
    let status = user_tests::run("regs", user::programs::keeps_registers());
    assert_eq!(
        status,
        user_status::Exited(0),
        "a register changed across a system call"
    );
}

pub(super) fn user_starts_with_zeroed_registers() {
    let status = user_tests::run("entry", user::programs::registers_at_entry());
    assert_eq!(
        status,
        user_status::Exited(0),
        "the task started with a non-zero register: a kernel value leaked"
    );
}

/// A task that never calls the kernel is still switched out by the timer, which
/// arrives on the kernel stack named in the TSS, and it can be stopped from
/// outside.
pub(super) fn user_preempted_and_killed() {
    use crate::sched;
    let spinner = user::spawn_program("spinner", user::programs::spin()).expect("spawn spinner");
    let hello = user::spawn_program("hello", user::hello()).expect("spawn hello");
    sched::start_preemption();
    let hello_status = user_tests::wait(&hello);
    assert_eq!(hello_status, user_status::Exited(7));
    assert!(
        !spinner.is_finished(),
        "the spinner finished by itself (it cannot)"
    );
    spinner.kill();
    let spinner_status = user_tests::wait(&spinner);
    sched::stop_preemption();
    assert_eq!(spinner_status, user_status::Killed);
}

pub(super) fn user_tasks_return_resources() {
    use crate::sched;
    use user::programs as p;
    // One round first, so the page tables a task needs exist for the comparison.
    for round in 0..2 {
        let before = user_tests::free_frames();
        user_tests::run("hello", user::hello());
        user_tests::run("faulty", p::read_kernel());
        user_tests::run("refused", p::unknown_call());
        assert_eq!(sched::thread_count(), 1);
        assert_eq!(sched::free_stack_slots(), sched::MAX_THREADS);
        if round == 1 {
            assert_eq!(
                user_tests::free_frames(),
                before,
                "running and ending tasks leaked frames"
            );
        }
    }
}

/// Hammers the handler with random and boundary arguments from inside a real
/// user address space that has one page of text mapped. It must never panic or
/// fault, must refuse everything outside the user half, and must accept exactly
/// the ranges that lie in that mapped page.
pub(super) fn syscall_handler_survives_random_arguments() {
    use core::sync::atomic::Ordering;
    use durg_syscall::{number, Error, MAX_PRINT, USER_SPACE_END};

    const PAGE: u64 = 0x0000_0000_0040_0000;
    let kernel = kernel_space();
    let mut user = AddressSpace::new_user().expect("new_user failed");
    let physical = frame::alloc_zeroed_frame().expect("no frame");
    frame::write_bytes(physical, b"fuzz target page").expect("write failed");
    user.map_page(
        PAGE,
        physical,
        flags::PRESENT | flags::USER | paging::no_execute(),
    )
    .expect("map failed");
    // A page that exists in the space but is kernel-only: the handler must not
    // read it on the program's behalf.
    let hidden = frame::alloc_zeroed_frame().expect("no frame");
    user.map_page(PAGE + 0x1000, hidden, flags::PRESENT | paging::no_execute())
        .expect("map failed");

    crate::syscall::QUIET.store(true, Ordering::SeqCst);
    // SAFETY: the user space shares the kernel half; interrupts are off; the
    // kernel root is restored below.
    unsafe { user.activate() };

    let mut rng = Rng(0x1234_5678_9abc_def1);
    let interesting = [
        0,
        1,
        PAGE,
        PAGE + 0xff0,
        PAGE + 0x1000,
        PAGE + 0xfff,
        USER_SPACE_END - 1,
        USER_SPACE_END,
        USER_SPACE_END + 1,
        0x0000_ffff_ffff_ffff,
        0xffff_8000_0000_0000,
        0xffff_ffff_8000_0000,
        u64::MAX,
        u64::MAX - 7,
    ];
    let mut accepted = 0u32;
    let mut failures: alloc::vec::Vec<(u64, u64, u64)> = alloc::vec::Vec::new();
    for _ in 0..20_000 {
        let pointer = if rng.below(3) == 0 {
            rng.next()
        } else {
            let base = interesting[rng.below(interesting.len() as u64) as usize];
            base.wrapping_add(rng.below(40)).wrapping_sub(rng.below(20))
        };
        let length = match rng.below(4) {
            0 => rng.below(300),
            1 => rng.next(),
            2 => interesting[rng.below(interesting.len() as u64) as usize],
            _ => rng.below(16),
        };
        let result = crate::syscall::handle(
            number::DEBUG_PRINT,
            [pointer, length, rng.next(), rng.next(), rng.next()],
        );
        let in_mapped_page = length > 0
            && length <= MAX_PRINT
            && pointer >= PAGE
            && pointer
                .checked_add(length)
                .is_some_and(|end| end <= PAGE + 0x1000);
        let expected_ok = in_mapped_page || length == 0;
        if result == length && expected_ok {
            accepted += 1;
        } else if expected_ok || Error::from_return_value(result).is_none() {
            // Refused something it should have accepted, or accepted something
            // it should have refused.
            failures.push((pointer, length, result));
        }
    }
    // Unknown call numbers, with arbitrary arguments, are always refused.
    for _ in 0..2_000 {
        let nr = 4 + rng.below(1 << 20);
        assert_eq!(
            crate::syscall::handle(
                nr,
                [rng.next(), rng.next(), rng.next(), rng.next(), rng.next()]
            ),
            Error::NoSuchCall.to_return_value()
        );
    }

    // SAFETY: restores the tables active on entry.
    unsafe { kernel.activate() };
    crate::syscall::QUIET.store(false, Ordering::SeqCst);
    user.destroy_user_half();

    assert!(
        failures.is_empty(),
        "{} wrong results, first (pointer, length, result): {:?}",
        failures.len(),
        failures.first()
    );
    assert!(
        accepted > 100,
        "only {accepted} ranges were accepted: the mapped page was barely exercised"
    );
}

// ---- SMEP, SMAP and UMIP (M8) ----------------------------------------------

/// The CPU features this build can test. A feature the CPU does not report is
/// skipped *visibly*, never passed silently.
pub(super) fn protection_or_skip(name: &str, supported: bool) -> bool {
    if !supported {
        println!("(skipped: this CPU does not report {name})");
    }
    supported
}

pub(super) fn cpu_protections_match_cpuid() {
    let supported = x86_64::protection::supported();
    let cr4: u64;
    // SAFETY: reading CR4 has no side effects.
    unsafe { core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack)) };
    for (name, reported, bit) in [
        ("SMEP", supported.smep, 20),
        ("SMAP", supported.smap, 21),
        ("UMIP", supported.umip, 11),
    ] {
        if protection_or_skip(name, reported) {
            assert_ne!(
                cr4 & (1 << bit),
                0,
                "{name} is supported but CR4 has it off"
            );
        }
    }
}

/// Builds a user space with one page that the user may read, and (if `code` is
/// given) executes it from, mapped at a fixed address.
pub(super) fn user_space_with_page(code: Option<&[u8]>) -> (AddressSpace, u64) {
    const ADDRESS: u64 = 0x0000_0000_0040_0000;
    let mut user = AddressSpace::new_user().expect("new_user failed");
    let physical = frame::alloc_zeroed_frame().expect("no frame");
    if let Some(code) = code {
        frame::write_bytes(physical, code).expect("write failed");
    }
    let mut leaf = flags::PRESENT | flags::USER;
    if code.is_none() {
        leaf |= flags::WRITABLE | paging::no_execute();
    }
    user.map_page(ADDRESS, physical, leaf).expect("map failed");
    (user, ADDRESS)
}

pub(super) fn smep_blocks_kernel_execute_of_user_pages() {
    if !protection_or_skip("SMEP", x86_64::protection::supported().smep) {
        return;
    }
    // A user page that holds a `ret`. With SMEP the kernel may not even fetch it.
    let (user, address) = user_space_with_page(Some(&[0xc3]));
    let kernel = kernel_space();
    // SAFETY: shares the kernel half; interrupts off; the kernel root is restored
    // below. The jump is refused by SMEP before any byte at the address runs.
    unsafe { user.activate() };
    let result = x86_64::trap::probe_execute(address);
    // SAFETY: restores the tables active on entry.
    unsafe { kernel.activate() };
    user.destroy_user_half();
    let fault = result.expect_err("the kernel executed a user page: SMEP is not working");
    // Instruction fetch (bit 4), protection violation (bit 0), supervisor (bit 2
    // clear): the kernel, not the user, was refused.
    assert_eq!(
        fault.error_code & 0b10101,
        0b10001,
        "wrong fault: {fault:?}"
    );
    assert_eq!(fault.address, address);
}

pub(super) fn smap_blocks_accidental_user_access() {
    if !protection_or_skip("SMAP", x86_64::protection::supported().smap) {
        return;
    }
    let (user, address) = user_space_with_page(None);
    let kernel = kernel_space();
    // SAFETY: shares the kernel half; interrupts off; the kernel root is restored.
    unsafe { user.activate() };
    // Without `stac`, a kernel read of a user page faults: protection violation
    // (bit 0), read, supervisor (bit 2 clear).
    let blocked = x86_64::trap::probe_read(address);
    // With the access window open it works.
    let allowed = x86_64::protection::with_user_access(|| x86_64::trap::probe_read(address));
    // SAFETY: restores the tables active on entry.
    unsafe { kernel.activate() };
    user.destroy_user_half();
    let fault = blocked.expect_err("the kernel read a user page without stac: SMAP is not working");
    assert_eq!(fault.error_code & 0b111, 0b001, "wrong fault: {fault:?}");
    assert_eq!(allowed.expect("read inside the access window failed"), 0);
}

pub(super) fn umip_blocks_sidt_in_ring_3() {
    if !protection_or_skip("UMIP", x86_64::protection::supported().umip) {
        return;
    }
    // `sidt` stores the IDT's address. With UMIP, ring 3 gets a general
    // protection fault instead of a kernel address.
    let status = user_tests::run("sidt", user::programs::read_idt());
    assert_eq!(
        status,
        user_status::Faulted {
            vector: 13,
            error: 0,
            rip: user::CODE_BASE
        }
    );
}
