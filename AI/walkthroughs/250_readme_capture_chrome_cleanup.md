# Walkthrough 250 — README Web Capture Ends the Whole Chrome Process Group

- **Date**: 2026-10-21
- **Issue**: follow-up of RAC24 (walkthrough 246 series, epic #356; GitHub-only, no backlog id) —
  **Branch**: `chore/readme-capture-chrome-cleanup`
- **Decision**: ADR 2026-10-13 "README Assets Are Generated From Synthetic Data, Never Hand-Edited"
  (unchanged)
- **Matrix criteria**: RAC25 (new, verified). RAC1–RAC24 unchanged; RAC18 stays pending (owner
  visual check).

## 1. Context & Objectives

`scripts/capture_readme_web.sh` (dev-only, never a CI step) bounds every headless Chrome run
(RAC24), but `stop_browser` and the exit trap only signalled the main Chrome pid. Chrome forks
renderer, GPU, zygote and utility children; nothing guaranteed that they ended with the main
process when the script exited, timed out or was interrupted. Goal: end the whole Chrome process
group in every case, keep the RAC24 timeout and the RAC22 run lines, and leave the capture output
unchanged.

## 2. Architect (Phase 1)

- **Own process group without changing the run line**: RAC22 requires both runs to start as
  `env "${BROWSER_ENV[@]}" "$browser" "${flags[@]}" …`, so `setsid` cannot prefix them. Instead,
  job control is switched on (`set -m`) on the line before each background start and switched off
  again by the first line of `await_browser`. A background job started with job control on gets its
  own process group (pgid = `$!`), and every Chrome child inherits it. Switching job control on
  for a background start only moves no terminal ownership. Such a job keeps the terminal as stdin,
  so both runs read `< /dev/null`.
- **`end_browser_group <pgid>`**: SIGTERM to `-<pgid>`, poll `kill -0 -- -<pgid>` every 0.2 s for at
  most `BROWSER_KILL_AFTER_S` seconds, then SIGKILL; a no-op when the group is empty. It runs only
  after the main process (the group leader) has been reaped.
- **`stop_browser`**: signals the group (`kill -TERM -- "-$browser_pid" || kill -TERM "$browser_pid"`,
  same for SIGKILL), keeps the RAC24 wait for the main pid, reaps it, then calls
  `end_browser_group`.
- **`await_browser`**: ends the group of a finished run before checking its status. Its wait now
  runs the RAC24 watcher (`timeout "${BROWSER_TIMEOUT_S}s" tail --pid=…`) in the background and
  awaits it with the `wait` builtin: Chrome no longer shares the terminal's process group, so it no
  longer gets Ctrl-C itself, and a foreground watcher would have delayed the trap by up to 45 s.
- **Traps**: `trap 'exit 130' INT` and `trap 'exit 143' TERM` exit through `trap cleanup EXIT`,
  which also stops the watcher.

## 3. Tester (Phase 2)

New `tests/fixtures/tests/readme_capture_cleanup_tests.rs`,
`test_rac_web_capture_ends_the_whole_browser_process_group` (static, same parser as RAC24): `set -m`
right before each of the two runs and nowhere else, `< /dev/null > /dev/null 2>&1 &`,
`end_browser_group` ordering (SIGTERM, bounded 0.2 s poll, SIGKILL), group signals in
`stop_browser` with the end after the reap, `set +m` first in `await_browser`, the background
watcher with `watch_pid=$!` and `if ! wait "$watch_pid"; then`, the group ended between the reap
and the status check, the watcher stopped by `cleanup`, and the three traps.

Red proof: against the script on `origin/main` the test fails; RAC22 and RAC24 tests pass before
and after, unchanged.

## 4. Auditor (Phase 3)

- Signals target only the group created for the run, whose leader is reaped by the script; the
  group id cannot be reused while a member is alive. The poll is bounded (5 s), so cleanup stays
  bounded (at most about 10 s on a stuck run, on top of the RAC24 bounds).
- No new dependency (`setsid` not needed), no download, no privilege; RAC16 forbidden-word list
  unaffected. Nothing is logged beyond the existing messages.
- Chrome's `chrome_crashpad_handler` processes start their own session (parent: the user
  subreaper), so no process-group signal reaches them; on the capture host they exit on their own
  when Chrome ends.

## 5. Developer (Phase 4)

- `scripts/capture_readme_web.sh`: `watch_pid`, `end_browser_group`, group signals in
  `stop_browser`, background watcher and group end in `await_browser`, `set -m` and `< /dev/null`
  on both runs, watcher stop in `cleanup`, INT/TERM traps, header comment.
- `Docs/README_ASSETS.md` §4: "No browser process outlives the script" bullet.
- `AI/VERIFICATION_MATRIX.md`: RAC25.

## 6. Verification

On the capture host (Arch, Google Chrome stable, no other Chrome running; counts are
`pgrep -f '^/opt/google/chrome'`):

- Full run: exit 0, Chrome group id equal to the main Chrome pid (not the script's group) while
  running, 0 Chrome processes right after exit and 2 s later; the four committed
  `web-*.png` were regenerated **byte-identical** (SHA-256 unchanged), so no PNG is committed.
- SIGTERM to the script mid-run: exit 143 within 0.22 s, 0 Chrome processes left.
- SIGINT to the script mid-run: exit 130 within 0.22 s, 0 Chrome processes left.
- A run hit the known intermittent Chrome hang (locked-alerts dark screenshot): exit 124 after 45 s,
  0 Chrome processes left; the PNGs written before it were byte-identical.
- Baseline (`origin/main` script): the timeout path also left no Chrome process on this host, and a
  probe that SIGKILLs only the main Chrome pid saw its children exit within 3 s. The orphan case
  was not reproduced here; the change makes the cleanup independent of Chrome's own parent-death
  handling.
- `shellcheck` (koalaman/shellcheck:stable) and `bash -n`: clean.
- `cargo fmt --check`, `cargo test --locked -p soos-test-fixtures -- --test-threads=3` and
  `cargo test --locked -p soos-invariants -- --test-threads=3`: green.
