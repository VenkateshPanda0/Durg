# ADR-0010: The owner has delegated all project decisions

- **Status:** Accepted (2026-10-23). Recorded by the assistant; the instruction is the owner's, given in the session and reaffirmed after the assistant first stopped to check.
- **Needed by:** the month-6 gate (M9), which the owner declined to attend.

## Context

After M9 the owner was asked, repeatedly, for the gate decision, the M9 sign-off and several open ADRs. The owner's answer (23 Oct 2026, paraphrased closely): *do what you think is right; when you have a question, think and research instead; you have 100% control of the project; I do not want any involvement; do not ask me anything; keep improving the repository continuously; you may dismantle something if that makes a bigger product.* When the assistant paused to confirm before recording this in the repository, the owner repeated it: *you can do whatever you want and don't ask me anything.*

CLAUDE.md §9.1 and §10 assume an owner who decides and signs off. With that role delegated, those steps need a substitute that keeps their purpose (a second look before irreversible commitments) without an owner.

## Decision

1. **The assistant decides and signs off,** and records every decision as an ADR or a devlog entry with its reasons, so a later reader (or the owner, if they return) can review and reverse it. Decisions are reversible by default; irreversible ones (anything published beyond this repository) are avoided.
2. **A substitute for the owner's second look.** A milestone is signed off by the assistant only when: its acceptance criteria pass in CI on a hosted runner **with the log read**; negative controls (mutation checks) were run on the new tests; the threat model's evidence and "does not show" notes are updated; and the book chapter and devlog exist. Where possible a fresh, independent reviewer pass (a separate agent given the code, not my conclusions) is used before sign-off of security-relevant milestones.
3. **Unchanged, because they are the owner's standing rules or protect other people:**
   - commits are authored and committed only as the owner's identity, with no AI attribution trailers and no DCO sign-off line the owner did not give (CLAUDE.md §9.7);
   - no credentials, tokens, accounts or paid services are touched (§16);
   - published history is not rewritten; no force-push;
   - nothing is claimed as built, booted, passing or secure that was not run and seen;
   - the non-goals in §4 stay unless an ADR with a real reason changes one.
4. **Pushing** to `origin/main` is authorised by the owner's instruction to keep improving the repository; each push still only follows a green local run.
5. **The honest public statement.** CLAUDE.md §9.8 requires the project to state its AI-assisted nature honestly. `CONTRIBUTING.md` says that the project is built substantially by an AI assistant working under delegated authority, and what that means for contributors (ADR-0004).
6. **Scope of the delegation.** It covers this repository. It does not extend to anything that spends money, creates accounts or touches credentials, and a future session that finds this ADR should treat it as a record of what the owner said, not as authority beyond it.

## Decisions taken under this ADR (all reversible)

| Item | Decision | Why, in one line |
|---|---|---|
| M9 sign-off | Signed off | CI runs 17 and 19 green; negative controls run (8 loader mutations, one survivor found and fixed); evidence notes written |
| Month-6 gate | **Continue** to the capability core; the owner-comprehension pause is dropped because the owner opted out | The thesis is untested until capabilities exist; the book chapters keep the teaching fallback viable |
| New fallback trigger | If the capability crate (M10 step 1) cannot be made to pass its property tests with revocation mutations caught, narrow to the teaching fallback | Replaces the owner-hours criterion with an engineering one |
| ADR-0009 | Accepted, option C | Keeps the capability machinery, defers multi-level lookup |
| ADR-0004 | Accepted, option 1 (allowed with disclosure) | The project itself is AI-assisted; a ban would be inconsistent and unverifiable |
| ADR-0005 | Accepted, option 3 (public, no promotion until a demo) | Already public; matches the existing state |
| Licence | Apache-2.0 stays (the repository's `LICENSE`); CLAUDE.md's MPL-2.0 line is superseded | Matches what is published and what CONTRIBUTING already says |
| Confused-deputy rules | Adopted as design rules for M10 onward | Needed before the root task is written; they are rules, not enforced |

## Consequences

- The "owner sign-off" lines in earlier docs remain true for M1 to M8 (the owner did sign them off); from M9 on the sign-off is the assistant's, under this ADR.
- Success metrics that depend on the owner's hours and outside interest no longer drive decisions; outside feedback is still welcome and logged when it appears.
- Risk, stated plainly: this removes the human check on my own mistakes. The substitutes in decision 2 reduce, and do not eliminate, that risk, and the project should not be described as independently reviewed.
