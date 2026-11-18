# Getting started: from a clean machine to a pull request

Follow these steps in order. Each one says what to run and what you should see.
You do not need to know kernel development to contribute; for documentation-only
changes you can stop after step 2 and skip QEMU entirely.

Before you write code, skim [ARCHITECTURE.md](ARCHITECTURE.md) (how the system
works today) and [TODO.md](TODO.md) (what to work on next).

## Step 1. Get a Linux environment

Durg builds and boots on Linux. Use one of:

- **Ubuntu or Debian** (a VM is fine).
- **Windows:** Ubuntu on **WSL2**. Work inside the WSL filesystem (for example
  `~/src/Durg`), not under `/mnt/c`, which is much slower.
- **macOS:** use a Linux VM; the build tool expects Linux paths for QEMU and OVMF.

KVM makes the boot tests faster but is not required: without `/dev/kvm` the build
tool falls back to software emulation automatically.

## Step 2. Get the code

```bash
git clone https://github.com/VenkateshPanda0/Durg.git
cd Durg
```

To contribute, fork the repository on GitHub first and clone your fork instead.

## Step 3. Install the tools

**Option A, one command** (Debian/Ubuntu/WSL; needs `sudo` and internet):

```bash
bash scripts/setup.sh
```

It installs the system packages and `rustup` if missing, then runs step 5.

**Option B, by hand:**

```bash
sudo apt-get install -y build-essential ca-certificates curl git \
    qemu-system-x86 xorriso ovmf
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
source "$HOME/.cargo/env"
```

You do not install a Rust version yourself: `rust-toolchain.toml` pins Rust
1.97.0 with the bare-metal target, `rustfmt` and `clippy`, and `cargo` fetches it
the first time you build. The first build also clones the pinned Limine
bootloader into `target/limine/`, so it needs internet access once.

## Step 4. Build

```bash
cargo xtask build          # the kernel only (add --release for optimized)
cargo xtask iso            # also builds the user programs and makes the bootable image
```

`cargo xtask` is an alias for the build tool in `xtask/`. `run` and `test` call
`iso` themselves, so you rarely need these two directly.

## Step 5. Run the tests

```bash
cargo xtask test           # six boot variants, debug build
cargo xtask test --release # the same, optimized
```

Expected: a line `selftest: all 113 tests passed` (the number grows as tests are
added) and, at the end, `boot test: PASS (6 variants)`. Two variants **fail on
purpose** (`heap-oom` and `fail-demo`) to prove the harness can catch a failure;
the tool counts them as passing when they fail the right way. Each variant is
described in [ARCHITECTURE.md §3](ARCHITECTURE.md#3-how-a-build-becomes-a-boot).

## Step 6. Watch it boot

```bash
cargo xtask run
```

This boots the normal kernel and prints the serial log, then exits. Look for:
`Durg kernel: boot ok`, the memory map, `VM: kernel page tables active`,
`timer: 5 ticks counted`, and the root task's lines ending with
`init: built the system: server, client and stranger ran; the stranger reached nobody`.
There is no interactive shell yet.

## Step 7. Find your way around

| You want to change | Look in | Then read |
|---|---|---|
| Capability rules (derive, revoke, retype) | `crates/cap/` | [book/m10-capability-core.md](book/m10-capability-core.md) |
| Other pure logic (paging, frames, heap, ELF, syscall decoding, virtio) | `crates/<name>/` | [crates/README.md](../crates/README.md) |
| Kernel code | `kernel/src/` | [kernel/README.md](../kernel/README.md) |
| User programs and the root task | `user/src/bin/` | [user/README.md](../user/README.md) |
| How things are built and booted | `xtask/src/main.rs` | [ARCHITECTURE.md §3](ARCHITECTURE.md#3-how-a-build-becomes-a-boot) |
| Security assumptions and what is shown | `docs/THREAT_MODEL.md` | the property's "evidence" and "does not show" notes |
| Why a decision was made | `docs/adr/` | [ADR index](adr/README.md) |

## Step 8. Make one focused change

1. Pick a task from [TODO.md](TODO.md) or an issue labelled
   [good first issue](https://github.com/VenkateshPanda0/Durg/labels/good%20first%20issue);
   say on the issue that you are taking it.
2. Create a branch: `git switch -c short-description`.
3. Keep the change to one logical step. Pure logic goes in a crate in `crates/`
   with host tests; kernel code keeps every `unsafe` block small, with a
   `// SAFETY:` comment.
4. Add a test that would fail if your change were wrong. For a fix, check that
   the test fails *before* the fix.

## Step 9. Run the same checks as CI

Documentation only: read the rendered Markdown and check the links you changed.
Rust changes: run what applies, or everything below (it is the CI matrix in
`.github/workflows/ci.yml`):

```bash
cargo fmt --all -- --check
cargo test --workspace --exclude durg-kernel --exclude durg-user
cargo clippy --workspace --exclude durg-kernel --exclude durg-user --all-targets -- -D warnings
cargo clippy -p durg-user --target x86_64-unknown-none -- -D warnings
cargo clippy -p durg-virtio --target x86_64-unknown-none -- -D warnings
for feature in '' selftest fail-demo df-test so-test oom-test; do
  cargo clippy -p durg-kernel --target x86_64-unknown-none --features "$feature" -- -D warnings || break
done
cargo xtask test
cargo xtask test --release
```

**Run clippy again after your last edit.** A warning introduced by the final
change is the most common way to turn CI red. Optional fuzzing needs a nightly
toolchain; see [fuzz/README.md](../fuzz/README.md).

## Step 10. Open the pull request

1. `git push` your branch to your fork and open a pull request against `main`.
2. Fill in the template: what changed, **which checks you actually ran** and their
   result, and anything you could not run. Do not describe a check you did not
   run as passing.
3. Link the issue (`Fixes #NUMBER`). If you used an AI tool, say so
   ([ADR-0004](adr/0004-ai-assisted-contributions.md)).
4. Update [TODO.md](TODO.md) if your change finishes or adds a task.

Sign-off and the full workflow are in [CONTRIBUTING.md](../CONTRIBUTING.md).
Report a suspected vulnerability privately, as described in
[SECURITY.md](../SECURITY.md).

## If something goes wrong

| Symptom | Likely cause and fix |
|---|---|
| `OVMF firmware not found (install the ovmf package)` | Install the `ovmf` package. |
| `could not start xorriso … (is it installed?)` | Install `xorriso`. |
| The first build hangs or fails while fetching | It is downloading Rust or Limine: check network access and retry. |
| `KVM: permission denied` or QEMU refuses `-enable-kvm` | The tool only uses KVM if `/dev/kvm` is usable; add yourself to the `kvm` group, or ignore it (software emulation works, slower). |
| A variant times out | Software emulation on a slow machine: retry; if it persists, open an issue with the serial output. |
| Clippy passes for you but CI fails | You ran clippy before your last edit, or not for every feature set: run the full loop in step 9. |

Still stuck? Open an issue with your OS, the command and its full output. A
report of a failed clean build is one of the most useful contributions.
