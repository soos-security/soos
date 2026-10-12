# Architect Spec — soos-remote: direct biometric prompt on Android (GitHub #354, sub-issue 1)

- Issue: GitHub #354 (GitHub-only, no `AI/BACKLOG.md` entry; commits say `Refs #354`; the branch is
  not registered in `scripts/sync_issue.py`).
- Branch: `feat/remote-direct-biometric`. Spec slug `remote_direct_biometric`. Walkthrough 195
  (`AI/walkthroughs/195_remote_direct_biometric.md`). Matrix component `remote-direct-biometric`,
  prefix `RDB`.
- Owner decisions (issue comments of 2026-10-12, "Owner decisions" parts 1 and 2, and "Oui, lance"):
  - OD-1: device-bound **and** password-manager passkeys stay accepted; no `BE=0` enforcement.
  - OD-2: remote unlock must keep working from anywhere over Funnel (no VPN on the phone).
  - OD-3: the **login** ceremony keeps the discoverable flow: no credential id in the public,
    unauthenticated login options (the provider picker may still appear at sign-in).
  - OD-4: the **unlock** and **camera-start** ceremonies, which already require a session, send
    `allowCredentials` so Android goes straight to the biometric prompt.
  - OD-5: the docs explain how to set the preferred passkey service on Android and how to delete a
    soos passkey saved in a third-party manager (Proton Pass) and re-enroll one stored on the phone.
  - OD-6: merges only after explicit owner approval (this run commits locally only).
- New ADR: "[2026-10-12] Named Credentials on Authenticated Passkey Ceremonies of `soos-remote`"
  (`AI/DECISIONS.md`). It amends, without superseding, the 2026-10-06 sentence "`allowCredentials`
  never sent (no credential id ever reaches an anonymous client)": the parenthetical invariant is
  kept word for word; `allowCredentials` is now sent on the two authenticated ceremonies only.

## 0. Summary of decisions

