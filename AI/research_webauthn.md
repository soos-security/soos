# Research: Minimal Server-Side WebAuthn (Passkeys) for `soos-remote` over Tailscale Funnel

- **Date**: 2026-10-06
- **Branch**: `feat/remote-funnel-passkey` (from `feat/remote-companion` at `b5d593a`, GitHub #339)
- **Scope**: research only. No code, no ADR. Feeds the architect spec for owner decisions D-A to D-I
  (Funnel reachability, in-house WebAuthn passkeys, fresh UV assertion per unlock, cookie sessions on
  the Funnel path, tailnet-only enrollment with a local one-time code, pure-Rust crypto, no logging of
  secrets).
- **Status of each claim**: **[spec]** quoted or paraphrased from a normative source, **[src]** read in
  third-party source code, **[verified]** reproduced locally on 2026-10-06 in a scratch copy (nothing
  in the repository was changed except this file), **[doc]** vendor documentation,
  **[report]** community report without vendor confirmation, **[rec]** recommendation of this research.

---

## 0. Summary of recommendations

1. **Implement WebAuthn in-house on RustCrypto `p256 = 0.13.2`** (`default-features = false,
   features = ["ecdsa"]`) plus the already-locked `sha2 0.10`, `ciborium 0.2`, `subtle 2.6`,
   `getrandom 0.4` and `base64ct 1.8`. Do **not** use `webauthn-rs` (hard `openssl` dependency, MPL-2.0
   licence not in `deny.toml`, `base64 0.21` and `thiserror 1` duplicates). `p256 0.14` is rejected
   because it drags `sha2 0.11`, `digest 0.11`, `rand_core 0.10` and `crypto-common 0.2` duplicates.
   The `0.13.2` choice adds exactly **one** duplicate (`der 0.7.10` next to `der 0.8.2`), which needs
   one documented `[bans] skip` entry; `cargo deny check bans licenses advisories` is otherwise green
   **[verified]** (section 4).
2. **Accept only ES256 (COSE alg `-7`), attestation `"none"`, user verification required on every
   ceremony.** Verify the full WebAuthn Level 3 §7.1/§7.2 procedures with the subset listed in sections
   1 and 2. The L3 test vector §16.2 ("ES256 Credential with No Attestation") was decoded and its
   assertion signature verified with exactly this crate stack **[verified]**.
3. **signCount**: iCloud Keychain synced passkeys return `0` forever. Accept `0`/`0`; only treat a
   non-increasing non-zero counter as a clone signal. Replay protection therefore rests entirely on
   **single-use, server-stored, purpose-bound, short-lived challenges** (section 2.4).
4. **RP ID = the full Funnel host** `<node>.<tailnet>.ts.net`, configured explicitly, never derived from
   a request header. `ts.net` **is** on the Public Suffix List, so each `<tailnet>.ts.net` is its own
   registrable domain and other Funnel users are cross-site **[verified]** (section 3).
5. **Path classification** (Funnel vs tailnet) uses the `Tailscale-Funnel-Request: ?1` header that
   `tailscaled` sets for Funnel traffic after stripping any client copy, exactly as it does for
   `Tailscale-User-Login` **[src]**, v1.102.4 matches `main`. A request carrying neither marker stays
   `403` as today, which keeps the current identity tests valid (section 3.4).
6. **Session cookie** for the Funnel path: `__Host-soos_session=<43-char base64url of 32 random bytes>;
   Path=/; Secure; HttpOnly; SameSite=Strict`, server stores only `SHA-256(token)`, in memory, bounded
   count, idle and absolute TTL. The existing `X-Soos-Action` + `Sec-Fetch-Site` + `Origin` rules stay
   mandatory on every state-changing route because sibling nodes of the same tailnet are *same-site*
   (section 5).
7. **iOS**: passkeys with Face ID work in Safari and in home-screen web apps (WebKit), from a button
   tap (`navigator.credentials.get/create` need a user gesture). Use the **modal** flow with
   `allowCredentials`, not conditional (autofill) mediation, which has open reports of misbehaviour
   in home-screen shortcuts. The existing CSP does not affect WebAuthn; it only requires the page JS
   to stay in `/app.js` (no inline script) (section 3.3). A physical iPhone check remains mandatory
   (it cannot be automated in CI).

---

## 1. Registration with attestation `"none"`

### 1.1 Options the server sends (`navigator.credentials.create`) [rec]

| Member | Value | Reason |
|---|---|---|
| `challenge` | 32 random bytes (`getrandom`), base64url in JSON | L3 §13.4.3: challenges "SHOULD therefore be at least 16 bytes long" **[spec]** |
| `rp.id` | configured `rp_id` (full node host) | section 3 |
| `rp.name` | `"soos"` | deprecated in L3 but still required by the IDL |
| `user.id` | 16 random bytes generated once and stored (user handle, no PII) | L3 §14.6.1 user handle contents |
| `user.name` / `displayName` | constant, e.g. `"soos"` | never the Tailscale login (D-H) |
| `pubKeyCredParams` | `[{type:"public-key", alg:-7}]` only | ES256 is what Apple platform authenticators produce |
| `authenticatorSelection` | `{residentKey:"required", userVerification:"required"}` (optionally `authenticatorAttachment:"platform"`) | D-B/D-C; iOS always creates discoverable credentials **[doc]** (WebKit blog) |
| `attestation` | `"none"` | no attestation needed; section 1.4 |
| `excludeCredentials` | ids of already-registered credentials | prevents duplicate registration on the same keychain |
| `timeout` | 120000 ms | section 2.4 (deliberate deviation from the 300000 ms default, owner decision) |

### 1.2 RP verification steps (subset of WebAuthn L3 §7.1, all mandatory) [spec]

Inputs: `id`/`rawId` (base64url), `response.clientDataJSON`, `response.attestationObject`.

1. Decode `clientDataJSON` as UTF-8 JSON (serde_json into a struct that **ignores unknown keys**,
   L3 §5.8.1 note: "it's critical when parsing to be tolerant of unknown keys and of any reordering").
   The §16.2 vector really contains an `extraData` key **[verified]**. serde_json rejects duplicate
   known keys, which is desirable.
2. `C.type == "webauthn.create"`.
3. `C.challenge` base64url-decodes (no padding) to an outstanding, unexpired, **registration-purpose**
   challenge; consume it (remove from the store) on the first verification attempt whether it
   succeeds or fails.
4. `C.origin == "https://" + rp_id` exactly (L3 §13.4.9: "A web application served only at
   https://example.org SHOULD require origin to exactly equal https://example.org").
5. `C.crossOrigin` absent or `false`; `C.topOrigin` absent (both only appear for cross-origin
   iframes, which `frame-ancestors 'none'` already forbids).
6. `hash = SHA-256(clientDataJSON raw bytes)` (only needed for non-`none` formats; keep for symmetry).
7. CBOR-decode `attestationObject` into a map with exactly the text keys `fmt`, `attStmt`, `authData`.
8. `fmt == "none"` and `attStmt` is an empty map (L3 §8.7 syntax `fmt: "none", attStmt: emptyMap`).
   See 1.4 for `packed` self attestation.
9. Parse `authData` (layout 1.3):
   - `rpIdHash == SHA-256(rp_id)`.
   - `UP` (bit 0) set. (Conditional-mediation create is not used.)
   - `UV` (bit 2) set (RP requires user verification).
   - If `BE` (bit 3) is clear, `BS` (bit 4) must be clear.
   - `AT` (bit 6) set; `ED` (bit 7) clear **[rec]** (no extension is requested; L3 lets the RP "ignore
     the unsolicited extensions or reject"; rejecting keeps the parser trivial).
10. Attested credential data: `credentialIdLength <= 1023` (L3 §6.5.1 "Value MUST be ≤ 1023"; §7.1
    "Credential IDs larger than this many bytes SHOULD cause the RP to fail"); credential public key
    is a COSE_Key (1.5) with `alg == -7`, matching `pubKeyCredParams`.
11. **No trailing bytes** after the COSE key when `ED` is clear. Decode the COSE key with
    `ciborium::de::from_reader(&mut slice)` on a `&[u8]` so the slice remainder tells how many bytes
    were consumed.
12. `rawId` bytes equal the credential id inside `authData`.
13. Credential id not already registered (L3 §7.1: "If the credentialId is already known then the
    Relying Party SHOULD fail this registration ceremony").
14. Store the credential record: `id`, public key (x, y), `signCount`, `uvInitialized = UV`,
    `backupEligible = BE`, `backupState = BS`, creation time, optional label. Bound the number of
    records (e.g. 4) **[rec]**.

### 1.3 Authenticator data layout (L3 §6.1, §6.5.1) [spec]

| Offset | Length | Field |
|---|---|---|
| 0 | 32 | `rpIdHash` = SHA-256(RP ID) |
| 32 | 1 | flags: bit0 `UP`, bit1 RFU1, bit2 `UV`, bit3 `BE`, bit4 `BS`, bit5 RFU2, bit6 `AT`, bit7 `ED` |
| 33 | 4 | `signCount`, u32 big-endian |
| 37 | 16 | `aaguid` (only if `AT`) |
| 53 | 2 | `credentialIdLength` L, u16 big-endian, `<= 1023` |
| 55 | L | `credentialId` |
| 55+L | var | `credentialPublicKey`, COSE_Key, CTAP2 canonical CBOR |
| … | var | extensions CBOR map (only if `ED`) |

Minimum length is 37 bytes (assertion without extensions). Registration minimum is
`37 + 16 + 2 + L + |COSE|`. The §16.2 registration `authData` is 164 bytes with flags `0x59`
(`UP|BE|BS|AT`, UV clear) **[verified]**. Apple sets `aaguid` to all zeros **[doc]** (WebKit blog: the
AAGUID "is all zero even if the attestation is enabled"); L3 no longer zeroes it for `none`, so do not
check its value.

### 1.4 Attestation formats

- `"none"` is the default; L3 §5.4.7: "If the authenticator generates an attestation statement that
  is not a self attestation, the client will replace it with a None attestation statement." **[spec]**
  Hence a client *may* pass a **`packed` self attestation** through unchanged (§16.3 vector).
  Safari/iOS returns `none` for this request in practice. **[rec]** Accept only `fmt == "none"` with an
  empty `attStmt` for the first iteration (strict, fewer branches). If an owner device ever returns
  `packed`, an ADR can allow "`packed` with `attStmt` ignored, treated as no attestation", which L3
  §7.1 explicitly permits ("MAY register the credential … but treat the credential as one with self
  attestation"). Attestation adds no security in this design: enrollment is already gated by tailnet
  identity + local one-time code + challenge (D-E).

### 1.5 COSE_Key EC2 P-256 extraction (L3 §6.5.1.1, §5.8.5; RFC 9052/9053) [spec]

```
{ 1: 2,    ; kty EC2
  3: -7,   ; alg ES256
 -1: 1,    ; crv P-256
 -2: x,    ; bstr, 32 bytes
 -3: y }   ; bstr, 32 bytes
```

- L3 §5.8.5: "Keys with algorithm -7 (ES256) MUST specify 1 (P-256) as the crv parameter and MUST NOT
  use the compressed point form." So `-3` must be a 32-byte byte string, never a boolean.
- L3 §6.5.1: the COSE key "MUST contain the "alg" parameter and MUST NOT contain any other OPTIONAL
  parameters". **[rec]** Require exactly the five integer keys above; reject anything else.
- Build the key with `p256::EncodedPoint::from_affine_coordinates(x, y, false)` then
  `VerifyingKey::from_encoded_point`, which rejects off-curve points and the identity (L3 §5.8.5 note
  on on-curve checks) **[verified]** (API compiles and round-trips).
- Persist the key as the 65-byte SEC1 uncompressed point (or x||y); never log it (D-H).

---

## 2. Authentication assertion verification

### 2.1 Options (`navigator.credentials.get`) [rec]

`{challenge, rpId: rp_id, allowCredentials: [{type:"public-key", id} …registered ids],
userVerification: "required", timeout: 120000}`. With a single owner and a short allow list, no
username step and no discoverable-credential lookup are needed.

### 2.2 RP verification steps (subset of WebAuthn L3 §7.2) [spec]

Inputs: `rawId`, `clientDataJSON`, `authenticatorData`, `signature`, optional `userHandle`.

1. `rawId` is in `allowCredentials` and identifies a stored credential record (L3 §7.2: "verify that
   credential.id identifies one of the public key credentials listed in pkOptions.allowCredentials").
2. If `userHandle` is present it must equal the stored user handle (the user was "identified before
   the ceremony" by the allow list). iOS returns it for discoverable credentials.
3. `C.type == "webauthn.get"`; `C.challenge` matches an outstanding, unexpired challenge **of the
   expected purpose** (`login` or `unlock`), consumed on first use; `C.origin` exact;
   `crossOrigin` absent/false; `topOrigin` absent.
4. `rpIdHash == SHA-256(rp_id)`.
5. `UP` set; `UV` set (L3 §7.2: "If user verification was determined to be required, verify that the
   UV bit of the flags in authData is set"). D-C makes this unconditional.
6. If `BE` clear then `BS` clear; `BE` must equal the stored `backupEligible` (L3 §7.2: "If
   credentialRecord.backupEligible is set, verify that currentBe is set", and the converse).
7. `AT` clear and `ED` clear **[rec]**, `authenticatorData.len() == 37` exactly then.
8. **Signature**: `sig` is an ASN.1 DER `Ecdsa-Sig-Value` (L3 §6.5.5: "For COSEAlgorithmIdentifier -7
   (ES256) … the sig value MUST be encoded as an ASN.1 DER Ecdsa-Sig-Value"), over
   `authenticatorData || SHA-256(clientDataJSON)`, using the **raw received bytes** (never
   re-serialised). With `p256`: `Signature::from_der(sig)` then
   `VerifyingKey::verify(&msg, &sig)` (ECDSA with SHA-256 applied internally to `msg`).
   This exact computation validates the §16.2 vector **[verified]**.
   - `p256 0.13` / `ecdsa 0.16` accept **high-S** signatures **[verified]** (a high-S variant of a
     valid signature verified). This is required: WebAuthn does not mandate low-S and authenticators
     may emit high-S. Do not call `normalize_s` as a rejection criterion.
   - `Signature::from_der` is available with the feature set of section 4 **[verified]**; bound the
     input to 72 bytes before parsing.
9. **signCount** (L3 §7.2): "If authData.signCount is nonzero or credentialRecord.signCount is
   nonzero" then a value `<=` the stored one "is a signal, but not proof, that the authenticator may
   be cloned … Whether the Relying Party updates … or fails the authentication ceremony or not, is
   Relying Party-specific." **[spec]** iCloud Keychain synced passkeys return `0` on every assertion
   and never increment it **[report]** (multiple RP implementer write-ups; consistent with L3 §6.1.1
   "Authenticators that do not implement a signature counter leave the signCount … constant at
   zero"). **[rec]** Accept `0/0`. If either value is non-zero and `new <= stored`, fail the ceremony
   (fail-closed; no non-Apple authenticator is expected in this single-owner design) and log only the
   reason class. Otherwise store the new value.
10. Update `backupState = BS`. Do not change `uvInitialized` (always true here since UV is required at
    registration).

### 2.3 What `allowCredentials` / `userHandle` buy here

The allow list restricts the authenticator to the owner's registered credentials and avoids
username enumeration (L3 §14.6.2). `userHandle` is redundant with the allow list but cheap to check.
Credential ids and the user handle are identifiers in the D-H sense: never log them, never return
them except in the options JSON the owner's page needs.

### 2.4 Challenge store and replay (D-C, D-G) [rec]

- Synced passkeys give **no** counter-based replay protection, so every challenge must be:
  random 32 bytes; stored server-side (L3 §13.4.3: "the Relying Party SHOULD store the challenge
  temporarily until the operation is complete"); **single use** (removed on the first verification
  attempt, success or failure); **purpose-bound** (`register`, `login`, `unlock`) so a login
  assertion can never be replayed as an unlock; **path-bound** (Funnel vs tailnet) so internet traffic
  cannot exhaust the tailnet pool.
- Expiry: L3 §13.4.3 says challenges "SHOULD be valid for a duration similar to the upper limit of the
  recommended range" (300000–600000 ms, L3 §15.1). For a remote unlock, freshness matters more than
  accessibility; **[rec]** 120 s TTL and `timeout: 120000`, documented as a deliberate deviation in the
  ADR.
- Bounds: e.g. at most 4 outstanding challenges per pool; when full, refuse new ones with `429`
  rather than evicting (eviction would let an internet client cancel the owner's in-flight ceremony).
  Rate-limit challenge issuance and verification failures per pool (e.g. 5 failures / 5 min then
  `429` for the remainder of the window). Separate pools keep a Funnel flood from blocking the tailnet
  path.
- An unlock (D-C) needs a fresh `unlock`-purpose assertion **in the same request** as the unlock
  (`POST /api/unlock` body carries the assertion; the server verifies it, then calls logind), on both
  paths. A session cookie never substitutes for it.
- Compare stored challenge bytes with `subtle::ConstantTimeEq` (cheap; avoids any timing discussion).

### 2.5 Body bounds [rec]

The current server reads no request body. The new routes need `Content-Length` (reject
`Transfer-Encoding`, reject absent/duplicate/oversized lengths with `411`/`413`), JSON only.
Sizes: `clientDataJSON` from Safari is about 130–250 bytes, `authenticatorData` 37 bytes, DER
signature `<= 72` bytes, credential id `<= 1023` bytes, `none` attestation object a few hundred bytes;
base64url adds 4/3. **[rec]** `MAX_WEBAUTHN_BODY_BYTES = 8192`, per-field decoded limits
(`clientDataJSON <= 1024`, `attestationObject <= 2048`, `authenticatorData <= 512`,
`signature <= 72`, `credentialId <= 1023`, `userHandle <= 64`), and a body read deadline like
`REQUEST_HEAD_TIMEOUT_MS`. `ciborium` decoding of `attestationObject` runs on an already size-bounded
slice (its default recursion limit is 256), so memory is bounded by the input.

---

## 3. RP ID, Funnel and iOS

### 3.1 RP ID = full Funnel host [spec][verified]

- L3 §1 (RP ID definition): the RP ID "must be equal to the origin's effective domain, or a
  registrable domain suffix of the origin's effective domain", the origin must be `https` (or
  `http://localhost`). The full host `<node>.<tailnet>.ts.net` equals the effective domain, so it is
  always valid.
- **`ts.net` is on the Public Suffix List** (private section, "Tailscale Inc.", entries `ts.net` and
  `*.c.ts.net`) **[verified]** (`publicsuffix.org/list/public_suffix_list.dat`, fetched 2026-10-06).
  Consequences:
  - `ts.net` itself can never be an RP ID; `<tailnet>.ts.net` could be (it is a registrable domain),
    but it would let every node of the tailnet exercise the credential. **[rec]** Use the full host:
    narrowest scope.
  - Every tailnet is a different *site*; another person's Funnel at `x.other.ts.net` is
    **cross-site**, so `SameSite=Strict` and `Sec-Fetch-Site` mean something. Sibling nodes of the
    owner's own tailnet (`other.<tailnet>.ts.net`) are **same-site** but not same-origin.
- **[rec]** `rp_id` is an explicit config value (validated like `allowed_hosts`), and WebAuthn and
  Funnel routes additionally require `effective host == rp_id`. Never derive the RP ID from
  `X-Forwarded-Host`. Changing the node or tailnet name invalidates every passkey (documented).
- Serve and Funnel share the same MagicDNS name and certificate, so the iPhone sees the **same origin**
  whether it arrives over the tailnet (VPN on) or over Funnel (VPN off): one passkey covers both
  paths.

### 3.2 Funnel facts [doc][src]

- Funnel is available on all plans including the free Personal plan; ports 443, 8443 or 10000 only;
  names under `<tailnet>.ts.net` only; requires MagicDNS, HTTPS certificates and the `funnel` node
  attribute in the policy file; non-configurable bandwidth limits **[doc]**.
- TLS is terminated by `tailscaled` on the PC; "Funnel relay servers do not decrypt the traffic"
  **[doc]**. The service still only sees the Unix socket (no network socket, D-G holds).

### 3.3 iOS Safari and home-screen web app [doc][report]

- iOS/iPadOS 16+ platform authenticator supports passkeys (iCloud Keychain), user verification uses
  the screen-unlock method (Face ID / Touch ID, passcode fallback) **[doc]** (passkeys.dev iOS page;
  WebKit "Meet Face ID and Touch ID for the web").
- WebAuthn with the platform authenticator **requires a user gesture** (`click`, `touchend`,
  `keydown` …); the gesture propagates through `fetch` but "will expire after 10 seconds" **[doc]**
  (WebKit blog). **[rec]** Call `create`/`get` from the button's click handler, fetching the options
  first inside that handler (one local round trip, well under 10 s), or prefetch the options when the
  page shows `Locked` and call `get` directly in the handler.
- Home-screen web apps run in WebKit with Web Authentication supported (listed since iOS 14.5) but
  with **storage not shared with Safari** **[report]** (firt.dev iOS PWA notes): the session cookie
  set in Safari is not seen by the home-screen app and vice versa; passkeys themselves live in iCloud
  Keychain and are shared. An open Apple Developer Forums report (iOS 18.6.2, February 2026, no reply)
  describes double prompts / freezes when using passkey **autofill (conditional mediation)** from a
  home-screen shortcut **[report]**. **[rec]** Use modal `get` triggered by a button, never
  `mediation: "conditional"`.
- **CSP**: no CSP directive governs `navigator.credentials`; `connect-src 'self'` covers the
  `fetch` calls; `frame-ancestors 'none'` is compatible (WebAuthn is disabled in cross-origin iframes
  anyway, L3 §5.10). The relevant browser control is Permissions Policy features
  `publickey-credentials-create` / `-get`, whose default allowlist is `'self'` (L3 §5.9) **[spec]**;
  the service sends no `Permissions-Policy` today, so nothing blocks it. If one is ever added it must
  keep `publickey-credentials-get=(self)` and `publickey-credentials-create=(self)`. The page JS must
  stay in a served `.js` asset (no inline script), and base64url <-> `ArrayBuffer` conversion should be
  hand-written (do not rely on `PublicKeyCredential.parse*OptionsFromJSON`, whose Safari availability
  was not established here).
- None of this is testable in CI; a physical iPhone pass (Safari tab and home-screen app, VPN on and
  off) belongs in the verification matrix as a manual item.

### 3.4 Distinguishing Funnel from tailnet traffic [src]

`ipn/ipnlocal/serve.go`, `addTailscaleIdentityHeaders` (identical in tag `v1.102.4`, the version
installed here, and on `main`):

```go
// Clear any incoming values squatting in the headers.
r.Out.Header.Del("Tailscale-User-Login")
...
r.Out.Header.Del("Tailscale-Funnel-Request")
...
if c.Funnel != nil {
    r.Out.Header.Set("Tailscale-Funnel-Request", "?1")
    return
}
```

So for Funnel traffic `tailscaled` deletes any client-supplied identity headers and sets
`Tailscale-Funnel-Request: ?1`; for tailnet user traffic it sets `Tailscale-User-Login`; for tagged
nodes or local direct clients it sets neither. **[rec]** Classification:

| `Tailscale-Funnel-Request` | `Tailscale-User-Login` | Path |
|---|---|---|
| exactly one, `?1` | absent | **Funnel**: static login page/assets, WebAuthn login, then cookie-session routes |
| absent | exactly one, allowed | **Tailnet**: today's behaviour (one-tap status/lock), plus enrollment and passkey unlock |
| anything else (both, neither, repeated, other value) | | `403` as today |

This keeps every existing "no identity → 403" test valid (they send neither header), so the only
assertions superseded by the new ADR are the identity-only unlock successes (D-C); the ADR must name
them (e.g. `test_rmc_unlock_flow_and_rate_limit` and the other `server_tests.rs` unlock successes).
The Docs/REMOTE_COMPANION.md trust model ("never run `tailscale funnel`") and ADR 2026-10-05 item
also need an explicit supersession.

---

## 4. Crate choice (pure Rust, no OpenSSL, `cargo deny` clean)

### 4.1 Candidates

| Option | Verdict | Evidence |
|---|---|---|
| **`webauthn-rs 0.5.5`** (+ `webauthn-rs-core`) | **Rejected** | `webauthn-rs-core 0.5.5` depends unconditionally on `openssl ^0.10.75` and `openssl-sys`, plus `base64 ^0.21`, `thiserror ^1`, `x509-parser`, `der-parser`, `nom`, `serde_cbor_2`; licence **MPL-2.0**, which is not in `deny.toml` `[licenses].allow` (crates.io API, 2026-10-06). Violates D-G. |
| **`passkey-types 0.6` / `passkey-crypto 0.1`** (1Password) | Rejected | Authenticator/client-side types, not an RP verifier; `passkey-crypto` pulls `p256 ^0.14`, `sha2 ^0.11`, `rand ^0.10`, `signature ^3`, optional `aws-lc-rs` → many duplicates. |
| **`coset 0.4.2`** (COSE structs) | Not needed | Apache-2.0, built on `ciborium`; parsing one fixed five-key EC2 map by hand is shorter than mapping coset types. |
| **`p256 0.14.0` + `ecdsa 0.17`** | Rejected for now | Requires `elliptic-curve 0.14` → `sha2 0.11`, `digest 0.11`, `rand_core 0.10`, `crypto-common 0.2`, `hybrid-array`; the workspace is on `sha2 0.10.9` / `digest 0.10.7` (`soos-daemon`, `soos-inference-ort`), so several new duplicate versions under `multiple-versions = "deny"`. Revisit when the workspace moves to `sha2 0.11`. |
| **`p256 0.13.2` + `ecdsa 0.16.9`** | **Recommended** | `Apache-2.0 OR MIT`; uses `sha2 ^0.10`, `digest 0.10`, `rand_core 0.6.4`, `subtle`, `zeroize`, `generic-array 0.14` already in the lockfile. |
| `ring` / `aws-lc-rs` | Rejected | C/assembly, not pure Rust (D-G preference); `ring` is not in the lockfile today. |

### 4.2 Verified dependency impact of the recommendation [verified]

Simulated in a scratch copy of the workspace by adding to `crates/remote/Cargo.toml`:

```toml
p256 = { version = "0.13.2", default-features = false, features = ["ecdsa"] }
sha2 = { workspace = true }
ciborium = { workspace = true }
base64ct = { version = "1.8", features = ["alloc"] }
subtle = "2.6"
getrandom = { workspace = true }
```

- New lockfile packages: `p256 0.13.2`, `ecdsa 0.16.9`, `elliptic-curve 0.13.8`, `primeorder 0.13.6`,
  `crypto-bigint 0.5.5`, `ff 0.13.1`, `group 0.13.0`, `sec1 0.7.3`, `base16ct 0.2.0`,
  `der 0.7.10`, `const-oid 0.9.6`, `rfc6979 0.4.0`, `hmac 0.12.1`, `signature 2.2.0`. All are RustCrypto
  `Apache-2.0 OR MIT` (or equivalent).
- `cargo tree -d` gains exactly one duplicate: **`der 0.7.10`** (via `ecdsa 0.16` and `sec1 0.7`)
  next to **`der 0.8.2`** (via `ureq 3.4.2`, the build dependency of `ort-sys`). `der 0.7` is pulled
  regardless of features (disabling `std`/`pem`/`pkcs8` does not remove it).
- `cargo deny check bans` then fails on `der` until a skip is added; with
  `{ crate = "der@0.7.10", reason = "Used by ecdsa 0.16 and sec1 0.7 (p256 0.13 via soos-remote) while ureq 3 (build dependency of ort-sys) uses der 0.8" }`
  the result is `bans ok, licenses ok`, and `cargo deny check advisories` is `advisories ok`
  (cargo-deny 0.20.2). The reason text must satisfy
  `tests/invariants/src/dependency_tooling_contract.rs` (direct dependents named from
  `cargo tree -i der@0.7.10 --depth 1`).
- `default-features = false` matters: the default `pem`/`pkcs8` features add `spki`/`pkcs8` for
  nothing. The `ecdsa` feature also enables signing (`rfc6979`, `hmac`), which is harmless and useful:
  the tester can build signed fixtures with `p256::ecdsa::SigningKey` (dev side) without another crate.
- `base64ct` (RustCrypto, constant-time, already locked at 1.8.3) is preferred over `base64 0.23`
  (only locked as a `ureq` build dependency today; its default `simd-unsafe` feature). Either avoids a
  duplicate. `getrandom 0.4` (already a workspace dependency) provides the CSPRNG; `rand` is not needed.
- Caveat: the `p256` README states its arithmetic "has never been independently audited". The service
  only *verifies* signatures over public data (no secret scalar in the server), which limits the
  side-channel concern; record this in the ADR as an accepted risk.
- MSRV: `p256 0.13.2` / `ecdsa 0.16.9` declare Rust 1.65; the toolchain here is 1.98.1.

### 4.3 End-to-end check on the official vector [verified]

A scratch program using only `p256 0.13.2`, `ciborium 0.2.2`, `sha2 0.10.9` decoded L3 §16.2:
`fmt = "none"`, `attStmt = {}`, 164-byte `authData`, `rpIdHash == SHA-256("example.org")`, reg flags
`0x59`, COSE `kty 2 / alg -7 / crv 1`, and verified the assertion DER signature over
`authenticatorData || SHA-256(clientDataJSON)` (`true`). The vector's assertion flags are `0x19`
(UV clear), so it is a ready-made **negative** test for "UV required"; positive UV fixtures must be
generated with `SigningKey` in the tests. The other L3 §16 vectors (§16.4 `crossOrigin: true`,
§16.5 `topOrigin`, §16.6 very long credential id, §16.3 packed self attestation) give further
rejection fixtures for free.

---

## 5. Session cookie and CSRF (Funnel path, D-D)

### 5.1 Cookie [doc][rec]

```
Set-Cookie: __Host-soos_session=<token>; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=<absolute TTL s>
```

- `__Host-` prefix: must be `Secure`, set from HTTPS, no `Domain`, `Path=/` (MDN Set-Cookie). This
  blocks cookie tossing from sibling `*.<tailnet>.ts.net` nodes (a sibling can set
  `Domain=<tailnet>.ts.net` cookies but never a `__Host-` one). Behind Funnel/Serve the browser sees
  HTTPS, so the prefix is accepted even though the backend speaks HTTP on a Unix socket.
- `HttpOnly`: page JS never needs the token; `fetch` and `EventSource` (same-origin) still send it.
- `SameSite=Strict`: other tailnets' Funnel sites are cross-site (PSL), so the browser never attaches
  the cookie to their requests; top-level navigations from elsewhere do not carry it either (the page
  then simply shows the login screen).
- Token: 32 bytes from `getrandom` (256 bits; OWASP minimum is 64 bits of entropy, 128-bit tokens),
  base64url without padding (43 chars). Store only `SHA-256(token)` in memory (OWASP "server stores
  … the full SHA-256 hash of the verifier"); look up by hash, compare with `subtle`. A process restart
  logs every session out (acceptable, documented).
- TTL: OWASP suggests "2-5 minutes" idle for high-value applications and 4–8 h absolute. **[rec]**
  idle 15 min, absolute 8 h, `MAX_REMOTE_SESSIONS = 4`; when full, refuse new logins (`429`) rather
  than evict the owner's session; `Max-Age` equals the absolute TTL and the server enforces both.
- Issue a **new** token on every successful login (no fixation: there is no pre-login session).
  `POST /api/logout` deletes the server record and sends `Max-Age=0`.
- The cookie only authorises status, events and lock on the Funnel path. **Unlock always requires a
  fresh `unlock`-purpose assertion in the request (D-C)**, cookie or not. On the tailnet path the
  Tailscale identity keeps one-tap status and lock; the cookie is ignored there (or not consulted), so
  the two authorisation models do not mix.
- Never log the token, its hash, the session count per client or any id (D-H). The cookie value must
  be excluded from any error echo.

### 5.2 CSRF interplay [rec]

- `SameSite=Strict` is not sufficient alone because sibling nodes of the same tailnet are same-site.
  Keep the existing rules on **every** state-changing route, now including the WebAuthn routes:
  exactly one `X-Soos-Action` with a route-specific value (e.g. `login-options`, `login`,
  `register-options`, `register`, `unlock-options`, `unlock`, `logout`, plus `lock`); `Sec-Fetch-Site`
  absent or `same-origin`; `Origin` absent or exactly `https://<rp_id>` (WebAuthn routes should
  **require** `Origin`, since Safari always sends it on `fetch` POST).
- `X-Soos-Action` is a non-simple header, so a cross-origin page cannot send it without a CORS
  preflight that the service never answers; this stays the primary CSRF defence, with the WebAuthn
  `clientDataJSON.origin` check as a second, cryptographically bound origin check for login, register
  and unlock.
- `GET /api/status` and `GET /api/events` stay side-effect free; with `SameSite=Strict` and no CORS
  headers, a cross-origin page can neither send the cookie nor read the response.

---

## 6. Enrollment (D-E) — notes for the architect [rec]

- Enrollment routes are reachable only on the **tailnet path** (allowed `Tailscale-User-Login`, no
  Funnel marker); on the Funnel path they answer `403` before reading the body.
- Local one-time code: a CLI subcommand (e.g. `soos-remote enroll-code`) generates e.g. 10 characters
  from a 32-symbol alphabet (50 bits), prints it once, and hands only `SHA-256(code)` + expiry to the
  running service through a `0600` file in the existing `0700` runtime directory (atomic rename) or an
  owner-only local request. TTL 5 min, single use, invalidated after 3 wrong attempts; compared with
  `subtle`. Never logged.
- The code is checked when issuing registration options **and** consumed when the attestation is
  verified, both tied to the same registration challenge.
- Credential store: `0600` JSON file under the service's state directory (not created by agents),
  bounded record count, written atomically; a `soos-remote passkeys list|remove` subcommand for
  revocation is the natural counterpart.

---

## 7. Open questions for the owner / ADR

1. Challenge TTL and `timeout` of 120 s (deviation from L3's 300–600 s recommendation).
2. Session idle/absolute TTL (15 min / 8 h proposed) and maximum session count.
3. Fail-closed on a non-zero non-increasing `signCount` (proposed) vs log-and-accept.
4. Strict `fmt == "none"` only (proposed) vs also accepting `packed` with `attStmt` ignored.
5. Accept the `der 0.7.10` duplicate (one `deny.toml` skip) for `p256 0.13.2`.
6. Wording of the supersession of ADR 2026-10-05 / 2026-10-06 (Funnel was forbidden, unlock was
   identity-only) and of the superseded unlock test assertions.

---

## Sources

- W3C, *Web Authentication: An API for accessing Public Key Credentials, Level 3*, Recommendation
  2026-08-25, <https://www.w3.org/TR/2026/REC-webauthn-3-20260825/> (§1 RP ID, §5.4.7, §5.8.1,
  §5.8.5, §5.9, §5.10, §6.1, §6.1.1, §6.1.3, §6.5.1, §6.5.5, §7.1, §7.2, §8.7, §13.4.3, §13.4.9,
  §15.1, §16.2–§16.6).
- W3C, *Web Authentication Level 2*, Recommendation 2021-04-08,
  <https://www.w3.org/TR/2021/REC-webauthn-2-20210408/>.
- Public Suffix List, <https://publicsuffix.org/list/public_suffix_list.dat> (entries `ts.net`,
  `*.c.ts.net`).
- Tailscale source, `ipn/ipnlocal/serve.go` at tag `v1.102.4` and `main`,
  <https://github.com/tailscale/tailscale/blob/v1.102.4/ipn/ipnlocal/serve.go>.
- Tailscale docs, *Tailscale Funnel*, <https://tailscale.com/kb/1223/funnel>.
- WebKit blog, *Meet Face ID and Touch ID for the Web*,
  <https://webkit.org/blog/11312/meet-face-id-and-touch-id-for-the-web/>.
- passkeys.dev, *iOS & iPadOS*, <https://passkeys.dev/docs/reference/ios/>.
- firt.dev, *iOS PWA Compatibility*, <https://firt.dev/notes/pwa-ios/>.
- Apple Developer Forums thread 815784, *Passkey authentication issues on iPhone when launching login
  pages via Home Screen shortcuts*, <https://developer.apple.com/forums/thread/815784>.
- MojoAuth, *signCount Is Dead* and *BE and BS Flags Explained* (signCount 0 for synced passkeys),
  <https://mojoauth.com/blog/signcount-is-dead-why-passkey-clone-detection-doesnt-work-anymore>,
  <https://mojoauth.com/blog/webauthn-be-bs-flags-synced-vs-device-bound-passkey>.
- MDN, *Set-Cookie*, <https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/Set-Cookie>.
- OWASP, *Session Management Cheat Sheet*,
  <https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html>.
- crates.io API (versions, licences, dependency requirements, 2026-10-06): `p256`, `ecdsa`,
  `elliptic-curve`, `webauthn-rs`, `webauthn-rs-core`, `passkey-types`, `passkey-crypto`, `coset`,
  `ciborium`, `base64`; RustCrypto `p256 0.13.2` README (audit warning).
- Repository: `deny.toml`, `Cargo.lock`, `crates/remote/src/{http,routes,identity,lib}.rs`,
  `crates/remote/tests/server_tests.rs`, `tests/invariants/src/{remote_companion_contract,dependency_tooling_contract}.rs`,
  `Docs/REMOTE_COMPANION.md`, `AI/DECISIONS.md`.
