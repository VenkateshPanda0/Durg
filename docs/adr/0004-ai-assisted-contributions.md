# ADR-0004: Policy on AI-assisted contributions

- **Status:** Proposed (awaiting owner's decision)
- **Needed by:** M0 (referenced from `CONTRIBUTING.md`)

## Context

Contributors increasingly use AI coding tools. Some operating-system projects have banned LLM-generated contributions, citing review burden, licensing uncertainty and quality. Others allow them with disclosure. Durg needs a stated position so contributors know what is expected, and so the position applies consistently. Considerations specific to this project: a security-focused kernel where every `unsafe` block must be justified, a strict originality policy, and DCO sign-off, which certifies the contributor's right to submit the code.

## Options

1. **Allowed, with disclosure.** Contributors may use AI tools but must disclose it in the pull request. They remain fully responsible for correctness, licensing and understanding every line, and must be able to explain it in review.
2. **Allowed for docs and tests, not for kernel code.** AI-assisted contributions are fine for documentation and test scaffolding. Kernel and `unsafe` code must be human-written.
3. **Not accepted.** No AI-generated code or text. Simplest to state, hard to verify.
4. **No special policy.** The DCO and normal review apply to every contribution regardless of tooling.

## Recommendation

None yet. This is a values and risk decision for the owner. The practical differences are enforceability (options 1 and 4 rely on honesty, option 3 is hard to check) and how the project wants to treat reviewer time.

## Consequences

Until this is decided, `CONTRIBUTING.md` says only that a policy is pending and to ask in an issue.
