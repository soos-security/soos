# Walkthrough 148 — PAM Response Expiry and Protocol Follow-ups

- **Date**: 2026-10-01
- **Issue**: GitHub #287 ("Protocol / PAM" items and the owner decisions of 2026-10-01)
- **Branch**: `fix/p3fu-pam-expiry`
- **Matrix criteria**: PRE1–PRE11 (new component `pam-response-expiry-protocol-followups`);
  in-place updates of PDR1, PDR5, DDS6, QFU3, PK7, DV3 and the PCZ6 statement of the
  `wire-single-interpretation-regression` intro
- **ADRs**: 2026-10-01 "PAM Client Enforces Response Expiry"; 2026-10-01 "Untagged Legacy
  Client Frames Stay Accepted"; ADR 2026-09-30 "Response Timestamps Are Informational" marked
  superseded in part

---

## 1. Context & Objectives

| #287 item | Outcome | Rows |
|---|---|---|
| Owner decision: the PAM client enforces the `Response` expiry | Done: `Response::check_freshness` + `IpcError::StaleResponse` → `PAM_IGNORE`; fixtures and `mock_daemon.py` emit real CLOCK_MONOTONIC stamps | PRE1–PRE7, PDR5 |
| Owner decision: untagged legacy client frames stay accepted | Documented only (ADR + `Docs/IPC_PROTOCOL.md` §12); codec and `wire_discriminator_tests::test_204_legacy_untagged_status_request_still_served` untouched | PRE8 |
| PCZ6, DDS6, QFU3, PK7, DV3 still Pending; `strict-codec-client-tag-integration` twice | PCZ6 was already `⏹ Superseded` (RFX6). DDS6, QFU3, PK7, DV3 re-checked against the CI job logs of run 36777041130 and set to `✅ Verified`. The section heading occurs once on `main`; the duplicated facts were the PCZ6 supersession (stated in two section intros) and the `StatusResponse` paragraph of `Docs/IPC_PROTOCOL.md` (twice); both merged | PRE9 |
| `pcx_wire_routing_tests` fails as root | Root cause: the precondition `assert_ne!(getuid(), 0, "run as a non-root peer")`; setup-only fix (privilege switch before any socket exists) | PRE10 |

## 2. Architect Design

**Clock.** `soos-daemon` stamps in `ConnectionDispatcher::build_response` from
`current_monotonic_nanos` (`nix::time::ClockId::CLOCK_MONOTONIC`, `crates/daemon/src/pipeline.rs`):
`issued > 0`, `expires = issued + RESPONSE_VALIDITY_NS` (2 s), `0 / 0` and never `Allow` on a
clock failure. `pam_soos.so` already reads CLOCK_MONOTONIC (`ipc::monotonic_nanos`,
`libc::clock_gettime`) for `deadline_monotonic_ns`, so the comparison uses the very same clock.

**Types.**

```rust
// crates/protocol/src/types.rs (re-exported from the crate root)
pub enum ResponseFreshnessError { ClockUnavailable, Unstamped, Inverted, FutureDated, Expired }
impl Response {
    pub fn check_freshness(&self, now_monotonic_ns: u64, max_future_skew_ns: u64)
        -> Result<(), ResponseFreshnessError>;
}
pub const MAX_RESPONSE_FUTURE_SKEW_NS: u64 = 10_000_000; // 10 ms, single source of truth
// crates/pam/src/ipc.rs
pub use soos_protocol::types::MAX_RESPONSE_FUTURE_SKEW_NS;
pub enum IpcError { /* … */ StaleResponse(ResponseFreshnessError) }
// crates/admin-cli/src/test_pam.rs: same check_freshness call, same constant
```

Rules, in order: `now == 0` → `ClockUnavailable`; `issued == 0 || expires == 0` → `Unstamped`;
`expires < issued` → `Inverted`; `issued > now.saturating_add(skew)` → `FutureDated`;
`now >= expires` → `Expired`. The protocol crate stays clockless (the caller supplies `now`).

**Placement.** `authenticate_before_with_progress`: decode → version → `request_id` binding →
socket drop → deadline check → `check_freshness(monotonic_nanos(), MAX_RESPONSE_FUTURE_SKEW_NS)`.
The clock is read after the last byte, so a legitimate response is at most a few milliseconds
old; the 2 s daemon window covers every clamped `timeout_ms` (≤ 5000 ms), because the daemon
stamps after its decision, not at request start.

