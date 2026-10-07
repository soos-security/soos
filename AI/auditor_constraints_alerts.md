# Auditor Constraints — GitHub #339 follow-up: Failed-Password Alerts in `soos-remote` (feature level 1)

- **Date**: 2026-10-06
- **Branch**: `feat/remote-auth-alerts` (from `feat/remote-companion` at `ea862cc`; nothing committed, pushed or
  stashed; O-5)
- **Audited**: `AI/architect_spec_remote_auth_alerts.md` (round 2), the drafted ADR "[2026-10-06] Failed-Password
  Alerts in `soos-remote` From the System Journal" (`AI/DECISIONS.md`), `AI/plan_evaluator_report.md` (round 2,
  APPROVED with G-1 to G-9), `AI/tester_contract_alerts.md` and its tests (`crates/remote/tests/{journal,alerts,
  alerts_server,journal_process}_tests.rs`, `tests/common/journal.rs`, the appended `mod alerts_contract` blocks,
  `tests/invariants/src/remote_alerts_contract.rs`), and the code to change: `crates/remote/src/{lib,server,routes,
  http,config,credentials,auth,audit,main}.rs`, `crates/remote/Cargo.toml`, `packaging/soos-remote.service`.
- **Owner decisions**: O-1 to O-5 (binding). The relayed request ("see on the phone the passwords that were tested")
  is answered by O-2 itself (an owner decision of the same day): only time, source class, account class, kind and
  count are shown. No constraint below may be relaxed to carry typed text; a request for it needs an `AGENTS.md`
  amendment and its own ADR (spec §0.1).

## Audit checklist results

| # | Check | Command / evidence | Result |
|---|---|---|---|
| 1 | Panic paths | `grep -nE '\.unwrap\(\|\.expect\(\|panic!\|todo!\|unimplemented!\|unreachable!' crates/remote/src/*.rs` | 0 hits today; workspace lints `unwrap_used`/`expect_used`/`panic`/`indexing_slicing`/`arithmetic_side_effects` deny; invariant `test_rmc_production_code_never_panics_or_prints` guards the new modules automatically (`remote_sources()` scans every file of `crates/remote/src`). |
| 2 | Forbidden raw-memory code | `grep -rnw` for the keyword over `crates/remote/src` | only the two `#![forbid(unsafe_code)]` lines (`lib.rs:13`, `main.rs:14`). `tokio::process` needs no `pre_exec`. The commit gate (`scripts/candid_review.sh:209`) fails on any added line under `crates/remote` (code, strings, tests; only lines starting with `//` are exempt) that contains the bare keyword; current untracked files: 0 hits. |
| 3 | Output isolation | `print!`-family in `crates/remote/src` | none; not a PAM crate, but RMC invariant forbids prints in production. |
| 4 | Bounded I/O & deadlines | spec §3.1 | every new input has a bound (line 24 576 B, fields 4096 B, cursor 256 B, probe 2 s, backoff 1–60 s, batch 256 lines / 50 ms, history 32, pending 16, recent 16, ack file 256 B, SSE ≤ 1/s, ack ≤ 1/s). PAM untouched. |
| 5 | Arithmetic | spec §3.3, §4.3 | `checked_add` on seq, saturating counts and windows; `u32`/`u64` conversions must use `try_from` (cast lints). |
| 6 | Filesystem | `credentials.rs:213-310` | `read_owned_file` accepts mode `0400` (`mode & 0o077`), which test 22 refuses → the ack read needs its own exact-mode check (C-20). `write_atomic` is reusable (G-6). |
| 7 | Secrets & privacy | spec A-8, A-11, A-14 | account mapped to three classes at parse time; no tracing in the new modules (RMC-S25); `Zeroizing` line/entry buffers (RMC-S31). |
| 8 | Fail-closed | spec A-3, §7 | state always in the view; no path from a feature failure to status/lock/unlock/PAM. One gap found: the follower must never *return* to the `serve` supervised set (C-26). |
| 9 | Supply chain | `Cargo.lock:3877-3889` | tokio 1.53.1 already depends on `signal-hook-registry` (the only extra crate of the `process` feature); no new crate, no new licence; `cargo deny --locked check` still mandatory in Phase 4. |
| 10 | CI / scripts | — | not touched (`scripts/install_remote.sh` only an optional informational `id -nG` note; C-41). |

Test integrity: `git diff --numstat -- crates/remote/tests tests/invariants` shows **additions only** (1 + 1 setup
lines in the two `RemoteConfig` literals, the appended `mod alerts_contract` blocks, one `mod` line). No assertion of
an existing test is changed. Compliant with the test-integrity invariant and the recorded setup-only exception (A-12).

## Audit Constraints — GitHub #339 follow-up (failed-password alerts)

