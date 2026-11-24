## What changes and why?

## Validation
List the commands you actually ran and their results; say which checks you did not run.

- [ ] I ran the gate commands that apply (fmt; every clippy invocation in `.github/workflows/ci.yml` with `-D warnings`; host tests; `cargo xtask test` and `cargo xtask test --release` for kernel changes) and read their output
- [ ] I ran clippy again after my last edit

## Records
- [ ] `docs/TODO.md` and `CHANGELOG.md` are updated if this finishes or adds a task
- [ ] A decision was made: there is an ADR (or an ADR is not needed because no decided ADR changes)
- [ ] Every `unsafe` block I added or changed has a `// SAFETY:` comment stating the invariant

## Security and scope
Does this affect authority, isolation, memory safety or the threat model? Say how, and
whether `docs/THREAT_MODEL.md` needs an evidence note or a "does not show" note.

- [ ] Threat-model impact considered (none / updated)

## AI assistance
- [ ] I disclosed AI assistance below (or I used none)

Describe how the change was checked, and whether an AI tool wrote any of it.
