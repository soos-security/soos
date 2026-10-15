# Candid Review Report

- **Date**: 2026-10-15
- **Target Branch**: `test/deflake-peer-limits`
- **Base (merge-base)**: `df7859d`
- **Reviewed-Diff-Fingerprint**: `478168855215de75b9187128c8ce6ced35569318085c57ded1bdd351d4c02d68`
- **Audited Files**:
  - `AI/VERIFICATION_MATRIX.md`
  - `AI/walkthroughs/210_deflake_peer_limits_reserve_test.md`
  - `crates/daemon/tests/peer_limits_tests.rs`

## 1. Executive Summary

The diff touches tests and documentation only. In `auth_reserve::test_rsh_auth_admitted_on_reserve_while_two_connections_held`, the sequence "async connect, 20 ms wall-clock sleep, async connect" is replaced by a new helper, `connect_back_to_back`. The helper performs two synchronous `std` connects without yielding, then registers both streams with the Tokio runtime through `from_std` after `set_nonblocking(true)`.

I verified the ordering argument against the daemon code instead of trusting the walkthrough:
- `ConnectionDispatcher::handle_connection` (`crates/daemon/src/dispatcher.rs:426-460`) runs `SO_PEERCRED`, `try_acquire` and `try_acquire_auth_reserve` synchronously, before its first `.await`.
- The 200 ms `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS` timer is armed only inside `read_and_process` (`dispatcher.rs:556-569`), when the admitted handler is first polled.
- The test's `spawn_server` accept loop spawns one handler per accepted stream. The kernel accept queue is FIFO, and the default `#[tokio::test]` current-thread scheduler runs spawned tasks in FIFO order. The held reserve's handler therefore takes the reserve before `second`'s handler is polled, so the 200 ms window cannot expire in between, whatever the host load.

Production code, PAM, IPC, Cargo and CI are untouched. No assertion changed. I found no CRITICAL or MAJOR issue. This report replaces the earlier report bound to merge-base `94a7cb3`, because `origin/main` moved to `df7859d`.

## 2. Test Changes

Mechanical listing (step 3) on `target/candid_diff.patch`:

- **Test files touched**: `crates/daemon/tests/peer_limits_tests.rs`. The walkthrough also matches because its file name contains `test`.
- **Removed or changed checks** (`assert`, `#[test]`, `#[tokio::test]`, `proptest!`, `should_panic`): **none**.
- **New escape hatches** (`#[ignore]`, `cfg(any())`, `should_panic`, `tolerance`, `epsilon`): **none**.
- **Inline `mod tests` changes**: none.
- **Removed lines**: two `UnixStream::connect(&sock_path).await.expect(..)` calls and one `tokio::time::sleep(Duration::from_millis(20)).await`. This is a setup and synchronisation change. `closed_promptly(&mut second)` ("only one auth-reserved connection per UID"), `permits_back(6)`, both `status_roundtrip` calls and every other helper body are byte-identical. A setup-only change with untouched assertions is acceptable.

## 3. Deep Reasoning Audit

### Logic & Architecture
- *Can `second` still be admitted on the reserve?* Only if `held_reserve` releases it first. Release needs the held handler to be polled, arm its 200 ms timer and let it expire. On the current-thread runtime, `second`'s handler is spawned after `held_reserve`'s handler and runs in the same scheduler pass, long before 200 ms elapse. Admission is synchronous, so it cannot be interleaved. PASS.
- *Can the accept loop yield between the two accepts, for example on the coop budget?* That changes nothing: both handlers are still spawned in accept order and admitted in that order. The held handler's timer only starts on its own first poll, and `second` is admitted on its first poll right after. PASS.
- *Can the test task hold the runtime between the connects?* Both connects are synchronous, and a Unix stream `connect` with a near-empty backlog returns immediately. Blocking `std` calls inside an async test are acceptable for test setup here. PASS.
- *Is the earlier `auth` connection still holding a permit?* `permits_back(6)` before the new sequence rules that out. PASS.
- *Is the documentation consistent?* The PLF1 matrix row and walkthrough 210 match the code. They state the reliance on the current-thread flavour and the sequential accept loop as a known limitation. PASS.

### PAM Concurrency & Deadlines
- `crates/pam` is not touched. N/A, PASS.

### Panic Safety & Fail-Closed
- No production code is changed. The new `expect` calls are in an integration test file whose top-level `#![allow(clippy::expect_used, ...)]` already covers them. No path to `Allow` or `PAM_SUCCESS` is introduced. PASS.

### Test Integrity & Anti-Weakening
- *Mutation:* with `AUTH_RESERVED_CONNECTIONS_PER_UID = 2`, `second` would be admitted on a reserve, answer its `Auth` with a payload, and `closed_promptly` would panic. The test still detects this regression. A daemon that never closes the refused connection fails on the 500 ms `CLOSE_DEADLINE`. A permit leak fails `permits_back(6)`. PASS.
- *Reproduced:* `cargo test --locked -p soos-daemon --all-features --test peer_limits_tests -- --test-threads=4` passed 28 tests. Under a starvation harness (test binary pinned to CPU 0 at `nice 19`, four busy loops pinned to CPU 0), the target test failed 0 times in 40 runs. PASS.

### Memory, Bounds & Secrets
- The diff adds no allocation, logging or secret handling. PASS.

### Supply Chain & Automation
- `Cargo.*`, `deny.toml`, `.github/`, `scripts/` and `.githooks/` are untouched. N/A, PASS.

### English-Only Policy
- Code comments, the matrix row and the walkthrough are in English. "Synchronisation" is British spelling, which is acceptable. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/daemon/tests/peer_limits_tests.rs:1004` — The determinism depends on the current-thread `#[tokio::test]` flavour. A future change to `flavor = "multi_thread"` would silently reopen #387. The helper's doc comment already says "(current-thread)", and the walkthrough §9 records this. You could also state this directly on the test attribute. Optional.

No CRITICAL, MAJOR or MINOR findings.

## 5. Final Verdict

**VERDICT: APPROVED**
