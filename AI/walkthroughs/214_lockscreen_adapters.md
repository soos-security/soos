# Walkthrough 214 — Lock-Screen Adapters: `soos-lockstatus` and the swaylock-plugin / hyprlock Widgets

- **Date**: 2026-10-16
- **Issue**: GitHub #401, PR 2 of the epic #352 (lock-screen face-scan widget). GitHub-only: no `AI/BACKLOG.md`
  entry, not registered in `scripts/sync_issue.py`; the commit carries `Closes #401` and `Refs #352`.
  **Branch**: `feat/lockscreen-adapters`.
- **ADR**: "[2026-10-15] Lock-Screen Adapters for swaylock-plugin and hyprlock" (`AI/DECISIONS.md`).
- **Documents**: spec `AI/architect_spec_lockscreen_adapters.md` (§9 Revision 1, §10 Revision 2, §11 Revision 3 and
  §12 Revision 4, §13 Revision 5, §14 Revision 6, §15 Revision 7, §16 Revision 8 and §17 Revision 9 supersede the conflicting earlier
  text); plan evaluation `AI/plan_evaluator_report.md` (round 3
  APPROVED); tester contract `AI/tester_contract_lockscreen_adapters.md`; auditor constraints
  `AI/auditor_constraints_lockscreen_adapters.md` (A1–A20, CLEARED); user documentation `Docs/LOCKSCREEN_WIDGET.md`;
  `Docs/IPC_PROTOCOL.md` §14 (client implementation); `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.5 (pointer).
- **Matrix criteria**: LKA1–LKA17 (LKA14–LKA17 pending: owner hardware and CI).

## 1. Context & Objectives

PR 1 (#380, walkthrough 205) gave the daemon a read-only `RequestKind::LockState` answering Idle / Scanning /
Not recognized for the caller's own UID. Wayland `ext-session-lock-v1` lets nothing draw over a lock screen, so each
locker needs its own adapter. Owner decision Q1 of 2026-10-13 puts the owner's own lockers first: swaylock-plugin
(driftwm host) and hyprlock. This PR delivers the client side for both: the user-level bridge `soos-lockstatus`
(planned PR 2) and the swaylock-plugin background widget with the hyprlock snippet (planned PR 3), merged by the
owner's instruction. Constraints: only the three coarse states are ever shown (Q2), animate for PAM and presence
scans alike (Q3, already published by the daemon), no biometric data, no socket permission change, no new privileged
component, reduced motion respected.

## 2. Architect Design

- **`soos-lockstatus`** (`crates/lockstatus`, lib + bin, `#![forbid(unsafe_code)]`): `client::query_lock_state` /
  `fetch_view` (one tagged request per connection, cumulative 250 ms deadline, non-blocking connect, re-armed socket
  timeouts, length checked before allocation, `LockStateResponse` first, any `Response` is a refusal);
  `view::{ViewState, LockView, Presented}` (Unavailable presented as Idle); `format` (`text`, `glyph`, `label` for
  `--once`, JSON for `--watch`, Pango-safe, no newline); `motion` (tokens mirrored from `Docs/UI_MOTION.md` with an
  `_MS` suffix, `PENDING_LOOP_MAX_MS` 4800, `resolve_motion_prefs`, bisection `cubic_bezier`); `shared` (per-user
  `flock` on `$XDG_RUNTIME_DIR/soos-lockstatus.lock`, private-file checks on the opened descriptor, 250 ms cache
  stamped with `CLOCK_BOOTTIME` and keyed by an FNV-1a socket hash, failure back-off 1/2/4 s in the record,
  `MIN_EXCHANGE_BUDGET_MS` 50, `Source::{Daemon, Cache, LockBusy, NoRuntimeDir}`); `poll` (`clamp_interval_ms`
  250..=2000, `Backoff`, `Poller`, `watch`, `output_closed`).