### A. Privacy (O-2) and logging

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C-1 | No type, field, route, buffer or JSON key is designed to carry typed text, any part, length or hash of it. The only outputs are `AlertsView` / `AlertRecord` with the exact key sets of spec §4.4. No `len()`/byte count of `MESSAGE` or of the account field is stored, returned, compared outside parsing, or logged. | `alerts.rs::AlertsView`, `AlertRecord`, `server.rs` alert routes and SSE | tests 21, 33 (exact key sets, needles); candid review |
| C-2 | The account NAME is a borrowed slice of `entry.message`, mapped to `AccountClass` inside `classify_entry` and never copied (`to_string`, `to_owned`, `String::from`, `format!`), stored, hashed, or compared after mapping. Mapping = byte equality with `OwnerLogin`, then `== "root"`, else `Other` (case-sensitive). | `journal.rs::classify_entry` | tests 11, 33; review of `journal.rs` |
| C-3 | `journal.rs` and `alerts.rs` contain no `tracing` macro, no `tracing::` path, no `use tracing`, no print macro. Unparseable, overlong, foreign or untrusted lines are skipped **silently**. | `journal.rs`, `alerts.rs` | invariant `test_rmc_s25_alert_modules_never_log` |
| C-4 | The three new audit events are added to `audit.rs` in the existing static-callsite style (`AuditCallsite` + `Metadata::new` + `emit`), never through a `tracing` macro: `password alerts active` (INFO), `password alerts unavailable` (WARN), `password alert acknowledgement not persisted` (WARN). No field besides the constant `message`; the reason, epoch, counts, paths and errors are never attached. Each fires once per **transition** (not per retry, not per line). | `audit.rs`; callers in `alerts.rs`/`server.rs` | test 34; invariant RMC-S25 (`Level::WARN`) |
| C-5 | Any new `debug!` in `server.rs`/`main.rs` for the alert routes carries at most `%err` of `CsrfError`, `AckHeaderError`, `BookError` or `JournalError` (fixed `thiserror` strings); never an epoch, a `through`, a view, a count, a cursor, a path, an owner login, a uid or a header value. No field key from the RC-5 / D-H forbidden lists (`id`, `value`, `name`, `user`, `*_id`, `*_name`, `key`, `hint`, …). | `server.rs`, `main.rs` | invariants `test_rmc_logging_never_names_identity_header_or_session_fields`, `test_rmc_s15_auth_logging_hygiene`; test 33 (captured logs) |
| C-6 | `JournalEntry` has no `Debug` (neither derived nor manual). `OwnerLogin`, `JournalCursor`, `AlertsEpoch` have a manual `Debug` printing `<redacted>` and never `self.0`. `LineRead`, `FieldValue` and any private type holding line bytes or entry text also have no `Debug`. | `journal.rs`, `alerts.rs` | invariant `test_rmc_s28_…`; review |
| C-7 | Memory hygiene (A-14): `BoundedLineReader` owns one `Zeroizing<Vec<u8>>` whose capacity (`MAX_JOURNAL_LINE_BYTES + JOURNAL_READ_CHUNK_BYTES`) is reserved once and never exceeded (no `reserve`, no `extend` beyond capacity, no `shrink_to_fit`); consumed or discarded ranges are zeroized before being drained/reused; lines are handed out as `Zeroizing<Vec<u8>>`; no `BufReader`; every owned string of `JournalEntry` is `Zeroizing<String>`; the visitor wraps an owned string in `Zeroizing` immediately. `ConfigError`/`EntryError` messages never echo input. | `journal.rs::BoundedLineReader`, `parse_entry` | test 42 (`capacity()`), invariant `test_rmc_s31_…` |
| C-8 | `OwnerLogin` is never serialised, never part of the view, never logged; the page says "your account". The uid is never in a response. | `journal.rs::OwnerLogin`, `alerts.rs::view` | test 33 (needle = owner login and foreign UID) |

