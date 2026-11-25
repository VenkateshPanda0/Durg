# ADR-0013: The owner resumes authority

- **Status:** Proposed (2026-11-24), awaiting the owner's acceptance. Supersedes [ADR-0010](0010-delegated-project-authority.md) once accepted.
- **Needed by:** presenting the project to anyone who must know who is accountable for it.

## Context

[ADR-0010](0010-delegated-project-authority.md) records that on 23 October 2026 the owner delegated every project decision to an AI assistant and asked not to be involved. That was accurate and workable for a research prototype. It leaves the project with no human maintainer of record: decisions in ADR-0004, 0005, 0008, 0009 and 0011, the M9 sign-off and the month-6 gate were taken by the assistant. A company, a contributor or a security reviewer would reasonably ask who is accountable. ADR-0010 also cites a private brief (`CLAUDE.md`) that is not in the repository.

On 2026-11-24 the owner ended the delegation: from that point the assistant proposes and the owner decides, and the owner is asked before any irreversible action, any change to scope or non-goals, and every push.

## Options

1. **Owner resumes authority; the assistant keeps implementing under written rules** (this ADR). One accountable person, decisions visible as ADRs, the assistant's work reviewed before it is published.
2. **Keep ADR-0010.** Simple, but the accountability gap stays.
3. **Add a second human maintainer first.** Better long term (see [GOVERNANCE.md](../../GOVERNANCE.md), "Adding maintainers"), not available today.

## Decision

**Option 1.**

1. The owner, Venkatesh Panda, is the **maintainer of record**. They decide scope, non-goals, releases and the handling of security reports.
2. An AI assistant may do most of the implementation, under [PROJECT_RULES.md](../PROJECT_RULES.md) and `AGENTS.md`. It proposes; the owner decides. It asks the owner before every push to the shared repository, before any irreversible action and before any change to scope or non-goals. The standing push authorisation in ADR-0010 decision 4 and any automation built on it (a scheduled daily-push task existed and is disabled) ends.
3. **Decisions taken under ADR-0010 stay in force until the owner reviews them.** They are listed below with a column for the owner's call (ratify, revisit, or reverse). Until a row is marked, the decision stands and carries no human review; the documents say so.
4. ADR-0010 is marked "Superseded by ADR-0013" and kept: history matters.
5. The honest public statement stays: the project is built substantially with an AI assistant and is not independently reviewed. [CONTRIBUTING.md](../../CONTRIBUTING.md) and [GOVERNANCE.md](../../GOVERNANCE.md) say it, now with a human maintainer named.

## Decisions taken under ADR-0010 (for the owner to ratify or revisit)

Owner's call: write **ratify**, **revisit** (with a note) or **reverse**. Blank means not yet reviewed.

| # | Decision | Where | Taken | Owner's call |
|---|---|---|---|---|
| 1 | AI-assisted contributions allowed with disclosure (option 1) | [ADR-0004](0004-ai-assisted-contributions.md) | 2026-10-23 | |
| 2 | Repository public, no promotion until a demo (option 3) | [ADR-0005](0005-repository-visibility.md) | 2026-10-23 | |
| 3 | Nightly Rust for host-side fuzzing only | [ADR-0008](0008-nightly-for-host-side-fuzzing.md) | 2026-10-19 | |
| 4 | Capability object list and syscall shape, option C (no multi-level lookup yet) | [ADR-0009](0009-capability-object-model-and-syscall-shape.md) | 2026-10-23 | |
| 5 | M9 signed off | [gate review](../reviews/m09-decision-gate.md), [devlog](../devlog/2026-10-15.md) | 2026-10-24 | |
| 6 | Month-6 gate: continue to the capability core; owner-comprehension pause dropped; new engineering-based fallback trigger | [gate review](../reviews/m09-decision-gate.md), ADR-0010 | 2026-10-23 | |
| 7 | Apache-2.0 stays; the brief's MPL-2.0 line superseded | ADR-0010, [ADR-0001](0001-locked-decisions.md) | 2026-10-23 | |
| 8 | Confused-deputy design rules adopted for M10 onward | ADR-0010, [THREAT_MODEL §8](../THREAT_MODEL.md#8-open-questions-for-the-owner) | 2026-10-23 | |
| 9 | Interim spawner capability and `join` | [ADR-0011](0011-spawner-capability-as-an-interim-step.md) | 2026-11-06 | |
| 10 | Desktop/laptop scope expansion (this followed the owner's request; the scope decision itself and platform choice remain open) | [ADR-0012](0012-desktop-laptop-roadmap.md) | 2026-11-08 | |
| 11 | M10 object lifecycle rules (ownership, teardown order, cancellation, limits) | [design doc §10](../design/m10-capability-core.md) | 2026-11-21 | |
| 12 | A task can operate on a second CNode; `move_from` (invoke label 10) | [design doc §10](../design/m10-capability-core.md), [ARCHITECTURE](../ARCHITECTURE.md), [devlog](../devlog/2026-11-22.md) | 2026-11-22 | |
| 13 | Documentation restructure: ARCHITECTURE, TODO order (M10 first, M18 in parallel) | [devlog](../devlog/2026-11-18.md) | 2026-11-18 | |
| 14 | Sign-offs of M1 to M8 were the owner's own, and are not in this table | [devlog](../devlog/2026-10-15.md) | 2026-10-15 to 19 | n/a |

## Open choice: the DCO (for the owner to pick)

[ADR-0001](0001-locked-decisions.md) and [CONTRIBUTING.md](../../CONTRIBUTING.md) require a Developer Certificate of Origin sign-off (`git commit -s`). In practice 2 of 166 commits carry one and nothing enforces it. ADR-0010 forbade adding a sign-off the owner had not given.

- **(a) Keep the DCO, practise it and enforce it.** The owner signs off commits from now on; a CI check on pull requests rejects commits without `Signed-off-by`. Old commits are not backfilled. A company accepting outside contributions usually wants this (or a CLA), because it records each contributor's certification of their right to submit.
- **(b) Drop the DCO.** Apache-2.0 section 5 already licenses contributions inbound on the same terms as outbound. Simpler for casual contributors; no per-commit certification. CONTRIBUTING and ADR-0001 are updated to say so.
- **Recommendation: (a).** The project wants outside review and contributions, and the cost is one flag per commit plus a small workflow. Until the owner picks, nothing is enforced and CONTRIBUTING is unchanged on this point.

## Consequences

- The project can name an accountable human. "Not independently reviewed" stays true: owner review is not an independent audit.
- Rows 1 to 13 are a review backlog, not a blocker. Nothing is reversed by this ADR.
- Automated or scheduled pushes are not allowed. Every push waits for the owner's yes.
- The reconstructed [PROJECT_RULES.md](../PROJECT_RULES.md) must be confirmed by the owner; until then it is labelled as reconstructed.
