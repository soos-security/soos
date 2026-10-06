# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `feat/remote-auth-alerts`
- **Base (merge-base)**: `222665f`
- **Reviewed-Diff-Fingerprint**: `39c34d962267df57fd25eccd1867a0b8b8fb35684a6f5ac3610375213b49c286`
- **Audited Files**: full branch diff vs `origin/main` (`target/candid_diff.patch`, 56 257 lines,
  108 files). Reviewed in depth (uncommitted alerts + Web Push work on top of `ea862cc`):
  `crates/remote/src/{alerts,journal,push,webpush}.rs`,
  `crates/remote/src/{server,routes,config,credentials,http,main,lib,assets,audit}.rs`,
  `crates/remote/assets/{app.js,sw.js,index.html,style.css}`, `crates/push-protocol/src/lib.rs`,
  `crates/push-sender/src/{lib,main}.rs`, `crates/push-sender/Cargo.toml`,
  `packaging/soos-push-sender.service`, `scripts/install_remote.sh`, `scripts/candid_review.sh`,
  `Cargo.toml`, `crates/remote/Cargo.toml`, `deny.toml`, `tests/invariants/src/{lib,
  remote_passkey_contract,remote_alerts_contract,remote_push_contract}.rs`,
  `crates/remote/tests/*`, `AI/DECISIONS.md`, `AI/tester_contract_push.md`,
  `AI/walkthroughs/{186,187}_*.md`. The committed remote companion / passkey / funnel part of
  the branch diff was scanned for test changes only (reviewed in earlier rounds).

## 1. Executive Summary

**Delta round (fingerprint `39c34d96…`, previous APPROVED report `11ca15a2…`).** Only two files
changed since the previous review (verified by modification time against the previous report:
no source, test, script or manifest file is newer): `AI/walkthroughs/187_remote_web_push.md`
§6 and `AI/DECISIONS.md` Web Push ADR item (10) (`git diff --stat`: 3 insertions, 1 deletion).
Both edits are accurate against the code and `AI/tester_contract_push.md` "Contract Migrations":

- Walkthrough §6 now describes the two applied migrations. Checked: `config_tests.rs:1329-1339`
  uses `MAX_SOCKET_PATH_LEN - 3` (asserted equal to 107) and `- 2` for the refused path, with
  `MAX_SOCKET_PATH_LEN = 107` still asserted at line 62; `remote_passkey_contract.rs:351-358`
  requires exactly one `.register(` and the `SERVICE_WORKER_PATH = "/sw.js"` registration with
  scope `/`, matching `app.js:63, 837`. The old `- 4` building 106 bytes (`1 + n + 2`) is
  correct arithmetic. Both tests re-run green here (`test_rmc_s18`, `test_rwp_push_config_keys`).
- ADR item (10) now records both migrations and points at the tester contract; the three
  setup-only `push: soos_remote::config::PushConfig::default(),` literals it lists exist in
  `harness.rs:783`, `server_tests.rs:774` and `alerts_server_tests.rs:216` (the ADR writes the
  short type name; prose only, not a defect).

Both previous MINOR findings are therefore resolved. The full-diff review below is carried
over unchanged, because no code changed since `11ca15a2…`.

### Carried-over summary of the previous round

This is a re-review after the previous `CHANGES_REQUESTED` (fingerprint `946bce8e…`), which
rested only on two failing contract tests. Both are now resolved by recorded Contract
Migrations (end of `AI/tester_contract_push.md`), and I judge both legitimate:

- (a) `test_rmc_s18`: `serviceWorker` removed from the forbidden tokens of `app.js`. This is a
  supersession justified by the 2026-10-06 Web Push ADR item (7), which explicitly introduces
  the same-origin `/sw.js` with no `fetch` handler and no storage. The removed needle is
  replaced by a **stricter** positive rule: exactly one `.register(` in `app.js` and it must be
  `navigator.serviceWorker.register(SERVICE_WORKER_PATH, { scope: "/" })` with
  `SERVICE_WORKER_PATH = "/sw.js"`. All other forbidden tokens (storage, `allowCredentials`,
  `innerHTML`, `eval(`) are untouched. `remote_push_contract` additionally pins the worker's
  behaviour. Not a weakening.
