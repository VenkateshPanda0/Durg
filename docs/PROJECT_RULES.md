# Project rules

> **Reconstructed on 2026-11-24; adopted 2026-11-25.**
> [ADR-0010](adr/0010-delegated-project-authority.md) and a few older documents cite
> sections of a private brief, `CLAUDE.md` (§4, §9.1, §9.5, §9.7, §9.8, §10, §16).
> That file was never committed (`git log --all -- CLAUDE.md` is empty), so the rules
> could not be audited. This page writes down the rules that survive in the
> repository's own text: ADR-0010, ADR-0001, `AGENTS.md` and `CONTRIBUTING.md`. It
> adds nothing new except where a rule says otherwise. Where the original wording is
> lost, the entry says so. These rules were adopted on the owner's instruction "You have all my permission" (2026-11-25), given in reply to the list of open decisions; the assistant applied its recommendation. The owner has not reviewed the text line by line, so this is delegated, not independently reviewed.

## 1. Identity and history

1. **Authorship.** Commits are authored and committed only as the owner,
   `Venkatesh Panda <venkateshpanda00@gmail.com>`. Some commits (32, up to 2026-11-22) used the
   display name `VenkateshPanda0` with the same email; both are the owner. Tools do
   not change the git configuration; they pass the identity per command.
   *Source: ADR-0010 decision 3 (cited there as CLAUDE.md §9.7).*
2. **No AI attribution trailers.** No `Co-Authored-By`, "Generated with" or session
   lines in commit messages. The AI-assisted nature of the project is stated openly
   in [CONTRIBUTING.md](../CONTRIBUTING.md) and [GOVERNANCE.md](../GOVERNANCE.md)
   instead. *Source: ADR-0010 decision 3.*
3. **Sign-off.** A `Signed-off-by:` line certifies the Developer Certificate of Origin.
   Until 2026-11-25 none was added on the owner's behalf. The owner has since chosen to
   keep the DCO ([ADR-0013](adr/0013-owner-resumes-authority.md)), so commits made for
   the owner from 2026-11-25 on carry `Signed-off-by: Venkatesh Panda <venkateshpanda00@gmail.com>`;
   older commits are not changed. *Source: ADR-0010 decision 3 (CLAUDE.md §9.7), updated.*
4. **Published history is not rewritten.** No force-push, rebase of pushed commits,
   amend of pushed commits, tag moves or branch deletion without the owner's say-so.
   *Source: ADR-0010 decision 3; `AGENTS.md`.*

## 2. Credentials and money

5. **No credentials, tokens, accounts or paid services** are created, handled or
   used by contributors or tools acting for the project. A host setting that needs
   a login is written down for the owner to change
   ([REPO_SETTINGS.md](REPO_SETTINGS.md)).
   *Source: ADR-0010 decision 3 and 6 (CLAUDE.md §16).*

## 3. Evidence

6. **Nothing is claimed as built, booted, passing or secure unless it was run and the
   output seen.** Observed results are kept apart from historical documentation.
   A security statement traces to the [threat model](THREAT_MODEL.md).
   *Source: ADR-0010 decision 3; `AGENTS.md`; CONTRIBUTING "Claims must be honest".*
7. **A milestone is signed off only when** its acceptance criteria pass on a hosted
   runner **with the log read**, negative controls were run on the new tests, the
   threat model's evidence and "does not show" notes are updated, and the book
   chapter and devlog exist. *Source: ADR-0010 decision 2.*
8. **Fuzzing** covers the system-call interface and other parsers from M8 on, on host
   (ADR-0008). *Source: ADR-0008 (CLAUDE.md §9.5).*

## 4. Scope

9. **Non-goals stay unless an ADR with a real reason changes one.** The original
   non-goal list (CLAUDE.md §4) is not in the repository. What is recorded is
   [UNSUPPORTED.md](UNSUPPORTED.md) ("Not promised": no formal-verification claim, no
   "more secure than Linux" without evidence, no universal hardware or application
   compatibility) and the foundations in
   [ADR-0001](adr/0001-locked-decisions.md) as narrowed by
   [ADR-0012](adr/0012-desktop-laptop-roadmap.md).
10. **Decisions are ADRs.** Changing something an ADR decided needs a new ADR that
    supersedes it.
11. **Originality.** Third-party crates in guest code (kernel, services, user
    programs) need an ADR each; host tooling is exempt (ADR-0001).

## 5. Working practice

12. **Session start** (`AGENTS.md`): check the tree, fetch, and fast-forward if behind;
    never delete the repository or force-push; if fetching fails, say that freshness
    could not be verified.
13. **Docs move with code.** When a change finishes or adds a task, `docs/TODO.md` is
    updated in the same commit.
14. **Every `unsafe` block carries a `// SAFETY:` argument** (CONTRIBUTING).

## 6. What was not recoverable

- CLAUDE.md §9.1 and §10 ("an owner who decides and signs off", the month-6 gate
  agenda: hours sustained, outside interest, continue or fall back) survive only as
  described in ADR-0010 and [the M9 gate review](reviews/m09-decision-gate.md).
- §11 ("open decisions", item 6, the capability ABI) and the "memory policy" in §7
  survive as ADR-0009 and [ADR-0011](adr/0011-spawner-capability-as-an-interim-step.md)
  (the kernel does not allocate on behalf of user tasks after boot).
- The MPL-2.0 licence line is superseded by Apache-2.0 (ADR-0001).
