# GitHub settings the owner must change

These settings live in the GitHub web interface. Files in the repository cannot change
them, and the project's rules ([PROJECT_RULES.md](PROJECT_RULES.md) §2) keep the tools
that work on the repository away from accounts and credentials, so **only the owner
does these**. Each entry says where to click and why. Tick them off as you go.

## 1. A ruleset on `main` (Settings, Rules, Rulesets, New ruleset, New branch ruleset)

- [ ] **Name** `main`; **Enforcement status** Active; **Target branches**: Include default branch.
- [ ] **Restrict deletions** on.
- [ ] **Block force pushes** on.
- [ ] **Require linear history** on.
- [ ] **Require a pull request before merging** on (0 required approvals while there is
  one maintainer; raise it when there are two). If you want to keep pushing straight
  to `main` yourself, add yourself (Repository admin role) to **Bypass list** with
  "Always"; everyone else then needs a pull request.
- [ ] **Require status checks to pass**: add the single check **`ci-ok`** (it appears in
  the list after the first CI run on a pull request or push). Optionally tick "Require
  branches to be up to date before merging".
- [ ] **Require signed commits**: optional. Commits are currently not signed; turning this
  on blocks unsigned pushes, including your own, until commit signing is set up.

The `reproducible`, `codeql` and `boot-test-kvm` checks are informational on purpose and
should **not** be added as required checks. The **`dco`** check (workflow `DCO`, pull
requests only) may be added as a second required check once pull requests are in use.

## 2. Security features (Settings, Advanced Security / Code security)

- [ ] **Private vulnerability reporting**: Enable (the link in SECURITY.md and the
  issue-form config depend on it).
- [ ] **Dependency graph**: Enable.
- [ ] **Dependabot alerts**: Enable. **Dependabot security updates**: Enable.
  (`.github/dependabot.yml` already schedules weekly version updates; the first run
  opens pull requests for cargo and workflow-action updates. Review them like any
  other change; the project's CI covers them.)
- [ ] **Secret scanning** and **Push protection**: Enable.
- [ ] **Code scanning**: leave "Default setup" **off**. `.github/workflows/codeql.yml` is an
  advanced setup, and the two conflict. Read the first run of that workflow; if the
  Rust analysis fails, see `docs/repository-checklist.md` and remove the workflow.

## 3. Actions (Settings, Actions, General)

- [ ] **Workflow permissions**: "Read repository contents and packages permissions";
  leave "Allow GitHub Actions to create and approve pull requests" **off**. Jobs that
  need more ask for it themselves.
- [ ] **Fork pull request workflows**: "Require approval for all outside collaborators"
  (or at least first-time contributors).
- [ ] **Actions permissions**: allow GitHub's own actions and the few third-party ones
  used (`Swatinem/rust-cache`, `EmbarkStudios/cargo-deny-action`,
  `ossf/scorecard-action`), and tick **Require actions to be pinned to a full-length
  commit SHA** if your plan shows it. Every workflow already pins by SHA.

## 4. General (Settings, General)

- [ ] **Description**: "A pre-alpha Rust capability microkernel for isolating untrusted
  AI-agent tools. Research software; not a production security boundary."
- [ ] **Topics**: `rust`, `operating-system`, `microkernel`, `capabilities`, `security`,
  `sandbox`, `ai-agents`, `x86-64`, `qemu`, `os-development`.
- [ ] **Social preview**: upload `docs/assets/social-preview.png`.
- [ ] **Pull requests**: allow squash merging only (matches linear history); tick
  "Automatically delete head branches".
- [ ] **Discussions**: your choice. Leave **off** until you can answer them; the issue
  forms and SUPPORT.md are the support path today.
- [ ] **Releases**: nothing to do until the first release ([RELEASING.md](RELEASING.md)).

## 5. After the first runs

- [ ] Open the Scorecard result (Security, Code scanning) and the badge on the README; it
  shows a score after `scorecard.yml` has run once.
- [ ] Close or merge the Dependabot pull requests it opened.
