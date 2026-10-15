# Candid Review Report

- **Date**: 2026-10-15
- **Target Branch**: `feat/onboarding-tour`
- **Base (merge-base)**: `3f10f61`
- **Reviewed-Diff-Fingerprint**: `d696b935a2007c52bbd32c9bb14e61549d17764acd42f163bcba85e314fa39ba`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_onboarding_tour.md`, `AI/auditor_constraints_onboarding_tour.md`, `AI/tester_contract_onboarding_tour.md`, `AI/walkthroughs/209_onboarding_tour.md`, `Cargo.lock`, `Docs/GUI_APPLICATION.md`, `Docs/NETWORK_LOCK.md`, `Docs/REMOTE_COMPANION.md`, `crates/gui/Cargo.toml`, `crates/gui/src/app.rs`, `crates/gui/src/app/capture.rs`, `crates/gui/src/feature_control.rs`, `crates/gui/src/header.rs`, `crates/gui/src/lib.rs`, `crates/gui/src/onboarding.rs`, `crates/gui/src/onboarding_view.rs`, `crates/gui/src/privileged.rs`, `crates/gui/tests/feature_control_tests.rs`, `crates/gui/tests/onboarding_audit_tests.rs`, `crates/gui/tests/onboarding_feature_tests.rs`, `crates/gui/tests/onboarding_state_tests.rs`, `crates/gui/tests/onboarding_view_tests.rs`

## 1. Executive Summary

Re-review of the full raw diff (merge-base `3f10f61` to the working tree, 25 files, 5776 patch
lines) after commit `c1853f2`, which answers the CHANGES_REQUESTED review of the rebase (M-1 and
M-2) and fixes a hidden merge conflict with the README captures of #381/#392
(`crates/gui/src/app/capture.rs` built `SoosApp` without the tour fields).

`git range-diff` shows that the two earlier branch commits (`3693833`, `d1e94e4`) differ from the
previously reviewed ones only in documentation context lines; the code delta since the last review
is exactly `c1853f2`, which this review read hunk by hunk against `crates/remote` on the base.

- **M-1 resolved**: `RemoteSwitch::Push.keys_to_clear()` is `["push_notifications",
  "offline_alerts"]`; `disable_remote_keys` only rewrites existing `key = true` lines and its
  verification refuses any added key, so a file without `offline_alerts` stays without it.
- **M-2 resolved**: `Feature::OfflineAlerts` is a `soos-remote` key feature
  (`RemoteSwitch::OfflineAlerts`, key `offline_alerts`, 7 keys parsed), with a GUI off route and a
  shown "on" command naming `push_notifications = true`.
- **capture.rs**: closed tour, no tour store, `features_loaded = true`, and a `NoFeatureExecutor`
  that returns an unknown snapshot for `Refresh` and an error for every change: no probe, no
  write, no process.

Gates (run in the worktree with the requested environment):
`cargo clippy --locked -p soos-gui --all-targets --all-features -- -D warnings` exit 0;
`cargo test --locked -p soos-gui -p soos-invariants --all-features -- --test-threads=4` exit 0,
959 passed, 0 failed, 0 ignored (the README snapshot drift test
`test_rac_capture_scenes_match_committed_assets` included and green, with `Docs/assets/`
untouched).

No CRITICAL or MAJOR finding. Two new MINOR findings (wording of the offline alerts "off" text,
duplicate matrix ID `OBT19`) and the carried-forward MINORs remain.

## 2. Test Changes (mechanical listing from step 3)

Against the merge-base, all five test files are **new** (`feature_control_tests.rs`,
`onboarding_audit_tests.rs`, `onboarding_feature_tests.rs`, `onboarding_state_tests.rs`,
`onboarding_view_tests.rs`); the frozen patch has no `^-` assertion/test line, no new
`#[ignore]`/`#[cfg(any())]`/`should_panic`/tolerance, and no inline `mod tests` change. No test
that exists on `main` is modified.

Intra-branch contract migration in `c1853f2` (`git show c1853f2 -- crates/gui/tests`), owner
approved on 2026-10-15 and recorded in spec §11, ADR item (6) and matrix OBT5/OBT6/OBT19. Each
change was checked to be a faithful re-expression of the new contract:

| Test | Change | Assessment |
|---|---|---|
| `test_obt_keys_to_clear_cascade_dependent_keys` | Push list becomes `["push_notifications", "offline_alerts"]`; new exact assertion for `OfflineAlerts`; `OfflineAlerts` added to the "first key is the switch key" loop | Stricter (exact lists, one more switch covered). Approved scope. |
| `test_obt_unknown_snapshot_shows_unknown_everywhere` (renamed from `..._but_offline_alerts`) | The `NotAvailable` exception for offline alerts is removed: every feature must be `Unknown` | Stricter (no exception left). Approved scope. |
| `test_obt_all_on_snapshot_shows_on` | Exception removed: every feature must be `On` | Stricter. Approved scope. |
| `test_obt_offline_alerts_have_an_off_route_and_need_push_to_turn_on` (replaces `..._have_no_control_yet`) | Asserts the exact off action `DisableRemote(OfflineAlerts)`, an "on" command containing `offline_alerts = true` and `push_notifications = true`, and that `when_off()` is no longer the placeholder | Exact re-expression of the new route; fails on a wrong switch, a missing prerequisite or a GUI "on" action. Approved scope. |
| `test_obt_parse_remote_keys`, `all_on()`, `test_obt_remote_key_states_follow_each_key`, `onboarding_view_tests::snapshots()` | `offline_alerts` field added to `RemoteKeys` literals; `REMOTE_FEATURES` gains the `(OfflineAlerts, OfflineAlerts)` pair; one exhaustive `match` arm `RemoteSwitch::OfflineAlerts => keys.offline_alerts = false` | Setup only (required by the new struct field and enum variant); the loops now also cover the new feature. No assertion changed. |
| `test_obt_disable_push_also_clears_offline_alerts` (new) | Push off on a file with `offline_alerts = true` yields both keys `false`, same line count; without the key nothing is added | Fails on the M-1 bug and on an implementation that appends the key. |
| `test_obt_parse_remote_keys_reads_offline_alerts` (new) | Key read as `true`; a non-boolean value is `Invalid` | Fails if the key is not parsed or silently coerced. |

No other assertion changed. Conclusion: the migration stays within the owner-approved scope and
does not weaken any check.

## 3. Deep Reasoning Audit

### Logic & Architecture
- **Dependency check against `soos-remote` (base `3f10f61`)**: `crates/remote/src/config.rs:106,357`
  — `offline_alerts` defaults to `false` and is refused (`offline_alerts requires
  push_notifications`, exit 78) without push; `main.rs` `offline_wiring` enables it only with push.
  Nothing else depends on `offline_alerts`. The Push cascade is now complete: after "Push off"
  every key that requires push is `false`. Clearing `offline_alerts` alone is always valid. PASS.
- **Never adds keys**: scenario "Push off on a file with no `offline_alerts`": the line loop has
  nothing to rewrite, the verification accepts an absent key (`None | Some(false)`), and
  `edited.keys().all(|k| original.contains_key(k))` refuses any added key. Pinned by the new test.
  PASS.
- **Unusual layouts**: `offline_alerts` quoted (`"offline_alerts" = true`) or dotted: the line is
  not matched, verification reads `true` and returns `VerifyFailed`, nothing is written. A
  non-boolean value: `UnsupportedValue`, nothing written. PASS.
- **State**: `OfflineAlerts` joins the remote-key arm (unit not installed → NotInstalled, missing
  config → NotInstalled, unreadable or invalid → Unknown, otherwise the key). A non-boolean
  `offline_alerts` makes the whole `remote.toml` `Invalid`, so every remote feature shows Unknown
  and the off routes fall back to the shown command — fail-safe, and consistent with
  `soos-remote` refusing such a file. PASS.
- **Routing**: off → `DisableRemote(OfflineAlerts)` (command when root or keys unknown, same as the
  other keys); on → shown command with `push_notifications = true`; the GUI never turns a remote
  key on (AS-1 preserved). `applied_message` maps the new switch to its feature. All `match`es are
  exhaustive (no wildcard swallowing the new variant in `feature_control.rs`). PASS.
