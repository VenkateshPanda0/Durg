//! ELF loading and isolation between tasks (M9).
//!
//! Part of the in-kernel self-tests; see [`super`] for how they are run.

use super::*;

// ---- ELF loading and isolation between tasks (M9) --------------------------

pub(super) const MODULES: [&str; 11] = [
    "init.elf",
    "task_a.elf",
    "task_b.elf",
    "dirty.elf",
    "probe.elf",
    "capdemo.elf",
    "ipc_server.elf",
    "ipc_client.elf",
    "ipc_stranger.elf",
    "crasher.elf",
    "notification_waiter.elf",
];

pub(super) fn module(name: &str) -> &'static [u8] {
    vm::module_by_name(name).unwrap_or_else(|| panic!("boot module {name} is missing"))
}

pub(super) fn boot_modules_parse_as_elf() {
    for name in MODULES {
        let file = module(name);
        assert_eq!(&file[..4], b"\x7fELF", "{name} is not an ELF file");
        let image = durg_elf::parse(file, &user::elf_policy())
            .unwrap_or_else(|error| panic!("{name} was refused: {error:?}"));
        // Where `_start` lands inside the code segment depends on how the
        // compiler orders functions (CI's build put it at 0x4000c0), so check
        // that the entry is inside the first segment, not at a fixed address.
        let code = image
            .segments()
            .first()
            .expect("a program has a code segment");
        assert!(
            code.perms.execute && (code.vaddr..code.vaddr + code.filesz).contains(&image.entry),
            "{name}: entry {:#x} is not inside the code segment",
            image.entry
        );
        // The layout depends on the build profile (a release build may have no
        // read-only constants at all), so check properties and not an exact list:
        // the first segment is code (read+execute); every other one is either
        // read-only or read+write and never executable; and every program with
        // data or bss (all but `init` and `capdemo`) has a writable segment.
        let perms: alloc::vec::Vec<(bool, bool, bool)> = image
            .segments()
            .iter()
            .map(|s| (s.perms.read, s.perms.write, s.perms.execute))
            .collect();
        assert_eq!(
            perms[0],
            (true, false, true),
            "{name}: first segment is not code"
        );
        for later in &perms[1..] {
            assert!(
                *later == (true, false, false) || *later == (true, true, false),
                "{name}: unexpected segment permissions {later:?}"
            );
        }
        // `init` and `capdemo` keep everything in registers and the stack, so they
        // have no data segment of their own.
        if !matches!(
            name,
            "init.elf"
                | "capdemo.elf"
                | "ipc_server.elf"
                | "ipc_client.elf"
                | "ipc_stranger.elf"
                | "crasher.elf"
                | "notification_waiter.elf"
        ) {
            assert!(
                perms.iter().any(|&(_, write, _)| write),
                "{name}: no writable data segment"
            );
        }
        assert!(
            image
                .segments()
                .iter()
                .all(|s| !(s.perms.write && s.perms.execute)),
            "{name}: a segment is writable and executable"
        );
    }
}