- **`soos-lock-widget`** (`crates/lock-widget`, lib + bin, `#![forbid(unsafe_code)]`): `palette` (brand colours
  mirrored from `theme.rs`, roles), `scene` (pure motion state machine), `layout` (geometry and memory budgets),
  `raster` (ring, 4 × 4 supersampled `STAR_SPIKES` star, bitmap text), `font` (5 × 7 glyphs of the two labels),
  `render` (`paint_full`, `paint_widget`), `wayland` (sctk 0.20 layer shell: no seat, background layer per output,
  two buffers, timer only while animating, poller thread, delayed exit 1).
- **Packaging**: `packaging/lockscreen/hyprlock-soos.conf` (one label, `cmd[update:500] soos-lockstatus --once
  --format label`). Installer opt-in stays a later PR of #352.
- **Invariants touched**: IPC Unix socket only and unchanged (`0660 root:soos`); no biometric data; PAM, daemon and
  protocol untouched; forbid-unsafe list extended (`lockstatus`, `lock-widget`, in `tests/invariants/src/lib.rs` and
  `scripts/candid_review.sh`).

## 3. Plan Evaluation

Three rounds (`AI/plan_evaluator_report.md`): round 1 REVISION_REQUIRED (MAJOR: uncoordinated pollers could take the
auth-reserved connection or exceed the 8/s quota; the star mark repeats every 180°, not 90°; acceptance coverage),
round 2 REVISION_REQUIRED (MAJOR: the star snapped back at each new scan of the presence cadence; MINOR: cache keyed
by socket, private modes, `CLOCK_BOOTTIME`, shared back-off, residual documented), round 3 APPROVED with two adopted
MINOR findings (`failures: u8` saturating at 3; a starved exchange is `LockBusy`, not a daemon failure). `openat`
was not adopted (nix 0.29 returns raw descriptors that cannot be owned without `unsafe`); the evaluator accepted the
same-user residual.

## 4. Tester Contract

All tests are new (`AI/tester_contract_lockscreen_adapters.md`); red was observed against signature stubs (assertion
failures, e.g. 15 of 17 `test_lka2_*`, 16 of 16 render tests). Files: `crates/lockstatus/tests/{lockstatus_client_tests,
lockstatus_shared_tests,lockstatus_view_format_tests,lockstatus_poll_tests,lockstatus_cli_tests}.rs` with the mock
daemon `crates/lockstatus/tests/common/mod.rs`; `crates/lock-widget/tests/{lock_widget_scene_tests,
lock_widget_render_tests,lock_widget_cli_tests}.rs`; `tests/invariants/src/lockscreen_adapters_contract.rs`.

Corrections made by the tester role before green, all on the new tests (no existing test touched, no assertion
weakened): two star sample points moved inside the thin spike tips (geometry), the clockwise check moved to 45°
(a 90° turn of a half-turn-symmetric mark cannot show direction), the `--watch` script gained a fourth change so the
closed pipe is actually written to, the mock's `Close` behaviour now closes at once (it lingered 300 ms probing for
a second frame), and two clippy-only rewrites (`<` instead of `+ 1 <=`, `contains`). Seven tests were added after the
reviews, each shown red against the code it corrects: `test_lka6_watch_exits_when_stdout_closes_while_the_state_is_stable`,
`test_lka8_scan_joining_a_running_turn_stops_within_the_loop_limit`, `test_lka13_usage_error_waits_the_restart_delay`
`test_lka8_daemon_restart_epoch_shakes_a_reused_sequence`, `test_lka8_scan_seen_again_after_a_transient_error_keeps_its_start`,
`test_lka5_poller_keeps_the_last_view_while_the_lock_is_busy` and `test_lka8_neutral_idle_answer_is_not_a_new_epoch`.

Existing tests migrated: none. `tests/invariants/src/lib.rs` only gains the module and two list entries.

## 5. Auditor Constraints

