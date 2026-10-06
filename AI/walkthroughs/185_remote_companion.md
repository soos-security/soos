# Walkthrough 185 — Remote Companion `soos-remote` (Lock Status and Remote Lock over Tailscale Serve)

- **Date**: 2026-10-05 (revision 4, D5a′ effective host: 2026-10-06)
- **Issue**: GitHub #339 (GitHub-only, no backlog id; not registered in `scripts/sync_issue.py`;
  commits carry `Refs #339`, never `Closes #339`) — **Branch**: `feat/remote-companion`, base
  `222665f`. Development branch reviewed through a draft PR; **not merged** until the owner gives
  an explicit go (owner decision 6).
- **Matrix criteria**: RMC1–RMC21 (component `remote-companion`; RMC20 verified 2026-10-06
  (D5a′), RMC21 pending), PAU17 annotation (zbus scope widened)

## 1. Context & Objectives

The owner wants to see from an iPhone whether the desktop session is locked, active or idle, and
to lock it remotely, without a native app, a hosted website or a cloud relay. Six owner decisions
of 2026-10-05 bound the design: a web page installable as a PWA; reachability only through the
owner's Tailscale tailnet (`tailscale serve` terminates HTTPS); a new **user-level** crate and
binary `soos-remote` leaving `soos-daemon`, `pam_soos.so` and the IPC protocol untouched; owner
only; low-risk scope (status and lock; push notifications, live camera and remote unlock are
future issues); development branch with a draft PR and no merge without approval.

Evidence gathered before designing (spec §0): no existing tool covers "logind lock state on a
phone"; the lock state lives in logind `LockedHint`, which GNOME, Plasma and niri set natively and
the sway family only with a `SetLockedHint` wrapper; `Manager.LockSession` only emits the `Lock`
signal, so the lock result is observed through `LockedHint`; the session owner may call
`LockSession` without a polkit rule; `tailscale serve` proxies to a Unix socket (`unix:<path>`),
sets `Tailscale-User-Login` and strips client copies; `httparse` and `http` were already locked
(no `hyper`/`axum`).

## 2. Architect Design

Spec: `AI/architect_spec_remote_companion.md`, revision 3 (revision 1 → F1–F12, revision 2 →
R2-1…R2-6, revision 3 approved with two MINOR findings applied during Phases 2 and 4), amended
by §13 "Revision 4" (D5a′, 2026-10-06; evaluator findings R4-1…R4-9 resolved in revision 5, §13.4)
after the owner's verification of what `tailscale serve unix:` forwards (§9).

- **Crate** `crates/remote` (`soos-remote`, lib + bin, `#![forbid(unsafe_code)]`): modules
  `config`, `identity`, `http`, `routes`, `session`, `logind`, `status`, `socket`, `assets`,
  `server`; embedded assets `index.html`, `app.js`, `style.css`, `manifest.webmanifest`,
  `icon.svg`, 180×180 `apple-touch-icon.png`; current-thread Tokio runtime.
- **Decisions** D1–D12: user-level service refusing root (`check_not_root`, exit 78); one `0600`
  Unix socket in a `0700` directory under `$XDG_RUNTIME_DIR`, no TCP; exactly one allowlisted
  `Tailscale-User-Login` (`403` otherwise, before routing); the **effective host** must be an
  allowed `*.ts.net` name (`421`; D5a′: `X-Forwarded-Host` when present, with `Host` not
  inspected and exactly one `X-Forwarded-Proto: https` required, otherwise exactly one `Host`
  with an optional-but-`https` `X-Forwarded-Proto`; one shared normalisation: OWS trim,
  ASCII lowercase, one `:443` stripped, `is_valid_host_name`, `*.ts.net` or `allowed_hosts`);
  CSRF through `X-Soos-Action: lock` + `Sec-Fetch-Site`/`Origin` compared with the normalised
  effective host; minimal bounded
  HTTP/1.1 over `httparse`; SSE fed by a poller that runs only while a stream is open, with a
  monotonic `seq` and a fresh first read per stream; own-uid local seat `user` session only; a
  logind failure is `unavailable`, never `unlocked`; `LockSession` on a fresh snapshot, one per
  2 s; the crate never names `UnlockSession`/`Unlock`/`SetLockedHint`; strict CSP, no inline
  script; `tracing` to the journal without identity, `Host`, path or header values.
