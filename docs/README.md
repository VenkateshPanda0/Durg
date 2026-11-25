# Durg documentation

Everything written about Durg lives here. If you are new, read in this order:

1. [../README.md](../README.md): what Durg is, why, and its honest status.
2. [contributor-quickstart.md](contributor-quickstart.md): **step by step** from a
   clean machine to a running boot test and a pull request.
3. [ARCHITECTURE.md](ARCHITECTURE.md): how the system works **today**: layout,
   boot, memory map, system calls, capabilities, tests.
4. [TODO.md](TODO.md): the ordered list of **what to do next**, with where and
   how to tell it is done.
5. [ROADMAP.md](ROADMAP.md): the full milestone plan (M0 to M29) and its order.

## The map

| Document | What it is for | Status |
|---|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | The system as built | current |
| [TODO.md](TODO.md) | Concrete next tasks, in order | current |
| [contributor-quickstart.md](contributor-quickstart.md) | Setup, build, test, change, pull request | current |
| [ROADMAP.md](ROADMAP.md) | Dependency-ordered plan, M0–M29 | integrated plan, 2026-11-08 |
| [THREAT_MODEL.md](THREAT_MODEL.md) | Assets, attackers, the five promises as testable properties, and the evidence so far | v0, approved; evidence notes per milestone |
| [PROJECT_RULES.md](PROJECT_RULES.md) | The working rules: identity, no AI trailers, no credentials, evidence, scope | reconstructed 2026-11-24, awaiting owner confirmation |
| [../GOVERNANCE.md](../GOVERNANCE.md) | Who decides what; how the AI assistant fits; adding maintainers | current |
| [VERSIONING.md](VERSIONING.md), [RELEASING.md](RELEASING.md) | Version rules and the release checklist | current |
| [REPRODUCIBLE_BUILDS.md](REPRODUCIBLE_BUILDS.md) | What is and is not reproducible, with evidence | current |
| [REPO_SETTINGS.md](REPO_SETTINGS.md) | GitHub settings only the owner can change | current |
| [UNSUPPORTED.md](UNSUPPORTED.md) | What does not exist today and what is never promised | aligned with ADR-0012 |
| [adr/](adr/README.md) | Architecture decision records: context, options, decision, consequences | see the index for each status |
| [book/](book/) | Design log, one chapter per milestone (M1 to M11): the problem, the hardware, the design, the bugs | chapters through M10 step 4 and M11 groundwork |
| [design/](design/) | Working designs: [M10 capability core](design/m10-capability-core.md), [desktop threat model draft](design/desktop-threat-model-draft.md), [hardware qualification plan](design/hardware-qualification-plan.md) | drafts and in-progress designs |
| [devlog/](devlog/) | Dated session notes: what was done, what was verified, what was not | append-only history |
| [reviews/](reviews/) | Gate reviews and teach-back material (M9 gate, M7–M9 questions) | historical |
| [requirements/](requirements/) | The [desktop/laptop specification](requirements/universal-desktop-laptop-v1.md) as supplied, and the [coverage register](requirements/coverage.md) mapping each requirement to a milestone | planned work, not features |
| [glossary.md](glossary.md) | Capability-system terms | placeholder, [#3](https://github.com/VenkateshPanda0/Durg/issues/3) |
| [reading-list.md](reading-list.md) | Prior art to read | placeholder, [#4](https://github.com/VenkateshPanda0/Durg/issues/4) |
| [repository-checklist.md](repository-checklist.md) | Repository presentation checklist (demo, CI, releases, templates) | maintained |
| [ROADMAP-before-desktop-expansion.md](ROADMAP-before-desktop-expansion.md) | The roadmap before the desktop expansion | historical snapshot |
| [assets/](assets/) | Demo GIF and social-preview image | n/a |

Next to the code: [kernel/README.md](../kernel/README.md),
[crates/README.md](../crates/README.md), [user/README.md](../user/README.md) and
[fuzz/README.md](../fuzz/README.md).

## Conventions

- **Claims need evidence.** A statement that something passes names the run that
  showed it; a security statement traces to [THREAT_MODEL.md](THREAT_MODEL.md).
  Historical numbers in the devlog and book are records of a past run, not the
  current state.
- **Decisions are ADRs.** A change to something an ADR decided needs a new ADR.
- **Book chapters and devlog entries** are written with each milestone step.
- **Historical references to `CLAUDE.md`** in older ADRs, reviews and devlog
  entries point to the project's original private brief, which was never part
  of the repository. The rules that survive are written down in
  [PROJECT_RULES.md](PROJECT_RULES.md) (reconstructed, awaiting the owner's
  confirmation); its decisions that still apply are in
  [ADR-0001](adr/0001-locked-decisions.md) and later ADRs.
