# Candid Review Report

- **Date**: 2026-10-15
- **Target Branch**: `feat/ir-antispoofing-mock`
- **Base (merge-base)**: `94a7cb3`
- **Reviewed-Diff-Fingerprint**: `19280cecb2e150c17048a6fc8cba55aec143d4554d1b9e88845cce3575bd8451`
- **Audited Files**: AI/DECISIONS.md, AI/VERIFICATION_MATRIX.md, AI/architect_spec_ir_emitter_control.md, AI/auditor_constraints_ir_emitter_control.md, AI/tester_contract_ir_emitter_control.md, AI/walkthroughs/206_ir_emitter_control.md, Cargo.lock, Docs/CAMERA_V4L_CRATE.md, Docs/DAEMON.md, crates/admin-cli/{Cargo.toml, src/args.rs, src/camera.rs, src/error.rs, src/ir_emitter.rs, src/lib.rs, src/main.rs, tests/ir_emitter_command_tests.rs}, crates/camera-v4l/{Cargo.toml, src/diagnostics.rs, src/ir_emitter.rs, src/ir_emitter_config.rs, src/ir_mock.rs, src/lib.rs, src/mock.rs, src/strobe.rs, src/uvc_xu.rs, tests/ir_emitter_config_tests.rs, tests/ir_emitter_tests.rs, tests/ir_mock_camera_tests.rs, tests/strobe_classification_tests.rs, tests/uvc_xu_tests.rs}, crates/daemon/{src/config.rs, tests/ir_emitter_daemon_config_tests.rs}, tests/fixtures/mod.rs, tests/invariants/src/{ir_emitter_contract.rs, lib.rs}, tests/physical/screensaver_test.md

## 0. Re-review after the rebase onto `main` (round 3)

