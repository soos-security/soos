# Candid Review Report

- **Date**: 2026-11-20
- **Target Branch**: `fix/setup-remote-defaults`
- **Base (merge-base)**: `0cce947`
- **Reviewed-Diff-Fingerprint**: `45502ca6f051bb8d007e662993516ea5e05015017a4f8c823aabd49f24552b00`
- **Audited Files**: `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/walkthroughs/259_setup_remote_defaults.md`, `Docs/GUI_APPLICATION.md`,
  `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/REMOTE_COMPANION.md`, `README.md`,
  `android/app/src/main/res/values/strings.xml`, `crates/gui/src/onboarding.rs`,
  `crates/gui/src/onboarding_view.rs`, `crates/gui/tests/onboarding_feature_tests.rs`,
  `crates/remote/assets/app.js`, `crates/remote/tests/setup_render_tests.rs`,
  `scripts/install_remote.sh`, `scripts/setup.sh`, `tests/fixtures/setup/remote.toml`,
  `tests/fixtures/setup/remote_android.toml`, `tests/fixtures/setup/remote_carried.toml`,
  `tests/invariants/src/readme_contract.rs`, `tests/invariants/src/setup_deps_contract.rs`,
  `tests/invariants/src/setup_user_phase_contract.rs`

## 1. Executive Summary

