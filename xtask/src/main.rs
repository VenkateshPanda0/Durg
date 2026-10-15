//! # `cargo xtask`: build orchestration for Durg
//!
//! Build steps are written in Rust instead of shell scripts so they behave the
//! same on every host and are easy to read.
//!
//! ```text
//! cargo xtask build [--release]   build the kernel ELF
//! cargo xtask iso   [--release]   build a bootable UEFI ISO (fetches Limine)
//! cargo xtask run   [--release]   boot the ISO in QEMU, serial on this terminal
//! cargo xtask test  [--release]   boot headless and check serial output + exit code
//! ```

use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{exit, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// Limine release whose prebuilt binaries we boot with. Pinned so builds repeat.
const LIMINE_TAG: &str = "v9.6.7-binary";
const LIMINE_REPO: &str = "https://github.com/limine-bootloader/limine.git";

/// Kernel target triple.
const TARGET: &str = "x86_64-unknown-none";

/// A boot test fails if QEMU has not exited after this long (a hang is a failure).
const TEST_TIMEOUT: Duration = Duration::from_secs(120);

/// What the kernel prints on success (keep in sync with `kernel/src/main.rs`).
const BOOT_MARKER: &str = "Durg kernel: boot ok (M1)";

/// QEMU exit status for the kernel's "success" debug-exit value `0x10`:
/// `(0x10 << 1) | 1`.
const QEMU_SUCCESS_STATUS: i32 = 33;

type Result<T> = std::result::Result<T, String>;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let release = args.iter().any(|a| a == "--release");
    let result = match args.first().map(String::as_str) {
        Some("build") => build(release).map(|_| ()),
        Some("iso") => iso(release).map(|_| ()),
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

/// Builds the kernel and returns the path of the ELF.
fn build(release: bool) -> Result<PathBuf> {
    let mut cmd = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.current_dir(root())
        .args(["build", "--package", "durg-kernel", "--target", TARGET]);
    if release {
        cmd.arg("--release");
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

/// Fetches the pinned Limine binaries once and returns their directory.
fn limine() -> Result<PathBuf> {
    let dir = root().join("target").join("limine");
    if dir.join("BOOTX64.EFI").exists() {
        return Ok(dir);
    }
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    check(
        Command::new("git")
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
    Ok(dir)
}

/// Builds the bootable ISO and returns its path.
fn iso(release: bool) -> Result<PathBuf> {
    let kernel = build(release)?;
    let limine = limine()?;
    let target = root().join("target");
    let iso_root = target.join("iso_root");
    if iso_root.exists() {
        fs::remove_dir_all(&iso_root).map_err(|e| e.to_string())?;
    }
    for sub in ["boot/limine", "EFI/BOOT"] {
        fs::create_dir_all(iso_root.join(sub)).map_err(|e| e.to_string())?;
    }
    let copy = |from: &Path, to: PathBuf| -> Result<()> {
        fs::copy(from, &to)
            .map(|_| ())
            .map_err(|e| format!("copy {} -> {}: {e}", from.display(), to.display()))
    };
    copy(&kernel, iso_root.join("boot/durg-kernel"))?;
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

    let out = target.join("durg.iso");
    check(
        Command::new("xorriso")
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
            .stdout(Stdio::null())
            .stderr(Stdio::null()),
        "xorriso",
    )?;
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
    fs::copy(&vars, &vars_copy).map_err(|e| format!("copy OVMF vars: {e}"))?;

    let mut cmd = Command::new("qemu-system-x86_64");
    cmd.args(["-machine", "q35", "-m", "256M"]);
    // Use KVM only when this user can actually open it. The device can exist
    // yet be unusable (for example on hosted CI runners), and asking QEMU for
    // KVM then makes it refuse to start. Otherwise use software emulation.
    if kvm_usable() {
        cmd.args(["-enable-kvm", "-cpu", "host"]);
    } else {
        cmd.args(["-cpu", "qemu64"]);
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
    let iso = iso(release)?;
    let status = qemu(&iso)?
        .status()
        .map_err(|e| format!("could not start QEMU: {e} (is qemu-system-x86 installed?)"))?;
    println!("QEMU exited with {status}");
    Ok(())
}

/// Boots headless and passes only if the marker appears and the status is the
/// kernel's success code. A hang past [`TEST_TIMEOUT`] fails.
fn test(release: bool) -> Result<()> {
    let iso = iso(release)?;
    let mut child = qemu(&iso)?
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start QEMU: {e} (is qemu-system-x86 installed?)"))?;

    let mut stdout = child.stdout.take().expect("stdout was piped");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        text.push_str(&String::from_utf8_lossy(&bytes));
        text
    });

    let start = Instant::now();
    let status: ExitStatus = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if start.elapsed() > TEST_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            let serial = reader.join().unwrap_or_default();
            println!("--- serial output ---\n{serial}---------------------");
            return Err(format!(
                "boot test timed out after {TEST_TIMEOUT:?} (hang counts as failure)"
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let serial = reader.join().unwrap_or_default();
    println!("--- serial output ---\n{serial}---------------------");
    println!("QEMU exit status: {:?}", status.code());

    if !serial.contains(BOOT_MARKER) {
        return Err(format!("serial output did not contain {BOOT_MARKER:?}"));
    }
    if status.code() != Some(QEMU_SUCCESS_STATUS) {
        return Err(format!(
            "expected QEMU exit status {QEMU_SUCCESS_STATUS}, got {:?}",
            status.code()
        ));
    }
    println!("boot test: PASS");
    Ok(())
}
