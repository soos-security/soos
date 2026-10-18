# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `feat/gui-android-safe-keys`
- **Base (merge-base)**: `144d409`
- **Reviewed-Diff-Fingerprint**: `f6fed2c7bcb48f524bfa94c967fc96639c839613a19fd1623a63b42fc6cdda3e`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/231_gui_android_safe_keys.md`, `Docs/GUI_APPLICATION.md`, `Docs/REMOTE_COMPANION.md`, `crates/gui/src/feature_control.rs`, `crates/gui/src/onboarding.rs`, `crates/gui/tests/remote_android_edit_tests.rs`

## 1. Executive Summary

`feature_control::disable_remote_keys` used to refuse any `remote.toml` with a line starting with `[`. It now accepts exactly one `[android]` table as the last section: top-level `key = true` lines above it are rewritten to `key = false`, the table lines are copied unchanged, and a target key inside the table, any second header, or any other header is `UnsupportedLayout`. A new `RemoteEditError::FunnelRequiredByAndroid` refuses the Internet access switch (`allow_funnel`) whenever the parsed file has an `android` key, before any edit, write or restart. `onboarding::toggle_route` appends a fixed note to the shown Internet access "off" command. The existing TOML verification (target keys false or absent, every other top-level key equal, `android` included, no key added) is unchanged and still runs on every result. One new test file (GAK1 to GAK6), no existing test touched. No PAM, IPC, Cargo or automation change. `cargo test --locked -p soos-gui -p soos-invariants` (all green, 771 lib tests plus every integration binary), `cargo clippy --locked -p soos-gui -p soos-invariants --all-targets -- -D warnings` and `cargo fmt --all -- --check` were re-run on this tree and pass. No CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing on `target/candid_diff.patch`:

- Test files touched: `crates/gui/tests/remote_android_edit_tests.rs` only (new file).
- Removed or changed assertions, `#[test]`, `proptest!`, `#[should_panic]` lines (`^-`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline `mod tests` added or removed: **none**.

The pre-existing `test_obt_disable_refuses_unsupported_layouts` (`crates/gui/tests/feature_control_tests.rs`, `[extra]` table) is untouched and still passes: a non-`[android]` header is still `UnsupportedLayout`. No justification is needed because nothing pre-existing changed.

## 3. Deep Reasoning Audit

### Logic & Architecture

Scenarios attempted:

- **Writing a file `soos-remote` refuses.** I listed every cross-key rule in `crates/remote/src/config.rs` (`allow_funnel` needs `rp_id`; `push_notifications` needs `password_alerts` and `rp_id`; `camera_view` needs `rp_id`; `camera_view_funnel` needs `camera_view` and `allow_funnel`; `offline_alerts` needs `push_notifications`; `[android]` needs `rp_id` and `allow_funnel`). The edit only ever sets keys to `false` and never touches `rp_id` or `password_alerts`. `keys_to_clear` cascades `offline_alerts` with Push and `camera_view_funnel` with CameraView and Funnel, so no "requires" rule can be broken by turning a key off, except `[android]` requiring `allow_funnel`, which is now refused up front. The edit cannot add an unknown key (verified: `edited.keys()` is a subset of `original`). PASS.
- **Funnel slipping through.** The guard is at the top of `disable_remote_keys`, which is the only writer path (`SystemFeatureExecutor` → `DisableRemote`). It triggers on any `android` key, including an inline table `android = { ... }` or dotted keys `android.package = ...`, so those layouts cannot bypass it. GAK5 shows nothing is written and no restart runs. PASS.
- **A table hidden so a table key is treated as top-level.** TOML headers must open their own line, so every header line is seen by `trimmed.starts_with('[')`. `[[android]]`, `[android.x]`, `[ "android" ]`, `[android]x`, `[android]]` and a second `[android]` all fail `is_android_header` or hit `in_table` and are refused. A multi-line string or array line that starts with `[` is refused (fail closed). A multi-line string containing a fake `[android]` line sets `in_table` early, so later edits are refused (fail closed). A multi-line string containing `allow_unlock = true` text would be rewritten, but the verification sees the string value change and returns `VerifyFailed` (behavior unchanged). PASS.
- **A target key inside `[android]`.** Refused whatever its value (`Some(_) if in_table`), even though `soos-remote` would already refuse it as an unknown `[android]` key. Bare-key detection only; a quoted key inside the table is copied unchanged and does not affect the top-level verification. PASS.
- **Line preservation.** Header and in-table lines are pushed as the original `line` (newline included). Above the table, only target lines are rebuilt; the rest are copied. GAK2 checks equal line count, per-line equality for non-target lines, and the byte-identical `[android]` suffix. PASS.
- **Routing.** The note is appended only for `RemoteSwitch::Funnel` on the shown-command path. Its wording is conditional ("Not while remote.toml has an [android] table"), so it stays correct when no `[android]` exists. `FeatureSnapshot` and `RemoteKeys` are unchanged. PASS.

### PAM Concurrency & Deadlines

No `crates/pam` change. No new blocking call: the edit is pure string work over text that was already read through the existing bounded reader. PASS (not applicable).

### Panic Safety & Fail-Closed

`is_android_header` uses `strip_prefix`, `find`, `get` and `saturating_add` only. There is no `unwrap`, `expect`, indexing or unchecked arithmetic in the new production code. Every new branch either copies a line or returns an error before any write. No path turns a refusal into a write or restart. PASS.

### Test Integrity & Anti-Weakening

The new tests would fail against plausible wrong implementations:

- treating a key inside `[android]` as top-level (GAK4 rows 5 and 6);
- accepting a second or other header (GAK4);
- letting Funnel through, or only refusing it when the key is on (GAK3, already-off case);
- dropping or reformatting table lines (GAK2 byte-identical suffix);
- writing or restarting after a refusal (GAK5).

The test-only rule checker `load` mirrors the `soos-remote` rules listed above, because `soos-remote` must stay a leaf crate (RMC-S4). GAK1 proves the checker rejects the broken variants. PASS.

### Memory, Bounds & Secrets

`String::with_capacity(text.len())`, and the output is at most a few bytes per edited line longer, so it stays bounded by the input. Error messages are fixed strings, with no file content, fingerprint or package. Writes still go through `write_user_file_atomically` (0600, same-directory temp file, rename, symlink refused), confirmed by GAK5. PASS.

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. The walkthrough records that a dev-dependency on `soos-remote` was tried and dropped. The manifest is unchanged in the diff. PASS.

### English-Only Policy

All code, comments, error strings, docs, the ADR amendment, matrix rows and the walkthrough are in English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/gui/src/feature_control.rs:551` (`bool_with_comment`, unchanged by this diff): on a CRLF file, a rewritten line loses its `\r`, because the comment is `trim_end_matches('\r')` and the newline pushed is `"\n"`. The TOML stays valid and `soos-remote` accepts it, but the file gets mixed line endings. This is out of scope for #429 and could be a follow-up.
- **[SUGGESTION]** `crates/gui/tests/remote_android_edit_tests.rs` (`load`): the rule checker is a hand-written mirror of `soos-remote`'s cross-key rules and can drift if `crates/remote/src/config.rs` gains a rule. An invariant that cross-checks the `ConfigError` "requires" variants against the mirror would keep them aligned. Optional.
- **[SUGGESTION]** `Docs/GUI_APPLICATION.md` / `Docs/REMOTE_COMPANION.md`: the docs accurately say "a file ending with an `[android]` table". They could add that an inline `android = { ... }` table is also handled (edited, and Funnel refused), which the code supports. Optional.

## 5. Final Verdict

**VERDICT: APPROVED**