Security review batch 3 (GitHub #480): findings #548 GUI-03, #597 RCFG-02, #625 SETB-01,
#626 SETB-02, #627 SETB-03, #629 SETBB-01. The diff touches only setup scripts, the GUI tour text,
the web/Android tour strings, docs and tests; no PAM, FFI, daemon or `soos-remote` runtime code.
Reviewed against the batch issue, each finding's fix-design caveats, `AI/ARCHITECTURE.md` and
`Docs/SECURITY_AND_QUALITY_GUIDELINES.md`. No CRITICAL or MAJOR finding. Four MINOR items, two
of them owner questions already recorded as deferrals in the ADR and walkthrough.

Tests run by the reviewer (per-worktree target dir, `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=3`,
`--test-threads=4`): `soos-invariants` 932 passed / 0 failed; `soos-remote` `setup_render_tests`
5 passed; `soos-gui` `onboarding_feature_tests` 29 passed. `bash -n` clean on both scripts.

## 2. Test Changes (mechanical listing, with justification)

Removed/changed assertion lines found by the step-3 grep: the `ARCHITECTURE.md` Unlock row (doc,
not a test), `assert!(config.allow_unlock)` in `setup_render_tests.rs` (now
`assert!(!config.allow_unlock)` on the default fixture, and the all-on variant still asserts
`allow_unlock` and `camera.funnel` on), and one `assert_eq!(written, managed(LOGIN, [true; 7], ...))`
in SDP16 (reformatted to the new default vector). No `#[ignore]`, `should_panic`, tolerance or
inline test-module change.

Every changed expectation in `setup_user_phase_contract.rs` (SET23, SET24, SET27, SET30, SET38),
`setup_deps_contract.rs` (SDP9, SDP10, SDP14, SDP16, SDP18, SDP27), `readme_contract.rs` (RDM12)
and the two fixtures was diffed line by line: each flips only index 1 (`allow_unlock`) and/or
index 6 (`camera_view_funnel`) from `true` to `false`, or removes `allow_unlock` from the
"turned on by setup's default profile" note, or narrows RDM12's expected on-set to the keys setup
still turns on. All fall within the owner-approved batch-3 migration of the old ON defaults
(decision of 2026-11-20). Setup-only changes (SET26 and the SET33 restart case pass
`--remote-unlock on` / `--camera-view-funnel on`; `with_off` starts from `fixture_all_on()`)
leave every assertion unchanged and are needed only because the default changed; they are listed
in walkthrough 259. No other test is weakened. New tests: SRD1-SRD8 (invariants), SRD6 render
parse (`soos-remote`), GUI-03 tour test (`soos-gui`).

## 3. Deep Reasoning Audit

### Logic & Architecture
- SETB-01: `resolve_features` now resolves option, then explicit file value (true or false), then
  the default; `ASKED_KEYS` default off unless an interactive yes. Tried: `--yes` fresh run (off),
  managed file with both lines deleted (written off, SRD2), user file with absent keys (off, not
  named in the default-profile note), `--yes` with `--remote-unlock on` (on). PASS.
- Questions only when `INTERACTIVE` (excludes `--yes`, `--dry-run`, non-tty); default no on
  empty input or EOF; skipped when an option or file value decides or a prerequisite is off. PASS.
- SETB-02 / SETBB-01: `toml_scan_line` replaces `count_of`/`bracket_depth`. Adversarial inputs
  were run against the extracted functions and compared with Python `tomllib`: line-ending
  backslash in `"""`, `'''` closed by four quotes, `"""` closed by five quotes, inline table
  containing `"]"`, array with a comment containing `]`, `"a\\"`, `\"""` inside `"""`, literal
  ending in a backslash, a real header followed by a key. Every case agreed with TOML on which
  top-level booleans exist. Unclosed constructs, a stray `]` and a bare backslash set
  `USER_UNMODELED` and turn every feature off (SRD5). The caveat (follow the real opener, and fail
  safe on any construct the reader cannot model) is respected. PASS.
- RCFG-02: carried keys pass a strict grammar; a `credentials_path` that cannot be written back
  refuses the run before any change (the verifier's safer option) rather than silently reverting
  the passkey store. The path grammar is stricter than `validate_credentials_path` and bounded
  like `soos-remote`. `allowed_hosts` is pinned only when the file already sets it (MINOR 1).
  PASS with a recorded deferral.
- SETB-03: the guide prints `tailscale serve status` and the port-443 warning before the Funnel
  command, in both `setup.sh` and `install_remote.sh`; setup still runs no tailscale command
  except `tailscale version` and never contains `--https` (SET3). PASS.
- GUI-03: the Off line is corrected in the three tours (kept equal, PTR-S5 green); the GUI shows a
  copyable command block and never runs `tailscale`; the accepted residual is recorded. PASS.

### PAM Concurrency & Deadlines
- No file under `crates/pam` or the daemon is touched. N/A, PASS.

### Panic Safety & Fail-Closed
- No new `unwrap`/`expect` in production Rust (a `const`, a `match` and egui view code only). No
  path converts an error into `PAM_SUCCESS`. Script failure modes fail safe: an unreadable file
  keeps every feature off; a bad `credentials_path` exits 2 before any change. PASS.

### Test Integrity & Anti-Weakening
- See section 2. The new tests target the old defects: SRD4 rows reproduce the issue bodies of
  #626/#629 and check the default-on keys `allow_funnel`/`camera_view`, so they distinguish the
  old parser; SRD6 checks the carried render byte-exact and parses it with the real `soos-remote`
  parser. PASS.

### Memory, Bounds & Secrets
- Reads stay bounded by `MAX_REMOTE_CONFIG_BYTES`; nothing is sourced or evaluated; carried
  values cannot break out of a TOML string (the character class excludes quotes and
  backslashes); the notes print key names only, never values (SRD5/SRD6 check that no value or
  path is printed). PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/` or hook change; no new external command in `setup.sh`
  (SDP3 green). PASS.

### English-Only Policy
- All added text is English; no host-specific path, login or private review material in the
  diff. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `scripts/setup.sh:1554` — `allowed_hosts = ["<rp_id>"]` is rendered only when the
  existing file already sets `allowed_hosts`; fresh installs still leave the host allowlist at
  "any `*.ts.net`". Deferred as an owner question in the ADR and walkthrough (it needs changes to
  byte-exact render tests outside the approved migration). Keep #597 open, or record the
  deferral explicitly when closing it.
- **[MINOR]** `scripts/setup.sh` `resolve_features` — an install made with the former default
  keeps `allow_unlock = true` / `camera_view_funnel = true` on a re-run, because setup cannot tell
  a value it wrote by default from an owner choice; the guide's risk line names them, but no
  question is asked. Owner question; consider a one-time note on re-run when both are `true` in a
  managed file.
- **[MINOR]** `scripts/setup.sh:2379` — the refusal message says "one absolute path in double
  quotes" while `carried_value` also accepts a single-quoted literal path; wording only.
- **[MINOR]** `Docs/PACKAGING_AND_PROVISIONING.md` §3.0 — "carries over `allowed_hosts` (always
  written as `["<rp_id>"]`)" can be read as a pin in every render; say "when the file sets it".

## 5. Final Verdict

**VERDICT: APPROVED**