- (b) `test_rwp_push_config_keys`: pure fixture arithmetic. `"/" + n×"d" + "/s"` is `n + 3`
  bytes; `MAX_SOCKET_PATH_LEN - 3` now builds exactly 107 (accepted) and `- 2` builds 108
  (refused). The assertions and `MAX_SOCKET_PATH_LEN = 107` are unchanged; the old fixture made
  the "over" case sit exactly at the limit, so the fix makes the test strictly more meaningful.
- `journal_tests.rs:1075` comment reword: the comment now states the rule the production code
  actually implements (first ` user=` after `rhost=`, ambiguous second occurrence → `Other`),
  which is the fix of the previous SUGGESTION in `journal.rs:708-728`. The assertion beside it
  is unchanged in substance and is consistent with the new production rule.

The previous MINOR (misplaced doc comment, `config.rs:399-409`) and both SUGGESTIONS
(`rfind(" user=")` ambiguity, synchronous store read in the dispatcher) are fixed.

Full suites are green here: `cargo test -p soos-invariants -p soos-remote -p soos-push-protocol
-p soos-push-sender` (0 failures, every test binary listed below ok), `cargo clippy ... --all-targets
-- -D warnings` clean, `cargo fmt --all -- --check` clean.

The core invariant holds: no typed password is ever captured, stored, logged, displayed or
sent. The relayed request to see tried passwords is correctly declined; the ADR records owner
confirmation of the safe subset as **pending**, and the branch must not be merged before the
owner confirms it in his own words (ADR item (1)). Remaining findings are documentation drift
only (MINOR).

## 2. Test Changes (mechanical listing from step 3)

- Test files touched: new `crates/push-protocol/tests/protocol_tests.rs`,
  `crates/push-sender/tests/sender_tests.rs`, `crates/remote/tests/{alerts_server,alerts,
  journal_process,journal,push_server,push,webpush}_tests.rs`,
  `crates/remote/tests/common/{journal,push}.rs`, `tests/invariants/src/{remote_alerts,
  remote_push}_contract.rs`; modified `crates/remote/tests/{config,http,routes,server}_tests.rs`,
  `crates/remote/tests/common/harness.rs`, `tests/invariants/src/{lib,remote_passkey_contract}.rs`;
  plus the test files of the earlier committed remote work.
- Removed/changed assertions (`^-` hits with assert/test markers):
  - `remote_passkey_contract.rs` — the `"serviceWorker",` entry of the S18 forbidden list.
    Contract Migration 1 (Web Push ADR item (7)); replaced by two stricter assertions. Justified.
  - `presence_unlock_contract.rs` `test_pau_zbus_is_used_only_by_the_daemon` (committed part):
    documented Contract Migration (ADR 2026-10-05 item (7), PAU17), adds an `assert_eq!`.
    Justified (reviewed in a previous round).
- `config_tests.rs` vs `HEAD`: additions only, except the two repeat counts of the new
  `test_rwp_push_config_keys` (Contract Migration 2, arithmetic defect). Justified.
- `harness.rs` / `server_tests.rs`: setup-only `alerts:`/`push:` defaults in `RemoteConfig`
  literals (ADR item (10)). Justified.
- `tests/invariants/src/lib.rs`: module registration and `push-protocol`/`push-sender` added to
  the forbid-unsafe list (strengthening).
- New escape hatches (`#[ignore]`, `should_panic`, tolerance): none in code (the only hits are
  prose).
- Inline `mod tests` changes: none.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Fixed failure parser: `rhost=` anchor, first ` user=` after it, refusal on a second
  occurrence. Tried a typed user name `x user=sooshost` (lands before or after `rhost=`) and a
  remote host name `a user=sooshost`: the first case yields the trailing name or `Other`; the
  second leaves a second ` user=` in the remainder → `Other`. Only a class label is at stake.
  PASS.
- Correlator / AlertBook / push scheduler (coalesce 3 s, 30 s min interval, 20/h, retry caps,
  only live attempts ≤ 300 s old, never the 24 h replay): re-checked against spec; unchanged
  since the previous round. PASS.
