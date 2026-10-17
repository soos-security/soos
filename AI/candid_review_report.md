# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `chore/companion-wording`
- **Base (merge-base)**: `74782b8`
- **Reviewed-Diff-Fingerprint**: `e32864ec11f16a7713afad0527313705361fab344ac0b0a1f76e70a88a5f673d`
- **Audited Files**: `AI/ARCHITECTURE.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_companion_wording.md`,
  `AI/auditor_constraints_companion_wording.md`, `AI/walkthroughs/228_companion_wording.md`,
  `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/REMOTE_COMPANION.md`, `README.md`, `scripts/setup.sh`,
  `tests/invariants/src/companion_wording_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

Wording-only change (GitHub #423): the `setup.sh` help and closing companion summary, the README security
model, `Docs/REMOTE_COMPANION.md` §2b, `Docs/PACKAGING_AND_PROVISIONING.md` "Defaults" and two
`AI/ARCHITECTURE.md` companion table row headers now match the code: `soos-gui` (Features) has switches only for
`allow_unlock`, `allow_funnel`, `push_notifications`, `camera_view` and `offline_alerts`
(`crates/gui/src/feature_control.rs` `RemoteSwitch::key`), so the failed-password alerts are turned off with
`--password-alerts off` or in `remote.toml`. A new std-only invariant module (CWD1–CWD5) locks the wording.
No runtime code, shell logic, option, exit code or dependency changes. No pre-existing test assertion is
removed or changed. No MAJOR or CRITICAL finding.

## 2. Test Changes

Mechanical listing on the frozen patch:

- Test files touched: `tests/invariants/src/companion_wording_contract.rs` (new), `tests/invariants/src/lib.rs`
  (one new `#[cfg(all(test, unix))] mod companion_wording_contract;` declaration with a doc comment).
- Removed/changed check lines (`^-` with `assert`/`#[test]`/...): one hit, patch line 17, which is the
  removed `AI/ARCHITECTURE.md` "Unlock (opt-in)" table row (the word "assertion" in prose), not a test.
- New escape hatches (`#[ignore]`, `should_panic`, `tolerance`, `epsilon`, `cfg(any())`): none.
- Inline `mod tests` changes: none.
- `git diff 74782b8 HEAD -- tests 'crates/*/tests' android/app/src/test`: 281 insertions, 0 deletions.

Verdict: additions only; no pre-existing assertion of `origin/main` modified, weakened or deleted.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Claim "Funnel, remote unlock, push, offline alerts and the camera view can be turned off later in soos-gui
  (Features), which also turns off the features that need them": checked against `RemoteSwitch::key` and
  `RemoteSwitch::keys_to_clear` (Funnel → `camera_view_funnel`, Push → `offline_alerts`, Camera view →
  `camera_view_funnel`). PASS. `[android]` (needs `allow_funnel`) is never written by `setup.sh` (grep), so
  the help sentence is accurate for setup-managed files.
- Claim "set password_alerts, push_notifications and offline_alerts to false in remote.toml": consistent with
  `crates/remote/src/config.rs` (`push_notifications requires password_alerts`, `offline_alerts requires
  push_notifications`, exit 78). PASS.
- Closing summary "run setup again with --password-alerts off": `resolve_features` sets `password_alerts`
  false and cascades push and offline alerts to false via `FEATURE_PREREQS` ("needs password alerts"); the
  line is printed only for a setup-managed configuration, since `print_companion_guide` returns early when
  `CONFIG_ACTION == kept` (user-written file, which setup never edits). PASS.
- `Docs/REMOTE_COMPANION.md`: `allow_funnel` code default is `file.allow_funnel.unwrap_or(false)` and Funnel
  requests are `403` while it is false; `setup.sh` `COMPANION_KEYS` includes `allow_funnel` default true. PASS.
- `AI/ARCHITECTURE.md` "Push sender" / "Unlock" headers "off by default in code; on with `scripts/setup.sh`":
  `push_notifications.unwrap_or(false)`, `allow_unlock` default false; setup writes both true and enables
  `soos-push-sender.service` with push. Cell text byte-identical apart from the header. PASS.
- README: "turning push off also sets `offline_alerts` to `false`" matches `keys_to_clear(Push)`. PASS.
- `--help` rendered (`bash scripts/setup.sh --help`): the new paragraph prints with no expansion artefact; the
  heredoc text contains no `$` or backtick. `bash -n` passes. PASS.

### PAM Concurrency & Deadlines

No change under `crates/pam` or any runtime crate. Not applicable; PASS.

### Panic Safety & Fail-Closed

No production code changed. The new test module uses `expect`/`panic!` under an explicit test-only
`#![allow(...)]`, consistent with the other invariant modules. PASS.

### Test Integrity & Anti-Weakening

- See §2: additions only.
- New tests can fail on a plausible wrong state: CWD5 proves the detector flags the old sentence of both the
  help and the summary; CWD2 fails if a `password_alerts` switch is added (forcing the help to be updated);
  CWD3 checks exactly one "Push sender" and one "Unlock" row and that the Unlock row keeps `default \`false\``.
- Scoped run: `cargo test -p soos-invariants -- --test-threads=3` → 776 passed, 0 failed (CWD1–CWD5 included,
  SET and RDM contracts green). `cargo clippy -p soos-invariants --all-targets -- -D warnings` clean;
  `cargo fmt --check -p soos-invariants` clean. PASS.

### Memory, Bounds & Secrets

No new I/O beyond test-time reads of repository files. No password, identity, frame or embedding appears in
new text (the help names configuration keys only). PASS.

### Supply Chain & Automation

`scripts/setup.sh` changes only a heredoc paragraph and one quoted `info` string: no command, variable,
option or exit code changed; no new command word (SET2/SET3 green). No `Cargo.*`, workflow or hook change.
PASS.

### English-Only Policy

All new text in English. Commit subject `chore(docs): align companion off-switch and opt-in wording`
(one scope, 59 characters). PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `Docs/PACKAGING_AND_PROVISIONING.md:97` — "each can be switched off later in soos-gui
  except the failed-password alerts": camera view over Funnel (`camera_view_funnel`) has no switch of its own
  either; it goes off only through the Funnel or camera view switch. Accurate in effect, slightly imprecise.
- **[SUGGESTION]** `AI/walkthroughs/228_companion_wording.md:28` — "`crates/remote/src/config.rs` (every key
  defaults to `false`)": true for the companion feature keys, but `battery_status` defaults to `true`;
  "every feature key" would be exact.
- **[SUGGESTION]** `scripts/setup.sh:175` — "which also turns off the features that need them" does not
  cover a manually added `[android]` table (needs `allow_funnel`; the GUI Funnel switch does not remove it).
  Not written by setup, so out of this help's scope; pre-existing GUI behaviour.

## 5. Final Verdict

**VERDICT: APPROVED**
