# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `feat/remote-companion` (GitHub #339, draft PR; not merged until the owner says so)
- **Base (merge-base)**: `222665f`
- **Reviewed-Diff-Fingerprint**: `13b1b076f8112f70811a587c58b24d4ed00703d97da414ab7cc6cf7efab3089b`
- **Audited Files** (49, from `target/candid_diff.patch`, 12 153 lines; commit `0b0b50c` plus the
  revision-4/5 working tree):
  - Production: `crates/remote/Cargo.toml`, `crates/remote/src/{lib,main,config,identity,http,routes,session,logind,status,server,socket,assets}.rs`, `crates/remote/assets/{index.html,app.js,style.css,manifest.webmanifest,icon.svg,apple-touch-icon.png}`
  - Tests: `crates/remote/tests/{config,http,identity,routes,server,session,socket}_tests.rs`, `tests/invariants/src/lib.rs`, `tests/invariants/src/presence_unlock_contract.rs`, `tests/invariants/src/remote_companion_contract.rs`
  - Build / automation: `Cargo.toml`, `Cargo.lock`, `packaging/soos-remote.service`, `scripts/install_remote.sh` (mode `100644 → 100755`, content unchanged), `scripts/candid_review.sh`
  - Documentation: `AGENTS.md`, `.agents/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/MOCK_STRATEGY.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_remote_companion.md` (revision 5, §13), `AI/auditor_constraints_remote_companion.md`, `AI/tester_contract_remote_companion.md`, `AI/walkthroughs/183_remote_companion.md`, `Docs/README.md`, `Docs/REMOTE_COMPANION.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`

## 1. Executive Summary

The diff adds the leaf crate `soos-remote`: a user-level, `#![forbid(unsafe_code)]`,
current-thread Tokio service that serves the owner's phone a lock-status page (Server-Sent Events
fed by bounded logind reads) and a remote `Manager.LockSession` over a `0600` Unix socket behind
`tailscale serve`, plus its user unit, per-user installer, static contracts and documentation.
The working tree on top of `0b0b50c` implements spec §13 (D5a′): the effective host is taken from
`X-Forwarded-Host` when present (`Host` not inspected), a proxied request must carry exactly one
`X-Forwarded-Proto: https`, and the owner's verification of the Serve head (Tailscale 1.102.4) is
recorded in redacted form only.

This review was done cold, from the frozen patch and the code it lands in. I read every
production file in full, the new D5a′ unit and end-to-end tests, the static contracts, the
migrated presence contract, the unit, the installer, the web assets and the documentation
deltas, and ran the CI commands on the reviewed fingerprint: `cargo fmt --all --check` (clean),
`cargo clippy -p soos-remote --all-targets -- -D warnings` (clean), `cargo test -p soos-remote`
(106 passed), `cargo test -p soos-invariants` (468 passed, matrix citations included) and
`cargo deny --locked check bans licenses sources` (ok). No CRITICAL or MAJOR defect was found.
Two MINOR findings (an installer template comment and a tie-break wording drift between the
phase documents) and three SUGGESTIONS are listed in §4; none blocks the merge.

## 2. Test Changes (mechanical listing from step 3)

Commands run on the frozen patch (`P=target/candid_diff.patch`):

- Test files touched: `crates/remote/tests/{config,http,identity,routes,server,session,socket}_tests.rs`
  (all new), `tests/invariants/src/lib.rs`, `tests/invariants/src/presence_unlock_contract.rs`,
  `tests/invariants/src/remote_companion_contract.rs` (new).
- Removed/changed checks (`^-[^-].*(assert|#\[test\]|#\[tokio::test|proptest!|#\[should_panic)`):
  **one** hit, patch line 10752, the `assert!(daemon manifest declares zbus)` of
  `tests/invariants/src/presence_unlock_contract.rs::test_pau_zbus_is_used_only_by_the_daemon`.
  All six removed lines in test files belong to that one hunk (listed with `awk` over the patch).
- New escape hatches (`#[ignore`, `#[cfg(any())]`, `should_panic`, `tolerance`, `epsilon`): **none**.
- Inline `mod tests` added or removed: **none** (no in-source `#[cfg(test)]` module in `crates/remote/src`).