- RFC 8291: `PRK_key = HMAC(auth, ecdh)`, `IKM = HMAC(PRK_key, "WebPush: info\0"‖ua‖as‖0x01)`,
  `PRK = HMAC(salt, IKM)`, `CEK/NONCE = HMAC(PRK, "Content-Encoding: aes128gcm|nonce\0"‖0x01)`
  truncated to 16/12; plaintext‖`0x02`; header `salt‖rs(BE)‖idlen=65‖as_public`; all-zero ECDH
  refused; Appendix A vector tested. RFC 8292: ES256 raw r‖s, `aud` = origin without trailing
  slash, `exp` = now + 12 h. PASS.
- Service worker: always shows a notification inside `waitUntil`, generic fallback on an
  unreadable payload, no `fetch` handler, no storage, click only focuses an existing window or
  opens `/` of its own origin. PASS.

### PAM Concurrency & Deadlines
- `crates/pam` untouched. New code is in user-level `soos-remote` (Tokio permitted) and the
  separate `soos-push-sender`. PASS.

### Panic Safety & Fail-Closed
- No `unwrap/expect/panic!` in new production code (clippy workspace lints clean); indexing via
  `get`. Journal/store/push failures surface as `unavailable`/`failed`, never as "no attempts";
  push errors never touch lock/unlock/PAM outcomes. Store reads now on `spawn_blocking`. PASS.

### Test Integrity & Anti-Weakening
- Both migrations analysed in §1/§2: one ADR-justified supersession replaced by a stricter rule,
  one arithmetic defect fix that preserves the "107 accepted, 108 refused" intent. No assertion
  relaxed elsewhere; full suite green under the CI commands. PASS.

### Memory, Bounds & Secrets
- No typed password, length or hash anywhere: journal names are reduced to `AccountClass`
  inside `classify_entry`; views, SSE events and push payloads carry enums and counts only.
  Journal reader buffers are zeroized (`journal.rs:293, 973-987`); lines ≤ 24 KiB checked before
  JSON. PASS.
- Authorization: `/api/alerts*` and `/api/push*` are never Funnel-public (`403 login_required`
  before body read), CSRF header and rate gate before body read, SSE re-validates the session
  before each send; push subscriptions ≤ 4, never evicted. PASS.
- SSRF: exact three-host allowlist checked at subscribe time and again in the sender; resolver
  refuses the request if any address is non-public (v4 special ranges incl. `100.64/10`; v6
  limited to `2000::/3` minus Teredo/doc/6to4, so ULA, mapped, NAT64 are refused); port 443,
  `https_only(true)`, `max_redirects(0)`, `proxy(None)`, `Agent::with_parts` with the filtering
  resolver only. PASS.
- Logging: no `tracing` in `push.rs`/`webpush.rs`; sender logs constant messages through
  `set_global_default`, no `log` bridge, `ureq`/`rustls` targets off. Keys, CEK/nonce, auth
  secrets and the body are `Zeroizing`; `DeliveryRequest` has no `Debug`. PASS.

### Supply Chain & Automation
- `ureq =3.4.2` with rustls/ring/webpki-roots only in the sender; RustCrypto in `soos-remote`;
  `deny.toml` skip reasons updated; `candid_review.sh` forbid-unsafe list extended; no
  workflow change. PASS.

### English-Only Policy
- Code, comments, docs, UI strings in the reviewed hunks are English. PASS.

## 4. Detailed Findings & Action Items

- Previous **[MINOR]** `AI/walkthroughs/187_remote_web_push.md` (stale "cannot pass" text):
  resolved, verified against the tests.
- Previous **[MINOR]** `AI/DECISIONS.md:125` Web Push ADR item (10): resolved, both migrations
  recorded.
- **[SUGGESTION]** Merge gate, not a code defect: the Web Push ADR item (1) keeps "owner
  confirmation pending" for the safe subset (classes and counts, never typed passwords); do not
  merge until the owner has confirmed it in his own words.

## 5. Final Verdict

No CRITICAL, MAJOR or MINOR finding. The delta since `11ca15a2…` is documentation only and
accurate. Both contract migrations are genuine (one ADR-backed supersession
with a stricter replacement, one fixture arithmetic fix) and not weakenings; the full suite,
clippy and fmt are green.

**VERDICT: APPROVED**