The branch was rebased from merge-base `6f32ef8` onto `origin/main` `94a7cb3`. That commit (#396)
only updates `rustls` from 0.23.45 to 0.23.46 in `Cargo.lock` (RUSTSEC-2026-0346, RUSTSEC-2026-0347)
and adds walkthrough 211. Because this branch also edits `Cargo.lock`, the diff fingerprint changed
from `067317d4...` to the value above. The whole frozen patch (`target/candid_diff.patch`, 37 files,
7392 lines) was re-read, with these extra checks:

- **The rebase changed no content.** Excluding this report:
  - `git diff 6f32ef8 origin/feat/ir-antispoofing-mock` (before the rebase) and
    `git diff 94a7cb3 HEAD` (after the rebase) were compared textually.
  - They differ in exactly one line: the `index <blob>..<blob>` header of `Cargo.lock`. That is
    expected, because the base and result blobs now contain rustls 0.23.46.
  - Every hunk, including the `Cargo.lock` hunk, is byte-identical.
  - `git range-diff origin/feat/ir-antispoofing-mock...HEAD` shows #396 as the new base commit and
    the single branch commit as changed only in `AI/candid_review_report.md`. That report change is
    the old and new `main` versions of the report appearing as range-diff context.
- **The lockfile is consistent.**
  - The branch's `Cargo.lock` hunk still only adds `proptest` and `soos-test-fixtures` as
    dependencies of `soos-camera-v4l` (dev-dependencies).
  - `Cargo.lock` on `HEAD` keeps `name = "rustls"` / `version = "0.23.46"`, so the rebase did not
    revert the advisory fix.
  - `cargo metadata --locked --format-version 1` succeeds: the lockfile matches every manifest
    without changes.
  - No cargo build was run for this round. The code is byte-identical to the round-2 fingerprint,
    whose test and clippy evidence is listed below.
- **The mechanical test listing is unchanged.**
  - No removed or changed assertion or test attribute.
  - No escape hatch (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon).
  - One new inline `mod tests` (`crates/camera-v4l/src/uvc_xu.rs`).
  - Every added `unwrap`/`expect`/`panic!` is in a test file or in the prose of the auditor
    constraints.
- **Re-checked spots.**
  - The single `unsafe` ioctl in `UvcXuDevice::query` still checks the length before the call and
    keeps its `// SAFETY:` invariant.
  - `ensure_same_node` refuses an unknown or different rdev.
  - `XuBudget` caps the device test at `MAX_DEVICE_TEST_QUERIES` = 11.
  - `parse_camera_table` ignores sibling keys of `[camera]` and stays strict inside
    `[camera.ir_emitter]`.
  - `ignored_camera_keys` returns at most 8 sanitized names.
  - The English-only scan found no non-English content in the added lines.

**Carried forward:**
- The round-2 MINOR (a top-level scalar `camera = "x"` is a deserialization error) stays open and
  non-blocking.
- The round-1 SUGGESTION (`StrobeStatus::vetoes_allow` for `Stable(Absent)`) stays recorded as an
  open ADR decision for split item 4.
- There is no new finding.

## 0b. Re-review (round 2)

This report supersedes round 1 (fingerprint `202312bc...`, CHANGES_REQUESTED). The whole diff was
re-frozen and re-read, with a focus on the round-1 delta.

**Round-1 MAJOR: resolved.**
- `parse_camera_table` no longer rejects sibling keys of `[camera]`. Strict parsing is kept inside
  `[camera.ir_emitter]`.
- `ignored_camera_keys` returns at most 8 key names. Each is sanitized with `sanitize_display_text`
  and truncated to 64 characters. Values are never read.
- `crates/daemon/src/config.rs` pushes the warning
  `"[camera] <key> is not recognized and is ignored ..."`.
- `test_ira_daemon_loads_with_unknown_camera_key` loads
  `[camera] idle_timeout_secs = 4242`. It asserts that the daemon starts, that `[pipeline]` still
  applies, that the warning names the key, and that the warning never contains `4242`.
- `tests/physical/screensaver_test.md:303` now reads `[pipeline] idle_timeout_secs`.
- `Docs/DAEMON.md` and the ADR were updated.

**Round-1 MINOR (budget): resolved.**
- `XuBudget` forwards at most `limit` requests and refuses later ones with
  `QueryBudgetExhausted` without forwarding them.
- `configure` wraps its 2 pre-checks in a budget of 2. `test_emitter_on_device` routes every
  emitter request through a budget of 9: open 3, on, get, off and get 4, restore 1, retry on drop 1.
  That is exactly the worst case, so the budget can never block a legitimate restore.
- The total is therefore 11 (`MAX_DEVICE_TEST_QUERIES`), enforced in code on every ioctl path of
  the device test.
- The probe keeps its own `MAX_PROBE_QUERIES` bound.

**Round-1 MINOR (node identity): resolved.**
- `check_ir_node` reads `st_rdev` of the given path before classification and of the resolved node
  after it. It keeps the value only when the two are equal.
- `UvcXuDevice::open` records the `fstat` rdev of the opened descriptor.
- `main.rs` calls `ensure_same_node` before any UVC request. An unknown or different rdev is
  refused.
- The device number identifies the character device itself, independent of the path. Only an
  A→B→A swap during classification could defeat this, and that needs root write access to `/dev`.
  This is adequate.

**Round-1 SUGGESTION:** recorded as an open ADR decision.

**Test changes in this round:**
- Three tests that are new in this PR were changed. Their contract changed with the review, and
  each change is listed with a reason in `AI/tester_contract_ir_emitter_control.md`:
  - `test_ira_unknown_keys_are_refused`, now scoped to `[camera.ir_emitter]`.
  - The `[camera] exposure` case moved from refused to loads-with-warning.
  - The cap changed from 10 to 11, the real enforced maximum.
- No pre-existing test was touched. `tests/fixtures/mod.rs` and `tests/invariants/src/lib.rs` have
  additions only. The patch removes no assertion and adds no escape hatch.

**Verification on this fingerprint:**
- `cargo test --all-features` passes for `soos-camera-v4l` and `soos-admin-cli`.
- The daemon IR config tests pass (4).
- `soos-invariants` passes (609).
- `cargo clippy --all-targets --all-features -D warnings` is clean on camera-v4l, admin-cli and
  daemon.
- `cargo fmt --check` is clean.

**New residual, MINOR (non-blocking):** `DaemonConfigFile.camera` is typed
`Option<toml::Table>`. A top-level scalar such as `camera = "x"` was ignored before and is now a
deserialization error. No documentation, installer or tool has ever produced such a key, so the
upgrade risk is negligible. Optional fix: accept `toml::Value` and warn when it is not a table.

The round-1 sections below are kept for traceability. Their MAJOR and MINOR items are resolved
as described above.

## 1. Executive Summary

GitHub #378 (PR 1 of #353) adds UVC extension-unit (XU) access in `soos-camera-v4l` (one new `unsafe`
ioctl, `UVCIOC_CTRL_QUERY`), an `IrEmitter` trait with a restore-on-drop UVC implementation and a
`mock-camera`-gated mock, a pure strobe classifier, a strict `[camera.ir_emitter]` parser shared by
`soos-daemon` and `soos-admin`, and `soos-admin camera ir-emitter probe|configure`. The daemon never
drives the emitter in this PR.