Justification per change:

| Change | Justification | Verdict |
|---|---|---|
| `presence_unlock_contract.rs::test_pau_zbus_is_used_only_by_the_daemon`: the single daemon assertion becomes a loop over `ALLOWED = ["crates/daemon/Cargo.toml", "crates/remote/Cargo.toml"]` asserting the exact `zbus = { workspace = true }` line **and** exactly one `zbus` mention per manifest, plus a new `allowed_seen == 2` assertion; the "every other manifest is zbus-free" loop and the PAM `no zbus / no dbus` assertion are unchanged; the test name is kept (matrix row PAU17). | Contract migration recorded in the test's doc comment and in spec §2.12: ADR 2026-10-05 "Remote Companion `soos-remote`" item (7) widens "zbus is daemon-only" to exactly two crates, an owner-approved scope change. The replacement is strictly stronger (adds exactly-once and both-manifests-exist). | Justified, not a weakening |
| `tests/invariants/src/lib.rs`: `"remote"` appended to `business_crates` in `test_business_crates_forbid_unsafe_code`; new `mod remote_companion_contract`. | Spec §1.1; mirrored by `scripts/candid_review.sh` `BUSINESS_CRATES` and enforced by `test_rmc_forbid_unsafe_list_and_review_tooling_include_remote`. | Strengthening |
| Revision 4/5 tests are **new functions only**: `identity_tests.rs` gains `test_rmc_forwarded_header_constants`, `test_rmc_check_host_effective_host_comes_from_x_forwarded_host`, `test_rmc_check_host_normalises_the_forwarded_host`, `test_rmc_check_host_requires_https_forwarded_proto`, `test_rmc_check_host_allowlist_applies_to_the_forwarded_host`; `server_tests.rs` gains `serve_head()` / `serve_head_without()` and `test_rmc_serve_head_status_and_lock_end_to_end`, `test_rmc_serve_head_with_http_proto_is_misdirected`, `test_rmc_serve_head_without_proto_or_with_foreign_host_is_misdirected`, `test_rmc_allowed_hosts_apply_to_the_forwarded_host`; `remote_companion_contract.rs` gains RMC-S12 and RMC-S7b. The three pre-existing `check_host` tests, `with_identity` and every pre-existing server fixture are untouched (they send no `X-Forwarded-*` header and stay valid). | Spec §13.3 rows "Unit tests", "End-to-end tests", "Static contracts". | New coverage |

## 3. Deep Reasoning Audit

### Logic & Architecture

Scenarios attempted:

- **D5a′ evaluation order** (`identity.rs::check_host`): `X-Forwarded-Host` is counted first over
  the whole header list (`single_header`), so "XFH twice + `X-Forwarded-Proto: http`" is
  `Repeated`, not `NotAllowed`; with one XFH `forwarded_proto_ok(headers, true)` runs and `Host`
  is never read (the `"host"` lookup is only in the `Err(Missing)` arm); without XFH, `Host`
  must occur exactly once, then `forwarded_proto_ok(headers, false)`. Matches §13.2 step by step.
  `Missing` is returned only when neither header exists (`X-Forwarded-For` never stands in).
- **Transport rule**: a proto header is accepted only as exactly one occurrence whose OWS-trimmed
  value equals `https` ASCII-case-insensitively. Tried `https, https`, `https:`, `httpsx`, an
  empty value, `\xff`, two identical `https` lines: all `NotAllowed`; all are in the tests.
- **Normalisation**: one shared `normalize_host` for both branches: OWS trim → ASCII lowercase →
  one `strip_suffix(":443")` → `is_valid_host_name` → `*.ts.net` with a non-empty prefix, or
  `allowed_hosts` membership. Tried `pc.tail1234.ts.net:443:443` (one strip leaves a colon →
  charset fails), `ts.net` (empty prefix), `100.64.0.1` (all-digit last label), `a.ts.net, b.ts.net`,
  a trailing dot, 254 bytes: all `NotAllowed`; a `lan-box.example` entry in `allowed_hosts` passes
  (the allowlist is exact membership, not `*.ts.net`-restricted, as the spec says).
