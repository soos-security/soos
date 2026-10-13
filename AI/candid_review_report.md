# Candid Review Report

- **Date**: 2026-10-13
- **Target Branch**: `feat/onboarding-tour`
- **Base (merge-base)**: `6f32ef8`
- **Reviewed-Diff-Fingerprint**: `335215c440578e0a3d1241a5996cb7aa32427aa718dc06a6c2c11dae6d3e3186`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_onboarding_tour.md`, `AI/auditor_constraints_onboarding_tour.md`, `AI/tester_contract_onboarding_tour.md`, `AI/walkthroughs/209_onboarding_tour.md`, `Cargo.lock`, `Docs/GUI_APPLICATION.md`, `Docs/NETWORK_LOCK.md`, `Docs/REMOTE_COMPANION.md`, `crates/gui/Cargo.toml`, `crates/gui/src/app.rs`, `crates/gui/src/feature_control.rs`, `crates/gui/src/header.rs`, `crates/gui/src/lib.rs`, `crates/gui/src/onboarding.rs`, `crates/gui/src/onboarding_view.rs`, `crates/gui/src/privileged.rs`, `crates/gui/tests/feature_control_tests.rs`, `crates/gui/tests/onboarding_audit_tests.rs`, `crates/gui/tests/onboarding_feature_tests.rs`, `crates/gui/tests/onboarding_state_tests.rs`, `crates/gui/tests/onboarding_view_tests.rs`

## 1. Executive Summary

The diff adds a first-run onboarding tour to `soos-gui` (GitHub #379, PR 1 of #363): one page per
feature with a plain-English explanation, the probed state and an on/off control, a per-user
`gui-tour.json` state file, and a header "Features" button that reopens the tour. Root-owned state
changes only through `pkexec` with absolute programs and fixed argument vectors over a closed enum
(`SystemFlag::{Global, Presence}` and `soos-admin network-lock install|uninstall`); the companion
is switched through `systemctl --user`; `remote.toml` keys are only ever turned off by an exact
line rewrite verified by a TOML parse. Every process and write runs off the UI thread. No CRITICAL
or MAJOR defect was found after an adversarial pass; four MINOR robustness observations are listed.
`cargo clippy -p soos-gui --all-targets -- -D warnings`, `cargo test -p soos-gui` and
`cargo test -p soos-invariants` (604 passed) are green locally.

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: five **new** files only (`crates/gui/tests/feature_control_tests.rs`,
  `onboarding_audit_tests.rs`, `onboarding_feature_tests.rs`, `onboarding_state_tests.rs`,
  `onboarding_view_tests.rs`), all status `A` in the index.
- Removed/changed assertions (`^-` with assert/#[test]/proptest/should_panic): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance/epsilon): **none**.
- Inline `mod tests` added or removed: **none**.
- Conclusion: no pre-existing test was modified, weakened or deleted.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Owner decisions: every feature is explained and offers an off route (an action, or the exact
  command when the GUI cannot or must not act); presence auto-unlock is toggleable at any time
  via `/etc/soos/presence.disable` (same flag names as `crates/daemon/src/presence/mod.rs`);
  copy is sober (no exclamation marks/emoji, pinned by tests). PASS.
- Flag semantics: `entry_exists` uses `symlink_metadata` (any entry counts), matching the
  daemon's switch rule; a stat error is `Unknown`, never `On`. PASS.
- `PresenceUnlock` state requires all three facts known to show `On`; any `Some(true)` disable
  shows `Off`. Turning it on while `daemon.toml` keeps it off yields the root command instead of
  a misleading flag removal. PASS.
- Wi-Fi lock: "on" never runs `install` blind — only when the rfkill rule is known absent and
  the polkit rule state is known, so an installed rfkill rule is never silently dropped. PASS.
- Remote dependency cascade checked against `crates/remote/src/config.rs`: `camera_view_funnel`
  requires both `camera_view` and `allow_funnel` and is cleared with either; `push_notifications`
  requires `password_alerts` (clearing push alone is valid); nothing depends on `allow_unlock`.
  The edited file therefore always stays loadable by `soos-remote`. PASS.
- Camera view state combines the user key with the daemon `[preview]` gate (`enabled`,
  `remote_view`, uid in `allowed_uids`), matching `crates/daemon/src/preview.rs` defaults. PASS.
- `FeatureRunner::poll`: `finished` is sampled before draining the channel, so a finished worker
  with an outcome is never misreported; a worker that dies without an outcome yields an error
  message for non-refresh actions. Refreshes requested while busy are coalesced. PASS.
- One Polkit dialog at a time: every `TaskRunner` submission in `app.rs` now goes through
  `submit_task` (grep shows no remaining direct `tasks.submit`), and `TourEvent::Apply` refuses
  while either runner is busy. PASS.

### PAM Concurrency & Deadlines
- `crates/pam` is untouched. In the GUI, `systemctl --user` calls are bounded by a 3 s deadline
  with kill+reap; `pkexec` intentionally has no deadline (the user may sit in the Polkit dialog)
  but runs on the `soos-gui-features` worker; the UI thread only polls a channel. Tour state is
  written on a short-lived thread. The startup tour-state read is a bounded `O_NONBLOCK|O_NOFOLLOW`
  open of a file of at most 4 KiB. PASS.

### Panic Safety & Fail-Closed
- Scanned the three new modules for `unwrap`/`expect`/`panic!`/slice indexing: none. All three
  carry `#![forbid(unsafe_code)]`. Errors become user-facing messages; no error path reports a
  feature as `On`. PASS.