- **Constants** (single source `crates/remote/src/lib.rs`, §3): `MAX_CONNECTIONS` 16,
  `MAX_SSE_STREAMS` 4, `MAX_REQUEST_HEAD_BYTES` 8192, `MAX_HEADERS` 32, `MAX_PATH_LEN` 256,
  `REQUEST_HEAD_TIMEOUT_MS` 5000, `RESPONSE_WRITE_TIMEOUT_MS` 2000, `SSE_KEEPALIVE_MS` 15 000,
  `MAX_SSE_STREAM_MS` 1 800 000, `MIN_LOCK_INTERVAL_MS` 2000, `DBUS_CALL_TIMEOUT_MS` 500,
  `DBUS_CONNECT_TIMEOUT_MS` 1000, `SNAPSHOT_DEADLINE_MS` 1500, `LOCK_FLOW_DEADLINE_MS` 2000,
  `MAX_ALLOWED_LOGINS` 8, `MAX_ALLOWED_HOSTS` 4, `MAX_SOCKET_PATH_LEN` 107, `EXIT_CONFIG` 78,
  `EXIT_RUNTIME` 1, `SYSTEM_BUS_ADDRESS = unix:path=/run/dbus/system_bus_socket`; revision 4
  adds `FORWARDED_HOST_HEADER = "x-forwarded-host"`, `FORWARDED_PROTO_HEADER =
  "x-forwarded-proto"` and `FORWARDED_PROTO_HTTPS = "https"` (the only spellings of these
  names in `src/`).
- **Invariants** RC-1…RC-5 (never root / no network socket / no unlock; identity required and an
  empty allowlist refuses to start; no `unlocked` from a failure or a stale reading; every bound
  explicit; no identity in logs or bodies) and static contracts RMC-S1…RMC-S11, plus RMC-S12
  (both installer scripts executable, `std::fs::metadata` only) and RMC-S7b (the operator page
  names `X-Forwarded-Host`, `X-Forwarded-Proto` and `effective host`, and no longer calls the
  Serve forwarding pending) from revision 4.
- **Contract migration** (§2.12): `test_pau_zbus_is_used_only_by_the_daemon` allows exactly the
  two manifests `crates/daemon` and `crates/remote`, keeps its name (PAU17 citation) and every
  other check.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`: round 1 `REVISION_REQUIRED` (F1 zbus daemon-only contract, F2
stale status replay, F3 root not refused, F4 `test-util`, F5 Host/DNS rebinding, F6 dead streams
holding slots, F7 config-error restart loop, F8 no overall logind deadline, F9 missing doc sync
points, F10 RMC-S3 wording and tie-break, F11 stderr lint, F12 iOS icon/AGPL link); round 2
`REVISION_REQUIRED` (R2-1 §2.6 vs D6 contradiction, R2-2 PAU17 citation, R2-3 RMC-S9 scope,
R2-4 wall-clock filter, R2-5 `SocketError` exit codes, R2-6 small gaps); round 3
**APPROVED** with two MINOR findings carried into Phases 2–4: R3-1 (`seq` reserved when a read
**starts**; paused-time tests use the clock injected through `ServerState`, never
`SystemTime::now`) and R3-2 (RMC-S9 also forbids `serve_at`, `request_name`, `#[interface`,
`SignalStream`, `zbus::blocking`, `Address::system`). The evaluator also required the Phase 4
verification, on the owner's host, that `tailscale serve unix:` forwards the original `Host`
(performed 2026-10-06, see §9: it does not; the effective host now comes from
`X-Forwarded-Host`, D5a′).

## 4. Tester Contract

`AI/tester_contract_remote_companion.md`: **115 new tests** (`test_rmc_*`, `prop_rmc_*`) plus
**one migrated invariant** in the first cycle, and **11 more** in revision 4 (126 in total; no
existing test edited).