**Error taxonomy.** Every `ResponseFreshnessError` → `IpcError::StaleResponse` → `Err(_)` →
`PAM_IGNORE` with the generic "Face verification unavailable." text, for every verdict.

**Latency budget.** Unchanged: one extra `clock_gettime` after the exchange.

**Invariants touched.** Fail closed (no error → `PAM_SUCCESS`), no Tokio/threads in the PAM
crate, no `unwrap`/`expect`/`panic!` in production code, no stamp or nonce in any log or error
text.

**Documentation drift.** ADR 2026-09-30 "Response Timestamps Are Informational", PDR1, PDR5,
`Docs/IPC_PROTOCOL.md` §3, `Docs/PAM_MODULE.md` §6, `Docs/DAEMON.md`, the `RESPONSE_VALIDITY_NS`
doc comment, the `Response` doc comment and the project-facts constants table.

## 3. Plan Evaluation

Condensed in this single-agent run (no separate plan-evaluator report). Two existing tests
contradicted the owner decision. The first round worked around them without touching any
assertion; the owner then approved migrating both on 2026-10-01 (second round, §4):

1. `distro_matrix::test_mock_daemon_malformed_modes_put_one_defect_on_the_wire` pinned the mock
   `allow` / `wrong-request-id` / `bad-version` / `malformed` frames to exactly 41 bytes, i.e.
   `0 / 0` stamps (one-byte varints); real stamps need 6–7 bytes each. First round: the mock kept
   `--stamps zero` as default. Second round: the test decodes the stamps, and the mock defaults
   to `--stamps monotonic`.
2. `pam_response_freshness_contract::test_ipc_protocol_documents_the_real_replay_protection`
   required "informational" and "not validated" on the `expires_monotonic_ns` line. First round:
   the line kept the words as history. Second round: the test requires the enforced rule.

## 4. Tester Contract

| Test (path::name) | Row | Red evidence |
|---|---|---|
| `crates/protocol/tests/response_freshness_tests.rs` (10 tests + `prop_pre_freshness_accepts_only_consistent_unexpired_windows`) | PRE1, PRE2 | stub `check_freshness` returning `Ok(())`: 9 of 11 failed (`left: Ok(()) right: Err(Expired)` …); the value-free display and the fresh-accept tests passed on the stub |
| `crates/pam/tests/response_expiry_tests.rs::test_pre_expired_allow_response_returns_ignore` (mandatory `PAM_IGNORE` pathway) | PRE3 | `left: 0 right: 25` (the stub returned `PAM_SUCCESS`) |
| `…::test_pre_expired_allow_response_is_rejected_by_the_ipc_client`, `…_unstamped_…`, `…_inverted_…` | PRE3 | `expected StaleResponse(..), got Ok((Allow, FaceMatch))` |
| `…::test_pre_future_dated_allow_response_returns_ignore`, `…::test_pre_future_skew_bound_is_ten_milliseconds` | PRE4 | `left: 0 right: 10000000` |
| `…::test_pre_fresh_allow_response_returns_success` (control) | PRE5 | passed on the stub, as intended |
| `tests/invariants/src/pam_response_expiry_contract.rs` (8 tests) | PRE6–PRE9, PRE11 | the first 7 failed before the mock, scripts, fixtures and docs changed; `test_pre_test_pam_and_pam_share_the_freshness_rule` was added with the shared constant |
| `crates/admin-cli/tests/test_pam_freshness_tests.rs` (4 tests) | PRE11 | before `test_pam.rs` called `check_freshness`: the expired, unstamped and future-dated cases failed (`an expired Allow must be interpreted as PAM_IGNORE: PAM_SUCCESS (Authentication authorized)`), the fresh control passed |

### Migrated existing tests — setup only (no assertion changed)

Approved by the owner decision ("real-clock fixtures"). Each fixture daemon now calls
`stamps::fresh_stamps()` (new `crates/pam/tests/common/stamps.rs`: `issued` = CLOCK_MONOTONIC
at response build time, `expires = issued + 2 s`) instead of a literal:

| File | Change |
|---|---|
| `crates/pam/tests/common/mod.rs` | `pub mod stamps;`; `spawn_daemon` stamps `1000 / 2000` → `fresh_stamps()` (used by `pam_silent_tests`, `pam_feedback_tests`, `pam_fail_quiet_tests`, `pam_bindings_tests`) |
| `crates/pam/tests/ipc_tests.rs` | `#[path = "common/stamps.rs"] mod stamps;`; five fixture daemons `1000 / 2000` → `fresh_stamps()` (nominal allow, deny, unavailable, request-id mismatch, drip-feeding allow) |
| `crates/pam/tests/pam_handle_tests.rs` | same module include; `spawn_daemon` `1000 / 2000` → `fresh_stamps()` |
| `crates/pam/tests/strict_decode_tests.rs` | same; `spawn_allow_daemon_with_trailing_bytes` `1000 / 2000` → `fresh_stamps()` |
| `crates/pam/tests/pcx_client_tag_tolerance_tests.rs` | same; `spawn_allow_daemon_with_trailing_bytes` `1000 / 2000` → `fresh_stamps()` |
| `crates/pam/tests/wire_tag_tests.rs` | same; `test_204_pam_auth_request_frame_is_tagged` fixture `0 / 0` → `fresh_stamps()` |
| `crates/admin-cli/tests/test_pam_tests.rs` | `#[path = "../../pam/tests/common/stamps.rs"] mod stamps;`; Allow and Deny fixtures `0 / 0` → `fresh_stamps()` (second round: `test_simulate_pam_auth_allow` would otherwise report the stale `PAM_IGNORE`) |
| `crates/admin-cli/tests/cli_deadline_json_tests.rs` | same include; `capture_deadline` fixture `0 / 0` → `fresh_stamps()` |
| `crates/admin-cli/tests/wire_tag_tests.rs` | same include; `test_204_admin_test_pam_request_frame_is_tagged` fixture `0 / 0` → `fresh_stamps()` |
| `crates/daemon/tests/pcx_wire_routing_tests.rs` | setup helper `run_as_unprivileged_peer_when_root()` called before the precondition (§6) |

Without the migration `test_ipc_nominal_allow_returns_success`,
`test_ipc_fragmented_body_within_budget_is_accepted`, the wire-tag `Deny` test and
`test_simulate_pam_auth_allow` fail, and the request-id-mismatch, trailing-byte and client-tag
tests would pass for the wrong reason (expiry instead of their own defect).

### Migrated existing tests — assertions changed with owner approval (2026-10-01)

**`tests/invariants/src/distro_matrix.rs::test_mock_daemon_malformed_modes_put_one_defect_on_the_wire`.**
The mock now stamps from CLOCK_MONOTONIC by default, so frames are no longer 41 bytes. The
strength "exactly one defect per mode" is kept and extended to the stamps:

| Old assertion | New assertion |
|---|---|
| `allow.len() == 41`, `&allow[..4] == 37u32.to_be_bytes()` | `decode_mock_response`: `frame.len() == 4 + declared`, single-byte verdict and reason, the body is consumed exactly after the two varint stamps; plus `before <= issued <= after` (CLOCK_MONOTONIC read around the exchange) and `expires == issued + 2 s` |
| `wrong.len() == 41`, `version.len() == 41`, `malformed.len() == 41` | same complete-frame decode and the same fresh-stamp checks for `wrong-request-id`, `bad-version`, `malformed` (and `deny`, which had no length check before) |
| `allow[4] == 1`, `&allow[5..37] == id`, `allow[37] == 0` (and the per-mode version / id / verdict checks) | unchanged, read from the decoded fields (`version`, `request_id`, `verdict`) |
| `malformed[37] > 3 && malformed[37] < 0x80` | unchanged on the decoded verdict byte |
| *(none)* | new: `expired` is a complete v1 frame echoing the id with an `Allow`, a consistent stamped window (`0 < issued <= expires`) that closed before the request |
| `crash-truncated`, `oversized`, `empty` checks | unchanged |

**`tests/invariants/src/pam_response_freshness_contract.rs::test_ipc_protocol_documents_the_real_replay_protection`.**

| Old assertion | New assertion |
|---|---|
| the `expires_monotonic_ns: u64` line contains "informational" and "not validated" | the line contains "Enforced by the PAM client", `IpcError::StaleResponse` and `PAM_IGNORE` |
| *(none)* | the "Response Freshness" rule table maps `` `now >= expires` `` to `Expired` and the zero-stamp rule to `Unstamped`, and states that a stale response is "`IpcError::StaleResponse` and therefore `PAM_IGNORE`" |
| `doc.contains("Response Freshness")` (request_id binding / deadline section) | unchanged |

`test_response_timestamps_are_not_documented_as_replay_protection` is unchanged. The module
comment now describes the enforced expiry instead of "does not validate".

### Flakiness check

