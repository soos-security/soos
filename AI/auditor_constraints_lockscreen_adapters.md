# Audit Constraints — GitHub #401: `soos-lockstatus` and `soos-lock-widget`

- **Date**: 2026-10-15 — **Branch**: `feat/lockscreen-adapters`
- **Inputs**: spec `AI/architect_spec_lockscreen_adapters.md` (§9–§11), tester contract
  `AI/tester_contract_lockscreen_adapters.md`, the stubs under `crates/lockstatus` and
  `crates/lock-widget`, `crates/admin-cli/src/test_pam.rs` (reviewed bounded-client pattern),
  `crates/daemon/src/dispatcher.rs` (`handle_connection`, `handle_lock_state_request`).

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| A1 | No `unwrap`/`expect`/`panic!`/`todo!`/`unimplemented!`/`unreachable!`/indexing/unchecked arithmetic in production code; workspace lints apply (`-D warnings`); every `#[allow]` carries a `reason` and is scoped to one item | both crates | clippy `--all-targets --all-features -D warnings`; `test_lka12_no_panic_or_print_macro_in_production_code` |
| A2 | `#![forbid(unsafe_code)]` first item of both `lib.rs` and both `main.rs`; no `unsafe` anywhere | both crates | `test_lka12_both_crate_roots_forbid_unsafe`; `test_business_crates_forbid_unsafe_code`; candid layer 1 |
| A3 | One cumulative deadline per poll covering the lock wait, connect, write and read: non-blocking `connect` retried until the deadline, `SO_SNDTIMEO`/`SO_RCVTIMEO` re-armed with the budget left before every write/read, never `Duration::ZERO` passed to `set_*_timeout`, `WouldBlock`/`TimedOut` → `Timeout`, `Interrupted` retried | `client::query_lock_state`, `shared::shared_fetch` | `test_lka1_stalled_*`, `test_lka1_late_answer_*`, `test_lka1_zero_budget_*`, `test_lka2_lock_held_*` |
| A4 | The response length prefix is checked (`0` or `> MAX_MESSAGE_SIZE` refused) **before** any allocation; the body is read byte-counted; strict decode `LockStateResponse` first, then `Response` → `Refused` (verdict never used), else `Decode`; `!is_current()` → `Version` | `client.rs` | `test_lka1_zero_and_oversize_*`, `test_lka1_refusal_*`, `test_lka1_other_version_*`, `test_lka1_garbage_*` |
| A5 | Exactly one request per connection; the stream is dropped on every path | `client.rs` | `test_lka1_request_fields_and_one_request_per_connection` |
| A6 | No error path yields a visible state other than Idle; no success state exists anywhere (no `Recognized`, no unlocked rendering) | `view.rs`, `format.rs`, `scene.rs` | `test_lka3_*`, `test_lka1_refusal_of_any_verdict_is_unavailable` (an `Allow` refusal included) |
| A7 | Coordination file: runtime dir absolute, `symlink_metadata` directory owned by the UID with `mode & 0o077 == 0`; lock file opened `O_RDWR|O_CREAT|O_NOFOLLOW|O_CLOEXEC` mode 0600, then `fstat` of the opened descriptor: regular file, owner, one link, `mode & 0o077 == 0`; any failure → `NoRuntimeDir` without daemon contact; the file is never `chmod`ed afterwards | `shared.rs` | `test_lka2_*runtime_dir*`, `test_lka2_unsafe_lock_file_*`, `test_lka12_coordination_file_is_private_and_checked` |
| A8 | The cache read is bounded (`MAX_CACHE_BYTES`, read with `take`), parsing is strict and total (malformed = miss), stamps are `CLOCK_BOOTTIME` taken after the exchange, a future stamp is a miss, `None` clock = no cache read/write, records are rewritten with `set_len(0)` + write at offset 0 under the lock | `shared.rs` | `test_lka2_record_*`, `test_lka2_malformed_*`, `test_lka2_freshness_*`, `test_lka2_missing_clock_*`, `test_lka2_future_stamp_*` |
| A9 | A starved exchange (< `MIN_EXCHANGE_BUDGET_MS` left after the lock) is `LockBusy`: no daemon contact, no failure count, no record | `shared.rs` | `test_lka2_starved_exchange_*` |
| A10 | Back-off: shared through the record (`failures` saturating at 3) and in `Poller` for `Source::Daemon` failures only; poll interval clamped (no busy loop with `0`) | `shared.rs`, `poll.rs` | `test_lka2_failures_back_off_*`, `test_lka5_*` |
| A11 | `--once`: no print macro; stdout via `write_all` + `flush`, errors ignored; nothing ever written to stderr (silent panic hook, `catch_unwind`, clap errors swallowed when `--once` is present); exit code 0 on every path | `crates/lockstatus/src/main.rs` | `test_lka6_*`, `test_lka12_stderr_is_only_written_by_the_binaries` |
| A12 | Nothing biometric and no state is logged: neither crate has a logger; the widget's stderr line names the setup error only | both crates | `test_lka13_no_compositor_*` (no state words), grep |
| A13 | Wayland: no seat, keyboard, pointer or touch object; `KeyboardInteractivity::None`; `Layer::Background`; namespace `soos-lock-widget`; no session-lock object | `wayland.rs` | `test_lka12_widget_binds_no_seat_and_draws_a_background_layer` |
| A14 | Memory: buffers only through `layout::buffer_len` (per-axis, per-surface and total pixel budgets, checked arithmetic); at most `MAX_OUTPUTS` surfaces, `BUFFERS_PER_SURFACE` buffers each; a surface over budget is not drawn; no per-frame allocation in `raster`/`render` | `layout.rs`, `wayland.rs`, `raster.rs` | `test_lka9_*`; code review |
| A15 | Drawing is total: every coordinate is clipped to the canvas and the clip, non-finite inputs draw nothing, float-to-integer conversions saturate (documented `#[allow(clippy::cast_*)]` with bounds) | `raster.rs`, `render.rs` | `test_lka10_*` |
| A16 | Frame pacing: redraw timer only while `FrameSpec::animating`, period `AMBIENT_REPAINT_MS`; idle = no timer; the poller thread sends only changed views through a bounded channel (`sync_channel(4)`, `try_send`, newest kept) | `wayland.rs` | code review; owner row LKA14 (idle CPU) |
| A17 | Every exit 1 of the widget (setup error, poller thread gone, caught panic) waits `SETUP_FAILURE_EXIT_DELAY_MS` first | `wayland.rs`, `main.rs` | `test_lka13_*` |
| A18 | Supply chain: no new crate in `Cargo.lock` besides the two path packages; `smithay-client-toolkit` 0.20.0 (MIT) with `default-features = false, features = ["calloop"]` (identical to the locked smithay-clipboard edge); `cargo deny --locked check` green; the `deny.toml` getrandom comment names `soos-lockstatus` | manifests, `deny.toml` | `test_lka12_dependency_allow_lists`; `dependency_tooling_contract`; `cargo deny` in `save.sh` |
| A19 | Socket path and permissions unchanged; the only `/run/soos` literal is `DEFAULT_SOCKET_PATH`; no network API | both crates | `test_lka12_no_network_and_one_socket_literal` |
| A20 | `pam_soos.so`, the daemon and the protocol crate are not modified | diff | candid review; `test_lks_pam_module_is_untouched` |

### Pre-existing violations found (not introduced by this change)

None relevant. Noted residual (ADR R2-5): a UID whose two regular daemon connections are held by
other clients still sends its single lock-status connection through the auth-reserved permit.

### Clearance: CLEARED