/// Every page of every segment has exactly the permissions of its segment, the
/// file's bytes are in it, the bss is zero, and everything around the image
/// (the null page, the page past it, the guard page under the stack) is unmapped.
pub(super) fn elf_loader_permissions() {
    use durg_paging::PAGE_SIZE;

    let file = module("task_a.elf");
    let image = durg_elf::parse(file, &user::elf_policy()).expect("parse");
    let (space, entry) = user::build_space(file).expect("build_space failed");
    assert_eq!(entry.rip, image.entry);
    assert_eq!(entry.rsp, user::STACK_TOP - 8);

    for segment in image.segments() {
        for page in 0..segment.pages() {
            let address = segment.vaddr + page * PAGE_SIZE;
            let mapping = space
                .translate(address)
                .unwrap_or_else(|_| panic!("page {address:#x} of a segment is not mapped"));
            let raw = mapping.entry.raw();
            assert_ne!(raw & flags::USER, 0, "{address:#x}: not user-accessible");
            assert_eq!(
                raw & flags::WRITABLE != 0,
                segment.perms.write,
                "{address:#x}: wrong write permission"
            );
            assert_eq!(
                raw & flags::NO_EXECUTE == 0,
                segment.perms.execute,
                "{address:#x}: wrong execute permission"
            );
            // Contents: file bytes where the file has them, zero beyond.
            let in_segment = page * PAGE_SIZE;
            let mut page_bytes = [0u8; 64];
            frame::read_bytes(mapping.physical & !(PAGE_SIZE - 1), &mut page_bytes)
                .expect("read frame");
            for (offset, byte) in page_bytes.iter().enumerate() {
                let position = in_segment + offset as u64;
                let expected = if position < segment.filesz {
                    file[(segment.file_offset + position) as usize]
                } else {
                    0
                };
                assert_eq!(*byte, expected, "{address:#x}+{offset}: wrong contents");
            }
        }
        // The page just past the segment is not mapped by this segment (it may be
        // another segment's first page, which the parser keeps page-disjoint).
    }
    // The stack: four pages, read-write, never executable.
    for page in 1..=user::STACK_PAGES {
        let address = user::STACK_TOP - page * PAGE_SIZE;
        let raw = space.translate(address).expect("stack page").entry.raw();
        assert_ne!(raw & flags::USER, 0);
        assert_ne!(raw & flags::WRITABLE, 0, "stack is not writable");
        assert_ne!(raw & flags::NO_EXECUTE, 0, "stack is executable");
    }
    // Unmapped: the null page, a low address, the guard page below the stack, the
    // top of user space itself, and kernel addresses are not in this half at all.
    let guard = user::STACK_TOP - (user::STACK_PAGES + 1) * PAGE_SIZE;
    for (name, address) in [
        ("null page", 0),
        ("low address", 0x1000),
        ("stack guard page", guard),
        ("first byte above the stack", user::STACK_TOP),
    ] {
        assert!(
            space.translate(address).is_err(),
            "{name} ({address:#x}) is mapped"
        );
    }
    space.destroy_user_half();
}

pub(super) fn root_task_runs_from_module() {
    // The boot path: init gets a capability space and an untyped region.
    user::boot_demo().expect("the root task did not exit cleanly");
    assert_eq!(
        user_tests::last_print(),
        "init: built the system: server, client and stranger ran; the stranger reached nobody"
    );
}