The PAM and admin-cli expiry tests use second-scale margins (expired = closed 1 s ago, future =
1 s beyond the bound, fresh = 2 s window) and no wall-clock latency assertion. The migrated mock
test brackets `issued` between two CLOCK_MONOTONIC reads taken around the exchange.

## 5. Auditor Constraints

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| 1 | No `unwrap`/`expect`/`panic!`/indexing in the new production code; saturating bound arithmetic | `types.rs::Response::check_freshness`, `ipc.rs`, `test_pam.rs` | workspace clippy `-D warnings`; `test_pre_freshness_saturates_near_u64_max` |
| 2 | Every freshness failure ends in `PAM_IGNORE`, never `PAM_SUCCESS` (and `test-pam` reports it so) | `ipc.rs::authenticate_before_with_progress`, `test_pam.rs::simulate_pam_auth` | PRE3, PRE4, PRE11 |
| 3 | Same clock on both sides (CLOCK_MONOTONIC); a failed client read (`0`) rejects | `ipc.rs::monotonic_nanos`, `test_pam.rs::monotonic_now_ns`, `check_freshness` | `test_pre_client_clock_failure_is_rejected`; code review of `pipeline.rs::current_monotonic_nanos` |
| 4 | No stamp, nonce or payload in error text, report text or logs | `ResponseFreshnessError::fmt`, `IpcError::fmt`, `pam_result` | `test_pre_freshness_error_display_is_value_free` |
| 5 | No Tokio, no thread, no new dependency in the PAM crate; protocol crate stays clockless and `#![forbid(unsafe_code)]`; one skew constant | `crates/pam`, `crates/protocol`, `crates/admin-cli` | `Cargo.lock` unchanged; `test_pre_test_pam_and_pam_share_the_freshness_rule` |
| 6 | Freshness checked after the nonce binding (replay protection unchanged) | `ipc.rs` | `deadline_uid_tests::test_replayed_allow_response_is_rejected_by_request_id_binding` still green |
| 7 | Mock socket stays `0660`, stamps bounded to `u64` | `mock_daemon.py::write_varint` | `distro_matrix::test_mock_daemon_socket_is_group_restricted`, PRE6 |
| 8 | Root-safe test changes only its own thread's credentials, restores root on drop and never weakens the precondition; the one `unsafe` block is documented | `pcx_wire_routing_tests.rs` | PRE10 run as root; `undocumented_unsafe_blocks` lint |

Pre-existing violations found: `soos-admin test-pam` does not check the response `request_id`
against its nonce (out of scope, follow-up). Clearance: CLEARED.

## 6. Implementation

- `crates/protocol/src/types.rs`, `lib.rs`: `ResponseFreshnessError`, `Response::check_freshness`,
  `MAX_RESPONSE_FUTURE_SKEW_NS` (moved here in the second round so PAM and `test-pam` share
  one constant), updated `Response` doc comments.
- `crates/pam/src/ipc.rs`: re-export of `MAX_RESPONSE_FUTURE_SKEW_NS`, `IpcError::StaleResponse`,
  the check after the deadline test.
- `crates/admin-cli/src/test_pam.rs` (second round, confined to `simulate_pam_auth`): after
  decoding, `resp.check_freshness(monotonic_now_ns().unwrap_or(0), MAX_RESPONSE_FUTURE_SKEW_NS)`;
  a stale response yields `pam_result = "PAM_IGNORE (stale daemon response: <rule>; the PAM
  module falls back to the password whatever the <verdict> verdict)"`, the daemon verdict and
  reason stay in the report, and `PamTestReport` keeps its fields (JSON shape unchanged).
- `crates/daemon/src/dispatcher.rs`: `RESPONSE_VALIDITY_NS` doc comment only.
- `tests/docker/mock_daemon.py`: `--stamps {zero,monotonic}` (default `monotonic` since the
  second round), `write_varint`, `response_stamps` read at send time (after the `timeout`
  delay), new mode `expired`; docstring.
- `tests/docker/test_suite.sh`, `tests/distro/{arch_linux,debian_ubuntu,fedora_rhel}_test.sh`,
  `tests/physical/pam_integration_test.sh`: every mock invocation passes `--stamps monotonic`
  explicitly (kept after the default flip; the `start_mock_daemon` helpers place it before
  `"$@"`, so a case can still override it); T15 adds `expired` to its loop and an unstamped
  `Allow` (`--stamps zero`).