- **Fail-closed if Serve changes**: a Serve release that stops sending `X-Forwarded-Host` leaves
  `Host: localhost` → `NotAllowed` → `421`, never `200` (stated in `Docs/REMOTE_COMPANION.md` §8).
  Funnel traffic passes the host check but carries no `Tailscale-User-Login` → `403`.
- **Dispatch order** (`server.rs::handle_connection`): HTTP error → `check_host` (`421`) →
  `authorize` (`403`) → `route` → `check_lock_csrf` (`403`) → handler; no logind call, SSE slot or
  subscriber registration before every check passed (proven by the `reads() == 0` assertions).
- **Lock CSRF** (`routes.rs::check_lock_csrf`): exactly one `X-Soos-Action: lock` (byte-exact);
  `Sec-Fetch-Site`, if present, exactly `same-origin` (so `none`, `same-site`, `cross-site` are
  refused); `Origin`, if present, lowercased and equal to `https://<effective host>` or `…:443`.
  A cross-site `fetch` with the custom header triggers a CORS preflight; `OPTIONS /api/lock` is
  `Method::Other` → `405` without any `Access-Control-Allow-*` header, so the browser blocks it.
  With the captured Serve head, `Origin: https://localhost` → `403` and
  `Origin: https://PC.Tail1234.TS.NET:443` → `202` (end-to-end test).
- **Lock flow** (`lock_flow`): the `Mutex<Option<Instant>>` gate serialises the flow; rate gate
  (monotonic `Instant`, `saturating_duration_since`) → fresh snapshot → `select_session` →
  `409 no_session` / `409 already_locked` → `LockSession`; the interval is recorded only when
  `lock_session` is called (a failed call still counts, a `409`/`503` snapshot does not); the
  whole flow is under `LOCK_FLOW_DEADLINE_MS` and the guard is released when the timeout drops
  the future. Verified by `test_rmc_lock_flow_and_rate_limit` and the hung-logind test.
- **Stream protocol** (`serve_stream` / `poller`): the stream reserves its own `seq`, makes its
  own fresh read after accept, then drops channel readings with `seq <= last_seq` (no stale
  replay even if the poller skipped its `None` reset because subscribers went 1 → 0 → 1 inside
  one sleep); a view change or the keep-alive rule (`now + poll_interval >= last_sent_at +
  SSE_KEEPALIVE_MS`) triggers a send; the `keepalive_pending` / `fallback_at` pair covers a
  slow poller read (bounded by `poll_interval + SNAPSHOT_DEADLINE_MS`, exactly the grace); the
  stream ends on read-half EOF or error, write failure, `MAX_SSE_STREAM_MS`, shutdown
  (`closing` watch, `biased` first) or counter overflow. The `Notify` + `AtomicUsize` wake-up
  is race-free: `notify_one` stores a permit when the poller is not waiting, and the
  `while subscribers == 0` loop absorbs a stale permit.
- **Session selection** (`session.rs::select_session`): candidates need `uid`, `Class == "user"`
  (exact), explicit `Remote == Some(false)` and a seat; `min_by_key((!active, id.len(),
  Reverse(id.as_bytes())))` is total and deterministic for any input order. The same-length
  tie-break takes the **greatest** byte sequence, which is what the binding tester contract item
  16 and `AI/ARCHITECTURE.md` §13 state, but not what spec D7 / auditor constraint 24 wrote
  (see §4, MINOR 2). No security consequence: every candidate is an owner session on a local
  seat, active sessions are preferred, and the lock result is confirmed through `LockedHint`.
- **Single source of constants**: all bounds and the three forwarded-header constants live in
  `lib.rs`; `grep -ri x-forwarded crates/remote/src` outside comments hits only the two
  `lib.rs` definitions; the matrix and project-facts rows list the same values.
- **Scope**: everything in the diff is #339; no PAM, daemon, protocol or policy code is touched;
  `soos-remote` is a leaf (RMC-S4 passes). The commit message carries `Refs #339`, not `Closes`.

→ **PASS**

### PAM Concurrency & Deadlines

