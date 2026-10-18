# Plan Evaluation Report
- **Date**: 2026-10-17
- **Issue**: GitHub-only (no backlog id) — companion off-switch and opt-in wording aligned with the setup defaults (GitHub #423, part of epic #362; #409 candid review S-1, S-2, S-4)
- **Branch**: `chore/companion-wording`
- **Base commit**: `74782b8`
- **Plan evaluated**: `AI/architect_spec_companion_wording.md` §0–§4
- **Mode**: self-check by the implementing agent (wording-only change; phases 1–4 condensed as allowed by the
  dev-workflow skill for a docs-only change). The independent candid review still runs separately.

## 1. Coverage Matrix

| #423 acceptance line | Spec element | Status |
|---|---|---|
| `setup.sh --help` stops claiming every feature has a `soos-gui` switch; names the password-alerts path | §2 item 1, CWD1 | Covered |
| Closing summary line of `setup.sh` likewise | §2 item 2, CWD1 | Covered |
| `Docs/REMOTE_COMPANION.md` §2b example without "opt-in", code default and setup value stated | §2 item 3, CWD3 | Covered |
| `AI/ARCHITECTURE.md` "Push sender" and "Unlock" headers reworded, default-false kept | §2 item 4, CWD3 | Covered |
| README security sentence names push and offline-alert switches | §2 item 5, CWD4 | Covered |
| Static invariant against the false GUI claim | CWD1, CWD2, CWD5 | Covered |
| Matrix rows, walkthrough 228, no ADR | §4 | Covered |

## 2. Facts Verified

| Fact | Check | Actual | Match |
|---|---|---|---|
| GUI switches | `crates/gui/src/feature_control.rs` `RemoteSwitch::key` | `allow_unlock`, `allow_funnel`, `push_notifications`, `camera_view`, `offline_alerts` | Yes |
| GUI clears dependants | `RemoteSwitch::keys_to_clear` | Funnel → `camera_view_funnel`; Push → `offline_alerts`; Camera view → `camera_view_funnel` | Yes |
| `--password-alerts off` cascades | `scripts/setup.sh` `resolve_features`, `FEATURE_PREREQS` | push and offline alerts written off ("needs password alerts"); only `--push on` with it exits 2 (SET22) | Yes |
| Code defaults | `crates/remote/src/config.rs` | `allow_funnel`, `allow_unlock`, `password_alerts`, `push_notifications`, `offline_alerts` default `false` | Yes |
| Manual install never enables the push sender | `scripts/install_remote.sh` line 70 | "installed, never enabled by this script" | Yes |
| Constrained text in `setup.sh` | SET2, SET3, SET21, `setup_user_phase_contract.rs:811` | option lines, marker comment and banned words untouched | Yes |
| README contracts | `readme_contract.rs` RDM10, RDM12 | new sentence makes no independent opt-out or opt-in claim | Yes |
| Same false claim elsewhere | grep of `README.md`, `Docs/`, `AI/`, `scripts/` | also `Docs/PACKAGING_AND_PROVISIONING.md` "Defaults" ("each switchable off later in soos-gui") | Added to scope (P-1) |
| Matrix prefix `CWD` unused | grep `AI/VERIFICATION_MATRIX.md` | 0 hits | Yes |
| Walkthrough 228 free | `ls AI/walkthroughs` | absent (number assigned by the lead) | Yes |

## 3. Pillar Analysis

- **Architecture alignment**: no runtime change; the text now matches `feature_control.rs`, `config.rs` and
  the `setup.sh` behaviour.
- **Test integrity**: no existing test file is edited except the module registration in `lib.rs` (addition
  only).
- **Truthfulness**: CWD2 pins the premise (no `password_alerts` switch), so the help cannot silently become
  wrong in the other direction.
- **Supply chain**: std only in the new test module.

## 4. Findings

| # | Severity | Finding | Resolution |
|---|---|---|---|
| P-1 | MINOR | `Docs/PACKAGING_AND_PROVISIONING.md` repeats the S-4 claim | Fixed in the same change; asserted by CWD3 |
| P-2 | INFO | `AI/ARCHITECTURE.md` §13 prose still says "Funnel (opt-in …)" and "Live camera view (opt-in …)" | Out of scope by request (only the two table headers); listed as a follow-up |

VALIDATION_VERDICT: APPROVED
