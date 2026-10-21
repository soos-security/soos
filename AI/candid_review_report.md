# Candid Review Report

- **Date**: 2026-10-21
- **Target Branch**: `chore/readme-capture-chrome-cleanup`
- **Base (merge-base)**: `ecfefa4`
- **Reviewed-Diff-Fingerprint**: `53ed21e00db6b73e0275cb604e31a93ee4e47e2495a125579b1c2f394cbae662`
- **Audited Files**: `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/250_readme_capture_chrome_cleanup.md`, `Docs/README_ASSETS.md`, `scripts/capture_readme_web.sh`, `tests/fixtures/tests/readme_capture_cleanup_tests.rs`

## 1. Executive Summary

Dev-only change to the optional README web capture script (never a CI step). Each headless Chrome
run is started with job control on (`set -m` on the line before the background start, `set +m` as
the first line of `await_browser`), so it becomes its own process group; the whole group is ended
(SIGTERM, bounded 0.2 s poll for `BROWSER_KILL_AFTER_S` seconds, then SIGKILL) after every run, on a
timeout and from the exit trap. The RAC24 watcher now runs in the background and is awaited with the
`wait` builtin so `trap 'exit 130' INT` / `trap 'exit 143' TERM` act at once and exit through the
existing `trap cleanup EXIT`. No production Rust, PAM, IPC, daemon or shipped asset is touched. No
CRITICAL or MAJOR finding.

## 2. Test Changes

Mechanical listing from the frozen patch:

- Test files touched: `tests/fixtures/tests/readme_capture_cleanup_tests.rs` (new file).
- Removed/changed lines under `tests/`: **none** (`git diff origin/main...HEAD -- tests` has zero `-`
  lines); RAC22 (`readme_capture_locale_tests.rs`) and RAC24 (`readme_capture_timeout_tests.rs`)
  and `tests/invariants` are unchanged.
- New escape hatches (`#[ignore]`, skips, tolerances): **none**.

The new static test is not vacuous: replacing the script with the `origin/main` version makes it
fail (reproduced here, panic at `readme_capture_cleanup_tests.rs:91`, missing `set -m` before the
run); RAC22 and RAC24 tests pass on both versions. It pins `set -m` right before exactly the two runs
and nowhere else, `< /dev/null`, the SIGTERM → bounded poll → SIGKILL ordering in
`end_browser_group`, group signals in `stop_browser` with the group end after the reap, `set +m`
first in `await_browser`, the background watcher, the group end before the status check, the watcher
stop in `cleanup`, and the three traps.

## 3. Deep Reasoning Audit

### Logic & Architecture
- `set -m` in a non-interactive bash: the background `env … &` child calls `setpgid` so `$!` is the
  pgid of Chrome and every child that does not call `setsid`. Verified under a pty (`script -qec`):
  the fake browser's pgid equalled its pid and the terminal's foreground group stayed the script's,
  so no terminal ownership moves. No foreground command runs while `set -m` is on (only the
  `browser_pid=$!` assignment and the call into `await_browser`, whose first line is `set +m`). PASS.
- Group-kill safety: `end_browser_group` and the `-$browser_pid` signals are only reached with a
  non-empty `browser_pid` from `$!` (never 0, 1 or the script's own group). If job control had not
  taken effect, `kill -- -$pid` would fail with ESRCH (no such group) and fall back to the plain
  pid, so the script's own group is never signalled. The leader is reaped before
  `end_browser_group`; Linux keeps the id reserved while any group member lives. PASS (see
  SUGGESTION on the empty-group window).
- `set -euo pipefail`: `wait "$browser_pid" || status=$?`, `if ! wait "$watch_pid"`, and every kill
  guarded with `|| true` / `|| return 0`; `end_browser_group` returns 0 on an empty group. PASS.
- Chrome already exited: `kill -TERM -- -pgid` fails → `return 0`; `stop_browser` with a reaped pid
  (exit between reap and `browser_pid=""`) only re-runs a no-op group end. PASS.
- Trap order: INT/TERM traps call `exit`, which runs the EXIT trap once; `cleanup` kills the
  `timeout` watcher (which forwards to its own group with `tail`), stops the run, then the server.
  PASS.

### Empirical checks (this review)
- Fake browser forking a TERM-ignoring orphan and a normal orphan:
  - failing run (normal-end path): exit 1, zero orphans (SIGKILL escalation after 5 s works);
  - SIGINT mid-run (default disposition restored for the test harness): exit 130, zero orphans,
    fixture server gone;
  - SIGTERM mid-run: exit 143, zero orphans;
  - hang: exit 124 after 45 s, zero orphans.
- Real Google Chrome stable, full capture: every Chrome process sat in a group led by a run's main
  pid (never the script's group), crashpad handlers in their own sessions as documented; the run hit
  the known intermittent hang (locked-alerts light screenshot) → exit 124, zero Chrome processes right
  after and 2 s later; the PNGs written before were byte-identical (`git status` clean).
- `bash -n`, shellcheck (`koalaman/shellcheck:stable`), `cargo fmt --all --check`,
  `cargo test --locked -p soos-test-fixtures -p soos-invariants -- --test-threads=3`: all green
  (invariants 871 passed).

### PAM Concurrency & Deadlines
- Not applicable: no `crates/pam` change. PASS.

### Panic Safety & Fail-Closed
- No Rust production code changed. The script still fails loudly (124 on a timeout, the run status
  on a failure, 130/143 on a signal). PASS.

### Test Integrity & Anti-Weakening
- See §2: zero removed or modified assertions; one new additive contract test with a red proof.
  PASS.

### Memory, Bounds & Secrets
- Cleanup stays bounded (at most about `BROWSER_KILL_AFTER_S` for the main pid plus the same for the
  group). No new dependency, download, privilege or logged data. PASS.

### Supply Chain & Automation
- No change to workflows, lockfiles, images or `deny.toml`. PASS.

### English-Only Policy
- All added text (script comments, docs bullet, matrix row, walkthrough, test) is English. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `scripts/capture_readme_web.sh` `end_browser_group` — once the group is empty
  its id is free, and in theory an unrelated process could reuse that pid and become its own group
  leader between the reap and `kill -TERM -- "-$pgid"`. The window is milliseconds and needs a full
  pid wrap plus a `setpgid`/`setsid` by the new process of the same user; acceptable for a dev-only
  script, no change required.
- **[SUGGESTION]** `AI/walkthroughs/250_readme_capture_chrome_cleanup.md` §2 — "Switching job control
  on for a background start only moves no terminal ownership" reads awkwardly; the claim itself was
  confirmed under a pty. Reword at the next touch ("Job control switched on only for a background
  start does not move terminal ownership").
- **[SUGGESTION]** The walkthrough's "within 0.22 s" exit times apply when every group member honours
  SIGTERM; a member that ignores it delays the exit by up to `BROWSER_KILL_AFTER_S` (observed 5.08 s
  here with a TERM-ignoring fake child). Consistent with the documented bound; informational only.

## 5. Final Verdict

**VERDICT: APPROVED**