| Suite | Tests | Criteria |
|---|---|---|
| `crates/remote/tests/config_tests.rs` | 18 (1 proptest) | RMC1, RMC2 |
| `crates/remote/tests/identity_tests.rs` | 9 + 5 (revision 4: `test_rmc_forwarded_header_constants`, `test_rmc_check_host_effective_host_comes_from_x_forwarded_host`, `test_rmc_check_host_normalises_the_forwarded_host`, `test_rmc_check_host_requires_https_forwarded_proto`, `test_rmc_check_host_allowlist_applies_to_the_forwarded_host`) | RMC3, RMC4 |
| `crates/remote/tests/http_tests.rs` | 16 (4 proptests) | RMC5, RMC6 |
| `crates/remote/tests/routes_tests.rs` | 7 | RMC7, RMC8 |
| `crates/remote/tests/session_tests.rs` | 10 | RMC9 |
| `crates/remote/tests/socket_tests.rs` | 9 | RMC13 |
| `crates/remote/tests/server_tests.rs` | 28 + 4 (revision 4: `test_rmc_serve_head_status_and_lock_end_to_end`, `test_rmc_serve_head_with_http_proto_is_misdirected`, `test_rmc_serve_head_without_proto_or_with_foreign_host_is_misdirected`, `test_rmc_allowed_hosts_apply_to_the_forwarded_host`; end to end over a temp Unix socket, `MockSource`, injected clock, frozen paused time) | RMC3–RMC12, RMC14, RMC20 |
| `tests/invariants/src/remote_companion_contract.rs` | 18 + 2 (revision 4: `test_rmc_s12_installer_scripts_are_executable`, `test_rmc_s7b_documentation_describes_the_effective_host`) | RMC2, RMC7, RMC14–RMC19 |
| `tests/invariants/src/presence_unlock_contract.rs` | 1 migrated | RMC17, PAU17 |

Revision 4 Red evidence (2026-10-06, stub constants only, `check_host` untouched):
`identity_tests` 10 passed / 4 failed, `server_tests` 28 / 4, invariants 467 / 1 (RMC-S7b;
RMC-S12 was green because the `100755` mode change was already staged, its teeth proven in a
scratch `chmod 644` run); the three pre-existing `check_host` tests and every pre-existing
`server_tests` fixture send no `X-Forwarded-*` header and stayed valid. The `serve_head()`
helper reproduces the §9 capture in shape with synthetic names only (`Host: localhost`,
`owner@example.com`, `100.64.0.1`, `pc.tail1234.ts.net`, `https`).

Red evidence (stubs with signatures only): `config_tests` 5 passed / 13 failed, `http_tests`
1 / 15, `identity_tests` 1 / 8, `routes_tests` 1 / 6, `session_tests` 3 / 7, `socket_tests` 1 / 8,
`server_tests` 0 / 28, invariants 7 passed / 11 failed (10 of this contract plus the pre-existing
workspace-tree test). The passing tests were guards (constants, fixed messages, serde shape).
Flakiness check: the 28 time-sensitive server tests ran 10× with identical outcomes in 0.00 s
of wall-clock time each, thanks to a `FrozenClock` that stops tokio's auto-advance so virtual time
moves only through `tokio::time::advance`.

Binding resolutions of spec ambiguities (contract items 1–28) include: streams subscribe before
their first read; `env::var` is allowed only in `main.rs` for `XDG_RUNTIME_DIR`,
`XDG_CONFIG_HOME` and `HOME`; two `Content-Length` values and `Transfer-Encoding` handling;
origin-form targets only; `HeadTooLarge` at the bound even when incomplete; extra RMC-S3
literals; RMC-S9/RMC-S10 positive needles; the logging field-key denylist; the UI constants and
strings; and the Phase 6 documentation needles.

Migrated test: `test_pau_zbus_is_used_only_by_the_daemon` — old assertion "only
`crates/daemon/Cargo.toml` declares `zbus = { workspace = true }`"; new assertion "exactly
`crates/daemon/Cargo.toml` and `crates/remote/Cargo.toml` declare it, name `zbus` once each, and
every other manifest is zbus-free"; PAM assertion unchanged; name kept; mandated by spec §2.12
and ADR 2026-10-05 item (7). The migration is strictly stronger for every manifest it already
covered. `test_business_crates_forbid_unsafe_code` gained `"remote"` (strengthening only).

## 5. Auditor Constraints

`AI/auditor_constraints_remote_companion.md`: **CLEARED**, 36 constraints, plus five
non-blocking test additions T1–T5; revision 4 **CLEARED** again with constraints 37–48 (header
trust argument audited: the `Host`-only path is unchanged or stricter, the `X-Forwarded-Host`
path accepts only a value Serve is observed to overwrite). How the constraints were met
(grouped):