A1–A20 (`AI/auditor_constraints_lockscreen_adapters.md`), all met: no panic or print path (lints + invariant), forbid
`unsafe` in all four roots, one cumulative deadline (A3), length before allocation and strict decode order (A4), one
request per connection (A5), no success state (A6), private coordination file checked on the opened descriptor (A7),
bounded strict cache (A8), starved exchange not a failure (A9), shared and local back-off (A10), silent `--once`
(A11), nothing logged (A12), no seat (A13), memory budgets (A14), total drawing primitives (A15), idle draws nothing
(A16), delayed exit 1 (A17), no new crate in `Cargo.lock`, `cargo deny` green, `deny.toml` getrandom comment updated
(A18), one socket literal and no network (A19), PAM/daemon/protocol untouched (A20).

## 6. Implementation

- New: `crates/lockstatus/**`, `crates/lock-widget/**`, `packaging/lockscreen/hyprlock-soos.conf`,
  `Docs/LOCKSCREEN_WIDGET.md`, `tests/invariants/src/lockscreen_adapters_contract.rs`, the three new AI documents above
  (spec, tester contract, auditor constraints; `AI/plan_evaluator_report.md` and `AI/candid_review_report.md` are
  overwritten).
- Changed: root `Cargo.toml` (members, `soos-lockstatus`, `smithay-client-toolkit = { version = "0.20",
  default-features = false, features = ["calloop"] }`), `Cargo.lock` (two path packages only), `deny.toml` (comment),
  `scripts/candid_review.sh` and `tests/invariants/src/lib.rs` (business crate lists, module), `AGENTS.md` and
  `AI/ARCHITECTURE.md` (workspace trees), `Docs/README.md`, `Docs/IPC_PROTOCOL.md` §14, `Docs/DISTRIBUTION_DEPLOYMENT.md`
  §5.5, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `.agents/skills/dev-workflow/references/project-facts.md`.
- Notable decisions: the scene keys every Idle view (Unavailable included) as `(Idle, 0)`, so a transport error never
  draws differently; the star turn keeps its anchor while running and stops only on a half-turn boundary; the widget
  validates `WAYLAND_SOCKET` through `/proc/self/fd` before `connect_to_env` (a stale descriptor number would make
  the standard library abort on an I/O-safety violation in debug builds); `LockStatusError::Encode` was added for an
  impossible encode failure instead of mislabelling it `Decode`.
- Not verified on hardware: no headless wlroots compositor is available on the development host and the owner's live
  session was not used; rendering under swaylock-plugin and hyprlock is owner rows LKA14–LKA16.

## 7. Candid Review

Round 1 (fresh sub-agent, fingerprint `ca3be261bbc77195a0e062a9259e285f9b86c9575eb26dfccfe82a9115aa617e`):
**APPROVED** with three MINOR findings and two suggestions, all handled (spec §12): an orphaned `--watch` kept polling
while the state was stable (fixed: `poll::output_closed` before every poll, new test); the documentation claimed the
widget always loads `libwayland-client.so` while a standalone build uses the pure-Rust backend (documentation and
ADR corrected); a skipped output was silent (fixed: one stderr line, size only). Suggestions kept as follow-ups:
refuse `LockState` on the auth-reserved permit in the daemon; shrink the 512 MiB worst-case shared memory. 

Round 2 (fresh sub-agent, fingerprint `be47922d6ddec105fe730f569da07bd3ba230ab5291649d262cbdb23cf0acbea`, reviewing
the fixes and this traceability): **APPROVED** with four MINOR findings, all fixed (spec §13): a scan joining a running
star turn could turn it up to 7.2 s after its start (the extension now rounds down to the half-turn grid, new test);
the longest cache record is 79 bytes, not 76; a usage error and a closed connection exited at once, which
swaylock-plugin could restart in a loop (every exit but help/version now waits 1000 ms, new test); two documentation
inaccuracies (start-up stderr line scope, ADR section range). 

Round 3 (fresh sub-agent, fingerprint `2d39a1ed0ff534406d979a31d4906223441c8fbd577a953d21d382f2db7baa85`):
**APPROVED** with three MINOR documentation findings and three suggestions, all applied to the documentation only
(request deadline wording in `Docs/IPC_PROTOCOL.md` §14, LKA12 stderr wording, new-file count here, a troubleshooting
row for a slow logind session check, the uncapped hyprlock glyph noted in ADR item (9g), the hyprlock snippet
wording). 

