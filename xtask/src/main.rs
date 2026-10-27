//! # `cargo xtask`: build orchestration for Durg
//!
//! Build steps are written in Rust instead of shell scripts so they behave the
//! same on every host and are easy to read.
//!
//! ```text
//! cargo xtask build [--release]   build the kernel ELF
//! cargo xtask iso   [--release]   build a bootable UEFI ISO (fetches Limine)
//! cargo xtask run   [--release]   boot the ISO in QEMU, serial on this terminal
//! cargo xtask test  [--release]   boot every variant headless; check serial output + exit codes
//! ```

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{exit, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// Limine release whose prebuilt binaries we boot with. Pinned so builds repeat.
const LIMINE_TAG: &str = "v10.8.5-binary";
const LIMINE_REPO: &str = "https://github.com/limine-bootloader/limine.git";

/// Kernel target triple.
const TARGET: &str = "x86_64-unknown-none";

/// A boot test fails if QEMU has not exited after this long (a hang is a failure).
const TEST_TIMEOUT: Duration = Duration::from_secs(120);

/// QEMU exit status for the kernel's "success" debug-exit value `0x10`:
/// `(0x10 << 1) | 1`.
const QEMU_SUCCESS_STATUS: i32 = 33;

/// QEMU exit status for the kernel's "failure" debug-exit value `0x11`.
const QEMU_FAILURE_STATUS: i32 = 35;

/// One way of building and booting the kernel, and what must be true after.
struct Variant {
    name: &'static str,
    /// Cargo features of `durg-kernel` to build with.
    features: &'static [&'static str],
    expected_status: i32,
    /// Every one of these strings must appear in the serial output.
    must_contain: &'static [&'static str],
}

/// The variant used by `run` and `iso`: a plain boot with no self-tests.
const DEFAULT_VARIANT: Variant = Variant {
    name: "default",
    features: &[],
    expected_status: QEMU_SUCCESS_STATUS,
    must_contain: &[
        "Durg kernel: boot ok",
        "COM1 16550 loopback self-test passed",
        "cpu: GDT, TSS and IDT loaded",
        "Limine base revision: 4 loaded",
        "CPU: local APIC yes",
        "Memory map (",
        "Usable RAM:",
        "HHDM offset:",
        "RSDP (virtual):",
        "MADT: local APIC at",
        "I/O APIC id",
        "PMM: ",
        "frames (",
        "PMM: allocate/free round trip ok",
        "VM: kernel page tables active",
        "VM: kernel stack ",
        "unmapped",
        "PMM: reclaimed",
        "VM: map/write/read/unmap round trip ok",
        "LAPIC: id",
        "mapped at 0xffffff0000000000 (uncached)",
        "timer: 5 ticks counted",
    ],
};

/// The variants `test` boots. The last one fails on purpose, and passing it
/// means the harness really reports failures.
const TEST_VARIANTS: [Variant; 6] = [
    DEFAULT_VARIANT,
    Variant {
        name: "selftest",
        features: &["selftest"],
        expected_status: QEMU_SUCCESS_STATUS,
        must_contain: &["selftest: running", "selftest: all", "tests passed"],
    },
    // A double fault on purpose. Passing means the CPU delivered it on the
    // dedicated IST stack and our handler recognised that, then exited with
    // the success code.
    Variant {
        name: "double-fault",
        features: &["df-test"],
        expected_status: QEMU_SUCCESS_STATUS,
        must_contain: &[
            "df-test: triggering a double fault",
            "EXCEPTION: Double Fault (vector 8)",
            "inside the IST1 stack",
            "double fault handled on the IST stack",
        ],
    },
    // The kernel stack is overflowed on purpose. Passing means the guard page
    // turned it into a fault, the double fault handler ran on its own stack, and
    // the handler traced it to the guard page.
    Variant {
        name: "stack-overflow",
        features: &["so-test"],
        expected_status: QEMU_SUCCESS_STATUS,
        must_contain: &[
            "so-test: overflowing the kernel stack",
            "EXCEPTION: Double Fault (vector 8)",
            "kernel stack overflow detected",
            "double fault handled on the IST stack",
        ],
    },
    // The heap is run out of memory on purpose. A fallible request must fail
    // cleanly and leave the heap intact, and an infallible one must end in the
    // allocation-failure panic: exit status "failure" with that message.
    Variant {
        name: "heap-oom",
        features: &["oom-test"],
        expected_status: QEMU_FAILURE_STATUS,
        must_contain: &[
            "oom-test: try_reserve failed gracefully",
            "oom-test: heap intact after the failed request",
            "PANIC:",
            "memory allocation of 1073741824 bytes failed",
        ],
    },
    Variant {
        name: "fail-demo",
        features: &["fail-demo"],
        expected_status: QEMU_FAILURE_STATUS,
        must_contain: &[
            "test deliberate_failure ... ",
            "PANIC:",
            "this failure is deliberate",
        ],
    },
];

