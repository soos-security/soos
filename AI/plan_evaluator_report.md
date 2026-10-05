# Plan Evaluation Report
- **Date**: 2026-10-05
- **Issue**: GitHub-only issue — Remote companion for real-time lock status and remote lock, `soos-remote` (GitHub #339; no `AI/BACKLOG.md` entry, acceptance taken from the issue body)
- **Branch**: `feat/remote-companion`
- **Base commit**: `222665f`
- **Plan evaluated**: `AI/architect_spec_remote_companion.md` **Revision 3** (round 3)
- **Round history**:
  - Round 1 (Revision 1): `REVISION_REQUIRED`, findings F1–F12.
  - Round 2 (Revision 2): `REVISION_REQUIRED`, findings R2-1 (MAJOR) and R2-2 to R2-6 (MINOR).
- **Scope of this round**: a focused re-check of D6, §2.6, the §2.4/§2.5 CSRF signature, §4, §8 RMC-S9 and §2.12, plus any side effects of these edits. The six owner decisions are binding and were not reopened.

## 1. Coverage Matrix

| Acceptance line (GitHub #339) / TDD test | Spec element (Rev. 3) | Status |
|---|---|---|
| Real-time status: locked / active / idle / no session | D6, §2.6 `Reading { seq, view }`, poller `send_replace` on every read, per-stream change detection + keep-alive re-send, §2.11 | Mapped (R2-1 resolved) |
| Remote lock | D9, §2.5, §2.8 | Mapped |
| iPhone PWA | §2.11, D11 PNG `apple-touch-icon` | Mapped |
| No hosted site / no cloud | D2, D5a, ADR (2)(8) | Mapped |
| User-level, never root | D1, `check_not_root`, RMC-S10 | Mapped |
| Daemon, PAM and IPC untouched | §1.2, RMC-S4 | Mapped |
| No TCP socket | D2, RMC-S2/S5 | Mapped |
| Owner only, fails closed when empty | D3, D5a, RC-2 | Mapped |
| No remote unlock | D10, RMC-S3 | Mapped |
| ADR + spec | §9 items (1)–(9) | Mapped |
| Feature branch, draft PR | Header | Mapped |
| TDD tests | §7: stale-replay test, steady-state keep-alive test (timestamps strictly advancing, no extra events), backward-clock test, `check_not_root`/`check_host` table tests, end-to-end `421`, SSE slot release, poller-exit test | Mapped |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| PAU17 cites `presence_unlock_contract::test_pau_zbus_is_used_only_by_the_daemon` | `AI/VERIFICATION_MATRIX.md:1895` | cited; Rev. 3 keeps the name and adds an italic annotation | Yes (R2-2 resolved) |
| Presence bus rules that RMC-S9 mirrors | `presence_unlock_contract::test_pau_presence_connects_only_to_the_pinned_system_bus` | RMC-S9 now covers `Connection::system`/`session`, `Builder::system`/`session`, `env::var`, both env bus names, `object_server`, `#[proxy`, `zbus::proxy`, `receive_signal`, `MessageStream` and `CacheProperties::{Yes,Lazily}`, and requires `Builder::address(` | Mostly. `serve_at`, `request_name`, `#[interface`, `SignalStream`, `zbus::blocking` and `Address::system` are not listed (Finding R3-2) |
| `check_lock_csrf` uses the normalized host | §2.5 `check_lock_csrf(head: &RequestHead, normalized_host: &str)`; §2.4 `check_host` returns the normalized host (lowercased, `:443` removed) | consistent | Yes |
| §4 exit codes | `SocketError::{NotADirectory, WrongOwner, NotASocket}` → 78; `Io(_)` → 1; `ConfigError` → 78; poller or accept task ended → 1 | consistent with `RestartPreventExitStatus=78` | Yes (R2-5 resolved) |
| §5 freshness bound | `poll_interval_ms` + `SNAPSHOT_DEADLINE_MS` | consistent with §3 | Yes |
| All round-1 and round-2 code facts (lockfile, features, `test-util`, sync points) | unchanged | — | Yes |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- No change to the trust boundary in Rev. 3. Host, identity and CSRF now run in this order: `check_host` (421), then `authorize` (403), then `check_lock_csrf` against the normalized host (403).
- Failure scenario: `Origin: https://PC.tail1.ts.net:443` with `Host: pc.tail1.ts.net`. Both normalize to the same name, so the request passes as intended. A `http://` origin or any other port still fails. **PASS**.

### Pillar 2 — PAM deadline & concurrency
- Not applicable to PAM. Every logind flow is bounded (`SNAPSHOT_DEADLINE_MS`, `LOCK_FLOW_DEADLINE_MS`). Each stream does one extra fresh read at open; at most `MAX_SSE_STREAMS` = 4 can run at once. **PASS**.

### Pillar 3 — Panic safety & fail-closed
- R2-1 is resolved:
  - The poller publishes every read with a fresh `checked_unix_ms` and a new `seq`, and resets the channel to `None` when idle.
  - Change detection is per stream.
  - Keep-alives re-send the newest reading.
  - The UI measures staleness from when an event arrives on the phone.
  - The new steady-state test fails against the round-2 "publish only on change" implementation.
- R2-4 is resolved: streams filter on the monotonic `seq`, so a backward wall-clock step cannot silence them.
- Failure scenario 1: the stream's own first read happens just after a lock (`locked`, seq n). A poller read that started *before* the lock completes later and receives seq n+1 (`unlocked`). The stream then shows `locked → unlocked → locked` within about one poll interval, because the counter is taken when a read completes, not when it starts. This brief flicker is bounded by `poll_interval_ms + SNAPSHOT_DEADLINE_MS`. It is not the hours-old replay F2 was about. **FINDING (MINOR, R3-1)**.
- Failure scenario 2: the server clock is under paused tokio time in tests. `SystemTime::now` does not pause, so the "strictly greater `checked_unix_ms`" assertion depends on the injected clock in `ServerState` (§7 "clocks"). The tester must inject it, or the assertion may flake at millisecond resolution. This is noted under R3-1 as an implementation note, not a separate finding.

### Pillar 4 — Dependencies
- The zbus migration (§2.12) keeps the exact two-manifest allowlist, the test name and the PAM assertions, and adds a PAU17 annotation. That is a legitimate, narrow contract migration. **PASS**.
- RMC-S9 is close to the presence C2 list. The few omissions concern server-side or blocking APIs, which `object_server`/`#[proxy`/`receive_signal` largely already exclude. **FINDING (MINOR, R3-2)**.

### Pillar 5 — Data confidentiality
- Unchanged from round 2. The UI source link is a plain `<a href>` that loads nothing, and RMC-S8 allows it. **PASS**.

### Pillar 6 — Test integrity
- No existing test is renamed or weakened. The only existing-test edit is the declared §2.12 migration, made in Phase 2 and reviewed as a contract change. The new tests can fail against plausible wrong implementations: change-only publishing, wall-clock filtering, and replay of a stale channel value. **PASS**.

## 4. Findings

### Round-2 findings: resolution check

| Finding | Status |
|---|---|
| R2-1 (MAJOR) §2.6 vs D6 | Resolved (`Reading { seq, view }`, `send_replace` every read, `None` reset, per-stream change detection, keep-alive re-send, steady-state test) |
| R2-2 PAU17 citation | Resolved (name kept, annotation) |
| R2-3 RMC-S9 scope | Resolved in substance (residual R3-2) |
| R2-4 wall-clock filter | Resolved (`seq`; UI staleness from event arrival) |
| R2-5 `SocketError` exit | Resolved (78 persistent / 1 `Io`) |
| R2-6 small gaps | Resolved (§5 deadline, UI source link, `check_host` doc split, normalized-host `Origin` comparison, `check_lock_csrf(head, normalized_host)`) |

### New findings (non-blocking)

1. **R3-1 [MINOR] `seq` is taken when a read completes, which allows a brief out-of-order flicker.**
   - Evidence: D6/§2.6 stamp a reading with the next value of the shared counter when it is published. A poller read that started before a stream's own first read can finish after it and override that newer state for one interval.
   - Required change, to apply during Phase 2/4 (no re-evaluation needed): reserve `seq` from the shared `AtomicU64` **when the read starts**, publish it with the result, and have the stream drop readings whose `seq` is not above the last one it sent. The paused-time tests use the injected clock from `ServerState` for `checked_unix_ms`, never `SystemTime::now`.

2. **R3-2 [MINOR] RMC-S9 omits a few presence C2 needles.**
   - Evidence: the presence contract also forbids `serve_at`, `request_name`, `#[interface`, `SignalStream`, `zbus::blocking` and `Address::system`.
   - Required change, during Phase 2: the tester adds these six needles to RMC-S9 so the companion's bus rules match presence exactly.

## 5. Verdict
VALIDATION_VERDICT: APPROVED

Revision 3 resolves every CRITICAL/MAJOR finding from rounds 1 and 2. The two remaining MINOR findings (R3-1, R3-2) are localized and must be carried into the tester contract (Phase 2) and the auditor constraints (Phase 3). They do not require another plan revision. As specified in D5a, Phase 4 must still verify on the owner's host that `tailscale serve unix:` forwards the original `Host`; if it does not, the design returns to the architect.
