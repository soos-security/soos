# Walkthrough 259 — Safe Companion Defaults and a Fail-Safe `remote.toml` Reader

- **Date**: 2026-11-20
- **Issue**: security review 2026-11-10, remediation batch 3 (GitHub #480; tracking issue #477), no backlog id — **Branch**: `fix/setup-remote-defaults`
- **Findings**: #548 GUI-03, #597 RCFG-02, #625 SETB-01, #626 SETB-02, #627 SETB-03, #629 SETBB-01
- **Matrix criteria**: SRD1–SRD9 (new); RDM12 and SET38 wording updated
- **ADR**: 2026-11-20 "Setup Leaves Remote Unlock and the Camera View Over Funnel Off, Keeps the Owner's Hardening Keys and Reads `remote.toml` Fail-Safe"

## 1. Context & Objectives

The review found that `scripts/setup.sh` wrote remote unlock and the camera view over Funnel on
with one aggregate consent, also for keys a re-run found absent (#625); that the line reader of a
`remote.toml` setup did not write lost track of strings and arrays in five ways, so an explicit
`false` after them was never seen and became `true` (#626, #629); that a rewrite dropped the
owner's hardening and privacy keys and never pinned `allowed_hosts` (#597); that the Funnel guide
never mentioned that Funnel publishes every handler on port 443 of the node (#627); and that the
soos-gui Off line of Internet access claimed tailnet-only reachability while the public listener
stays published (#548). Objectives, from the batch: ask for the two exposure features, never turn
an absent key on, read fail-safe, keep the hardening keys, check the serve configuration first,
and say what the Off switch really does.

## 2. Architect Design

- **Asked keys** (`scripts/setup.sh`): `ASKED_KEYS=(allow_unlock camera_view_funnel)` and
  `FEATURE_QUESTION`. `resolve_features` resolves, per key: option, else an explicit value of the
  existing file (`true` or `false`), else the default — on for the five other keys, and for an
  asked key on only when `FEATURE_ANSWER[key]=on`, otherwise off with the note
  `--<option> on to turn it on`. `ask_features` (interactive runs only, after the identity, before
  the plan) asks each asked key that no option or file value decides and whose prerequisites are
  on; only `y`/`yes` turns it on. `user_config_notes` never names an answered key as turned on by
  the default profile.
- **Reader** (`toml_scan_line`, state `LEX_ML`, `LEX_DEPTH`, `LEX_BAD`): every top-level line is
  scanned whole and the state carries to the next line: comments end a line; basic strings honour
  backslash escapes; literal strings have none; `"""` and `'''` strings end only at their own
  delimiter, with up to two quotes of content before it; `[` and `{` count only outside strings
  and comments. This replaces `count_of` and `bracket_depth`. A one-line string never closed, a
  closing bracket without an opening one, a backslash outside a string, more than five closing
  quotes, or a string or array still open at the end sets `USER_UNMODELED`: every feature key is
  off and the plan says "every feature kept off: the file has a construct setup cannot read". The
  existing fail-safe rules (a feature key named by an unparsed line, the escaped-key rule) stay.
- **Carried keys** (`CARRIED_KEYS`, `carried_value`, `carry_path_ok`, `FILE_CARRIED`,
  `CARRY_REFUSED`): `allowed_hosts` (any value, written `["<rp_id>"]`), `push_previews`
  (`detailed` or `generic`), `battery_status` (boolean), `lock_screen_programs` (0–4 paths) and
  `credentials_path` (one path); paths absolute, at most 4096 bytes, `[A-Za-z0-9._/@+-]`, no
  trailing `/`, no `..`. Read from a user file (one line each, strict grammar) and accepted by the
  managed grammar in their rendered spelling; rendered after the feature keys, before `[android]`.
  A `credentials_path` setup cannot write back refuses the run before any change (exit 2).
- **Guide**: with Funnel on, `tailscale serve status` and the port-443 sentence come before the
  Funnel command, then a warning lists what is reachable from the Internet behind a passkey (sign
  in, status, lock; unlock and the live camera only when on) and that the soos-gui switch keeps the
  listener. `setup.sh` may not contain `--http` (SET3), so it refers to the command of
  `Docs/REMOTE_COMPANION.md` §2b and of soos-gui instead of printing it.
- **GUI** (`crates/gui/src/onboarding.rs`): new `Feature::off_follow_up` and
  `FUNNEL_LISTENER_OFF_COMMANDS`; `onboarding_view::feature_controls` shows the commands in a
  copyable block while Internet access is off. The GUI still never runs `tailscale`. The Off line
  changes in the GUI, the web app (`crates/remote/assets/app.js`) and the Android app
  (`strings.xml`) together (PTR-S5 keeps them equal).
- **Invariants touched**: ADR 2026-10-23 part 1 item (3) "an explicit `false` is kept" (now true
  for every file layout TOML accepts); SET3 (only `tailscale version` runs, no `--http`); no value
  printed (R2-11).

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`: **APPROVED** with three minor findings that shaped the plan:
F1 the default `allowed_hosts` pin in every render would change byte-exact renders beyond the
approved migration (owner question; the pin applies when the file sets `allowed_hosts`); F2 the
five default-on keys keep the owner profile for absent keys (owner question); F3 the removal
command is referenced, not printed, by `setup.sh`.

## 4. Tester Contract

New tests (written first; Red evidence observed before the implementation):

| Test | Criterion | Red evidence |
|---|---|---|
| `soos-invariants::setup_deps_contract::test_srd1_yes_run_leaves_unlock_and_camera_view_funnel_off` | SRD1 | render had `allow_unlock = true`, `camera_view_funnel = true` |
| `...::test_srd2_an_absent_key_is_never_turned_on_by_a_rerun` | SRD2 | rewrite turned both absent keys on |
| `...::test_srd3_interactive_run_asks_each_question_default_no` | SRD3 | "the unlock question is asked" not found |
| `...::test_srd4_reader_tracks_strings_comments_dotted_keys_and_escapes` | SRD4 | row `literal`: `allow_funnel`/`camera_view` written `true` |
| `...::test_srd5_unreadable_construct_turns_every_feature_off` | SRD5 | row `openml`: features written on |
| `...::test_srd6_hardening_keys_carried_over_and_allowed_hosts_pinned` | SRD6 | carried lines missing |
| `...::test_srd7_funnel_guide_checks_serve_status_and_states_the_exposure` | SRD7 | "serve status check" missing |
| `...::test_srd8_help_and_kept_summary_name_the_asked_features` | SRD8 | help sentence missing |
| `...::test_srd9_adr_matrix_and_walkthrough` | SRD9 | ADR missing |
| `soos-remote::setup_render_tests::test_srd6_carried_render_is_accepted` | SRD6 | new fixture parsed by the real parser |
| `soos-gui::onboarding_feature_tests::test_obt_internet_access_off_names_the_public_listener` | SRD9 | `Feature::off_follow_up` did not exist (specified API) |

SRD7 was adjusted before the implementation was finished: its first draft expected the literal
`tailscale funnel --https=443 off` in the setup output, which SET3 forbids in `setup.sh`; it now
expects the reference to `Docs/REMOTE_COMPANION.md` §2b, and the literal command is checked in
the GUI test and the docs.

### Migrated existing tests (owner decision of 2026-11-20, batch 3 only)

Pins of the old ON defaults of `allow_unlock` / `camera_view_funnel`, migrated to `false`:

| Test | Old assertion | New assertion |
|---|---|---|
| SET23 `setup_user_phase_contract::test_set_u5_dry_run_prints_the_companion_plan_without_change` | plan contains `remote unlock on` | `remote unlock off` |
| SET24 `test_set_u6_full_run_sets_up_the_default_on_companion` | `managed_remote([true; 7])` | `[true, false, true, true, true, true, false]` |
| SET27 `test_set_u9_prerequisites_cascade_and_offline_needs_push` | three vectors with `allow_unlock` (and, `--push off`, `camera_view_funnel`) `true` | the same vectors with both `false` |
| SET30 `test_set_u12_template_replaced_user_file_untouched` | template render `[true; 7]`; user-file renders `[false, true, ...]` | `allow_unlock` `false` in all three |
| SET38 `test_set_u20_writable_config_dir_refused_and_render_matches_fixture` | `managed_remote([true; 7])` equals the fixture | default vector equals the fixture |
| SDP9 `setup_deps_contract::test_sdp9_user_file_backed_up_and_replaced` | vector with `allow_unlock` `true`; note lists `allow_unlock, password_alerts, ...` | `allow_unlock` `false`; note lists `password_alerts, push_notifications, offline_alerts, camera_view` |
| SDP10 `test_sdp10_identity_from_the_user_file_else_kept` | `[true; 7]` | default vector |
| SDP14 `test_sdp14_tolerant_fail_safe_booleans_of_a_user_file` | six rows with the two keys `true` where absent | both `false` |
| SDP16 `test_sdp16_user_android_block_carried_over` | two renders `[true; 7]` | default vector |
| SDP18 `test_sdp18_non_strict_user_android_table_not_carried` | `allow_unlock` `true` | `false` |
| SDP27 `test_sdp27_quoted_keys_and_unparsed_lines_fail_safe` | six rows with the two keys `true` where absent | both `false` |
| RDM12 `readme_contract::test_rdm_companion_opt_out_names_dependent_keys` | fixture sets all four exposure keys `true` | fixture sets `allow_funnel`, `camera_view` |
| `soos-remote::setup_render_tests::test_set_render_with_every_default_on_is_accepted` | fixture parses with `allow_unlock` and `camera.funnel` on | off in the fixture; the all-on render (both turned on) also parsed with both on |
| fixtures `tests/fixtures/setup/remote.toml`, `remote_android.toml` | `allow_unlock = true`, `camera_view_funnel = true` | both `false` |

Setup-only migrations (assertions unchanged; the test's purpose is not the defaults):

| Test | Change |
|---|---|
| SET26 `test_set_u8_explicit_false_is_kept_unless_its_flag_says_on` | first run passes `--remote-unlock on --camera-view-funnel on`, so the GUI turn-off of `camera_view_funnel` still has a `true` to edit |
| SET33 `test_set_u22_rollback_stops_a_unit_started_by_the_run` (restart case) | first run passes `--remote-unlock on`, so `--remote-unlock off` still changes the file |
| `soos-remote::setup_render_tests` `with_off` | starts from `fixture_all_on()` (the fixture with both keys back on, byte-equal to the former fixture), so the cascade and refusal variants parse the same inputs as before |

### Flakiness check

The pseudo-terminal test SRD3 was run 5 times in a row: green each time (about 10 s per run).

## 5. Auditor Constraints

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| 1 | No new external command in `setup.sh`; nothing sourced or evaluated | `toml_scan_line`, `carried_value`, `ask_features` | SDP3 scanner, SCN suites, code review |
| 2 | Questions only when `INTERACTIVE` (never `--yes` or `--dry-run`), default no | `ask_features` | SRD3, SET1 static rule |
| 3 | A carried value can never break out of its TOML string | `carry_path_ok`, `carried_value` | SRD6 (fixture parsed by `soos-remote`) |
| 4 | No value of the file printed, names only | `user_config_notes` | SRD5, SRD6 |
| 5 | Every construct the reader cannot model ends with all features off | `parse_user_remote_config` | SRD5 |
| 6 | Setup runs no `tailscale` command but `tailscale version` and never contains `--http` | `print_companion_guide` | SET1, SRD7 |
| 7 | The GUI never runs `tailscale`; the follow-up is text only | `Feature::off_follow_up` | SRD9, code review |
| 8 | A refused run changes nothing | `CARRY_REFUSED` preflight | SRD6 snapshot |

Pre-existing violations found: none. Clearance: CLEARED.

## 6. Implementation

- `scripts/setup.sh`: `ASKED_KEYS`, `FEATURE_QUESTION`, `CARRIED_KEYS`, `asked_key`,
  `resolve_features`, `ask_features`, `toml_scan_line` (replaces `count_of` and
  `bracket_depth`), `carry_path_ok`, `carried_value`, the carried keys in `render_remote_config`,
  `classify_remote_config` and `parse_user_remote_config`, `user_config_notes`, the guide, the
  kept-file summary and `--help`.
- `scripts/install_remote.sh`: the `tailscale serve status` check before its Funnel command.
- `crates/gui/src/onboarding.rs`, `crates/gui/src/onboarding_view.rs`,
  `crates/remote/assets/app.js`, `android/app/src/main/res/values/strings.xml`: the Off line and
  the GUI follow-up block.
- Docs: `Docs/REMOTE_COMPANION.md` §2b and §4, `Docs/PACKAGING_AND_PROVISIONING.md` §3.0,
  `Docs/GUI_APPLICATION.md` §1e, `README.md`, `AI/ARCHITECTURE.md` (Unlock row),
  `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`.

## 7. Candid Review

Run by an independent reviewer on the committed branch, after this walkthrough (fingerprint-bound
report `AI/candid_review_report.md`); not part of this commit.

## 8. Verification Results

With `CARGO_TARGET_DIR` per worktree, `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=3`:

```bash
cargo fmt --all -- --check                                                     # clean
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings  # clean
cargo test --locked -p soos-invariants --all-targets --all-features -- --test-threads=4
cargo test --locked -p soos-gui --all-targets --all-features -- --test-threads=4
cargo test --locked -p soos-remote --all-targets --all-features -- --test-threads=4
./scripts/candid_review.sh                                                     # layer 1
bash -n scripts/setup.sh scripts/install_remote.sh
```

All green: `soos-invariants` 932 passed, `soos-gui` and `soos-remote` every test binary passed (0 failed), candid layer 1 PASSED. SRD3 (pseudo-terminal) run 5 times in a row: green each time.

## 9. Known Limitations / Follow-ups

- **#597, partly deferred (owner question)**: `allowed_hosts` is pinned to `rp_id` only when the
  file already sets it. Writing the pin into every render changes the byte-exact render that
  SET24–SET38 and SDP9–SDP27 pin, which the batch 3 migration approval does not cover.
- **#625, partly deferred (owner question)**: the five default-on keys (Funnel, password alerts,
  push, offline alerts, camera view) are still turned on when a user file leaves them out (owner
  decision of 2026-10-23 and its tests); only the two exposure keys are never turned on that way.
  The daemon-side `daemon.toml` creation is unchanged.
- **#548, accepted residual kept**: the soos-gui switch still only writes `allow_funnel = false`;
  the public listener and the pre-auth head parsing stay until the owner runs the Tailscale
  command, which the tour now shows. An owner who upgraded from an older setup keeps an explicit
  `allow_unlock = true` / `camera_view_funnel = true` already written; the guide's warning lists
  them.
- `vapid_subject`, `poll_interval_ms` and the other unmanaged keys are still not carried over.
