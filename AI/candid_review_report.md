# Candid Review Report

- **Date**: 2026-10-15
- **Target Branch**: `feat/onboarding-tour` (PR #389, rebased onto `origin/main` `8f76427`)
- **Base (merge-base)**: `8f76427`
- **Reviewed-Diff-Fingerprint**: `b19df03d98efd156f9f7f768303f0415685beac841118f7af46232e7b46d6e0e`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_onboarding_tour.md`, `AI/auditor_constraints_onboarding_tour.md`, `AI/tester_contract_onboarding_tour.md`, `AI/walkthroughs/209_onboarding_tour.md`, `Cargo.lock`, `Docs/GUI_APPLICATION.md`, `Docs/NETWORK_LOCK.md`, `Docs/REMOTE_COMPANION.md`, `crates/gui/Cargo.toml`, `crates/gui/src/app.rs`, `crates/gui/src/app/capture.rs`, `crates/gui/src/feature_control.rs`, `crates/gui/src/header.rs`, `crates/gui/src/lib.rs`, `crates/gui/src/onboarding.rs`, `crates/gui/src/onboarding_view.rs`, `crates/gui/src/privileged.rs`, `crates/gui/tests/feature_control_tests.rs`, `crates/gui/tests/onboarding_audit_tests.rs`, `crates/gui/tests/onboarding_feature_tests.rs`, `crates/gui/tests/onboarding_state_tests.rs`, `crates/gui/tests/onboarding_view_tests.rs`

## 1. Executive Summary

Re-review of the full raw diff (merge-base `8f76427` to the working tree, 25 files, 5776 patch
lines) after the rebase onto `origin/main` `8f76427` (#397) and the new commit `2d7e3ee`, which
answers the two MINOR findings of the previous APPROVED review (report bound to merge-base
`3f10f61`, fingerprint `d696b935...`).

- **Rebase**: `git range-diff` and a per-file comparison of added and removed lines show that the
  rebase changed no branch content; only context lines of `AI/DECISIONS.md` and
  `AI/VERIFICATION_MATRIX.md` moved, with the branch's matrix section now after main's PLF1
  section and a `---` separator.
- **`2d7e3ee`**: exactly the N-1 text fix (`Feature::OfflineAlerts.when_off()`) and the N-2 row
  renumbering to `OBT23` with the README snapshot evidence (matrix and spec §11). No test file
  touched. Both claims were checked against `crates/remote` and the matrix.

Gates (run in the worktree with `LIBONNXRUNTIME_NO_PKG_CONFIG=1`, an isolated
`CARGO_TARGET_DIR`, `CARGO_INCREMENTAL=0`, no debug info, `CARGO_BUILD_JOBS=4`):
`cargo clippy --locked -p soos-gui --all-targets --all-features -- -D warnings` exit 0;
`cargo test --locked -p soos-gui -p soos-invariants --all-features -- --test-threads=4` exit 0,
959 passed, 0 failed, 0 ignored (including `test_rac_capture_scenes_match_committed_assets`, with
`Docs/assets/` untouched, and 695 invariant tests).

No CRITICAL or MAJOR finding. N-1 and N-2 are resolved; the SUGGESTION, the documented
`ActiveState` follow-up and the earlier MINORs are carried forward.

## 2. Test Changes (mechanical listing from step 3)

Against the merge-base `8f76427`, all five test files are **new** (`feature_control_tests.rs`,
`onboarding_audit_tests.rs`, `onboarding_feature_tests.rs`, `onboarding_state_tests.rs`,
`onboarding_view_tests.rs`); the frozen patch has no `^-` assertion/test line, no new
`#[ignore]`/`#[cfg(any())]`/`should_panic`/tolerance, and no inline `mod tests` change. No test
that exists on `main` is modified. `2d7e3ee` touches no test file.

Intra-branch contract migration in `6c24ae4` (pre-rebase `c1853f2`; `git show 6c24ae4 -- crates/gui/tests`), owner
approved on 2026-10-15 and recorded in spec §11, ADR item (6) and matrix OBT5/OBT6/OBT23. Each
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

### Rebase Onto `8f76427` and Commit `2d7e3ee`
- *Did the rebase change the branch's content?* `origin/main` gained one commit since `3f10f61`:
  `8f76427` (#397), touching only `crates/daemon/tests/peer_limits_tests.rs`, walkthrough 210 and
  one matrix section. `git range-diff 3f10f61..c178093 8f76427..HEAD` shows commits 2 to 5 as `=`
  (identical patches) and commit 1 (`3693833` → `084a533`) as `!`, with differences only in
  context lines of `AI/DECISIONS.md` and `AI/VERIFICATION_MATRIX.md` and in the two excluded
  review reports. For every file of the frozen diff, the sorted set of added and removed lines
  vs the merge-base is identical before (`3f10f61..c178093`) and after (`8f76427..HEAD`) the
  rebase. `git diff c178093 HEAD` lists exactly the three files of #397. The branch's matrix
  section is appended after main's PLF1 section and a `---` separator; the branch removes no
  matrix line. PASS.
- *Is `2d7e3ee` limited to N-1 and N-2?* `git show --stat 2d7e3ee`: three files, 4 insertions and
  4 deletions: the `OfflineAlerts` `when_off()` string in `crates/gui/src/onboarding.rs`, the row
  ID and evidence cell in `AI/VERIFICATION_MATRIX.md`, and "new OBT23" in
  `AI/architect_spec_onboarding_tour.md`. No test file is touched, and no test pins the old
  wording (`grep` for "offline history"/"back-online" in `crates/gui/tests` finds nothing). PASS.
- *Does #397 interact with the GUI?* No: it is a daemon integration test change; `soos-gui` does
  not depend on `soos-daemon` test code. PASS.

### Logic & Architecture
- **Dependency check against `soos-remote` (base `8f76427`; `crates/remote` is unchanged by #397)**: `crates/remote/src/config.rs:106,357`
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
  long it was away") matches §2j of `Docs/REMOTE_COMPANION.md`. The "off" text now reads "Off: no
  offline history is recorded and no back-online notification is sent; push alerts keep working."
  Checked against `crates/remote` on the new base: `config.rs:974` (`enabled:
  file.offline_alerts.unwrap_or(false)`), `main.rs:236` (no offline settings when off) and
  `offline.rs:860` (disabled view): with the key off nothing is recorded, and push alerts are an
  independent key. Previous N-1 resolved. PASS.
- **`FeatureState::NotAvailable`** stays in the enum with its view label but no feature uses it;
  spec §11 records this deliberately. Acceptable.
- **Follow-up (non-blocking, documented in spec §11 and walkthrough 209)**: `try-restart` of the
  `Type=exec` unit still reports success before `soos-remote` parses its configuration, so a file
  refused for another reason would be reported as applied. With M-1 fixed, every file the GUI
  writes is accepted by `soos-remote` whenever the original was.

### PAM Concurrency & Deadlines
- `crates/pam` untouched. No new blocking path: `6c24ae4` adds only data (a key, a switch) to the
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
- `6c24ae4` and `2d7e3ee` change no manifest, lockfile, script or workflow. The branch's only dependency change
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
- The new matrix row is now `OBT23` (IDs OBT1 to OBT23 are unique; OBT19 is the original "tour
  state in the user's own tree" row), spec §11 says "new OBT23", and the row cites
  `readme_snapshots::test_rac_capture_scenes_match_committed_assets` for its capture clause.
  Previous N-2 resolved. PASS.

## 4. Detailed Findings & Action Items

Resolved since the previous review: N-1 (offline alerts "off" text) and N-2 (duplicate matrix ID,
now `OBT23` with the README snapshot evidence), both in `2d7e3ee`.

Carried forward (unchanged code):

- **[SUGGESTION]** `crates/gui/src/onboarding.rs:319` — a hand-written file with
  `offline_alerts = true` and `push_notifications = false` (already refused by `soos-remote`)
  shows offline alerts as On; the companion page shows the unit state, so this is cosmetic.
  Optionally show such a pair as Unknown or with a hint.
- **[MINOR, non-blocking follow-up, documented]** `crates/gui/src/feature_control.rs:760` — check
  `ActiveState` after `systemctl --user try-restart soos-remote.service` so that a configuration
  refused for any reason is never reported as success (spec §11, walkthrough 209).
- **[MINOR]** `crates/gui/src/feature_control.rs:423` (`bool_with_comment`) — a CRLF file gets
  mixed line endings on rewritten lines (still valid TOML).
- **[MINOR]** `crates/gui/src/feature_control.rs` (`disable_remote`) — read-edit-rename is not
  guarded against a concurrent same-user edit (last writer wins; no privilege impact).
- **[MINOR]** `crates/gui/src/onboarding.rs` (`TourStore`) — no `fsync` of the parent directory
  after `rename` (durability only).
- **[MINOR]** `crates/gui/src/app.rs` (`render_tour`) — in polkit store mode a pending startup
  `pkexec` dialog keeps the first-run tour from closing until it is answered.

## 5. Final Verdict

The rebase onto `8f76427` changed no branch content, `2d7e3ee` resolves N-1 and N-2 without
touching any test, the test migration reviewed earlier is unchanged, and clippy and the 959 tests
are green. Remaining findings are MINOR or SUGGESTION.

**VERDICT: APPROVED**
