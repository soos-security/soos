# Architect Spec — GitHub #339 follow-up: Tailscale Funnel Access and In-House Passkey Authentication for `soos-remote`

- **Date**: 2026-10-06
- **Branch**: `feat/remote-funnel-passkey` (from `feat/remote-companion` at `b5d593a`); nothing is committed,
  pushed or deployed by agents (D-I).
- **Inputs**: owner decisions D-A to D-I (2026-10-06), `AI/research_funnel.md`, `AI/research_webauthn.md`,
  ADRs 2026-10-05 and 2026-10-06 on `soos-remote`, `Docs/REMOTE_COMPANION.md`, walkthroughs 185/186,
  `crates/remote/src/*.rs`, `crates/remote/tests/*.rs`, `tests/invariants/src/remote_companion_contract.rs`.
- **ADR**: "[2026-10-06] Tailscale Funnel Access and In-House Passkey Authentication for `soos-remote`"
  (drafted in `AI/DECISIONS.md`, text in §13 below).
- **Matrix**: new rows RMC26–RMC41 (§12); amended rows RMC23–RMC25 (§11).
- **Revision**: round 2 (2026-10-06), resolving every finding of `AI/plan_evaluator_report.md` round 1
  (F-1 to F-10). The revision log is §0; every changed paragraph below is marked *(rev 2, F-n)*.

## 0. Revision log (round 2)

| Finding | Severity | Resolution | Where |
|---|---|---|---|
| F-1 pending registration user handle not representable | MAJOR | The `Register` challenge entry carries the pending user handle (`PendingRegistration`); `take` returns it; register-verify creates the store with exactly that handle or, when the store exists, requires an equal handle (`409 registration_conflict`, nothing written, code kept). Tests 41a/41b. | S-10, §4.2, §4.6, §5 register rows, §5.3, §10.6 |
| F-2 availability not bounded per path class | MAJOR | (a) Per-class connection caps applied right after classification: `MAX_FUNNEL_CONNECTIONS = 8` of the 16 (tailnet keeps ≥ 8), `MAX_ANONYMOUS_FUNNEL_CONNECTIONS = 4` inside them, `MAX_ANONYMOUS_BODY_READS = 2` (1 per client hint), `MAX_FUNNEL_SSE_STREAMS = 2` of the 4; refusal `503 busy`, never a failure count. (b) The anonymous Funnel login pool evicts (oldest first; at most 2 pending per client hint, 16 in all). (c) Limiters are keyed by `LimitKey` {`Tailnet`, `FunnelSession`, `FunnelAnonymous(ClientHint)`}; `ClientHint` comes from the tailscaled-set `X-Forwarded-For` and is a rate-limit bucket only, never an authorization input; anonymous garbage can only lock its own bucket, never a session holder or the tailnet. Residual DoS stated in the ADR. Tests 24a, 42 (rewritten), 53a, 54–59. | S-11, S-12, §2.4, §3.1, §4.2, §4.6, §5.2, §6, §10.6, §10.9 |
| F-3 test 46 substring check fails on `rand_core` | MINOR | Exact crate-name match on `cargo tree --format {p}` first tokens; `rand_core` explicitly allowed. | §3.4, test 46 |
| F-4 SSE touch semantics | MINOR | `validate` takes `Touch`; SSE keep-alive re-validation uses `Touch::Keep`; every request-level validation (incl. `GET /api/auth/state`) uses `Touch::Refresh`. Pinned in test 38. | §4.3, §5, test 38 |
| F-5 no end-to-end UV-clear unlock negative | MINOR | Test 39a: validly signed UP-only assertion to `POST /api/unlock` ⇒ `403 passkey_rejected`, zero logind calls, tailnet and Funnel; a login-purpose assertion with a valid session ⇒ rejected. | test 39a |
| F-6 revocation step and session rotation | MINOR | Dispatch step 6 calls `live_credential_hashes` + `retain_credentials` before `validate`; a successful login removes the presented valid session record (rotation). Test 38 extended, test 44 kept. | §4.3, §5 login row, §6, test 38 |
| F-7 StoreError after a valid assertion | MINOR | §5.1 step 9 split: verification failure ⇒ `403 passkey_rejected` (counted); store failure while persisting ⇒ `503 store_unavailable`, challenge consumed, **not** counted, no logind call; `Busy` keeps sessions, the other store errors drop them. Counter written only when changed. Test 43 extended. | §5.1, §5.3, test 43 |
| F-8 existing invariant needles | MINOR | RMC-S3: no Rust source ever contains the quoted literal `"Unlock"` (no `Display`/serde of `ChallengePurpose`). `test_rmc_unlock_is_opt_in_and_documented` needles listed verbatim and kept by §8, §9.3 and §14. | §7, §8, §9.3, §14 |
| F-9 body framing through tailscaled | MINOR | Bounded strict `Transfer-Encoding: chunked` decoding on the four body routes (Go `ReverseProxy` forwards an HTTP/2 request without `content-length` as chunked); non-body routes keep `413 body_not_allowed`, only body routes use `413 body_too_large`. RMC29 and RMC40 extended. | §3.1, §4.8, §5.3, tests 12/13 |
| F-10 local same-uid residual risk | MINOR | Stated in §2.2 item 9 and in the ADR accepted risks. | §2.2, ADR |

Owner decisions restated (binding):