The unsafe site, buffer bounds, restore logic, signal blocking and atomic writer are sound. Tests
pass (`soos-camera-v4l`, `soos-admin-cli`, daemon IR config suite, 609 invariants) and clippy
`-D warnings --all-features --all-targets` is clean on the three touched crates.

One upgrade-safety defect blocks approval: every key other than `ir_emitter` under `[camera]` now
makes `soos-daemon` refuse to start. Every other daemon table still ignores unknown keys, and the
repository's own physical test procedure tells operators about a `[camera] idle_timeout_secs`
setting. An operator who followed that document has a config that worked before and now stops the
daemon after upgrade, so face unlock is lost (PAM falls back to the password).

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: all new except `tests/fixtures/mod.rs` and `tests/invariants/src/lib.rs`, which are
  modified with additions only (new synthetic IR fixtures; `mod ir_emitter_contract;` registration).
- Removed or changed assertions (`^-` lines with `assert`/`#[test]`/`proptest!`/`should_panic`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): **none**.
- Inline test modules: one new `mod tests` in `crates/camera-v4l/src/uvc_xu.rs` (ABI layout, request
  code, length rules). No inline module removed.

No existing test was modified or weakened.

## 3. Deep Reasoning Audit

### Logic & Architecture
- `UvcXuControlQuery` matches `struct uvc_xu_control_query` (u8 unit, u8 selector, u8 query, u16 size
  at offset 4, pointer at 8). `_IOWR('u', 0x21, 16)` = `0xC0107521` (64-bit) and `0xC00C7521`
  (32-bit) are asserted. Request codes 0x01/0x81..0x87 match UVC 1.5 table A-8. PASS.
- Length rules (`GET_LEN` 2, `GET_INFO` 1, others 1..=64) are checked in `query` before the ioctl and
  in the helpers. A `size` that differs from the control's real length makes the kernel return
  `ENOBUFS`, which maps to `Io`. That is transient in the probe and an error in the emitter. PASS.
- The probe is bounded (32 units by 32 selectors, at most 3 requests each, capped at
  `MAX_PROBE_QUERIES` 4096, aborts after 16 transient errors or on the first EPERM/ENODEV). It sits
  behind `XuReadOnly`, which refuses `SET_CUR` without forwarding it. PASS.
- `configure` without `--test-on-device` issues only `GET_INFO` and `GET_LEN`. `--mode auto` is
  refused unless a device test runs in the same invocation. PASS.
- Strobe classifier: input is bounded (8..=64 samples), non-finite and out-of-range luma is refused,
  sequences must be strictly increasing, periodic command patterns (p = 2..=4) are refused, and the
  status is sticky after a change. Nothing consumes it yet. PASS. One SUGGESTION below.
