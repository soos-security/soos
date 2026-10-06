# Walkthrough 188 — Failed-Password Alerts in `soos-remote`

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

## Round 3 — Clear Acknowledged Entries (2026-10-06)

- **Date**: 2026-10-06
- **Issue**: GitHub-only follow-up of #339. **Branch**: `feat/remote-auth-alerts` (head `e8e3f28`, change
  uncommitted at hand-off).
- **Matrix criteria**: RMC75 (new); RMC54 evidence extended; RMC59 and RMC74 owner results recorded.

### R3.1 Context & Objectives

Owner request of 2026-10-06, binding, in the owner's words: "once the acknowledge button is clicked, delete the
entries". Agreed interpretation: acknowledged entries disappear from everything the page and the API show, at once
after a successful acknowledgement and after a service restart (replayed attempts at or before the persisted marker
never reappear); only newer attempts are shown. Refused and documented: deleting system journal entries (`journald`
cannot delete a single entry; the root-only vacuum works on whole files and would destroy unrelated logs and the
evidence of the reported intrusion attempts). No password is stored anywhere (O-2 unchanged).

### R3.2 Architect Design

Spec `AI/architect_spec_remote_auth_alerts.md` "Round 3 — clear acknowledged entries" (R3.0–R3.8) and the ADR
amendment "clear acknowledged entries" in `AI/DECISIONS.md`. Decision: acknowledged records are **dropped from
memory** at once, not filtered from views. `AlertBook::acknowledge` raises the high water with every covered record
and then keeps only records with `last_seq > through` (coalescing only joins the back record, so the covered records
are always a prefix of the deque). `AlertBook::record` returns `Recorded::{Kept, Discarded}`; a replayed attempt with
`at_us <= loaded_marker && at_us < started_us` is discarded without creating a record but still consumes a seq, so
`through`, `BeyondNewest` and `409 stale_view` are unchanged. The `acknowledged` field and JSON key are removed
(records serialise 7 keys); the page no longer dims rows. Only `Kept` live attempts reach the push sink.

### R3.3 Plan Evaluation

`AI/plan_evaluator_report.md` round 3: `VALIDATION_VERDICT: APPROVED`, six MINOR findings. F-1 (a private flag plus a
view filter would pass every behavioural test) folded into invariant test 62; F-2 (two further fail-safe residuals),
F-4 (marker "never lower", not "identical") and F-6 (UX sentence) folded into the ADR amendment and §2c; F-3 folded
into the test-54 migration; F-5 (stale `LiveAttemptSink` comment) fixed by the developer. F-4 also binds this phase:
RMC75 is not marked hardware-verified from the owner's report on the round-2 build.

### R3.4 Tester Contract

`AI/tester_contract_alerts.md` "Round 3": tests 58 `test_rmc_alerts_book_acknowledge_removes_records`, 59
`test_rmc_alerts_book_replay_covered_attempts_are_discarded` (`alerts_tests`), 60
`test_rmc_alerts_ack_clears_history_end_to_end` (`alerts_server_tests`), 61 `test_rwp_acknowledge_never_changes_push`
(`push_server_tests`) and 62 `test_rmc_s43_acknowledged_entries_are_not_kept` with
`test_rmc_s43_scanner_self_test` (`soos-invariants::remote_alerts_contract`). Red evidence: compile errors on the new
API (`Recorded`, removed field) and the record key-set assertion. Contract migration (recorded in the tester contract,
citing the owner request): tests 18, 19, 21, 30, 31, 54 and the `counts`/`key`/`RECORD_KEYS` helpers, where an
assertion encoded "acknowledged records stay in the history"; every count, `through`, marker and file-value assertion
kept its value.

### R3.5 Auditor Constraints

`AI/auditor_constraints_alerts.md` C-43 to C-56, among them: push independence (C-49, test 61), the unchanged API
shape apart from the removed key (C-50), the page without any acknowledgement branch (C-51), the journal never touched
and `packaging/soos-remote.service` byte-identical (C-52), documentation of F-2/F-4/F-6 (C-53), traceability honesty
(C-54, applied below), test integrity (C-55) and the gate with five consecutive suite runs (C-56).

### R3.6 Implementation

- `crates/remote/src/alerts.rs`: `Recorded`, field removal, `retain` after the high-water update in `acknowledge`,
  discard in `record`, eviction without the acknowledged branch, views without filters, runtime forwards only `Kept`
  attempts; `LiveAttemptSink` comment corrected.
- `crates/remote/assets/app.js`, `style.css`: dimming branch and `.alerts-history li.acknowledged` rule removed.
- `AI/DECISIONS.md` (amendment), `Docs/REMOTE_COMPANION.md` §2c ("Acknowledge removes the entries" with the
  residuals; "The system journal is never modified: journal entries are never deleted" with the reason).
- `AI/ARCHITECTURE.md` §13 does not describe the alert history; unchanged.

### R3.7 Candid Review

`AI/candid_review_report.md` (2026-10-06, fingerprint `655a58cb…`): **VERDICT: APPROVED**, no CRITICAL or MAJOR
finding; one SUGGESTION (raw byte strings in the test-only scanner of test 62). The review described the failure
direction as a false positive only; that was wrong: a raw byte or raw C string ending in a backslash
(`br"C:\"; let acknowledged = true; let q = "x";`) was scanned as one escaped string running into the next quote, so
the scanner could hide an `acknowledged` binding (a false negative). The brand change fixed the scanner (Contract
Migration CM-1 in `AI/tester_contract_brand.md`, test 75 `test_rmc_s56_scanner_handles_raw_byte_and_c_strings`). This
traceability phase edits documentation only, after that review.

### R3.8 Verification Results

```bash
cargo test --locked -p soos-remote --all-features --test alerts_tests -- <tests 18, 19, 21, 58, 59> --exact   # 5 passed
cargo test --locked -p soos-remote --all-features --test alerts_server_tests -- <tests 30, 31, 54, 60> --exact # 4 passed
cargo test --locked -p soos-remote --all-features --test push_server_tests -- test_rwp_acknowledge_never_changes_push --exact  # 1 passed
cargo test --locked -p soos-invariants --all-features rmc_s43                                                  # 2 passed
cargo test --locked -p soos-remote --all-features            # 5 runs in a row
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --all-features -- -D warnings
cargo test --workspace --locked --all-features
cargo deny check
```

Owner hardware results recorded (owner report 2026-10-06, made on the round-2 build `e8e3f28`): a wrong `sudo`
password produced a notification on the iPhone (RMC59, RMC74); *Acknowledge* resets the banner and it stays reset after
a service restart (RMC59); GDM login was not tested (owner: not important); the test notification and the lock-screen
notification were already recorded in RMC59/RMC74; the page lists the phone under devices (RMC74; host check
`GET /api/push` shows one `apple` device). RMC59 and RMC74 stay partly verified.

### R3.9 Known Limitations / Follow-ups

- RMC75 needs its own owner check after the reinstall: rows disappear on *Acknowledge* and do not return after a
  service restart. Agents never reinstall or restart the service.
- Accepted fail-safe residuals (all show more alerts, never fewer): acknowledged attempts above a marker held down
  by an older unacknowledged attempt or pending check reappear after a restart; entries reappear after a restart when
  `remote-alerts.json` cannot be written; evicted attempts stay counted in the totals when more than 32 records
  arrive between the displayed view and the acknowledgement.
- Journal entries are never deleted; the raw history stays available to root through `journalctl`.
