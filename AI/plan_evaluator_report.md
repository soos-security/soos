# Plan Evaluation Report
- **Date**: 2026-11-20
- **Issue**: Security review 2026-11-10, remediation batch 3 (GitHub #480: #548 GUI-03, #597 RCFG-02, #625 SETB-01, #626 SETB-02, #627 SETB-03, #629 SETBB-01); no `AI/BACKLOG.md` id
- **Branch**: `fix/setup-remote-defaults`
- **Base commit**: `0cce947`

The plan (architect spec in `AI/walkthroughs/259_setup_remote_defaults.md` §2) was checked against
`scripts/setup.sh` (`resolve_features`, `classify_remote_config`, `parse_user_remote_config`,
`count_of`, `bracket_depth`, `render_remote_config`, `user_config_notes`,
`print_companion_guide`), `scripts/install_remote.sh`, `crates/remote/src/config.rs`,
`crates/gui/src/onboarding.rs`, the setup harnesses of `tests/invariants` and the tests that pin
the current behaviour.

## 1. Coverage Matrix
| Acceptance line (issue #480 "What to change") | Spec element | Status |
|---|---|---|
| `allow_unlock` and `camera_view_funnel` default to off with a dedicated question each | `ASKED_KEYS`, `FEATURE_QUESTION`, `ask_features`, `resolve_features` default | Covered (SRD1, SRD3) |
| An absent key is never switched on by a re-run | `resolve_features`: an asked key is on only by option, explicit file `true` or answer | Covered for the two asked keys (SRD2); the five default-on keys stay on the owner profile (finding F2) |
| The reader fails safe on any construct it cannot model | `toml_scan_line`, `LEX_BAD`, `USER_UNMODELED` | Covered (SRD4, SRD5) |
| Setup checks the existing Serve configuration before publishing `:443` | guide prints `tailscale serve status` and the port-443 sentence first | Covered (SRD7) |
| Setup pins `allowed_hosts` to `rp_id` | `CARRIED_KEYS` pins a present `allowed_hosts` | Partial (finding F1) |
| Setup keeps the owner's hardening keys when it rewrites the file | `carried_value`, managed grammar of the carried keys, `CARRY_REFUSED` | Covered (SRD6) |
| GUI "Internet access Off" says the listener stays and shows the command | `when_off`, `Feature::off_follow_up`, `FUNNEL_LISTENER_OFF_COMMANDS` | Covered (SRD9) |

## 2. Facts Verified Against Code
| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Every key starts `true` in `resolve_features` | `scripts/setup.sh` `resolve_features` | `value=true` unless flag off or file `false` | Yes |
| `allowed_hosts` must contain `rp_id` when non-empty | `crates/remote/src/config.rs` `validate_rp_id` | refuses otherwise | Yes |
| `push_previews` values | `parse_push_previews` | `detailed` or `generic` | Yes |
| `lock_screen_programs` bound | `MAX_LOCK_SCREEN_PROGRAMS` | 4 | Yes |
| `credentials_path` bound | `MAX_CREDENTIALS_PATH_LEN` | 4096 | Yes |
| SET3 forbids `--http` anywhere in `setup.sh` code | `setup_user_phase_contract::test_set_u1_static_rules_of_the_user_phase` | literal check | Yes: the removal command is referenced, not printed, by `setup.sh` |
| Phone tours mirror the GUI Off line | `remote_tour_contract` PTR-S5, `android/app/src/main/res/values/strings.xml` | equality checks | Yes: the three copies change together |

## 3. Pillar Analysis
### Pillar 1 — Architecture & threat model
- Failure scenario considered: a carried value breaks out of its TOML string and adds a key.
- Result: PASS. Carried paths are limited to `[A-Za-z0-9._/@+-]`, `push_previews` to two
  literals, `battery_status` to a boolean, `allowed_hosts` to the already validated `rp_id`.
### Pillar 2 — PAM deadline & concurrency
- Not touched (no PAM, daemon or IPC change). PASS.
### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered: the scanner loses track and a later `true` inside a string is read.
- Result: PASS. Every top-level line is scanned whole, dotted keys included; anything TOML
  refuses, or a construct still open at the end, forces all seven feature keys off.
### Pillar 4 — Dependencies
- No dependency change. PASS.
### Pillar 5 — Data confidentiality
- Failure scenario considered: the plan line prints a carried path or a value.
- Result: PASS. Names only; SRD6 checks no path or value is printed.
### Pillar 6 — Test integrity
- Failure scenario considered: the default change breaks tests that pin `allow_unlock = true` and
  `camera_view_funnel = true`.
- Result: FINDING (MINOR), resolved by the owner decision of 2026-11-20 for batch 3: those pins are
  migrated to the new `false`, and tests whose purpose is not the defaults keep their assertions
  and get the two options (or an all-on render) in their setup. Each is listed in walkthrough 259.

## 4. Findings
- **[MINOR] F1** Writing `allowed_hosts = ["<rp_id>"]` into every render changes the byte-exact
  render pinned by SET24-SET38, SDP9-SDP27 and the fixtures beyond the approved migration. Plan
  change: pin only when the file already sets `allowed_hosts`; record the default pin as an owner
  question.
- **[MINOR] F2** Resolving the five default-on keys to `false` when a user file leaves them out
  would reverse the owner decision of 2026-10-23 and change SDP9/SDP10/SDP14/SDP27 assertions.
  Plan change: none in this PR; owner question.
- **[MINOR] F3** `setup.sh` cannot print `tailscale funnel --https=443 off` (SET3 bans `--http`).
  Plan change: setup refers to `Docs/REMOTE_COMPANION.md` §2b and the soos-gui command; the GUI,
  the docs and the phone tours carry the command.

## 5. Verdict
VALIDATION_VERDICT: APPROVED
