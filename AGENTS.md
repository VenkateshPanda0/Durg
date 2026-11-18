# Repository session rules

Before beginning work in each session, check the working tree and fetch the latest GitHub state. Compare the current branch with its upstream. If behind, update this same folder, using a fast-forward when possible. Preserve local changes and commits; resolve divergence deliberately without deleting the repository or force-pushing. If fetching fails, report that freshness could not be verified.

Read CONTRIBUTING.md, docs/ARCHITECTURE.md (the system as built), docs/TODO.md (the ordered next tasks) and the relevant architecture decisions. When a change finishes or adds a task, update docs/TODO.md in the same commit. Validate changes with the appropriate checks, and distinguish observed test results from historical documentation. Never describe an unrun build or test as passing.