| Id | Decision |
|---|---|
| D-A | Reach the PC from the iPhone without a permanent VPN and without cost: Tailscale Funnel (public HTTPS on the node's `*.ts.net` name). No Cloudflare, no domain. The tailnet path stays supported. |
| D-B | Authentication is in-house WebAuthn passkeys (Face ID / Touch ID, user verification required). |
| D-C | A fresh passkey assertion with UV is required for **every** unlock, on **every** path (tailnet included). |
| D-D | Over Funnel, status, events and lock need an authenticated session obtained by a passkey login; without it only the static page/assets (and the login ceremony) are reachable. On the tailnet an allowed Tailscale identity keeps one-tap status and lock. |
| D-E | Passkey registration only from the tailnet path, with an allowed identity **and** a short-lived one-time code generated locally (`soos-remote enroll-code`). |
| D-F | No automatic re-lock; unlock stays opt-in (`allow_unlock`). |
| D-G | Pure Rust, no OpenSSL, no network socket (`RestrictAddressFamilies=AF_UNIX` kept), cargo-deny clean; every bound explicit. |
| D-H | Never log or expose credentials, challenges, session tokens, credential ids, identities or session ids. |
| D-I | Agents never commit/push/stash/open PRs, never run `tailscale serve`/`tailscale funnel`, never restart host services, never touch `~/.config`. |

Spec-level decisions taken here (each is restated in the ADR):

| Id | Decision | Rationale |
|---|---|---|
| S-1 | Funnel is supported on **port 443 only** (`tailscale funnel --bg unix:<socket>`, deployment (A) of `research_funnel.md` §1.3). `X-Forwarded-Host: <host>:8443` stays `421` exactly as today. | One origin `https://<rp_id>` for both paths; no change to the existing host check or its tests (`identity_tests` already pin `:8443` ⇒ `NotAllowed`). Deployment (B) needs its own ADR. |
| S-2 | Funnel is **opt-in**: `allow_funnel = false` by default. With it false, a Funnel-marked request is `403 forbidden`, exactly as today. | Upgrading the binary never exposes anything new. |
| S-3 | Login and unlock ceremonies send **no `allowCredentials`** (discoverable credentials only; `residentKey: "required"` at registration) and require `userHandle`. | Anonymous Funnel clients never receive a credential id (D-H). |
| S-4 | A Funnel unlock needs **both** a valid web session **and** a fresh `unlock`-purpose assertion. A tailnet unlock needs the allowed identity **and** the assertion. | D-C on every path; anonymous internet clients cannot even make the server read an unlock body or run ECDSA. |
| S-5 | The credential store lives next to the configuration file: default `<dir of remote.toml>/remote-passkeys.json`, override `credentials_path`. | `main.rs` may read only `XDG_RUNTIME_DIR`, `XDG_CONFIG_HOME`, `HOME` (invariant RMC-S9); `XDG_STATE_HOME` is therefore not used. |
| S-6 | The enrollment code reaches the running service through a `0600` file `enroll-code` in the existing `0700` socket directory; the service never accepts a "local-only" HTTP route. | The socket is also reached by `tailscaled`, so a local-only route cannot be told apart reliably. |
| S-7 | Only `fmt == "none"` with an empty `attStmt`, only ES256 (COSE `-7`), `signCount` `0/0` accepted, a non-zero non-increasing counter is rejected (fail-closed). | `research_webauthn.md` §1.4, §2.2 step 9; owner question 3/4 answered with the proposed default. |
| S-8 | Challenge TTL and client `timeout` are 120 s (deliberate deviation from L3's 300–600 s). Sessions: 15 min idle, 8 h absolute, at most 4. | Freshness of a remote unlock; owner questions 1/2 answered with the proposed default. |
| S-9 | `p256 = 0.13.2` (`default-features = false`, `features = ["ecdsa"]`) with one `deny.toml` skip for `der@0.7.10`. | Owner question 5; only RustCrypto, no OpenSSL. |
| S-10 *(rev 2, F-1)* | The user handle of a first registration is generated at register-options time and **held in the `Register` challenge entry**; register-verify writes exactly that handle, or refuses when the store already holds another one. The handle is never persisted before a successful registration. | No disk write before a verified attestation; a concurrent or stale ceremony can never create a passkey that cannot authenticate. |
| S-11 *(rev 2, F-2)* | Capacity is reserved per class: Funnel requests may hold at most 8 of the 16 connections, anonymous Funnel requests at most 4 of those, anonymous body reads at most 2 (1 per client hint), Funnel SSE streams at most 2 of the 4. A refused request gets `503 {"result":"busy"}` and is never counted as an authentication failure. | The owner's tailnet path (and a logged-in Funnel owner) keeps capacity whatever the internet sends. |
| S-12 *(rev 2, F-2)* | Anonymous Funnel limits are bucketed by a `ClientHint` derived from the single `X-Forwarded-For` value set by `tailscaled` (IPv4 address, IPv6 `/64`); it is a rate-limit bucket only, never an authorization input, never logged. Session holders and tailnet callers have their own limiter keys. The anonymous login challenge pool evicts its oldest entry when full. | `research_funnel.md` §2.3 allows the address as a hint; eviction can only fail a ceremony, never admit one. |
| S-13 *(rev 2, F-9)* | Body routes accept `Content-Length` or `Transfer-Encoding: chunked` (strict, bounded decoder), never both. | Go `ReverseProxy` forwards an inbound HTTP/2 request with no `content-length` as chunked HTTP/1.1; refusing chunked could break every passkey POST from Safari over Funnel. |

---

## 1. Scope & Blast Radius

### 1.1 Crates, modules, binaries

| Location | Change |
|---|---|
| `crates/remote/src/lib.rs` | New `pub mod` lines (`auth`, `challenge`, `credentials`, `enroll`, `webauthn`, `websession`) and the constants of §3.1 (single source of truth). |
| `crates/remote/src/config.rs` | `RemoteConfig` gains `pub auth: AuthConfig`; `FileConfig` gains `rp_id`, `allow_funnel`, `credentials_path`; new `ConfigError` variants; new pure `resolve_credentials_path`. |
| `crates/remote/src/identity.rs` | New pure `classify_request` (§2.1), `PathClass`, `Caller`; *(rev 2)* `ClientHint`, `client_hint` (§2.4); `authorize` and `check_host` unchanged. |
| `crates/remote/src/http.rs` | New `parse_request` (body framing for body routes only), `ParsedRequest`, `HttpError::BodyTooLarge`, `read_body`, `BodyError`; `Response` gains nothing (Set-Cookie travels in `extra_headers`); `reason_phrase` unchanged (no new status). `parse_request_head` keeps its exact current semantics. |
| `crates/remote/src/routes.rs` | New `Route` variants, `accepts_body`, `is_funnel_public`, `check_auth_csrf`; `check_lock_csrf`/`check_unlock_csrf` unchanged. |
| `crates/remote/src/webauthn.rs` (new) | Pure WebAuthn L3 §7.1/§7.2 verification subset (ES256, attestation `none`). |
| `crates/remote/src/challenge.rs` (new) | Bounded, purpose/path/binding-scoped, single-use challenge store. |
| `crates/remote/src/websession.rs` (new) | Funnel web sessions (cookie format/parse, hashed tokens, TTLs, cap). Named `websession` because `session.rs` already means logind sessions. |
| `crates/remote/src/credentials.rs` (new) | Credential store file (`0600`, bounded, `O_NOFOLLOW`, owner check, `flock`, atomic write) and the in-memory cache with change detection. |
| `crates/remote/src/enroll.rs` (new) | Enrollment code generation, normalisation, hashing, code file write/read/remove. |
| `crates/remote/src/auth.rs` (new) | `AuthState` (stores + limiters keyed by `LimitKey`, rev 2), `Capacity` (class permits, rev 2), ceremony orchestration (`register_options`, `register_verify`, `login_options`, `login_verify`, `unlock_options`, `verify_unlock_assertion`, `logout`, `state_view`), `RandomSource`. |
| `crates/remote/src/server.rs` | New dispatch order (§6), body reading, Funnel gate, auth handlers, SSE session re-validation, `Shared` gains `auth: tokio::sync::Mutex<AuthState>` and *(rev 2)* `capacity: Capacity`; `ServerState::with_random` test hook. `lock_flow`/`unlock_flow` bodies unchanged. |
| `crates/remote/src/main.rs` | clap subcommands `enroll-code`, `passkeys list`, `passkeys remove <N>`; resolves `credentials_path`; start-up warning when `allow_unlock` without `rp_id`. Still reads only the three env vars; still `check_not_root` before `load_config`. |
| `crates/remote/assets/index.html`, `app.js`, `style.css` | Login screen, Face ID unlock, enrollment form, sign-out (§8). |
| `crates/remote/Cargo.toml` | New deps (§3.4). |
| Root `Cargo.toml` | `[workspace.dependencies]` gains `p256`, `base64ct`, `subtle`. |
| `deny.toml` | One `[bans] skip` for `der@0.7.10` (§3.4). |
| `packaging/soos-remote.service` | **No change** (still `RestrictAddressFamilies=AF_UNIX`; `~/.config` writable because the unit sets no `ProtectHome`). |
| `scripts/install_remote.sh` | Template gains commented `rp_id`, `allow_funnel`, `credentials_path`; printed next steps mention `soos-remote enroll-code` and (inside `echo`) `tailscale funnel --bg unix:`. Never runs `tailscale`. |
| `Docs/REMOTE_COMPANION.md`, `AI/ARCHITECTURE.md` §13, `AI/MOCK_STRATEGY.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` (crate list), `.claude/skills/dev-workflow/references/project-facts.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/187_remote_funnel_passkey.md` | Documentation (traceability phase). |
| `tests/invariants/src/remote_passkey_contract.rs` (new) + `tests/invariants/src/lib.rs` (`mod` line) | Static contracts RMC-S13–RMC-S21 (§10.8). |

### 1.2 Consumers

`soos-remote` is a leaf crate (RMC-S4): no other workspace crate depends on it, so no cross-crate consumer
update exists. `soos-daemon`, `pam_soos.so`, the IPC protocol and every PAM path are untouched; no PAM
`PAM_IGNORE` pathway is affected (no PAM test is required by this change).

### 1.3 Out of scope

Funnel on 8443/10000 (S-1), push notifications, live camera, automatic re-lock (D-F), passkey sync
management beyond list/remove, attestation formats other than `none`, algorithms other than ES256,
multiple users (single owner, one user handle).

---

## 2. Request classification and trust model

### 2.1 `classify_request` (pure, `identity.rs`)

```rust
/// Which proxy path a request came through. Never derived from `Host`, `X-Forwarded-Host`
/// or `X-Forwarded-For`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PathClass {
    /// Tailnet request with an allowed `Tailscale-User-Login`.
    Tailnet,
    /// Public request through Tailscale Funnel (`Tailscale-Funnel-Request: ?1`, no identity).
    Funnel,
}

/// The authenticated caller of a request (before any web-session check).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// Allowed tailnet identity (the login is never logged; `TailscaleLogin` redacts `Debug`).
    Tailnet(TailscaleLogin),
    /// Anonymous Funnel client; may still carry a web session cookie (checked later).
    Funnel,
}

impl Caller {
    #[must_use]
    pub fn class(&self) -> PathClass;
}

/// Lowercased name of the Funnel marker header set by tailscaled (research_funnel §2.1).
pub const FUNNEL_HEADER: &str = "tailscale-funnel-request";   // in lib.rs
/// The only accepted marker value (exact bytes, after OWS trim).
pub const FUNNEL_HEADER_VALUE: &str = "?1";                   // in lib.rs

/// Pure. Counts, over the whole header list (names compared ASCII-case-insensitively, exact
/// hyphenated names only, `_` never normalised to `-`):
///   L = occurrences of `Tailscale-User-Login`, F = occurrences of `Tailscale-Funnel-Request`.
/// Decision table, evaluated in this order:
///   1. L ≥ 1 and F ≥ 1                      → Err(AuthError::Ambiguous)
///   2. F == 0                               → `authorize(headers, allowed)` → Caller::Tailnet
///                                             (L == 0 ⇒ Missing, L ≥ 2 ⇒ Repeated, … unchanged)
///   3. F ≥ 2                                → Err(AuthError::Repeated)
///   4. F == 1, value (OWS-trimmed) != "?1"  → Err(AuthError::Malformed)
///   5. F == 1, value "?1", !allow_funnel    → Err(AuthError::FunnelDisabled)
///   6. F == 1, value "?1", allow_funnel     → Ok(Caller::Funnel)
///
/// # Errors
/// [`AuthError`] (every variant → `403 {"result":"forbidden"}`).
pub fn classify_request(
    headers: &[(&str, &[u8])],
    allowed: &[TailscaleLogin],
    allow_funnel: bool,
) -> Result<Caller, AuthError>;
```

`AuthError` gains two variants with fixed texts: `Ambiguous` ("identity and funnel markers both present"),
`FunnelDisabled` ("funnel access disabled"). Existing variants and texts unchanged.

### 2.2 Why a forged identity on the Funnel path is defeated

1. `tailscaled` (v1.102.4, `ipn/ipnlocal/serve.go:1078-1107`) **deletes** every client copy of
   `Tailscale-User-Login` (canonical key, so every case variant and every repetition) and of
   `Tailscale-Funnel-Request` on every proxied request, *then* sets `Tailscale-Funnel-Request: ?1` for a
   Funnel connection and returns before the identity branch. A Funnel request therefore never reaches the
   socket with a login header.
2. If both markers ever arrive together (proxy bug, foreign proxy), rule 1 refuses (`Ambiguous`).
3. A tailnet client that forges `Tailscale-Funnel-Request: ?1` has it deleted by `tailscaled`; it arrives
   with its real identity only (rule 2).
4. Spelling variants (`Tailscale_User_Login`) pass through `tailscaled` but are never matched by the
   service (exact hyphenated name). Tested.
5. Tagged devices and local direct clients carry neither marker ⇒ rule 2 ⇒ `Missing` ⇒ `403`, exactly as
   today (every existing "no identity → 403" test stays valid).
6. If a future Tailscale release stops sending the (undocumented) marker, Funnel requests carry neither
   header ⇒ `403` (fail-closed, never anonymous access).
7. Defence in depth: even a successful identity forgery would no longer unlock anything (D-C needs a
   passkey assertion) and could not register a passkey (D-E needs the local one-time code).
8. `Host`, `X-Forwarded-Host`, `X-Forwarded-For` are never an authorization input. On the Funnel path
   `X-Forwarded-Host` is the client's `Host` (attacker-chosen for a non-browser client): it only selects
   the `421` misdirection answer; the cryptographic origin binding is `clientDataJSON.origin` +
   `rpIdHash`, which the attacker cannot forge without the passkey. *(rev 2, F-2)* `X-Forwarded-For` is
   read only by `client_hint` (§2.4) to pick a rate-limit bucket for anonymous Funnel callers.
9. *(rev 2, F-10)* **Residual local threat (accepted, no escalation)**: a process running as the owner's
   uid can connect to the `0600` socket directly, forge `Tailscale-User-Login`, read the `enroll-code`
   file (or run `soos-remote enroll-code`) and register a software authenticator (attestation `none`).
   This grants nothing new: such a process can already call logind `UnlockSession` on its own session and
   read `remote.toml`; it is the same boundary as the 2026-10-05 ADR (the socket trusts every same-uid
   process and `tailscaled`).

### 2.4 Client hint (rev 2, F-2; pure, `identity.rs`)

```rust
/// Rate-limit bucket of an anonymous Funnel caller. NEVER an authorization input, never logged;
/// `Debug` prints "<redacted>". Holds the masked address bytes only (IPv4 mapped into 16 bytes,
/// IPv6 masked to its /64 prefix) or the shared unknown bucket.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClientHint([u8; 16]);

impl ClientHint {
    /// Shared bucket for a missing, repeated or malformed `X-Forwarded-For`.
    pub const UNKNOWN: ClientHint = ClientHint([0xff; 16]);
}

/// Lowercased `X-Forwarded-For` (in lib.rs).
pub const FORWARDED_FOR_HEADER: &str = "x-forwarded-for";

/// Pure. Exactly one `X-Forwarded-For` header whose OWS-trimmed value parses as one
/// `std::net::IpAddr` (no comma list, no port, no brackets, ≤ 45 bytes) ⇒ its hint; IPv4 and
/// IPv4-mapped IPv6 share the same hint; IPv6 keeps its first 8 bytes and zeroes the rest.
/// Anything else ⇒ `ClientHint::UNKNOWN`. Called only for `Caller::Funnel`.
#[must_use]
pub fn client_hint(headers: &[(&str, &[u8])]) -> ClientHint;
```

`tailscaled` deletes every client copy of `X-Forwarded-For` and sets exactly one value, the public
client address reported by its ingress relay (`research_funnel.md` §2.2/§2.3), so a Funnel client
cannot choose another client's bucket with a header; it can only change buckets by changing its
public address (residual, ADR accepted risks).

### 2.3 Funnel host rule

After `check_host` (unchanged) and `classify_request`, a `Caller::Funnel` request whose normalized
effective host is not exactly `config.auth.rp_id` ⇒ `421 {"result":"misdirected_request"}` (no other
work). `allow_funnel = true` requires `rp_id` (config error otherwise), so the comparison always has a
value.

---

## 3. Constants, configuration, dependencies

### 3.1 Constants (`crates/remote/src/lib.rs`, the only definition of each)

| Name | Type | Value | Meaning / behaviour at the bound |
|---|---|---|---|
| `FUNNEL_HEADER` | `&str` | `"tailscale-funnel-request"` | §2.1 |
| `FUNNEL_HEADER_VALUE` | `&str` | `"?1"` | §2.1 |
| `MAX_AUTH_BODY_BYTES` | `usize` | `8192` | Largest body on a body route: a `Content-Length` above it ⇒ `413 body_too_large` before reading a byte; a chunked body whose decoded size would exceed it ⇒ `413 body_too_large` at that chunk (rev 2, F-9) |
| `BODY_READ_TIMEOUT_MS` | `u64` | `5000` | Deadline for reading a declared or chunked body, from head completion; exceeded ⇒ connection closed without response |
| `MAX_BODY_CHUNKS` *(rev 2, F-9)* | `usize` | `64` | Data chunks of a chunked body (the terminating `0` chunk not counted); the 65th ⇒ `400 bad_request` |
| `MAX_CHUNK_SIZE_DIGITS` *(rev 2, F-9)* | `usize` | `8` | Hex digits of one chunk-size line (1..=8, no extension, no OWS); more or none ⇒ `400 bad_request` |
| `MAX_FUNNEL_CONNECTIONS` *(rev 2, F-2)* | `usize` | `8` | Classified Funnel requests in flight (permit taken right after classification, held to connection end); the 9th ⇒ `503 busy`. `MAX_CONNECTIONS − MAX_FUNNEL_CONNECTIONS = 8` slots stay for tailnet callers |
| `MAX_ANONYMOUS_FUNNEL_CONNECTIONS` *(rev 2, F-2)* | `usize` | `4` | Funnel requests in flight **without** a valid web session (subset of the above); the 5th ⇒ `503 busy`; leaves ≥ 4 Funnel slots for a logged-in owner |
| `MAX_ANONYMOUS_BODY_READS` *(rev 2, F-2)* | `usize` | `2` | Concurrent body reads by anonymous Funnel callers (only `POST /api/auth/login/verify`); the 3rd ⇒ `503 busy` before reading a byte |
| `MAX_ANONYMOUS_BODY_READS_PER_HINT` *(rev 2, F-2)* | `usize` | `1` | Same, per `ClientHint`; a second concurrent one from the same hint ⇒ `503 busy` |
| `MAX_FUNNEL_SSE_STREAMS` *(rev 2, F-2)* | `usize` | `2` | Open SSE streams of Funnel session holders (subset of `MAX_SSE_STREAMS = 4`); the 3rd ⇒ `503` exactly like the existing stream cap; tailnet keeps ≥ 2 streams |
| `MAX_CLIENT_HINTS` *(rev 2, F-2)* | `usize` | `64` | Anonymous limiter buckets kept in memory (options window, failure window, pending login challenges and body reads per hint). When full after purging ended windows, the bucket with the oldest window start and no body read in flight is evicted |
| `MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES` *(rev 2, F-2)* | `usize` | `16` | Pool `(Funnel, Login)`; full after purging expired ⇒ the **oldest** entry is evicted (never refuses) |
| `MAX_LOGIN_CHALLENGES_PER_HINT` *(rev 2, F-2)* | `usize` | `2` | Pending login challenges per `ClientHint`; a 3rd issuance evicts that hint's oldest entry (never another hint's) |
| `MAX_CLIENT_DATA_JSON_BYTES` | `usize` | `1024` | Decoded `clientDataJSON`; larger ⇒ `400 bad_request` |
| `MAX_ATTESTATION_OBJECT_BYTES` | `usize` | `2048` | Decoded `attestationObject`; larger ⇒ `400` |
| `ASSERTION_AUTH_DATA_LEN` | `usize` | `37` | Exact assertion `authenticatorData` length (AT and ED clear); other ⇒ `403 passkey_rejected` |
| `MAX_SIGNATURE_BYTES` | `usize` | `72` | DER ECDSA signature; larger ⇒ `400` |
| `MAX_CREDENTIAL_ID_BYTES` | `usize` | `1023` | L3 §6.5.1; larger ⇒ `400` (decode) / `passkey_rejected` (authData) |
| `USER_HANDLE_BYTES` | `usize` | `16` | Exact random user handle; other length ⇒ `passkey_rejected` |
| `CHALLENGE_BYTES` | `usize` | `32` | Random challenge; a decoded `clientData.challenge` of another length ⇒ `passkey_rejected` |
| `CHALLENGE_TTL_MS` | `u64` | `120_000` | Challenge lifetime (tokio `Instant`); expired ⇒ removed, `passkey_rejected` |
| `WEBAUTHN_TIMEOUT_MS` | `u64` | `120_000` | `timeout` sent to the client; must equal `CHALLENGE_TTL_MS` (const assert) |
| `MAX_PENDING_CHALLENGES` | `usize` | `4` | Per authenticated pool — `(Tailnet, Register)`, `(Tailnet, Unlock)`, `(Funnel, Unlock)` (session holders only); full after purging expired ⇒ `429 too_many_challenges`, **never evicts**. *(rev 2, F-2)* The anonymous `(Funnel, Login)` pool uses `MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES` instead |
| `MAX_PASSKEYS` | `usize` | `4` | Records in the store; register options at the bound ⇒ `409 passkey_limit`; a file with more ⇒ `StoreError::Malformed` |
| `MAX_CREDENTIAL_STORE_BYTES` | `usize` | `16_384` | Store file; larger ⇒ `StoreError::TooLarge` |
| `CREDENTIALS_FILE_NAME` | `&str` | `"remote-passkeys.json"` | Default store file name (sibling of `remote.toml`) |
| `MAX_CREDENTIALS_PATH_LEN` | `usize` | `4096` | Explicit `credentials_path` bound |
| `MAX_WEB_SESSIONS` | `usize` | `4` | Live sessions; full after purging expired ⇒ `429 too_many_sessions` (never evicts) |
| `WEB_SESSION_IDLE_MS` | `u64` | `900_000` | Idle TTL (15 min) |
| `WEB_SESSION_ABSOLUTE_MS` | `u64` | `28_800_000` | Absolute TTL (8 h); also the cookie `Max-Age` (28 800 s) |
| `SESSION_TOKEN_BYTES` | `usize` | `32` | Random token; cookie value is its 43-char base64url (no padding) |
| `SESSION_COOKIE_NAME` | `&str` | `"__Host-soos_session"` | Cookie name |
| `SESSION_COOKIE_ATTRIBUTES` | `&str` | `"Path=/; Secure; HttpOnly; SameSite=Strict"` | Never a `Domain` attribute |
| `MAX_AUTH_FAILURES` | `u32` | `5` | Failed verifications (assertion, attestation, enrollment code, malformed body) per `LimitKey` per window *(rev 2, F-2: was per `PathClass`)*; reaching it ⇒ every ceremony route using that key answers `429 rate_limited` until the window ends |
| `AUTH_FAILURE_WINDOW_MS` | `u64` | `300_000` | Fixed window started by the first failure |
| `MAX_OPTIONS_PER_WINDOW` | `u32` | `10` | Challenge issuances per `LimitKey` per window *(rev 2, F-2)*; more ⇒ `429 rate_limited` |
| `OPTIONS_WINDOW_MS` | `u64` | `60_000` | Fixed window started by the first issuance |
| `ENROLL_CODE_LEN` | `usize` | `10` | Symbols (50 bits); displayed `XXXXX-XXXXX` |
| `ENROLL_CODE_ALPHABET` | `&[u8; 32]` | `b"0123456789ABCDEFGHJKMNPQRSTVWXYZ"` | Crockford base32, no `I L O U` |
| `ENROLL_CODE_TTL_S` | `u64` | `300` | Code lifetime (Unix seconds, injected clock) |
| `MAX_ENROLL_CODE_ATTEMPTS` | `u32` | `3` | Wrong codes against one code file; reaching it deletes the file |
| `ENROLL_CODE_FILE_NAME` | `&str` | `"enroll-code"` | In the socket directory (`0700`) |
| `MAX_ENROLL_CODE_FILE_BYTES` | `usize` | `256` | Larger ⇒ treated as absent (and removed) |
| `STORE_LOCK_TIMEOUT_MS` | `u64` | `500` | Bounded `flock` acquisition (non-blocking attempts every 25 ms); exceeded ⇒ `StoreError::Busy` ⇒ `503 store_unavailable` |
| `COSE_ALG_ES256` | `i64` | `-7` | Only accepted algorithm |
| `RP_NAME` | `&str` | `"soos"` | `rp.name` and `user.name`/`displayName` (never the login, D-H) |

Compile-time asserts: `WEBAUTHN_TIMEOUT_MS == CHALLENGE_TTL_MS`; `MAX_AUTH_BODY_BYTES >= 4 * MAX_ATTESTATION_OBJECT_BYTES / 3 + 4 * MAX_CREDENTIAL_ID_BYTES / 3 + 4 * MAX_CLIENT_DATA_JSON_BYTES / 3 + 256`
(5 715 ≤ 8 192); `MAX_SSE_STREAMS < MAX_CONNECTIONS` (existing). *(rev 2, F-2)*:
`MAX_ANONYMOUS_FUNNEL_CONNECTIONS < MAX_FUNNEL_CONNECTIONS`, `MAX_FUNNEL_CONNECTIONS < MAX_CONNECTIONS`,
`MAX_CONNECTIONS - MAX_FUNNEL_CONNECTIONS >= 8`, `MAX_ANONYMOUS_BODY_READS < MAX_ANONYMOUS_FUNNEL_CONNECTIONS`,
`MAX_ANONYMOUS_BODY_READS_PER_HINT <= MAX_ANONYMOUS_BODY_READS`, `MAX_FUNNEL_SSE_STREAMS < MAX_SSE_STREAMS`,
`MAX_LOGIN_CHALLENGES_PER_HINT < MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES`.

New result string *(rev 2)*: `busy` (`503`, class capacity reached; §5.3).

Route action values (`X-Soos-Action`, exact bytes, constants in `lib.rs`):
`ACTION_LOCK = "lock"`, `ACTION_UNLOCK = "unlock"` (existing), `ACTION_UNLOCK_OPTIONS = "unlock-options"`,
`ACTION_LOGIN_OPTIONS = "login-options"`, `ACTION_LOGIN = "login"`, `ACTION_LOGOUT = "logout"`,
`ACTION_REGISTER_OPTIONS = "register-options"`, `ACTION_REGISTER = "register"`.

### 3.2 Configuration (`remote.toml`)

```rust
/// Passkey and Funnel settings (ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey
/// Authentication"). `Default` = everything off.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AuthConfig {
    /// WebAuthn RP ID = the full node host; `None` disables every passkey route and makes
    /// every unlock impossible (`403 passkeys_not_configured`).
    pub rp_id: Option<String>,
    /// Accept `Tailscale-Funnel-Request: ?1` requests (S-2). Requires `rp_id`.
    pub allow_funnel: bool,
    /// Explicit credential store path; `None` ⇒ resolved by `resolve_credentials_path`.
    pub credentials_path: Option<PathBuf>,
}

pub struct RemoteConfig {
    // … the five existing fields, unchanged …
    /// Passkey / Funnel settings.
    pub auth: AuthConfig,
}
```

| Key | Default | Validation (fail-closed, never clamped) | Error |
|---|---|---|---|
| `rp_id` | absent ⇒ `None` | lowercased; `is_valid_host_name`; no port, no scheme; if `allowed_hosts` is non-empty it must be a member; otherwise it must end with `.ts.net` and have **at least two** labels before it (`<node>.<tailnet>.ts.net`; `tailnet.ts.net` alone is refused as too broad). Empty string ⇒ error, never `None`. | `ConfigError::InvalidRpId` ("rp_id must be the full node host name") |
| `allow_funnel` | `false` | TOML boolean only; `true` without `rp_id` ⇒ error | `Syntax` / `ConfigError::FunnelNeedsRpId` ("allow_funnel requires rp_id") |
| `credentials_path` | absent ⇒ `None` | absolute, no trailing `/`, has a parent and file name, ≤ `MAX_CREDENTIALS_PATH_LEN` bytes, valid UTF-8 | `ConfigError::InvalidCredentialsPath { max }` ("credentials_path must be absolute and at most {max} bytes") |

`allow_unlock = true` **without** `rp_id` stays a valid configuration (keeps
`test_rmc_parse_config_allow_unlock_is_opt_in` valid); `main.rs` logs one `warn` line at start-up
("allow_unlock is set but rp_id is not: every remote unlock is refused") and every unlock answers
`403 passkeys_not_configured`.

```rust
/// Pure. `explicit` wins; otherwise `<config_path parent>/CREDENTIALS_FILE_NAME`.
/// # Errors
/// `ConfigError::InvalidCredentialsPath` when `config_path` has no parent or the result
/// exceeds `MAX_CREDENTIALS_PATH_LEN`.
pub fn resolve_credentials_path(auth: &AuthConfig, config_path: &Path) -> Result<PathBuf, ConfigError>;
```

The resolved path is stored in `ServerState` (not in `RemoteConfig`), so `parse_config` keeps its
signature.

### 3.3 Sentinel semantics

| Value | Meaning |
|---|---|
| `rp_id` absent | passkeys off: auth routes `403 passkeys_not_configured`, unlock impossible, Funnel impossible |
| credential store file missing | zero passkeys (not an error); first successful registration creates it with a fresh user handle |
| store file empty (0 bytes), wrong owner, mode & `0o077 != 0`, symlink, not a regular file, > bound, bad JSON, unknown key, version ≠ 1, > `MAX_PASSKEYS` records, invalid record | `StoreError` ⇒ `503 store_unavailable` on every passkey route; existing web sessions are invalidated (fail-closed); never treated as "zero passkeys" |
| enrollment code file missing / expired / malformed / insecure | no valid code ⇒ `403 enroll_code_rejected` (counts as a failure); an expired or malformed file is removed |
| `signCount` 0 stored and 0 received | accepted (synced passkeys) |
| `Content-Length` absent on a body route | without `Transfer-Encoding: chunked`: `BodyFraming::None` (empty body); the route then answers per §5 (`passkey_required` for unlock, `400 bad_request` elsewhere). *(rev 2, F-9)* A chunked body whose first chunk is the terminating `0` chunk is also an empty body (same answers) |
| `X-Forwarded-For` absent, repeated or malformed on a Funnel request *(rev 2)* | `ClientHint::UNKNOWN` (one shared bucket); never a refusal by itself |
| Unix clock `0` / before the code's creation | a code with `expires_unix_s <= now` is expired; a code with `expires_unix_s > now + ENROLL_CODE_TTL_S` is malformed (future-dated) |
| RNG failure (`getrandom` error) | `503 unavailable`; no challenge, token or code is produced; nothing is stored |

### 3.4 Dependencies (D-G)

Root `Cargo.toml` `[workspace.dependencies]` additions:

```toml
p256 = { version = "0.13.2", default-features = false, features = ["ecdsa"] }
base64ct = { version = "1.8", features = ["alloc"] }
subtle = "2.6"
```

`crates/remote/Cargo.toml` `[dependencies]` additions (all `{ workspace = true }`): `p256`, `sha2`,
`ciborium`, `getrandom`, `base64ct`, `subtle`, `zeroize`. `[dev-dependencies]` gain nothing new beyond
what the tests need (`p256` signing comes from the same `ecdsa` feature; `ciborium` is already a normal
dependency).

`deny.toml` `[bans] skip` gains exactly:
`{ crate = "der@0.7.10", reason = "Used by ecdsa 0.16 and sec1 0.7 (p256 0.13 via soos-remote) while ureq 3 (build dependency of ort-sys) uses der 0.8" }`
(direct dependents as printed by `cargo tree -i der@0.7.10 -e normal,build,dev --depth 1`; the
developer re-checks the names against `dependency_tooling_contract`). Forbidden in `soos-remote`'s
normal dependency tree, compared as **exact crate names** *(rev 2, F-3)*: `openssl`, `openssl-sys`,
`native-tls`, `ring`, `aws-lc-rs`, `aws-lc-sys`, `rand`, and every name starting with `webauthn-rs`
(prefix match for that family only). `rand_core` is **allowed** (it comes with
`p256 → elliptic-curve 0.13` and is already in the lock through `aes-gcm`); the CSPRNG is still
`getrandom::fill` only (RMC-S16). `cargo deny --locked check` must stay
`advisories ok, bans ok, licenses ok, sources ok`. Accepted risk: the `p256` arithmetic is unaudited; the
service only verifies public signatures (no secret scalar on the server).

---

## 4. Types & signatures

All new types: no `unwrap`/`expect`/panic/indexing/unchecked arithmetic (workspace lints); every secret
buffer (`token`, enrollment code, challenge copies held by tests aside) is `Zeroizing<…>` or
`ZeroizeOnDrop`; `Debug` of any type holding a token, challenge, credential id, public key, user handle,
code or login is redacted (`"<redacted>"`).

### 4.1 `webauthn.rs` (pure, no I/O, no clock)

```rust
/// Relying-party constants derived once from `rp_id`.
#[derive(Clone)]
pub struct RelyingParty {
    rp_id: String,            // validated host (config)
    origin: String,           // exactly "https://" + rp_id (no port: S-1)
    rp_id_hash: [u8; 32],     // SHA-256(rp_id)
}
impl RelyingParty {
    #[must_use] pub fn new(rp_id: &str) -> Self;
    #[must_use] pub fn rp_id(&self) -> &str;
    #[must_use] pub fn origin(&self) -> &str;
}

/// Expected ceremony type of `clientDataJSON.type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeremonyType { Create /* "webauthn.create" */, Get /* "webauthn.get" */ }

/// `clientDataJSON` after §7.1 steps 5–9 / §7.2 steps 10–14 except the challenge lookup.
/// Unknown JSON keys are ignored (L3 §5.8.1); duplicate known keys are refused (serde_json).
pub struct ClientData {
    /// Decoded challenge, exactly `CHALLENGE_BYTES`.
    pub challenge: [u8; CHALLENGE_BYTES],
}

/// Pure. `raw` ≤ `MAX_CLIENT_DATA_JSON_BYTES`, UTF-8 JSON object with string `type`,
/// `challenge`, `origin`; `type` must equal `expected`; `origin` must equal `rp.origin()`
/// byte for byte; `crossOrigin` absent or `false`; `topOrigin` absent; `challenge` is
/// base64url **without padding** decoding to exactly `CHALLENGE_BYTES`.
/// # Errors  WebAuthnError::{ClientDataMalformed, WrongType, OriginMismatch, CrossOrigin}
pub fn parse_client_data(rp: &RelyingParty, raw: &[u8], expected: CeremonyType)
    -> Result<ClientData, WebAuthnError>;

/// Flags byte (L3 §6.1).
pub mod flags { pub const UP: u8 = 0x01; pub const UV: u8 = 0x04; pub const BE: u8 = 0x08;
                pub const BS: u8 = 0x10; pub const AT: u8 = 0x40; pub const ED: u8 = 0x80; }

/// A verified new credential (registration result). `public_key` is the 65-byte SEC1
/// uncompressed point, already validated on-curve and not the identity.
pub struct NewCredential {
    pub credential_id: Vec<u8>,      // 1..=MAX_CREDENTIAL_ID_BYTES
    pub public_key: [u8; 65],
    pub sign_count: u32,
    pub backup_eligible: bool,
    pub backup_state: bool,
}

/// Pure; L3 §7.1 subset (research_webauthn §1.2 steps 1–12). Inputs are the decoded bytes
/// of the browser response. Checks, in order: attestationObject ≤ bound, CBOR map with
/// exactly the text keys `fmt`, `attStmt`, `authData` and no trailing bytes; `fmt == "none"`;
/// `attStmt` an empty map; authData ≥ 37 + 16 + 2 bytes; `rpIdHash == rp.rp_id_hash`; UP set;
/// UV set; BE clear ⇒ BS clear; AT set; ED clear; credentialIdLength 1..=1023 and within
/// bounds; COSE key = CBOR map with exactly the five integer keys {1:2, 3:-7, -1:1, -2:bstr32,
/// -3:bstr32} (no duplicate, no extra key, no compressed point); no byte after the COSE key;
/// point on curve (`p256::ecdsa::VerifyingKey::from_encoded_point`); `raw_id == credentialId`.
/// The client data (challenge, type, origin) is checked by the caller with `parse_client_data`
/// BEFORE this call (the challenge is consumed first).
/// # Errors  every registration `WebAuthnError` variant.
pub fn verify_registration(rp: &RelyingParty, raw_id: &[u8], attestation_object: &[u8])
    -> Result<NewCredential, WebAuthnError>;

/// The stored record view the assertion check needs.
pub struct StoredCredential<'a> {
    pub credential_id: &'a [u8],
    pub public_key: &'a [u8; 65],
    pub sign_count: u32,
    pub backup_eligible: bool,
}

