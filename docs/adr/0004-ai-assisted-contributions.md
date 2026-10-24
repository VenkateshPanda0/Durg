# ADR-0004: Policy on AI-assisted contributions

- **Status:** Accepted, option 1 (2026-10-23, decided by the assistant under [ADR-0010](0010-delegated-project-authority.md))
- **Needed by:** M0 (referenced from `CONTRIBUTING.md`)

## Context

Contributors increasingly use AI coding tools. Some operating-system projects have banned LLM-generated contributions, citing review burden, licensing uncertainty and quality. Others allow them with disclosure. Durg needs a stated position so contributors know what is expected, and so the position applies consistently. Considerations specific to this project: a security-focused kernel where every `unsafe` block must be justified, a strict originality policy, and DCO sign-off, which certifies the contributor's right to submit the code.

## Options

1. **Allowed, with disclosure.** Contributors may use AI tools but must disclose it in the pull request. They remain fully responsible for correctness, licensing and understanding every line, and must be able to explain it in review.
2. **Allowed for docs and tests, not for kernel code.** AI-assisted contributions are fine for documentation and test scaffolding. Kernel and `unsafe` code must be human-written.
3. **Not accepted.** No AI-generated code or text. Simplest to state, hard to verify.
4. **No special policy.** The DCO and normal review apply to every contribution regardless of tooling.

## Decision

**Option 1: allowed, with disclosure.** This project is itself built substantially by an AI assistant, so banning AI-assisted contributions would be inconsistent, and option 3 cannot be verified anyway. Option 2 is rejected because the code that most needs scrutiny is the kernel, and the rule that matters there applies to everyone: the contributor must understand and be able to defend every line, every `unsafe` block needs a `// SAFETY:` argument, and tests must show the claim.

## Consequences

`CONTRIBUTING.md` states the policy: disclose AI assistance in the pull request, take full responsibility for correctness and licensing, and be able to explain the change in review. Reviewers may decline changes the author cannot explain.
