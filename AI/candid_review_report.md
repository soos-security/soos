# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `feat/remote-companion`
- **Base (merge-base)**: `222665f`
- **Reviewed-Diff-Fingerprint**: `723a0c0cce5cce5068305797b2973959c187a18f579d0e062afd00fa958f90c6`
- **Audited Files**: full branch patch (`target/candid_diff.patch`, 74 files, 30274 lines). Everything
  up to HEAD `a2ab864` was approved under fingerprint `9d326db3…`, and the candid fingerprint fix
  (`scripts/candid_subagent.sh` drops `--binary`; new test
  `test_candid_fingerprint_is_portable_for_binary_files`) was approved under `4a670e7a…`; both are
  unchanged and their audit is kept below (section "Prior delta"). The new delta is
  `crates/remote/src/audit.rs` (new `unlock_requested()` audit event) and
  `crates/remote/src/server.rs` (`unlock_flow` calls it instead of `info!`; `info` import dropped).

## 1. Executive Summary

`soos-remote::server_tests::test_rmc_unlock_is_audited_without_identity` was intermittent: the
`info!("remote unlock requested")` macro callsite caches its interest process-wide, so when a
parallel test thread with no subscriber registered the callsite first, the event stayed disabled
for this test's thread-scoped subscriber. The fix routes the line through the same hand-built
callsite path that `audit.rs` already uses for `remote login accepted` and `passkey registered`
(`get_default` + per-dispatch `enabled` check, no cached interest). Message text, level (INFO),
position (after the gate update, right before `unlock_session`) and the absence of any field are
unchanged; only the event target changes from `soos_remote::server` to `soos_remote::audit`, which
no document or test names. The test is untouched. No CRITICAL or MAJOR findings.

## 2. Test Changes

Mechanical step-3 listing over the full patch:
- Removed/changed assertion lines: one hit (`presence_unlock_contract.rs`, patch line 28227), the
  zbus "daemon-only" contract migration already justified in the earlier approved reviews (ADR
  2026-10-05 item (7), PAU17). Unchanged.
- New escape hatches: one hit at patch line 4800, report/walkthrough prose only.
- New delta: no test file touched. `crates/remote/tests/server_tests.rs`
  (`test_rmc_unlock_is_audited_without_identity`) is byte-identical to HEAD.
- Prior delta (approved under `4a670e7a…`), unchanged:
  - `artifact_freshness_contract.rs` adds one test and one module doc bullet,
  and moves the body of `ScratchRepo::fingerprint()` into
  `fingerprint_with_compression(Option<&str>)`; `fingerprint()` calls it with `None`. No assertion
  removed or weakened. The old body called `candid(&["--fingerprint"])`; the new body builds the same
  command through the same isolated `self.command("bash")` (all `GIT_*` and `CANDID_BASE_REF`
  removed, `GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`, `HOME` in the scratch dir), merges
  stdout and stderr the same way, asserts success with the same message, and parses the last
  64-hex line. With `None` no extra env var is set, so behavior is identical. → PASS.

## 3. Deep Reasoning Audit

### Logic & Architecture
- **Same event semantics?** `UNLOCK_META` is a copy of the two existing audit metadata entries with
  name `remote unlock requested`, target `soos_remote::audit`, `Level::INFO`, `Kind::EVENT`, a
  single `message` field whose value is the constant name. `emit` checks
  `dispatch.enabled(meta)` on every call, so a subscriber filtering `INFO` still sees it and one
  filtering it out still drops it. With the fmt subscriber the output line is
  `INFO soos_remote::audit: remote unlock requested`. → PASS.
- **Placement unchanged?** `server.rs:1737-1739`: gate set, `audit::unlock_requested()`, then
  `unlock_session`. Refusal paths (403 disabled/CSRF/passkey, 429, 503 snapshot, 409 no_session,
  409 already_unlocked) all return before the call. A logind failure or the
  `UNLOCK_FLOW_DEADLINE_MS` timeout after the call still leaves exactly one line, as before (the
  line marks a request, not a success). → PASS.
- **Cached-interest diagnosis.** `AuditCallsite::set_interest` is a no-op and the callsite is never
  registered with the callsite registry, so there is no global interest cache to poison; this is the
  documented pattern of the module and works the same for the third line. Attempted: a thread with
  no subscriber calling `unlock_requested()` first → `get_default` yields the no-op dispatcher,
  `enabled` false, nothing cached; a later call under a scoped subscriber is evaluated afresh.
  → PASS.
- **Remaining macros.** `server.rs` still uses `debug!` for refusals; those are not asserted by any
  test as "present", only as absent/clean, so the same caching effect cannot cause a false failure.
  → PASS.
- **Docs.** `Docs/REMOTE_COMPANION.md:85`, `AI/DECISIONS.md:122`, `AI/VERIFICATION_MATRIX.md`
  RMC25 and `AI/walkthroughs/184_remote_unlock.md:44` describe an `info` line
  `remote unlock requested` without naming a target; all remain accurate. Planning artifacts
  (`AI/auditor_constraints_remote_passkey_funnel.md` C15, architect spec) describe the line in
  `info!(...)` macro form, the same historical wording already accepted for the two other audit
  lines. → PASS.
- **Prior delta (candid fingerprint portability)**, unchanged since the `4a670e7a…` review:
  `review_diff` without `--binary` still binds every binary change kind through the
  `--full-index` blob ids; `--rev`, the working-tree gate and `--prepare` share one code path; the
  repository has no `.gitattributes` or submodules. → PASS.

### PAM Concurrency & Deadlines
No PAM code in the delta. The unlock flow is still under `timeout(UNLOCK_FLOW_DEADLINE_MS)`; the
audit call is synchronous and non-blocking (a subscriber write). → PASS (n/a).

### Panic Safety & Fail-Closed
`emit` has no `unwrap`/indexing; a missing `message` field returns silently (impossible with the
static `FIELDS`). Audit emission cannot change the HTTP outcome. No path to an unlock without the
existing passkey, CSRF, opt-in and rate-limit checks. → PASS.

### Test Integrity & Anti-Weakening
Test unchanged. It would still fail against a wrong implementation that emits the line on a refusal
(first `assert!`), emits twice (`audit.len() == 1`), drops it, logs at another level (`INFO`
check), or adds the login/session id/host/uid/path/header or passkey material (forbidden needles;
the target `soos_remote::audit` and message contain none of them). Ran `cargo test -p soos-remote`:
all suites green; `cargo clippy -p soos-remote --all-targets -- -D warnings` clean;
`cargo fmt -p soos-remote -- --check` clean. Prior-delta test analysis unchanged. → PASS.

### Memory, Bounds & Secrets
The event has one field holding a `&'static str` constant; no identity, session id, address or
credential can reach it. `module_path!`/`file!`/`line!` metadata are compile-time constants. → PASS.

### Supply Chain & Automation
No dependency, workflow or script change in the new delta (`tracing` already used). Prior script
change unchanged. → PASS.

### English-Only Policy
New doc comments and module doc are English. → PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/remote/src/audit.rs:2` — the edited module doc line is 139 columns
  (rustfmt does not reflow comments); rewrap to the crate's 100-column style.
- **[SUGGESTION]** `scripts/candid_subagent.sh:134` — carried over: consider pinning
  `-c core.bigFileThreshold=512m` and `-c core.attributesFile=/dev/null` in `review_diff`.
- **[SUGGESTION]** `scripts/candid_subagent.sh:126` — carried over: the comment could say the
  binding is now through the SHA-1 blob id, so `--binary` is not reintroduced for "stronger" binding.

## 5. Final Verdict

**VERDICT: APPROVED**
