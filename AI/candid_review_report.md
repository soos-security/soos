# Candid Review Report

- **Date**: 2026-10-03
- **Target Branch**: `fix/install-readiness-and-presence-warmup`
- **Base (merge-base)**: `b16f567`
- **Reviewed-Diff-Fingerprint**: `1d7229d954d017fa29c5f28c6468067625835f310a45c2d9772637ae8cf1064c`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_install_presence_warmup.md`, `AI/auditor_constraints_install_presence_warmup.md`, `AI/tester_contract_install_presence_warmup.md`, `AI/walkthroughs/178_install_readiness_presence_warmup.md`, `Docs/DAEMON.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `README.md`, `crates/daemon/src/consensus.rs`, `crates/daemon/src/inference.rs`, `crates/daemon/src/presence/mod.rs`, `crates/daemon/src/presence/worker.rs`, `crates/daemon/tests/presence_wake_settle_consensus_tests.rs`, `crates/daemon/tests/presence_wake_settle_tests.rs`, `scripts/install.sh`, `scripts/wait_daemon_ready.sh`, `tests/invariants/src/install_presence_warmup_contract.rs`, `tests/invariants/src/lib.rs`

## 1. Executive Summary

GitHub #329 delivers three changes: (1) `scripts/wait_daemon_ready.sh` polls `soos-admin status` until it exits 0, within one `--timeout` bound and with each attempt bounded by `timeout --kill-after=1 5`; (2) `scripts/install.sh --build` runs a read-only check that refuses a cargo target directory holding entries not owned by the build user, or directories without `u+w`, and prints the `chown`/`chmod` fix; (3) a presence-only wake settle: when a presence scan wakes the camera, captures stamped before `wake + 1000 ms` are never evaluated, through `RequestDeadline::with_not_before`.

The raw diff and the code around it were reviewed. Author summaries were not relied on. The skip only filters captures out. It cannot reach `Allow` or erase a veto. Only the presence worker sets it. The dispatcher, config, protocol, PAM and camera-v4l code are byte-identical to `origin/main`. All gates are green: fmt, clippy `-D warnings`, `cargo test -p soos-daemon -p soos-invariants`, `cargo deny` and `bash -n`. No CRITICAL or MAJOR findings. Four MINOR findings and two SUGGESTIONs remain, all in operator messages or documentation wording.

## 2. Test Changes

Mechanical listing from the frozen patch:

- Test files touched: `crates/daemon/tests/presence_wake_settle_consensus_tests.rs` (new, +212), `crates/daemon/tests/presence_wake_settle_tests.rs` (new, +328), `tests/invariants/src/install_presence_warmup_contract.rs` (new), `tests/invariants/src/lib.rs` (+5 lines: the doc comment, `#[cfg(all(test, unix))]` and `mod install_presence_warmup_contract;`).
- Removed or changed assertions (`^-.*assert|#[test]|...`): **none**.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance or epsilon): **none**.
- Inline `mod tests` changes: **none**.

Only additions, as expected. No contract migration is needed. Spot checks of the new tests:
- `test_iwp_woken_scan_unlocks_after_spoof_looking_wake_frames` fails if `woke` is sampled after `notify_activity()`, because the scan then vetoes.
- `test_iwp_not_before_past_the_deadline_fails_closed` asserts `Pending`, 0 captures and 0 inferences.
- `test_iwp_spoof_after_the_settle_still_vetoes` and `test_iwp_constant_spoof_never_unlocks_a_woken_scan` assert the veto survives the settle.

Each of these fails against a plausible wrong implementation. Every test name referenced in matrix rows IWP1–IWP13 resolves to an existing `fn` (checked by grep).

## 3. Deep Reasoning Audit