### Test Integrity & Anti-Weakening
- Only new tests; they exercise exact argv vectors, the off-only editor (comments, tables
  refused, non-boolean values refused, cascade), the state machine, the tour store (0600, symlink
  refusal, foreign-owner refusal) and the runner with injected executors. A plausible wrong
  implementation (e.g. editing a key inside a multi-line string, adding keys, following a
  symlink) would fail them or the verification step. PASS.

### Memory, Bounds & Secrets
- Privilege model: `pkexec` argv built only from `const` absolute paths and a `Copy` enum; no
  shell; no user-controlled string (env, file contents, UI text) reaches any argv.
  `SOOS_ADMIN_PROGRAM` follows the build-time-validated `SOOS_BINDIR` like `SOOS_ENROLL_PROGRAM`.
  The GUI process never writes under `/etc`. PASS.
- `remote.toml` editor attack scenarios tried: quoted key `"allow_unlock" = true` (line not
  matched, verification sees `true`, `VerifyFailed`, nothing written); target key text inside a
  multi-line string (other-key equality check fails, nothing written); `trueish`/`"true"` values
  (`UnsupportedValue`); tables/array-of-tables (`UnsupportedLayout`); duplicate keys (parse error);
  dotted keys (not matched, untouched). Writes are a fresh `O_CREAT|O_EXCL|O_NOFOLLOW` 0600 temp
  file in the same directory, `fsync`, `rename` (rename replaces a link itself, never its target),
  temp removed on error; a symlinked `remote.toml` is refused for both read and write. Refused
  when running as root. PASS.
- Tour state: 0600 file in a 0700 directory, atomic, at most 4096 bytes read, symlink refused,
  `owned_by_euid` refuses another user's tree (a root GUI started with `sudo -E` keeps the user's
  `HOME`, gets `NotOwned`, and the tour is not auto-shown). PASS.
- Child output: stdout captured only for `systemctl show` (at most 1024 bytes via
  `read_bounded`), stderr always null, stdin null. PASS.
- Logs: `tracing::warn!(?action, ...)` logs only the closed-set action enum; error messages carry
  `io::ErrorKind`, file names and exit codes, never file contents or config values. PASS.

### Supply Chain & Automation
- `toml` added to `crates/gui` as `{ workspace = true }`; it is already in the lockfile (only the
  dependency edge is added to `Cargo.lock`). No new crate, no CI/script change. PASS.

### English-Only Policy
- All UI strings, comments and docs are English; non-ASCII scan of the new modules is clean. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/gui/src/feature_control.rs:424` — on a CRLF file, `bool_with_comment`
  trims `\r` and the edited line is written with `\n` only, leaving mixed line endings (still
  valid TOML). Preserve the original `\r` on rewritten lines.
- **[MINOR]** `crates/gui/src/feature_control.rs:747` — read-edit-rename is not guarded against a
  concurrent edit by the same user between read and rename (last writer wins). Same-user only, no
  privilege impact; optionally compare mtime/inode before rename.
- **[MINOR]** `crates/gui/src/onboarding.rs:856` — no `fsync` of the parent directory after
  `rename`, so the rename may not survive a crash. Durability only.
- **[MINOR]** `crates/gui/src/app.rs` (`render_tour`, `busy` includes `tasks.is_busy()`) — in
  polkit store mode the startup `ListProfiles` `pkexec` dialog makes Skip/Done/Escape unavailable
  on the first-run tour until that dialog is answered, and a `pkexec` that never returns (hung
  agent) keeps the modal open. Consider allowing close while only a non-feature task is in flight.

## 5. Final Verdict

**VERDICT: APPROVED**