/// The M9 acceptance test: task A cannot read task B's memory, nor B A's.
///
/// Both programs keep a different secret at the *same virtual address*. Three
/// layers of evidence:
///
/// 1. **Structure:** the two address spaces own disjoint sets of physical frames
///    (page tables and pages alike), and the shared virtual address maps to
///    different frames holding different secrets.
/// 2. **Behaviour:** run interleaved (each yields eight times), each program
///    checks that its own secret and a scratch area are untouched, and scans its
///    whole loaded image for the other's secret. Run again under timer
///    preemption, where the interleaving lands wherever the tick falls.
/// 3. The programs report through their exit status; any disagreement is a
///    failure.
pub(super) fn tasks_cannot_read_each_others_memory() {
    let (space_a, entry_a) = user::build_space(module("task_a.elf")).expect("build A");
    let (space_b, entry_b) = user::build_space(module("task_b.elf")).expect("build B");

    // 1. Structure.
    let frames_a = space_a.owned_frames();
    let frames_b = space_b.owned_frames();
    assert!(
        frames_a.len() > 10 && frames_b.len() > 10,
        "suspiciously few frames"
    );
    for frame_a in &frames_a {
        assert!(
            !frames_b.contains(frame_a),
            "frame {frame_a:#x} belongs to both tasks"
        );
    }
    // The same virtual address, different frames, each holding its own secret.
    let secret_address = 0x60_0000;
    let map_a = space_a.translate(secret_address).expect("A's secret page");
    let map_b = space_b.translate(secret_address).expect("B's secret page");
    assert_ne!(map_a.physical, map_b.physical, "both tasks map one frame");
    let mut secret_a = [0u8; 16];
    let mut secret_b = [0u8; 16];
    frame::read_bytes(map_a.physical, &mut secret_a).expect("read A");
    frame::read_bytes(map_b.physical, &mut secret_b).expect("read B");
    assert_eq!(&secret_a, b"SECRET-OF-TASK-A");
    assert_eq!(&secret_b, b"SECRET-OF-TASK-B");

    // 2. Behaviour, cooperative.
    let a = crate::sched::spawn_user("task-a", space_a, entry_a).expect("spawn A");
    let b = crate::sched::spawn_user("task-b", space_b, entry_b).expect("spawn B");
    let status_a = user_tests::wait(&a);
    let status_b = user_tests::wait(&b);
    assert_eq!(
        status_a,
        user_status::Exited(0),
        "task A reported a problem"
    );
    assert_eq!(
        status_b,
        user_status::Exited(0),
        "task B reported a problem"
    );

    // 2b. Behaviour under preemption: the timer decides where the switches fall.
    let a = user::spawn_module("task-a", "task_a.elf").expect("spawn A");
    let b = user::spawn_module("task-b", "task_b.elf").expect("spawn B");
    crate::sched::start_preemption();
    let status_a = user_tests::wait(&a);
    let status_b = user_tests::wait(&b);
    crate::sched::stop_preemption();
    assert_eq!(status_a, user_status::Exited(0), "task A (preempted)");
    assert_eq!(status_b, user_status::Exited(0), "task B (preempted)");
}

/// A task that fills its memory with junk, then exits, must not leave the junk
/// for the next task that is handed the same physical frames.
pub(super) fn freed_frames_leave_no_residue() {
    let (dirty_space, dirty_entry) = user::build_space(module("dirty.elf")).expect("build dirty");
    let dirty_frames = dirty_space.owned_frames();
    let dirty = crate::sched::spawn_user("dirty", dirty_space, dirty_entry).expect("spawn dirty");
    assert_eq!(user_tests::wait(&dirty), user_status::Exited(0));

    // The dirty task's frames are free again, but the probe's pages need not land
    // on the frames that held the dirty task's *bss*: the two programs allocate
    // in different orders. So also fill a block of free frames with junk and give
    // it back, which makes every page the probe gets come from dirty memory.
    // (Found by mutation: a loader that stopped zeroing segment frames survived
    // the version of this test that relied on the dirty task alone.)
    // The probe's bss must really be zero-fill (memsz > filesz), or the loader's
    // zeroing is never what makes it read as zero.
    let probe_image =
        durg_elf::parse(module("probe.elf"), &user::elf_policy()).expect("parse probe");
    assert!(
        probe_image
            .segments()
            .iter()
            .any(|s| s.memsz >= s.filesz + 3 * 4096),
        "probe has no zero-fill (bss) pages, so this test would not test the loader"
    );
    let junk = alloc::vec![0xaau8; 4096];
    let mut poisoned = alloc::vec::Vec::new();
    for _ in 0..256 {
        let physical = crate::mm::frame::alloc_frame().expect("frame for poisoning");
        crate::mm::frame::write_bytes(physical, &junk).expect("poison a frame");
        poisoned.push(physical);
    }
    for physical in &poisoned {
        crate::mm::frame::free_frame(*physical).expect("return poisoned frame");
    }
    // Build the probe's space now, and check the premise: every frame it was
    // given held junk before (the dirty task's or the poisoned block), so a
    // missing zeroing step cannot hide.
    let (probe_space, probe_entry) = user::build_space(module("probe.elf")).expect("build probe");
    let probe_frames = probe_space.owned_frames();
    let used_junk = probe_frames
        .iter()
        .filter(|frame| poisoned.contains(frame) || dirty_frames.contains(frame))
        .count();
    assert_eq!(
        used_junk,
        probe_frames.len(),
        "some frames given to the probe never held junk, so this test proves too little"
    );
    let probe = crate::sched::spawn_user("probe", probe_space, probe_entry).expect("spawn probe");
    let status = user_tests::wait(&probe);
    assert_eq!(
        status,
        user_status::Exited(0),
        "the probe found non-zero bytes (its exit code counts them) in memory it was given"
    );
}