1. **Panic paths (1, 5, 29)**: no `unwrap`/`expect`/panic or print macro in `crates/remote/src`;
   `checked_add` on `seq`, `checked_div` on `IdleSinceHint`, `u64::try_from` on the clock; the
   workspace lints deny the panic family and `-D warnings` catches indexing and arithmetic;
   pinned by `test_rmc_production_code_never_panics_or_prints`.
2. **Unsafe (2)**: forbidden crate-wide; every privileged operation uses safe `std`/`nix` APIs.
3. **Output isolation (3)**: `tracing-subscriber` fmt writer to stderr; `clap::try_parse` so clap
   never exits the process (`--help` is the one accepted stdout path, see §7).
4. **Bounded I/O and deadlines (4–6, 11–17, 28, 29)**: head read under `timeout_at` armed at
   accept, 1024-byte chunks into a buffer capped at `MAX_REQUEST_HEAD_BYTES + 1`; every write
   under `RESPONSE_WRITE_TIMEOUT_MS` through a generic `write_bounded`; lingering close bounded
   by the same timeout and `LINGER_MAX_BYTES` (64 KiB); the whole lock flow under one mutex
   guard inside `LOCK_FLOW_DEADLINE_MS`; every snapshot under `SNAPSHOT_DEADLINE_MS`; RAII
   `SlotGuard`/`SubscriberGuard` for streams; `const _: () = assert!(MAX_SSE_STREAMS <
   MAX_CONNECTIONS)`; every time primitive is `tokio::time`.
5. **Refusal order and fail-closed (7–12, 24)**: parse → `check_host` (421) → `authorize` (403)
   → `route` → `check_lock_csrf` (403) → handler; `SourceError` → `unavailable` or `503`;
   `serde_json` failure → `503`; no path yields `unlocked` or `202` from an error.
6. **Filesystem (18, 19, 21)**: socket directory verified through an `O_DIRECTORY | O_NOFOLLOW`
   descriptor (`fstat` owner, `fchmod 0700`); stale socket unlinked only when `symlink_metadata`
   says socket-owned-by-uid; configuration opened `O_NONBLOCK | O_CLOEXEC` and required to be a
   regular file, read through `take(MAX_CONFIG_BYTES + 1)`.
7. **Secrets and privacy (3, 23, 25, 27)**: `TailscaleLogin` has a redacting `Debug`; logging
   keys are `route` (enum), `status`, `%err` of fixed-text enums, `dbus_error` (truncated name
   only), `socket_path` once at `info!`; the identity-shaped key denylist is a static invariant.
8. **D-Bus trust (25, 26)**: lazily built connection through
   `zbus::connection::Builder::address(SYSTEM_BUS_ADDRESS)` with `method_timeout` and
   `max_queued(16)`, `build()` under `DBUS_CONNECT_TIMEOUT_MS`; every `call_method` under
   `DBUS_CALL_TIMEOUT_MS` with the well-known logind destination and object paths from logind
   replies; `ListSessions` bounded before any per-session call; `GetAll` only on own-uid rows.
9. **Supply chain and tooling (30, 31)**: no new external crate; `cargo deny` clean; the only
   `candid_review.sh` change is `remote` in `BUSINESS_CRATES`.
10. **Assets, tests, docs, host checks, English (32–36)**: assets embedded with
    `include_str!`/`include_bytes!`; no test file edited; the documentation needles were
    delivered; the `tailscale` host check was performed by the owner on 2026-10-06 (constraint
    35 still forbids the agents to run `tailscale`); every string is English.
11. **Revision 4, D5a′ (37–48)**: `check_host` follows the §13.2 evaluation order exactly
    (`X-Forwarded-Host` counted first, `Repeated` before any proto or `Host` lookup; `Host`
    never read in the forwarded branch; no fallback between the two headers) (37); the proto
    rule is one pure helper `forwarded_proto_ok(headers, required)` built on `single_header`,
    `from_utf8`, `trim_matches(OWS)` and `eq_ignore_ascii_case(FORWARDED_PROTO_HTTPS)`, with
    the header names and `"https"` spelled only through the `lib.rs` constants (38); one
    normalisation pipeline `normalize_host` shared by both branches, `is_valid_host_name`,
    `server.rs` and `routes.rs` untouched (39); no new `HostError` variant, `Display` text or
    log field, `x-forwarded-for` never read (40); still pure, allocation-bounded (one
    lowercase copy, three scans over at most 32 headers) and panic-free (41); the module doc
    of `identity.rs` states the trust basis without a stronger claim (42); no test edited
    (43); `Docs/REMOTE_COMPANION.md` rewritten per §13.3 with no `(pending)` left (44); the
    Shortcuts section carries the trailing-dot fix, `misdirected_request`, the hedged
    *Request Body: File* wording and no iOS claim stated as fact (45); every quotation of the
    owner's capture is redacted (`arch.<tailnet>.ts.net`, `<redacted>`, `100.x.y.z`) (46); the
    traceability edits are exactly §13.3 and constraint 35 is amended, not revoked (47);
    `scripts/install_remote.sh` changes git mode only, content byte-identical to `HEAD` (48).