type Result<T> = std::result::Result<T, String>;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    // A misspelt flag must not silently fall back to a debug build.
    if let Some(bad) = args.iter().skip(1).find(|a| a.as_str() != "--release") {
        eprintln!("error: unknown argument {bad:?}");
        eprintln!("usage: cargo xtask <build|iso|run|test> [--release]");
        exit(2);
    }
    let release = args.iter().any(|a| a == "--release");
    let result = match args.first().map(String::as_str) {
        Some("build") => build(release, &[]).map(|_| ()),
        Some("iso") => iso(release, &DEFAULT_VARIANT).map(|_| ()),
        Some("run") => run(release),
        Some("test") => test(release),
        _ => {
            eprintln!("usage: cargo xtask <build|iso|run|test> [--release]");
            exit(2);
        }
    };
    if let Err(message) = result {
        eprintln!("error: {message}");
        exit(1);
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
        .to_path_buf()
}

fn profile(release: bool) -> &'static str {
    if release {
        "release"
    } else {
        "debug"
    }
}

/// Runs a command, failing with a readable message if it does not succeed.
fn check(cmd: &mut Command, what: &str) -> Result<()> {
    let status = cmd
        .status()
        .map_err(|e| format!("could not start {what}: {e} (is it installed?)"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed with {status}"))
    }
}

/// Builds the kernel with the given features and returns the path of the ELF.
fn build(release: bool, features: &[&str]) -> Result<PathBuf> {
    let mut cmd = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(root())
        .args(["build", "--package", "durg-kernel", "--target", TARGET]);
    if release {
        cmd.arg("--release");
    }
    if !features.is_empty() {
        cmd.arg("--features").arg(features.join(","));
    }
    check(&mut cmd, "cargo build")?;
    let elf = root()
        .join("target")
        .join(TARGET)
        .join(profile(release))
        .join("durg-kernel");
    if elf.exists() {
        Ok(elf)
    } else {
        Err(format!("expected kernel at {}", elf.display()))
    }
}

/// The user programs in the `user/` crate, packed into the image as boot modules
/// named `<program>.elf`.
const USER_PROGRAMS: [&str; 9] = [
    "init",
    "task_a",
    "task_b",
    "dirty",
    "probe",
    "capdemo",
    "ipc_server",
    "ipc_client",
    "ipc_stranger",
];

/// Builds the user programs and returns the directory holding their ELF files.
fn build_user(release: bool) -> Result<PathBuf> {
    let mut cmd = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(root())
        .args(["build", "--package", "durg-user", "--target", TARGET]);
    if release {
        cmd.arg("--release");
    }
    check(&mut cmd, "cargo build (user programs)")?;
    let dir = root().join("target").join(TARGET).join(profile(release));
    for program in USER_PROGRAMS {
        if !dir.join(program).exists() {
            return Err(format!(
                "expected user program at {}",
                dir.join(program).display()
            ));
        }
    }
    Ok(dir)
}

/// Fetches the pinned Limine binaries once and returns their directory.
fn limine() -> Result<PathBuf> {
    let dir = root().join("target").join("limine");
    // Record which tag the cache holds, so bumping LIMINE_TAG refetches
    // instead of silently booting stale binaries.
    let stamp = dir.join(".durg-limine-tag");
    let cached = fs::read_to_string(&stamp).unwrap_or_default();
    if dir.join("BOOTX64.EFI").exists() && cached.trim() == LIMINE_TAG {
        return Ok(dir);
    }
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    check(
        Command::new("git")
            .args(["-c", "advice.detachedHead=false"])
            .args([
                "clone",
                "--quiet",
                "--depth",
                "1",
                "--branch",
                LIMINE_TAG,
                LIMINE_REPO,
            ])
            .arg(&dir),
        "git clone of Limine",
    )?;
    fs::write(&stamp, LIMINE_TAG).map_err(|e| format!("write {}: {e}", stamp.display()))?;
    Ok(dir)
}

/// Builds the bootable ISO for `variant` and returns its path.
fn iso(release: bool, variant: &Variant) -> Result<PathBuf> {
    let kernel = build(release, variant.features)?;
    let user_programs = build_user(release)?;
    let limine = limine()?;
    let target = root().join("target");
    let iso_root = target.join(format!("iso_root-{}", variant.name));
    if iso_root.exists() {
        fs::remove_dir_all(&iso_root).map_err(|e| e.to_string())?;
    }
    for sub in ["boot/limine", "boot/user", "EFI/BOOT"] {
        fs::create_dir_all(iso_root.join(sub)).map_err(|e| e.to_string())?;
    }
    let copy = |from: &Path, to: PathBuf| -> Result<()> {
        fs::copy(from, &to)
            .map(|_| ())
            .map_err(|e| format!("copy {} -> {}: {e}", from.display(), to.display()))
    };
    copy(&kernel, iso_root.join("boot/durg-kernel"))?;
    for program in USER_PROGRAMS {
        copy(
            &user_programs.join(program),
            iso_root.join(format!("boot/user/{program}.elf")),
        )?;
    }
    copy(
        &root().join("boot/limine.conf"),
        iso_root.join("boot/limine/limine.conf"),
    )?;
    copy(
        &limine.join("limine-uefi-cd.bin"),
        iso_root.join("boot/limine/limine-uefi-cd.bin"),
    )?;
    copy(
        &limine.join("BOOTX64.EFI"),
        iso_root.join("EFI/BOOT/BOOTX64.EFI"),
    )?;

    let out = target.join(format!("durg-{}.iso", variant.name));
    let output = Command::new("xorriso")
        .args(["-as", "mkisofs", "-R", "-r", "-J"])
        .args(["--efi-boot", "boot/limine/limine-uefi-cd.bin"])
        .args([
            "-efi-boot-part",
            "--efi-boot-image",
            "--protective-msdos-label",
        ])
        .arg(&iso_root)
        .arg("-o")
        .arg(&out)
        .output()
        .map_err(|e| format!("could not start xorriso: {e} (is it installed?)"))?;
    if !output.status.success() {
        // Show why it failed instead of hiding it.
        return Err(format!(
            "xorriso failed with {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(out)
}

/// Finds UEFI firmware (OVMF) code and vars files.
fn ovmf() -> Result<(PathBuf, PathBuf)> {
    let candidates = [
        (
            "/usr/share/OVMF/OVMF_CODE_4M.fd",
            "/usr/share/OVMF/OVMF_VARS_4M.fd",
        ),
        (
            "/usr/share/OVMF/OVMF_CODE.fd",
            "/usr/share/OVMF/OVMF_VARS.fd",
        ),
        (
            "/usr/share/edk2/ovmf/OVMF_CODE.fd",
            "/usr/share/edk2/ovmf/OVMF_VARS.fd",
        ),
        (
            "/usr/share/qemu/OVMF_CODE.fd",
            "/usr/share/qemu/OVMF_VARS.fd",
        ),
    ];
    for (code, vars) in candidates {
        if Path::new(code).exists() && Path::new(vars).exists() {
            return Ok((code.into(), vars.into()));
        }
    }
    Err("OVMF firmware not found (install the `ovmf` package)".into())
}

/// Returns `true` if `/dev/kvm` exists and this user may open it read-write.
fn kvm_usable() -> bool {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok()
}

/// Builds the QEMU command line shared by `run` and `test`.
fn qemu(iso: &Path) -> Result<Command> {
    let (code, vars) = ovmf()?;
    // OVMF writes to its vars file, so each run gets a private copy.
    let vars_copy = root().join("target").join("ovmf_vars.fd");
    // Drop any previous copy first, and make the new one writable: if the
    // system file is read-only, `fs::copy` would carry that over and the next
    // run could neither overwrite the copy nor let OVMF write to it.
    let _ = fs::remove_file(&vars_copy);
    fs::copy(&vars, &vars_copy).map_err(|e| format!("copy OVMF vars: {e}"))?;
    let mut perms = fs::metadata(&vars_copy)
        .map_err(|e| format!("stat OVMF vars copy: {e}"))?
        .permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(&vars_copy, perms).map_err(|e| format!("chmod OVMF vars copy: {e}"))?;

    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.args(["-machine", "q35", "-m", "256M"]);
    // Use KVM only when this user can actually open it. The device can exist
    // yet be unusable (for example on hosted CI runners), and asking QEMU for
    // KVM then makes it refuse to start. Otherwise use software emulation.
    if kvm_usable() {
        cmd.args(["-enable-kvm", "-cpu", "host"]);
    } else {
        // The plain QEMU CPU plus the three protection features the kernel
        // enables when present. (`-cpu max` would also add x2APIC, which changes
        // how the interrupt controller is set up.)
        cmd.args(["-cpu", "qemu64,+smep,+smap,+umip"]);
    }
    cmd.args(["-display", "none", "-serial", "stdio", "-no-reboot"])
        .args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"])
        .arg("-drive")
        .arg(format!(
            "if=pflash,format=raw,readonly=on,file={}",
            code.display()
        ))
        .arg("-drive")
        .arg(format!("if=pflash,format=raw,file={}", vars_copy.display()))
        .arg("-cdrom")
        .arg(iso);
    Ok(cmd)
}

/// Boots the ISO interactively. Success is not judged here.
fn run(release: bool) -> Result<()> {
    let iso = iso(release, &DEFAULT_VARIANT)?;
    let status = qemu(&iso)?
        .status()
        .map_err(|e| format!("could not start QEMU: {e} (is qemu-system-x86 installed?)"))?;
    println!("QEMU exited with {status}");
    Ok(())
}

/// Removes terminal escape sequences and carriage returns so the serial log
/// reads as plain text. The firmware and bootloader draw with cursor codes.
fn plain_text(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => {
                // Skip up to and including the sequence's final letter.
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            '\r' => {}
            other => out.push(other),
        }
    }
    out
}

/// Boots every test variant. Each must reach its expected exit status and print
/// everything it is required to print. A hang past [`TEST_TIMEOUT`] fails.
fn test(release: bool) -> Result<()> {
    let mut failures = Vec::new();
    for variant in &TEST_VARIANTS {
        println!("=== variant: {} ===", variant.name);
        match test_variant(release, variant) {
            Ok(()) => println!("variant {}: PASS\n", variant.name),
            Err(message) => {
                println!("variant {}: FAIL ({message})\n", variant.name);
                failures.push(format!("{}: {message}", variant.name));
            }
        }
    }
    if failures.is_empty() {
        println!("boot test: PASS ({} variants)", TEST_VARIANTS.len());
        Ok(())
    } else {
        Err(format!(
            "{} variant(s) failed: {}",
            failures.len(),
            failures.join("; ")
        ))
    }
}

fn test_variant(release: bool, variant: &Variant) -> Result<()> {
    let iso = iso(release, variant)?;
    let mut child = qemu(&iso)?
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start QEMU: {e} (is qemu-system-x86 installed?)"))?;

    let mut stdout = child.stdout.take().expect("stdout was piped");
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    });

    let start = Instant::now();
    let status: ExitStatus = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                // Do not leave QEMU running if we can no longer poll it.
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
        if start.elapsed() > TEST_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            let serial = plain_text(&reader.join().unwrap_or_default());
            println!("--- serial output ---\n{serial}---------------------");
            return Err(format!(
                "timed out after {TEST_TIMEOUT:?} (a hang counts as failure)"
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let serial = plain_text(&reader.join().unwrap_or_default());
    println!("--- serial output ---\n{serial}---------------------");
    println!("QEMU exit status: {:?}", status.code());

    for needle in variant.must_contain {
        if !serial.contains(needle) {
            return Err(format!("serial output did not contain {needle:?}"));
        }
    }
    if status.code() != Some(variant.expected_status) {
        return Err(format!(
            "expected QEMU exit status {}, got {:?}",
            variant.expected_status,
            status.code()
        ));
    }
    Ok(())
}
