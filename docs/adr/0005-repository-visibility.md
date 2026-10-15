# ADR-0005: Repository visibility

- **Status:** Proposed (awaiting owner's decision)
- **Needed by:** M1

## Context

The repository can be public from the start or made public at a later milestone. Public early means design feedback and early contributors, at the cost of exposing unfinished work. Public later means a more polished first impression, but fewer chances for outside review of the foundations. Public repositories also get free CI minutes on GitHub standard runners.

## Options

1. **Public from day one.** Design, threat model and issues are open now.
2. **Private until M9** (the first kernel proof of concept).
3. **Public now, but without promotion** until there is a demo.

## Recommendation

**Option 3.** Keeping the repository public gets design review and free CI. Delay any launch posts until there is something to boot, so early attention is not spent on a project with nothing to try yet.

## Consequences

To be filled in once decided. Current state, checked on 2026-10-15: the repository is already **public**, and GitHub detects the license as Apache-2.0. Option 3 therefore matches reality, and accepting it needs no change to the repository.