- `crates/daemon/tests/pcx_wire_routing_tests.rs` (item 4): as root, the scenario "peer UID ≠
  `uid_hint` 0" is impossible, so the precondition `assert_ne!(getuid(), 0, …)` failed
  (`left: 0, right: 0`). The setup helper switches the real and effective UID to 65534 (saved
  UID 0) before the temporary directory, the listener and the client socket exist, and restores
  root on drop. **Choice of the per-thread syscall (second round):** the first version used
  `nix::unistd::setresuid`, i.e. glibc's wrapper, which broadcasts the change to every thread
  of the process (POSIX semantics). The file has a single `#[tokio::test]` today, so nothing
  raced, but any test added later would run concurrently on another libtest thread as UID 65534.
  The helper now issues the raw `libc::syscall(libc::SYS_setresuid, ..)`: Linux keeps
  credentials per thread, so only the test thread changes. That is sufficient because
  `#[tokio::test]` runs a current-thread runtime on that same thread: the listener bind, the
  dispatcher tasks and the client `connect` (whose credentials `SO_PEERCRED` records) all happen
  there, and threads the test creates afterwards inherit its credentials. Serialising with a
  lock was rejected: it would not stop the broadcast from reaching tests that do not take the
  lock. The precondition and every assertion are unchanged; non-root runs skip the switch.
- Docs: `AI/DECISIONS.md` (two ADRs, superseded marker; second round: shared constant,
  `test-pam`, mock default, both approved migrations), `Docs/IPC_PROTOCOL.md` (§2 encoder note,
  §3 fields and "Response Freshness" rule table, merged `StatusResponse` paragraph, §12 ADR
  reference), `Docs/PAM_MODULE.md` §6, `Docs/DAEMON.md`, `Docs/PAM_DOCKER_TEST_MATRIX.md` (T15),
  `.agents/skills/dev-workflow/references/project-facts.md` (constants).
- Matrix: PRE1–PRE11; PDR5 → `✅ Verified`; PDR1 annotated; DDS6, QFU3, PK7, DV3 →
  `✅ Verified` citing jobs 110097611926, 110097611816 and 110097611919 of run 36777041130
  (logs re-read on 2026-10-01: "Toolchain in use: rustc 1.98.1"; "rpm -U from a %ghost-owning
  build kept the exact master key (no leftover copy)"; "Arch Linux Deployment Validation
  Succeeded (100%)" and "Arch Linux package test passed cleanly").

## 7. Candid Review

Layer 2 (fresh-context reviewer) not run in this batch: the orchestrator runs it on the
integrated branch. Layer 1 (`./scripts/candid_review.sh`) passed (§8).

## 8. Verification Results

Second round, 2026-10-01, with `CARGO_BUILD_JOBS=4`:

- `cargo test --locked --all-features -p soos-protocol -p soos-pam -p soos-daemon -p soos-admin-cli -p soos-invariants --no-fail-fast`:
  100 test binaries, 986 tests passed, 0 failed.
- `cargo clippy --locked --all-targets --all-features -p soos-protocol -p soos-pam -p soos-daemon -p soos-admin-cli -p soos-invariants -- -D warnings`: clean.
- `cargo fmt --all -- --check`: clean.
- `./scripts/candid_review.sh` (layer 1): PASSED.
- `python3 scripts/sync_issue.py --check`: OK (no branch registration: GitHub-only finding).
- PAM Docker matrix: `tests/docker/test_suite.sh` in the `soos-sandbox` image on a copy of the
  source: "ALL IN-CONTAINER PAM MATRIX TESTS PASSED SUCCESSFULLY!" — T1 Allow with real stamps
  (`PAM_SUCCESS`), T15 `unstamped` and `expired` rejected with the password fallback intact.
- `pcx_wire_routing_tests` as root in `archlinux:latest`: the `main` version fails
  (`assertion left != right failed: run as a non-root peer`, `left: 0`, `right: 0`); the
  per-thread version passes (`1 passed`).

## 9. Known Limitations / Follow-ups

- `soos-gui` decodes `Response` refusals without the freshness check (out of scope).
- `soos-admin test-pam` does not compare the response `request_id` with its nonce, unlike
  `pam_soos.so`.
- A PAM host in a different time namespace than the daemon is unsupported (already true for
  `deadline_monotonic_ns`).
- The per-thread `SYS_setresuid` call in `pcx_wire_routing_tests` assumes a 32-bit-UID syscall
  (`x86_64`, `aarch64`); on legacy 32-bit x86 the 32-bit variant is `SYS_setresuid32`.
