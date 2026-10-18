# Walkthrough 228 — Companion Off-Switch and Opt-In Wording Aligned With the Setup Defaults

- **Date**: 2026-10-17
- **Issue**: GitHub #423 (sub-issue of epic #362; no backlog id, branch not registered in `scripts/sync_issue.py`) — **Branch**: `chore/companion-wording`
- **Base commit**: `74782b8`
- **Origin**: suggestions S-1, S-2 and S-4 of the #409 candid review
- **Matrix criteria**: CWD1–CWD5 (new component `companion-wording`)

## 1. Context & Objectives

The defaults are settled (#400, #362 OD-1): the one-command setup turns every phone companion feature on,
while a manual install (`scripts/install_remote.sh`) and the code defaults leave them off. Some text still
disagreed with that or with the GUI:

- `scripts/setup.sh --help` and its closing summary said every companion feature "can be turned off later in
  soos-gui (Features)". `soos-gui` has no switch for the failed-password alerts (S-4).
  `Docs/PACKAGING_AND_PROVISIONING.md` repeated the claim ("each switchable off later in soos-gui").
- `Docs/REMOTE_COMPANION.md` §2b and the `AI/ARCHITECTURE.md` companion table called `allow_funnel`,
  the push sender and remote unlock "opt-in", though setup turns them on (S-2).
- The README security section named only the Funnel, unlock and camera-view switches (S-1).

## 2. Architect Design

Spec `AI/architect_spec_companion_wording.md`: wording only, no behaviour change. The facts come from
`RemoteSwitch` (`crates/gui/src/feature_control.rs`: switches for `allow_unlock`, `allow_funnel`,
`push_notifications`, `camera_view`, `offline_alerts`; turning one off also clears its dependants),
`resolve_features` in `setup.sh` (`--password-alerts off` also writes push and offline alerts off) and
`crates/remote/src/config.rs` (every key defaults to `false`).

## 3. Plan Evaluation

`AI/plan_evaluator_report.md` (self-check, `VALIDATION_VERDICT: APPROVED`). P-1 brought the packaging doc into
scope. P-2 leaves the §13 prose "opt-in" mentions of `AI/ARCHITECTURE.md` (Funnel, live camera view) for a
follow-up, as the task limited the change to the two table headers.

## 4. Tester Contract

New module `tests/invariants/src/companion_wording_contract.rs` (std only, registered in `lib.rs`):

| Row | Test | Red on `74782b8` |
|---|---|---|
| CWD1 | `test_cwd_setup_names_the_password_alerts_off_path` | Yes: both setup sentences flagged |
| CWD2 | `test_cwd_gui_has_no_password_alerts_switch` | No (premise, green by design) |
| CWD3 | `test_cwd_docs_do_not_call_setup_enabled_keys_opt_in` | Yes: the §2b example said "opt-in"; the packaging assertion was also checked red on its own |
| CWD4 | `test_cwd_readme_names_push_and_offline_switches` | Yes: the off-switch sentence lacked push and offline alerts |
| CWD5 | `test_cwd_detector_helpers` | No (helper unit test) |

No existing test was modified.

## 5. Auditor Constraints

`AI/auditor_constraints_companion_wording.md` C1–C10: text only in `setup.sh`, every option line kept (SET21),
no new command word (SET2, SET3), the `remote.toml` marker comment byte-identical, no expansion characters in
the unquoted heredoc, every claim true on the base commit, README RDM10/RDM12 phrasing rules respected.

## 6. Implementation

- `scripts/setup.sh` `usage()`: "Every feature below is on by default (owner decision). Funnel, remote
  unlock, push, offline alerts and the camera view can be turned off later in soos-gui (Features), which also
  turns off the features that need them. The failed-password alerts have no switch there: turn them off with
  --password-alerts off, or set password_alerts, push_notifications and offline_alerts to false in
  remote.toml." The "kept off" sentence and every option line are unchanged.
- `scripts/setup.sh` closing summary: the same split, then the unchanged uninstall hint.
- `Docs/REMOTE_COMPANION.md` §2b: `allow_funnel = true  # code default false keeps Funnel requests at 403;
  scripts/setup.sh writes true`.
- `Docs/PACKAGING_AND_PROVISIONING.md` "Defaults": the failed-password alerts exception and
  `--password-alerts off`.
- `AI/ARCHITECTURE.md`: "Push sender (off by default in code; on with `scripts/setup.sh`)" and "Unlock (off
  by default in code; on with `scripts/setup.sh`)"; cell text unchanged.
- `README.md` security model: push and offline alerts named among the switches; turning push off also sets
  `offline_alerts` to `false`; the failed-password alerts have no switch there.

## 7. Candid Review

Run separately on the final diff (not part of this walkthrough's authoring step).

## 8. Verification Results

- `cargo test -p soos-invariants -- --test-threads=3`: all tests pass, CWD1–CWD5 included; the setup
  contracts (SET1–SET39) and README contracts (RDM1–RDM13) stay green.
- `bash -n scripts/setup.sh` passes and `scripts/setup.sh --help` renders the new paragraph.
- `cargo fmt --check` and `cargo clippy -p soos-invariants --all-targets -- -D warnings` clean.

## 9. Known Limitations / Follow-ups

- `AI/ARCHITECTURE.md` §13 prose still calls Funnel and the live camera view "opt-in" when describing the code
  default (out of scope here).
- If `soos-gui` ever gains a `password_alerts` switch, CWD2 fails on purpose: update the setup help, the
  summary, the README and CWD1 together.