| # | Decision |
|---|---|
| D-1 | `POST /api/auth/unlock/options` and `POST /api/auth/camera/options` answer `200 {allow_credentials, challenge, rp_id, timeout_ms}`; `allow_credentials` lists **every** registered credential (store order, at most `MAX_PASSKEYS` = 4) as `{"id": <b64url>, "transports": [..]}`. |
| D-2 | `POST /api/auth/login/options` is byte-for-byte unchanged in shape: exactly `{challenge, rp_id, timeout_ms}`, never a credential id (OD-3). Enforced at the type level: `issue_options` takes an `OptionsCredentials` argument and the login handler passes `OptionsCredentials::Discoverable`. |
| D-3 | Transports per entry: the record's stored transports in canonical order when non-empty, otherwise the fixed default `["internal", "hybrid"]` (`TransportSet::DEFAULT_ALLOW`, single source of truth). |
| D-4 | Registration: `authenticatorSelection.authenticatorAttachment = "platform"` and top-level `hints: ["client-device"]` (ignored by browsers that do not know them); `residentKey: "required"`, `requireResidentKey: true`, `userVerification: "required"`, `attestation: "none"`, ES256 unchanged. |
| D-5 | The page sends `getTransports()` with the registration (optional `transports` field). The server bounds it (≤ 8 entries, each ≤ 16 bytes, else `400 bad_request`), keeps only the six WebAuthn L3 names, de-duplicates, and stores the set in the record (optional on-disk key `transports`, omitted when empty, strictly validated on read). Old store files and old request bodies stay valid. |
| D-6 | WebAuthn verification is unchanged: UV required, `userHandle` required, BE/BS rules, counter rule, origin, `crossOrigin`/`topOrigin`, challenge purpose/class/binding. The allow list is a UX hint only; the server never trusts it (the assertion's credential id is still looked up in the store). |
| D-7 | Funnel gates are unchanged: `UnlockOptions` and `CameraOptions` are not in `is_funnel_public`, so an anonymous Funnel caller gets `403 login_required` before routing and never sees a credential id. On the tailnet the caller already passed the `allowed_logins` identity check. |
| D-8 | CSP, CSRF (`X-Soos-Action` + required `Origin`), limiter keys, RC-1, `MAX_AUTH_BODY_BYTES`, the response headers and the asset set are unchanged. |

## 1. Verified assumptions (against the code at `fb2977c`)

| # | Assumption | Evidence |
|---|---|---|
| V-1 | `getAssertion` sends no `allowCredentials` on any ceremony; the three call sites are `requestUnlock` (unlock), `requestLogin` (login), `startCameraView` (camera). | `crates/remote/assets/app.js` lines 270–279, 771, 820, 1686. |
| V-2 | Registration sets no `authenticatorAttachment` and no `hints`. | `app.js` lines 862–879. |
| V-3 | The three options routes share `Shared::issue_options`, which already loads a store snapshot under the auth mutex (`Ok(Some(file)) if !file.passkeys.is_empty()`), then calls `options_response(rp_id, challenge)`. | `crates/remote/src/server.rs` `issue_options` (≈546–582), `options_response` (≈1378). |
| V-4 | Funnel: every route outside `is_funnel_public` (`Asset`, `AuthState`, `LoginOptions`, `LoginVerify`, `Logout`, `NotFound`, `MethodNotAllowed`) is refused `403 login_required` without a valid session **before** routing; `RegisterOptions`/`RegisterVerify` are always `403 forbidden` over Funnel. `UnlockOptions` and `CameraOptions` are therefore never reachable unauthenticated over Funnel. | `server.rs` `handle_connection` (≈1190–1215), `routes.rs` `is_funnel_public` (≈379). |
| V-5 | Tailnet requests reach routing only after the `Tailscale-User-Login` allowlist check. | `identity.rs` classification, `handle_connection` (≈1100–1160). |
| V-6 | `verify_assertion` requires `userHandle` and is independent of what the options named. | `webauthn.rs` `verify_assertion`. |
| V-7 | The store is `deny_unknown_fields` on both the file and each record; `PasskeyRecord` is a public struct with public fields built by literal in `server.rs` and in two test files. | `credentials.rs`; `crates/remote/tests/credentials_tests.rs:42`, `crates/remote/tests/auth_store_tests.rs:778`. |
| V-8 | `RegisterVerifyBody` is `deny_unknown_fields`; the page and the server ship together (embedded assets), so a new optional field cannot break a deployed page. | `auth.rs`, `assets.rs`. |
| V-9 | Body bound: `MAX_AUTH_BODY_BYTES` = 8192 ≥ 4/3·2048 + 4/3·1023 + 4/3·1024 + 256 = 5714 (compile-time assertion). The `transports` field adds at most 8 × (16 + 3) + 16 = 168 bytes; worst case 5882 < 8192. | `lib.rs` const assertions. |
| V-10 | Store bound: the largest legal record set today (4 × 1023-byte ids) encodes well under `MAX_CREDENTIAL_STORE_BYTES` = 16 384 (test 27); six transport names add ≤ 70 bytes per record (≤ 280 bytes total). | `credentials_tests.rs` test 27. |
| V-11 | Existing tests that pin the old behaviour and must be amended: unlock options key set and "no credential id" (auth_server test 39), camera options key set (camera_server test 24), the static "never `allowCredentials`" needle (invariants test 51), and two `PasskeyRecord` literals. Login test 36 (keys + no credential id + no `allowCredentials`/`allow_credentials` text) is **unchanged** and keeps guarding OD-3. | §9. |

## 2. Scope and blast radius

| Area | Change |
|---|---|
| `crates/remote/src/credentials.rs` | `AuthenticatorTransport`, `TransportSet`, `PasskeyRecord.transports`, optional `transports` key in `RecordJson`, strict parse, `allow_credentials(&PasskeyFile)`. |
| `crates/remote/src/auth.rs` | `RegisterVerifyBody.transports`, `DecodedRegistration.transports`, bounded decoding. |
| `crates/remote/src/server.rs` | `OptionsCredentials`, `issue_options(.., credentials)`, `options_response(.., allow)`, login/unlock/camera handlers, `register_verify` stores the transports. `persist_counter` mutates in place (transports preserved). |
| `crates/remote/src/lib.rs` | `MAX_TRANSPORT_FIELD_ENTRIES`, `MAX_TRANSPORT_NAME_BYTES`, one compile-time body-bound assertion. |
| `crates/remote/assets/app.js` | `getAssertion(options, nameCredentials)`, three call sites, registration `authenticatorAttachment`/`hints`/`transports`. |
| `crates/remote/src/main.rs` (CLI) | none (`passkeys list` output unchanged; transports are not shown). |
| Daemon, PAM, push sender, packaging, installer | none. |
| Docs | `Docs/REMOTE_COMPANION.md` §2h (new items), §2b (one sentence), §6 table rows, §8 residual; `AI/ARCHITECTURE.md` §13 passkey row; `AI/DECISIONS.md` ADR; walkthrough 195. |

Consumers of `PasskeyRecord` literals: `server.rs` `register_verify`, `credentials.rs` `record_from`,
`credentials_tests.rs::record`, `auth_store_tests.rs` (Debug redaction test). Consumers of
`options_response` / `issue_options`: `server.rs` only.

## 3. Types and signatures

### 3.1 `crates/remote/src/credentials.rs`

```rust
/// A WebAuthn L3 `AuthenticatorTransport` value (§5.8.4). Declaration order is the
/// canonical output order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticatorTransport {
    /// "internal"
    Internal,
    /// "hybrid"
    Hybrid,
    /// "usb"
    Usb,
    /// "nfc"
    Nfc,
    /// "ble"
    Ble,
    /// "smart-card"
    SmartCard,
}

impl AuthenticatorTransport {
    /// Every value, canonical order.
    pub const ALL: [Self; 6] = [
        Self::Internal, Self::Hybrid, Self::Usb, Self::Nfc, Self::Ble, Self::SmartCard,
    ];
    /// The exact WebAuthn name.
    #[must_use]
    pub const fn as_str(self) -> &'static str;
    /// Exact, case-sensitive match of one of the six names; anything else is `None`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self>;
}

/// A set of transports (bit `i` = `ALL[i]`); bounded by construction (≤ 6 values, no
/// duplicates). The field is private so no bit outside `ALL` can be set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransportSet(u8);

impl TransportSet {
    /// No transport (legacy records, or a browser without `getTransports`).
    pub const EMPTY: Self;
    /// `{Internal, Hybrid}`: what an allow-list entry carries when the record has none.
    pub const DEFAULT_ALLOW: Self;
    /// `self` plus `transport`.
    #[must_use]
    pub const fn with(self, transport: AuthenticatorTransport) -> Self;
    #[must_use]
    pub const fn contains(self, transport: AuthenticatorTransport) -> bool;
    #[must_use]
    pub const fn is_empty(self) -> bool;
    /// Names in canonical (`ALL`) order.
    #[must_use]
    pub fn names(self) -> Vec<&'static str>;
}

pub struct PasskeyRecord {
    // ... existing fields unchanged ...
    /// Transports reported by the browser at registration (`EMPTY` when unknown).
    pub transports: TransportSet,
}

/// The `allow_credentials` value of the unlock and camera options (spec D-1, D-3): one
/// `{"id": <b64url id>, "transports": [<names>]}` per record, store order; a record with
/// `EMPTY` transports carries `TransportSet::DEFAULT_ALLOW`. Pure, never logs.
#[must_use]
pub fn allow_credentials(file: &PasskeyFile) -> serde_json::Value;
```

`RecordJson` gains
`#[serde(default, skip_serializing_if = "Vec::is_empty")] transports: Vec<String>`.
`record_from` validates it strictly: at most `AuthenticatorTransport::ALL.len()` entries, each
exactly one of the six names, no duplicate, else `StoreError::Malformed`. `encode_store` writes the
names in canonical order and omits the key when the set is empty, so a store without transports
encodes byte-identically to the current format (backward compatible both ways for such files).
`PasskeyRecord`'s `Debug` stays redacted.

### 3.2 `crates/remote/src/auth.rs`

```rust
pub struct RegisterVerifyBody {
    pub id: String,
    pub client_data_json: String,
    pub attestation_object: String,
    /// `getTransports()` as reported by the browser; absent or `null` ⇒ empty.
    #[serde(default)]
    pub transports: Option<Vec<String>>,
}

pub struct DecodedRegistration {
    // ... existing fields ...
    /// Allowlisted, de-duplicated transports.
    pub transports: TransportSet,
}
```

`decode_registration` rules for `transports`:

| Input | Result |
|---|---|
| key absent, or `null` | `TransportSet::EMPTY` |
| `[]` | `EMPTY` |
| more than `MAX_TRANSPORT_FIELD_ENTRIES` (8) entries | `BodyMalformed` (`400 bad_request`, counted) |
| any entry longer than `MAX_TRANSPORT_NAME_BYTES` (16) bytes | `BodyMalformed` |
| a non-string entry, or a non-array value | `BodyMalformed` (serde) |
| an unknown name of ≤ 16 bytes (e.g. a future WebAuthn value) | ignored (WebAuthn L3 §5.8.4: RPs ignore unknown values) |
| duplicates | collapsed |

The transports are UX hints with no security weight: they never influence verification.

### 3.3 `crates/remote/src/server.rs`

```rust
/// Whether an options response names the registered credentials (spec D-1, D-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptionsCredentials {
    /// Login: discoverable flow, no credential id (OD-3).
    Discoverable,
    /// Unlock and camera: every registered credential as `allow_credentials` (OD-4).
    Registered,
}

async fn issue_options(
    &self,
    key: LimitKey,
    class: PathClass,
    purpose: ChallengePurpose,
    binding: ChallengeBinding,
    credentials: OptionsCredentials,
) -> Response;

/// `allow` is `Some` only for `OptionsCredentials::Registered`.
fn options_response(
    rp_id: &str,
    challenge: &[u8; CHALLENGE_BYTES],
    allow: Option<serde_json::Value>,
) -> Response;
```

- `issue_options` builds `allow_credentials(&file)` from the **same** snapshot that passed the
  non-empty check, inside the same mutex hold (no second store read, no TOCTOU between the check
  and the list). The order of refusals is unchanged (`403 passkeys_not_configured` → `429
  rate_limited` → `409 no_passkey` / `503 store_unavailable` → `503 unavailable` → `429
  too_many_challenges`); no refusal ever carries a credential id.
- `login_options` passes `Discoverable`; `unlock_options` and `camera_options` pass `Registered`.
  No other change to their gates.
- `register_verify` sets `transports: registration.transports` on the new record (read before
  `drop(registration)`).

### 3.4 `crates/remote/src/lib.rs`

```rust
/// Most entries accepted in the registration `transports` field (WebAuthn defines six).
pub const MAX_TRANSPORT_FIELD_ENTRIES: usize = 8;
/// Longest accepted entry of the registration `transports` field, in bytes.
pub const MAX_TRANSPORT_NAME_BYTES: usize = 16;

const _: () = assert!(
    MAX_AUTH_BODY_BYTES
        >= 4 * MAX_ATTESTATION_OBJECT_BYTES / 3
            + 4 * MAX_CREDENTIAL_ID_BYTES / 3
            + 4 * MAX_CLIENT_DATA_JSON_BYTES / 3
            + MAX_TRANSPORT_FIELD_ENTRIES * (MAX_TRANSPORT_NAME_BYTES + 3)
            + 256
);
```

(The existing assertion stays as is; this is an additional one.)

### 3.5 `crates/remote/assets/app.js`

```js
// The six WebAuthn transport names (anything else is dropped).
const PASSKEY_TRANSPORTS = ["internal", "hybrid", "usb", "nfc", "ble", "smart-card"];

// A modal passkey assertion over a server challenge. With `nameCredentials`, the
// credentials listed by the server (unlock, camera) are passed as allowCredentials so the
// phone goes straight to the biometric prompt; sign-in never names one.
function getAssertion(options, nameCredentials) { ... }
```

- `allowCredentials` is set only when `nameCredentials === true` **and**
  `Array.isArray(options.allow_credentials)` **and** it is non-empty; at most 4 entries are used
  (`MAX_PASSKEYS`); each entry is `{ type: "public-key", id: fromBase64Url(e.id), transports }`
  with `transports` filtered to `PASSKEY_TRANSPORTS` (an entry whose `id` is not a string is
  skipped). The literal `allowCredentials` appears exactly once in `app.js`.
- Call sites: `requestLogin` → `getAssertion(options.body, false)`; `requestUnlock` and
  `startCameraView` → `getAssertion(options.body, true)`.
- Registration `publicKey` adds `hints: ["client-device"]` and, inside
  `authenticatorSelection`, `authenticatorAttachment: "platform"` (existing keys unchanged).
- Registration body adds `transports: credentialTransports(response)`:
  `typeof response.getTransports === "function"`, called inside `try`/`catch` (any throw ⇒
  `[]`), only string values from `PASSKEY_TRANSPORTS`, at most 8.
- No storage, no `innerHTML`, no `eval(`, no conditional mediation (unchanged rules).

## 4. Constants and sentinel semantics

| Name | Location | Value | Sentinels |
|---|---|---|---|
| `TransportSet::DEFAULT_ALLOW` | `credentials.rs` | `{Internal, Hybrid}` → `["internal","hybrid"]` | used whenever a record's set is `EMPTY` |
| `TransportSet::EMPTY` | `credentials.rs` | no bit | legacy record, absent/`null`/`[]` field, browser without `getTransports` |
| `MAX_TRANSPORT_FIELD_ENTRIES` | `lib.rs` | 8 | 0 entries = `EMPTY`; 9+ = `400` |
| `MAX_TRANSPORT_NAME_BYTES` | `lib.rs` | 16 | `""` is an unknown name (ignored); 17+ bytes = `400` |
| `MAX_PASSKEYS` | `lib.rs` (unchanged) | 4 | bounds `allow_credentials` |
| `PASSKEY_TRANSPORTS` | `app.js` | the six names | mirrors `AuthenticatorTransport::ALL` (pinned by an invariant test) |

## 5. Error taxonomy

No new error variant and no new response code.

| Condition | Result |
|---|---|
| malformed `transports` in register verify | `400 bad_request`, counted on `LimitKey::Tailnet` (existing `BodyMalformed` path) |
| malformed `transports` in the store file | `StoreError::Malformed` → every web session dropped, passkey routes `503 store_unavailable` (existing fail-closed path) |
| empty store at unlock/camera options | `409 no_passkey` (unchanged; no `allow_credentials`) |
| anonymous Funnel caller at unlock/camera options | `403 login_required` before routing (unchanged) |

Fail-closed: no new path can authorize. The allow list never reaches `check_assertion`.

## 6. Latency budget

Not on the PAM auth path. The options handlers add one in-memory iteration over ≤ 4 records and
≤ 4 base64url encodings of ≤ 1023 bytes under the auth mutex (microseconds). Response size grows by
at most 4 × (1364 + 60) ≈ 5.7 KiB; no bound on responses is affected.

## 7. Invariants touched

| Invariant | Status |
|---|---|
| ADR 2026-10-06 "no credential id ever reaches an anonymous client" | **kept**: login options unchanged (D-2); unlock/camera options unreachable anonymously (D-7). |
| ADR 2026-10-06 "`allowCredentials` never sent" | **amended** by the new ADR (authenticated ceremonies only). |
| RMC-S18 (modal WebAuthn, UV, discoverable credentials, attestation none, ES256, no storage) | kept; the `allowCredentials` needle is re-scoped (AM-3). |
| Privacy row of `AI/ARCHITECTURE.md` §13 (no credential id logged) | kept: no new log line; `debug!` lines never carry the list. |
| CSP, CSRF, Funnel gates, RC-1, limiter keys, challenge binding | unchanged. |
| Store file rules (`0600`, owner, `O_NOFOLLOW`, ≤ 16 KiB, `flock`, atomic rename) | unchanged; one optional key. |

## 8. Test plan (Phase 2; all new tests must fail before the implementation)

### 8.1 Static — new `tests/invariants/src/remote_direct_biometric_contract.rs` (registered in `tests/invariants/src/lib.rs`)

- `test_rdb_s1_login_ceremony_stays_discoverable`: `login_options` calls `issue_options` with
  `OptionsCredentials::Discoverable`; `requestLogin` calls `getAssertion(options.body, false)`;
  `allowCredentials` occurs exactly once in `app.js`, inside `function getAssertion`, guarded by
  `nameCredentials`.
- `test_rdb_s2_unlock_and_camera_name_the_registered_credentials`: `unlock_options` and
  `camera_options` pass `OptionsCredentials::Registered`; `requestUnlock` and `startCameraView`
  call `getAssertion(options.body, true)`; `OptionsCredentials::Registered` appears in no other
  handler.
- `test_rdb_s3_registration_prefers_the_platform_authenticator`: `app.js` contains
  `authenticatorAttachment: "platform"`, `hints: ["client-device"]`, `getTransports`, and still
  `residentKey: "required"`, `requireResidentKey: true`, `attestation: "none"`, `alg: -7`.
- `test_rdb_s4_transport_names_single_sourced`: the six names of `PASSKEY_TRANSPORTS` in `app.js`
  equal the `as_str` names of `AuthenticatorTransport` in `credentials.rs`, same order.
- `test_rdb_s5_options_routes_are_not_funnel_public`: `is_funnel_public` lists neither
  `Route::UnlockOptions` nor `Route::CameraOptions`.
- `test_rdb_s6_docs_adr_and_walkthrough`: `Docs/REMOTE_COMPANION.md` §2h contains
  `preferred passkey`, `Proton Pass`, `soos-remote enroll-code`, `RDB10` and the previous
  needles; §6 mentions `allow_credentials`; `AI/DECISIONS.md` has the ADR title;
  `AI/walkthroughs/195_remote_direct_biometric.md` exists.

### 8.2 Behaviour — new `crates/remote/tests/direct_biometric_tests.rs` (harness of `auth_server_tests.rs` / `camera_server_tests.rs` via `tests/common`)

- `test_rdb_unlock_options_name_every_registered_credential`: tailnet and Funnel-with-session,
  two passkeys (one with stored transports `{Hybrid, Internal}` written by registration, one legacy
  without): exact keys `["allow_credentials","challenge","rp_id","timeout_ms"]`; exact array in
  store order; legacy entry carries `["internal","hybrid"]`.
- `test_rdb_camera_options_name_every_registered_credential`: same for `/api/auth/camera/options`.
- `test_rdb_login_options_never_name_a_credential`: with two passkeys and stored transports, login
  options keys exactly `["challenge","rp_id","timeout_ms"]`; the body contains no credential id of
  either passkey, no `allow`, no `transports`.
- `test_rdb_named_options_refused_without_funnel_session`: Funnel without cookie, with an unknown
  cookie, and with a session revoked by removing its passkey: both routes answer `403
  login_required`, body contains no credential id; no challenge issued.
- `test_rdb_refusals_never_name_a_credential`: `409 no_passkey`, `429 rate_limited`, `403
  unlock_disabled`, `403 camera_disabled` bodies contain no `allow_credentials`.
- `test_rdb_register_stores_allowlisted_transports`: register with `["hybrid","internal","usb",
  "future-x","internal"]` ⇒ stored `{Internal, Hybrid, Usb}`, store file key `transports` =
  `["internal","hybrid","usb"]`, next unlock options entry carries the same list.
- `test_rdb_register_without_transports_is_accepted`: the pre-change body (no key) and `null` both
  register; the record has `EMPTY`; the store file has no `transports` key.
- `test_rdb_register_transports_bounds`: 9 entries, a 17-byte entry, a number entry, an object
  value ⇒ `400 bad_request`, counted (5 such ⇒ `429`), nothing stored, enrollment code kept.
- `test_rdb_counter_update_keeps_transports`: a device-bound passkey with transports; after an
  unlock with a higher counter the record keeps its transports.
- `test_rdb_assertion_rules_unchanged_with_allow_list`: after named options, an assertion without
  `userHandle`, with UV clear, or by a non-registered credential id is `403 passkey_rejected`.
- `test_rdb_options_never_log_a_credential_id`: captured logs of unlock/camera options contain no
  base64url credential id.

### 8.3 Unit — `crates/remote/tests/credentials_tests.rs` (new tests only)

- `test_rdb_transport_set_canonical_and_bounded`: `from_name` exact and case-sensitive; `names()`
  canonical; `DEFAULT_ALLOW.names() == ["internal","hybrid"]`; every set has ≤ 6 names.
- `test_rdb_store_transports_roundtrip_and_strict_parse`: legacy file parses with `EMPTY`; a file
  with transports round-trips; an empty set encodes without the key (byte-identical to the legacy
  encoding of the same record); unknown name, duplicate, 7 entries, non-array, non-string ⇒
  `Malformed`; the largest store (4 × 1023-byte ids, all six transports) ≤ 16 KiB.
- `test_rdb_allow_credentials_shape`: store order, ids base64url, default transports for `EMPTY`.

### 8.4 Test hooks

Existing seams only: `Server::with_credentials_path`, `with_random`, the `Authenticator` helpers
of `tests/common/passkey.rs` (a `register_body_with_transports` helper may be added there; adding a
helper is not an amendment), `LogCapture`, `store_json` (unchanged; a new helper writes records
with transports).

## 9. Amendments to existing tests (exhaustive)

| # | File / test | Exact change | Kind | Justification |
|---|---|---|---|---|
| AM-1 | `crates/remote/tests/auth_server_tests.rs`, test 39 `test_rmc_unlock_requires_a_fresh_passkey_on_every_path` (≈ lines 675–679) | (a) `vec!["challenge", "rp_id", "timeout_ms"]` → `vec!["allow_credentials", "challenge", "rp_id", "timeout_ms"]`; (b) `assert!(!String::from_utf8_lossy(&options.body).contains(&h.owner.credential_id_b64()));` → `assert_eq!(options.json()["allow_credentials"], serde_json::json!([{"id": h.owner.credential_id_b64(), "transports": ["internal", "hybrid"]}]));` | exact value replacement **of a disclosure assertion** (security-relevant) | OD-4 requires the unlock options of an authenticated caller to name the registered credentials. The replacement is stricter than a removal (exact content). The anonymous non-disclosure property it protected is kept by login test 36 (unchanged) and by the new `test_rdb_named_options_refused_without_funnel_session`. |
| AM-2 | `crates/remote/tests/camera_server_tests.rs`, test 24 `test_rlc_start_requires_fresh_uv_camera_assertion` (≈ line 248) | `vec!["challenge", "rp_id", "timeout_ms"]` → `vec!["allow_credentials", "challenge", "rp_id", "timeout_ms"]` | exact value replacement | OD-4 (camera start). |
| AM-3 | `tests/invariants/src/remote_passkey_contract.rs`, test 51 `test_rmc_s18_page_uses_modal_webauthn_without_storage` | the entry `"allowCredentials",` is removed from the `forbidden` list and replaced, in the same test, by `assert_eq!(app.matches("allowCredentials").count(), 1, "allowCredentials only in getAssertion");` plus `assert!(app.contains("getAssertion(options.body, false)"), "sign-in stays discoverable");`; the doc comment "never `allowCredentials`" becomes "`allowCredentials` only for the authenticated unlock and camera ceremonies, never at sign-in" | re-scoping of a static needle (security-relevant) | OD-3/OD-4: the literal is required by WebAuthn for the two authenticated ceremonies; the original intent (S-3: sign-in names no credential) is preserved by an exact count and a call-site pin, and by `test_rdb_s1`. |
| AM-4 | `crates/remote/tests/credentials_tests.rs`, helper `record` (≈ line 42) | add the field `transports: Default::default(),` | setup only, strictly additive | New public field of `PasskeyRecord`; no assertion touched. |
| AM-5 | `crates/remote/tests/auth_store_tests.rs`, `PasskeyRecord` literal of the Debug-redaction test (≈ line 778) | add the field `transports: Default::default(),` | setup only, strictly additive | Same. |

No other existing test changes. If the plan evaluator classifies AM-1(b) or AM-3 as a relaxation of
a security assertion rather than an owner-required exact replacement, the run must stop and report
to the owner instead of applying them.

## 10. Documentation to update

- `Docs/REMOTE_COMPANION.md`
  - §2h item 5 ("Passkey"): keep the existing sentences (pinned needles: "Google Password
    Manager", "enroll-code"); add: the unlock and camera prompts go straight to fingerprint/face
    because the page names the registered passkey; sign-in may still show a picker.
  - §2h new item "Preferred passkey service": Android 14+: Settings → Passwords, passkeys &
    accounts (Samsung: Settings → General management → Passwords, passkeys and autofill) →
    **Preferred service** → Google Password Manager (or Samsung Pass); turn off other providers
    under "Additional services" if the picker still appears. Android 13 and older: passkeys live in
    Google Password Manager only.
  - §2h new item "Move a passkey out of Proton Pass": (1) in Proton Pass, open the item for the
    soos host and delete its passkey; (2) on the PC: `soos-remote passkeys list` then
    `soos-remote passkeys remove <n>` for the old one (or keep it until the new one works);
    (3) set the preferred service as above; (4) over the tailnet run `soos-remote enroll-code`
    and enroll from the phone (the page asks for a platform passkey); (5) check unlock shows the
    fingerprint prompt directly.
  - §2h item 11 owner check: add step "(8) RDB10".
  - §2b: one sentence on the named credentials of unlock/camera and the discoverable sign-in.
  - §6 table: unlock and camera options `200 {allow_credentials, challenge, rp_id, timeout_ms}`;
    login options unchanged; register verify accepts optional `transports`.
  - §8: residual R-1/R-2 below.
- `AI/ARCHITECTURE.md` §13 Passkeys row: "discoverable credentials; `allowCredentials` only on the
  authenticated unlock and camera ceremonies; platform attachment and `client-device` hint at
  registration; stored transports".
- `AI/DECISIONS.md`: the ADR (written in this phase).
- `AI/walkthroughs/195_remote_direct_biometric.md` (traceability phase).

## 11. Verification matrix rows (component `remote-direct-biometric`)

| # | Criterion | Planned tests | Status |
|---|---|---|---|
| RDB1 | Unlock options of a tailnet caller or a Funnel session name every registered credential (`allow_credentials`, store order, stored or default `["internal","hybrid"]` transports) | `direct_biometric_tests::test_rdb_unlock_options_name_every_registered_credential`, `auth_server_tests` test 39 (AM-1) | Pending |
| RDB2 | Camera options name every registered credential the same way | `direct_biometric_tests::test_rdb_camera_options_name_every_registered_credential`, `camera_server_tests` test 24 (AM-2) | Pending |
| RDB3 | Login options stay exactly `{challenge, rp_id, timeout_ms}` with no credential id; the page signs in without `allowCredentials` | `test_rdb_login_options_never_name_a_credential`, `auth_server_tests` test 36 (unchanged), `test_rdb_s1_login_ceremony_stays_discoverable`, test 51 (AM-3) | Pending |
| RDB4 | Unlock and camera options are never reachable without a Funnel session; no refusal names a credential | `test_rdb_named_options_refused_without_funnel_session`, `test_rdb_refusals_never_name_a_credential`, `test_rdb_s5_options_routes_are_not_funnel_public` | Pending |
| RDB5 | Registration asks for a platform authenticator with the `client-device` hint, discoverable, UV, attestation none, ES256 | `test_rdb_s3_registration_prefers_the_platform_authenticator` | Pending |
| RDB6 | Registration transports are bounded, allowlisted, de-duplicated and stored; old bodies still register | `test_rdb_register_stores_allowlisted_transports`, `test_rdb_register_without_transports_is_accepted`, `test_rdb_register_transports_bounds`, `test_rdb_s4_transport_names_single_sourced` | Pending |
| RDB7 | Store: optional `transports` key, strict on read, omitted when empty, legacy files valid, ≤ 16 KiB; counter updates keep transports | `credentials_tests::{test_rdb_transport_set_canonical_and_bounded, test_rdb_store_transports_roundtrip_and_strict_parse, test_rdb_allow_credentials_shape}`, `test_rdb_counter_update_keeps_transports` | Pending |
| RDB8 | WebAuthn assertion rules unchanged; no credential id logged | `test_rdb_assertion_rules_unchanged_with_allow_list`, `test_rdb_options_never_log_a_credential_id` | Pending |
| RDB9 | Docs (§2h preferred service and Proton Pass re-enrollment, §6, §8), ARCHITECTURE §13, ADR and walkthrough 195 | `test_rdb_s6_docs_adr_and_walkthrough` | Pending |
| RDB10 | Hardware (owner): on the owner's Android phone, after re-enrolling a passkey stored on the phone, **unlock** and **camera start** show the fingerprint/face prompt directly with no provider picker, over the tailnet and over Funnel; sign-in may still show the picker; the iPhone flow still works | Owner hardware check (`Docs/REMOTE_COMPANION.md` §2h) | Pending (owner hardware) |

## 12. Assumptions (minor choices decided here)

- A-1: Default transports `["internal","hybrid"]` (task wording); "internal" routes Chrome Android to
  Credential Manager, "hybrid" keeps cross-device use possible.
- A-2: Unknown transport names are ignored, not refused (WebAuthn forward compatibility); only the
  size bounds are refused.
- A-3: `soos-remote passkeys list` does not show transports (no CLI change; transports carry no
  security meaning).
- A-4: `authenticatorAttachment: "platform"` applies to every new enrollment; enrolling a FIDO2
  hardware key (cross-platform) is deferred to the device-bound sub-issue of #354, which will add
  its own option. Already-registered credentials of any kind keep working.
- A-5: Existing passkeys are not migrated; they get the default transports until re-enrolled.
- A-6: Downgrade: an older `soos-remote` refuses a store file containing a `transports` key
  (`deny_unknown_fields`) and fails closed (`503 store_unavailable`). Accepted; documented in the
  walkthrough (rollback = re-enroll, or delete the key by hand).

## 13. Residual risks

- R-1: A tailnet caller with an allowed login, or a Funnel caller holding a valid session, learns the
  registered credential ids. Both already authenticated (the Funnel session was obtained with one of
  those passkeys); a credential id is an opaque public identifier, not a secret; it does not let an
  attacker sign. Accepted by OD-4.
- R-2: Android may still show a picker when the passkey sits in a third-party provider and several
  providers are enabled; the docs (§2h) explain the preferred service and the re-enrollment.
  Sign-in keeps the picker by design (OD-3).
- R-3: `userHandle` stays required: an authenticator that omits it when `allowCredentials` is set
  would be refused (fail-closed). All registered credentials are discoverable (`residentKey:
  "required"`), which return the user handle; covered by RDB10 on hardware.