/// Successful assertion: values to persist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssertionOutcome { pub sign_count: u32, pub backup_state: bool }

/// Pure; L3 §7.2 subset (research_webauthn §2.2). `client_data_raw` is the exact received
/// `clientDataJSON` (already validated by `parse_client_data` with `CeremonyType::Get` and
/// its challenge consumed by the caller). Checks, in order: `user_handle` present and equal
/// to `stored_user_handle` (constant time); authData length exactly `ASSERTION_AUTH_DATA_LEN`;
/// `rpIdHash`; UP; UV; AT clear; ED clear; BE clear ⇒ BS clear; BE == stored.backup_eligible;
/// signature ≤ `MAX_SIGNATURE_BYTES`, DER (`Signature::from_der`), verified with ES256 over
/// `authenticator_data || SHA-256(client_data_raw)` (high-S accepted, never normalised);
/// signCount rule: if `received != 0 || stored != 0` then `received > stored` else
/// `SignCountRegression`.
/// # Errors  every assertion `WebAuthnError` variant.
pub fn verify_assertion(
    rp: &RelyingParty,
    credential: &StoredCredential<'_>,
    stored_user_handle: &[u8; USER_HANDLE_BYTES],
    client_data_raw: &[u8],
    authenticator_data: &[u8],
    signature: &[u8],
    user_handle: Option<&[u8]>,
) -> Result<AssertionOutcome, WebAuthnError>;

/// Verification failure. Every variant → HTTP `403 {"result":"passkey_rejected"}` (except the
/// size variants mapped to `400` by the body decoder, §5.3). Display texts are fixed English
/// reason classes; no value is ever formatted into them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WebAuthnError {
    #[error("client data malformed")]            ClientDataMalformed,
    #[error("wrong ceremony type")]              WrongType,
    #[error("origin mismatch")]                  OriginMismatch,
    #[error("cross-origin ceremony")]            CrossOrigin,
    #[error("attestation object malformed")]     AttestationMalformed,
    #[error("unsupported attestation format")]   UnsupportedAttestation,
    #[error("authenticator data malformed")]     AuthDataMalformed,
    #[error("rp id hash mismatch")]              RpIdHashMismatch,
    #[error("user presence missing")]            UserPresenceMissing,
    #[error("user verification missing")]        UserVerificationMissing,
    #[error("backup flags invalid")]             BackupFlagsInvalid,
    #[error("unexpected extensions")]            UnexpectedExtensions,
    #[error("credential id invalid")]            CredentialIdInvalid,
    #[error("credential id mismatch")]           CredentialIdMismatch,
    #[error("public key malformed")]             PublicKeyMalformed,
    #[error("unsupported algorithm")]            UnsupportedAlgorithm,
    #[error("signature malformed")]              SignatureMalformed,
    #[error("signature invalid")]                SignatureInvalid,
    #[error("user handle mismatch")]             UserHandleMismatch,
    #[error("sign count regression")]            SignCountRegression,
    #[error("backup eligibility changed")]       BackupEligibilityChanged,
}
```

`b64url_decode(input: &str, max_decoded: usize) -> Result<Vec<u8>, ()>` and `b64url_encode(&[u8]) ->
String` helpers (base64ct `Base64UrlUnpadded`; the encoded length is checked against
`max_decoded * 4 / 3 + 4` **before** decoding; padding and non-alphabet bytes rejected) live in
`webauthn.rs` and are reused by `websession.rs`, `credentials.rs` and `auth.rs`.

### 4.2 `challenge.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChallengePurpose { Register, Login, Unlock }

/// What a challenge is bound to besides purpose and path.
#[derive(Clone, PartialEq, Eq)]
pub enum ChallengeBinding {
    /// No extra binding (tailnet unlock).
    None,
    /// Anonymous Funnel login, bucketed by client hint (rev 2, F-2). The hint is a pool key,
    /// not a security binding: `take` does NOT compare it (a phone may change address between
    /// options and verify).
    Login(ClientHint),
    /// SHA-256 of the web-session token (Funnel unlock).
    WebSession([u8; 32]),
    /// SHA-256 of the normalized enrollment code (registration).
    EnrollCode([u8; 32]),
}

/// Registration state carried by a `Register` challenge (rev 2, F-1). `Debug` redacted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PendingRegistration {
    /// The user handle sent as `user.id` in the options: the stored handle when the store
    /// existed at options time, otherwise a fresh random one (never persisted before verify).
    pub user_handle: [u8; USER_HANDLE_BYTES],
}

/// What a successful `take` hands back (rev 2, F-1).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Taken {
    /// Login or unlock challenge.
    Plain,
    /// Register challenge with its pending user handle.
    Register(PendingRegistration),
}

pub struct ChallengeStore {
    /* Vec<Pending>; len ≤ 3 × MAX_PENDING_CHALLENGES + MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES = 28 */
}

impl ChallengeStore {
    #[must_use] pub fn new() -> Self;
    /// Purges expired entries of the pool, then stores `bytes` with `expires = now + CHALLENGE_TTL_MS`.
    /// `pending` must be `Some` exactly when `purpose == Register` (else `Err(Mismatch)`, nothing stored).
    /// Authenticated pools (`(Tailnet, Register)`, `(Tailnet, Unlock)`, `(Funnel, Unlock)`): full ⇒
    /// `Err(PoolFull)`, never evicts. Anonymous pool `(Funnel, Login)` (binding `Login(hint)`)
    /// (rev 2, F-2): if the hint already holds `MAX_LOGIN_CHALLENGES_PER_HINT` entries its oldest is
    /// removed; then, if the pool holds `MAX_PENDING_ANONYMOUS_LOGIN_CHALLENGES`, the oldest entry of
    /// the pool is removed; then the new entry is stored (this pool never answers `PoolFull`).
    /// Returns the number of entries evicted (test hook; 0..=2).
    /// # Errors  ChallengeError::{PoolFull, Mismatch}
    pub fn issue(&mut self, now: Instant, class: PathClass, purpose: ChallengePurpose,
                 binding: ChallengeBinding, pending: Option<PendingRegistration>,
                 bytes: [u8; CHALLENGE_BYTES]) -> Result<usize, ChallengeError>;
    /// Finds the entry whose bytes equal `bytes` (constant-time per entry, every entry
    /// compared), REMOVES it whatever the outcome, then checks expiry, class, purpose and
    /// binding (binding compared in constant time; for `Login(_)` only the variant is compared,
    /// never the hint). Returns `Taken::Register(pending)` for a register challenge.
    /// # Errors  ChallengeError::{Unknown, Expired, Mismatch}
    pub fn take(&mut self, now: Instant, class: PathClass, purpose: ChallengePurpose,
                binding: &ChallengeBinding, bytes: &[u8; CHALLENGE_BYTES]) -> Result<Taken, ChallengeError>;
    /// Live entries in a pool (test hook; purges nothing).
    #[must_use] pub fn pending(&self, class: PathClass, purpose: ChallengePurpose) -> usize;
    /// Live anonymous login entries of one hint (test hook; rev 2).
    #[must_use] pub fn pending_for_hint(&self, hint: ClientHint) -> usize;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ChallengeError {
    #[error("challenge pool full")] PoolFull,       // → 429 too_many_challenges
    #[error("challenge unknown")]   Unknown,        // → 403 passkey_rejected (failure counted)
    #[error("challenge expired")]   Expired,        // → 403 passkey_rejected (failure counted)
    #[error("challenge mismatch")]  Mismatch,       // → 403 passkey_rejected (failure counted)
}
```

### 4.3 `websession.rs`

```rust
pub struct WebSessionStore { /* ≤ MAX_WEB_SESSIONS records */ }

