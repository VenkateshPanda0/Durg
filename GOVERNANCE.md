# Governance

Durg is a pre-alpha research project. This page says who decides what. It is short
on purpose and changes only through an [ADR](docs/adr/README.md).

## Who is in charge

**Venkatesh Panda ([@VenkateshPanda0](https://github.com/VenkateshPanda0)) is the
maintainer of record**, a benevolent-dictator model ([ADR-0013](docs/adr/0013-owner-resumes-authority.md),
which supersedes [ADR-0010](docs/adr/0010-delegated-project-authority.md)). The
owner decides scope and non-goals, what is merged, when and what is released, and
how security reports are handled. See [MAINTAINERS.md](MAINTAINERS.md).

## How decisions are made

- **Significant decisions are ADRs** (context, options, decision, consequences) in
  `docs/adr/`. Changing a decided ADR needs a new ADR that supersedes it. Open
  questions stay `Proposed` until the owner accepts them; they are never decided
  silently.
- **Everything else** is decided in a pull request: one focused change, the checks
  listed in the PR template run and reported honestly, `docs/TODO.md` and
  `CHANGELOG.md` updated in the same change.
- **Security decisions** (disclosure, fixes, advisories, what the threat model claims)
  sit with the owner. Claims need a test or a written argument in the
  [threat model](docs/THREAT_MODEL.md).

## How an AI assistant fits

Durg is built substantially with an AI coding assistant, and says so. The assistant
works under [PROJECT_RULES.md](docs/PROJECT_RULES.md) and `AGENTS.md`: it proposes,
the owner decides; it does not push without the owner's yes, does not touch
credentials, accounts or paid services, does not rewrite published history, and does
not claim anything it did not run. Commits are authored as the owner and carry no
AI trailers; the assistant's role is disclosed here and in
[CONTRIBUTING.md](CONTRIBUTING.md) rather than in every commit. Nothing is
independently reviewed by a second human yet, and the project does not claim it is.

## Adding maintainers

[ADR-0001](docs/adr/0001-locked-decisions.md) says: owner-led, moving to a maintainer
group at three or more regular contributors. In practice: someone who has landed
several reviewed changes in an area and shows good judgment on `unsafe`, capability
or security-relevant code may be invited by the owner to own that area in
[CODEOWNERS](.github/CODEOWNERS). Maintainers are added by an ADR that names them and
their area. Inactive maintainers are moved to an emeritus list after twelve months.

## Conduct, support and disclosure

[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md), [SUPPORT.md](SUPPORT.md) and
[SECURITY.md](SECURITY.md) apply.
