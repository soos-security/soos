# Plan Evaluation Report
- **Date**: 2026-10-17
- **Issue**: GitHub-only (no backlog id) — README restructure as a product landing page (GitHub #408, PR 2 of epic #356)
- **Branch**: `chore/readme-redesign`
- **Base commit**: `9e82795`
- **Plan evaluated**: `AI/architect_spec_readme_redesign.md` §0–§5
- **Mode**: self-check by the implementing agent (docs-only change; phases 1–4 condensed as allowed by the
  dev-workflow skill for a docs-only change). The independent candid review still runs separately.

## 1. Coverage Matrix

| #356 PR 2 acceptance line | Spec element | Status |
|---|---|---|
| Hero: title, tagline, at most 6 real badges, no release badge | §1 item 1, §2, RDM3 | Covered |
| No logo image (Q8 unanswered, no brand generator, owner pack never used) | §0 Q8, §1 item 1 | Covered |
| Hero visual with mock/synthetic caption and source commit | §1 items 2 and 5 | Covered |
| Why, feature table of merged features with doc links, screenshots | §1 items 3–5, RDM1, RDM6 | Covered |
| Lock-screen widget only as merged (#403), doc link, no screenshot | §1 item 4 | Covered |
| Quick Start verbatim; `setup.sh` paragraph byte-identical for #402 | §1 item 6 | Covered |
| Mermaid diagram (PAM, UDS, daemon, store, soos-remote, push-sender) | §1 item 8, RDM6 | Covered |
| Security model in plain words, PAD caveat #212, ARCHITECTURE and REMOTE_COMPANION §8 links | §1 item 9, RDM7 | Covered |
| Cited comparison with Howdy and Windows Hello, no numbers or superlatives | §1 item 10, RDM5, RDM8 | Covered |
| FAQ 5–8, static roadmap #352–#363, contributing/security links, no new SECURITY/CONTRIBUTING | §1 items 11–14 | Covered |
| License text kept | §1 item 15, `licence_contract` | Covered |
| Banned words, relative links, heading order, security needles | RDM1–RDM7 | Covered |
| Docs index, DEVELOPMENT_WORKFLOW, ADR, matrix rows, walkthrough 216 | §4 | Covered |
| Manual light/dark/mobile render | RDM9 (owner row) | Covered (pending owner) |

## 2. Facts Verified

| Fact | Check | Actual | Match |
|---|---|---|---|
| No git tag, no release | `git tag --list`, `gh release list` | both empty | Yes |
| Workflows exist | `.github/workflows/` | `ci.yml` (name CI), `fuzz.yml` (name Fuzz, nightly schedule) | Yes |
| Rust pin | `rust-toolchain.toml` | `channel = "1.98.1"` | Yes |
| Screenshot source commit | `git log -- Docs/assets/` | `a3cae16` (#392) only | Yes |
| Epics #352–#363 | `gh issue view` | all 12 open | Yes |
| Lock-screen widget merged | `git log` | `9e82795` (#403), `Docs/LOCKSCREEN_WIDGET.md` | Yes |
| Daemon has no network | `packaging/soos-daemon.service` | `RestrictAddressFamilies=AF_UNIX`, `PrivateNetwork=yes`, `IPAddressDeny=any` | Yes |
| Templates encrypted | `Docs/BIOMETRIC_STORE_CRATE.md` | AES-256-GCM | Yes |
| IPC framing | `Docs/IPC_PROTOCOL.md` §2 | u32 big-endian length prefix + postcard over a Unix stream socket | Yes |
| PAD is single-scale by default | `AI/ARCHITECTURE.md` §7, `models/README.md` §2, GitHub #212 (open) | MiniFASNetV2 at 2.7×; MiniFASNetV1SE attested but disabled | Yes |
| Howdy facts | upstream README and `howdy/src/pam/main.cc` (master) | MIT; built-in IR emitters and camera; "in no way as secure as a password"; PAM module spawns a Python compare process | Yes |
| Windows Hello facts | Microsoft Learn "Windows Hello face authentication", "Enhanced Sign-in Security" | near-IR camera; ESS isolates the face algorithm and templates in VBS on ESS-capable IR cameras | Yes |
| Existing README contracts | grep of `tests/invariants/src` | 9 tests read `README.md`; needles listed in spec §1 | Yes |
| Matrix prefix `RDM` unused | grep `AI/VERIFICATION_MATRIX.md` | 0 hits | Yes |
| Walkthrough 216 free | `ls AI/walkthroughs` | highest 214, 216 reserved | Yes |

## 3. Pillar Analysis

- **Architecture alignment**: no runtime code; the README describes the existing invariants (ARCHITECTURE §2)
  without changing them.
- **Test integrity**: no existing test file is edited except the module registration in `lib.rs` (addition
  only). Every existing README needle is kept by moving the Quick Start, Upgrade and License text verbatim.
- **Fail-closed / truthfulness**: the PAD caveat, the password-as-root-of-trust statement and the remote
  residual limitations link are asserted by RDM7.
- **Privacy**: only the three synthetic PNGs already covered by RAC11–RAC13; no new image.
- **Supply chain**: std only in the new test module.

## 4. Findings

| # | Severity | Finding | Resolution |
|---|---|---|---|
| P-1 | MINOR | A test that runs `git tag --list` may run inside a git hook with `GIT_DIR` set, and CI checkouts are shallow without tags | Clear every `GIT_*` variable and run in the workspace root; a missing or failing `git` counts as "no tag" (stricter, never weaker) |
| P-2 | MINOR | The release-badge rule could later block a legitimate release badge in a shallow CI clone | Accepted: a release file (`CHANGELOG.md`) also counts as release evidence; documented in the ADR |

VALIDATION_VERDICT: APPROVED