- **Copy**: the description ("told when this PC comes back online after being offline, with how
  long it was away") matches §2j of `Docs/REMOTE_COMPANION.md`. The "off" text claims the offline
  history on the phone page keeps working, which is wrong (see MINOR N-1).
- **`FeatureState::NotAvailable`** stays in the enum with its view label but no feature uses it;
  spec §11 records this deliberately. Acceptable.
- **Follow-up (non-blocking, documented in spec §11 and walkthrough 209)**: `try-restart` of the
  `Type=exec` unit still reports success before `soos-remote` parses its configuration, so a file
  refused for another reason would be reported as applied. With M-1 fixed, every file the GUI
  writes is accepted by `soos-remote` whenever the original was.

### PAM Concurrency & Deadlines
- `crates/pam` untouched. No new blocking path: `c1853f2` adds only data (a key, a switch) to the
  existing worker-thread edit and the bounded `systemctl --user` restart. PASS.

### Panic Safety & Fail-Closed
- No `unwrap`/`expect`/`panic!`/indexing added in production code; the array destructuring
  `[...; 7]` is length-checked at compile time. No error path shows a feature as On. PASS.

### Test Integrity & Anti-Weakening
- See section 2. The migration is limited to the approved tests and setup literals; all changed
  assertions are equal or stricter. Two new tests cover M-1 and the new parser key. PASS.

### Memory, Bounds & Secrets
- **capture.rs executor**: scenario "a capture clicks the Features button or the startup path
  submits a refresh": `capture_app` sets `features_loaded = true` and `refresh_pending = false`,
  and the constructor's startup `refresh_features()` (`app.rs:248`) is not on the capture path;
  even if a refresh or action were submitted, `NoFeatureExecutor` returns
  `FeatureSnapshot::unknown()` or `Err(..)` without touching the file system or spawning a
  process. `tour_store: None` means `persist_tour_seen` writes nothing. Only the `FeatureRunner`
  worker thread exists, as in the GUI. The module stays behind the non-default `readme-capture`
  feature. The README drift test passes with `Docs/assets/` unchanged, so the captures did not
  change. PASS.
- No secret, frame or embedding touched; the `offline_alerts` value is never logged. PASS.

### Supply Chain & Automation
- `c1853f2` changes no manifest, lockfile, script or workflow. The branch's only dependency change
  remains the `toml` workspace edge for `crates/gui`. PASS.

### English-Only Policy
- All added strings, comments and docs are English; the only non-ASCII characters added are the
  em dash in the spec heading and the matrix status emoji, both existing conventions. PASS.

### Documentation Consistency
- `Docs/GUI_APPLICATION.md` (off routes table lists offline alerts and the cascade; the "not
  available" row is gone), ADR item (6) amended, spec §11 revision, walkthrough 209 (AS-3
  superseded, `ActiveState` follow-up), matrix OBT5/OBT6 updated with the renamed tests; no stale
  test name or "not available in this version" claim remains outside the revision history (the
  original §4/§5 rows of the spec are explicitly superseded by §11).
- The new matrix row reuses the ID `OBT19` (see MINOR N-2).

## 4. Detailed Findings & Action Items

- **[MINOR] N-1** `crates/gui/src/onboarding.rs:164` (`Feature::OfflineAlerts.when_off()`) —
  "push alerts and the offline history on the phone page keep working" is inaccurate: with
  `offline_alerts = false`, `soos-remote` records nothing and `GET /api/offline` returns
  `state = "disabled"` with no intervals (`crates/remote/src/offline.rs` `OfflineView::disabled`,
  `crates/remote/src/main.rs` `offline_wiring`). Suggested text: "Off: no offline history is
  recorded and no back-online notification is sent; push alerts keep working." (No test pins this
  wording beyond "not the placeholder".)
- **[MINOR] N-2** `AI/VERIFICATION_MATRIX.md:2603` — the new row reuses `OBT19`, already used at
  line 2596 (tour state written only in the user's own tree); spec §11 and the commit message also
  say "new OBT19". Renumber it (next free ID is `OBT23`) and list the README capture evidence
  (`readme_snapshots::test_rac_capture_scenes_match_committed_assets`) for its capture clause.
- **[SUGGESTION]** `crates/gui/src/onboarding.rs:319` — a hand-written file with
  `offline_alerts = true` and `push_notifications = false` (already refused by `soos-remote`)
  shows offline alerts as On; the companion page shows the unit state, so this is cosmetic.
  Optionally show such a pair as Unknown or with a hint.
- **[MINOR, non-blocking follow-up, documented]** `crates/gui/src/feature_control.rs:761` — check
  `ActiveState` after `systemctl --user try-restart soos-remote.service` so that a configuration
  refused for any reason is never reported as success (spec §11, walkthrough 209).

Carried forward (unchanged code):

- **[MINOR]** `crates/gui/src/feature_control.rs:423` (`bool_with_comment`) — a CRLF file gets
  mixed line endings on rewritten lines (still valid TOML).
- **[MINOR]** `crates/gui/src/feature_control.rs` (`disable_remote`) — read-edit-rename is not
  guarded against a concurrent same-user edit (last writer wins; no privilege impact).
- **[MINOR]** `crates/gui/src/onboarding.rs` (`TourStore`) — no `fsync` of the parent directory
  after `rename` (durability only).
- **[MINOR]** `crates/gui/src/app.rs` (`render_tour`) — in polkit store mode a pending startup
  `pkexec` dialog keeps the first-run tour from closing until it is answered.

## 5. Final Verdict

M-1 and M-2 of the previous review are resolved in production code, the test migration is
faithful and within the owner-approved scope, the README capture path is side-effect-free, and
clippy and the 959 tests are green. Remaining findings are MINOR or SUGGESTION.

**VERDICT: APPROVED**