struct WebSessionRecord {           // private
    token_hash: [u8; 32],           // SHA-256(token); the token itself is never stored
    credential_hash: [u8; 32],      // SHA-256(credential id) that logged in (revocation, §4.4)
    created: Instant,
    last_used: Instant,
}

/// A freshly issued token (returned once, to build the cookie). Zeroized on drop; `Debug` redacted.
pub struct IssuedToken(Zeroizing<String>);   // 43-char base64url
impl IssuedToken { #[must_use] pub fn set_cookie_value(&self) -> String; }

impl WebSessionStore {
    #[must_use] pub fn new() -> Self;
    /// Purges expired records; full ⇒ Err(TooMany) (never evicts); else stores the hash.
    /// # Errors  WebSessionError::TooMany
    pub fn create(&mut self, now: Instant, token: [u8; SESSION_TOKEN_BYTES], credential_hash: [u8; 32])
        -> Result<IssuedToken, WebSessionError>;
    /// Valid ⇔ a record with an equal hash exists (constant-time compare over every record),
    /// `now - last_used < WEB_SESSION_IDLE_MS` and `now - created < WEB_SESSION_ABSOLUTE_MS`.
    /// A valid lookup returns the token hash and, only with `Touch::Refresh`, sets
    /// `last_used = now` (rev 2, F-4); an expired record is removed.
    #[must_use] pub fn validate(&mut self, now: Instant, token_hash: &[u8; 32], touch: Touch) -> Option<[u8; 32]>;
    /// Removes the record (logout); idempotent.
    pub fn remove(&mut self, token_hash: &[u8; 32]);
    /// Removes every record whose credential is not in `live` (passkey removal) or all of them
    /// when `live` is `None` (store unreadable).
    pub fn retain_credentials(&mut self, live: Option<&[[u8; 32]]>);
    #[must_use] pub fn len(&self) -> usize;
}

/// Pure. Scans every `cookie` header (several are allowed: HTTP/2 crumbs re-joined by the
/// proxy); splits on `;`, trims OWS; exactly one pair named `SESSION_COOKIE_NAME` across all
/// headers (zero or ≥ 2 ⇒ None); its value must be exactly 43 base64url chars decoding to
/// `SESSION_TOKEN_BYTES`; returns SHA-256(decoded token).
#[must_use] pub fn session_token_hash(headers: &[(&str, &[u8])]) -> Option<[u8; 32]>;

/// `"__Host-soos_session=<token>; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=28800"`.
#[must_use] pub fn set_cookie_header(token: &IssuedToken) -> String;
/// `"__Host-soos_session=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0"`.
#[must_use] pub fn clear_cookie_header() -> String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WebSessionError { #[error("too many web sessions")] TooMany }   // → 429 too_many_sessions

/// Whether a validation counts as activity (rev 2, F-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Touch {
    /// Every request-level validation (dispatch step 6): assets excluded (they never validate),
    /// `GET /api/auth/state`, status, the opening of `/api/events`, lock, unlock options, unlock,
    /// logout, login (rotation lookup).
    Refresh,
    /// SSE keep-alive re-validation only: an open page that nobody touches therefore reaches the
    /// 15 min idle expiry and its stream ends within one keep-alive tick.
    Keep,
}
```

Session lifecycle rules *(rev 2, F-6)*:
- **Revocation before validation.** Every Funnel validation (dispatch step 6 and every SSE keep-alive
  re-validation) first calls `store.live_credential_hashes()` (cached `load`, one `fstat` when the stamp
  is unchanged) and `sessions.retain_credentials(result.as_deref())`, then `validate`. A removed passkey
  therefore revokes its sessions at the next request or keep-alive tick; an unreadable store
  (`None`) drops every session.
- **Rotation on login.** `login_verify`, after a successful assertion and after drawing the new token
  from the RNG, removes the record of the presented cookie when that cookie is valid
  (`validate(…, Touch::Refresh)` then `remove`), then calls `create`. Repeated logins from the same
  browser therefore never accumulate records; a stale or foreign cookie is ignored (never an error).

### 4.4 `credentials.rs`

On-disk format (JSON, `deny_unknown_fields`, UTF-8, ≤ `MAX_CREDENTIAL_STORE_BYTES`):

```json
{"version":1,"user_handle":"<b64url 16 bytes>","passkeys":[
  {"credential_id":"<b64url ≤1023 B>","public_key":"<b64url 65 B SEC1>","sign_count":0,
   "backup_eligible":true,"backup_state":true,"created_unix_s":1790000000}]}
