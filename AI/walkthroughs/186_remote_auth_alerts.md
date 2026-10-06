# Walkthrough 186 — Failed-Password Alerts in `soos-remote`

- **Date**: 2026-10-06
- **Issue**: GitHub-only follow-up of #339. **Branch**: `feat/remote-auth-alerts`, from
  `feat/remote-companion` at `ea862cc` (draft PR #340 belongs to `feat/remote-companion`).
  Nothing is committed, pushed, deployed or restarted by the agents (owner decision O-5).
- **ADR**: "[2026-10-06] Failed-Password Alerts in `soos-remote` From the System Journal".
- **Spec**: `AI/architect_spec_remote_auth_alerts.md` (round 2); research `AI/research_alerts.md`;
  tester contract `AI/tester_contract_alerts.md`; auditor constraints
  `AI/auditor_constraints_alerts.md` (C-1 to C-42).
- **Matrix criteria**: RMC45–RMC59 (RMC59 is the owner's hardware check).

## 1. Context & Objectives

The owner wants to be told on the phone, in the existing `soos-remote` page, when someone types a
wrong password on the PC: at the lock screen (`swaylock-plugin` under driftwm), with `sudo`, at
GDM, or through any other local PAM password check (O-1). The relayed request asked to "see the
passwords that were tested"; owner decision O-2 of the same day forbids capturing, storing,
logging, displaying or sending the typed password or any part, length or hash of it, and
`AGENTS.md` forbids credentials anywhere. The feature therefore shows only **when**, **where**
(lock screen, sudo, login, other), **which account class** (your account, root, another
account), **what happened** (wrong password, or an attempt while `pam_faillock` locked the
account) and **how many times**. Showing the typed text would be a different feature that needs
an amendment of `AGENTS.md` and its own ADR; this one has no field, route or buffer designed to
carry it.

## 2. Architect Design

- **Source** (O-3): the system journal, read by the owner without root. A `journalctl` child
  (`tokio::process`, feature `process` in `crates/remote` only) with a fixed argument vector,
  `--output=json`, a fixed `--output-fields` list and the match `SYSLOG_FACILITY=10`; environment
  cleared, no shell, stdin and stderr closed, `kill_on_drop`. No C binding; the unit keeps
  `RestrictAddressFamilies=AF_UNIX` and is byte-for-byte unchanged.
- **Trust**: only journald-set fields decide (`_TRANSPORT=syslog`, `_UID` 0 or the owner, `_EXE`
  of `sudo`/`su` or of a configured lock-screen program, `_COMM` of `unix_chkpwd`);
  `SYSLOG_IDENTIFIER` and `MESSAGE` are prefix-anchored grammar inputs only.
- **Counting**: one attempt per `unix_chkpwd` check, merged with its `pam_unix` failure within
  2 s; helper-only checks inherit the class of the last trusted failure of the same side and
  account (1 h); owner-side checks without a trusted anchor (test binaries) are dropped.
- **History**: 32 records, coalescing within 60 s, saturating counts, rebuilt from the last 24 h
  of the journal at each start; only an acknowledgement marker is persisted
  (`remote-alerts.json`, `0600`). A 64-bit per-start epoch names the view, so an acknowledgement
  can never cover an attempt the page did not display.
- **Exposure** (O-4): `GET /api/alerts`, `POST /api/alerts/ack` (no body, CSRF header
  `alerts-ack`, epoch and through headers, one per second) and `event: alerts` on the existing
  stream; authenticated callers only.

## 3. Tester Contract (Phase 2)

New suites `journal_tests.rs` (tests 1–12, 51), `alerts_tests.rs` (13–22, 52),
`alerts_server_tests.rs` (23–36, 53–55), `journal_process_tests.rs` (42, 43), appended
`mod alerts_contract` blocks in `config_tests.rs`, `routes_tests.rs` and `http_tests.rs`, the
fixture `tests/common/journal.rs` (`ScriptedJournal`), and the static invariants
`tests/invariants/src/remote_alerts_contract.rs` (RMC-S24–RMC-S31). Existing tests gained only
the setup line `alerts: AlertsConfig::default(),` in two `RemoteConfig` literals.

## 4. Auditor Constraints (Phase 3)

Cleared with 42 constraints. The ones that shaped the code most: C-7/C-12 (a fixed-capacity,
zeroized, cancel-safe line reader without `BufReader`), C-13 (hand-written visitor: duplicate read
keys refused, unknown keys skipped with `IgnoredAny`, bounds enforced while visiting), C-20 (the
ack file needs mode exactly `0600`, unlike the credential store's `& 0o077` rule), C-23 (the
book mutex is released before any file write; marker writes are serialised by their own mutex),
C-26 (the follower never ends `serve`), C-40 (documentation corrections G-3, G-4, G-5, G-8).

## 5. Developer Implementation (Phase 4)

- `crates/remote/src/lib.rs`: the constants of spec §3.1 and their compile-time relations;
  `pub mod journal; pub mod alerts;`.
- `crates/remote/src/journal.rs` (new): `probe_args`, `follow_args`, `JournalCursor`,
  `parse_entry` (bounded visitor), `classify_entry` (gate, three grammars, trust table,
  `exe_for_comparison`), `OwnerLogin`, the `JournalSource`/`JournalLines` seam, the production
  `JournalctlSource` (one spawn helper; the probe reads one line within 2 s and requires `_UID`
  `"0"`), and `BoundedLineReader` (one buffer of 28 KiB allocated once, consumed bytes wiped,
  overlong lines reported once at their newline).
- `crates/remote/src/alerts.rs` (new): `Correlator`, `AlertsEpoch`, `AlertBook` (rules R, A, M),
  `AlertsView`, `read_ack_file`/`write_ack_file` (atomic write through
  `credentials::write_atomic`), the shared runtime (state, coverage, version channel, ack gate,
  marker flush at most once per second, at once after an acknowledgement) and `run_follower`
  (probe → follow → catch-up → backoff 1 s → 60 s, reset after a 60 s run; cursor resume; the
  cursor is dropped when a cursor start ends within 1 s without a line).
- `crates/remote/src/config.rs`: `password_alerts`, `lock_screen_programs`, `AlertsConfig`,
  `DEFAULT_LOCK_SCREEN_PROGRAMS`, two new errors (the path is never echoed),
  `resolve_alerts_ack_path`.
- `crates/remote/src/routes.rs`: `Route::Alerts`, `Route::AlertsAck`, `check_alerts_ack_csrf`,
  `parse_alerts_ack_headers`, `AckTarget`, `AckHeaderError`; `is_funnel_public` and
  `accepts_body` unchanged.
- `crates/remote/src/http.rs`: `encode_sse_alerts_event`.
- `crates/remote/src/server.rs`: `ServerState::with_password_alerts` (ignored while disabled);
  `serve` sets the runtime up (epoch, marker, book) and supervises the follower; the two routes in
  the spec's gate order; `serve_stream` sends the first `alerts` event right after the first
  status event, then at most one per second, re-validating a Funnel session before each one.
- `crates/remote/src/audit.rs`: three fixed-text events.
- `crates/remote/src/main.rs`: resolves the owner's login from the passwd entry of the process
  uid and wires `JournalctlSource` only when `password_alerts = true`.
- `crates/remote/assets/`: `#alerts` section, banner, coverage line, history (10 rows),
  *Acknowledge* with the snapshot headers, `stale_view` re-fetch; `textContent` only.
- Documentation: `Docs/REMOTE_COMPANION.md` §1, §2c (new), §5, §6, §8; ADR and spec wording
  corrected for G-3, G-4, G-5 and G-8; matrix rows RMC45–RMC59.

## 6. Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --all-features -- -D warnings
cargo test --workspace --locked --all-features
cargo deny check
```

The `soos-remote` suite was run repeatedly in a row (flake check, branch lesson). RMC59 is the
owner's hardware check: with `password_alerts = true`,
`lock_screen_programs = ["/home/<owner>/.local/bin/swaylock-plugin"]` and the owner in `wheel`,
wrong passwords at the lock screen, with `sudo` and at GDM must appear on the phone within 5 s;
agents never restart the service.

## 7. Residual Risks

See ADR "Accepted risks" and `Docs/REMOTE_COMPANION.md` §8: false alerts (and suppression of
helper-only lock-screen checks) by a process running as the owner; class relabelling of
helper-only checks; the broad journal read right needed by the service; volatile journal storage;
oversized fields of other accounts; unwiped kernel pipe and `serde_json` scratch buffers; the
catch-up heuristic and cold start; the unit sandbox verified only on hardware.
