# Walkthrough 189 — Web Push Notifications for Failed-Password Alerts

- **Date**: 2026-10-06
- **Issue**: GitHub-only follow-up of #339 (feature level 2 of the failed-password alerts).
  **Branch**: `feat/remote-auth-alerts`, from `feat/remote-companion` at `ea862cc` (draft PR
  #340 belongs to `feat/remote-companion`). Nothing is committed, pushed, deployed, installed or
  restarted by the agents, and no real push request is sent (owner decision O-5).
- **ADR**: "[2026-10-06] Web Push Notifications for Failed-Password Alerts Through a Separate
  Sender Unit" (`AI/DECISIONS.md`).
- **Spec**: `AI/architect_spec_remote_web_push.md` (round 2); research `AI/research_push.md`;
  tester contract `AI/tester_contract_push.md`; auditor constraints
  `AI/auditor_constraints_push.md` (C-1 to C-43, C-50).
- **Matrix criteria**: RMC60–RMC74 (RMC74 is the owner's hardware check).

## 1. Context & Objectives

Level 1 (walkthrough 188) shows failed password attempts on the PC inside the open
`soos-remote` page. The owner wants to be told **even when the app is closed** (O-1): a standard
Web Push notification to the home-screen web app on the iPhone (iOS 16.4 or later).

The relayed request also asked to "see on the phone the passwords that were tried". That is not
done and cannot be done: no journal producer contains the typed text, `AGENTS.md` forbids
capturing, storing or transmitting passwords, owner decision O-2 forbids any part, length or
hash of it, and a notification is readable on the iPhone lock screen and transits a third-party
push service. A notification carries only **source class, account class, kind and counts**
(or a fixed generic text with `push_previews = "generic"`). **Owner confirmation of this safe
subset: pending** — no workflow script output, relayed task text or agent message counts as
that confirmation; the hand-off asks the owner for it in the owner's own words.

## 2. Architect Design

- **Process split (W-3)**: `soos-remote` keeps `RestrictAddressFamilies=AF_UNIX` and its unit
  unchanged; it holds the VAPID key, the subscriptions and all cryptography. A new key-less
  user service `soos-push-sender` (crate `crates/push-sender`) has network access, listens on
  `$XDG_RUNTIME_DIR/soos-push/push.sock` (`0600`) and performs exactly one outbound HTTPS POST
  per framed request. A pure crate `soos-push-protocol` (`crates/push-protocol`) is the single
  source of the frame codec, the endpoint allowlist, the public-address predicate and the status
  classification.
- **SSRF (W-5)**: exact host allowlist (`web.push.apple.com`, `fcm.googleapis.com`,
  `updates.push.services.mozilla.com`) at subscribe time and in the sender; a filtering ureq
  resolver refuses the whole request when any resolved address is not public; no redirect, no
  proxy, https only, 10 s bound.
- **Crypto (W-7)**: RFC 8291 `aes128gcm` (ECDH P-256, HKDF as HMAC-SHA-256 steps, AES-128-GCM),
  ES256 VAPID JWT (12 h, reused ≤ 1 h per origin), randomness only from the `getrandom` seam.
- **Delivery (W-13)**: live attempts only (never the 24 h replay), coalesced 3 s, ≥ 30 s apart,
  ≤ 20 per hour; two retries; 404/410 remove the device; undelivered counts carried forward.
- **Routes (W-9)**: `GET /api/push`, `POST /api/push/{subscribe,unsubscribe,test}`, all
  authenticated and CSRF-checked before any body byte; `/sw.js` is a public asset.

## 3. Tester Contract (Phase 2)

New suites: `crates/push-protocol/tests/protocol_tests.rs`,
`crates/push-sender/tests/sender_tests.rs`, `crates/remote/tests/{webpush,push,push_server}_tests.rs`,
`crates/remote/tests/common/push.rs`, the `push_contract` modules of `config_tests.rs` and
`routes_tests.rs`, and `tests/invariants/src/remote_push_contract.rs` (RMC-S32–RMC-S42). The
only changes to existing test files are the three setup lines `push: PushConfig::default()`
(W-15) and the two business-crate names appended to the forbid-lint invariant.

## 4. Auditor Constraints (Phase 3)

Cleared with constraints C-1 to C-43 and C-50: no typed text anywhere, no `tracing` macro in
`push.rs`/`webpush.rs`, redacted `Debug` for keys and endpoints, a bridgeless fixed logging
filter in the sender, exact sandbox unit, bounded frames before allocation, zeroized secret
buffers (the push store is read into a buffer preallocated to its bound), non-blocking `flock`
with async retries, exact gate order of the routes.

## 5. Developer Implementation (Phase 4)

- `crates/push-protocol/src/lib.rs`: constants, `PushEndpoint` (byte-level parser, redacted
  `Debug`), `is_public_address` (integer masks), frames (`deny_unknown_fields`, every key
  mandatory, encoder and decoder validate alike, `Zeroizing` buffers), `classify_status`,
  `parse_retry_after`.
- `crates/push-sender/src/{lib,main}.rs`: `filter_addresses`, `FilteringResolver` (the only
  resolver, `Agent::with_parts`), `ResolveRefused` → `refused`, `UreqDeliverer` (status
  classified before a bounded, discarded body read), `serve_connection` (peer uid via
  `SO_PEERCRED`, one cumulative deadline per frame), `bind_socket`, `check_not_root`,
  `install_logging` (registry + fmt layer + fixed `Targets`, `set_global_default`).
- `crates/remote/src/webpush.rs`: `VapidKey`, `UaKeys`, `encrypt`/`encrypt_with`/`derive_keys`,
  `vapid_authorization`, `JwtCache`.
- `crates/remote/src/push.rs`: `PushStore` (only service start and `push reset` create the
  file), `PushScheduler`, `PushSummary::merge`, payloads, `PushTransport` +
  `UnixPushTransport` (whole exchange under 18 s), `PushRuntime` (created before the alerts
  runtime so the live sink can be handed over, opened after it), the dispatcher (test, then due
  retries, then a due summary; carries per subscription), the route helpers.
- `alerts.rs` (`is_live`, `AlertBook::started_us`, the live sink called under the alerts mutex),
  `config.rs` (`PushConfig`, four keys, five errors, `resolve_push_store_path`), `routes.rs`
  (four routes, `/sw.js`, `check_push_csrf`, two new body routes), `server.rs` (`with_push`,
  set-up, supervised dispatcher, handlers), `audit.rs` (five fixed events), `credentials.rs`
  (`try_lock_file` and a capacity-aware bounded read shared with the push store), `main.rs`
  (wiring, `push list|remove N|reset`).
- Page: `assets/sw.js` (always shows a notification; tag per kind), the "Notifications" card in
  `index.html`/`app.js`/`style.css` (permission from the tap, key comparison on load, devices
  list, test and disable buttons).
- `packaging/soos-push-sender.service` (exact sandbox of spec §8.2 minus `ProtectControlGroups=`),
  `scripts/install_remote.sh` (builds and installs both, enables nothing),
  `scripts/candid_review.sh` (`BUSINESS_CRATES`), `deny.toml` (skip reasons name `ring`),
  `Cargo.toml` (`hmac`, `ureq` exact pin), `AGENTS.md`/`AI/ARCHITECTURE.md` workspace trees.
- **Lock file**: one online resolution added only `ring 0.17.14`, `rustls 0.23.45`,
  `rustls-webpki 0.103.15`, `untrusted 0.9.0`, `webpki-roots 1.0.9` and the two path packages;
  no existing locked version changed.

## 6. Verification

- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked --all-features
  -- -D warnings`, `cargo deny --locked check`: clean.
- `soos-push-protocol` (8 tests) and `soos-push-sender` (7 tests, real-time trickle bounds) green;
  `soos-remote` push suites (`webpush_tests`, `push_tests`, `push_server_tests`,
  `routes_tests::push_contract`) green and repeated in loops to look for flakes.
- **Two contract tests were defective and were migrated after the developer phase** (recorded as
  Contract Migrations at the end of `AI/tester_contract_push.md`; the whole workspace suite is now
  green):
  1. `config_tests::push_contract::test_rwp_push_config_keys` built its at-limit path with
     `MAX_SOCKET_PATH_LEN - 4` (106 bytes, `1 + n + 2`) while asserting 107; the repeat counts are
     now `- 3` (exactly 107, accepted) and `- 2` (108, refused). Assertions and
     `MAX_SOCKET_PATH_LEN` are unchanged.
  2. `remote_passkey_contract::test_rmc_s18_page_uses_modal_webauthn_without_storage` forbade the
     word `serviceWorker` in `app.js`, which the Web Push ADR supersedes; the token was replaced by
     a stricter rule: exactly one `.register(` call in `app.js`, the same-origin `/sw.js` with
     scope `/`.
- Not verifiable by agents: RMC74 (owner hardware: home-screen subscription, test notification,
  lock-screen and `sudo` notifications, the sender sandbox with MagicDNS and HTTPS).

## 7. Residual Risks

See the ADR and `Docs/REMOTE_COMPANION.md` §8: lock-screen readability of detailed notifications;
delivery metadata at Apple/Google/Mozilla; false notifications from an owner-uid process (≤ 20 per
hour); a compromised sender keeps network access, reaches abstract-namespace sockets (for example
Xwayland's) and can forge replies; a client of the owner's uid that delays `soos-remote` past
18 s can cause a duplicate notification; a newer summary replaces a pending `Retry-After` retry
and is sent at once (accepted against Apple's rate limit); ureq's resolver thread may outlive a
lookup timeout (bounded by glibc's resolver timeouts and the sequential sender); a stolen Funnel
session can register a device (device list, `push list`/`push remove N`, `passkeys remove N`);
`ring`, `rustls` and the RustCrypto crates are not independently audited.
