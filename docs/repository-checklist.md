# Repository presentation checklist

Adapted from the owner's reference video. Durg is a pre-alpha research kernel.

| Item | Status |
|---|---|
| Demo GIF | README includes animated excerpts of observed QEMU output |
| One-line description | README describes the current research scope |
| One-command setup | `bash scripts/setup.sh` for Debian/Ubuntu/WSL; shell syntax checked; underlying install and boot commands were run separately |
| Live demo | Local `cargo xtask run`; no hosted live demo |
| License | Apache-2.0 already present |
| Topics | Suggested: rust, operating-system, microkernel, capability-security, qemu, ai-agents; requires GitHub settings access |
| Social preview | `docs/assets/social-preview.png`, 1280 x 640; upload through repository settings |
| CI | Workflow and README badge present; hosted CI passed for b7a2dc4 (run 42395658407), including debug and release boots |
| Secrets in history | Gitleaks v8.28.0 scanned 137 commits on 2026-11-07 with no findings; detection is not exhaustive; no history rewrite performed |
| Good first issues | Candidate: add a reproducible timer-timeout regression test; scope and label through GitHub after review |
| Contributing guide | Existing guide retained |
| Issue templates | Bug and design templates added, plus PR template |
| Discussions | Requires authenticated GitHub settings access |
| Release/changelog | Changelog added; hosted CI verified; publishing a pre-alpha release requires authenticated GitHub access |
| Show HN | External publication, not part of these repository changes |
| Respond within a day | Maintainer practice, not an automated promise |

GitHub CLI was not authenticated during this pass. Repository settings and a
published GitHub release have not been changed. No security audit is claimed.