### Logic & Architecture
- **Can the skip produce `Allow` on frames the rules would not accept?** In `consensus.rs:197`, a skipped capture only updates `last_sequence`. It touches neither the aggregator, the spoof capture, admission, the permit, inference nor estimate decay. `Allow` still requires `k` consecutive passing captures that `PadAggregator::record` actually evaluated, and the aggregator is never reset. The skip removes inputs and adds none, and an evaluated spoof still vetoes. → PASS.
- **Skip placement.** The skip sits before `is_frame_fresh`, the background preemption check, admission, `decay_estimate` (the `frames_evaluated() == 0` branch) and `acquire_within`. A settle therefore cannot decay the estimate, so it cannot make later PAM admission more permissive or less permissive. → PASS.
- **Timestamp 0.** Production frames are stamped with `CLOCK_MONOTONIC` at dequeue (`camera-v4l/src/capture.rs:745`, `v4l_impl.rs:994`). That is the same domain as the worker's `clock_fn` (`current_monotonic_nanos`). The stamp is 0 only when `clock_gettime` fails, and `0 < not_before` then skips the frame (fail closed). With bound 0, `x < 0` never holds for a `u64`, so PAM behaviour is unchanged. → PASS.
- **Deadline arithmetic.** `not_before = start_ns.saturating_add(1000 * 1_000_000)`. `compute(max(not_before, start_ns), 0, Instant::now(), 900 + margin + settle)` gives a client deadline of `not_before + 900 ms` and an outer deadline of `now + 900 ms + settle`, which agree. If `start_ns` saturates to `u64::MAX`, every capture is skipped, the result is `Pending` and `ScanOutcome::NoMatch` follows (fail closed). `MAX_ALLOW_TO_UNLOCK_MS` is measured from `allow_ns`, which is read after the consensus, so the longer window does not affect it. → PASS.
- **Woke detection race.** `woke = !is_ready()` is sampled before `notify_activity()`. If the stream stops between the sample and `notify_activity`, `woke` is false and the scan behaves as before #329, so nothing is weakened. A camera that PAM woke milliseconds earlier also gives `woke = false` (SUGGESTION 2). In the opposite case, a camera that has not yet suspended reports ready, which is correct. → PASS.
- **Scope / unchanged surfaces.** `git diff --stat origin/main` over `crates/daemon/src/dispatcher.rs`, `crates/daemon/src/config.rs`, `crates/daemon/src/presence/config.rs`, `crates/protocol`, `crates/pam` and `crates/camera-v4l` is empty. `with_not_before` is called only from `presence/worker.rs:675`. Nothing on the IPC path can set the bound: `compute` always sets 0. → PASS.
- **Preemption during the settle.** While captures are being skipped, the background loop does not check `interactive_demand`. It also holds no inference permit and no policy lock, so a PAM request is never blocked. Preemption fires on the first settled capture. → PASS.

### PAM Concurrency & Deadlines
- `crates/pam` is unchanged, and PAM windows come only from `compute` (`not_before = 0`, test IWP11). The worst-case presence scan grows to about 1.9 s plus the wake wait. It runs at background priority and cannot stall `login`, `sudo` or `gdm`. → PASS.

### Panic Safety & Fail-Closed
- The new Rust code has no `unwrap`, `expect` or indexing (`with_not_before` is a `const fn` struct update). No path leads from an error to `Allow`. A bound past the deadline gives `Pending`, then `NoMatch`.
- Shell checks:
  - `install.sh` under `set -euo pipefail`. A failed `find` sets `rc` and is refused ("cannot be fully inspected"). A target that is a dangling symlink or not a directory is refused. A target that does not exist is not checked; cargo creates it. `id -u -- user` failures skip the check, and `runuser` then fails closed.
  - Exercised in `--dry-run --destdir` with `/etc`, a dir holding a `u-w` subdir, an unreadable subtree, a plain file, a missing path and `-x<ESC>[31m`. Each gave the expected refusal or pass, and preflight exited 2 with `nothing was modified`.
  - `wait_daemon_ready.sh` was exercised with four fake admins. One unhealthy twice then healthy gave exit 0 and one JSON document. One always unhealthy gave exit 1, the last JSON and the stderr error. One that hangs and ignores TERM was killed by `--kill-after` (≈6 s), printed no partial output, then exited 1. A missing binary exited 1 within the bound.
  - The `install.sh` exit 70 mapping is unchanged (`install.sh:977-983`).
  → PASS.

