# Support

Durg is pre-alpha research software built and maintained by one person with an AI
assistant. There is no support service and no guarantee of an answer, but questions
and reports are welcome.

| You want to... | Do this |
|---|---|
| Build or run it | Start with [docs/contributor-quickstart.md](docs/contributor-quickstart.md); its last section lists common failures |
| Report a build, boot or test failure | Open a **bug report** issue. Include the commit, boot variant, build profile, QEMU version, whether `cargo xtask` said KVM or TCG, and the serial log |
| Ask how something works | Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and [docs/README.md](docs/README.md), then open an issue tagged `question` |
| Propose a change | Open a **design proposal** issue first for anything larger than a small fix |
| Report a vulnerability | **Not** a public issue: use [private vulnerability reporting](https://github.com/VenkateshPanda0/Durg/security/advisories/new) and read [SECURITY.md](SECURITY.md) |

Only the latest `main` and the latest pre-release are looked at. Durg is not ready to
protect real workloads, so do not depend on it for anything that matters.