```

```rust
/// One stored passkey. `Debug` redacted (credential id and key are identifiers, D-H).
#[derive(Clone, PartialEq, Eq)]
pub struct PasskeyRecord {
    pub credential_id: Vec<u8>,
    pub public_key: [u8; 65],
    pub sign_count: u32,
    pub backup_eligible: bool,
    pub backup_state: bool,
    pub created_unix_s: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PasskeyFile {
    pub user_handle: [u8; USER_HANDLE_BYTES],
    pub passkeys: Vec<PasskeyRecord>,     // 0..=MAX_PASSKEYS, unique credential ids
}

/// Pure (de)serialisation; every record re-validated (key on curve, id length, unique ids).
/// # Errors  StoreError::{Malformed, TooLarge}
pub fn parse_store(bytes: &[u8]) -> Result<PasskeyFile, StoreError>;
#[must_use] pub fn encode_store(file: &PasskeyFile) -> Vec<u8>;

/// File I/O; `uid` is the service uid.
pub struct CredentialStore { path: PathBuf, uid: u32, cache: Option<(FileStamp, PasskeyFile)> }

/// (dev, ino, len, mtime_sec, mtime_nsec) of the last read; a different stamp ⇒ reload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp { /* private fields */ }

impl CredentialStore {
    #[must_use] pub fn new(path: PathBuf, uid: u32) -> Self;
    /// Reads (or returns the cached copy when the stamp is unchanged). Missing file ⇒ Ok(None).
    /// Open with O_RDONLY|O_NOFOLLOW|O_NONBLOCK|O_CLOEXEC; fstat: regular file, owner == uid,
    /// mode & 0o077 == 0, len ≤ bound; read through `take(bound + 1)`.
    /// # Errors  StoreError::{Io, Insecure, TooLarge, Malformed}
    pub fn load(&mut self) -> Result<Option<&PasskeyFile>, StoreError>;
    /// Read-modify-write under an exclusive `nix::fcntl::Flock` on `<path>.lock`
    /// (O_CREAT|O_NOFOLLOW|O_CLOEXEC, 0600, non-blocking attempts for ≤ STORE_LOCK_TIMEOUT_MS):
    /// load fresh from disk (cache ignored), apply `f`, then atomic write: temp file
    /// `<path>.tmp-<8 random hex>` created O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC mode 0600 in the
    /// same directory, write all, `sync_all`, `rename` over `path`, fsync the directory; on any
    /// failure the temp file is removed and the old file is untouched. The parent directory
    /// must exist, be owned by uid and not be group/world-writable (never created here).
    /// # Errors  StoreError::*
    pub fn update<T>(&mut self, random: &RandomSource,
                     f: impl FnOnce(Option<PasskeyFile>) -> Result<(PasskeyFile, T), StoreError>)
        -> Result<T, StoreError>;
    /// SHA-256 of every stored credential id (session revocation, §4.3); `None` on error.
    #[must_use] pub fn live_credential_hashes(&mut self) -> Option<Vec<[u8; 32]>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("credential store unreadable")]               Io,
    #[error("credential store permissions are insecure")]   Insecure,
    #[error("credential store too large")]                TooLarge,
    #[error("credential store malformed")]                Malformed,
    #[error("credential store busy")]                     Busy,
    #[error("credential store full")]                     Full,        // → 409 passkey_limit
    #[error("passkey already registered")]                Duplicate,   // → 409 already_registered
    #[error("no such passkey")]                           NoSuchPasskey, // CLI only
    #[error("registration conflicts with the stored user")] UserHandleConflict, // → 409 registration_conflict (rev 2, F-1)
}
```

User handle lifecycle *(rev 2, F-1)*: the handle is written once, by the first successful
registration, and never changes afterwards. `passkeys remove` of the last record keeps the file with
`"passkeys":[]` and the same handle (`parse_store` already accepts 0..=`MAX_PASSKEYS` records); only
deleting the file by hand resets it. Inside `register_verify`'s `update` closure: file absent ⇒ create
`PasskeyFile { user_handle: pending.user_handle, passkeys: vec![new] }`; file present ⇒ its
`user_handle` must equal `pending.user_handle` (`subtle::ConstantTimeEq`), else
`Err(StoreError::UserHandleConflict)` and nothing is written.

Blocking file I/O inside the current-thread runtime is accepted because every file is bounded
(≤ 16 KiB) and local; the lock wait is never blocking (non-blocking attempts with `tokio::time::sleep`
between them in the server, `std::thread::sleep` in the CLI).

### 4.5 `enroll.rs`

```rust
/// A plaintext enrollment code (CLI output only). Zeroized on drop; `Debug` redacted.
pub struct EnrollCode(Zeroizing<String>);   // ENROLL_CODE_LEN symbols of ENROLL_CODE_ALPHABET
impl EnrollCode {
    /// Uniform symbols by rejection-free mapping (each random byte & 31 indexes the 32-symbol alphabet).
    /// # Errors  RandomError
    pub fn generate(random: &RandomSource) -> Result<Self, RandomError>;
    /// "XXXXX-XXXXX".
    #[must_use] pub fn display(&self) -> String;
    #[must_use] pub fn hash(&self) -> [u8; 32];
}

/// Pure. Uppercases, removes `-` and ASCII spaces; must then be exactly ENROLL_CODE_LEN symbols
/// of the alphabet; returns SHA-256 of the normalized code. `input` > 32 bytes ⇒ None.
#[must_use] pub fn normalized_code_hash(input: &str) -> Option<[u8; 32]>;

/// File line: "soos-remote-enroll-v1 <64 lowercase hex SHA-256> <expires_unix_s>\n".
#[derive(Clone, Copy, PartialEq, Eq)]   // Debug redacted
pub struct CodeFile { pub code_hash: [u8; 32], pub expires_unix_s: u64 }

#[must_use] pub fn encode_code_file(file: &CodeFile) -> String;
/// # Errors  EnrollError::Malformed
pub fn parse_code_file(bytes: &[u8]) -> Result<CodeFile, EnrollError>;

/// Writes `<socket_dir>/ENROLL_CODE_FILE_NAME` atomically (temp + rename, 0600, O_NOFOLLOW),
/// replacing any previous code. `socket_dir` must already pass `prepare_socket_dir`.
/// # Errors  EnrollError::Io
pub fn write_code_file(socket_dir: &Path, file: &CodeFile, random: &RandomSource) -> Result<(), EnrollError>;
/// Reads with the store's file rules (regular, owner uid, mode & 0o077 == 0, ≤ bound,
/// O_NOFOLLOW|O_NONBLOCK). Missing ⇒ Ok(None).
/// # Errors  EnrollError::{Io, Insecure, Malformed}
pub fn read_code_file(socket_dir: &Path, uid: u32) -> Result<Option<CodeFile>, EnrollError>;
/// Removes the file if it is a regular file (never follows a symlink); idempotent.
pub fn remove_code_file(socket_dir: &Path);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EnrollError {
    #[error("enrollment code file unreadable")]          Io,
    #[error("enrollment code file permissions are insecure")] Insecure,
    #[error("enrollment code file malformed")]           Malformed,
}
```

### 4.6 `auth.rs`

```rust
/// CSPRNG seam. Production: `getrandom::fill`. Tests inject a failing or scripted source.
pub type RandomSource = Arc<dyn Fn(&mut [u8]) -> Result<(), RandomError> + Send + Sync>;
#[must_use] pub fn system_random() -> RandomSource;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RandomError { #[error("random source failed")] Failed }      // → 503 unavailable

/// Fixed-window counter used by both limiters.
pub struct Window { started: Option<Instant>, count: u32 }

/// Limiter key (rev 2, F-2; replaces the per-`PathClass` key of round 1).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]   // Debug redacted (the hint is an address prefix)
pub enum LimitKey {
    /// Tailnet caller (register, unlock options, unlock).
    Tailnet,
    /// Funnel caller holding a valid web session (unlock options, unlock). All sessions belong
    /// to the owner's passkeys, so they share one key.
    FunnelSession,
    /// Anonymous Funnel caller (login options, login verify), one bucket per client hint.
    FunnelAnonymous(ClientHint),
}

/// Per-hint anonymous limiter windows (rev 2, F-2).
struct HintBucket { options: Window, failures: Window }

/// Every mutable auth structure, behind one `tokio::sync::Mutex` in `Shared`.
/// Never held across a logind call nor across a body read.
pub struct AuthState {
    pub challenges: ChallengeStore,
    pub sessions: WebSessionStore,
    pub store: CredentialStore,
    /// `Tailnet` and `FunnelSession` windows (exactly 2 keys).
    options_windows: HashMap<LimitKey, Window>,
    failure_windows: HashMap<LimitKey, Window>,
    /// Anonymous buckets, ≤ MAX_CLIENT_HINTS (eviction rule in §3.1; a bucket with a body read in
    /// flight, per `Capacity`, is never evicted).
    hints: HashMap<ClientHint, HintBucket>,
    /// Wrong-code count against the current code file, keyed by its code hash.
    code_attempts: Option<([u8; 32], u32)>,
}
```

Class capacity *(rev 2, F-2)*, kept **outside** the async auth mutex so that releasing a slot never
awaits:

```rust
/// In `Shared`: two extra semaphores and one small synchronous counter table.
pub struct Capacity {
    funnel: Arc<tokio::sync::Semaphore>,            // MAX_FUNNEL_CONNECTIONS permits
    anonymous: Arc<tokio::sync::Semaphore>,         // MAX_ANONYMOUS_FUNNEL_CONNECTIONS permits
    body_reads: Arc<std::sync::Mutex<BodyReads>>,   // never held across an await
    funnel_streams: AtomicUsize,                    // ≤ MAX_FUNNEL_SSE_STREAMS (same CAS pattern as MAX_SSE_STREAMS)
}
struct BodyReads { total: usize, per_hint: HashMap<ClientHint, usize> }   // ≤ MAX_ANONYMOUS_BODY_READS entries

impl Capacity {
    /// `try_acquire_owned` on `funnel`. # Errors CapacityError::Busy
    pub fn enter_funnel(&self) -> Result<OwnedSemaphorePermit, CapacityError>;
    /// `try_acquire_owned` on `anonymous`. # Errors CapacityError::Busy
    pub fn enter_anonymous(&self) -> Result<OwnedSemaphorePermit, CapacityError>;
    /// Global and per-hint caps; the guard decrements both on drop (a poisoned mutex is
    /// recovered with `into_inner`, never a panic). # Errors CapacityError::Busy
    pub fn reserve_anonymous_body_read(&self, hint: ClientHint) -> Result<BodyReadGuard, CapacityError>;
    /// Hints with a body read in flight (the eviction rule of `AuthState::hints` skips them).
    pub fn hints_in_flight(&self) -> Vec<ClientHint>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CapacityError { #[error("capacity reached")] Busy }   // → 503 busy, never counted as a failure
```

```rust
/// Outcome of every auth handler: a fully computed response (JSON body, optional Set-Cookie).
pub type AuthResponse = crate::http::Response;
```

Handler functions (async only where they sleep for the store lock; all bounded):
`auth_state_view`, `login_options`, `login_verify`, `logout`, `unlock_options`, `verify_unlock_assertion`,
`register_options`, `register_verify`. Their exact behaviour is §5.

Request bodies (serde, `deny_unknown_fields`, every string bounded by the field limits **before** decoding):

```rust
#[derive(Deserialize)] #[serde(deny_unknown_fields)]
pub struct RegisterOptionsBody { pub code: String }                       // ≤ 32 bytes
#[derive(Deserialize)] #[serde(deny_unknown_fields)]
pub struct RegisterVerifyBody {
    pub id: String,                  // b64url, ≤ MAX_CREDENTIAL_ID_BYTES decoded
    pub client_data_json: String,    // b64url, ≤ MAX_CLIENT_DATA_JSON_BYTES decoded
    pub attestation_object: String,  // b64url, ≤ MAX_ATTESTATION_OBJECT_BYTES decoded
}
#[derive(Deserialize)] #[serde(deny_unknown_fields)]
pub struct AssertionBody {           // login verify and unlock
    pub id: String,
    pub client_data_json: String,
    pub authenticator_data: String,  // b64url, ≤ 512 decoded (exact 37 enforced later)
    pub signature: String,           // b64url, ≤ MAX_SIGNATURE_BYTES decoded
    pub user_handle: Option<String>, // b64url, ≤ 64 decoded
}
```

A body that is not valid UTF-8 JSON of exactly this shape, or any field over its bound, or not
base64url-unpadded ⇒ `400 {"result":"bad_request"}` and **counts as an auth failure** of the caller's
`LimitKey` *(rev 2, F-2: an anonymous Funnel caller's malformed body counts only in its own
`FunnelAnonymous(hint)` bucket, so it can never lock a session holder, the tailnet, or another hint)*.

Option responses (`200 application/json`, every value server-computed; no login, no identity):

- login / unlock: `{"challenge":"<b64url>","rp_id":"<rp_id>","timeout_ms":120000}` (no `allowCredentials`, S-3)
- register: `{"challenge":"<b64url>","rp_id":"<rp_id>","timeout_ms":120000,"user_id":"<b64url user handle>","exclude_credentials":["<b64url>",…]}`
  (`user_id` is the stored handle, or, when the store file is absent, a fresh one drawn from the RNG;
  in both cases it is stored in the issued challenge as `PendingRegistration` *(rev 2, F-1)* and
  returned by `take` at verify time; tailnet + valid code only).
- `GET /api/auth/state`: `{"mode":"tailnet"|"funnel","authenticated":bool,"passkeys":bool,"unlock_enabled":bool,"enrollment":bool}`;
  for an unauthenticated Funnel caller only `{"mode":"funnel","authenticated":false}`.
  `passkeys` = store non-empty (false on store error), `enrollment` = tailnet and `rp_id` configured.

### 4.7 `routes.rs` additions

```rust
pub enum Route {
    // … existing variants …
    AuthState,            // GET|HEAD /api/auth/state
    LoginOptions,         // POST /api/auth/login/options
    LoginVerify,          // POST /api/auth/login/verify
    Logout,               // POST /api/auth/logout
    UnlockOptions,        // POST /api/auth/unlock/options
    RegisterOptions,      // POST /api/auth/register/options
    RegisterVerify,       // POST /api/auth/register/verify
}
/// Pure: true exactly for (POST, /api/unlock), (POST, /api/auth/login/verify),
/// (POST, /api/auth/register/options), (POST, /api/auth/register/verify) (query ignored).
#[must_use] pub fn accepts_body(method: Method, path: &str) -> bool;
/// Pure: routes reachable on the Funnel path without a web session: every `Route::Asset`,
/// `AuthState`, `LoginOptions`, `LoginVerify`, `NotFound`, `MethodNotAllowed`.
#[must_use] pub fn is_funnel_public(route: Route) -> bool;
/// Pure; the lock CSRF rules with the route's action AND `Origin` **required**: exactly one
/// `Origin` equal to `https://<rp_id>` (ASCII-lowercased; `:443` accepted); absent ⇒ OriginMismatch.
/// # Errors  CsrfError
pub fn check_auth_csrf(head: &RequestHead, rp_id: &str, action: &str) -> Result<(), CsrfError>;
```

`allow_header` returns `"POST"` for every new POST path and `"GET, HEAD"` for `/api/auth/state`.

### 4.8 `http.rs` additions

```rust
/// How the body of a body route is framed (rev 2, F-9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyFraming {
    /// No body (non-body route, or body route with neither `Content-Length` nor chunked).
    None,
    /// `Content-Length: n`, 1..=MAX_AUTH_BODY_BYTES (`Content-Length: 0` ⇒ `None`).
    Length(usize),
    /// `Transfer-Encoding: chunked`, decoded by `read_body` under the bounds of §3.1.
    Chunked,
}

/// A parsed request head and its body framing.
pub struct ParsedRequest {
    pub head: RequestHead,
    /// Bytes of the head (index of the first body byte in the buffer).
    pub head_len: usize,
    /// Never other than `None` when `accepts_body` was false.
    pub framing: BodyFraming,
}

impl ParsedRequest {
    /// True unless `framing == BodyFraming::None` (§5.1 step 5 "no assertion").
    #[must_use] pub fn has_body(&self) -> bool;
}

/// Pure. When `accepts_body(method, path)` is false: exactly `parse_request_head` (same
/// errors, same statuses — including the existing `413 body_not_allowed` / `400` for a body or
/// `Transfer-Encoding` on a non-body route — framing `None`). When true (rev 2, F-9):
///   - `Transfer-Encoding` present: exactly one such header, OWS-trimmed value equal to
///     `chunked` ASCII-case-insensitively, and **no** `Content-Length` ⇒ `Chunked`; any other
///     value, a list (`gzip, chunked`), several TE headers, or TE together with
///     `Content-Length` ⇒ `HttpError::Malformed` (400, request-smuggling guard);
///   - else every `Content-Length` must parse as u64 and all must be equal, else Malformed (400);
///     0 ⇒ `None`; a value > MAX_AUTH_BODY_BYTES ⇒ BodyTooLarge (413 `body_too_large`);
///   - else `None`.
/// # Errors  HttpError
pub fn parse_request(buf: &[u8], accepts_body: fn(Method, &str) -> bool) -> Result<ParsedRequest, HttpError>;

// HttpError gains:  #[error("request body too large")] BodyTooLarge,   // status() → Some(413)

/// Reads the body under one `BODY_READ_TIMEOUT_MS` deadline, `prefix` (bytes already received
/// after the head) first, then the stream; never reads past the end of the body.
///   - `Length(n)`: exactly `n` bytes (only the first `n` prefix bytes are used).
///   - `Chunked` (rev 2, F-9), strict RFC 9112 §7.1 subset: chunk-size = 1..=MAX_CHUNK_SIZE_DIGITS
///     hex digits (either case) immediately followed by CRLF (no chunk extension, no OWS, no bare
///     LF); chunk data followed by exactly CRLF; the last chunk is `0` CRLF followed by exactly CRLF
///     (any trailer field ⇒ Malformed); at most MAX_BODY_CHUNKS data chunks; the decoded total is
///     checked **before** reading a chunk's data: total + size > MAX_AUTH_BODY_BYTES ⇒ TooLarge.
///     Bytes read from the socket are bounded by MAX_AUTH_BODY_BYTES + MAX_BODY_CHUNKS ×
///     (MAX_CHUNK_SIZE_DIGITS + 4) + 5 (the framing overhead), read in ≤ 1024-byte steps.
/// # Errors  BodyError::{Timeout, Closed, Malformed, TooLarge}
pub async fn read_body(stream: &mut UnixStream, prefix: &[u8], framing: BodyFraming)
    -> Result<Zeroizing<Vec<u8>>, BodyError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BodyError {
    #[error("body read deadline")]   Timeout,     // connection closed, no response
    #[error("peer closed")]          Closed,      // connection closed, no response
    #[error("body framing invalid")] Malformed,   // 400 bad_request (counted like a malformed body)
    #[error("body too large")]       TooLarge,    // 413 body_too_large (not counted)
}
```

`parse_request_head` keeps its exact signature and behaviour (all `http_tests` stay valid).

*Evidence for S-13 (rev 2, F-9)*: `tailscaled` serves Funnel and tailnet HTTPS with Go `net/http`
(HTTP/2 negotiated by ALPN with Safari) and forwards to the socket over HTTP/1.1 with
`httputil.ReverseProxy` (`research_funnel.md` §1.1). Go's HTTP/2 server sets
`Request.ContentLength = -1` when the client sends no `content-length` and the stream is not ended
by the headers; `ReverseProxy` copies `ContentLength` to the outgoing request, and Go's
`http.Transport` writes a request with `ContentLength == -1` and a non-nil body with
`Transfer-Encoding: chunked` (lowercase hex sizes, no extensions, no trailers). Safari usually sends
`content-length` for a `fetch` with a string body, but HTTP/2 does not require it, so the service
must not depend on it. The decoder accepts exactly what Go writes and nothing looser. The owner
check RMC40 confirms a passkey POST succeeds over Funnel and over the tailnet.

---

## 5. Route behaviour (both paths)

`T` = tailnet caller, `F` = Funnel caller, `F+s` = Funnel caller with a valid web session.

| Route | Path rule | CSRF (`X-Soos-Action`) | Body | Behaviour |
|---|---|---|---|---|
| assets | T, F | — | none | unchanged |
| `GET /api/auth/state` | T, F | — | none | §4.6 shape; no logind read; validates a Funnel session if present with `Touch::Refresh` (rev 2, F-4) |
| `GET /api/status`, `GET /api/events` | T, F+s | — | none | unchanged; F without session ⇒ `403 login_required`; opening validates with `Touch::Refresh`; SSE on F+s re-validates (revocation first, §4.3) at every keep-alive tick with `Touch::Keep` (rev 2, F-4: an untouched open page expires after 15 min idle) and ends when it is invalid; stream end ≤ min(`MAX_SSE_STREAM_MS`, session absolute expiry); a Funnel stream also needs one of the `MAX_FUNNEL_SSE_STREAMS` slots (rev 2, F-2) |
| `POST /api/lock` | T, F+s | `lock` (existing rules) | none | unchanged `lock_flow` |
| `POST /api/auth/login/options` | F only (T ⇒ `403 forbidden`) | `login-options` + Origin required | none | rp_id, failure lockout and options limiter of `FunnelAnonymous(hint)` (rev 2, F-2; also when a cookie is presented: login is the anonymous ceremony), store non-empty (`409 no_passkey`), RNG, issue `(Funnel, Login, Login(hint))` (evicting, §4.2) |
| `POST /api/auth/login/verify` | F only | `login` + Origin required | `AssertionBody` | failure lockout of `FunnelAnonymous(hint)`; anonymous body-read reservation (`503 busy`, rev 2, F-2); read body; decode; `parse_client_data(Get)`; `take(Funnel, Login, Login(_))`; look up credential by `id` (unknown ⇒ `passkey_rejected`); `verify_assertion`; persist sign_count/backup_state only when changed (store `update`; failure ⇒ §5.1 step 9b rule, rev 2, F-7); RNG token; rotation: remove the presented valid session (rev 2, F-6); `sessions.create` (`429 too_many_sessions`); `200 {"result":"logged_in"}` + `Set-Cookie`; `info!("remote login accepted")` |
| `POST /api/auth/logout` | F (with or without session) ; T ⇒ `403 forbidden` | `logout` + Origin required | none | remove record if any; `200 {"result":"logged_out"}` + clearing `Set-Cookie` |
| `POST /api/auth/unlock/options` | T, F+s | `unlock-options` + Origin required | none | `allow_unlock` (`403 unlock_disabled`), rp_id, failure lockout and options limiter of `Tailnet` / `FunnelSession` (rev 2, F-2), store non-empty (`409 no_passkey`), RNG, issue `(class, Unlock, None | WebSession(hash))` |
| `POST /api/unlock` | T, F+s | `unlock` (existing `check_unlock_csrf`, Origin optional) | `AssertionBody` | §5.1 |
| `POST /api/auth/register/options` | T only (F ⇒ `403 forbidden` before any body read) | `register-options` + Origin required | `RegisterOptionsBody` | rp_id, failure lockout and options limiter of `Tailnet`, read code file (`403 enroll_code_rejected` + failure + attempt count; 3rd wrong code deletes the file), store `load` (`503 store_unavailable`), store full ⇒ `409 passkey_limit`, user handle = stored handle or (file absent) 16 RNG bytes (rev 2, F-1), RNG challenge, issue `(Tailnet, Register, EnrollCode(hash))` with `PendingRegistration { user_handle }` |
| `POST /api/auth/register/verify` | T only | `register` + Origin required | `RegisterVerifyBody` | failure lockout of `Tailnet`; decode; `parse_client_data(Create)`; re-read code file and require its hash (still unexpired) ⇒ binding; `take(Tailnet, Register, EnrollCode(hash))` ⇒ `Taken::Register(pending)`; `verify_registration`; store `update` (rev 2, F-1: file absent ⇒ create with `pending.user_handle`; file present with another handle ⇒ `409 registration_conflict`, nothing written, code file **kept**, not counted as a failure; duplicate ⇒ `409 already_registered`; full ⇒ `409 passkey_limit`); remove the code file; `200 {"result":"registered"}`; `info!("passkey registered")` |

Common rules for every `/api/auth/*` route and for `/api/unlock` with a body: `rp_id` must be
configured (else `403 passkeys_not_configured`) and the normalized effective host must equal `rp_id`
(else `421 misdirected_request`). Every refusal happens before any logind call; no auth route ever calls
logind.

### 5.1 `POST /api/unlock` (supersedes the identity-only unlock)

Order (every step refuses before the next one runs):

1. path rule (T, or F with a valid session, else `403 login_required`);
2. `check_unlock_csrf` ⇒ `403 forbidden` (unchanged, keeps `test_rmc_disabled_unlock_checks_csrf_first`);
3. `allow_unlock == false` ⇒ `403 unlock_disabled` without reading the body or logind (unchanged);
4. `rp_id` absent ⇒ `403 passkeys_not_configured`;
5. `!parsed.has_body()` (no assertion: legacy Shortcut, page bug) ⇒ `403 passkey_required`;
6. effective host ≠ `rp_id` ⇒ `421`;
7. failure lockout of the caller's `LimitKey` (`Tailnet` or `FunnelSession`, rev 2, F-2) ⇒ `429 rate_limited`;
8. read body (`BODY_READ_TIMEOUT_MS`, `Length` or `Chunked`), decode `AssertionBody` ⇒ `400 bad_request`
   (failure counted), `413 body_too_large` for an oversized chunked body (not counted);
9. *(rev 2, F-7: split)*
   - 9a. **verification**: `parse_client_data(Get)`, `take(class, Unlock, binding)` with binding `None`
     (T) or `WebSession(token hash)` (F), credential lookup, `verify_assertion` ⇒ any failure
     `403 passkey_rejected` (failure counted, challenge consumed, no logind call);
   - 9b. **persistence** (only when `sign_count` or `backup_state` changed; synced passkeys with
     `0/0` never write): store `update` ⇒ any `StoreError` ⇒ `503 store_unavailable`, challenge
     consumed, **not** counted as a failure, **no logind call**; `Busy` keeps the web sessions, every
     other store error drops them (`retain_credentials(None)`), exactly as at validation time;
10. **unchanged** `unlock_flow` (rate limit `MIN_UNLOCK_INTERVAL_MS` ⇒ `429`, fresh snapshot, `409 no_session` /
    `409 already_unlocked`, `UnlockSession` ⇒ `202 unlock_requested`, `503` on any logind failure or
    `UNLOCK_FLOW_DEADLINE_MS`, one `info` audit line `remote unlock requested`).

Login verify uses the same 9a/9b rule (a store failure after a valid login assertion ⇒ `503
store_unavailable`, no session created, not counted). The auth mutex is released between steps 9 and 10.
No unlock path skips `verify_assertion` (whose UV check is unconditional), so an assertion without UV
can never reach step 10 on either path (test 39a, rev 2, F-5). A valid assertion consumed by a `409`/`429` outcome
is not refunded (the user repeats Face ID).

### 5.2 Limiters

*(rev 2, F-2: every limiter is keyed by `LimitKey`, §4.6, instead of `PathClass`.)*

- **Options limiter** per `LimitKey`: `MAX_OPTIONS_PER_WINDOW` issuances per `OPTIONS_WINDOW_MS`
  (fixed window from the first issuance); the 11th ⇒ `429 rate_limited`, no RNG call, no challenge.
- **Failure lockout** per `LimitKey`: the window starts at the first failure; when the count reaches
  `MAX_AUTH_FAILURES` every options/verify/unlock-assertion route using that key answers
  `429 rate_limited` (before reading a body) until the window ends; then both reset. Failures are:
  `400 bad_request` on a body route (incl. `BodyError::Malformed`), every `passkey_rejected`, every
  `enroll_code_rejected`. Never failures: `503 busy`, `503 store_unavailable`, `503 unavailable`,
  `413 body_too_large`, `409 registration_conflict`, every `429`.
- Key per route: login options/verify ⇒ `FunnelAnonymous(client_hint)`; unlock options/unlock ⇒
  `Tailnet` (T) or `FunnelSession` (F+s); register options/verify ⇒ `Tailnet`.
- Keys are independent: anonymous traffic (any number of hints) can never lock `FunnelSession` or
  `Tailnet`; one hint's garbage locks only that hint. `X-Forwarded-For` is used **only** to choose an
  anonymous bucket (`client_hint`, §2.4), never for authorization and never on the tailnet path.
- **Capacity** (S-11): `503 busy` when the Funnel, anonymous, anonymous-body-read or Funnel-SSE cap is
  reached (§3.1); checked before any body read; never a failure.
- **Residual denial of service (stated in the ADR)**: (i) a distributed attacker (many public
  addresses, or one IPv6 `/48`) can keep the anonymous connection, body-read and login-challenge caps
  busy or evict the owner's login challenge, so a Funnel **login** can fail or be delayed (retry); a
  logged-in owner (`FunnelSession`) keeps ≥ 4 Funnel slots, its own limiter and its own unlock pool, and
  the tailnet keeps ≥ 8 connection slots and its own limiters; (ii) slot classification needs the
  request head, so a burst of more than 16 simultaneous Funnel requests can still make the accept loop
  drop a tailnet connection for the few milliseconds each request lives (heads reach the socket
  complete, because Go's server parses the whole head before `ReverseProxy` dials the socket, so a slow
  Funnel head cannot hold an unclassified slot); (iii) `tailscaled` itself has no read timeouts;
  (iv) a hint shared with strangers (carrier-grade NAT) shares its anonymous bucket.
- `lock`/`unlock_flow` intervals (`MIN_LOCK_INTERVAL_MS`, `MIN_UNLOCK_INTERVAL_MS`) are unchanged and
  independent of these limiters.

### 5.3 Error taxonomy → HTTP

| Source | HTTP | `result` |
|---|---|---|
| `AuthError::*` (incl. `Ambiguous`, `FunnelDisabled`) | 403 | `forbidden` |
| Funnel route without a valid session | 403 | `login_required` |
| Register route on Funnel, login/logout route on tailnet | 403 | `forbidden` |
| `CsrfError::*` | 403 | `forbidden` |
| `rp_id` not configured | 403 | `passkeys_not_configured` |
| unlock without body | 403 | `passkey_required` |
| `WebAuthnError::*`, `ChallengeError::{Unknown, Expired, Mismatch}`, unknown credential | 403 | `passkey_rejected` |
| wrong/expired/missing enrollment code | 403 | `enroll_code_rejected` |
| `allow_unlock = false` | 403 | `unlock_disabled` (unchanged) |
| Funnel effective host ≠ `rp_id`, auth route host ≠ `rp_id` | 421 | `misdirected_request` |
| `StoreError::Full` / `Duplicate`; empty store on login/unlock options | 409 | `passkey_limit` / `already_registered` / `no_passkey` |
| `ChallengeError::PoolFull` | 429 | `too_many_challenges` |
| `WebSessionError::TooMany` | 429 | `too_many_sessions` |
| limiter / lockout | 429 | `rate_limited` |
| malformed body JSON/base64/field bound | 400 | `bad_request` |
| `HttpError::BodyTooLarge`, `BodyError::TooLarge` (body routes only) | 413 | `body_too_large` |
| body or `Transfer-Encoding` on a **non-body** route (existing `parse_request_head`, unchanged) | 413 / 400 | `body_not_allowed` / `bad_request` |
| `BodyError::Malformed` (chunked framing), TE + `Content-Length`, TE other than `chunked` on a body route (rev 2, F-9) | 400 | `bad_request` |
| `StoreError::UserHandleConflict` (rev 2, F-1) | 409 | `registration_conflict` |
| `CapacityError::Busy` (rev 2, F-2) | 503 | `busy` |
| `StoreError` after a valid assertion (rev 2, F-7) | 503 | `store_unavailable` (not counted, no logind call) |
| `StoreError::{Io, Insecure, TooLarge, Malformed, Busy}` | 503 | `store_unavailable` |
| `RandomError` | 503 | `unavailable` |
| body read timeout / peer closed | — | connection closed without response |

Every response keeps the §2.8 mandatory headers. `Set-Cookie` is only ever sent by `login_verify`
(success) and `logout`.

---

## 6. Dispatch order in `handle_connection` (rev 2, F-2/F-6/F-9)

1. Global permit (`MAX_CONNECTIONS`, unchanged accept loop), bounded head read (unchanged), then
   `parse_request(buf, accepts_body)` ⇒ existing statuses (non-body routes keep `413 body_not_allowed`)
   plus, on body routes only, `413 body_too_large` / `400 bad_request`; the bytes after `head_len` are
   kept as the body prefix (bounded by the head buffer).
2. `check_host` ⇒ `421` (unchanged).
3. `classify_request` ⇒ `403 forbidden`.
4. **Funnel class permit** (Funnel caller): `capacity.enter_funnel()` ⇒ `503 busy`; the permit lives
   until the connection ends (SSE included). Tailnet callers take no class permit.
5. Funnel caller: effective host must equal `rp_id` ⇒ `421`; `hint = client_hint(headers)`.
6. `route(method, path)`.
7. Funnel caller, under the auth mutex, in this order: (a) **revocation**:
   `let live = store.live_credential_hashes(); sessions.retain_credentials(live.as_deref());`
   (b) `session_token_hash` + `sessions.validate(now, hash, Touch::Refresh)`; then, outside the mutex:
   if `!is_funnel_public(route)` and no valid session ⇒ `403 login_required`; register routes ⇒
   `403 forbidden`; (c) **anonymous permit** when no valid session: `capacity.enter_anonymous()` ⇒
   `503 busy` (held to connection end).
8. Route CSRF (`check_lock_csrf`, `check_unlock_csrf`, `check_auth_csrf`).
9. Route gates (§5, §5.1 steps 3–7: `allow_unlock`, `rp_id`, body presence, host, lockout of the
   `LimitKey`); Funnel SSE: `MAX_FUNNEL_SSE_STREAMS` slot ⇒ `503` like the existing stream cap.
10. Anonymous body read reservation (only `POST /api/auth/login/verify` without session):
    `capacity.reserve_anonymous_body_read(hint)` ⇒ `503 busy`.
11. Body read (only for `accepts_body` routes that reached this point; `Length` or `Chunked`), under
    `BODY_READ_TIMEOUT_MS`, with no mutex held.
12. Handler.

A body-route request refused at steps 2–10 is answered without its body being read (the lingering
close discards it, bounded as today). A `503 busy` is answered at once and never counted.

Capacity arithmetic: tailnet callers can always obtain at least `MAX_CONNECTIONS −
MAX_FUNNEL_CONNECTIONS = 8` classified slots; a logged-in Funnel owner at least
`MAX_FUNNEL_CONNECTIONS − MAX_ANONYMOUS_FUNNEL_CONNECTIONS = 4`; anonymous body trickling holds at most
`MAX_ANONYMOUS_BODY_READS = 2` slots for at most `BODY_READ_TIMEOUT_MS` each, one per hint. Unclassified
connections (before step 3) are bounded by `REQUEST_HEAD_TIMEOUT_MS` as today (residual (ii) of §5.2).

---

## 7. Security invariants touched

| Invariant | Status |
|---|---|
| ARCHITECTURE §2 "no network socket" (RMC-S2, `RestrictAddressFamilies=AF_UNIX`) | kept: Funnel terminates in `tailscaled`, the service still only accepts on its Unix socket |
| Never root (RMC2, RMC-S10) | kept; subcommands also refuse root first |
| RMC-S3 (logind method literals, `test_rmc_s3_no_unlock_or_locked_hint_literal`) | kept: no new D-Bus literal; `UnlockSession`/`LockSession` still once each. *(rev 2, F-8)* That test forbids the quoted literal `"Unlock"` (and `"UnlockSessions"`, `"SetLockedHint"`, …) in **every** comment-stripped `crates/remote/src` file: `ChallengePurpose::Unlock` is never rendered through a string literal (no `serde` rename, no `Display`/`as_str`, no purpose in any JSON body or log); wire values use the lowercase `ACTION_UNLOCK_OPTIONS`/`ACTION_UNLOCK` constants and result strings (`"unlock_disabled"`, …), which do not contain the forbidden quoted literal |
| `test_rmc_unlock_is_opt_in_and_documented` *(rev 2, F-8)* | kept unchanged; its needles must survive the documentation and page rewrites verbatim: `Docs/REMOTE_COMPANION.md` ⊇ {`allow_unlock = true`, `POST /api/unlock`, `` X-Soos-Action` = `unlock ``, `unlock_disabled`, `already_unlocked`, `Accepted risk`}; `crates/remote/assets/app.js` ⊇ {`/api/unlock`, `"X-Soos-Action": "unlock"`, `Unlock now`, `window.confirm(`, `unlock_disabled`}; `index.html` ⊇ `id="unlock"`; `AI/DECISIONS.md` ⊇ the title ``Remote Unlock in `soos-remote`, Tailscale Identity Only, Opt-In`` (the old ADR keeps its title; only a supersession note is appended) |
| RMC-S4 leaf crate | kept |
| RMC-S9 env vars (`XDG_RUNTIME_DIR`, `XDG_CONFIG_HOME`, `HOME` only) | kept (S-5) |
| RMC14 / RC-5 privacy | extended to challenges, tokens, cookies, credential ids, public keys, user handles, enrollment codes, attestation/assertion bytes (D-H); new logging needles §10.6 |
| Fail-closed logind (`unavailable`, never `unlocked`) | unchanged |
| Identity-only unlock (ADR 2026-10-06 item 2) | **superseded** by D-C |
| Funnel prohibition (ADR 2026-10-05 item 8, Docs §2) | **superseded** for `tailscale funnel` on 443 with `allow_funnel = true`; `tailscale serve --http`, other proxies and other Funnel ports stay refused |
| No `unwrap`/`expect`/panic/print in production (RMC14) | applies to every new module; CLI output through `writeln!` on a locked `stdout` (never `println!`) |
| `SystemTime::now` only in `main.rs`/`server.rs` (R3-1) | kept: `enroll.rs`/`credentials.rs` take `now_unix_s` parameters |

---

## 8. Page and JavaScript (`assets/`)

`index.html` (no inline script, no inline handler, same-origin only):
- `<section id="login" hidden>`: text "Sign in with your passkey to reach this PC over the internet.",
  `<button id="login-button" type="button">Sign in with Face ID</button>`.
- Existing status card (`#state`, `#activity`, `#updated`, `#lock`, `#unlock`, `#feedback`) shown when
  authenticated or on the tailnet.
- `<button id="logout" type="button" class="secondary" hidden>Sign out</button>` (Funnel only).
- `<section id="enroll" hidden>` (tailnet with `enrollment: true`): `<input id="enroll-code"
  autocomplete="one-time-code" autocapitalize="characters" maxlength="11">`,
  `<button id="enroll-button" type="button">Add this device's passkey</button>`.

`app.js`:
- New constants: `AUTH_STATE_PATH = "/api/auth/state"`, `LOGIN_OPTIONS_PATH`, `LOGIN_PATH`
  (`/api/auth/login/verify`), `LOGOUT_PATH`, `UNLOCK_OPTIONS_PATH`, `REGISTER_OPTIONS_PATH`,
  `REGISTER_PATH`. Hand-written base64url ⇄ `ArrayBuffer` helpers (no `parse*OptionsFromJSON`).
- `resume()`: `fetch(AUTH_STATE_PATH)` first; Funnel + `authenticated: false` ⇒ show `#login`, close the
  stream, no status fetch; otherwise the existing stream/status logic. Any `403 login_required` from
  status/events/lock/unlock ⇒ back to the login screen.
- Login: click handler ⇒ `POST LOGIN_OPTIONS_PATH` (`X-Soos-Action: login-options`) ⇒
  `navigator.credentials.get({ publicKey: { challenge, rpId, timeout, userVerification: "required" } })`
  (**modal**, never `mediation: "conditional"`, no `allowCredentials`) ⇒ `POST LOGIN_PATH`
  (`X-Soos-Action: login`, `Content-Type: application/json`, body `AssertionBody`) ⇒ `resume()`.
- Unlock: keeps `window.confirm("Unlock the PC now?")` (required by `test_rmc_unlock_is_opt_in_and_documented`),
  then (same click handler, user gesture) `POST UNLOCK_OPTIONS_PATH` ⇒ `navigator.credentials.get(…)` ⇒
  `POST UNLOCK_PATH` with `X-Soos-Action: unlock` and the JSON assertion; maps the new results
  (`passkey_rejected`, `passkey_required`, `passkeys_not_configured`, `no_passkey`, `login_required`,
  `rate_limited`, `too_many_challenges`, `store_unavailable`) and the existing ones (`unlock_disabled`, …).
- Enrollment: `POST REGISTER_OPTIONS_PATH` `{code}` ⇒ `navigator.credentials.create({ publicKey: { rp: { id, name: "soos" },
  user: { id, name: "soos", displayName: "soos" }, challenge, pubKeyCredParams: [{ type: "public-key", alg: -7 }],
  authenticatorSelection: { residentKey: "required", requireResidentKey: true, userVerification: "required" },
  attestation: "none", excludeCredentials, timeout } })` ⇒ `POST REGISTER_PATH`.
- Sign out: `POST LOGOUT_PATH` (`X-Soos-Action: logout`) ⇒ login screen.
- Still: `textContent` only; no `innerHTML`, `eval`, `http://`/`https://` literal; no `localStorage`,
  `sessionStorage`, `indexedDB` or `document.cookie` (the cookie is `HttpOnly`); no service worker.
- All existing required strings of RMC-S8 stay (`STALE_UI_MS = 45000`, `LOCK_CONFIRM_UI_MS = 5000`, …).
- *(rev 2, F-8)* The unlock request keeps the exact header literal `"X-Soos-Action": "unlock"`, the
  button text `Unlock now`, `window.confirm(`, the path `/api/unlock` and the `unlock_disabled` mapping
  (needles of `test_rmc_unlock_is_opt_in_and_documented`).
- *(rev 2)* New result mappings: `busy` ("The PC is busy, try again"), `registration_conflict`
  ("Enrollment changed meanwhile: get a new code and retry; remove the extra passkey from the
  phone's Passwords app"), `body_too_large`.
- *(rev 2, F-4)* The page never polls `/api/auth/state` or `/api/status` on a timer while the stream is
  healthy, so an untouched page expires after `WEB_SESSION_IDLE_MS`; `resume()` on
  `visibilitychange`/`pageshow` is user-driven and refreshes the session.

`style.css`: styles for the new sections; still no `url()` to a remote resource, no `@import`.

---

## 9. Operations: CLI, unit, installer, Shortcuts

### 9.1 CLI (`main.rs`, clap)

```text
soos-remote [--config PATH]                     serve (unchanged default)
soos-remote [--config PATH] enroll-code         print a one-time code (5 min, single use)
soos-remote [--config PATH] passkeys list       index, created_unix_s, backup state, 8-hex fingerprint of SHA-256(credential id)
soos-remote [--config PATH] passkeys remove N   remove record N (1-based, 1..=MAX_PASSKEYS)
```

Every subcommand: `check_not_root` first, then `load_config` (exit 78 on error). `enroll-code` requires
`rp_id` (else exit 78 "rp_id is not configured"), runs `prepare_socket_dir`, writes the code file, then
prints `Enrollment code: XXXXX-XXXXX (valid 5 min, single use)` and the instruction to open
`https://<rp_id>` over the tailnet. `passkeys list|remove` work on the resolved store path (store errors
exit 1; `remove` of a missing index exits 78 "no such passkey"). Output via `writeln!(stdout.lock(), …)`,
never `print!` macros; nothing printed contains a credential id, key or user handle. A running service
notices a removal through the store stamp (§4.4) and drops sessions of removed credentials at the next
session validation.

### 9.2 Unit and installer

- `packaging/soos-remote.service`: **unchanged** (all RMC-S5 lines kept).
- `scripts/install_remote.sh`: template gains commented lines
  `# rp_id = "mypc.tail1234.ts.net"`, `# allow_funnel = false`, `# credentials_path = "/home/me/.config/soos/remote-passkeys.json"`;
  the final instructions (inside `echo`/`printf` only) add: set `rp_id`, restart, run `soos-remote enroll-code`,
  register Face ID from the phone **over the tailnet**, then optionally set `allow_funnel = true` and run
  `tailscale funnel --bg unix:$XDG_RUNTIME_DIR/soos-remote/remote.sock`. The installer never runs `tailscale`
  (RMC-S6 unchanged).

### 9.3 Shortcuts consequences (documented in `Docs/REMOTE_COMPANION.md` §4)

- "Lock PC" and "PC status" shortcuts: unchanged, **tailnet only** (VPN on). Over Funnel they receive
  `403 login_required` (Shortcuts cannot run WebAuthn and do not share the web app's cookie).
- "Unlock PC" shortcut: **no longer possible** (D-C: every unlock needs a passkey assertion, which only the
  page can produce). It now receives `403 passkey_required` (or `unlock_disabled`). Replacement: an
  "Open URL `https://<pc>.<tailnet>.ts.net`" shortcut that opens the web app, then *Unlock now* + Face ID.
  The documentation keeps the literal `` `X-Soos-Action` = `unlock` `` in the HTTP surface section (needle of
  `test_rmc_unlock_is_opt_in_and_documented`) *(rev 2, F-8: together with `allow_unlock = true`,
  `POST /api/unlock`, `unlock_disabled`, `already_unlocked` and an `Accepted risk` paragraph, all kept
  verbatim; full list in §7)*.

---

## 10. Test list for the tester (Phase 2)

Conventions: `#[tokio::test(start_paused = true)]` for server tests (existing frozen-clock harness);
fixtures signed with `p256::ecdsa::SigningKey` and CBOR built with `ciborium`; the L3 §16.2 vector
(ES256, no attestation, `rp_id = "example.org"`) is copied verbatim as a negative-UV fixture. New
integration tests go in new files; a shared `crates/remote/tests/common/passkey.rs` (fixture builders:
`attestation_object(rp_id, flags, cred_id, key)`, `client_data(type, challenge, origin, extra)`,
`assertion(rp_id, flags, sign_count, key, client_data)`) may be added. Harness hooks: `Options` gains
`rp_id`, `allow_funnel`, `credentials_path` (in the `TempDir`), `random` (scripted / failing).
*(rev 2)* Funnel request builders take an `X-Forwarded-For` value (the client hint); `AuthState`
exposes `challenges.pending_for_hint` and the session count for assertions; `Capacity` is public for
the unit part of test 55.

### 10.1 `config_tests.rs` (new tests only)
1. `test_rmc_passkey_constants_match_the_adr` — every §3.1 value, `WEBAUTHN_TIMEOUT_MS == CHALLENGE_TTL_MS`, `SESSION_COOKIE_ATTRIBUTES` contains `Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/` and no `Domain`.
2. `test_rmc_parse_config_auth_defaults_are_off` — absent keys ⇒ `AuthConfig::default()`.
3. `test_rmc_parse_config_rp_id_rules` — lowercased; `pc.tail1234.ts.net` ok; `tail1234.ts.net`, `ts.net`, `pc.example.com` (no allowlist), `pc.tail1234.ts.net:443`, `https://…`, `""`, IP literal ⇒ `InvalidRpId`; with `allowed_hosts` it must be a member.
4. `test_rmc_parse_config_allow_funnel_requires_rp_id` — `allow_funnel = true` alone ⇒ `FunnelNeedsRpId`; non-boolean ⇒ `Syntax`.
5. `test_rmc_parse_config_credentials_path_rules` — relative, trailing `/`, > 4096 bytes ⇒ `InvalidCredentialsPath`.
6. `test_rmc_resolve_credentials_path` — explicit wins; default is the sibling `remote-passkeys.json`.
7. `test_rmc_auth_config_error_messages_are_fixed_english_text`.

### 10.2 `identity_tests.rs` (new tests only)
8. `test_rmc_classify_request_table` — the six rules of §2.1 (incl. `?1` with OWS, `?0`, `?1?1`, repeated marker, login + marker ⇒ `Ambiguous`, marker with `allow_funnel = false` ⇒ `FunnelDisabled`).
9. `test_rmc_classify_request_ignores_spelling_variants` — `Tailscale_User_Login`, `Tailscale-User-Name` never authorize; `Tailscale_Funnel_Request` never classifies as Funnel.
10. `test_rmc_classify_request_never_reads_forwarding_headers` — `X-Forwarded-For`/`X-Forwarded-Host`/`Host` values do not change the outcome.

### 10.3 `http_tests.rs` / `routes_tests.rs` (new tests only)
11. `test_rmc_parse_request_matches_parse_request_head_for_non_body_routes` — property test (proptest) over generated heads: same result/status when `accepts_body` is false.
12. `test_rmc_parse_request_body_framing` *(rev 2, F-9)* — body route: CL 0/absent ⇒ `None`; CL 8192 ⇒ `Length(8192)`; 8193 ⇒ `BodyTooLarge` (status 413); `Transfer-Encoding: chunked` (also `Chunked`, with OWS) ⇒ `Chunked`; `gzip`, `gzip, chunked`, two TE headers, TE + CL ⇒ `Malformed` (400); conflicting CL ⇒ `Malformed`; `head_len` points at the body; non-body route with a body keeps `BodyNotAllowed` and its existing 413/400 status.
13. `test_rmc_read_body_is_bounded` *(rev 2, F-9)* — `Length`: prefix used, exact length read, extra pipelined bytes ignored, timeout at exactly `BODY_READ_TIMEOUT_MS`, EOF ⇒ `Closed`. `Chunked`: Go-style body (`"1a\r\n…\r\n0\r\n\r\n"`) split across prefix and stream decodes exactly; uppercase hex ok; chunk extension `;x`, OWS, bare LF, missing CRLF after data, a trailer field, 9 hex digits, 65 data chunks ⇒ `Malformed`; decoded total 8192 ok, 8193 ⇒ `TooLarge` without reading that chunk's data; `0\r\n\r\n` alone ⇒ empty body; bytes after the last CRLF never read.
14. `test_rmc_auth_route_table` — every new route/method, `405` + `Allow`, `accepts_body`, `is_funnel_public`.
15. `test_rmc_check_auth_csrf_requires_origin` — action exact per route, `Origin` absent ⇒ `OriginMismatch`, `https://<rp_id>` and `:443` ok, sibling node / `http://` / `null` refused.

### 10.4 `webauthn_tests.rs` (new file, pure)
16. `test_rmc_parse_client_data_rules` — type, origin byte equality (no port, no trailing `/`), `crossOrigin: true`, `topOrigin`, unknown keys tolerated, duplicate key refused, challenge padding/length, size bound.
17. `test_rmc_verify_registration_accepts_a_valid_none_attestation` — generated key, flags `UP|UV|AT` (± `BE|BS`).
18. `test_rmc_verify_registration_rejections` — table: `fmt` `packed`, non-empty `attStmt`, extra/missing top-level key, trailing bytes, rpIdHash, UP/UV missing, BS without BE, AT clear, ED set, id length 0 / 1024, id mismatch with `raw_id`, COSE extra key / duplicate key / `alg -8` / `crv 2` / compressed `-3` / 31-byte `x` / off-curve point / identity, byte after the COSE key.
19. `test_rmc_l3_vector_16_2_is_rejected_for_missing_uv` — the §16.2 registration ⇒ `UserVerificationMissing`; its assertion signature verifies but the assertion ⇒ `UserVerificationMissing`.
20. `test_rmc_verify_assertion_accepts_a_valid_uv_assertion` — incl. a **high-S** variant of the same signature.
21. `test_rmc_verify_assertion_rejections` — user handle absent/mismatch, authData 36/38 bytes, AT/ED set, UP/UV missing, BE changed, BS without BE, signature > 72 bytes / not DER / wrong key / over re-serialised client data, sign count regression (`5`→`5`, `5`→`3`), `0/0` accepted, `0`→`7` accepted with outcome 7.
22. `test_rmc_webauthn_errors_never_echo_values` — every variant's text is fixed and contains no input byte.

### 10.5 `auth_store_tests.rs` / `credentials_tests.rs` / `enroll_tests.rs` (new files)
23. `test_rmc_challenge_store_is_single_use_and_scoped` — take twice ⇒ `Unknown`; wrong purpose/class/binding ⇒ `Mismatch` **and removed**; expired at exactly `CHALLENGE_TTL_MS` ⇒ `Expired`; *(rev 2, F-1)* a `Register` challenge issued with `PendingRegistration { user_handle: H }` is returned by `take` as `Taken::Register` with `H`; `issue(Register, pending: None)` and `issue(Login, pending: Some(_))` ⇒ `Mismatch`, nothing stored; a `Login(hint_a)` challenge taken with `Login(hint_b)` succeeds (hint is not a binding).
24. `test_rmc_challenge_pools_are_bounded_without_eviction` — authenticated pools: 5th issue in one pool ⇒ `PoolFull`, other pools unaffected, expiry frees a slot.
24a. `test_rmc_anonymous_login_pool_evicts_oldest` *(rev 2, F-2)* — a 3rd issuance from one hint evicts that hint's oldest only (other hints' entries intact, return value 1); 16 entries from 8 hints then a 17th from a 9th hint evicts the pool's oldest (return 1); the pool never answers `PoolFull`; an evicted challenge's later `take` ⇒ `Unknown`; `pending_for_hint` never exceeds 2, `pending(Funnel, Login)` never exceeds 16.
25. `test_rmc_web_sessions_ttl_cap_and_logout` — idle expiry at exactly 15 min, absolute at exactly 8 h even when touched, 5th ⇒ `TooMany`, `remove` idempotent, `retain_credentials`; *(rev 2, F-4)* `validate(…, Touch::Keep)` never moves `last_used` (a session validated only with `Keep` every 15 s expires at exactly 15 min) while `Touch::Refresh` does.
26. `test_rmc_session_cookie_parsing` — several `cookie` headers, duplicate name ⇒ None, 42/44 chars, padding, wrong alphabet, other cookies ignored; `set_cookie_header`/`clear_cookie_header` exact strings.
27. `test_rmc_credential_store_roundtrip_and_validation` — encode/parse; unknown key, version 2, 5 records, duplicate ids, off-curve key, 16 385 bytes ⇒ errors.
28. `test_rmc_credential_store_file_rules` — mode `0644` ⇒ `Insecure`, symlink ⇒ refused (never followed), FIFO never blocks, foreign owner (when testable) ⇒ `Insecure`, missing ⇒ `Ok(None)`.
29. `test_rmc_credential_store_atomic_update` — new file mode `0600`; failure inside `f` leaves the old bytes and no temp file; a concurrent holder of the lock ⇒ `Busy` after `STORE_LOCK_TIMEOUT_MS`; stamp change ⇒ reload.
30. `test_rmc_enroll_code_generation_and_normalisation` — alphabet, length, display `XXXXX-XXXXX`, lowercase and dashes accepted, `I`/`L`/`O`/`U`, 9/11 symbols, > 32 bytes ⇒ None; scripted RNG ⇒ deterministic code.
31. `test_rmc_enroll_code_file_rules` — format, 0600, O_NOFOLLOW, > 256 bytes, future-dated, malformed hex.

### 10.6 Server integration (`auth_server_tests.rs`, new file, same harness pattern)
32. `test_rmc_funnel_disabled_by_default_is_forbidden` — marker `?1` with defaults ⇒ `403 forbidden` on every route, zero logind reads.
33. `test_rmc_funnel_public_routes_and_login_required` — Funnel without session: assets `200`, `/api/auth/state` minimal shape, status/events/lock/unlock/unlock-options ⇒ `403 login_required`, register routes ⇒ `403 forbidden`, no logind read, no body read (a gated body route answers after its head only: a declared 4 KiB body that is never sent still gets the `403` at once).
34. `test_rmc_funnel_host_must_equal_rp_id` — another `*.ts.net` host ⇒ `421`.
35. `test_rmc_forged_identity_on_funnel_is_refused` — login + marker ⇒ `403`; underscore variant + marker ⇒ Funnel anonymous (`login_required`).
36. `test_rmc_funnel_login_issues_a_host_cookie_session` — full ceremony; `Set-Cookie` exact attributes; then status/events/lock with the cookie ⇒ `200`/stream/`202`; the token never appears in any later response.
37. `test_rmc_funnel_login_rejections_consume_the_challenge` — replayed assertion ⇒ `passkey_rejected`; login assertion reused for unlock ⇒ `passkey_rejected` (purpose); tailnet-issued challenge used on Funnel ⇒ rejected (class).
38. `test_rmc_funnel_sessions_expire_and_logout` — idle 15 min ⇒ `login_required`; absolute 8 h; logout clears cookie and record; open SSE stream ends within one keep-alive after logout/expiry; 5th login from 5 distinct cookie-less clients ⇒ `429 too_many_sessions`. *(rev 2, F-4)* an SSE stream left open with no other request ends within one keep-alive after 15 min idle and the next status with the cookie ⇒ `login_required` (keep-alive does not touch); a `GET /api/status` at 14 min keeps the session valid at 20 min (request-level validation touches). *(rev 2, F-6)* 6 consecutive logins from one browser, each presenting the previous cookie, all ⇒ `logged_in`, the store holds exactly 1 record, and every previous cookie ⇒ `login_required`.
39. `test_rmc_unlock_requires_a_fresh_passkey_on_every_path` — tailnet identity without body ⇒ `403 passkey_required`, zero logind reads; with a valid assertion ⇒ `202`; same assertion again ⇒ `passkey_rejected`; Funnel session + assertion ⇒ `202`; Funnel session without assertion ⇒ `passkey_required`; Funnel unlock challenge bound to another session ⇒ rejected; `rp_id` absent ⇒ `passkeys_not_configured`.
39a. `test_rmc_unlock_refuses_assertions_without_user_verification_end_to_end` *(rev 2, F-5)* — on the tailnet path and on the Funnel path with a valid session: a fresh `unlock`-purpose challenge, an assertion with flags `UP` only (UV clear), validly signed by the registered key with a correct user handle ⇒ `403 passkey_rejected`, zero logind calls (reads included), the challenge consumed (a retry with the same challenge ⇒ `passkey_rejected`), one failure counted; a validly signed UV assertion over a **login**-purpose challenge (from `/api/auth/login/options`) sent to `POST /api/unlock` with a valid session ⇒ `403 passkey_rejected`, zero logind calls; the valid session alone (no body) ⇒ `passkey_required`.
40. `test_rmc_unlock_order_disabled_before_body_and_logind` — `allow_unlock = false` with a body ⇒ `unlock_disabled`, body not read, no logind read.
41. `test_rmc_registration_is_tailnet_only_with_a_local_code` — Funnel ⇒ `403 forbidden` before body; tailnet without code file ⇒ `enroll_code_rejected`; wrong code ×3 deletes the file; expired code; valid code ⇒ options with `user_id` and `exclude_credentials`; verify ⇒ `registered`, store file `0600` with one record, code file removed; second registration of the same credential ⇒ `409 already_registered`; 5th ⇒ `409 passkey_limit`.
41a. `test_rmc_first_registration_then_login_and_unlock_use_the_same_user_handle` *(rev 2, F-1)* — empty `TempDir` store; tailnet registration with a valid code; the store file's `user_handle` equals the options' `user_id`; then a Funnel login whose assertion carries that user handle ⇒ `logged_in`; then a Funnel unlock and a tailnet unlock with the same passkey ⇒ `202` each.
41b. `test_rmc_interleaved_registrations_on_an_empty_store` *(rev 2, F-1)* — empty store, scripted RNG: two register-options with the same valid code ⇒ two challenges with user handles `X ≠ Y`; verify the second (`Y`) ⇒ `registered`, store handle `Y`, code file removed; verify the first ⇒ `403 enroll_code_rejected` (code consumed), store bytes unchanged; then login with the `Y` passkey ⇒ `logged_in`. Variant: register-options on an empty store (handle `X`), then the test writes a valid store with handle `Z` and no passkey, then verify ⇒ `409 registration_conflict`, store bytes unchanged, code file still present, failure counter unchanged.
42. `test_rmc_auth_limiters_are_per_limit_key` *(rev 2, F-2; renamed from `…_per_path_class`, never written in round 1)* — per key: 11th options in 60 s ⇒ `429`; 5 failures ⇒ `429` for 5 min then reset; a locked `FunnelAnonymous(hint A)` leaves hint B, `FunnelSession` and `Tailnet` working; a locked `FunnelSession` leaves anonymous login and `Tailnet` working.
43. `test_rmc_auth_fails_closed_on_store_and_random_errors` — insecure store ⇒ `503 store_unavailable` and existing sessions dropped; RNG failure ⇒ `503 unavailable` with no challenge/session created. *(rev 2, F-7)* A valid UV unlock assertion with `signCount` 7 over a stored 6 while another holder keeps the store lock ⇒ `503 store_unavailable` after `STORE_LOCK_TIMEOUT_MS`, zero logind calls, challenge consumed, sessions kept (`Busy`), and the failure window unchanged (4 further malformed bodies still answer `400`, the 5th then locks); a `0/0` synced assertion never takes the store lock (`202` while the lock is held).
44. `test_rmc_passkey_removal_revokes_sessions` — rewrite the store without the credential ⇒ next request with its cookie ⇒ `login_required`.
45. `test_rmc_auth_never_logs_secrets` *(rev 2: also no client address or hint)* — TRACE capture over every auth route and failure: no challenge, token, cookie value, credential id, public key, user handle, enrollment code, login, host, `Set-Cookie`; exactly one `remote login accepted` / `passkey registered` `info` line per success.

### 10.7 Superseded assertions in existing `server_tests.rs` (ADR item 12)

Only these change, and only as stated; every other assertion of the file stays byte-for-byte:

- `Harness::start_with`: the `RemoteConfig` literal gains `auth: …` and `ServerState` gets the store
  path / RNG hooks (setup only).
- `test_rmc_unlock_flow_and_rate_limit`, `test_rmc_unlock_and_lock_rate_limits_are_independent`,
  `test_rmc_unlock_flow_deadline`, `test_rmc_unlock_is_audited_without_identity`: every unlock request
  that previously succeeded on identity alone now carries a fresh valid assertion (new helper
  `unlock_with_passkey`); all status/result/logind-call/log assertions are unchanged. The audit test adds
  the credential id, challenge and user handle to its forbidden-needle list.
- `test_rmc_unlock_requires_identity_host_and_csrf`: the assertion "a 2-byte body `{}` ⇒ `413`" is
  superseded by "`{}` ⇒ `400 bad_request` and a 8 193-byte body ⇒ `413`, both without a logind read";
  the final `202` carries an assertion.
- Unchanged and still valid: `test_rmc_unlock_disabled_by_default_never_reaches_logind`,
  `test_rmc_disabled_unlock_checks_csrf_first`, `test_rmc_unlock_route_is_post_only`,
  `test_rmc_identity_is_required_before_routing`, every `http_tests`, `routes_tests`, `identity_tests`
  and `config_tests` test.

### 10.8 Invariants (`tests/invariants/src/remote_passkey_contract.rs`, new)
46. `test_rmc_s13_passkey_dependencies_are_pure_rust` *(rev 2, F-3: exact names)* — workspace `p256` line exact (`0.13.2`, `default-features = false`, `["ecdsa"]`); remote manifest uses `{ workspace = true }` for `p256`, `sha2`, `ciborium`, `getrandom`, `base64ct`, `subtle`; no **dependency key** of the remote manifest (parsed with `toml`) equals `openssl`, `openssl-sys`, `native-tls`, `ring`, `aws-lc-rs`, `aws-lc-sys`, `rand` or starts with `webauthn-rs`; when `cargo` is available, `cargo tree -p soos-remote -e normal --prefix none --format {p}` is split into lines, the **first whitespace-separated token** of each line is the crate name, and no name equals a forbidden name or starts with `webauthn-rs`; `rand_core` (and any other name that merely contains `rand`, e.g. `rand_core`) is explicitly allowed and the test asserts it is not refused.
47. `test_rmc_s14_deny_skip_for_der_is_documented` — exactly one `der@0.7.10` skip naming `ecdsa`, `sec1` and `ureq`.
48. `test_rmc_s15_auth_logging_hygiene` — no tracing field key or inline capture named `challenge`, `token`, `cookie`, `credential`, `credential_id`, `public_key`, `user_handle`, `code`, `enroll_code`, `assertion`, `attestation`, `signature`, `client_data` in any remote source.
49. `test_rmc_s16_csprng_is_getrandom_only` — `getrandom::fill` appears only in `auth.rs`; no `rand::`, `thread_rng`, `OsRng` in the crate.
50. `test_rmc_s17_cookie_attributes` — `lib.rs` defines `SESSION_COOKIE_NAME` with the `__Host-` prefix and `SESSION_COOKIE_ATTRIBUTES` exactly; no `Domain=` anywhere in the crate.
51. `test_rmc_s18_page_uses_modal_webauthn_without_storage` — `app.js` contains `navigator.credentials.get`, `navigator.credentials.create`, `userVerification: "required"`, `residentKey: "required"`, `attestation: "none"`, `alg: -7`, `/api/auth/login/options`, `/api/auth/unlock/options`, `/api/auth/register/options`; never `conditional`, `localStorage`, `sessionStorage`, `indexedDB`, `document.cookie`, `allowCredentials`; `index.html` carries `id="login-button"`, `id="enroll-code"`, `id="logout"`.
52. `test_rmc_s19_funnel_and_passkeys_are_documented` — `Docs/REMOTE_COMPANION.md` mentions `tailscale funnel --bg unix:`, `allow_funnel`, `rp_id`, `soos-remote enroll-code`, `soos-remote passkeys`, `Tailscale-Funnel-Request`, `passkey_required`, `login_required`, `port 443`; `AI/DECISIONS.md` registers the ADR title; the unit file still has `RestrictAddressFamilies=AF_UNIX` and no `ProtectHome=` that would make `~/.config` read-only.
53. `test_rmc_s20_subcommands_refuse_root_and_never_print` — `main.rs` names `enroll-code` / `EnrollCode` and `passkeys`; `check_not_root(` precedes every subcommand dispatch; no `println!`/`print!` (existing test also covers this).
53a. `test_rmc_s21_forwarded_for_is_read_only_by_client_hint` *(rev 2, F-2)* — in comment-stripped `crates/remote/src`, `FORWARDED_FOR_HEADER` is defined once in `lib.rs` and used only in `identity.rs` inside `client_hint`; the literal `x-forwarded-for` appears nowhere else; `classify_request`, `authorize`, `check_host` bodies do not name it.

### 10.9 Class capacity and anonymous limits (rev 2, F-2; `auth_capacity_tests.rs`, new file)
54. `test_rmc_tailnet_is_served_while_funnel_slots_are_saturated` — with `allow_funnel = true`: hold 8 Funnel connections (2 session-holder SSE streams, 2 anonymous login-verify requests from two hints declaring 4 KiB and sending nothing, 4 session-holder unlock requests declaring a body and sending nothing); a 9th Funnel request (asset) ⇒ `503 busy` at once; a tailnet `GET /api/status` ⇒ `200` and a tailnet `POST /api/lock` ⇒ `202`, both within the frozen-clock bound; after the body deadline the slots are released and a Funnel asset ⇒ `200`.
55. `test_rmc_anonymous_funnel_capacity_and_body_reads` — server: two anonymous login-verify requests from hints A and B declaring 4 KiB and sending nothing are held; a second concurrent login-verify from hint A ⇒ `503 busy` before any body byte is read; one from a new hint C ⇒ `503 busy`; meanwhile an anonymous asset ⇒ `200` and a session holder's status ⇒ `200`; no `503 busy` is ever counted as a failure (hint A still needs 5 malformed bodies to be locked). Unit (`Capacity`): `enter_anonymous` succeeds `MAX_ANONYMOUS_FUNNEL_CONNECTIONS` times then `Busy`, dropping one permit frees one slot; `enter_funnel` likewise at `MAX_FUNNEL_CONNECTIONS`; dropping a `BodyReadGuard` releases both the total and the per-hint count.
56. `test_rmc_owner_login_survives_a_single_hint_filling_the_login_pool` — hint A issues 10 login options (pool entries for A never exceed 2); the owner (hint B) gets options, completes Face ID and logs in ⇒ `logged_in`; hint A's 11th options ⇒ `429` while hint B's next options ⇒ `200`. Distributed residual: 16 hints each issuing after the owner's options evict it and the owner's verify ⇒ `passkey_rejected` (documents residual (i); no session created).
57. `test_rmc_anonymous_failures_never_lock_session_holders_or_other_hints` — hint A sends 5 malformed bodies to login/verify (pre-challenge garbage) ⇒ hint A's 6th request to login options ⇒ `429`; hint B login options ⇒ `200`; an existing session holder's unlock options + valid unlock assertion ⇒ `202`; tailnet unlock with a valid assertion ⇒ `202` (after `MIN_UNLOCK_INTERVAL_MS`); requests with a missing `X-Forwarded-For` share `ClientHint::UNKNOWN`.
58. `test_rmc_client_hint_parsing` (pure, `identity_tests.rs`) — one header `203.0.113.7` and `::ffff:203.0.113.7` ⇒ same hint; `2001:db8:1:2:aaaa::1` and `2001:db8:1:2:bbbb::2` ⇒ same hint (/64), `2001:db8:1:3::1` ⇒ different; absent, two headers, `a, b`, `203.0.113.7:443`, `[2001:db8::1]`, 46 bytes, non-UTF-8, empty ⇒ `UNKNOWN`; `format!("{:?}", hint)` is `<redacted>` and contains no digit of the address.
59. `test_rmc_client_hint_table_is_bounded` (pure, `auth_store_tests.rs`) — 65 hints each issuing one options ⇒ at most `MAX_CLIENT_HINTS` buckets kept, the oldest window evicted first, a bucket with a body read in flight never evicted.

---

## 11. Amended matrix rows

- **RMC23** — "Unlock flow" stays as written but every successful request now carries a fresh passkey
  assertion (ADR 2026-10-06 Funnel/passkey item 4); evidence tests unchanged in name.
- **RMC24** — "Unlock authorization": the clause "body (`413`)" becomes "a body over
  `MAX_AUTH_BODY_BYTES` (`413`), a malformed assertion (`400`), no assertion (`403 passkey_required`),
  an invalid assertion (`403 passkey_rejected`)"; the identity/host/CSRF clauses unchanged.
- **RMC25** — the page now asks for Face ID after the confirmation tap; the hardware check runs with a
  registered passkey; Shortcuts "Unlock PC" removed from the documentation.

## 12. New matrix rows (RMC26–RMC41)

| # | Criterion | Test method | Status |
|---|---|---|---|
| RMC26 | Request classification (§2.1): tailnet identity, Funnel marker `?1` (only with `allow_funnel = true`), refusal of both/neither/repeated/other values; spelling variants never match; forwarding headers never an input | tests 8–10, 32, 35 | ⬜ |
| RMC27 | Funnel gate (D-D): without a session only assets, `/api/auth/state` and the login ceremony; status/events/lock/unlock ⇒ `403 login_required`; registration ⇒ `403 forbidden`; Funnel host must equal `rp_id` (`421`); no logind read, no body read | tests 33, 34 | ⬜ |
| RMC28 | Configuration: `rp_id` (full node host), `allow_funnel` (requires `rp_id`, default `false`), `credentials_path`; fail-closed errors with fixed texts; `allow_unlock` without `rp_id` stays valid but unlock is refused | tests 1–7 | ⬜ |
| RMC29 | Body framing: only the four body routes accept a body, ≤ 8 192 bytes (`413 body_too_large`), `Content-Length` or strict bounded `Transfer-Encoding: chunked` (rev 2: ≤ 64 chunks, ≤ 8 hex digits, no extension/trailer; TE + CL, other TE values ⇒ `400`), conflicting lengths `400`, read under 5 s; every other route keeps `parse_request_head` semantics (`413 body_not_allowed`) | tests 11–13 | ⬜ |
| RMC30 | WebAuthn registration (L3 §7.1 subset): `none` only, ES256 only, strict COSE, UV required, rpIdHash, flags, id bounds, no trailing bytes | tests 16–18 | ⬜ |
| RMC31 | WebAuthn assertion (L3 §7.2 subset): userHandle required, 37-byte authData, UP+UV, BE/BS rules, DER signature over raw bytes, high-S accepted, signCount `0/0` accepted and regression refused; §16.2 vector refused for missing UV | tests 19–22 | ⬜ |
| RMC32 | Challenges: 32 random bytes, single use (removed on first attempt), purpose/path/binding scoped, 120 s TTL, 4 per authenticated pool without eviction; anonymous login pool 16 with oldest-first eviction and ≤ 2 per client hint (rev 2); register challenges carry the pending user handle (rev 2); constant-time compare | tests 23, 24, 24a, 37 | ⬜ |
| RMC33 | Web sessions: `__Host-soos_session` with `Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=28800`, token stored only as SHA-256, 15 min idle / 8 h absolute, at most 4, logout, revocation on passkey removal checked before every validation, rotation on login, SSE keep-alive never refreshes idle time (rev 2), SSE ends with the session | tests 25, 26, 36, 38, 44 | ⬜ |
| RMC34 | Enrollment (D-E): tailnet + allowed identity + local one-time code (`soos-remote enroll-code`, 50 bits, 5 min, single use, 3 attempts, `0600` file in the `0700` socket dir); impossible over Funnel; the first registration writes the user handle sent in its options, a conflicting handle is refused (`409 registration_conflict`, rev 2) | tests 30, 31, 41, 41a, 41b | ⬜ |
| RMC35 | Credential store: `0600`, owner, `O_NOFOLLOW`, ≤ 16 KiB, ≤ 4 passkeys, `flock`, atomic temp+rename+fsync, cache by file stamp; `passkeys list/remove` | tests 27–29, 44 | ⬜ |
| RMC36 | Unlock (D-C): a fresh `unlock`-purpose UV assertion on every path (tailnet and Funnel), Funnel also needs a session; UV-clear and login-purpose assertions refused end to end with zero logind calls (rev 2); order disabled → body → assertion → persistence (store failure ⇒ `503`, not counted, rev 2) → unchanged unlock flow; superseded assertions listed in the ADR | tests 39, 39a, 40, 43 and the §10.7 rewritten tests | ⬜ |
| RMC37 | Limiters (rev 2): 10 options / 60 s and 5 failures / 5 min per `LimitKey` (`Tailnet`, `FunnelSession`, `FunnelAnonymous(hint)`); anonymous traffic never locks a session holder, the tailnet or another hint; the client hint comes only from the tailscaled-set `X-Forwarded-For`, is never an authorization input and is never logged; ≤ 64 hint buckets | tests 42, 56, 57, 58, 59, 53a | ⬜ |
| RMC38 | Fail-closed and privacy (D-H): store/RNG errors ⇒ `503`, sessions dropped; no challenge, token, cookie, credential id, key, user handle, code or identity in logs, error bodies or `Debug` | tests 22, 43, 45, 48 | ⬜ |
| RMC39 | Static contracts RMC-S13–RMC-S21 (rev 2: exact crate-name dependency check, `rand_core` allowed; `X-Forwarded-For` only in `client_hint`): pure-Rust dependencies (`p256 0.13.2`, no OpenSSL/ring/aws-lc/webauthn-rs/rand), one documented `der` skip, `getrandom` only, cookie attributes, modal WebAuthn page without storage, documentation and ADR, subcommands refuse root; `cargo deny --locked check` clean | tests 46–53, 53a; `cargo deny --locked check` | ⬜ |
| RMC40 | Hardware (owner): with `rp_id` set and `allow_funnel = true`, after `tailscale funnel --bg unix:<socket>`: enrollment over the tailnet with a local code and Face ID; with the VPN off, Safari and the home-screen app log in with Face ID, show the status, lock, and unlock with Face ID; with the VPN on, status/lock stay one-tap and unlock asks for Face ID; every passkey POST (login, unlock, registration) succeeds over Funnel and over the tailnet whatever body framing Safari/tailscaled use (rev 2, F-9); `tailscale funnel status` shows only port 443 |
| RMC41 *(rev 2, F-2)* | Class capacity: Funnel ≤ 8 of 16 connections (tailnet keeps ≥ 8), anonymous Funnel ≤ 4, anonymous body reads ≤ 2 (1 per hint), Funnel SSE ≤ 2 of 4; refusals `503 busy`, answered before any body read, never counted as failures; residual DoS stated in the ADR | tests 54, 55 | ⬜ | Manual check on the owner's iPhone and PC (`Docs/REMOTE_COMPANION.md`) | ⬜ Pending (owner) |

---

## 13. ADR text (added to `AI/DECISIONS.md`)

See the entry "[2026-10-06] Tailscale Funnel Access and In-House Passkey Authentication for
`soos-remote`" in `AI/DECISIONS.md`; it is the normative version of this section.

---

## 14. Documentation drift

- `Docs/REMOTE_COMPANION.md` §2 ("never run `tailscale funnel`"), §2a ("Accepted risk: the only
  authentication is the Tailscale identity … no passkey"), §4 Shortcuts ("Unlock PC"), §5 table,
  §6 table, §7 rows, §9 out-of-scope ("A second factor for the remote unlock") all contradict this ADR and
  must be rewritten in the traceability phase (keeping the invariant needles listed in §7 (rev 2, F-8),
  §9.3 and §10.8; the §2a "Accepted risk" paragraph is rewritten, never deleted, and also states the
  residual DoS of §5.2 and the local same-uid threat of §2.2 item 9).
- `AI/ARCHITECTURE.md` §13 table: the unlock row ("Authentication is the Tailscale identity alone") and
  the bounds row must be updated.
- `AI/DECISIONS.md` ADR 2026-10-05 item (8) and ADR 2026-10-06 item (2) carry a supersession note pointing
  to the new ADR (added with the ADR).
- `AI/VERIFICATION_MATRIX.md` component intro sentence ("never unlocks anything") is stale since ADR
  2026-10-06 and must be corrected with RMC26–RMC41.

## 15. Latency budget

Not on the PAM / daemon authentication path (no PAM, IPC or camera change). Service-local bounds:
head 5 s + body 5 s + verification (one ECDSA P-256 verify, < 1 ms) + store write (bounded lock wait
500 ms) + the unchanged unlock flow (2 s). The iPhone gesture window (10 s) covers one options round trip
before `navigator.credentials.get/create`.

## 16. Exit criteria

Every owner decision maps to: D-A → §2, S-1, S-2, RMC26/27/40; D-B → §4.1, RMC30/31; D-C → §5.1, RMC36;
D-D → §5, §6, RMC27/33; D-E → §4.5, §9.1, RMC34; D-F → §5.1 step 10 (unchanged flow, no re-lock);
D-G → §3.1, §3.4, §6 capacity, RMC29/37/39/41; D-H → §4 (redacted `Debug`), RMC38; D-I → §9.2 (installer never runs
`tailscale`), deployment left to the owner. Every new field and collection has a bound in §3.1.
