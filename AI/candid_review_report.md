# Candid Review Report

- **Date**: 2026-10-05
- **Target Branch**: `feat/remote-companion` (GitHub #339, draft PR; owner decision: no merge without an explicit go)
- **Base (merge-base)**: `222665f`
- **Reviewed-Diff-Fingerprint**: `a2b42b2fdeab35ec6b6e8844e4ca3f8b276496b5eb040515d203f23372aa5a15`
- **Audited Files** (48, from `target/candid_diff.patch`, 11 095 lines):
  `.agents/skills/dev-workflow/references/project-facts.md`, `AGENTS.md`, `AI/ARCHITECTURE.md`,
  `AI/DECISIONS.md`, `AI/MOCK_STRATEGY.md`, `AI/VERIFICATION_MATRIX.md`,
  `AI/architect_spec_remote_companion.md`, `AI/auditor_constraints_remote_companion.md`,
  `AI/tester_contract_remote_companion.md`, `AI/walkthroughs/183_remote_companion.md`,
  `Cargo.lock`, `Cargo.toml`, `Docs/README.md`, `Docs/REMOTE_COMPANION.md`,
  `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `crates/remote/Cargo.toml`,
  `crates/remote/assets/{app.js, apple-touch-icon.png, icon.svg, index.html, manifest.webmanifest, style.css}`,
  `crates/remote/src/{assets.rs, config.rs, http.rs, identity.rs, lib.rs, logind.rs, main.rs, routes.rs, server.rs, session.rs, socket.rs, status.rs}`,
  `crates/remote/tests/{config_tests.rs, http_tests.rs, identity_tests.rs, routes_tests.rs, server_tests.rs, session_tests.rs, socket_tests.rs}`,
  `packaging/soos-remote.service`, `scripts/candid_review.sh`, `scripts/install_remote.sh`,
  `tests/invariants/src/lib.rs`, `tests/invariants/src/presence_unlock_contract.rs`,
  `tests/invariants/src/remote_companion_contract.rs`

This is a fresh, context-free review of the complete working tree (the whole change is
uncommitted: `git log origin/main..HEAD` is empty). It supersedes the earlier report bound to
fingerprint `793d899a…51f4`, which predates the Phase 6 documents (walkthrough 183, matrix rows
RMC1–RMC21, ADR item (4) wording, `Docs/REMOTE_COMPANION.md` corrections). I did not rely on
that report, the walkthrough or the author summaries; every statement below was checked
against the patch and the surrounding code.

## 1. Executive Summary

The diff adds the leaf crate `soos-remote` (user-level companion: real-time lock status over
Server-Sent Events and remote lock through `Manager.LockSession`, served on a `0600` Unix
socket in a `0700` directory behind `tailscale serve`), its user unit, a per-user installer,
operator documentation, an ADR, the static contract `remote_companion_contract.rs` (18 tests),
seven integration suites (97 tests) and the spec-mandated migration of
`test_pau_zbus_is_used_only_by_the_daemon`. No existing crate, PAM pathway, IPC schema,
daemon code or CI workflow changes (`crates/pam`, `crates/daemon`, `crates/protocol`,
`.github/` are absent from the patch).

Evidence gathered on the frozen tree (all commands run by me during this review):
`cargo fmt --all -- --check` clean; `cargo clippy -p soos-remote --all-targets -- -D warnings`
clean; `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` clean;
`cargo test -p soos-remote` → 97 passed, 0 failed (config 18, http 16, identity 9, routes 7,
server 28, session 10, socket 9); `cargo test -p soos-invariants` → 466 passed, 0 failed;
`cargo deny --locked check` → `advisories ok, bans ok, licenses ok, sources ok`; `Cargo.lock`
gains exactly one `[[package]]` (`soos-remote`) and no external crate.

I tried to break the design along every pillar: a path from a logind failure, a missing
header, a stale reading or a slow read to `unlocked`/`202`; an unbounded read, write,
allocation or stream; a race between the poller's `seq` and a stream's first read; a
rate-limit bypass through concurrency; a Host/Origin/CSRF bypass; a symlink or ownership race
on the socket directory; an identity or request datum reaching a log line; a weakened or
masked test. None of these produced a CRITICAL or MAJOR finding. Five MINOR findings and three
suggestions are recorded in §4 (two of them already known from the superseded report and now
carried in the ADR and the walkthrough follow-ups). The verdict is **APPROVED**.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Commands run on `target/candid_diff.patch`:

- **Test files touched**: `crates/remote/tests/{config,http,identity,routes,server,session,socket}_tests.rs`
  (all new), `tests/invariants/src/lib.rs`, `tests/invariants/src/presence_unlock_contract.rs`,
  `tests/invariants/src/remote_companion_contract.rs` (new).
- **Removed/changed checks** (`^-[^-].*(assert|#\[test\]|…)`): **one** hit, patch line 9743,
  in `tests/invariants/src/presence_unlock_contract.rs::test_pau_zbus_is_used_only_by_the_daemon`.
  The single `assert!(toml_table(&daemon, "dependencies").contains(&"zbus = { workspace = true }"))`
  over `crates/daemon/Cargo.toml` becomes the same assertion over each of exactly two allowed
  manifests (`crates/daemon/Cargo.toml`, `crates/remote/Cargo.toml`) **plus** two new
  assertions (`zbus` named exactly once per allowed manifest; both allowed manifests exist,
  `allowed_seen == ALLOWED.len()`). The "every other manifest under `crates/` and `tests/`
  must not mention `zbus`" loop is kept with `ALLOWED.contains(&rel.as_str())` replacing the
  single-path comparison; the PAM `no zbus / no dbus` assertion and the test name are
  unchanged; a doc comment cites the ADR. **Justification**: architect spec §2.12 "Contract
  migration" and ADR 2026-10-05 item (7), an owner-approved scope change of the 2026-10-02
  decision; the migrated test is strictly stronger for every manifest it already covered and
  weaker for none. Matrix row PAU17 keeps citing the test (italic scope annotation added).
  **Not a weakening.**
- **New escape hatches** (`#[ignore`, `#[cfg(any())]`, `should_panic`, `tolerance`, `epsilon`):
  **none**.
- **Inline `mod tests`** added or removed: **none** (all remote tests live in `tests/`; the
  lib and bin unit-test targets are empty, as `cargo test` reports).
- `tests/invariants/src/lib.rs`: `"remote"` appended to the `business_crates` list of
  `test_business_crates_forbid_unsafe_code` and `mod remote_companion_contract;` registered
  (strengthening only).

## 3. Deep Reasoning Audit

### Logic & Architecture

Scenarios attempted:

- **Dispatch order leak** — an unauthenticated `GET /nope` or a misdirected `Host` must learn
  nothing about routes. `server.rs::handle_connection` runs `parse → check_host (421) →
  authorize (403) → route → check_lock_csrf (403) → handler`; `test_rmc_identity_is_required_before_routing`
  and `test_rmc_host_is_checked_before_identity_and_routing` prove `403`/`421` on unknown
  paths with zero logind reads. PASS.
- **Stale `unlocked` replay to a new stream** (plan F2) — stream A sees `unlocked`, closes;
  logind fails; stream B opens. B's first event is its own `fresh_view` after `reserve_seq`;
  the poller resets the channel to `None` once `subscribers == 0`; `Sender::subscribe()` marks
  the current value seen. `test_rmc_events_new_stream_never_replays_a_stale_unlocked` covers
  `unavailable` and `locked`. PASS.
- **Slow read overriding a newer state** (R3-1) — the poller reserves `seq` *before*
  awaiting `own_sessions`; a stream drops any reading with `seq <= last_seq`. I walked both
  interleavings (poller reserves first / stream reserves first); in each the newer reservation
  wins and the older reading is skipped or superseded. `test_rmc_seq_is_reserved_when_a_read_starts`
  holds the poller read with the mock and asserts silence on B. PASS.
- **Keep-alive starvation** — `next_read_too_late = now + poll_interval >= keepalive_at`
  sends the reading that precedes the deadline; `keepalive_pending` sends the next reading
  after the deadline; `fallback_at = keepalive_at + poll_interval + SNAPSHOT_DEADLINE_MS`
  re-sends the newest unsent reading. With `poll_interval_ms` 250, 1000 and 10 000 an event
  arrives within `SSE_KEEPALIVE_MS` in every case I traced; the only silent branch (no new
  reading at `fallback_at`) requires a stopped poller, which ends the server. PASS.
- **Channel `None` handling** — a stream reading `None` `continue`s (no event, no break); a
  poller sleep spanning a stream close/open cannot publish `None` to a live subscriber
  (checked after the sleep against the current count). PASS.
- **Rate-limit atomicity** — the whole lock flow (gate acquisition, snapshot, select, record,
  `lock_session`) runs under one `tokio::sync::Mutex` guard inside
  `timeout(LOCK_FLOW_DEADLINE_MS)`; the interval is recorded only immediately before
  `lock_session` is awaited, so `409`/`503`-from-snapshot record nothing and a failed or cut
  `LockSession` does. `test_rmc_lock_flow_and_rate_limit` and
  `test_rmc_deadlines_bound_hung_logind_calls` cover each branch at exact boundaries. PASS.
- **Session selection** — candidates are `uid == Some(uid) && class == "user" &&
  remote == Some(false) && seat.is_some()`; active first. The tie-break among same-length ids
  is the **greatest** byte sequence (`"c9"` before `"10"`), as the tester contract and
  `test_rmc_select_session_prefers_active_local_user_seat_session` require, while spec D7 and
  auditor constraint 24 say "smallest". Deterministic, own-uid only, inactive ties only; ADR
  item (4) and `AI/ARCHITECTURE.md` §13 record the built behaviour. FINDING F1 (MINOR).
- **Missing `LockedHint`** — `locked = LockedHint == Bool(true)`; a missing or ill-typed
  property yields `locked == false` and therefore `unlocked` for a seated session. This is
  exactly what spec §2.6 `SessionProps`, auditor constraint 24 and
  `test_rmc_session_props_tolerates_missing_or_ill_typed_optionals` prescribe (parity with
  the presence worker), and `Docs/REMOTE_COMPANION.md` §3 states the desktop requirement; but
  it is weaker than the D8 sentence "never reports `unlocked` unless a fresh read returned
  `LockedHint == false`". FINDING F3 (MINOR, spec-internal, contract-bound).
- **Host normalisation** — exactly one `Host`, OWS trimmed, lowercased, one `:443` stripped,
  `is_valid_host_name`, `.ts.net` suffix with a non-empty prefix or exact allowlist
  membership; the normalized value feeds the `Origin` comparison (`https://<host>` or
  `https://<host>:443`). The table tests cover IP literals, IPv6 brackets, trailing dot,
  double port, `https://` prefix, non-UTF-8, 254 bytes. No per-label 63-byte bound. FINDING F2
  (MINOR).
- **HTTP parser bounds** — `read_head` caps the buffer at `MAX_REQUEST_HEAD_BYTES + 1`,
  re-parses after each 1024-byte chunk; `Partial` at the bound → `431`; `MAX_HEADERS` slots
  → `431`; path without query > 256 → `414`; `Transfer-Encoding` → `400`; any positive
  `Content-Length` → `413` (two different positive lengths → `400`); HTTP/1.0 and the h2
  preface → `400`. Proptests cover arbitrary input and every positive length. PASS.
- **Scope** — no unlock, no push, no camera, no change to `install.sh`/packaging (deferred by
  ADR item 6); nothing missing from the §10 acceptance mapping that the code can deliver
  (RMC20/RMC21 are owner/hardware checks, honestly marked pending). PASS.

### PAM Concurrency & Deadlines

`crates/pam` is not in the patch; no PAM pathway, deadline or IPC frame changes. The
companion's own deadlines: head read `timeout_at(accept + 5 s)`, every write
`timeout(2 s)` through `write_bounded`, lingering close `timeout(2 s)` and 64 KiB, snapshot
`timeout(1.5 s)`, lock flow `timeout(2 s)`, D-Bus connect `timeout(1 s)` and call
`timeout(500 ms)` plus `method_timeout`, stream lifetime 30 min, keep-alive 15 s. Every bound
is a `tokio::time` primitive and the tests drive them under a frozen paused clock at exact
boundaries (`REQUEST_HEAD_TIMEOUT_MS - 1` vs `+ 1`, `SNAPSHOT_DEADLINE_MS - 1` vs `+ 1`). A
hung logind cannot stall `login`/`sudo`/`gdm` because nothing on the authentication path
depends on this process. PASS.

### Panic Safety & Fail-Closed

- Grep of `crates/remote/src` for `.unwrap()`, `.expect(`, `panic!`, `unreachable!`, `todo!`,
  indexing: none in production code (the static test `test_rmc_production_code_never_panics_or_prints`
  enforces it with comments stripped, and `[lints] workspace = true` denies the panic family
  under `-D warnings`). The only `unwrap_or` family uses have total fallbacks
  (`checked_add` → `from`, `u64::try_from` → `u64::MAX`, `get(..)` → empty slice). PASS.
- Fail-closed paths: `SourceError::*` → `unavailable` (status) or `503` (lock);
  `serde_json` failure → `503`; `reserve_seq` overflow → `set_fatal` → `PollerEnded` → exit
  `EXIT_RUNTIME`; `ConfigError::*` and persistent `SocketError` → exit `EXIT_CONFIG` = 78
  (`RestartPreventExitStatus=78`); `SocketError::Io` → exit 1 (restart). No path converts an
  error into `202`, `unlocked` or `PAM_SUCCESS` (no PAM code exists here). PASS.
- `check_not_root(getuid, geteuid)` runs before `clap`, before any environment or file read.
  PASS.

### Test Integrity & Anti-Weakening

- §2 listing reviewed: the single migrated assertion is strengthened and justified by spec
  §2.12 / ADR (7). PASS.
- New tests can fail against plausible wrong implementations: the stale-replay test fails if
  the channel is not reset; the `seq` test fails if `seq` were reserved after the read or
  stamped from the wall clock; the keep-alive test asserts strictly increasing
  `checked_unix_ms` and "at most one keep-alive old"; the deadline tests assert no answer at
  `deadline - 1` and an answer at `deadline`; the connection-limit test asserts zero bytes to
  the 17th connection and 16 idle ones alive until exactly 5 s; the log test runs at `TRACE`
  with identity, Host, path and header-value probes. PASS.
- Mock masking: `MockSource` snapshots its answer when a call starts (models a slow read) and
  can hold calls; it cannot mask the D-Bus wire contract, which is covered by
  `session_props_from_properties` tests over hand-built `OwnedValue` maps (`a(susso)` listing
  shape, `(uo)`/`(so)` structures, boolean-only hints, µs → s). The production
  `ZbusSessionSource` is unexercised in CI, as `AI/MOCK_STRATEGY.md` states; this is the
  project's declared strategy for D-Bus. PASS, with the auditor's T1–T5 additions still
  absent (SUGGESTION S1).

### Memory, Bounds & Secrets

- Allocations: head buffer ≤ 8193 bytes, header vector ≤ 32 entries, config read through
  `take(16 385)`, `ListSessions` capped at 256 rows and 16 own rows before any per-session
  call, stream sink 64 bytes, lingering sink 1024 bytes / 64 KiB total, 16 connections, 4
  streams. PASS.
- Secrets: no frames, embeddings, passwords or keys exist in this crate (nothing to
  `Zeroize`). Identity: `TailscaleLogin` has a redacting `Debug`; `authorize`'s result is
  discarded by the caller; tracing fields are `%err` of fixed-text enums, `?resolved`
  (`Route` enum), `status`, `dbus_error` (logind error *name*, truncated on a char boundary),
  `socket_path` once at `info!`; the `TRACE` capture test and the static field-key denylist
  both pass. The JSON bodies carry exactly five fields and no identity. PASS.
- Files: socket directory created `0700` or opened `O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC`,
  owner checked by `fstat`, tightened by `fchmod`; stale socket unlinked only when
  `symlink_metadata` says socket-owned-by-uid; listener `0600` (bind → chmod window inside the
  `0700` parent and `UMask=0077`); config opened `O_NONBLOCK | O_CLOEXEC`, must be a regular
  file. The installer writes the template under `umask 077` then `chmod 0600`. PASS.
- `unsafe`: `#![forbid(unsafe_code)]` in `lib.rs` and `main.rs`; `remote` added to the
  business list, `scripts/candid_review.sh` and `AI/ARCHITECTURE.md`. PASS.

### Supply Chain & Automation

- `Cargo.toml`: `crates/remote` member, `httparse = "1.10"` (already locked through `ureq`),
  `soos-remote` path dependency. `Cargo.lock`: one new `[[package]]`, no new external crate,
  no `hyper`/`axum`/`tower` (asserted by `test_rmc_crate_is_registered_in_the_workspace`).
  `cargo deny --locked check` clean. PASS.
- `scripts/candid_review.sh`: only `remote` appended to `BUSINESS_CRATES`. `.github/`,
  `deny.toml`, `.githooks/` untouched. PASS.
- `packaging/soos-remote.service`: user unit, no `User=`/`Group=`, `NoNewPrivileges=yes`,
  `RestrictAddressFamilies=AF_UNIX`, `UMask=0077`, `RestartPreventExitStatus=78`, seccomp
  options that work in a user instance. PASS.
- `scripts/install_remote.sh`: `set -euo pipefail`, `EUID` root refusal, `bash -n` clean, no
  `sudo`/`tailscale`/`systemctl --user enable` outside `echo` text. Hard-coded
  `target/release` path ignores `CARGO_TARGET_DIR` (FINDING F5, MINOR).

### English-Only Policy

Grep of every added line for common French tokens: none. Code, comments, docs, UI strings,
unit, installer and walkthrough are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/remote/src/session.rs:158` — `select_session` breaks same-length ties
  by the **greatest** byte sequence (`Reverse(s.id.as_bytes())`), while
  `AI/architect_spec_remote_companion.md` D7 says "smallest session ID under (byte length,
  then bytes)" and `AI/auditor_constraints_remote_companion.md` constraint 24 prescribes
  `min_by_key(|s| (s.id.len(), s.id.as_bytes()))`. The tester contract item 16 and
  `test_rmc_select_session_prefers_active_local_user_seat_session` encode the built
  behaviour, and ADR 2026-10-05 item (4) plus `AI/ARCHITECTURE.md` §13 now record it. No
  security impact (own uid, local seat, inactive ties only; deterministic). **Correction**:
  reconcile spec D7 and auditor constraint 24 with the ADR wording in a documentation
  follow-up; never edit the test silently.
- **[MINOR]** `crates/remote/src/identity.rs:69` — `is_valid_host_name` bounds the whole name
  (≤ 253 bytes) but not each label (RFC 1035: ≤ 63 bytes). Harmless today (exact allowlist
  membership or `.ts.net` suffix, total bound, charset restricted). **Correction**: add
  `label.len() <= 63` with two table rows (63 accepted, 64 refused) in a later developer
  cycle with a fresh review.
- **[MINOR]** `crates/remote/src/session.rs:124` — a missing or ill-typed `LockedHint` maps to
  `locked = false`, so a seated `user` session is reported `unlocked` when the desktop never
  sets the hint. This follows spec §2.6 `SessionProps`, auditor constraint 24 and the
  contract test, and `Docs/REMOTE_COMPANION.md` §3 documents the desktop requirement; it is
  nevertheless weaker than D8's "never reports `unlocked` unless a fresh read returned
  `LockedHint == false`". **Correction**: architect follow-up — either amend D8 to the
  implemented rule or specify `locked: Option<bool>` with `None` → `unavailable` and migrate
  the contract test (owner-approved, not a silent edit).
- **[MINOR]** `crates/remote/assets/app.js:117` — `source.onerror` always treats the error as a
  transient reconnect and only fetches `/api/status`; when the browser closes the
  `EventSource` for good (`readyState === EventSource.CLOSED`, e.g. after `503
  too_many_streams`), the page keeps the last status until `STALE_UI_MS` and then shows
  `Unreachable`, with no new stream until `visibilitychange`/`pageshow`. UI robustness only;
  not an acceptance criterion. **Correction**: on `CLOSED`, schedule `openStream()` after a
  short back-off (same-origin `textContent` discipline unchanged).
- **[MINOR]** `scripts/install_remote.sh:54` — `install -Dm755 "${REPO_ROOT}/target/release/soos-remote"`
  ignores `CARGO_TARGET_DIR`; with that variable set the build succeeds elsewhere and the
  install step fails under `set -e` (safe failure, no partial install of the unit before the
  binary). **Correction**: resolve the artifact path from `cargo metadata --format-version 1`
  or honour `${CARGO_TARGET_DIR:-${REPO_ROOT}/target}`.
- **[SUGGESTION]** S1 — the auditor's non-blocking test additions T1–T5 (slow-reader stream
  write timeout, two concurrent lock requests, `HEAD /api/events` opens no slot, edge-table
  rows, static `O_DIRECTORY`/`O_NOFOLLOW` needle) are still absent. I verified each property
  by reading the code; adding the tests would pin them.
- **[SUGGESTION]** S2 — `crates/remote/src/main.rs:63` lets clap print `--help`/`--version` to
  stdout through `err.print()`, the only stdout path of the binary; acceptable for an
  operator-run `--help`, could route through tracing for strict output isolation.
- **[SUGGESTION]** S3 — `crates/remote/src/main.rs:41` reads `RUST_LOG` through
  `EnvFilter::try_from_default_env()`, an environment input not listed in spec §2.3 (the
  RMC-S9 invariant only matches `env::var` literals). It only sets the log level; the
  `TRACE`-capture test proves no identity or request datum is emitted at any level. Document
  the variable in `Docs/REMOTE_COMPANION.md` or pin the filter to `info`.

## 5. Final Verdict

No CRITICAL or MAJOR finding: no security invariant is broken, no path fails open, no test is
weakened or masked, no identity or request datum leaks, every I/O and allocation is bounded,
and the diff matches the approved revision-3 spec except for the documented tie-break wording
(F1) and the two spec-internal consistency items (F2, F3), all MINOR follow-ups.

**VERDICT: APPROVED**
