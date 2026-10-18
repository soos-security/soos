# Candid Review Report

- **Date**: 2026-10-18
- **Target Branch**: `chore/companion-wording`
- **Base (merge-base)**: `74782b8`
- **Reviewed-Diff-Fingerprint**: `e32864ec11f16a7713afad0527313705361fab344ac0b0a1f76e70a88a5f673d`
- **Audited Files**: `AI/ARCHITECTURE.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_companion_wording.md`,
  `AI/auditor_constraints_companion_wording.md`, `AI/walkthroughs/228_companion_wording.md`,
  `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/REMOTE_COMPANION.md`, `README.md`, `scripts/setup.sh`,
  `tests/invariants/src/companion_wording_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

Wording-only change (GitHub #423): the `setup.sh` help paragraph and closing companion summary, two
`AI/ARCHITECTURE.md` table row headers, one `Docs/REMOTE_COMPANION.md` example comment, one
`Docs/PACKAGING_AND_PROVISIONING.md` sentence and one README paragraph now state that `soos-gui` has
switches for Funnel, remote unlock, push, offline alerts and the camera view, but none for the
failed-password alerts, and stop calling setup-enabled keys "opt-in". A new invariant module
(CWD1–CWD5) pins the wording. Every factual claim was checked against the code; no runtime behaviour
changes. No CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/companion_wording_contract.rs` (new, 274 lines added),
  `tests/invariants/src/lib.rs` (+7 lines: doc comment and `#[cfg(all(test, unix))] mod companion_wording_contract;`).
- `git diff --stat 74782b8 HEAD -- tests 'crates/*/tests' android/app/src/test`: 281 insertions, 0 deletions.
- Removed/changed checks (`^-` with `assert`/`#[test]`/...): one hit, patch line 17, which is the old
  `AI/ARCHITECTURE.md` Unlock row (the word "assertion" in prose), not a test. No test line removed.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.

Conclusion: no pre-existing assertion of `origin/main` is removed, weakened or changed.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Claim "switches for Funnel, remote unlock, push, offline alerts, camera view; none for password
  alerts": `RemoteSwitch::key` (`crates/gui/src/feature_control.rs:101`) maps exactly
  `allow_unlock`, `allow_funnel`, `push_notifications`, `camera_view`, `offline_alerts` → PASS.
- Claim "also turns off the features that need them" / README "turning push off also sets
  `offline_alerts` to `false`": `RemoteSwitch::keys_to_clear` (line 114) clears `offline_alerts` with
  Push and `camera_view_funnel` with Funnel and Camera view → PASS.
- Claim "`--password-alerts off` turns them off" (and push/offline with it): `resolve_features` in
  `setup.sh` with `FEATURE_PREREQS` sets dependants to `false` ("needs password alerts") unless the
  dependant flag is explicitly `on` (usage error) → PASS.
- Claim "set `password_alerts`, `push_notifications` and `offline_alerts` to `false` in `remote.toml`":
  `config.rs` errors `push_notifications requires password_alerts` and `offline_alerts requires
  push_notifications`; no other key requires `password_alerts` (`[android]` requires only `rp_id` and
  `allow_funnel`) → PASS, the three-key set is complete.
- Claim "code default false keeps Funnel requests at 403; scripts/setup.sh writes true", ARCHITECTURE
  "off by default in code; on with `scripts/setup.sh`" for push sender and unlock: `allow_funnel`,
  `allow_unlock`, `push_notifications` all `unwrap_or(false)` (`config.rs:948`, `988`, `774`);
  `COMPANION_KEYS` default `value=true` in `resolve_features`; setup enables
  `soos-push-sender.service` when `push_notifications` is true (`setup.sh:1338-1340`) → PASS.
- `setup.sh` change is text only: the heredoc paragraph and one double-quoted `info` string; no `$`,
  backtick or unescaped `"` added; `bash -n` passes; `--help` renders the new paragraph verbatim;
  every option line unchanged → PASS.
- Detector logic (CWD1): split on ". " and ";" of the collapsed non-comment source; the new help's
  "Every feature below is on by default (owner decision)" unit has no "soos-gui", so not flagged;
  CWD5 proves the old wording is flagged → PASS.

### PAM Concurrency & Deadlines
- No file under `crates/pam` or any runtime crate changed → PASS (not applicable).

### Panic Safety & Fail-Closed
- Only test code uses `expect`/`panic!`/indexing, under the established module-level `#![allow(...)]`
  with `reason`; no production code changed → PASS.

### Test Integrity & Anti-Weakening
- Section 2: additions only. The new tests read real repository files and fail on the base wording
  (CWD1, CWD3, CWD4 red on `74782b8` per the detector unit test and the needles absent from the base
  text); CWD2 guards the premise so a future GUI switch forces the help to be revisited → PASS.

### Memory, Bounds & Secrets
- No I/O beyond test-time `read_to_string` of repository files; no password, identity or secret in any
  new text → PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or `.githooks/` change; `scripts/setup.sh` change is a help/
  summary string only → PASS.

### English-Only Policy
- All added text is English; non-ASCII characters are typographic (dashes, arrows, `≤`, `§`, status
  marks) already used across the docs → PASS.

### Verification re-run (this review)
- `cargo test -p soos-invariants -- --test-threads=3` (scoped env from the task): 776 passed, 0 failed.
- `cargo clippy -p soos-invariants --all-targets -- -D warnings`: clean.
- `cargo fmt --check -p soos-invariants`: clean.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `AI/walkthroughs/228_companion_wording.md:28` — "`crates/remote/src/config.rs`
  (every key defaults to `false`)" is broader than the code: `battery_status` defaults to `true`
  (`config.rs:998`). The statement is true for the seven setup-managed companion feature keys; consider
  "every companion feature key".
- **[SUGGESTION]** `scripts/setup.sh:177` — the `remote.toml` route needs a `soos-remote` restart to take
  effect (as `Docs/REMOTE_COMPANION.md` says for configuration in general); the help could say so.
  Not required: the summary line points to the setup re-run path, which try-restarts `soos-remote.service` (`setup.sh:2075`, `2213`).

## 5. Final Verdict

**VERDICT: APPROVED**