T1–T5 (slow-reader write timeout on a stream, two concurrent lock requests, `HEAD /api/events`
opens no slot, edge table rows, static `O_DIRECTORY`/`O_NOFOLLOW` needle) were not added in this
cycle; the production code satisfies each by reading (candid review F4, §9).

## 6. Implementation

Files added: `crates/remote/{Cargo.toml, src/*.rs, assets/*, tests/*.rs}`,
`packaging/soos-remote.service`, `scripts/install_remote.sh`, `Docs/REMOTE_COMPANION.md`,
`tests/invariants/src/remote_companion_contract.rs`, this walkthrough. Files changed:
`Cargo.toml` (member, `httparse = "1.10"`, `soos-remote` path dependency), `Cargo.lock` (one new
`[[package]]`, no new external crate), `tests/invariants/src/lib.rs`,
`tests/invariants/src/presence_unlock_contract.rs` (migration), `scripts/candid_review.sh`,
`AGENTS.md`, `AI/ARCHITECTURE.md` (line 22 zbus scope, §8 tree and forbid list, new §13),
`AI/DECISIONS.md` (ADR), `AI/MOCK_STRATEGY.md`, `AI/VERIFICATION_MATRIX.md` (PAU17 annotation,
component `remote-companion`), `Docs/README.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`,
`.claude/skills/dev-workflow/references/project-facts.md`.

Notable decisions:

- **`seq` reserved when a read starts** (R3-1): the poller and each stream's first read take the
  next value of the shared `AtomicU64` before awaiting `own_sessions`; a stream drops any reading
  whose `seq` is not above the last one it sent; `checked_unix_ms` comes from the clock injected
  through `ServerState::with_unix_clock` (production: `SystemTime`).
- **Channel reset**: the poller publishes every read with `send_replace(Some(Reading))` and
  resets to `None` when the subscriber count returns to 0, so a new stream can never see a
  stale value; change detection and the 15 s keep-alive re-send belong to each stream.
- **Session tie-break**: `select_session` prefers active sessions, then the shortest id, then
  the **greatest** same-length byte sequence (`"c9"` before `"10"`), as the tester contract
  item 16 and `test_rmc_select_session_prefers_active_local_user_seat_session` require. This
  departs from the spec D7 wording ("smallest") and auditor constraint 24; the developer kept
  the test (zero test weakening) and documented the choice; the candid review recorded it as
  F1 and the ADR now records the built behaviour (§9 follow-up: reconcile the spec).
- **`main` sequence**: tracing → `check_not_root(getuid, geteuid)` → `clap::try_parse` →
  `default_config_path` → `load_config` → `prepare_socket_dir` → `bind_listener` → `serve`
  under SIGTERM/SIGINT; `ExitCode` 78 for `ConfigError` and persistent `SocketError`
  variants, 1 for `Io(_)` and runtime failures; `--help` is printed by clap on stdout.
- **Installer**: `install_remote.sh` builds `--release --locked`, installs the binary under
  `~/.local/bin` and the user unit under the systemd user directory, writes the `0600`
  configuration template only when absent (`allowed_logins = []`, so the service refuses to
  start until filled), runs `systemctl --user daemon-reload` and only prints the enable and
  `tailscale serve` steps.
- **UI**: `app.js` keeps `STALE_UI_MS` 45 000 (staleness measured from event arrival on the
  phone) and `LOCK_CONFIRM_UI_MS` 5000, reconnects on `visibilitychange`/`pageshow`, uses
  `textContent` only, and links the source repository (AGPL-3.0 §13).

## 7. Candid Review