### Test Integrity & Anti-Weakening
- See §2. Only additions, and the new tests discriminate. → PASS.

### Memory, Bounds & Secrets
- One added `u64` field and no new allocations. The template is still dropped (zeroized) right after the consensus.
- The new `debug!` logs only `settle_ms`: no frames, embeddings or credentials.
- Script loops are bounded:
  - The socket wait is capped at `2*T` polls.
  - The health poll breaks once `SECONDS - start >= T + 1`.
  - Each attempt is capped at 6 s.
  - An inherited or garbage `SECONDS` is harmless because only deltas are used (tested: `SECONDS=abc` → 0).

  `find -H` uses `-print -quit`, so the cost is at most one walk per check on a clean tree. → PASS.

### Supply Chain & Automation
- No change to `Cargo.*`, `deny.toml` or `.github/`. `cargo deny --locked check`: advisories, bans, licenses and sources all ok.
- `find` is required only under `--build`.
- Option injection is closed: a relative `CARGO_TARGET_DIR` gets a `./` prefix, so `-x…` cannot be parsed as a `find` option or predicate.
- User-controlled paths are printed with `printf '%q'` through `printf '%b…%s'`, never `echo -e`, so escape sequences stay inert.

→ PASS.

### English-Only Policy
- Code, comments, docs, ADR, matrix and walkthrough 178 are in English. The only non-ASCII characters are typographic symbols (`—`, `⇒`, `≠`, `✅`). → PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `scripts/install.sh:545-546`: when `find` fails (unreadable subtree, e.g. a `0000` directory owned by the build user, reachable in a non-root `--dry-run`/`--destdir` run), the advice is `sudo chown -R <user>: <target>`. Ownership is already correct, so that command does not fix the error. Correction: print an advice that covers both causes, e.g. `Fix it with: sudo chown -R <user>: <target> && chmod -R u+rwX <target>`, or say plainly that the subtree is unreadable and name the first `find` error.
- **[MINOR]** `scripts/install.sh:534-539`: only directories are checked for `u+w`. A file owned by the build user but read-only (e.g. `chmod -R a-w target`) is not detected. Cargo can still fail on files it rewrites in place. Correction: drop `-type d` from the second `find` (`! -perm -u+w` on every entry), or document that only directories are checked.
- **[MINOR]** `README.md:32` and `Docs/PACKAGING_AND_PROVISIONING.md` ("Start and readiness"): "waits at most 30 s" / "One bound, `--timeout` seconds ... plus a 1 s granularity margin" understates the real worst case. The last attempt starts before the bound is checked and may itself take up to 6 s (`timeout --kill-after=1 5`), plus the 0.5 s poll, so the wait can reach about `T + 7.5` s. Correction: state "about `--timeout` seconds (the final attempt may add up to 6 s)", or check the remaining budget before starting an attempt.
- **[MINOR]** `scripts/wait_daemon_ready.sh:126`: `2>/dev/null` discards `soos-admin`'s stderr on every attempt, including the last one. A persistent failure (permission denied on the socket, missing binary, protocol error) now prints only the generic "kept failing" line, where the previous version showed the admin error. Correction: keep the last attempt's stderr in a variable or temp file and print it with the final error.
- **[SUGGESTION]** `scripts/wait_daemon_ready.sh:131-135`: exit codes 126 and 127 (admin binary not executable or not found) are retried until the bound instead of failing fast. Consider exiting 1 at once on 126/127.
- **[SUGGESTION]** `crates/daemon/src/presence/worker.rs:634`: a camera that PAM woke just before the presence scan reports `is_ready()` and gets no settle. This is fail-safe and the pre-#329 behaviour, but the settle could instead be keyed on the stream start time, if the camera manager ever exposes it.

## 5. Final Verdict

**VERDICT: APPROVED**
