# Audit Constraints — GitHub #405: GNOME Shell Unlock-Dialog Extension `soos-lockscreen@soos`

- **Date**: 2026-10-17 — **Branch**: `feat/lockscreen-gnome`
- **Inputs**: spec `AI/architect_spec_lockscreen_gnome.md`, tester contract `AI/tester_contract_lockscreen_gnome.md`,
  `crates/lockstatus/src/format.rs` (`--watch` JSON), GNOME Shell 45.0 and 51.1 sources (`layout.js`,
  `screenShield.js`, `sessionMode.js`, `extensionSystem.js`, `animationUtils.js`).

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| G1 | No call into `Main.screenShield`, no `unlock`/`deactivate`/`activate`, no `LoginManager`, `AuthPrompt` or `Gdm`: the extension has no path to end the lock | `extension.js` | `test_lkg3_no_unlock_path_and_no_input_signal` |
| G2 | No input: no key, button, motion, scroll, touch, enter/leave, `captured-event` or `event` signal, no grab or modal, no key focus; every actor `reactive: false`, `can_focus: false`, `track_hover: false` | `extension.js` | `test_lkg3_*` |
| G3 | Exactly one spawn site, `Gio.Subprocess.new([path, '--watch'], STDOUT_PIPE \| STDERR_SILENCE)`; no `GLib.spawn*`, shell, `-c`, launcher, `PATH` or environment lookup, no home directory | `extension.js` | `test_lkg4_subprocess_without_shell_and_no_network` |
| G4 | The binary is executed only if root-owned, regular, without group/other write bits and executable, from `/usr/bin` or `/usr/local/bin` | `findLockstatus` | `test_lkg4_*`; harness (user-owned and group-writable binaries refused) |
| G5 | Bounded reading: `read_bytes_async(256)`, lines ≤ 256 bytes, pending buffer ≤ 512 bytes; no `read_line`, `DataInputStream`, `communicate` or `splice` | `_readNext`, `_consume` | `test_lkg5_three_states_three_fields_bounded_lines`; harness (overlong line) |
| G6 | Strict parsing: exactly the keys `available`, `sequence`, `state`; the three states; boolean; safe non-negative integer; anything else ignored; a UTF-8 error ignored (`TextDecoder` fatal inside `try`) | `parseLine`, `_consume` | `test_lkg5_*`; harness (rejected lines) |
| G7 | No success rendering; `available:false` renders as Idle; transport failure (EOF, read error, exit, overlong line) renders Idle | `present`, `_bridgeFailed` | `test_lkg5_*`; harness |
| G8 | Every callback (signal handlers, async completions, GLib sources, ease `onComplete`, `repaint`) and `enable()`/`_sync()`/`_on*` bodies wrapped in `try`/`catch`; a failure stops quietly or renders Idle | `extension.js` | `test_lkg6_disable_cleans_up_and_callbacks_are_guarded` |
| G9 | No sensitive or variable data in logs: only fixed `console.warn` strings, never the state, a line, a path from the bridge or an exception; no `console.log`/`log`/`logError` | `extension.js` | `test_lkg6_*` |
| G10 | `disable()` documents the `unlock-dialog` use and removes both signal handlers, both GLib sources, the process (`cancel`, `force_exit`) and the card; each step in its own `try`; idempotent | `disable` and helpers | `test_lkg6_*`; harness (disable while locked, re-enable) |
| G11 | At most one bridge process, one restart timer and one turn timer; stale async completions of a stopped bridge are ignored (identity check) | `_startBridge`, `_scheduleRespawn`, `_startTurning`, `_bridgeFailed` | harness (no double restart after kill) |
| G12 | Kill switch read before anything is built at every lock | `_sync` | `test_lkg7_kill_switch_and_reduced_motion`; harness |
| G13 | Reduced motion through `St.Settings.enable_animations` only (no own `GSettings`, no D-Bus); no turn or shake while reduced | `_reducedMotion`, CSS | `test_lkg7_*`, `test_lkg4_*`; harness |
| G14 | No network, D-Bus, `Soup` or socket use | `extension.js` | `test_lkg4_*` |
| G15 | PAM, daemon, protocol and `soos-lockstatus` untouched; no new Rust dependency; `Cargo.lock` unchanged | change set | `test_lkg7_extension_change_set_leaves_pam_daemon_protocol_and_bridge_alone`; `git diff --stat` |
| G16 | Invariants test code follows the workspace lint policy (scoped `#![allow]` with `reason`), no `unsafe` | `lockscreen_gnome_contract.rs` | clippy `-D warnings` |

Clearance: CLEARED