First review (commit `0b0b50c`), Reviewed-Diff-Fingerprint
`793d899a1c43e496a6cce8ae7da28b6e07631667c08434aead9c6ae9a28f51f4` (the code and the
pre-Phase-6 documents; the first Phase 6 matrix rows came after it, as that report noted).
**VERDICT: APPROVED**, no CRITICAL or MAJOR finding. Findings and their resolution:

| Finding | Resolution |
|---|---|
| F1 (MINOR) `select_session` tie-break is "greatest bytes", contradicting spec D7 and auditor constraint 24 | Not a code or test change in this cycle. The built behaviour is recorded in the ADR item (4) and ARCHITECTURE §13; the spec/auditor reconciliation is a follow-up (§9). |
| F2 (MINOR) `is_valid_host_name` has no 63-byte label bound | Follow-up for the next developer cycle (new fingerprint); harmless today (total ≤ 253, exact membership, `.ts.net` suffix). |
| F3 (MINOR) `Docs/REMOTE_COMPANION.md` said `loginctl lock-session`-style hints set `LockedHint` | **Fixed in Phase 6**: the sentence now states that `loginctl lock-session` only emits `Lock`, gives the `busctl … SetLockedHint b true` call and points to the `swaylock-presence` wrapper of `Docs/DISTRIBUTION_DEPLOYMENT.md` §5.5. |
| F4 (MINOR) auditor additions T1–T5 absent | Follow-up for the tester (§9). |
| Suggestion: `err.print()` lets clap write `--help` to stdout | Follow-up (§9); accepted for an operator-run `--help`. |
| Suggestion: "the code is right" wording in the doc header | **Fixed in Phase 6**: replaced by "report the drift" wording citing the invariant and the matrix rows. |

Second review (revision 4/5 working tree on top of `0b0b50c`, 2026-10-06), current
`AI/candid_review_report.md`, Reviewed-Diff-Fingerprint
`112e25dde568f7b1ceb8815860b80d73782d5108af764cee801960189d1c8500` (49 files; this walkthrough's
revision-4 sections and the `AI/ARCHITECTURE.md` §13 wording came after it). **VERDICT:
APPROVED**, no CRITICAL or MAJOR finding; the single changed assertion in the whole diff is the
recorded `test_pau_zbus_is_used_only_by_the_daemon` migration. Findings and their resolution:

| Finding | Resolution |
|---|---|
| MINOR `scripts/install_remote.sh:72` — the configuration template comment still says "DNS names accepted in the Host header" | **Follow-up** (§9): auditor constraint 48 pins the installer to a mode-only change in this cycle (content byte-identical to `HEAD`), so the comment is reworded to "accepted as the effective host (`X-Forwarded-Host` set by `tailscale serve`, or `Host` for a direct local client)" in the next cycle that touches the script. `Docs/REMOTE_COMPANION.md` §5 already carries the correct wording. |
| SUGGESTION `test_pau_zbus_is_used_only_by_the_daemon` now allows two crates | Name kept on purpose (PAU17 citation, spec §2.12); a rename with a matrix update is a follow-up (§9). |
| SUGGESTION `server.rs::handle_connection` answers a head parse error as for `GET` even to a `HEAD` request | Harmless (`Connection: close`); documenting the choice in the doc comment is a follow-up (§9). |

## 8. Verification Results

Commands run on 2026-10-06 on the final revision-4 working tree (no commit; the orchestrator
owns the commit, the draft PR and CI). The 2026-10-05 results of the first cycle were 97
`soos-remote` tests and 466 invariants, all green.

- `cargo fmt --all -- --check`: clean (candid review; no Rust file changed in Phase 6).
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean (candid
  review).
- `cargo deny --locked check`: `advisories ok, bans ok, licenses ok, sources ok` (candid review
  and auditor; `Cargo.lock` unchanged in revision 4).
- `cargo test --locked -p soos-remote --all-features`: **106 passed**, 0 failed
  (`config_tests` 18, `http_tests` 16, `identity_tests` 14, `routes_tests` 7, `session_tests` 10,
  `socket_tests` 9, `server_tests` 32; the lib and bin unit test targets are empty).
- `cargo test --locked -p soos-invariants --all-features`: **468 passed**, 0 failed, re-run
  after the Phase 6 edits (matrix citations of the 21 RMC rows, including the eight revision-4
  test names, resolve under `test_matrix_claimed_rows_cite_only_existing_evidence`; RMC-S12 and
  RMC-S7b green; documentation needles, workspace trees,
  `test_pau_zbus_is_used_only_by_the_daemon`, `test_business_crates_forbid_unsafe_code`).