- **Behaviour change: `[camera]` strictness** (`crates/daemon/src/config.rs:330-333,693-704`,
  `crates/camera-v4l/src/ir_emitter_config.rs:356-361`).
  - Before: `DaemonConfigFile` had no `camera` field and no `deny_unknown_fields`, so a `[camera]`
    table was ignored. Unknown keys in `[pipeline]`, `[socket]` and the other tables are still
    ignored.
  - Now: any key under `[camera]` other than `ir_emitter` returns a `DaemonError::Config` and the
    daemon does not start.
  - Exposure: `tests/physical/screensaver_test.md:303` (unchanged by this diff) tells the operator
    the camera LED goes off after "`[camera] idle_timeout_secs` (default 10 s)". The real key is
    `[pipeline] idle_timeout_secs`. An operator who tuned it where the procedure says has a
    `daemon.toml` that loaded on 6f32ef8 and now stops `soos-daemon`.
  - No shipped file writes `[camera]`: install.sh, packaging, GUI and enroll were checked. The
    upgrade note in `Docs/DAEMON.md` and the test `"[camera]\nexposure = 3"` record the new
    behaviour, but there is no migration path.
  - `scripts/install.sh` does fail loudly (`wait_daemon_ready.sh`, exit 70). Distribution package
    upgrades only restart the unit.
  - **FINDING (MAJOR).**
- Architecture: the `unsafe` code is confined to the adapter crate `soos-camera-v4l`. Business
  crates keep `#![forbid(unsafe_code)]`. `soos-frame` is unchanged, as the macOS ADR requires. The
  daemon still never drives the emitter. `soos-admin` opens the node only for UVC requests and
  never captures frames, the same exception the existing `camera probe` metadata path has. PASS.

### PAM Concurrency & Deadlines
- `crates/pam` is untouched. No Tokio, threads or blocking calls are added to PAM. PASS (N/A).

### Panic Safety & Fail-Closed
- No `unwrap`/`expect`/`panic!`/unbounded indexing on daemon or admin production paths. `buf[0]` on
  a `[u8; 1]` is constant-index. `serde_json` failures fall back to `"{}"`. PASS.
- The config parser fails closed. Error messages name the key, never the value. An unknown key name
  is sanitized and truncated to 64 characters. The daemon warning for `mode = "auto"` echoes no value
  (tested). PASS.
- The emitter marks itself `dirty` before every write, so a failed write is restored too. `restore`
  runs explicitly and again on drop. In `test_emitter_on_device` the emitter is dropped before
  `SignalBlock`, so the original value is written back before the signal mask is restored.
  SIGINT/TERM/HUP/QUIT/TSTP are blocked. The CLI is single-threaded, so the thread mask is the
  process mask. PASS.
- Request count: the pre-checks (2) plus the device test (8) equal 10. If the restore fails, `Drop`
  retries once, making 11. `MAX_DEVICE_TEST_QUERIES` is only asserted in tests and never enforced.
  **FINDING (MINOR).**

### Test Integrity & Anti-Weakening
- No test was removed or modified (§2). The new tests would fail against plausible wrong
  implementations: wrong struct layout or request code, `SET_CUR` forwarded by the read-only probe,
  restore skipped after a failed write, signals not blocked, a symlinked config overwritten,
  unparsable output written, a value echoed in an error. The mock (`MockXuControl`) is
  `mock-camera`-gated and models the kernel length rules. PASS.

### Memory, Bounds & Secrets
- All XU buffers are at most 64 bytes. sysfs id reads are capped at 16 bytes. The config read is
  capped at `MAX_DAEMON_CONFIG_BYTES` and opened with `O_NOFOLLOW|O_NONBLOCK`. The strobe monitor
  keeps at most 64 floats and copies no pixel. PASS.
- Device type is checked before `open(2)` (stat, major 81) and on the opened descriptor (`fstat`),
  with `O_CLOEXEC|O_NONBLOCK|O_NOCTTY`. The fstat check confirms "a V4L2 node". It does not confirm
  "the same node `check_ir_node` classified as Infrared uvcvideo", because the rdev is not compared
  and `VIDIOC_QUERYCAP` is not re-run on the fd. `main.rs` classifies, then opens the path, and
  `probe_ir_emitter` classifies again after the open. The command needs root and `/dev` is
  root-controlled, so exploitability is low. **FINDING (MINOR).**