`crates/pam` is not in the diff; the PAM manifest assertion (`!pam.contains("zbus") &&
!pam.contains("dbus")`) is unchanged and passes. For the new service every blocking point is a
`tokio::time` bound: head read `REQUEST_HEAD_TIMEOUT_MS` (5 s, connection closed without a byte),
every write `RESPONSE_WRITE_TIMEOUT_MS`, lingering close bounded by the same timeout and
`LINGER_MAX_BYTES`, snapshot `SNAPSHOT_DEADLINE_MS`, lock flow `LOCK_FLOW_DEADLINE_MS` (the wait
on the lock gate included), D-Bus connect/call `DBUS_CONNECT_TIMEOUT_MS` / `DBUS_CALL_TIMEOUT_MS`
(plus `method_timeout` on the builder), stream life `MAX_SSE_STREAM_MS`. A hung logind cannot
stall `login`/`sudo`/`gdm`: the companion is a separate user process that only reads logind,
and `test_rmc_deadlines_bound_hung_logind_calls` shows the cut at exactly the deadline with no
retry. `deadline()` falls back to "fire at once" on an unrepresentable `Instant` (fail closed).

→ **PASS**

### Panic Safety & Fail-Closed

- `grep -rnE '\.unwrap\(|\.expect\(|panic!|unreachable!|todo!|unimplemented!' crates/remote/src`
  → empty; the only `unwrap_or*` forms are total. No `[]` indexing in production; `get`,
  `checked_add`, `checked_div`, `saturating_*` throughout (`reserve_seq`, `deadline`, `trim_ows`,
  `truncated_error_name`, `TestClock` is test-only). `const _: () = assert!(MAX_SSE_STREAMS <
  MAX_CONNECTIONS)` is compile-time. `test_rmc_production_code_never_panics_or_prints` enforces
  the same with comments stripped.
- Fail-closed paths: `status_from(Err(_))` → `Unavailable` (never `Unlocked`); `Unlocked` needs
  a fresh `Ok` read with `LockedHint == false` on the selected session; `serde_json` failure →
  `503 unavailable`; every `HostError` → `421`, every `AuthError` → `403`, every `CsrfError` →
  `403`; snapshot or lock error → `503`; configuration errors refuse to start (`EXIT_CONFIG`,
  `RestartPreventExitStatus=78`); persistent socket errors (`NotADirectory`, `WrongOwner`,
  `NotASocket`) → `EXIT_CONFIG`, transient `Io` → `EXIT_RUNTIME`; root → refused before any
  configuration read (RMC-S10). The poller or accept loop ending makes `serve` return `Err` and
  the process exits `EXIT_RUNTIME`.
- `main.rs` handles `clap` help/version, runtime build failure and signal setup failure without
  panicking; `ExitCode` only.

→ **PASS**

### Test Integrity & Anti-Weakening

- §2 above: one changed assertion, justified by a recorded contract migration and strictly
  stronger. No `#[ignore]`, no tolerance, no deleted test, no inline test module.
- Power of the new D5a′ tests against plausible wrong implementations: a first-header-wins proto
  check fails "two identical `https` values are still repeated"; an implementation that still
  inspects `Host` fails "`Host: evil.com` + XFH → OK" and "`Host` repeated + XFH → OK"; an
  optional proto with XFH fails "proto absent with X-Forwarded-Host → NotAllowed"; a check that
  skips normalisation on XFH fails `" PC.Tail1234.TS.NET:443 "` → OK and `:8443` → `NotAllowed`;
  a fallback from a refused XFH to a valid `Host` fails "a valid Host never rescues a refused
  X-Forwarded-Host"; a `Host`-based `Origin` comparison fails the e2e `Origin: https://localhost
  → 403` / `Origin: https://pc.tail1234.ts.net → 202` pair; a proto check applied only with XFH
  fails "`Host` only + proto `http` → NotAllowed".
- The `MockSource` records reads and `LockSession` ids, so the e2e tests prove "no logind call
  before every check" instead of assuming it; time is frozen (`start_paused` plus an
  auto-advance inhibitor), so the deadline and keep-alive assertions are exact, not tolerant.
- No real-model contract is involved (no mock of channel order, layout or class index).

→ **PASS**

### Memory, Bounds & Secrets

