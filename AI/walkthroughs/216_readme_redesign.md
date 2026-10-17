# Walkthrough 216 — README Restructure as a Product Landing Page

- **Date**: 2026-10-17
- **Issue**: GitHub #408 (PR 2 of epic #356; no backlog id, branch not registered in `scripts/sync_issue.py`) — **Branch**: `chore/readme-redesign`
- **Base commit**: `9e82795`
- **Matrix criteria**: RDM1–RDM11 (component `readme-redesign`)

## 1. Context & Objectives

Epic #356 asks for a README that explains within one screen what soos does, why it is safe and how to install
it, while every claim stays verifiable. PR 1 (#392, closes #381) added the synthetic GUI screenshots, the
asset manifest and the image contract. This PR rewrites the README text and structure on those visuals.

Before: 172 lines of install text that opened with "enterprise-grade", with no badge, image, diagram, feature
overview or security summary.

The owner questions of #356 have no answer yet (no comment on the issue). As in PR 1, the recommended options
are assumed and listed for confirmation (ADR 2026-10-17 item 6): Q1 tagline, Q4 comparison, Q5 static
roadmap, Q6 no `SECURITY.md` / `CONTRIBUTING.md`, Q7 repository settings proposed only, Q8 no logo image.

## 2. Architect Design

Spec `AI/architect_spec_readme_redesign.md`:
- **Hero**: plain-text title and tagline, five badges (CI, Fuzz, License, Rust 1.98.1, Platform Linux), the
  synthetic live-diagnostic PNG with a caption naming the mock models and the source commit `a3cae16` (#392).
  No logo: no brand generator is allowed in the manifest and the owner brand pack contains the owner's face.
- **New sections**: Why soos, Features (merged features only, each with its doc), Screenshots, How it works
  (Mermaid flowchart of `pam_soos.so` → Unix socket → `soos-daemon` → encrypted store, plus logind,
  `soos-remote` and `soos-push-sender`), Security model, Comparison, FAQ (7 items), Roadmap (#352–#363),
  Documentation, Contributing and security.
- **Moved verbatim**: Quick Start, Upgrade and License. The only change inside the Quick Start is a
  `<details>` wrapper around the long installer paragraph. The `setup.sh` paragraph and its context stay
  byte-identical, so the open PR #402 (which edits that paragraph) rebases cleanly.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md` (self-check, `VALIDATION_VERDICT: APPROVED`). It verified every fact the README
states against the repository and upstream sources:
- no tag or release; workflow files; toolchain pin; screenshot commit; epics open; daemon unit
  `PrivateNetwork=yes`; AES-256-GCM templates; IPC framing; single-scale PAD and #212;
- Howdy README and `howdy/src/pam/main.cc` (default branch `master`, MIT);
- Microsoft Learn pages on face authentication and Enhanced Sign-in Security.

Two MINOR findings were resolved in the test design: git hook environment and shallow clones without tags.

## 4. Tester Contract

New module `tests/invariants/src/readme_contract.rs` (std only), registered in `tests/invariants/src/lib.rs`:

| Row | Test |
|---|---|
| RDM1 | `test_rdm_readme_relative_links_resolve` |
| RDM2 | `test_rdm_link_parser_and_resolver` |
| RDM3 | `test_rdm_readme_badges_are_bounded_and_real` |
| RDM4 | `test_rdm_badge_and_word_helpers` |
| RDM5 | `test_rdm_readme_has_no_banned_words` |
| RDM6 | `test_rdm_readme_sections_in_order_with_mermaid` |
| RDM7 | `test_rdm_security_section_states_fallback_kill_switch_and_pad_caveat` |
| RDM8 | `test_rdm_comparison_cites_upstream_sources` |
| RDM10 | `test_rdm_companion_wording_matches_setup_defaults` |
| RDM11 | `test_rdm_companion_wording_helpers` |

Red run on the old README: RDM5 failed on "enterprise-grade"; RDM6 and RDM7 failed with no `Why` heading;
RDM8 failed with no Comparison section. RDM1 and RDM3 passed (the old README had no broken link and no badge),
and so did the helper self-tests RDM2 and RDM4.

RDM10 and RDM11 were added after the rebase onto #402 (review round of 2026-10-17): `scripts/setup.sh` now
writes `allow_funnel`, `allow_unlock`, `camera_view` and `camera_view_funnel` = `true`, so the README may no
longer call remote unlock or the camera view opt-in. Red run before the wording fix: RDM10 failed on six text
units (the Why bullet, the companion row, two Mermaid labels, the security bullet "Remote unlock is off by
default" and the FAQ camera answer, which also named only the Tailscale network); RDM11 passed.

## 5. Auditor Constraints

`AI/auditor_constraints_readme_redesign.md`, AC-1..AC-9, `Clearance: CLEARED`. Key constraints:
- no existing assertion edited;
- the Quick Start keeps every pinned needle, and no text before it names `./scripts/setup.sh` or
  `sudo ./scripts/install.sh --build` (SET17 order);
- every shown `soos-admin` / `soos-enroll` command is real (#207);
- images and badges as allowed by RAC10;
- no banned word, no accuracy number, every comparison cell sourced;
- no logo, no `SECURITY.md` / `CONTRIBUTING.md`.

## 6. Implementation

- `README.md`: new hero and sections around the verbatim Quick Start, Upgrade and License blocks. The old
  "Technical Documentation" table and "Architecture & Security Reference" list are kept under
  `## Documentation`.
- `Docs/README.md`: "Project README" paragraph (structure, contracts, regeneration script).
- `Docs/DEVELOPMENT_WORKFLOW.md` §10: README structure and editing rules.
- `AI/DECISIONS.md`: ADR "[2026-10-17] README Restructure as a Product Landing Page".
- `AI/VERIFICATION_MATRIX.md`: component `readme-redesign`, rows RDM1–RDM11.

No runtime crate, packaging, script, workflow or asset file changed; no dependency added.

## 7. Candid Review

Run by an independent reviewer on the final diff. This walkthrough does not record its verdict.

## 8. Verification Results

All runs are scoped (`-p soos-invariants`, `--test-threads=3`):
- `cargo fmt --all -- --check`: clean.
- `cargo clippy -p soos-invariants --locked --all-targets --all-features -- -D warnings`: clean.
- `cargo test -p soos-invariants --locked --all-features`: all tests pass. That covers RDM1–RDM8, RDM10, RDM11, RAC10–RAC16,
  the installer, onboarding, setup, upgrade, licence and GDM follow-up README contracts, and the matrix
  citation check.

The full workspace gates run through `./save.sh` before the push.

## 9. Known Limitations / Follow-ups

- RDM9: the owner checks the GitHub render in light and dark themes and at mobile width.
- The phone web app captures (RAC18) and a lock-screen widget screenshot are not in the README yet. They come
  with the PR 3 visual refresh, after the UI redesign.
- PR #402 merged before this PR: its `setup.sh` paragraph is the one in the Quick Start, and the companion
  wording of the Why, Features, How it works, Security model and FAQ sections follows its default-on profile
  (ADR 2026-10-17 item 7).
- Repository settings proposed in the PR body (description, topics, social preview) are applied by the owner.
- The release-badge rule treats a CI clone without tags as untagged. A future release badge therefore needs a
  `CHANGELOG.md` or a tag fetched in CI.