### B. Journal reading (O-3), process spawning

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C-9 | Exactly **one** `Command::new(JOURNALCTL_PATH)` in the whole crate, in `journal.rs`, used by both `probe` and `follow` through one private spawn helper. Always: `env_clear()`, `current_dir("/")`, `stdin(Stdio::null())`, `stderr(Stdio::null())`, `stdout(Stdio::piped())`, `kill_on_drop(true)`; argv exactly `probe_args()` / `follow_args()`; no shell, no `pre_exec`, no `--user`, `-g`, `--grep`, `--all`, `--merge`, `--directory`, `--file`, `--root`, `--cursor-file`; no `PATH` lookup. | `journal.rs::JournalctlSource` | invariant `test_rmc_s24_…`; test 1 |
| C-10 | The cursor reaches argv only as one element `--after-cursor=<cursor>` after `JournalCursor::parse` (1..=256 bytes of `[A-Za-z0-9=;_-]`); never split, never quoted, never a separate argv element (no option injection). | `journal.rs::follow_args`, `JournalCursor::parse` | tests 1, 2 |
| C-11 | The probe returns `Ok(())` only when a line read through the bounded reader within `JOURNAL_PROBE_TIMEOUT_MS` parses (bounded, as in §2.3 rules for `_UID`) and its `_UID` is exactly `"0"`; EOF or a line without `_UID == "0"` → `NoAccess`; deadline → `ProbeTimeout`; spawn `ENOENT` → `NotFound`, other spawn errors → `Spawn`. The child is killed (drop) on every exit path of the probe; nothing waits for its exit without a deadline. | `journal.rs::JournalctlSource::probe` | tests 26, 29; review |
| C-12 | `BoundedLineReader::next_line` and the production `JournalLines::next_line` are **cancel-safe**: the follower polls them inside `tokio::select!` against the idle tick and the shutdown signal, so no byte may be lost or duplicated when the future is dropped. State (`buf` length, `discarding`) is updated synchronously right after a completed `read`, with no `.await` between the read completing and the commit. A read error is `End`; a partial last line at EOF is wiped and dropped. | `journal.rs::BoundedLineReader::next_line`, follower loop in `alerts.rs` | test 42 (arbitrary split points); review of every `select!` around `next_line` |
| C-13 | `parse_entry`: length bound (`MAX_JOURNAL_LINE_BYTES`) checked **before** JSON parsing; `serde_json::Deserializer::from_slice` + hand-written `Visitor`; nine read keys in a fixed `[bool; 9]` seen-set (duplicate → `Shape`); unknown keys via `IgnoredAny`; per-field bound enforced while visiting (a string or byte sequence over its bound stops with `Shape`, no unbounded allocation; byte arrays collected into a pre-bounded `Vec`); `Deserializer::end()` required; no `serde_json::Value`/`Map` in `journal.rs`; no recursion beyond serde_json's built-in limit; never panics. `MESSAGE` with any byte `< 0x20` or `0x7F` → `Shape`. `__REALTIME_TIMESTAMP` and `_UID` parsed as strict decimal (1..=20 digits, `u64`/`u32` range by `str::parse` after an all-digit check; no sign, no whitespace). | `journal.rs::parse_entry` | tests 3, 4, 5 (proptest), invariant RMC-S31 |
| C-14 | Trust (A-5): common gate `_TRANSPORT == "syslog"`, facility 10, `_UID ∈ {0, owner_uid}`; `SYSLOG_IDENTIFIER` and `MESSAGE` are only grammar inputs; `_EXE` compared byte-exactly after `exe_for_comparison` (strips exactly one trailing ` (deleted)`); trusted rows exactly as the §2.4 table; prefix-anchored grammars (`starts_with`), never `contains`. `owner_uid` is the process uid (`ServerState::uid`), never a request value. | `journal.rs::classify_entry`, `exe_for_comparison` | tests 6–11, 51 |
| C-15 | Lock-screen coverage: `std::fs::metadata(path).is_ok_and(|m| m.is_file())` on at most `MAX_LOCK_SCREEN_PROGRAMS` configured paths at each follower start; no path is ever echoed; a stat failure is `NotConfigured`, never an error. | `alerts.rs` follower step 1 | tests 25, 27 |
| C-16 | Wall-clock time for `--since`, `started_us`, `follow_started_us` and idle expiry comes only from the injected `UnixClock` (`ServerState::unix_clock`); `SystemTime::now` stays confined to `main.rs`/`server.rs`. Monotonic timing (idle tick, backoff, throttles, rate gate) uses `tokio::time` only (paused-clock tests); never `std::thread::sleep` or `std::time::Instant` for waits. | `alerts.rs`, `server.rs` | invariants `test_rmc_production_code_never_panics_or_prints` (`SystemTime::now` rule), `test_rmc_s22_…` (`thread::sleep`); tests 26–28 (paused clock) |
| C-17 | Back-pressure and fairness on the current-thread runtime: at most `JOURNAL_LINES_PER_BATCH` lines are processed between two `JOURNAL_BATCH_PAUSE_MS` sleeps; no synchronous loop over an unbounded number of lines; backoff `JOURNAL_RESTART_MIN_MS` doubling (saturating) to `JOURNAL_RESTART_MAX_MS`, reset after a run ≥ `JOURNAL_STABLE_RUN_MS`. | `alerts.rs::run_follower` | tests 26, 28; compile-time relations (§3.1) |