- Request head buffer capped at `MAX_REQUEST_HEAD_BYTES + 1` and re-parsed after every read;
  `httparse` with exactly `MAX_HEADERS` slots; path ≤ `MAX_PATH_LEN`; no body ever read
  (`Content-Length > 0` → `413`, `Transfer-Encoding` → `400`, conflicting positive lengths →
  `400`, `Content-Length: 0` accepted); lingering close discards at most `LINGER_MAX_BYTES`;
  config read through `take(MAX_CONFIG_BYTES + 1)` on an `O_NONBLOCK | O_CLOEXEC` descriptor
  that must be a regular file (a FIFO cannot stall the start); `ListSessions` bounded by
  `MAX_LISTED_SESSIONS` / `MAX_OWN_SESSIONS`; `Properties.GetAll` only for own sessions;
  D-Bus queue `max_queued(16)`; logind error name truncated on a char boundary.
- Secrets: `TailscaleLogin` has a redacting `Debug`; `StatusView` carries no uid, session id,
  seat or login; `HostError` / `AuthError` / `CsrfError` `Display` texts are fixed; every
  tracing field key passes `test_rmc_logging_never_names_identity_header_or_session_fields`
  (`%err` of fixed-text errors, `status`, `?resolved` route enum, `dbus_error` truncated name,
  `socket_path` of the configuration). No frames, embeddings or passwords exist in this crate.
- Files: socket directory created `0700` or verified through `O_DIRECTORY | O_NOFOLLOW` +
  `fstat` owner check + `fchmod`; stale socket unlinked only when `symlink_metadata` says socket
  owned by the service uid; listener `set_permissions(0o600)` with unlink on failure; unit
  `UMask=0077`; config template written under `umask 077` then `chmod 0600`.
- Redaction: the patch contains no address other than the synthetic `100.64.0.1` and the
  placeholder `100.x.y.z`, only `*.tail1234.ts.net` / `arch.<tailnet>.ts.net` / `<pc>.<tailnet>.ts.net`
  names and `example.com` / `example.org` / `x.io` logins; `Docs/REMOTE_COMPANION.md`, the
  matrix (RMC20) and the walkthrough §9 use `<redacted>` / `<login>` / `<uid>`.
- `unsafe`: none in the crate (RMC-S1 passes; `remote` is in the forbid list and in
  `scripts/candid_review.sh`).

→ **PASS**

### Supply Chain & Automation

- `Cargo.toml`: `crates/remote` registered; `httparse = "1.10"` added to
  `[workspace.dependencies]` (already locked through `ureq`, exactly one version in the lock);
  `soos-remote` path dependency declared; `zbus` comment widened. `Cargo.lock` adds only the
  `soos-remote` package entry; no `hyper` / `axum` / `tower` (contract test). `proptest = "1"`
  as a dev-dependency follows the existing convention. `cargo deny --locked check bans licenses
  sources`: ok.
- `packaging/soos-remote.service`: `Type=exec`, `Restart=on-failure`,
  `RestartPreventExitStatus=78`, `NoNewPrivileges`, `RestrictAddressFamilies=AF_UNIX`,
  `LockPersonality`, `MemoryDenyWriteExecute`, `RestrictRealtime`, `RestrictSUIDSGID`,
  `SystemCallArchitectures=native`, `UMask=0077`, no `User=` / `Group=` / `DynamicUser=`,
  `WantedBy=default.target`. All of these are seccomp-based and apply to a user-manager unit
  (unlike the namespace-based `Protect*` / `Private*` options, which are correctly not used).
- `scripts/install_remote.sh`: `set -euo pipefail`, refuses `EUID 0`, never runs `sudo`,
  `tailscale` or `systemctl --user enable` (only prints them), `cargo build --release --locked`,
  `bash -n` clean; mode `100755` staged, content byte-identical to `HEAD` (auditor constraint 48);
  RMC-S12 covers both installer scripts.
- `scripts/candid_review.sh`: `remote` added to `BUSINESS_CRATES`, consistent with the invariant.
- No `.github/` or `deny.toml` change.

→ **PASS**

### English-Only Policy