- Atomic writer (`admin-cli/src/ir_emitter.rs:626-695`):
  - New text is written to a `create_new` + `O_NOFOLLOW` 0600 temp sibling.
  - Mode is the original's masked to 0644 and the owner is kept as root.
  - The file is fsynced, checked against identity (dev, ino, size, mtime), renamed, and the
    directory is fsynced.
  - The temp file is removed on failure. A symlinked target and unbalanced markers are refused.
  - The result is re-parsed and must round-trip.
  - The window between the identity check and `rename` is inherent and acceptable in a root-owned
    `/etc/soos`. PASS.
- Payload bytes appear only in `probe` output (hex control values, root-only, not biometric) and in
  the written section. No frames, embeddings or credentials. PASS.

### Supply Chain & Automation
- Only dev-dependencies change: `proptest = "1"` (already in the workspace lockfile) and
  `soos-test-fixtures` for camera-v4l, plus a self-dependency with `mock-camera` for admin-cli tests.
  With resolver 2 the mock feature does not reach production builds. `Cargo.lock` gains 2 lines. No
  CI or script changes. PASS.

### English-Only Policy
- Code, comments, docs and walkthrough are English. No non-English token found in the added
  lines. PASS.

## 4. Detailed Findings & Action Items

- **[MAJOR]** `crates/camera-v4l/src/ir_emitter_config.rs:356-361` (with `crates/daemon/src/config.rs:693-695`).
  - Defect: any key under `[camera]` other than `ir_emitter` now refuses daemon start-up, while
    every other daemon table ignores unknown keys. The repository's own operator procedure
    (`tests/physical/screensaver_test.md:303`) names a `[camera] idle_timeout_secs` setting. A
    config written per that document loaded before this change and stops `soos-daemon` after it, so
    face unlock is lost on upgrade.
  - Required fix: keep fail-closed strictness inside `[camera.ir_emitter]`. Turn unknown sibling
    keys of `[camera]` into a startup warning that names only the key, as `sensor_preference` does,
    with a test asserting that the daemon still starts and the warning is emitted.
  - Alternative: provide an explicit migration and upgrade check. Also correct
    `tests/physical/screensaver_test.md:303` to `[pipeline] idle_timeout_secs`, and update
    `Docs/DAEMON.md` and the walkthrough to match.
- **[MINOR]** `crates/admin-cli/src/ir_emitter.rs:447-448` / `crates/camera-v4l/src/ir_emitter.rs:213-222`
  - Defect: `MAX_DEVICE_TEST_QUERIES` (10) is not enforced. In `configure --test-on-device`, a
    failed explicit `restore()` is retried by `Drop`, which makes 11 requests.
  - Fix: either enforce the cap with a counting `XuControl` wrapper in `test_emitter_on_device`, or
    not retry in `Drop` after an explicit restore failure and document the bound precisely.
- **[MINOR]** `crates/admin-cli/src/main.rs` (`open` closure) / `crates/camera-v4l/src/uvc_xu.rs:452-470`
  - Defect: the post-open `fstat` check proves "some V4L2 char device", not "the node classified as
    Infrared uvcvideo" (rdev not compared, driver not re-queried on the fd).
  - Fix: pass the expected `rdev` from the classification into `UvcXuDevice::open`, or re-run
    `VIDIOC_QUERYCAP` on the opened descriptor and compare driver and bus info.
- **[SUGGESTION]** `crates/camera-v4l/src/strobe.rs` `StrobeStatus::vetoes_allow`
  - `Stable(Absent)` (no illumination) and `Stable(Steady)` do not veto.
  - The future strict-IR consumer (split item 4) should decide explicitly whether `Absent` may ever
    contribute to `Allow`. Record that in the umbrella ADR.

## 5. Final Verdict

Round 3 (rebase onto `94a7cb3`) confirms the reviewed content is unchanged apart from the lockfile base, which keeps rustls 0.23.46. Round 2 resolves every round-1 MAJOR and MINOR item. The only open item is the non-blocking MINOR
on a top-level scalar `camera` key (§0).

**VERDICT: APPROVED**