- `./scripts/candid_review.sh`: `Candid Review PASSED` (layer 1: forbid-unsafe list including
  `remote`, English policy, invariants) on the final tree.
- `python3 scripts/sync_issue.py --check`: offline mapping self-check only (GitHub-only issue, no
  backlog tick, no GitHub call).

## 9. Known Limitations / Follow-ups

- **RMC20 verified by the owner on 2026-10-06 (D5a′, spec §13)**: with Tailscale 1.102.4 and
  `tailscale serve --bg unix:/run/user/<uid>/soos-remote/remote.sock` (tailnet only), a capture
  listener on the socket received, for `curl https://arch.<tailnet>.ts.net/api/status`:
  `Host: localhost`, `Tailscale-User-Login: <redacted>`, `X-Forwarded-For: 100.x.y.z`,
  `X-Forwarded-Host: arch.<tailnet>.ts.net`, `X-Forwarded-Proto: https`. A second request with
  forged `X-Forwarded-Host`, `X-Forwarded-Proto: http` and `Tailscale-User-Login` reached the
  socket with the real values: Serve overwrites all three. The deployed revision-3 build
  answered `421` end to end (it pinned `Host`). Resolution D5a′: the effective host is decided
  by `X-Forwarded-Host` when present (`Host` not inspected), otherwise by `Host`; a request
  carrying `X-Forwarded-Host` must carry exactly one `X-Forwarded-Proto: https`; the same
  normalisation applies; `X-Forwarded-For` is ignored. Implemented in
  `crates/remote/src/identity.rs::check_host` (`forwarded_proto_ok`, `normalize_host`),
  pinned by four new `identity_tests` and four new `server_tests` (the captured head
  reproduced with synthetic names), RMC-S12 (installer mode `100755`) and RMC-S7b
  (documentation needles). The agents still never run `tailscale`, `sudo` or `systemctl`
  (constraint 35, amended by constraint 47).
- **RMC21 pending hardware check**: Safari "Add to Home Screen", lock/unlock follow-up, "Lock
  now" confirmation through `LockedHint`, reconnection after background, `Unreachable` off the
  tailnet; plus the "Lock PC" shortcut (run once unlocked → `lock_requested` and the desktop
  locks; again within 2 s → `rate_limited`), after which the hedged iOS wording of the
  Shortcuts section may be removed (spec §13.3, R4-7).
- **Installer template comment (candid review of revision 4, MINOR)**: `scripts/install_remote.sh`
  line 72 still describes `allowed_hosts` as "accepted in the Host header"; reword to the
  effective-host wording in the next cycle that may change the script's content (constraint 48
  limited this cycle to the `100755` mode change). The spec §13.3 installer row now records
  that `tests/invariants/src/lib.rs` already checked `scripts/install.sh` (auditor A7); every
  "verified with Tailscale 1.102.4" statement keeps the version and the date (auditor A8).
- **Candid F1**: reconcile spec D7 and auditor constraint 24 with the built tie-break
  (shortest id, then greatest same-length id) in one place; the test must not be edited
  silently.
- **Candid F2**: add the 63-byte label bound to `is_valid_host_name` with table rows, in a new
  developer cycle with a fresh review.
- **Candid F4 / auditor T1–T5**: add the slow-reader stream write timeout, concurrent lock
  requests, `HEAD /api/events`, the edge rows and the static `O_DIRECTORY`/`O_NOFOLLOW` needle.
- **`--help` on stdout** through `clap`'s `err.print()`; acceptable for an operator, could move
  to tracing.
- **Status truthfulness** depends on the desktop setting `LockedHint` and honouring the logind
  `Lock` signal (sway family: `SetLockedHint` wrapper and `swayidle lock` hook).
- **Suspended iOS web apps** keep a stream slot until `tailscaled` closes the connection or the
  30 min lifetime elapses; four slots serve the owner's own reconnects.
- **System-wide packaging** (`install.sh`, deb/rpm/Arch) is deferred to a follow-up issue after
  the owner approves the merge; `scripts/install_remote.sh` is the only installer.
- **Out of scope** (each needs its own ADR): remote unlock, push notifications, live camera.