Code, comments, docs, test names, the commit message of `0b0b50c` and the installer output are
English. The only non-ASCII letters in the patch are the `é` test inputs that assert non-ASCII
logins and hosts are refused, typographic arrows/dashes in the documentation and one
non-breaking space placeholder in `app.js`. `index.html` declares `lang="en"`.

→ **PASS**

## 4. Detailed Findings & Action Items

- **[MINOR]** `scripts/install_remote.sh:72` — the configuration template comment still reads
  "DNS names accepted in the Host header", while D5a′ (spec §13.2, `Docs/REMOTE_COMPANION.md`
  §5 row `allowed_hosts`) defines the list as names accepted as the **effective host**
  (`X-Forwarded-Host` set by `tailscale serve`, or `Host` for a direct local client); behind
  Serve the `Host` header is `localhost` and is not inspected, so the comment can mislead an
  owner filling in the file. Already recorded as a follow-up in the walkthrough §9 (auditor
  constraint 48 pins this cycle's installer change to the git mode). Correction: reword to
  "DNS names accepted as the effective host (`X-Forwarded-Host` set by `tailscale serve`, or
  `Host` for a direct local client) (0..=4)". Behaviour and tests are unaffected.
- **[MINOR]** `AI/architect_spec_remote_companion.md` D7 (§2.1) and
  `AI/auditor_constraints_remote_companion.md` constraint 24 — both describe the same-length
  tie-break of `select_session` as the **smallest** id under `(byte length, then bytes)`
  (`min_by_key(|s| (s.id.len(), s.id.as_bytes()))`), whereas the binding tester contract item 16
  asserts `"c9"` before `"10"` (which is the **greatest** same-length byte sequence, not the
  smallest: `b'1' < b'c'`), and the implementation (`session.rs:158`, `Reverse(s.id.as_bytes())`)
  and `AI/ARCHITECTURE.md` §13 follow the tester. The shipped behaviour is deterministic, covered
  by `test_rmc_select_session_prefers_active_local_user_seat_session`, and harmless (every
  candidate is an owner session on a local seat, active sessions win first, and the lock is
  confirmed through `LockedHint`), but the phase documents contradict each other. Correction
  (documentation only, no code or test change): amend spec D7 and auditor constraint 24 to
  "shortest id, then the greatest same-length byte sequence" and note in the tester contract that
  `"c9" < "10"` is false under byte order (the intended rule is "most recently allocated among
  same-length numeric ids").
- **[SUGGESTION]** `crates/remote/src/server.rs:478` — on a request-head parse error the
  response is always encoded as for `GET` (a body is sent even to a `HEAD` request), because the
  method is unknown when parsing failed. Harmless (`Connection: close`); a sentence in the
  `handle_connection` doc comment would avoid a future "bug" report.
- **[SUGGESTION]** `crates/remote/src/main.rs:41` — `EnvFilter::try_from_default_env()` reads
  `RUST_LOG`, a fourth environment variable beyond the three documented in spec §2.3
  (`XDG_RUNTIME_DIR`, `XDG_CONFIG_HOME`, `HOME`); the RMC-S9 needle (`env::var`) cannot see it.
  It is the standard tracing pattern and can only change log verbosity (no field value is
  identity-bearing), so no behaviour change is requested; list it in spec §2.3 / D12 or switch
  to a fixed `EnvFilter::new("info")`.
- **[SUGGESTION]** `tests/invariants/src/presence_unlock_contract.rs:195` — the test name
  `test_pau_zbus_is_used_only_by_the_daemon` now checks two allowed crates. The name is kept on
  purpose (matrix row PAU17); a later rename with a matrix update would remove the mismatch.

## 5. Final Verdict

**VERDICT: APPROVED**

No CRITICAL or MAJOR finding. The single changed assertion in the whole diff is a recorded,
strictly stronger contract migration; the D5a′ implementation follows spec §13.2 exactly and is
covered by unit, end-to-end and static tests that fail against the plausible wrong
implementations listed above; every blocking point is bounded, every failure path fails closed,
no identity or header value is logged or echoed, and all local CI commands pass on the reviewed
fingerprint. The two MINOR findings are documentation corrections that may land in this cycle or
as the follow-ups already listed in walkthrough 183 §9.