Round 4 (fresh sub-agent, fingerprint `8d03692b1e7cf4432433e08f6ed81dd370e4298271aa54caa4286bd770f8c70c`):
**APPROVED** with two MINOR findings and three suggestions: the `--once` comments overstated the usage-error output
(reworded), the skipped-output stderr line now names the zero-size case, outputs beyond `MAX_OUTPUTS` are reported once,
and a daemon restart (lower sequence) re-arms the Not recognized shake (new test, spec §14). A busy coordination lock still read Idle for one poll here (fixed in round 6).


Round 5 (fresh sub-agent, fingerprint `78b483be8b53e2ff9ffa58b7744992cb250cc0203ee8ce5bf0bb6e087eb4f434`):
**APPROVED** with two MINOR findings and two suggestions, all applied (spec §15): the poller thread could deliver a
stale unsent view after the state returned (the newest poll now always replaces it); one flag silenced the second kind
of skipped-output report (two flags); two wording fixes. 

Round 6 (fresh sub-agent, fingerprint `a9ac220d0fae898b57fc7e9f45ee5a0546eefcf9a7488c0d13c7b25b54a714b3`):
**APPROVED** with one MINOR finding and one suggestion, both applied (spec §16): a scan seen again after a transient
poll error restarted its loop limit (the scene now remembers the start of each scan sequence, new test), and a busy
coordination lock now keeps the poller's last view instead of reading Idle (new test; this closes the flicker
follow-up). 

Round 7 (fresh sub-agent, fingerprint `0d47e13fb87203c21b6ad5f8c7cfcba1583f458e9432260be022533ace71064c`):
**APPROVED** with one MINOR finding and one suggestion, both applied (spec §17): the daemon's neutral answer (Idle,
sequence 0) was taken for a daemon restart (now only a lower non-zero sequence starts an epoch, new test), and the
LKA2 wording of the cache read bound. An eighth fresh review binds the final fingerprint (`AI/candid_review_report.md`).

## 8. Verification Results

Environment: `LIBONNXRUNTIME_NO_PKG_CONFIG=1 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3`, `--test-threads=3`.

- `cargo clippy --locked -p soos-lockstatus -p soos-lock-widget -p soos-invariants --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked -p soos-lockstatus --all-features`: 65 passed (client 13, shared 17, view/format 14, poll 8, CLI 13).
- `cargo test --locked -p soos-lock-widget --all-features`: 38 passed (scene 17, render 16, CLI 5).
- `cargo test --locked -p soos-invariants --all-features`: 707 passed (11 new `lockscreen_adapters_contract` tests).
- Both crates' suites run 10 times in a row: 0 failures.
- `cargo deny --locked check`: advisories, bans, licenses, sources ok. `./scripts/candid_review.sh`: passed.
- Full workspace gates (fmt, clippy, tests, deny, both candid layers) run by `./save.sh --push-pr`; CI on the pull request.

## 9. Known Limitations / Follow-ups

- Owner hardware rows LKA14–LKA16 (swaylock-plugin on the driftwm host, kill/restart and unchanged unlock paths,
  hyprlock); LKA17 (CI). Owner to confirm the ADR item (9) assumptions (a)–(h).
- Daemon follow-up: refuse to admit `LockState` onto the auth-reserved permit at accept time (removes the residual of
  ADR item 3).
- Memory: one full buffer plus small widget-box buffers would cut the 512 MiB worst case (two 8K outputs).
- When `feat/gui-motion` adds the motion tokens to `theme.rs`, extend `test_lka11_motion_tokens_mirror_the_normative_table`
  to compare them too.
- Later PRs of #352: GNOME Shell extension, KDE Plasma lock screen, installer opt-in (deciding which Wayland backend
  variant of the widget ships), `/etc/soos/lockwidget.disable`.