### C. State, persistence and fail-closed (O-4)

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C-18 | Opt-in and gating (A-2, G-2): when `config.alerts.enabled == false` (or no source wired) nothing is spawned, **no random byte is drawn**, no ack file is read or written, the view is `Disabled`, the ack answers `403 alerts_disabled` (after CSRF, before header parsing), and no `alerts` SSE event is written. `with_password_alerts` is a no-op when disabled. Enabled without a wired source → `unavailable` / `journal_reader_failed`. | `server.rs::serve`, `ServerState::with_password_alerts`, ack route | tests 23, 55; existing server suites unchanged |
| C-19 | Epoch (A-13): drawn once per `serve` through `ServerState::random()` (8 bytes); never `getrandom::fill` outside `auth.rs`; never logged, never persisted; RNG failure → `unavailable` / `rng_failed`, nothing spawned, ack `503 unavailable`. Mismatch → `409 stale_view` with nothing changed. | `alerts.rs::AlertsEpoch::draw`, `server.rs::serve` | tests 53, 55; invariant `test_rmc_s16_csprng_is_getrandom_only` |
| C-20 | Ack file read: `OpenOptions` with `O_NOFOLLOW \| O_NONBLOCK \| O_CLOEXEC`; `fstat` of the opened descriptor: regular file, owner == `ServerState::file_owner_uid()`, **mode `& 0o777 == 0o600` exactly** (do not reuse `read_owned_file`'s `& 0o077` rule, which accepts `0400`), length ≤ 256 B, read through `take(257)`; a FIFO or directory refused without blocking; content must be a JSON **object** `{"version":1,"acknowledged_until_us":<u64>}` (array form refused, unknown keys refused, wrong types refused). Any error → marker 0 + one WARN (fail-safe). | `alerts.rs::read_ack_file` | test 22 |
| C-21 | Ack file write (G-6): reuse `credentials::write_atomic` (temp `<name>.tmp-<8 hex>`, `create_new`, mode `0600` at creation, `O_NOFOLLOW`, `sync_all`, `rename`, directory fsync, temp removed on failure); the random suffix comes from `auth::system_random()` (or the state's `RandomSource`), never a direct `getrandom` call and never a fixed temp name; the parent directory is the credential directory (spec §3.2 `resolve_alerts_ack_path`) and a write into a missing directory is an `Err`, never a panic. A symlink planted at the path is replaced, never written through. `ack_path == None` → in-memory only. | `alerts.rs::write_ack_file` | test 22; invariant RMC-S16 |
| C-22 | Marker rules R, A, M, P exactly as spec §4.3, plus G-1: `AlertBook::new` initialises `ack_high_water_us` **and** the "last written" value to the loaded marker, so two restarts without a new acknowledgement keep the file and the acknowledged state unchanged. The marker may only decrease towards the fail-safe direction (more alerts). | `alerts.rs::AlertBook` | tests 18, 19, 31, 52, 54 |
| C-23 | Concurrency of marker writes: on the current-thread runtime the computation of `marker(...)`, the comparison with the last written value, `write_ack_file` and the update of the last written value form **one synchronous section with no `.await`** (or are serialised by one async flush mutex), so an older marker can never be renamed over a newer one. The book's `std::sync::Mutex` is released before the file write and is **never** held across an `.await`, a socket write or a file write. | `server.rs` ack route, `alerts.rs` follower flush | review; tests 31, 54 |
| C-24 | Poisoned book mutex: handled with a match on `lock()` (no `unwrap`), view → `unavailable` / `journal_reader_failed`, ack → `503 unavailable`; never a panic. | `server.rs`, `alerts.rs` | review; invariant no-panic |
| C-25 | Bounds: `VecDeque` ≤ `MAX_ALERT_HISTORY`; pending ≤ `MAX_PENDING_CHECKS`; recent failures ≤ `MAX_RECENT_FAILURES`; anchors keyed by `(HelperSide, AccountClass)` (≤ 6); every count `saturating_add`; seq `checked_add` → `Overflow`; evicted totals tracked **per kind** (tester contract choice 5). No allocation proportional to journal volume. | `alerts.rs::Correlator`, `AlertBook` | tests 15–17, 20 |
| C-26 | **The follower never ends `serve`.** It runs under the `serve` supervision like the poller (a panic → `ServeError::TaskPanicked`, as the spec states), but every non-panic outcome — seq overflow, repeated probe failures, child exits — keeps the task alive: after `Overflow` the task sets `unavailable` / `overflow`, emits the WARN once and then awaits forever (`std::future::pending`) instead of returning a `ServeError`. Aborted with the other supervised tasks at shutdown (dropping the child kills it). | `alerts.rs::run_follower`, `server.rs::serve` | test 35 (serve does not return), review of the return type and every `return` |
| C-27 | No failure of the feature changes `/api/status`, `/api/events` status events, `/api/lock`, `/api/unlock` or the auth routes; the feature never reports `active` with zero attempts before the catch-up rule (§5 step 3); "No failed password attempts" is shown only in `active`. | `server.rs`, `assets/app.js` | tests 28, 35, 36; invariant RMC-S30 |

### D. HTTP surface and authentication (O-4)

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C-28 | `Route::Alerts` (`GET`/`HEAD /api/alerts`) and `Route::AlertsAck` (`POST /api/alerts/ack`) are **not** added to `is_funnel_public`; anonymous Funnel → `403 login_required` before routing (existing gate); tailnet callers pass the existing identity gate. `allow_header` returns `GET, HEAD` / `POST`. `accepts_body` unchanged (a body on the ack → `413`). | `routes.rs::route`, `is_funnel_public`, `allow_header` | tests 24, 39 |
| C-29 | Ack gate order exactly spec §6: CSRF (`check_alerts_ack_csrf` = `check_action_csrf(head, host, ACTION_ALERTS_ACK)`) `403 forbidden` → disabled `403 alerts_disabled` → `parse_alerts_ack_headers` `400 bad_request` → rate gate (`MIN_ALERT_ACK_INTERVAL_MS`, own `tokio::sync::Mutex<Option<Instant>>`, consumed by every request that reaches it) `429` → no book / poisoned `503` → `StaleView` `409` → `BeyondNewest` `400` → apply → persist (failure: WARN, still `200`). | `server.rs` ack route, `routes.rs` | tests 30, 40 |
| C-30 | `parse_alerts_ack_headers` uses `optional_single` on the lowercased names; epoch exactly 16 `[0-9a-f]`; through 1..=20 ASCII digits, `u64` via checked accumulation or `str::parse` after an all-digit check (no sign/whitespace); never reads `head.path`; error values never echo the header. | `routes.rs::parse_alerts_ack_headers` | tests 30, 40 |
| C-31 | SSE: `event: alerts` only when not `Disabled`; the first one right after the first `status` event; at most one per `ALERT_EVENT_MIN_INTERVAL_MS` per stream (latest view at the end of the interval); written with `write_bounded` (existing `RESPONSE_WRITE_TIMEOUT_MS`); alert writes never update the status `last_sent_at` keep-alive deadline. **On a Funnel stream, `session_still_valid(hash)` runs immediately before every `alerts` write; `false` ends the stream without writing.** The view is cloned under the book mutex and serialised after releasing it. | `server.rs::serve_stream`, `http.rs::encode_sse_alerts_event` | tests 24, 32, 41 |
| C-32 | `HEAD /api/alerts` returns headers only (existing `answer` path). `GET /api/alerts` builds the view from the current book snapshot; a serialisation failure → `503 unavailable`, never a panic. | `server.rs` | tests 23, 24 |

### E. Configuration, wiring, packaging, documentation

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C-33 | Config keys `password_alerts: Option<bool>`, `lock_screen_programs: Option<Vec<String>>` added to the `deny_unknown_fields` `FileConfig`; validation: ≤ 4 entries (`TooManyLockScreenPrograms{max}`), each absolute, 1..=4096 bytes, no trailing `/`, no `..` component, no control byte, not ending in ` (deleted)` (`InvalidLockScreenProgram{index}`, path never in the message); duplicates removed keeping order; `[]` kept. Both errors exit 78. | `config.rs::parse_config`, `AlertsConfig` | tests 37, 57 |
| C-34 | `resolve_alerts_ack_path` = sibling of the credential store; no parent or too long → `InvalidCredentialsPath`; no credential path configured → `ack_path = None`. | `config.rs` | test 38 |
| C-35 | `main.rs` resolves the owner login once with `nix::unistd::User::from_uid(getuid())` (error or invalid name → `None` → `owner_unresolved`, the service still starts) and wires `JournalctlSource` only when `password_alerts` is set; it reads no new environment variable and calls no test hook (`with_random`, `with_file_owner_uid`, `with_unix_clock`, `with_next_seq`, `force_newest_count`). | `main.rs` | test 29; invariant `test_rmc_s22_…`; review |
| C-36 | Test seams (`AlertBook::with_next_seq`, `AlertBook::force_newest_count`, `BoundedLineReader::capacity`, `JournalLines::child_pid`) are `#[doc(hidden)]`, cannot widen trust or bypass a bound, and are never called by production code. | `alerts.rs`, `journal.rs` | review; grep of `main.rs`/`server.rs` |
| C-37 | `crates/remote/Cargo.toml` tokio line becomes `tokio = { workspace = true, features = ["process"] }`; the workspace pin, every other manifest and `crates/pam` unchanged; no new crate; `cargo deny --locked check` green. | manifests | invariant `test_rmc_s27_…`; `cargo deny --locked check` |
| C-38 | `packaging/soos-remote.service` byte-for-byte unchanged (`RestrictAddressFamilies=AF_UNIX`, `NoNewPrivileges`, `MemoryDenyWriteExecute`, `RestrictSUIDSGID`, `UMask=0077`; no `SupplementaryGroups=`). | unit | invariant `test_rmc_s26_…`; `git diff --stat packaging/` empty |
| C-39 | Page: `textContent` only; no `innerHTML`, storage, `document.cookie` or `?through=`; the ack sends `X-Soos-Action: alerts-ack`, `X-Soos-Alerts-Epoch`, `X-Soos-Alerts-Through` from the displayed view, no body; `409 stale_view` → re-fetch, never auto-acknowledge; the anonymous sign-in view never calls alert routes; HH:MM via `toLocaleTimeString`. No new inline script (existing RMC-S8 CSP rules). | `assets/app.js`, `index.html`, `style.css` | invariants RMC-S8, `test_rmc_s30_…` |
| C-40 | Documentation drift from plan-evaluator round 2 that the spec has not folded must be corrected in the deliverables (spec, ADR, `Docs/REMOTE_COMPANION.md` §2c/§8, walkthrough 188): **G-3** spec §8 "in-memory lookup (no I/O)" is wrong — `session_still_valid` → `revoke()` reads the credential store file (bounded, ≤ 1/s per Funnel stream); **G-5** spec §15.1 and the ADR sentence "root-side signals cannot be forged without root" must say that only `_UID=0` lines are unforgeable and that `_EXE`-trusted lines (`sudo`, `su`, configured lockers) can be forged by an owner process through journald's PID-reuse race (false alerts and class relabelling only, never suppression of a trusted failure); **G-8** spec §7/§15.3 and §2c: a new group needs a new user-manager session (re-login), not a service restart; **G-4** the cold-start case (no line for 2 s right after spawn on a cold cache) is named as a residual in §15.9 and the ADR; **G-7** RMC-S24–RMC-S31. The §2c needles of RMC-S29 are mandatory. | `AI/architect_spec_remote_auth_alerts.md`, `AI/DECISIONS.md`, `Docs/REMOTE_COMPANION.md`, `AI/walkthroughs/188_remote_auth_alerts.md` | invariant `test_rmc_s29_…`; candid review |
| C-41 | O-5 host boundaries: no agent runs `tailscale`, restarts/reinstalls the service, edits `~/.config`, `/etc`, `journald.conf` or group membership; `scripts/install_remote.sh` may only print an informational note from `id -nG` (no `usermod`, no `sudo`). Test 43 only spawns a read-only `journalctl --follow` and kills it. | scripts, tests | invariant `test_rmc_s6_…`; review |
| C-42 | Gate before hand-off: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo test --workspace --locked --all-features`, `cargo deny --locked check`; the four new `soos-remote` suites and the full `soos-remote` suite run **at least 5 times** in a row green (flake lesson of this branch); no test file modified (`git diff --numstat` on existing test files: unchanged from this audit's baseline). No added line under `crates/remote` (sources, tests, assets, string literals) contains the bare keyword that `scripts/candid_review.sh:209` blocks on, and the other new files avoid it too (use "forbid lint", "raw-memory code" instead). | whole change | CI commands; `git diff --numstat`; `./scripts/candid_review.sh` |

### Pre-existing violations found (not introduced by this change)

- None in `crates/remote/src` (0 panic macros, 0 forbidden raw-memory blocks, no prints).
- Observation (not a violation): `credentials.rs::read_owned_file` accepts any mode with `mode & 0o077 == 0`
  (e.g. `0400`); acceptable for the credential store, but it must not be reused as-is for the ack file (C-20).
- Spec/ADR wording drift G-3, G-5, G-8 (round-2 plan-evaluator findings not yet folded) — fix in Phase 4/6 (C-40).

### Residual risks accepted (documented, not blocking)

1. An owner-UID process can create false alerts and, by sustained forged untrusted `pam_unix` lines, suppress
   helper-only owner-side lock-screen checks (spec §15.1); root-side attempts and the first failure per lock-screen
   PAM handle are unaffected. Consistent with O-3 ("a local process can at most create false alerts"), stated
   precisely in the ADR.
2. Kernel pipe buffer and `serde_json` scratch memory are not wiped (§15.7).
3. Catch-up heuristic and cold start (§15.9, G-4): transient `active` before the backlog is read on very slow storage.
4. `journalctl` under the exact unit sandbox is verified only on hardware by the owner (RMC59).
5. Synchronous ≤ 256-byte `fsync` writes of the ack file on the current-thread runtime (≤ 1/s from acknowledgements,
   ≤ 1/s from marker lowering), same precedent as the credential store.

### Clearance: CLEARED

The spec, the tester contract and the code to change satisfy the checklist; every risk found is expressed as a
constraint above (notably C-12 cancel safety, C-20 exact ack-file mode, C-21 temp naming, C-23 marker write
ordering, C-26 follower never ends `serve`, C-40 documentation corrections). The developer-agent may start Phase 4
and must satisfy C-1 to C-42.

---

## Round 3 — clear acknowledged entries (owner request 2026-10-06)

- **Date**: 2026-10-06
- **Branch**: `feat/remote-auth-alerts` (head `e8e3f28`; spec, ADR amendment and tests uncommitted; nothing committed,
  pushed or stashed by this phase)
- **Audited**: spec section "Round 3 — clear acknowledged entries" (R3.0–R3.8) of
  `AI/architect_spec_remote_auth_alerts.md`; the ADR amendment "clear acknowledged entries" of the alerts ADR in
  `AI/DECISIONS.md`; `AI/plan_evaluator_report.md` (round 3, APPROVED, F-1 to F-6); `AI/tester_contract_alerts.md`
  "Round 3" (tests 58–62, contract migrations of 18, 19, 21, 30, 31, 54 and the server-suite helpers); the code to
  change: `crates/remote/src/alerts.rs` (`AlertRecord`, `AlertBook::{record, acknowledge, marker, view_with}`,
  `LiveAttemptSink`, `AlertsRuntime::{acknowledge, record}`), `crates/remote/src/{server,push}.rs` (readers of the
  view / sink), `crates/remote/assets/{app.js,style.css,sw.js}`.
- **Owner request (binding)**: "once the acknowledge button is clicked, delete the entries" — acknowledged entries
  disappear from the page and the API at once and after a restart; journal deletion refused (R3.0); O-2 unchanged.

### Audit checklist results (round 3)

| # | Check | Command / evidence | Result |
|---|---|---|---|
| 1 | Panic paths | `grep -nE '\.unwrap\(\|\.expect\(\|panic!\|todo!\|unimplemented!\|unreachable!' crates/remote/src/*.rs` | 0 hits. The change uses `VecDeque::retain`, a two-variant enum and existing `saturating_*`/`checked_add`; no indexing is needed. |
| 2 | Forbidden raw-memory code | grep of the raw-memory keyword in `crates/remote/src/*.rs` | only the crate-level `forbid` attribute in `lib.rs`/`main.rs`; nothing to add. |
| 3 | Output isolation | invariant `test_rmc_production_code_never_panics_or_prints` | unchanged; no print added by the spec. |
| 4 | Bounded I/O & deadlines | spec R3.4/R3.8 | no new I/O: the ack file write path, format (≤ 256 B) and throttle are unchanged; no new route, header or status. |
| 5 | Arithmetic | `alerts.rs::AlertBook::record` | `seq` still `checked_add` before anything else (overflow reported even for an attempt that would be discarded — test 59); counts `saturating_add`; `ack_high_water_us` via `max`. |
| 6 | Filesystem | `write_ack_file` / `credentials::write_atomic` | untouched (C-20/C-21 still apply verbatim). |
| 7 | Secrets & privacy | `grep -nE 'tracing::\|info!\|warn!\|error!\|debug!' crates/remote/src/alerts.rs` | 0 hits; records hold only classes, times and counts (no password, no raw journal data); dropping them shrinks retained data. No new log/audit line. |
| 8 | Fail-closed | spec R3.4, ADR (d) | every residual errs towards **more** alerts (rule-M residual, ack-file write failure, partial ack after eviction); no path hides an unacknowledged attempt. |
| 9 | Supply chain | `git diff --stat HEAD -- Cargo.lock crates/remote/Cargo.toml deny.toml` | empty; no dependency change. `cargo deny --locked check` still required by the gate. |
| 10 | CI/workflow | — | not touched. |
| — | Red state | `cargo test --locked -p soos-remote --all-features --no-run` | compile-red exactly as the contract states: `E0432 unresolved import soos_remote::alerts::Recorded` (alerts_tests.rs:46), `E0063 missing field acknowledged` (alerts_tests.rs:1328). |
| — | Remaining `acknowledged` identifiers | `grep -rnw acknowledged crates/remote/src crates/remote/assets` | production code: `alerts.rs` 327, 414, 420, 440, 449, 488–489, 509, 535, 541; `app.js` 492–493; `style.css` 203. The other hits are comments or the fixed `StaleView` message (blanked/stripped by the test-62 scanner). |

### Audit Constraints — round 3

| # | Constraint | Applies to (file::fn) | Verified by |
|---|---|---|---|
| C-43 | **Drop, never flag** (R3.1, F-1): the `acknowledged` field, every binding, access and filter on it disappear from all production code of `crates/remote/src` (any visibility, any module; comments and the fixed `StaleView` message may keep the word). No replacement flag, set, tombstone or "removed ids" list may be introduced under another name: after a successful acknowledgement the removed `AlertRecord` values no longer exist anywhere in the process. | `alerts.rs::AlertRecord`, `AlertBook` | test 62 (`test_rmc_s43_acknowledged_entries_are_not_kept`, scanner self-test), test 58; review |
| C-44 | **A′ order and atomicity**: `AlertBook::acknowledge` keeps the exact round-2 refusal order (epoch → `through == 0` no-op → `BeyondNewest`), and nothing changes on a refusal. On success, `ack_high_water_us = max(ack_high_water_us, r.last_us)` is computed over **every** record with `last_seq <= through` **before** they are removed by one `VecDeque::retain(|r| r.last_seq > through)` (survivor order preserved); the evicted reset rule (`through >= evicted.max_seq`, `max_seq` kept) is unchanged. All of it runs inside the one `inner` mutex section of `AlertsRuntime::acknowledge`, so no view can observe a half-applied acknowledgement. | `alerts.rs::AlertBook::acknowledge`, `AlertsRuntime::acknowledge` | tests 18 (migrated), 53, 58, 60; review |
| C-45 | **R′ discard is side-effect free except for the seq**: in `AlertBook::record`, overflow is checked first (unchanged), then `highest_seq = seq`, then the predicate `attempt.at_us <= loaded_marker_us && attempt.at_us < started_us` returns `Ok(Recorded::Discarded)` without touching `records`, any count, `evicted` or `ack_high_water_us`. The predicate text is unchanged from rule R (no `<` / `<=` drift). | `alerts.rs::AlertBook::record` | tests 19 (migrated), 59 |
| C-46 | **Coalescing and eviction without the flag**: coalescing compares only source, account, kind and the `ALERT_COALESCE_WINDOW_US` window against the newest stored record; eviction (`len > MAX_ALERT_HISTORY`) always adds the popped count to the per-kind evicted totals (saturating) and updates `max_seq`/`first_us` — the old `continue` branch is deleted, never replaced by a silent drop. The `VecDeque` capacity stays `MAX_ALERT_HISTORY + 1`; no allocation depends on the number of acknowledged or discarded attempts. | `alerts.rs::AlertBook::record` | tests 52, 58 (1 000 attempts → ack → 40 → exactly `MAX_ALERT_HISTORY`) |
| C-47 | **Marker correctness (rule M unchanged)**: `marker()` uses every stored record (all unacknowledged), `evicted.first_us`, the oldest pending check and `ack_high_water_us`; it must stay strictly below every attempt known to be unacknowledged. The persisted value may be equal to or higher than round 2 only in the late-line case named by F-4, never lower than an unacknowledged attempt; no code path may lower `ack_high_water_us`. The ack file format, `ALERTS_MARKER_FLUSH_MIN_INTERVAL_MS` throttle, `flush(true)` after an acknowledgement and the C-23 single synchronous write section are unchanged. | `alerts.rs::AlertBook::marker`, `AlertsRuntime::flush` | tests 18, 19, 31, 52, 54 (migrated, numeric values unchanged), 60 |
| C-48 | **Runtime record path**: `AlertsRuntime::record` must match the `Result<Recorded, BookError>` explicitly: `Err(_)` → overflow path (unchanged); `Ok(Recorded::Discarded)` → neither `changed`, `bump()` nor the live list; `Ok(Recorded::Kept)` → `changed = true` and, only if `is_live(...)` holds, the attempt is pushed to the live list. No `let _ =` / `.is_err()`-only handling that would treat `Discarded` as `Kept`. The live list is still delivered by `deliver_live` after the `inner` mutex is released (lock order alerts → push unchanged), and the stale `LiveAttemptSink` doc comment ("Called under the alerts runtime mutex") is corrected (F-5). | `alerts.rs::AlertsRuntime::record`, `LiveAttemptSink` | test 61; clippy `-D warnings`; review |
| C-49 | **Push independence**: no change to `crates/remote/src/push.rs` is needed or allowed beyond compile fixes; the push scheduler never reads the book; an acknowledgement never calls the transport, never cancels, reduces or resends a pending summary or a carried count; replayed attempts (kept or discarded) are never pushed. | `push.rs`, `alerts.rs::AlertsRuntime::acknowledge` | test 61; `git diff push.rs` review |
| C-50 | **API shape**: `AlertsView` keeps its 10 top-level keys; each record serialises exactly the 7 keys `id`, `first_unix_ms`, `last_unix_ms`, `source`, `account`, `kind`, `count` (`last_seq`/`first_us`/`last_us` stay `#[serde(skip)]`). The `200` body of `POST /api/alerts/ack`, `GET /api/alerts` and the next `event: alerts` are computed from the book after the removal; no new route, header, status code, response key or query parameter. Authorization, Funnel gating, CSRF (`X-Soos-Action: alerts-ack`), header parsing and the rate gate (C-28 to C-31) are untouched. | `alerts.rs::AlertRecord`, `server.rs` ack/GET/SSE handlers | tests 21, 30 (migrated), 60 |
| C-51 | **Page**: `renderAlerts` loses the `record.acknowledged` branch and reads no acknowledgement field; `style.css` loses `.alerts-history li.acknowledged`; `textContent` only, no storage (`localStorage`/`sessionStorage`/IndexedDB), no service-worker cache of alert data, CSP and pinned asset headers unchanged (C-39 still applies). After a `200` the page renders the returned body. | `assets/app.js::renderAlerts`, `assets/style.css` | test 62; invariant RMC-S30, RMC-S8 |
| C-52 | **Journal untouched** (R3.0, ADR (e)): no code, script, unit or doc instruction may run `journalctl --vacuum-*`, `--rotate`, `--flush`, delete or truncate journal files, or widen the `journalctl` argument list of C-9; `packaging/soos-remote.service` stays byte-identical (C-38). `Docs/REMOTE_COMPANION.md` §2c states "journal entries are never deleted" with the reason (single entries cannot be deleted; root-only whole-file vacuum would destroy unrelated logs and the intrusion evidence). | `alerts.rs` spawn args, `scripts/`, `packaging/`, `Docs/REMOTE_COMPANION.md` §2c | test 62; invariant `test_rmc_s26_…`; `grep -rn 'vacuum\|--rotate' crates/remote scripts packaging` empty |
| C-53 | **Documentation folds the round-3 plan-evaluator findings that are not yet in the deliverables**: F-2 (both extra fail-safe residuals: ack-file write failure → entries reappear after a restart; > 32 records between view and ack → evicted counts stay in the totals) in spec R3.4, ADR (d) and §2c; F-4 rewording of ADR (b) and spec R3.1 ("identical persisted values" → "never lower than round 2 and still strictly below every unacknowledged attempt; identical in all migrated tests") — the current ADR (b) text still overclaims; F-6 UX sentence in §2c (a row that grew after display is kept with its new count; a removed list may flash back for under a second). English only; avoid the word that the commit gate greps for raw-memory code. | `AI/DECISIONS.md`, `AI/architect_spec_remote_auth_alerts.md`, `Docs/REMOTE_COMPANION.md` §2c, walkthrough 188 | candid review; invariant RMC-S29 needles still present |
| C-54 | **Traceability honesty** (F-4): the owner's 2026-10-06 hardware report (wrong `sudo` password → iPhone notification; Acknowledge resets the banner and it stays reset after a restart; GDM login not tested, owner: not important; the test and lock-screen notifications already in RMC59/RMC74; the phone listed under devices, host check `GET /api/push` shows one `apple` device) is recorded against RMC59/RMC74 as made on the round-2 build `e8e3f28`; RMC75 is **not** marked hardware-verified from it and needs its own owner check after the reinstall (rows disappear and do not return after a restart). No agent reinstalls or restarts the service to obtain it (C-41). | `AI/VERIFICATION_MATRIX.md`, walkthrough 188 | candid review |
| C-55 | **Test integrity**: tests 58–62 and the migrated assertions are immutable; the only permitted test changes are those already recorded in `AI/tester_contract_alerts.md` "Contract migration — owner request 2026-10-06". `Recorded` derives at least `Debug, Clone, Copy, PartialEq, Eq`, is `pub` in `soos_remote::alerts`, and carries no `#[must_use]` (tests drop it). | `crates/remote/tests/*`, `tests/invariants/src/remote_alerts_contract.rs`, `alerts.rs::Recorded` | `git diff --stat` of test files vs. this audit; compile |
| C-56 | **Gate before hand-off** (unchanged C-42 plus round 3): `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`; `cargo test --workspace --locked --all-features`; `cargo deny --locked check`; `alerts_tests`, `alerts_server_tests`, `push_server_tests` and the full `soos-remote` suite **at least 5 times in a row** green; `soos-invariants` green. Agents never commit, push, stash, open PRs, run `tailscale`, restart/reinstall host services or touch `~/.config`. | workspace | command output recorded by the developer |

### Pre-existing violations found (not introduced by this change)

- None in `crates/remote/src` (0 panic macros, 0 raw-memory blocks, no `tracing` macro in `alerts.rs`).
- Stale doc comment on `LiveAttemptSink` (`alerts.rs:803-804`, F-5) — fixed under C-48.
- ADR amendment (b) "persisted values are identical" overclaims (F-4) and (d) names only the rule-M residual (F-2) —
  fixed under C-53.

### Residual risks accepted (round 3, documented, not blocking)

1. Rule-M residual: acknowledged attempts above a marker held down by an older unacknowledged attempt or pending
   lock-screen check reappear after a restart (pinned by migrated test 54; fail-safe direction).
2. Ack-file write failure: entries vanish at once but reappear after a restart (WARN audit line already emitted).
3. Partial acknowledgement after eviction: evicted counts stay in the totals (no record data) until the next
   acknowledgement; the page may then show non-zero totals with fewer rows.
4. An `event: alerts` serialised just before the acknowledgement can briefly re-render the removed rows; the
   acknowledgement bumps the version, so the next event (≤ `ALERT_EVENT_MIN_INTERVAL_MS`) corrects it.
5. Removed records are plain freed memory (not wiped); they contain only classes, times and counts, no secret.
6. The journal keeps every entry by design (R3.0); the page is a view, not a deletion tool.

### Clearance: CLEARED

The round-3 spec, ADR amendment and tester contract satisfy the checklist; the drop-at-once design keeps every bound,
the epoch/stale-view order, the marker rules and push independence, adds no I/O, field, log line or dependency, and
retains no acknowledged data. The developer-agent may start Phase 4 for round 3 and must satisfy C-43 to C-56 (in
addition to C-1 to C-42), notably C-48 (explicit `Recorded` match, F-5 comment) and C-53 (F-2/F-4/F-6 wording).
