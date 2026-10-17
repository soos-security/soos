# Audit Constraints — GitHub #408: README Restructure as a Product Landing Page (PR 2 of #356)

- **Date**: 2026-10-17
- **Branch**: `chore/readme-redesign`
- **Inputs**: `AI/architect_spec_readme_redesign.md`, `AI/plan_evaluator_report.md`, tester contract
  `tests/invariants/src/readme_contract.rs` (RDM1–RDM8; red run: RDM5, RDM6, RDM7, RDM8 fail on the current
  README for the expected reasons, the helper self-tests RDM2 and RDM4 and the vacuous RDM1 and RDM3 pass).

## Checklist results

| # | Item | Result |
|---|---|---|
| 1 | Panic paths | Only test code changes (`#![allow]` for assertions, as every invariant module). No runtime crate touched. |
| 2 | Unsafe | None. |
| 3 | Output isolation | No production output; the test prints nothing. |
| 4 | Bounded I/O | The test reads `README.md` and `rust-toolchain.toml`, runs one `git tag --list` and writes a scratch tree under `target/readme_contract/` only. |
| 5 | Process hygiene | `git` runs in the workspace root with every `GIT_*` variable removed (hook-safe); a failing or missing `git` means "no tag" (stricter). |
| 6 | Filesystem | Link resolution refuses absolute paths and any path that climbs above the repository root, so a README link cannot point outside the checkout. |
| 7 | Secrets & privacy | No new image; the README shows only the three synthetic PNGs covered by RAC11–RAC13. The owner brand pack (contains the owner's face) is not read, copied or referenced. No real host name, login, tailnet name or e-mail address in the README. |
| 8 | Truthfulness (security property) | The README must not overstate PAD (single-scale MiniFASNetV2, #212, no independent certification), must say the password stays available and remains the root of trust, and must link the remote residual limitations. |
| 9 | Supply chain | No dependency added (std only); no workflow, script, packaging or asset file edited. |
| 10 | Existing contracts | Quick Start, Upgrade and License text moved verbatim; no existing test edited. |

## Audit Constraints

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| AC-1 | No existing test assertion edited; `lib.rs` only gains the `mod readme_contract;` registration | `tests/invariants/src/*` | `git diff origin/main -- tests/` review |
| AC-2 | Quick Start keeps every pinned needle; the `setup.sh` paragraph and three lines of context on each side are byte-identical to `origin/main` (open PR #402) | `README.md` `## Quick Start` | `onboarding_packaging_contract`, `installer_contract`, `install_gdm_followups_contract`, `setup_contract`, diff review |
| AC-3 | No text before the Quick Start contains `./scripts/setup.sh` or `sudo ./scripts/install.sh --build` | `README.md` | `setup_contract` SET17 |
| AC-4 | Every new `soos-admin` / `soos-enroll` command shown names a real subcommand | `README.md` | `onboarding_packaging_contract` (#207) |
| AC-5 | Images only from `Docs/assets/` with alt text, badges only `img.shields.io` or this repository's workflow `badge.svg`; at most 6 badges; no release badge | `README.md` | RAC10, RDM3 |
| AC-6 | No banned word (`enterprise-grade`, `military-grade`, `unbreakable`, `certified`); no accuracy number for soos, Howdy or Windows Hello; every comparison cell sourced | `README.md` | RDM5, RDM8, review |
| AC-7 | No logo image, no `SECURITY.md` / `CONTRIBUTING.md`, no repository-settings change | repository | diff review |
| AC-8 | Every relative link resolves inside the repository | `README.md` | RDM1 |
| AC-9 | Security section names `PAM_IGNORE`, `/etc/soos/disabled`, `#212` and links `AI/ARCHITECTURE.md` and `Docs/REMOTE_COMPANION.md` | `README.md` | RDM7 |

Clearance: CLEARED
