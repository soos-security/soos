# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `feat/gui-android-safe-keys`
- **Base (merge-base)**: `4671888` (origin/main after #422, #424 and #428)
- **Reviewed-Diff-Fingerprint**: `f457a30fa1e780ad637e8a79f016afa1a049047e638de75612f50f12656bfd50`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/231_gui_android_safe_keys.md`, `Docs/GUI_APPLICATION.md`, `Docs/REMOTE_COMPANION.md`, `crates/gui/src/feature_control.rs`, `crates/gui/src/onboarding.rs`, `crates/gui/tests/remote_android_edit_tests.rs`

## 1. Executive Summary

Independent re-review after the rebase onto `origin/main` `4671888`. The raw diff against the new
merge base (778 lines, 8 files) was read in full; it contains only the #429 change, no hunk of
#422, #424 or #428 leaks into it.

`feature_control::disable_remote_keys` used to refuse any `remote.toml` with a line starting with
`[`. It now accepts exactly one `[android]` header (optionally indented, spaced inside the
brackets, followed by a `#` comment): top-level `key = true` lines above it are rewritten to
`key = false`, the table lines are copied byte for byte, and a second header, any other header, or
a key of the switch inside the table is `UnsupportedLayout`. A new
`RemoteEditError::FunnelRequiredByAndroid` refuses the Internet access switch whenever the parsed
file has an `android` key, before any edit, write or restart. `onboarding::toggle_route` appends a
fixed note to the shown Internet access "off" command. The existing TOML verification (target keys
`false` or absent, every other top-level key equal, `android` included, no key added) is unchanged
and runs on every result.

On this tree: `cargo test --locked -p soos-gui -p soos-invariants -- --test-threads=4` passes
(1070 tests, 0 failed, including `companion_wording_contract` CWD1–CWD5, `readme_contract`,
`test_rmc_s4_remote_is_a_leaf_crate`, the OBT and GAK suites); `cargo clippy --locked -p soos-gui
-p soos-invariants --all-targets -- -D warnings` and `cargo fmt --all -- --check` pass. No CRITICAL
or MAJOR finding. No documentation fix was needed after the rebase.

## 2. Test Changes

Mechanical listing on `target/candid_diff.patch`:

- Test files touched: one new file, `crates/gui/tests/remote_android_edit_tests.rs` (GAK1–GAK6).
- Removed or changed assertions (`^-` with `assert`, `#[test]`, `proptest!`, `should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` added or removed: **none**.

`crates/gui/tests/feature_control_tests.rs` is not in the diff:
`test_obt_disable_refuses_unsupported_layouts` (an `[extra]` table, a quoted `"true"`, invalid TOML)
is byte-identical to main and passes, because `[extra]` is not `[android]`. The OBT9 matrix row is
not modified; the new component section states that OBT9 still holds for every table other than one
`[android]` table, which matches the code and the ADR amendment.

## 3. Deep Reasoning Audit

### Logic & Architecture

Scenarios attempted:

- **Funnel under `[android]`** (on, already off, absent): refused with `FunnelRequiredByAndroid`
  before the line walk, so nothing is written and `try-restart` is never called (GAK3, GAK5). This
  matches the `soos-remote` error "[android] requires allow_funnel" (`crates/remote/src/config.rs`).
  A top-level dotted key `android.package = ...` also creates the `android` key, so the pre-check
  still refuses Funnel there. PASS.
- **Safe switches never produce a file `soos-remote` refuses**: the only cross-key rules a disable
  can violate run from dependent to prerequisite. `Push` clears `offline_alerts` ("offline_alerts
  requires push_notifications"); `CameraView` clears `camera_view_funnel` ("requires camera_view");
  `Unlock` and `OfflineAlerts` are not prerequisites of anything; `[android]` needs `rp_id` and
  `allow_funnel`, neither of which a safe switch touches. `password_alerts` is never touched. GAK2
  re-checks each result with the mirrored rule checker. PASS.
- **Line preservation**: non-target lines are pushed unchanged (`out.push_str(line)`), the
  `[android]` header and every table line included; GAK2 asserts line-by-line equality and that the
  output ends with the original table text. PASS.
- **Header spoofing**: `[android]x`, `[android]]`, `[android.extra]`, `[[android]]`, `["android"]`,
  `[extra]` before or after `[android]`, and a second `[android]` all reach `UnsupportedLayout`
  (the `in_table` guard rejects any header after the first). `[android] # c ]` is accepted and is
  valid TOML. PASS.
- **Multi-line string hiding a header or a key**: a `"""` string containing `[android]` turns on
  table mode, so a later target line is refused; a string containing `allow_unlock = true` at top
  level would be rewritten, but the verification parse then sees the string value changed and
  returns `VerifyFailed` (pre-existing OBT9 guard, unchanged). PASS.
- **Verification covers the table**: `android` is a top-level key of the parsed table, so the
  "every other key equal" check proves the table was not altered. PASS.

### PAM Concurrency & Deadlines

No change under `crates/pam`, no daemon or IPC change. N/A — PASS.

### Panic Safety & Fail-Closed

`is_android_header` uses `strip_prefix`, `find`, `get` and `saturating_add` only; no `unwrap`,
`expect`, indexing or unchecked arithmetic in the new production code. Every refusal returns an
error before `write_user_file_atomically`. No path turns an error into a success. PASS.

### Test Integrity & Anti-Weakening

No pre-existing assertion changed (section 2). The new tests can fail against plausible wrong
implementations: accepting any table (GAK4 fails), editing inside `[android]` (GAK4), letting
Funnel through (GAK3/GAK5), dropping the `offline_alerts` cascade (GAK2 rule checker), mangling
the table (GAK2 suffix and parsed-table equality). GAK1 proves the rule checker rejects the broken
variants, so it is not vacuous. The checker mirrors `soos-remote` rules instead of calling
`parse_config_with_android`, because RMC-S4 forbids depending on `soos-remote`; this is documented
and the mirrored rules match `config.rs`. PASS.

### Memory, Bounds & Secrets

The output buffer is bounded by the input length plus at most one byte per rewritten line
(`false` vs `true`); the input is still read through the existing bounded reader. Error messages
are fixed strings and carry no file content; nothing is logged. The write is still the atomic
0600, symlink-refusing `write_user_file_atomically` (GAK5 checks mode 0600). PASS.

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. N/A — PASS.

### English-Only Policy

All code, comments, messages and docs in the diff are English. PASS.

### Consistency With #424 And #428 (After The Rebase)

- `README.md` (from #424) says Funnel, remote unlock, the camera view, push and offline alerts each
  have an off switch in `soos-gui`, and that an `[android]` table needs `allow_funnel`. Both stay
  true: the switch exists and refuses with a specific message when `[android]` is configured. No
  #424 text claims that the GUI cannot change a file with `[android]`, so nothing had to be updated.
- `Docs/REMOTE_COMPANION.md` §5 (this branch) now lists `offline_alerts` and the `[android]`
  behaviour, consistent with #424's §2k wording and with #428's native tour text, which both say
  `allow_funnel = false` goes with `camera_view_funnel = false` and no `[android]` table
  (`crates/remote/assets/app.js`, `android/app/src/main/res/values/strings.xml`).
- `Docs/GUI_APPLICATION.md` §1e row and paragraph match the code; `AI/ARCHITECTURE.md` and
  `scripts/setup.sh` say nothing about the GUI and tables.
- `companion_wording_contract` (CWD1–CWD5) and `readme_contract` pass on this tree. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `AI/VERIFICATION_MATRIX.md:2726` — the OBT9 criterion text still reads "refuses
  tables". It is qualified by the new `gui-android-safe-keys` section ("OBT9 still holds for every
  table other than one `[android]` table") and the test it cites is unchanged, so this is not
  wrong; a later matrix pass could add "other than one `[android]` table" to the OBT9 cell.
- **[SUGGESTION]** `README.md:56` — "all but the failed-password alerts can be turned off in
  soos-gui" does not mention that Internet access needs the `[android]` table removed first. The
  security section (line 303) states the `[android]` requirement, so the README is not
  contradicted; an owner may want the short sentence to say it too.

## 5. Final Verdict

**VERDICT: APPROVED**
