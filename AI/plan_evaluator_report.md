# Plan Evaluation Report
- **Date**: 2026-10-06
- **Issue**: GitHub-only follow-up of #339 — Tailscale Funnel access and in-house passkey authentication for `soos-remote` (no `AI/BACKLOG.md` entry; owner decisions D-A to D-I of 2026-10-06)
- **Branch**: `feat/remote-funnel-passkey` (from `feat/remote-companion` at `b5d593a`)
- **Base commit**: `b5d593a` (branch base; `origin/main` is `222665f`; draft PR #340 belongs to `feat/remote-companion`)
- **Plan evaluated**: `AI/architect_spec_remote_passkey_funnel.md` **round 2** (revision log §0 answering round-1 findings F-1 to F-10) and the drafted ADR "[2026-10-06] Tailscale Funnel Access and In-House Passkey Authentication for `soos-remote`" in the uncommitted `AI/DECISIONS.md` diff (plus the supersession notes appended to the 2026-10-05 and 2026-10-06 ADRs)

## 0. Round-1 findings: resolution check

| Round-1 finding | Resolution in round 2 | Verdict |
|---|---|---|
| F-1 (MAJOR) pending user handle | `PendingRegistration` carried by the `Register` challenge, `Taken::Register` returned by `take`, store created with exactly that handle or `409 registration_conflict` (nothing written, code kept, not counted); handle never changes afterwards; `passkeys remove` keeps the handle. Tests 23 (extended), 41a (end-to-end handle consistency), 41b (interleaved options and conflicting store). | Resolved |
| F-2 (MAJOR) availability per path class | Class permits (`MAX_FUNNEL_CONNECTIONS = 8`, anonymous 4, anonymous body reads 2 / 1 per hint, Funnel SSE 2), `503 busy` never counted; anonymous login pool evicts oldest (≤ 2 per hint, 16 total); limiters keyed by `LimitKey` with `ClientHint` from the tailscaled-set `X-Forwarded-For` (rate-limit bucket only, static contract 53a); residual DoS stated in the ADR. Tests 24a, 42, 54–59. | Resolved, with one remaining gap in the capacity guarantee (G-1, MINOR) |
| F-3 test 46 substring | Exact first-token crate names; `rand_core` explicitly allowed and asserted. | Resolved |
| F-4 SSE touch | `Touch::{Refresh, Keep}`; SSE keep-alive uses `Keep`; test 25 and 38 pin both directions. | Resolved (one wording conflict for assets, G-3) |
| F-5 UV-clear unlock negative | Test 39a, both paths, zero logind calls, challenge consumed, plus login-purpose assertion to `/api/unlock`. | Resolved |
| F-6 revocation + rotation | Dispatch step 7(a) revocation before validation; rotation in `login_verify`; test 38 extended. | Resolved |
| F-7 store error after valid assertion | §5.1 9a/9b split; `503 store_unavailable`, not counted, no logind; test 43 extended. | Resolved |
| F-8 invariant needles | §7 lists RMC-S3 and every `test_rmc_unlock_is_opt_in_and_documented` needle verbatim; ADR keeps the old title with a supersession note (verified in the diff). | Resolved |
| F-9 chunked framing | Strict bounded chunked decoder on the four body routes only, TE+CL refused, evidence paragraph, tests 12/13, RMC29/RMC40. | Resolved |
| F-10 local same-uid threat | §2.2 item 9 and ADR "local same-uid processes" accepted risk. | Resolved |

## 1. Coverage Matrix

No backlog entry exists; the acceptance lines are the binding owner decisions and the spec's own matrix rows.

| Acceptance line / TDD test | Spec element | Status |
|---|---|---|
| D-A Funnel, no VPN, no cost; tailnet path kept | §2.1 `classify_request`, §2.3, S-1 (443 only), S-2 (`allow_funnel` opt-in), §9.2 owner deployment; RMC26/27/40; tests 8–10, 32–35 | Covered |
| D-B in-house WebAuthn, UV required | §4.1 `webauthn.rs` (L3 §7.1/§7.2 subset), S-3, S-7; RMC30/31; tests 16–22 | Covered |
| D-C fresh UV assertion for every unlock, every path | §5.1 steps 1–10 (no path skips `verify_assertion`, UV unconditional), `Unlock` purpose, `WebSession` binding on Funnel, `None` on tailnet; RMC36; tests 39, 39a, 40, 43, §10.7 rewrites | Covered |
| D-D Funnel: session needed for status/events/lock; tailnet one-tap kept | §5 table, §6 step 7, `is_funnel_public`; RMC27/33; tests 33, 36, 38 | Covered (G-2 wording conflict on logout/register ordering) |
| D-E registration tailnet-only + local one-time code | §4.5, §5 register rows (code file re-read at verify, binding `EnrollCode(hash)`), §6 step 7 (Funnel ⇒ `403` before body), §9.1 `enroll-code`; RMC34; tests 30, 31, 41, 41a, 41b | Covered |
| D-F no auto re-lock; unlock opt-in | §5.1 step 3 and step 10 unchanged | Covered |
| D-G pure Rust, AF_UNIX only, cargo-deny, explicit bounds | §3.1 (every collection bounded), §3.4, §9.2 unit unchanged; RMC29/37/39/41; tests 46/47, 54–59 | Covered (G-1 on the capacity claim) |
| D-H no secrets in logs/responses | §4 redacted `Debug`, `ClientHint` redacted, §7 RMC14/RC-5 extension; RMC38; tests 22, 45, 48, 58 | Covered |
| D-I agents never deploy | §9.2 installer `echo` only, RMC-S6 unchanged | Covered |
| Superseded assertions declared | §10.7 and ADR item (10) list each test and the exact assertion changed | Covered |
| Existing invariants (RMC-S2/S3/S4/S9/S10, RC-5, unlock-opt-in needles) | §7 table | Covered |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `MAX_CONNECTIONS`, `MAX_SSE_STREAMS` | `crates/remote/src/lib.rs:42,44` | 16, 4 | Yes |
| `REQUEST_HEAD_TIMEOUT_MS`, `RESPONSE_WRITE_TIMEOUT_MS` | `lib.rs:52,54` | 5000, 2000 | Yes |
| `SSE_KEEPALIVE_MS`, `MAX_SSE_STREAM_MS` | `lib.rs:56,58` | 15 000, 1 800 000 | Yes (test 38 "within one keep-alive" = 15 s) |
| `MIN_UNLOCK_INTERVAL_MS`, `UNLOCK_FLOW_DEADLINE_MS` | `lib.rs:99,101` | 2000, 2000 | Yes |
| Accept loop drops a stream when the global semaphore is empty | `server.rs:377-380` (`try_acquire_owned`, `drop(stream)`) | yes, permit held to connection end | Yes — relevant to G-1 |
| Lingering close after every response | `server.rs:427-444` `finish` | discards input up to `RESPONSE_WRITE_TIMEOUT_MS` (2 s) or 64 KiB while the global permit is still held | Not considered by §6 capacity arithmetic (G-1) |
| tailscaled uses `ReverseProxy` with `Rewrite` (client `X-Forwarded-For` stripped, one value set) | `AI/research_funnel.md` lines 100-127 (source quotes) | `Header.Del` then `Header.Set(c.SrcAddr…)` | Yes (S-12 sound) |
| tailscaled deletes `Tailscale-User-Login` / `Tailscale-Funnel-Request` before setting `?1` for Funnel | `research_funnel.md` lines 129-147 | as cited | Yes |
| `remote` already depends on `serde_json` (duplicate-field refusal via derive) | `crates/remote/Cargo.toml` | `serde_json = { workspace = true }` | Yes |
| `deny.toml` `multiple-versions = "deny"`; only `der 0.8.2` locked | `deny.toml:59`, `Cargo.lock` | deny; `der 0.8.2` only; `crypto-bigint`, `signature`, `hmac`, `rfc6979`, `ff`, `group`, `elliptic-curve`, `sec1`, `primeorder` absent (new, single versions); `rand_core 0.6.4` present | Yes (one `der@0.7.10` skip suffices as far as the lock shows; test 47 + `cargo deny` confirm) |
| `base64ct 1.8.3`, `subtle 2.6.1`, `zeroize 1.9.0`, `digest 0.10.7`, `generic-array 0.14.7` locked | `Cargo.lock` | as listed | Yes |
| Existing tests named in §10.7 / §7 | `crates/remote/tests/server_tests.rs`, `config_tests.rs`, `tests/invariants/src/remote_companion_contract.rs` | all 9 checked names found | Yes |
| ADR supersession notes on the old ADRs, old titles kept | `AI/DECISIONS.md` diff | items (8) and (2) carry notes; titles unchanged | Yes |
| `app.js` has no timer that refetches status/state | `assets/app.js:99, 308` | `setInterval(renderUpdated)` only re-renders; refetch only on `visibilitychange`/`pageshow` | Yes (F-4 premise holds) |
| `allowed_hosts` may hold non-`.ts.net` names | `identity.rs:111`, `config.rs:66` | "empty = any `*.ts.net`; otherwise a configured name" | Relevant to G-6 |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Scenario: internet client sends `Tailscale-User-Login: owner@…` (any case, repeated) through Funnel. tailscaled deletes it (canonical key) and sets `Tailscale-Funnel-Request: ?1`; rule 1 also refuses both markers; `_` spellings never match. Forgery excluded; even a hypothetical forgery cannot unlock (D-C) or register (D-E).
- Scenario: internet client forges `X-Forwarded-For` to pick the owner's bucket. Excluded: `Rewrite` strips it and sets one value; it is only a bucket, never authorization (contract 53a).
- Scenario: internet client tries to register. Funnel ⇒ register routes `403` at dispatch step 7 before any body read; tailnet additionally needs the 50-bit local code (3 attempts, 5 min, plus the 5-failure `Tailnet` lockout).
- Scenario: internet flood to starve the tailnet. Classified Funnel requests are capped at 8, but every request refused **before classification** (dispatch steps 1–3: `413 body_not_allowed` for a non-body route with a declared body, `400`, `421`, `403`) and every `503 busy` refusal goes through the existing lingering close while still holding a **global** permit and **no** class permit. A client that declares a body and keeps the request open can keep each such connection for up to `RESPONSE_WRITE_TIMEOUT_MS` = 2 s (plus the write bound). About 8 requests/s keep all 16 global permits busy and the accept loop drops tailnet connections. Whether this is reachable through tailscaled depends on Go's transport closing the upstream connection once it has the response (plausible but not verified in `research_funnel.md`). The ADR says the cost is "the milliseconds each request lives", which understates it. Denial of service only, never an authentication bypass.
- Result: FINDING (MINOR, G-1).

### Pillar 2 — PAM deadline & concurrency
- Not applicable to PAM: leaf user-level crate, no PAM/IPC/daemon change (RMC-S4). Inside the service, the `AuthState` mutex can be held across a bounded store-lock wait (≤ 500 ms) and bounded file I/O. `Capacity` uses a std mutex that is never held across an await, with poison recovery. Scenario: a stuck `.lock` holder delays every Funnel request (step 7 takes the auth mutex) by ≤ 500 ms. This is bounded.
- Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- Scenario: store unreadable ⇒ `live_credential_hashes() = None` ⇒ every session dropped, `503 store_unavailable`; never "zero passkeys". RNG failure ⇒ `503 unavailable`, nothing stored. Funnel marker missing in a future tailscaled ⇒ `403`. Store failure after a valid assertion ⇒ `503`, no logind call. A conflicting user handle ⇒ `409`, nothing written. Every refusal comes before any logind call.
- Scenario: an UP-only assertion validly signed, sent to `/api/unlock` on either path ⇒ `verify_assertion` UV check is unconditional, and test 39a pins it end to end.
- Scenario: register-verify with the code file already consumed by a concurrent registration ⇒ `enroll_code_rejected` before `take` (the stale challenge stays until its 120 s expiry, inside the 4-entry pool). This is bounded and fail-closed.
- Result: PASS.

### Pillar 4 — Dependencies
- Scenario: `p256 0.13.2` (`ecdsa`) adds `ecdsa 0.16`, `elliptic-curve 0.13`, `sec1 0.7`, `der 0.7.10`, `crypto-bigint 0.5`, `ff`, `group`, `primeorder`, `rfc6979`, `hmac`, `signature`. In today's lock only `der` would be duplicated, and `rand_core 0.6.4` is already present. All are RustCrypto crates under Apache-2.0/MIT. Test 46 now matches exact names, test 47 pins the skip, and `cargo deny --locked check` is the final gate. No OpenSSL, `ring`, `aws-lc` or `webauthn-rs`. `forbid(unsafe_code)` is kept.
- Result: PASS.

### Pillar 5 — Data confidentiality
- Scenario: an anonymous client harvests credential ids. This is excluded: no `allowCredentials` is sent, and `exclude_credentials` reaches only a tailnet caller who holds the code.
- Scenario: session fixation or login CSRF. Tokens are server-generated after an assertion. `__Host-` blocks `Domain` cookie tossing from sibling `*.tailnet.ts.net` Funnel nodes. Rotation on login, plus an exact `Origin`, custom `X-Soos-Action` (preflight never answered) and `Sec-Fetch-Site` on every state-changing route. A sibling node is same-site but cross-origin, so its requests are refused.
- WebAuthn completeness is checked: `type`, `origin` byte-exact, `crossOrigin`/`topOrigin`, challenge length, single use (removed before any check), expiry, purpose, class and binding, `rpIdHash`, UP, UV, AT/ED, BE/BS, strict COSE ES256 on-curve, DER over `authData || SHA-256(clientDataJSON)`, `userHandle` constant-time, stored credential lookup, counter rule. The challenge is consumed before ECDSA runs, so anonymous CPU per verify needs a rate-limited issuance.
- Logging: `ClientHint` `Debug` is redacted, test 45 adds client address/hint, and contract 48 covers field names.
- Result: PASS.

### Pillar 6 — Test integrity
- Scenario: a wrong `client_hint` that maps IPv4 into 16 bytes and then masks *every* address to `/64` puts all IPv4 clients into one bucket. Test 58 compares `203.0.113.7` with `::ffff:203.0.113.7` (both collapse to the same value, so it passes) and checks distinct IPv6 `/64`s, but never two distinct IPv4 addresses. That wrong implementation passes test 58.
- Scenario: test 54 saturates only the class permits with well-behaved held requests. It cannot detect G-1 (pre-classification and `503 busy` refusals lingering on global permits).
- Scenario: dispatch step 7 as written ("non-public without session ⇒ `login_required`; register routes ⇒ `forbidden`") gives `login_required` for an anonymous Funnel register request, while test 33 expects `forbidden`. The tester and developer would have to guess which one is the contract.
- §10.7 contract migration is explicit and tied to ADR item (10). Every other existing test stays.
- Result: FINDING (MINOR, G-2, G-4; G-1 test part).

## 4. Findings

- **[MINOR] G-1 — The class reservation does not cover refusals that linger on a global permit.** Dispatch steps 1–3 refusals (`413 body_not_allowed`, `400`, `421`, `403`) take no class permit. `503 busy` refusals hold the global permit through `finish` (≤ `RESPONSE_WRITE_TIMEOUT_MS` = 2 s of lingering). So "tailnet callers can always obtain at least 8 classified slots" (§6) and "the tailnet always keeps 8" (ADR item 7) are not guaranteed by the service itself. Residual (ii) ("milliseconds") understates the window. Required plan change, folded into the same spec/ADR before or during Phase 2:
  - (a) Bound it. Either skip or shorten the lingering close for every refusal of a request that is not a classified tailnet request (for example ≤ 100 ms, or close at once after the response for `503 busy`), or take the Funnel class permit as soon as the head shows the Funnel marker, before framing/host refusals.
  - (b) Restate residual (ii) in §5.2 and the ADR with the true bound.
  - (c) Add a test 54a: 16 Funnel requests, each declaring a body on a non-body route or receiving `503 busy` and then holding the connection open. A tailnet `GET /api/status` must still get `200` within the chosen bound.
- **[MINOR] G-2 — Dispatch step 7 contradicts the route table and test 33.**
  - (a) Register routes on the Funnel path must answer `403 forbidden` *before* the `login_required` check (test 33 and §5.3 expect `forbidden` for anonymous callers too). Reorder step 7.
  - (b) §5 says `POST /api/auth/logout` works on the Funnel path "with or without session", but `is_funnel_public` (§4.7) does not list `Logout`, so an expired-session sign-out gets `login_required` and the cookie is never cleared. Add `Logout` to `is_funnel_public`, and to tests 14 and 33.
- **[MINOR] G-3 — Asset requests and `Touch`.** §4.3 says assets "never validate". Dispatch step 7(b) validates every Funnel request with `Touch::Refresh`, and it has to validate in order to choose the anonymous permit. State that asset and `NotFound`/`MethodNotAllowed` validations use `Touch::Keep`, and pin this in test 38 (an asset fetch at 14 min does not extend the idle expiry).
- **[MINOR] G-4 — Test 58 cannot catch an IPv4-collapsing hint.** Add: `203.0.113.7` vs `203.0.113.8` ⇒ different hints; `198.51.100.1` vs `2001:db8::1` ⇒ different; and an IPv4-mapped address must not be masked to `/64`.
- **[MINOR] G-5 — Hint-bucket eviction resets lockouts.** An attacker with ≥ 65 public addresses (one IPv6 `/48`) can evict its own locked bucket and start a fresh failure window. This does not weaken authentication (no assertion can be brute-forced and every verify needs an issued challenge), but it should appear among the ADR residuals next to "one IPv6 `/48`". It is already partly implied, so only the wording needs to change.
- **[MINOR] G-6 — `rp_id` breadth with a non-empty `allowed_hosts`.** The two-labels-before-`.ts.net` rule only applies when `allowed_hosts` is empty. With an allowlist, `rp_id` only has to be a member, and the existing code accepts configured names that are not `.ts.net`. Apply the "full node host under `.ts.net`, at least two labels" rule in both cases (Funnel only serves `*.ts.net`), and extend test 3.
- **[MINOR] G-7 — Editorial.** In the §12 table, row RMC40 lacks its status cell and row RMC41 carries RMC40's "Manual check … Pending (owner)" cells. Fix both before the rows are copied into `AI/VERIFICATION_MATRIX.md`. §2.4 is placed before §2.3.

## 5. Verdict
VALIDATION_VERDICT: APPROVED

Round 2 resolves both round-1 MAJOR findings and all eight MINOR ones. No CRITICAL or MAJOR finding remains. Four parts of the security core are sound:
- Identity-header handling over Funnel: classification, forgery and spelling variants are covered, and the design fails closed when the marker is missing.
- WebAuthn verification is complete for the declared ES256/`none` subset.
- Enrollment cannot be reached from the internet: it needs the tailnet path, an allowed identity and a local one-time code.
- Unlock always needs a fresh `unlock`-purpose assertion with UV, on both paths, with zero logind calls on any refusal.

The cookie and CSRF design, the bounded memory (every collection capped in §3.1) and the fail-closed mapping all hold. G-1 to G-7 are MINOR. The architect should fold G-1 (with its test 54a), G-2, G-3 and G-4 into the spec before the tester writes tests 14, 33, 38, 54 and 58, because these change test expectations. G-5 to G-7 are wording fixes for the ADR, the config rule and the matrix.