/// The kernel maps module memory in its own tables, and nothing should be able to
/// change a boot module through that mapping or run code from it.
pub(super) fn module_window_is_read_only() {
    for (index, name) in MODULES.iter().enumerate() {
        let base = vm::MODULE_WINDOW + index as u64 * limine::MAX_MODULE_BYTES;
        let file = module(name);
        let pages = (file.len() as u64).div_ceil(4096);
        for page in [0, pages - 1] {
            let raw = paging::translate(base + page * 4096)
                .unwrap_or_else(|_| panic!("{name}: page {page} of the module is not mapped"))
                .entry
                .raw();
            assert_eq!(
                raw & flags::WRITABLE,
                0,
                "{name}: module memory is writable"
            );
            assert_ne!(
                raw & flags::NO_EXECUTE,
                0,
                "{name}: module memory is executable"
            );
            assert_eq!(
                raw & flags::USER,
                0,
                "{name}: module memory is user-accessible"
            );
        }
        // And nothing is mapped past the module's last page.
        assert!(
            paging::translate(base + pages * 4096).is_err(),
            "{name}: mapped past the end of the module"
        );
    }
}

/// A positive control for the isolation test. The two programs report "intact"
/// whenever nothing is wrong, so a check that never fires proves little. Here a
/// copy of task B's secret is planted in a page of task A's own data segment,
/// inside the range A scans, and A must notice (exit code 3).
///
/// The copy goes at offset 0x100 of the page, because task A fills its scratch
/// area (whose tail covers the first 16 bytes of that page) with its own tag
/// before it scans, which would overwrite a secret placed at the start.
pub(super) fn leak_detector_detects_a_leak() {
    use durg_paging::PAGE_SIZE;

    let (mut space_a, entry_a) = user::build_space(module("task_a.elf")).expect("build A");
    let (space_b, _) = user::build_space(module("task_b.elf")).expect("build B");
    // B's real secret, read from B's own frame.
    let b_page = space_b
        .translate(0x60_0000)
        .expect("B's secret page")
        .physical;
    let mut b_secret = [0u8; 16];
    frame::read_bytes(b_page & !(PAGE_SIZE - 1), &mut b_secret).expect("read B's secret");
    assert_eq!(&b_secret, b"SECRET-OF-TASK-B");
    space_b.destroy_user_half();

    // Replace the last page of A's data segment (bss, inside the scanned range)
    // with a fresh page that holds that secret.
    let last_page = 0x60_0000 + 2 * PAGE_SIZE;
    let removed = space_a.unmap(last_page).expect("A's last data page");
    frame::free_frame(removed.physical).expect("free A's page");
    let planted = frame::alloc_zeroed_frame().expect("no frame");
    let mut contents = [0u8; 0x110];
    contents[0x100..].copy_from_slice(&b_secret);
    frame::write_bytes(planted, &contents).expect("write the planted secret");
    space_a
        .map_page(
            last_page,
            planted,
            flags::PRESENT | flags::WRITABLE | flags::USER | paging::no_execute(),
        )
        .expect("map the planted page into A");

    let a = crate::sched::spawn_user("task-a", space_a, entry_a).expect("spawn A");
    let status = user_tests::wait(&a);
    assert_eq!(
        status,
        user_status::Exited(3),
        "task A did not notice the other task's secret in its own memory"
    );
}
