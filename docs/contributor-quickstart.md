# Contributor quickstart

Durg is a pre-alpha research kernel. Contributions to documentation, testing,
Rust, security review and design are welcome; you do not need to be an OS expert.

## Choose a first task

- [Good first issues](https://github.com/VenkateshPanda0/Durg/labels/good%20first%20issue): small starting points, including documentation and reading-list work.
- [Help wanted](https://github.com/VenkateshPanda0/Durg/labels/help%20wanted): design and security questions that benefit from outside perspectives.
- [Discussions](https://github.com/VenkateshPanda0/Durg/discussions): ask questions or share ideas before committing to a change.

Read the issue and check recent comments before starting. If the scope is unclear,
ask on the issue. For a new large idea, open an issue first; for a focused fix,
a draft pull request is welcome. See [CONTRIBUTING](../CONTRIBUTING.md) for the
full workflow and sign-off requirements.

## Find your way around

| Area | Start here |
|---|---|
| Why the project exists and what is implemented | [README](../README.md), [roadmap](ROADMAP.md) |
| Security assumptions and evidence | [Threat model](THREAT_MODEL.md), [M10 design](design/m10-capability-core.md) |
| Teaching chapters | [Book chapters](book/) |
| Pure Rust logic and host tests | `crates/`, especially `crates/cap` |
| Kernel and syscall implementation | `kernel/src/` |
| Root task and example programs | `user/src/bin/` |
| Build and QEMU harness | `xtask/src/main.rs` |

## Run the checks relevant to your change

On Ubuntu/Debian or WSL, `bash scripts/setup.sh` installs prerequisites and runs
six boot tests. This requires network access and `sudo`. Rust is selected by
`rust-toolchain.toml`. For documentation-only work, read the rendered Markdown
and verify changed links; QEMU is not necessary.

```bash
cargo fmt --all -- --check
cargo test -p durg-cap                  # Example: capability logic
cargo clippy -p durg-cap --all-targets -- -D warnings
cargo xtask test                       # Six debug QEMU variants
cargo xtask test --release             # Six optimized QEMU variants
```

GitHub CI runs the complete matrix listed in `.github/workflows/ci.yml`.
If a check is unavailable, say so in your pull request with its error; don't
claim a test passed unless you observed it. For short fuzz runs, see
[fuzz/README.md](../fuzz/README.md). Report suspected sensitive vulnerabilities
privately as described in [SECURITY](../SECURITY.md).
