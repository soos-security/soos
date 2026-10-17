# Walkthrough 215 — GNOME Shell Lock-Screen Extension `soos-lockscreen@soos`

- **Date**: 2026-10-17
- **Issue**: GitHub #405, PR 4 of the epic #352 (lock-screen face-scan widget). GitHub-only: no `AI/BACKLOG.md`
  entry, not registered in `scripts/sync_issue.py`; the commit carries `Closes #405` and `Refs #352`.
  **Branch**: `feat/lockscreen-gnome`.
- **ADR**: "[2026-10-17] GNOME Shell Lock-Screen Extension" (`AI/DECISIONS.md`).
- **Documents**: spec `AI/architect_spec_lockscreen_gnome.md`; plan self-check `AI/plan_evaluator_report.md`
  (APPROVED); tester contract `AI/tester_contract_lockscreen_gnome.md`; auditor constraints
  `AI/auditor_constraints_lockscreen_gnome.md` (G1–G16, CLEARED); user documentation `Docs/LOCKSCREEN_WIDGET.md` §11;
  `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.5 (GNOME note).
- **Matrix criteria**: LKG1–LKG11 (LKG9–LKG11 pending: owner hardware on Ubuntu GNOME and CI).

## 1. Context & Objectives

PR 1 (#380, walkthrough 205) gave the daemon a read-only `LockState` request; PR 2 (#401, walkthrough 214) added the
user-level bridge `soos-lockstatus` with `--watch` JSON lines and the swaylock-plugin and hyprlock adapters. The
owner's order (Q1 of 2026-10-13) puts GNOME next, as a **system** extension in the `unlock-dialog` session mode
(owner comment of 2026-10-12). GNOME Shell is its own locker, so no external program can draw on its lock screen;
an extension that lists `unlock-dialog` is the supported extension point. Constraints carried over: only Idle,
Scanning and Not recognized (Q2), the same animation for PAM requests and presence scans (Q3), no biometric data,
no new privileged component, no change to PAM, the daemon or the protocol, the brand and `Docs/UI_MOTION.md` tokens.
The `/etc/soos/lockwidget.disable` kill switch, deferred by ADR 2026-10-15 item 8, lands here.

## 2. Architect Design

- **Files**: `packaging/gnome-shell/soos-lockscreen@soos/metadata.json` (uuid, `session-modes`
  `["user", "unlock-dialog"]`, `shell-version` 45–51), `extension.js` (ESM, `export default class … extends
  Extension`), `stylesheet.css` (brand palette by value).
- **Lifecycle**: `enable()` connects `Main.sessionMode` `updated` and `St.Settings` `notify::enable-animations`, then
  `_sync()` builds the card and starts the bridge while `Main.sessionMode.isLocked` and the kill switch is absent, and
  removes both otherwise. `disable()` carries the review comment on the `unlock-dialog` use and removes both signal
  handlers, both GLib sources, the process and the card, each step in its own `try`.
- **Bridge**: `Gio.Subprocess.new([path, '--watch'], STDOUT_PIPE | STDERR_SILENCE)` with `path` the first root-owned,
  not group/other-writable, executable regular file among `/usr/bin/soos-lockstatus` and
  `/usr/local/bin/soos-lockstatus`; 256-byte reads, lines of at most 256 bytes; strict three-key parsing;
  `available:false` drawn as Idle; restart after 1, 2, 4 then 8 s while locked.
- **Card**: one non-reactive `St.Widget` in a non-reactive frame bound to the primary monitor
  (`Layout.MonitorConstraint({primary: true})`, as the shell's unlock dialog) inside
  `Main.layoutManager.screenShieldGroup` (top centre, below the top bar),
  ring with three state classes, the brand star drawn with cairo from `STAR_SPIKES`, an `St.Label` with the words.
- **Motion**: ring cross-fade 180 ms (CSS), quarter turns of 1200 ms while scanning up to 4.8 s after the latest
  scan start, rest on a half turn, one shake (≤ 8 px, 260 ms) per new Not recognized sequence, 180 ms fade-in;
  reduced motion from `enable-animations` through `St.Settings` (no turn, no shake, 0 ms transition).
- **Invariants touched**: none of PAM, daemon, protocol, IPC socket or permissions; no new Rust dependency.

## 3. Plan Evaluation

Self-check against the architecture, the ADRs, the matrix and the GNOME Shell sources: every shell API was read in
the GNOME 45.0 sources (fetched from the upstream tag) and in the 51.1 sources embedded in the locally installed
`libshell-51.so` (extracted with a small GJS `Gio.Resource` dump), the `gnome-extensions` behaviour in the 45.0
extensions tool. Verdict APPROVED; the owner-to-confirm defaults (version list, position, binary candidates,
easing approximation) are listed in spec §7.

## 4. Tester Contract

`tests/invariants/src/lockscreen_gnome_contract.rs`, 11 tests in LKG1–LKG8 (metadata; ESM shape and an optional
`node --check`; no unlock path or input signal; spawn without shell and no network; three states, three keys and
bounded lines; `disable()` cleanup and guarded callbacks; kill switch, reduced motion and the change-set guard;
palette, tokens, star, documentation). Red against stub files: 9 failed, the 2 guards passed. Existing tests
migrated: none; the only edit to an existing test file is the `mod` declaration in `tests/invariants/src/lib.rs`.

## 5. Auditor Constraints

G1–G16 (`AI/auditor_constraints_lockscreen_gnome.md`): no unlock or input path, one spawn site without a shell,
root-owned binary only, bounded and strict reading, no success rendering, guarded callbacks, fixed log messages,
complete `disable()`, single process and timers with stale-completion checks, kill switch before building, reduced
motion without an own `GSettings`, no network or D-Bus, PAM/daemon/protocol/bridge untouched. CLEARED.

## 6. Implementation

- `extension.js` keeps all logic in one class plus four pure helpers (`parseLine`, `present`, `findLockstatus`,
  `drawStar`). Async completions carry their `bridge` object; `_bridgeFailed` ignores a bridge that is no longer
  current, so the cancelled read and the cancelled `wait_async` of a stopped process never schedule a second
  restart.
- The restart back-off index resets only when a valid line arrives, so a bridge that exits at once is retried at
  most every 8 s.
- `_applyView` keeps a scan's start while the same sequence is reported again (after a transient Idle) and restarts
  it for a new sequence or after the daemon's 10 s scan limit; the turn timer is single and ends itself.
- The star colour comes from the CSS `color` of `.soos-lock-star` through `get_theme_node().get_foreground_color()`,
  read field by field (`red`, `green`, `blue`, `alpha`), which works with `Clutter.Color` (45, 46) and `Cogl.Color`
  (47 and later).

## 7. Candid Review

Not run by the implementing agent: the independent pre-push review (`./scripts/candid_subagent.sh --prepare`,
fresh context) is performed separately and writes `AI/candid_review_report.md`.

## 8. Verification Results

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked -p soos-invariants --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked -p soos-invariants -- --test-threads=3`: all green, including `lockscreen_gnome_contract`
  (11 tests), `matrix_citations` and `lockscreen_adapters_contract`.
- `node --check` on the extension (Node 20): passes.
- Node harness with mocked shell modules (scenarios listed in the tester contract): all assertions pass.
- GJS 1.90 run with real `GLib`/`Gio` against a fake `--watch` script: states seen Scanning → Not recognized
  (split line) → Idle (EOF), restart timer armed, `disable()` leaves no card, timer or process.
- Not verified here: a real GNOME Shell session (CI has none): owner rows LKG9 and LKG10.

## 9. Known Limitations / Follow-ups

- Owner to confirm: `shell-version` 45–51, the top-centre position, the binary candidates, `EASE_OUT_CUBIC` for
  `EASE_STANDARD`.
- The installer opt-in (copying the extension, installing `soos-lockstatus` root-owned) is PR 6 of #352; KDE is PR 5.
- A `sequence` above 2^53 (never reached in practice) is not a safe JavaScript integer and its lines are ignored.
- The change-set guard of LKG7 runs only in full clones; the CI test job is shallow.
