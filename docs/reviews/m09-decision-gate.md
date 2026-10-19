# Month-6 decision gate: review after M9

**Status:** prepared for the owner. **The decision is the owner's.** This document sets out the evidence, what it does and does not show, and a recommendation. It makes no sign-off.

CLAUDE.md §10 asks for three things at this gate: hours actually sustained, interest from anyone outside, and a choice between continuing to the capability work (Concept 1) or narrowing to the teaching-OS fallback. The first two are facts only the owner has; the sections below mark them **[owner to fill in]**.

## 1. What exists (verified, with the evidence behind each line)

| Item | State | Evidence |
|---|---|---|
| M0 to M8 | signed off by the owner | CI green, runs 11 to 15 (read from job logs) |
| M9 (ELF loader, isolated tasks) | implemented, **not signed off** | local runs: 88 self-tests, 6 boot variants, debug and release. CI run 16 **failed** (a test assumed a fixed entry address; fixed in `6a27f98`). Run 17 on the fix: see the devlog for its result |
| Kernel source | about 9,900 lines | `wc` over `kernel/src` |
| Host-tested crates | about 4,200 lines (including tests) | `crates/` |
| `unsafe` | 97 `unsafe {` blocks in the kernel (94 at M8), every one with a `// SAFETY:` comment | counted by search; the "percentage of kernel" metric in §12 is a block count, not a measured share of lines |
| Docs | book chapters m01 to m09, 9 ADRs (2 Proposed, plus ADR-0009 added now), threat model v0 with evidence notes, a devlog | `docs/` |
| Tests | host unit and property tests, QEMU self-tests, mutation checks, libFuzzer targets (host only, not in CI) | see the devlog for each run |

## 2. What the evidence does not show

- **No core promise exists.** P1 to P5 (no ambient authority, labels, journal and undo, approval capability, brokered egress) are design intent. M9 is evidence for one supporting property, kernel integrity (P6), under QEMU software emulation only.
- **Hardware and KVM are untested.** Everything ran under QEMU TCG.
- **CI found a bug local runs could not.** Run 16's failure came from a toolchain-dependent layout. The toolchain is `stable` and not pinned to an exact version (an open item), so a new stable release can change what CI builds.
- **The pace is not a measure of owner effort.** The repository has 100 commits on four calendar days (15, 16, 18 and 19 October). That is assistant-driven throughput. CLAUDE.md estimated M0 to M9 at roughly 20 to 38 weeks at 8 to 10 hours a week; the elapsed calendar time was far less. The estimate-versus-actual comparison in §12 is therefore meaningless as a sign of how sustainable the project is for one student around coursework.

## 3. The risk I would put first: understanding

About 15,000 lines of kernel and support code, and nine chapters, now exist. The project's value as a portfolio and as the teaching fallback depends on the owner being able to explain it: why the syscall entry stub switches stacks the way it does, why `nomem` was wrong on `stac`, what the W^X mapping does. If the owner cannot, interviews and credibility suffer, and later milestones (the capability core is the hard, subtle part) will stall. This is a risk I created by working fast, and I want to flag it plainly.

## 4. Options

**A. Continue to M10 (capabilities and the agent thesis).** This is where the project's claim begins; everything so far is the foundation. Cost: the hardest design work yet; revocation bugs are subtle.

**B. Narrow to the teaching-capability-OS fallback now.** Keep M1 to M9 as the book, polish it, publish. Lowest risk, but it drops the thesis that makes the project distinct.

**C. Continue, but with a comprehension pause first (recommended).** Before M10 code:
1. The owner reads the M7 to M9 chapters and answers a short set of "why" questions with me (I will write them; they are teach-back, not a test with a pass mark).
2. Decide ADR-0009 (capability shape) and threat-model question 5 (confused-deputy rules) on paper.
3. Fix the open housekeeping: pin an exact toolchain, decide ADR-0004 (AI-contribution policy) and ADR-0005 (visibility), and settle MPL-2.0 versus Apache-2.0 in CLAUDE.md (the repository's `LICENSE` is Apache-2.0).
Then start M10 with the pure `durg-cap` crate and its property tests, kernel integration after.

## 5. Recommendation and what would change it

Recommend **C**. The thesis is still worth testing and the fallback stays viable either way, because the book chapters already exist.

I would recommend **B** instead if the owner's real weekly hours are well below 8, if the comprehension pause shows the material is not landing, or if no one outside the project ever looks at it and that discourages continuing. I would also want a hard stop in M10: if the capability crate and its property tests are not working within about 6 to 8 weeks of the owner's real time, narrow to B.

## 6. For the owner to fill in

- Hours actually sustained per week since 15 October: **[owner to fill in]**
- Anyone outside who has built it, opened an issue, or commented (the §12 signals): **[owner to fill in]**
- Choice: A, B or C: **[owner to fill in]**
- M9 sign-off (after CI run 17 is read): **[owner to fill in]**
