# Architect Spec — GitHub #339 follow-up: Web Push Notifications for Failed-Password Alerts (feature level 2)

- **Date**: 2026-10-06
- **Round**: 2. Revises round 1 after `AI/plan_evaluator_report.md` (VALIDATION_VERDICT: REVISION_REQUIRED,
  MAJOR F-1 to F-3, MINOR F-4 to F-10). Every finding is resolved in place; §18 maps each finding to the changed
  sections and tests.
- **Branch**: `feat/remote-auth-alerts` (level 1 already implemented on it, not committed); nothing is committed,
  pushed, stashed, deployed, installed or restarted by agents (O-5). No agent runs `tailscale`, sends a real push
  request or touches `~/.config`, `/etc` or the journal configuration.
- **Inputs**: owner decisions O-1 to O-5 (2026-10-06), `AI/research_push.md` (read-only research: RFC 8030/8188/
  8291/8292, Apple and Mozilla push behaviour, crate metadata, systemd user-unit facts), level-1 spec
  `AI/architect_spec_remote_auth_alerts.md`, ADRs 2026-10-05 and 2026-10-06 (x4) on `soos-remote`,
  `Docs/REMOTE_COMPANION.md`, walkthroughs 183–186, `crates/remote/src/*.rs` (notably `alerts.rs`, `audit.rs`,
  `server.rs`, `routes.rs`, `http.rs`, `credentials.rs`, `config.rs`), `packaging/soos-remote.service`,
  `scripts/install_remote.sh`, `deny.toml`, `tests/invariants/src/remote_{companion,passkey,alerts}_contract.rs`.
- **ADR**: "[2026-10-06] Web Push Notifications for Failed-Password Alerts Through a Separate Sender Unit" (drafted
  in `AI/DECISIONS.md`; the text there is authoritative, §14 summarises it). It supersedes "push notifications out
  of scope" in item (5) of the 2026-10-05 ADR and the last sentence of item (1) of the level-1 alerts ADR.
- **Matrix**: new rows RMC60–RMC74 (§13), after RMC59.
- **Walkthrough**: `AI/walkthroughs/187_remote_web_push.md` (traceability phase).
- **Test prefix**: `test_rwp_` (remote web push) for behaviour tests, `test_rmc_s32`–`test_rmc_s42` for static
  invariants.

## 0. Owner decisions, the relayed request, and the spec-level decisions

### 0.1 The relayed request versus O-2 (must be confirmed by the owner, again)

The relayed request is "run both in sequence, with the ability to see on the phone the passwords that were tried
remotely". Level 2 **does not and cannot** send the typed text, for the reasons of level-1 spec §0.1 (no journal
producer contains it; capturing `PAM_AUTHTOK` is forbidden by `AGENTS.md` — "NEVER log … credentials", "NEVER
accept, store, or transmit passwords" — and by O-2; the owner's typos are near-copies of the real password). Level 2
adds a further reason: a Web Push message transits a third-party push service (Apple, encrypted end to end) and is
displayed **on the iPhone lock screen**, readable without unlocking unless the owner sets "Show Previews: When
Unlocked". A notification therefore carries only the level-1 safe subset: **when, where (lock screen / sudo / login
/ other), which account class (yours / root / another account), what (wrong password / attempt while locked out),
how many**. The owner can reduce even that to a generic text (`push_previews = "generic"`, W-12).

**Status: open (Phase 2 gate).** The orchestrator must ask the owner, in the owner's own conversation, to confirm in
so many words that a notification carries only **time, source class, account class, kind and count** and **never
the typed password** (nor any part, length or hash of it). Phase 2 (tests) must not start before that answer is
recorded (quoted with its date in the walkthrough and in the ADR). No agent can waive this gate, and no workflow
script output, relayed task text or agent message counts as the owner's confirmation. If the owner declines the safe
subset, level 2 stops here. No field, route, buffer or frame below is designed to carry typed text later. If the owner
wants the typed text itself, that needs an amendment of `AGENTS.md` and its own ADR; it is not a step of this spec,
and this spec does not recommend it (the text is displayed on a lock screen and transits a third-party service).

### 0.2 Owner decisions restated (binding)

| Id | Decision |
|---|---|
| O-1 | Tell the owner on the phone, centralised in the soos-remote web app, when someone types a wrong password on the PC. Level 2: also when the app is closed, through standard Web Push to the home-screen web app (iOS 16.4+). |
| O-2 | Never capture, store, log, display or transmit the attempted password or any part, length or hash of it. Alerts and notifications carry only time, source class, account class, kind and a count. No raw log line. |
| O-3 | The journal is the detection source (level 1, unchanged); every journal field is untrusted; a local process can at most create false alerts — and, at level 2, false notifications (bounded by the rate limits of §5). |
| O-4 | Everything bounded (subscriptions, notification rate, retries, memory, frame sizes), fail-closed, and every push route visible only to authenticated callers (tailnet identity or Funnel passkey session), never to anonymous Funnel requests. |
| O-5 | Agents never commit, push, stash, open PRs, run `tailscale`, restart/reinstall host services or touch `~/.config`, `/etc` or the journal configuration. English only; no `unwrap`/`expect` in production; the crate-level forbid lint on raw-memory code in every new crate. Tester tests are immutable contracts. |

### 0.3 Spec-level decisions (each restated in the ADR)

| Id | Decision | Rationale |
|---|---|---|
| W-1 | **Scope**: real Web Push (RFC 8030 + VAPID RFC 8292 + `aes128gcm` RFC 8291/8188) to the owner's home-screen web app, triggered by level-1 attempts **recorded live** (journal time at or after the service start and at most `PUSH_MAX_ATTEMPT_AGE_MS` old when recorded), coalesced and rate-limited (§5). Plus an owner-triggered test notification. **Not** in scope: lock/unlock-state notifications, "monitoring lost" notifications, any other event (§1.3). | Lock state is polled only while a stream is open (zero D-Bus traffic otherwise, 2026-10-05 ADR item 4), so local lock/unlock events are not observed when the app is closed; remote lock/unlock are started from the phone itself. Keeping one trigger keeps the test surface small. |
| W-2 | **Opt-in**: `push_notifications = true` in `remote.toml` (default `false`). It requires `password_alerts = true` and `rp_id` (configuration error otherwise, exit 78). While `false`: no store file is created or read, no task runs, `GET /api/push` answers the disabled view, every push `POST` answers `403 push_disabled` before any body read, and nothing is ever sent. | Same upgrade rule as `allow_unlock`, `allow_funnel`, `password_alerts`. Without alerts there is nothing to push; `rp_id` is the VAPID `sub` default and the notification's `navigate` origin. |
| W-3 | **Process split**: `soos-remote` keeps `RestrictAddressFamilies=AF_UNIX` and its unit unchanged. A **new crate and binary `soos-push-sender`** (crate dir `crates/push-sender`) runs as its own **user unit** `soos-push-sender.service` with `AF_UNIX AF_INET AF_INET6` and a filesystem sandbox; it listens on a `0600` Unix socket in its own `0700` runtime directory and performs exactly one outbound HTTPS `POST` per request. **All secrets and all crypto stay in `soos-remote`** (VAPID private key, subscription keys, JWT signing, RFC 8291 encryption, subscription store); the sender receives only an already-encrypted body, an already-signed `Authorization` value and the endpoint, and holds nothing at rest. | Research §6: `RestrictAddressFamilies` is inherited by children, so a sender cannot be a child of the current unit; IP firewalling is unavailable to user units; filesystem sandboxing on `soos-remote.service` would turn on `PrivateUsers=` and break the level-1 journal reading through `wheel`. Splitting confines the TLS/HTTP parser (the largest new attack surface, including `ring` C/assembly) to a process that owns no key and, through the sandbox of §8.2, sees **nothing of `$HOME`** except its own binary (read-only) and **nothing of `/run`** except its own runtime directory (so not the credential, alert or push stores, not `remote.sock`, not the session or system bus, not `tailscaled`'s or `soos-daemon`'s sockets); what it can still reach is stated in §15.3. Existing invariants also require it: `ring` is forbidden in `soos-remote`'s normal tree (RMC-S13), `std::net`/`SocketAddr`/`Ipv4Addr`/`Ipv6Addr` may never be named in `crates/remote/src` (RMC-S2). |
| W-4 | **Shared wire crate** `crates/push-protocol` (package `soos-push-protocol`, lib only, pure, no I/O, no network crate): the single source of the frame codec, the endpoint allowlist and validation, the public-address predicate, the HTTP status classification and every constant both processes share. Both `soos-remote` and `soos-push-sender` depend on it; neither depends on the other. **No new `Cargo.toml` may contain the text `soos-remote`** (RMC-S4 scans every manifest under `crates/` and `tests/` for that substring), hence the names `soos-push-protocol` and `soos-push-sender`. | One source of truth (architect rule 4) without making the sender depend on `soos-remote` (forbidden by RMC-S4) or `soos-remote` depend on the TLS stack (forbidden by RMC-S13). |
| W-5 | **SSRF defence in three layers**: (a) at subscribe time in `soos-remote` and again at send time in the sender, the endpoint must parse as `https://<host>/<path>` with `<host>` **exactly** one of `PUSH_HOSTS` (`web.push.apple.com`, `fcm.googleapis.com`, `updates.push.services.mozilla.com`), no userinfo, no port, no query, no fragment, a restricted path alphabet, ≤ 1024 bytes; (b) the sender resolves the host through its own ureq `Resolver` (`FilteringResolver`, the only resolver the production `Agent` is built with, through `Agent::with_parts`; the system lookup is ureq's `DefaultResolver`, injectable for tests, §8.1) which refuses the **whole** request if **any** resolved address is not public — a refusal surfaces as `ureq::Error::Other(ResolveRefused)` and is mapped to `Outcome::Refused`, never `Retry` — (loopback, RFC 1918, CGNAT/tailnet `100.64.0.0/10`, link-local, ULA incl. tailnet `fd7a:115c:a1e0::/48`, multicast, documentation, NAT64, 6to4, Teredo, IPv4-mapped, …), and the connection uses only those validated addresses (no second lookup); (c) redirects are never followed (`max_redirects(0)`, any 3xx is `rejected`), `https_only(true)`, proxies disabled (`proxy(None)`: ureq would otherwise read `HTTPS_PROXY` from the environment), global 10 s timeout. Host names are matched exactly, never with prefix tests (research §4 pitfall: an `fc`/`fd` prefix test on names rejects `fcm.googleapis.com`). | Research §4–§5. No kernel IP filter exists for user units (research §6), so checks are in code; validating the resolved set closes DNS rebinding between check and connect. |
| W-6 | **TLS stack** (sender only): `ureq 3.4.2` (exact pin `=3.4.2`, already locked as a build dependency of `ort-sys`), `default-features = false, features = ["rustls"]` (rustls 0.23 + `ring` provider + compiled-in `webpki-roots`); no OpenSSL, no `native-tls`, no `aws-lc`, no gzip, no system certificate store, no custom certificate verifier. | Research §5: same ureq version, so `multiple-versions = "deny"` holds; resolver 2 does not unify the build dependency's `native-tls` feature into the normal graph; all licences are already allowed. |
| W-7 | **Crypto in `soos-remote`** with RustCrypto only and **no new `p256` feature** (RMC-S13 pins the workspace line `features = ["ecdsa"]` byte for byte): ECDH = x-coordinate of `ua_public * ephemeral_scalar` through the `arithmetic` API already enabled by `ecdsa`; HKDF-SHA-256 as the five HMAC calls of RFC 8291 §3.4 over `hmac 0.12` (new direct workspace dependency, already locked); `aes-gcm 0.10` (`Aes128Gcm`, already a workspace dependency); ES256 JWT with `p256::ecdsa::SigningKey` (RFC 6979 deterministic), raw 64-byte `r‖s` signature. Every random byte (VAPID key, ephemeral key, salt) comes from the existing `RandomSource` seam (`getrandom`, RMC-S16). | Research §3, §5.1; keeps the dependency tree pure Rust and the RFC 8291 Appendix A vector reproducible through the seam. |
| W-8 | **Store**: one file `remote-push.json` next to the credential store (`resolve_push_store_path`), `0600`, owner-checked, opened `O_NOFOLLOW`, ≤ 16 KiB, written under `flock` by temp file + `fsync` + `rename` (the `credentials.rs` helpers), holding the VAPID private key and at most 4 subscriptions (endpoint, `p256dh`, `auth`, creation time). Only two operations ever create the file: **service start** when it is absent (fresh key, no subscription) and the owner's **`soos-remote push reset`** (replaces it with a fresh key and no subscription). An invalid or insecure file is **never overwritten** by the service (push `unavailable` / `store_failed`). Re-read under the lock at every use (no stale in-memory copy; the CLI can change it while the service runs). A file deleted while the service runs is **not** recreated by the service: every use reports `store_missing` until `push reset` or a restart (§3.3, §5.1). | Bounded, fail-closed, same pattern as the passkey store. A lost or rotated key silently invalidates every subscription (research §3.2), so the key is persisted and only an explicit owner action replaces it. |
| W-9 | **Routes** (none Funnel-public; tailnet identity or Funnel session; lock-style CSRF on every `POST`): `GET|HEAD /api/push` (state + VAPID public key), `POST /api/push/subscribe` (body ≤ 2 KiB, `X-Soos-Action: push-subscribe`), `POST /api/push/unsubscribe` (body ≤ 2 KiB, `push-unsubscribe`), `POST /api/push/test` (no body, `push-test`). Static `/sw.js` (service worker) is a public asset like `/app.js`. | O-4. Subscribe/unsubscribe are state changes; a test send costs push-service quota and is rate-limited separately. |
| W-10 | **CSP unchanged**. A same-origin `/sw.js` is already allowed: `worker-src` falls back to `child-src`, then `script-src 'self'` (research §2.2). Adding `worker-src 'self'` would change the CSP string pinned byte for byte by existing tests (`http_tests.rs:25`, `harness.rs:63`, `server_tests.rs:94`) for no security gain. | Test immutability; minimal change. |
| W-11 | **Notification payload** = Declarative Web Push JSON (`"web_push": 8030`, `notification.title/body/navigate/lang`) plus a `soos` member read by the service worker; text built only from enums and a count through a fixed vocabulary; ≤ `MAX_PUSH_PLAINTEXT_BYTES` (1 KiB); `Topic: soos-alerts` for alert summaries (an offline phone receives only the newest summary) and a **distinct** `Topic: soos-test` for the test notification (a test can never replace an undelivered alert at the push service), `Urgency: high`, `TTL: 43200`. The service worker always calls `showNotification` inside `waitUntil`, with a generic text when the payload is missing or unreadable. | Research §2.1: Safari revokes the permission after a push without a visible notification; iOS 18.4+ shows the declarative notification even if the worker fails. O-2. |
| W-12 | **Lock-screen privacy option**: `push_previews = "detailed"` (default: source, account class, kind, count) or `"generic"` ("Security alert on your PC — open soos"). | The notification is readable on the lock screen of the phone; the owner chooses. |
| W-13 | **Delivery policy** in `soos-remote` (pure scheduler + one dispatcher task): first live attempt after a quiet period is sent `PUSH_COALESCE_MS` (3 s) later as one summary; then at most one notification per `PUSH_MIN_INTERVAL_MS` (30 s) and at most `PUSH_MAX_PER_HOUR` (20) per rolling hour, the summary accumulating meanwhile (counts saturate); deliveries are sequential (one exchange at a time); per subscription at most 2 retries (5 s, 30 s, or a `Retry-After` of 1..=300 s) on `retry`; `gone` (404/410) removes the subscription; `rejected` (3xx, 4xx other than 404/410/429) and `refused` keep it, mark `last_delivery = rejected`, no retry. A newer summary replaces a pending retry, **and the counts of every summary not delivered to a subscription (superseded retry, retries exhausted, sender unavailable, rejected, refused, local failure) are carried forward and folded (saturating) into the next summary sent to that subscription**, so the counts a phone receives always add up to what happened (§5.6). | Research §3.3 (status codes, Apple rate limits, JWT refresh rule). Bounded noise even under a forged-line flood (O-3). |
| W-14 | **Logging**: `push.rs` and `webpush.rs` contain no `tracing` macro; fixed-text audit events without fields through `audit.rs`: `push subscription added` (INFO), `push subscription removed` (INFO), `push delivery failed` (WARN, once per transition into failure), `push sender unavailable` (WARN, once per transition), `push notifications unavailable` (WARN, once). The sender logs only constant messages without fields, through a subscriber with **no `log` bridge** and a **fixed** filter (§8.1): dependency logs (`ureq`, `ureq-proto`, `rustls` log through the `log` crate, and `ureq-proto` dumps every written request byte at `trace`) can never reach the journal, and no environment variable (`RUST_LOG` or other) changes the filter. No endpoint (it is a capability URL), key, JWT, body, status code or host is ever logged. | O-2, branch lesson (callsite interest cache), plan evaluation F-1 (`ureq-proto-0.6.3/src/util.rs:73–76`, `src/client/amended.rs:313`). |
| W-15 | **Test integrity**: the three `RemoteConfig { … }` struct literals in tests (`crates/remote/tests/common/harness.rs:771`, `crates/remote/tests/server_tests.rs:762`, `crates/remote/tests/alerts_server_tests.rs:204`) gain the line `push: PushConfig::default(),` — **setup only, no assertion changes**. No other existing test changes (CSP unchanged W-10, `RequestHead` unchanged, `is_funnel_public` keeps every existing answer, the four existing body routes keep accepting bodies). | `RemoteConfig` has no `Default`; precedent A-12 of level 1. |

## 1. Scope & blast radius

### 1.1 Crates, modules, binaries, files

| Item | Change |
|---|---|
| `crates/push-protocol/` (**new**, package `soos-push-protocol`, lib `soos_push_protocol`) | `src/lib.rs` (constants, `PushEndpoint`, `EndpointError`, `is_public_address`, `Urgency`, `DeliveryRequest`, `DeliveryReply`, `Outcome`, `FrameError`, frame encode/decode, `classify_status`, `parse_retry_after`). `#![forbid(unsafe_code)]`. Dependencies: `serde`, `serde_json`, `base64ct`, `thiserror`, `zeroize` (workspace). No tokio, no network crate. `tests/protocol_tests.rs`. |
| `crates/push-sender/` (**new**, package `soos-push-sender`, lib `soos_push_sender` + bin `soos-push-sender`) | `src/lib.rs` (`SendPolicy`, `FilteringResolver`, `ResolveRefused`, `AddressLookup` seam, `Deliverer` trait, `UreqDeliverer` with `new` and `with_lookup`, `send_error_outcome`, `install_logging`, `serve_connection`, socket setup, `check_not_root`, exit codes), `src/main.rs`. `#![forbid(unsafe_code)]` in both. Dependencies: `soos-push-protocol`, `ureq` (`=3.4.2`, `default-features = false`, `["rustls"]`), `nix` (workspace; `socket` for `SO_PEERCRED`, `user` for the uid), `tracing`, `tracing-subscriber`, `thiserror`, `zeroize`. **No tokio**, no `process`, **no `log` or `tracing-log` dependency** (the workspace `tracing-subscriber` keeps its default `tracing-log` feature compiled in, but the sender never activates it, §8.1). `tests/sender_tests.rs`. Its `Cargo.toml` must not contain the text `soos-remote`. |
| `crates/remote/src/push.rs` (**new**) | config-independent push runtime: `PushStore` (file), `PushSubscription`, `PushScheduler` (pure), `PushSummary`, payload builder, `PushTransport` seam + `UnixPushTransport`, dispatcher task, route helpers, `PushView`. No `tracing` macro. |
| `crates/remote/src/webpush.rs` (**new**) | pure crypto: `VapidKey`, `vapid_authorization`, `JwtCache`, `encrypt_with`, `encrypt`, `derive_keys` (`#[doc(hidden)]`), `parse_subscription_keys`. No `tracing` macro, no clock read (time is passed in). |
| `crates/remote/src/lib.rs` | `pub mod push; pub mod webpush;`, new constants (§3.1), compile-time relations. |
| `crates/remote/src/config.rs` | `PushConfig`, `PushPreviews`, keys `push_notifications`, `vapid_subject`, `push_socket_path`, `push_previews`; new `ConfigError` variants; `resolve_push_store_path`. |
| `crates/remote/src/alerts.rs` | `pub fn is_live(at_us, started_us, now_us) -> bool`; accessor `AlertBook::started_us(&self) -> u64` (the field stays private); `AlertsRuntime::setup` gains an `Option<LiveAttemptSink>`; `record()` calls the sink for each recorded live attempt (no I/O, no await under the mutex). |
| `crates/remote/src/routes.rs` | `Route::Push`, `Route::PushSubscribe`, `Route::PushUnsubscribe`, `Route::PushTest`, `AssetId::ServiceWorker` route `/sw.js`; path constants; `accepts_body` adds exactly the subscribe and unsubscribe paths; `check_push_csrf`. |
| `crates/remote/src/assets.rs` + `assets/sw.js` (**new file**) | `AssetId::ServiceWorker`, `text/javascript; charset=utf-8`. |
| `crates/remote/src/server.rs` | `ServerState::with_push(settings, transport)`; `serve` sets up the push runtime and spawns the dispatcher; four route handlers; `/sw.js` served like other assets. |
| `crates/remote/src/audit.rs` | five fixed-text events (W-14). |
| `crates/remote/src/main.rs` | wires `PushSettings` (store path, subject, previews, socket path) and `UnixPushTransport`; subcommands `push list`, `push remove <N>`, `push reset`. |
| `crates/remote/Cargo.toml` | add `soos-push-protocol`, `hmac`, `aes-gcm` (all `{ workspace = true }`). |
| `crates/remote/assets/app.js`, `index.html`, `style.css` | "Notifications" card (§9). |
| Root `Cargo.toml` | members `crates/push-protocol`, `crates/push-sender`; `[workspace.dependencies]`: `hmac = "0.12"`, `ureq = { version = "=3.4.2", default-features = false, features = ["rustls"] }`, `soos-push-protocol = { path = "crates/push-protocol", version = "0.1.0" }`. The `p256` line is **not** touched. |
| `deny.toml` | skip reasons of `getrandom@0.2.17` and `windows-sys@0.52.0` rewritten to also name `ring` (via `soos-push-sender`); any further duplicate found by `cargo deny check` gets a reason from `cargo tree -i` (enforced by `dependency_tooling_contract.rs`). |
| `packaging/soos-push-sender.service` (**new**) | §8.2. `packaging/soos-remote.service` **unchanged** (RMC-S5, RMC-S26). |
| `scripts/install_remote.sh` | builds and installs both binaries and both units (`-p soos-remote -p soos-push-sender`), new commented template keys, prints the owner steps; never enables anything; `--uninstall` removes both. |
| `tests/invariants/src/lib.rs` | `mod remote_push_contract;`; `"push-protocol"` and `"push-sender"` appended to `business_crates` of `test_business_crates_forbid_unsafe_code`. |
| `scripts/candid_review.sh` | `BUSINESS_CRATES` gains `push-protocol push-sender`. |
| `tests/invariants/src/remote_push_contract.rs` (**new**) | RMC-S32–RMC-S41 (§12.9). |
| `Docs/REMOTE_COMPANION.md` | new §2d "Push notifications (opt-in)", §5 keys, §6 routes, §8 residual risks, §9 out-of-scope list updated. `Docs/README.md` index unchanged (same page). |
| `AI/DECISIONS.md` | new ADR; supersede notes on 2026-10-05 item (5) and level-1 item (1). |

### 1.2 Consumers

`soos-remote` is a leaf (RMC-S4); the two new crates are consumed only by `soos-remote` (protocol) and by nobody
(sender binary). `soos-daemon`, `pam_soos.so`, the IPC protocol, the GUI and every other crate are untouched. The
level-1 alert view JSON, routes and SSE events are unchanged (no new field in `AlertsView`).

### 1.3 Out of scope (each needs its own ADR)

Lock/unlock-state notifications; "monitoring lost" notifications; Edge/WNS (`*.notify.windows.com`) or any other
push host; widening to `*.push.apple.com` (only if the owner's real subscription shows another Apple host, RMC74);
notification actions (buttons) or replies; the Badging API beyond what the worker sets; a system-wide unit or
`DynamicUser=`; sending the typed text (§0.1); a cloud relay.

## 2. `soos-push-protocol` (pure, shared)

### 2.1 Constants (`crates/push-protocol/src/lib.rs`, the only definition of each)

| Name | Value | Meaning / bound behaviour |
|---|---|---|
| `PUSH_HOSTS: [&str; 3]` | `["web.push.apple.com", "fcm.googleapis.com", "updates.push.services.mozilla.com"]` | exact host allowlist; anything else → `EndpointError::HostNotAllowed` |
| `MAX_PUSH_ENDPOINT_BYTES` | `1024` | longer → `TooLong` |
| `MAX_PUSH_BODY_BYTES` | `4096` | encrypted body (RFC 8291 §4: a push service need not accept more); longer → `FrameError::Body` |
| `MAX_AUTHORIZATION_BYTES` | `1024` | `Authorization` value; longer → `FrameError::Authorization` |
| `MAX_PUSH_TTL_S` | `2_419_200` | 28 days; larger → `FrameError::Ttl` |
| `MAX_TOPIC_LEN` | `32` | RFC 8030 §5.4 |
| `MAX_PUSH_FRAME_BYTES` | `12_288` | request frame payload (after the 4-byte prefix); larger → `FrameError::TooLarge` before any allocation beyond the bound |
| `MAX_PUSH_REPLY_BYTES` | `256` | reply frame payload |
| `PUSH_FRAME_VERSION` | `1` | other → `FrameError::Version` |
| `PUSH_FRAME_IO_TIMEOUT_MS` | `2000` | each read or write of a frame (both sides) |
| `PUSH_SEND_TIMEOUT_MS` | `10_000` | sender: whole outbound exchange (`timeout_global`) |
| `PUSH_CONNECT_TIMEOUT_MS` | `5000` | sender: TCP+TLS connect |
| `MAX_RETRY_AFTER_S` | `300` | `Retry-After` cap |
| `MAX_RESOLVED_ADDRESSES` | `16` | sender: addresses kept after filtering |
| `MAX_PUSH_RESPONSE_HEADER_BYTES` | `16_384` | sender: response head bound |
| `MAX_PUSH_RESPONSE_BODY_BYTES` | `1024` | sender: response body read and discarded |
| `PUSH_SOCKET_DIR_NAME` | `"soos-push"` | under `$XDG_RUNTIME_DIR` (created by `RuntimeDirectory=`) |
| `PUSH_SOCKET_FILE_NAME` | `"push.sock"` | |
| `EXIT_CONFIG` / `EXIT_RUNTIME` | `78` / `1` | sender exit codes (same meaning as `soos-remote`) |

Compile-time relations: `MAX_PUSH_FRAME_BYTES ≥ 4 * MAX_PUSH_BODY_BYTES / 3 + MAX_PUSH_ENDPOINT_BYTES +
MAX_AUTHORIZATION_BYTES + 512`; `PUSH_CONNECT_TIMEOUT_MS < PUSH_SEND_TIMEOUT_MS`.

### 2.2 Endpoint

```rust
/// A validated push endpoint (capability URL: never logged, never echoed).
#[derive(Clone, PartialEq, Eq)]          // no derived Debug
pub struct PushEndpoint(String);
impl fmt::Debug for PushEndpoint { /* writes "PushEndpoint(<redacted>)" */ }
impl PushEndpoint {
    pub fn parse(raw: &str) -> Result<Self, EndpointError>;
    #[must_use] pub fn as_str(&self) -> &str;
    /// The allowlisted host (a `PUSH_HOSTS` element, `'static`).
    #[must_use] pub fn host(&self) -> &'static str;
    /// `https://<host>` (the VAPID `aud`, RFC 8292 §2: origin, no path, no trailing slash).
    #[must_use] pub fn origin(&self) -> String;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]   // fixed texts, never the value
pub enum EndpointError { TooLong, NotHttps, UserInfo, Port, HostNotAllowed, BadPath }
```

`parse` rules, in order: length 1..=`MAX_PUSH_ENDPOINT_BYTES` (`TooLong`); every byte printable ASCII `0x21..=0x7E`
(`BadPath`); starts with exactly `https://` (lowercase; `HTTPS://`, `http://` → `NotHttps`); authority = bytes up to
the first `/` after the scheme (none → `BadPath`); authority containing `@` → `UserInfo`, `:` or `[` → `Port`;
authority byte-equal to one `PUSH_HOSTS` element, else `HostNotAllowed` (so a trailing dot, a suffix such as
`web.push.apple.com.evil.example`, an IP literal, uppercase or a percent-encoded host are refused); path = the rest,
starting with `/`, length ≥ 2, every byte in `A–Z a–z 0–9 - . _ ~ / : = + %`, no `//`, no `.` or `..` segment
(`BadPath`; this refuses `?`, `#`, `\`, spaces).

### 2.3 Public addresses

```rust
/// Pure. True only for globally routable unicast addresses.
#[must_use] pub fn is_public_address(ip: std::net::IpAddr) -> bool;
```

Refused IPv4: `0.0.0.0/8`, `10.0.0.0/8`, `100.64.0.0/10` (CGNAT, tailnet, MagicDNS `100.100.100.100`),
`127.0.0.0/8`, `169.254.0.0/16`, `172.16.0.0/12`, `192.0.0.0/24`, `192.0.2.0/24`, `192.88.99.0/24`,
`192.168.0.0/16`, `198.18.0.0/15`, `198.51.100.0/24`, `203.0.113.0/24`, `224.0.0.0/4`, `240.0.0.0/4` (incl.
`255.255.255.255`). Refused IPv6: everything outside `2000::/3`, and inside it `2001::/23` (IETF special purpose,
incl. Teredo `2001::/32`), `2001:db8::/32`, `2002::/16` (6to4); plus explicitly `::/128`, `::1/128`,
`::ffff:0:0/96` (IPv4-mapped, refused whatever the embedded address), `64:ff9b::/96`, `64:ff9b:1::/48`,
`100::/64`, `fc00::/7` (incl. tailnet `fd7a:115c:a1e0::/48`), `fe80::/10`, `fec0::/10`, `ff00::/8`. Implemented
with integer masks on the address bits (`u32::from`, `u128::from`), never on text.

### 2.4 Frames

A frame is a 4-byte big-endian length `n` followed by `n` bytes of compact JSON (`serde_json`). One request and
one reply per connection; the client half-closes nothing; both sides close after the reply.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Urgency { VeryLow, Low, Normal, High }

/// No Debug (authorization and body are secret-bearing).
#[derive(Clone, PartialEq, Eq)]
pub struct DeliveryRequest {
    pub endpoint: PushEndpoint,
    /// `vapid t=<jwt>, k=<key>`; 1..=MAX_AUTHORIZATION_BYTES printable ASCII, starts with `vapid t=`.
    pub authorization: zeroize::Zeroizing<String>,
    /// 1..=MAX_PUSH_TTL_S (0 refused: Apple `BadTtl`).
    pub ttl_s: u32,
    pub urgency: Urgency,
    /// 1..=MAX_TOPIC_LEN chars of the base64url alphabet, or None.
    pub topic: Option<String>,
    /// 1..=MAX_PUSH_BODY_BYTES (an empty body is refused: the payload is mandatory, W-11).
    pub body: zeroize::Zeroizing<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome { Delivered, Gone, Rejected, Retry, Refused }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryReply {
    pub outcome: Outcome,
    /// The push service status, when a response was received (100..=599).
    pub status: Option<u16>,
    /// Seconds, 1..=MAX_RETRY_AFTER_S, only with `Outcome::Retry`.
    pub retry_after_s: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]   // fixed texts
pub enum FrameError { Truncated, TooLarge, Json, Version, Endpoint(EndpointError), Authorization, Ttl, Topic, Body, Reply }

pub fn encode_request(req: &DeliveryRequest) -> Result<zeroize::Zeroizing<Vec<u8>>, FrameError>; // prefix + JSON
pub fn decode_request(payload: &[u8]) -> Result<DeliveryRequest, FrameError>;                  // payload without prefix
pub fn encode_reply(reply: &DeliveryReply) -> Result<Vec<u8>, FrameError>;
pub fn decode_reply(payload: &[u8]) -> Result<DeliveryReply, FrameError>;
/// Validates a 4-byte prefix against `max` before any allocation.
pub fn frame_len(prefix: [u8; 4], max: usize) -> Result<usize, FrameError>;
```

Request JSON (exact key set, `deny_unknown_fields`, every key mandatory, `topic` may be `null`):
`{"v":1,"endpoint":"https://…","authorization":"vapid t=…, k=…","ttl":43200,"urgency":"high","topic":"soos-alerts","body":"<base64url, no padding>"}`.
Reply JSON: `{"v":1,"outcome":"delivered","status":201,"retry_after_s":null}`. Decoders re-run every validation
(the endpoint through `PushEndpoint::parse`), refuse `=` padding and the standard alphabet in `body`, refuse
`status` outside 100..=599, refuse `retry_after_s` unless `outcome == retry` and within 1..=`MAX_RETRY_AFTER_S`.
Intermediate JSON buffers of a request are `Zeroizing`.

### 2.5 Status classification (sender, pure)

```rust
#[must_use] pub fn classify_status(status: u16, retry_after: Option<&[u8]>) -> DeliveryReply;
#[must_use] pub fn parse_retry_after(raw: &[u8]) -> Option<u32>;
```

| Status | Outcome |
|---|---|
| 200..=299 | `Delivered` |
| 404, 410 | `Gone` (RFC 8030 §6.2/§7.3; Apple `Unregistered`; Mozilla errno 102–106) |
| 429, 500..=599 | `Retry`, `retry_after_s = parse_retry_after(value)` |
| 300..=399 | `Rejected` (never followed) |
| anything else (1xx, 400–499 other than 404/410/429) | `Rejected` |

`parse_retry_after`: 1..=10 ASCII digits only (delta-seconds; an HTTP-date, a sign, spaces or anything else →
`None`); value 0 → `None`; result `min(value, MAX_RETRY_AFTER_S)`.

## 3. Constants and configuration of `soos-remote`

### 3.1 Constants (`crates/remote/src/lib.rs`)

| Name | Value | Meaning |
|---|---|---|
| `MAX_PUSH_SUBSCRIPTIONS` | `4` | 5th distinct endpoint → `409 too_many_subscriptions`; never evicted |
| `MAX_PUSH_STORE_BYTES` | `16_384` | larger file → `StoreError`-class refusal |
| `PUSH_STORE_FILE_NAME` | `"remote-push.json"` | sibling of the credential store |
| `MAX_PUSH_SUBSCRIBE_BODY_BYTES` | `2048` | subscribe/unsubscribe body; larger → `413 body_too_large` (after the shared 8 KiB read bound) |
| `MAX_PUSH_PLAINTEXT_BYTES` | `1024` | notification JSON; larger → never sent (`last_delivery = failed`) |
| `PUSH_RECORD_SIZE` | `4096` | RFC 8188 `rs` |
| `PUSH_COALESCE_MS` | `3000` | first send after a quiet period waits this long |
| `PUSH_MIN_INTERVAL_MS` | `30_000` | spacing between two alert notifications |
| `PUSH_MAX_PER_HOUR` | `20` | alert notifications per rolling 3 600 000 ms |
| `PUSH_MAX_ATTEMPT_AGE_MS` | `300_000` | a live attempt older than this when recorded is not pushed |
| `PUSH_RETRY_DELAYS_MS: [u64; 2]` | `[5_000, 30_000]` | retries of one subscription for one message |
| `PUSH_TTL_S` | `43_200` | `TTL` header |
| `PUSH_TOPIC` | `"soos-alerts"` | `Topic` header of alert summaries only |
| `PUSH_TEST_TOPIC` | `"soos-test"` | `Topic` header of the test notification only (never equal to `PUSH_TOPIC`, so a test never replaces an undelivered alert) |
| `PUSH_URGENCY` | `Urgency::High` | |
| `PUSH_TEST_MIN_INTERVAL_MS` | `10_000` | `POST /api/push/test` gate → `429` |
| `PUSH_ROUTE_MIN_INTERVAL_MS` | `1000` | shared gate of subscribe/unsubscribe → `429` |
| `PUSH_EXCHANGE_TIMEOUT_MS` | `18_000` | `soos-remote` side: bound of the **whole** exchange (Unix connect + request write + wait for the reply + reply read); strictly above the sender's own worst case (§3.1 relation), so `soos-remote` never abandons an exchange the sender is still completing |
| `PUSH_CONNECT_UNIX_TIMEOUT_MS` | `1000` | connect to the sender socket |
| `VAPID_JWT_LIFETIME_S` | `43_200` | `exp = now + 12 h` (< Apple's 24 h) |
| `VAPID_JWT_REUSE_S` | `3600` | a cached token is reused for at most 1 h (Apple: refresh at most hourly) |
| `MAX_VAPID_SUBJECT_LEN` | `256` | |
| `MIN_PLAUSIBLE_UNIX_S` | `1_700_000_000` | a Unix clock below this → no JWT, nothing sent (`failed`) |
| `ACTION_PUSH_SUBSCRIBE` / `ACTION_PUSH_UNSUBSCRIBE` / `ACTION_PUSH_TEST` | `"push-subscribe"` / `"push-unsubscribe"` / `"push-test"` | `X-Soos-Action` values |

Compile-time relations: `PUSH_COALESCE_MS < PUSH_MIN_INTERVAL_MS`; `PUSH_MAX_PER_HOUR as u64 * PUSH_MIN_INTERVAL_MS
<= 3_600_000`; `MAX_PUSH_SUBSCRIBE_BODY_BYTES <= MAX_AUTH_BODY_BYTES`; `MAX_PUSH_PLAINTEXT_BYTES + 1 + 16 + 86 <=
soos_push_protocol::MAX_PUSH_BODY_BYTES`; `PUSH_RECORD_SIZE > MAX_PUSH_PLAINTEXT_BYTES + 17`;
`PUSH_EXCHANGE_TIMEOUT_MS > PUSH_CONNECT_UNIX_TIMEOUT_MS + 3 * soos_push_protocol::PUSH_FRAME_IO_TIMEOUT_MS +
soos_push_protocol::PUSH_SEND_TIMEOUT_MS` (1 000 + 3 × 2 000 + 10 000 = 17 000 < 18 000: sender prefix read, frame read
and reply write each bounded by `PUSH_FRAME_IO_TIMEOUT_MS`, the outbound exchange by `PUSH_SEND_TIMEOUT_MS`, which
includes DNS); `PUSH_TOPIC != PUSH_TEST_TOPIC` (const byte comparison); `PUSH_TEST_TOPIC.len() <=
soos_push_protocol::MAX_TOPIC_LEN`;
`VAPID_JWT_REUSE_S < VAPID_JWT_LIFETIME_S`; `PUSH_TTL_S <= soos_push_protocol::MAX_PUSH_TTL_S`;
`PUSH_TOPIC.len() <= soos_push_protocol::MAX_TOPIC_LEN`.

### 3.2 Configuration (`remote.toml`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PushPreviews { #[default] Detailed, Generic }

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PushConfig {
    /// `push_notifications`; default false.
    pub enabled: bool,
    /// `vapid_subject`; None → `https://<rp_id>` at resolution time.
    pub vapid_subject: Option<String>,
    /// `push_socket_path`; None → `$XDG_RUNTIME_DIR/soos-push/push.sock`.
    pub socket_path: Option<PathBuf>,
    /// `push_previews`: "detailed" | "generic".
    pub previews: PushPreviews,
}
// RemoteConfig gains: pub push: PushConfig,
```

New `ConfigError` variants (fixed texts, never echo a value): `PushRequiresAlerts`, `PushRequiresRpId`,
`InvalidVapidSubject`, `InvalidPushSocketPath { max: usize }`, `InvalidPushPreviews`. All are configuration-class
(exit 78).

`vapid_subject` rules: 1..=256 bytes, printable ASCII without space; either `mailto:<local>@<domain>` with exactly one
`@`, `local` 1..=64 bytes without `<>()[],;:\"`, `domain` a valid host name (`identity::is_valid_host_name`) with at
least one dot; or `https://<host>` optionally followed by `/` and a path of printable ASCII without `?`/`#`, `host`
a valid host name with at least one dot and no port. In both forms the host or domain must not be `localhost` or end
with `.localhost`, `.local`, `.invalid`, `.test`, `.example`, `.internal`, `.home.arpa`, and `mailto:mailto:` is
refused (research §3.2: Apple `403 BadJwtToken`).

`push_socket_path`: absolute, ≤ `MAX_SOCKET_PATH_LEN`, a parent and a file name, no trailing `/` (same rules as
`socket_path`). Default: `$XDG_RUNTIME_DIR/` + `PUSH_SOCKET_DIR_NAME` + `/` + `PUSH_SOCKET_FILE_NAME`; no runtime
dir → the existing missing-runtime-dir error, only when push is enabled.

`pub fn resolve_push_store_path(credentials_path: &Path) -> Result<PathBuf, ConfigError>` — the sibling
`remote-push.json` (same rules and errors as `resolve_alerts_ack_path`).

### 3.3 Sentinel semantics

| Value | Meaning |
|---|---|
| `push_notifications` absent / `false` | disabled (W-2); no file, no task, `403 push_disabled` |
| `push_notifications = true`, `password_alerts` absent/false | `ConfigError::PushRequiresAlerts` |
| `push_notifications = true`, no `rp_id` | `ConfigError::PushRequiresRpId` |
| `vapid_subject` absent | `https://<rp_id>` |
| `vapid_subject = ""` | `InvalidVapidSubject` |
| `push_previews` absent | `detailed` |
| store file absent at start | created with a fresh key and `[]` |
| store file present but invalid/insecure | push `unavailable` / `store_failed`, file untouched |
| store file deleted while running | **never recreated by the service**: `load()` returns `Ok(None)`; `upsert`/`remove_*` return `PushStoreError::Missing` and create nothing; `GET /api/push` reports `unavailable` / `store_missing` (`public_key: null`) for that response; subscribe, unsubscribe and test answer `503 store_unavailable`; the dispatcher drops due messages (`last_delivery = failed`, undelivered counts carried, §5.6). Recovery: `soos-remote push reset` (fresh key, no subscription) or a service restart (start creates it); the page then sees a different public key and asks to re-enable (§9). Rationale: a subscription the phone made with the old key is useless under a new key, so the service never silently mints one. |
| `soos-remote push reset` | after the ownership checks (regular file or absent, owned by the uid, symlink never followed), atomically replaces the file with a fresh key and `[]` (temp + `fsync` + `rename` under `flock`); the running service sees the new key at its next load and clears its `JwtCache` |
| 0 subscriptions | the dispatcher discards due summaries (nothing to send); `POST /api/push/test` → `409 no_subscriptions` |
| `Retry-After: 0` or absent | the fixed retry delay |
| Unix clock 0 or `< MIN_PLAUSIBLE_UNIX_S` | nothing sent, `last_delivery = failed` |

## 4. Crypto (`crates/remote/src/webpush.rs`, pure)

```rust
/// VAPID P-256 key. No derived Debug (manual "VapidKey(<redacted>)"); the secret zeroizes on drop.
pub struct VapidKey(p256::SecretKey);
impl VapidKey {
    /// Draws 32 bytes from `random` and retries (at most 4 draws) while the scalar is zero or ≥ n.
    pub fn generate(random: &RandomSource) -> Result<Self, WebPushError>;
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self, WebPushError>;
    pub fn to_bytes(&self) -> zeroize::Zeroizing<[u8; 32]>;
    /// 65-byte uncompressed SEC1 point, base64url without padding (87 chars): `applicationServerKey` and `k=`.
    pub fn public_key_b64(&self) -> String;
}

/// The UA keys of one subscription. No Debug.
pub struct UaKeys { pub p256dh: p256::PublicKey, pub auth: zeroize::Zeroizing<[u8; 16]> }
/// `p256dh`: base64url (no padding, URL alphabet) of exactly 65 bytes starting 0x04 and on the curve;
/// `auth`: base64url of exactly 16 bytes.
pub fn parse_subscription_keys(p256dh_b64: &str, auth_b64: &str) -> Result<UaKeys, WebPushError>;

/// RFC 8291 + RFC 8188 `aes128gcm`, one record, rs = PUSH_RECORD_SIZE, delimiter 0x02, no padding.
/// Body = salt(16) ‖ rs(u32 BE) ‖ idlen(1)=65 ‖ as_public(65) ‖ AES-128-GCM(plaintext ‖ 0x02) with its 16-byte tag.
pub fn encrypt_with(plaintext: &[u8], ua: &UaKeys, as_secret: &[u8; 32], salt: &[u8; 16])
    -> Result<zeroize::Zeroizing<Vec<u8>>, WebPushError>;
/// Draws as_secret (32 bytes, retried like `generate`) and salt (16 bytes) from `random`, then `encrypt_with`.
pub fn encrypt(plaintext: &[u8], ua: &UaKeys, random: &RandomSource)
    -> Result<zeroize::Zeroizing<Vec<u8>>, WebPushError>;
/// Test seam: (CEK, NONCE) of RFC 8291 §3.4 for the RFC Appendix A vector.
#[doc(hidden)]
pub fn derive_keys(ua: &UaKeys, as_secret: &[u8; 32], salt: &[u8; 16])
    -> Result<(zeroize::Zeroizing<[u8; 16]>, zeroize::Zeroizing<[u8; 12]>), WebPushError>;

/// `vapid t=<jwt>, k=<public key b64url>`; JWT header `{"typ":"JWT","alg":"ES256"}`, claims in this order
/// `{"aud":"<origin>","exp":<now_s + VAPID_JWT_LIFETIME_S>,"sub":"<subject>"}`, ES256 raw r‖s, base64url.
pub fn vapid_authorization(key: &VapidKey, origin: &str, subject: &str, now_s: u64)
    -> Result<zeroize::Zeroizing<String>, WebPushError>;

/// At most one token per allowlisted origin (≤ PUSH_HOSTS.len() entries); reused while now_s < issued + VAPID_JWT_REUSE_S.
pub struct JwtCache { /* bounded map origin → (issued_s, token) */ }
impl JwtCache {
    pub fn new() -> Self;
    pub fn authorization(&mut self, key: &VapidKey, origin: &str, subject: &str, now_s: u64)
        -> Result<zeroize::Zeroizing<String>, WebPushError>;
    /// Drops every cached token (key replaced).
    pub fn clear(&mut self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]   // fixed texts
pub enum WebPushError { Random, InvalidKey, InvalidSubscriptionKeys, PlaintextTooLarge, Encrypt, Clock, Encode }
```

Key schedule (RFC 8291 §3.4), all intermediates `Zeroizing`: `ecdh = x(ua_public · as_scalar)` (32 bytes, via
`ProjectivePoint` arithmetic and `AffineCoordinates::x`); `PRK_key = HMAC(auth, ecdh)`; `key_info = "WebPush:
info" ‖ 0x00 ‖ ua_public(65) ‖ as_public(65)`; `IKM = HMAC(PRK_key, key_info ‖ 0x01)`; `PRK = HMAC(salt, IKM)`;
`CEK = HMAC(PRK, "Content-Encoding: aes128gcm" ‖ 0x00 ‖ 0x01)[0..16]`; `NONCE = HMAC(PRK, "Content-Encoding:
nonce" ‖ 0x00 ‖ 0x01)[0..12]`. A zero ECDH result (impossible for a valid point and non-zero scalar) →
`Encrypt`. `vapid_authorization` with `now_s < MIN_PLAUSIBLE_UNIX_S` → `Clock`; `exp` uses `checked_add`
(`Clock` on overflow).

## 5. Push runtime (`crates/remote/src/push.rs`)

### 5.1 Store

```rust
/// One stored subscription. No Debug.
pub struct PushSubscription { pub endpoint: PushEndpoint, pub keys: UaKeys, pub created_unix_s: u64 }

/// The parsed store. No Debug.
pub struct PushStoreFile { pub key: VapidKey, pub subscriptions: Vec<PushSubscription> }   // ≤ MAX_PUSH_SUBSCRIPTIONS

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]   // fixed texts
pub enum PushStoreError { Refused, Malformed, Io, Locked, Random, TooMany, Missing }

pub struct PushStore { /* path, owner uid */ }
impl PushStore {
    pub fn new(path: PathBuf, owner_uid: u32) -> Self;
    /// Absent → Ok(None). Present → strict checks (regular file, owner, mode exactly 0600, ≤ MAX_PUSH_STORE_BYTES,
    /// O_NOFOLLOW|O_NONBLOCK|O_CLOEXEC), then JSON.
    pub fn load(&self) -> Result<Option<PushStoreFile>, PushStoreError>;
    /// Service start only: load; absent → generate a key (random seam) and write `{"version":1,…,"subscriptions":[]}`
    /// (O_EXCL temp + fsync + rename under flock). Never writes over a present file.
    pub fn load_or_create(&self, random: &RandomSource) -> Result<PushStoreFile, PushStoreError>;
    /// Locked read-modify-write (flock, STORE_LOCK_TIMEOUT_MS, temp + fsync + rename, credentials.rs helpers).
    /// Absent file → Err(Missing), nothing created.
    pub fn upsert(&self, sub: PushSubscription, random: &RandomSource) -> Result<UpsertResult, PushStoreError>;
    /// Absent file → Err(Missing), nothing created.
    pub fn remove_endpoint(&self, endpoint: &PushEndpoint, random: &RandomSource) -> Result<bool, PushStoreError>;
    /// CLI, 1-based. Absent file → Err(Missing), nothing created.
    pub fn remove_index(&self, index: usize, random: &RandomSource) -> Result<bool, PushStoreError>;
    /// CLI `push reset`: the path must be absent or a regular file owned by the uid (symlink or foreign owner →
    /// Refused, untouched; any mode and any content accepted, so an invalid store can be reset); writes a fresh key
    /// and no subscription atomically under flock. RNG failure → Random, the old file untouched.
    pub fn reset(&self, random: &RandomSource) -> Result<PushStoreFile, PushStoreError>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertResult { Added, Replaced }
```

On-disk JSON (`deny_unknown_fields`, exact):
`{"version":1,"vapid_private_key":"<43 chars b64url>","subscriptions":[{"endpoint":"https://…","p256dh":"<87>","auth":"<22>","created_unix_s":1759700000}]}`.
Every subscription is re-validated on load (`PushEndpoint::parse`, `parse_subscription_keys`); any invalid entry,
a duplicate endpoint, more than 4 entries, `version != 1` or an invalid key → `Malformed` (whole file refused, never
repaired). Parse buffers are `Zeroizing`. `upsert` on a full store with a new endpoint → `TooMany` (nothing written);
on an existing endpoint replaces the keys and keeps `created_unix_s`. Only `load_or_create` (service start) and `reset` (CLI)
ever create the file (§3.3 "store file deleted while running").

### 5.2 Scheduler (pure)

```rust
/// Accumulated live attempts since the last send. Built only from enums, counts and times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PushSummary {
    pub wrong_password: u32,          // saturating
    pub locked_out: u32,              // saturating
    pub newest_source: SourceClass,
    pub newest_account: AccountClass,
    pub newest_kind: AttemptKind,
    pub first_unix_ms: u64,
    pub last_unix_ms: u64,
}
impl PushSummary {
    /// Pure fold of an undelivered (older) summary into a newer one: counts `saturating_add`, `first_unix_ms` = min,
    /// `last_unix_ms` = max, `newest_*` from the operand with the greater `last_unix_ms` (ties: `newer`).
    #[must_use] pub fn merge(older: &PushSummary, newer: &PushSummary) -> PushSummary;
}

pub struct PushScheduler { /* pending: Option<(PushSummary, first_noted_ms)>, last_sent_ms, sent: VecDeque<u64> ≤ PUSH_MAX_PER_HOUR */ }
impl PushScheduler {
    pub fn new() -> Self;
    /// Adds one live attempt noted at monotonic `now_ms`.
    pub fn note(&mut self, attempt: Attempt, now_ms: u64);
    /// When the pending summary may be sent: max(first_noted + PUSH_COALESCE_MS, last_sent + PUSH_MIN_INTERVAL_MS,
    /// oldest_sent + 3_600_000 when PUSH_MAX_PER_HOUR sends are in the window); None without a pending summary.
    #[must_use] pub fn due_at(&self) -> Option<u64>;
    /// Some(summary) iff due_at() <= now_ms; records the send time, drops times older than 1 h, clears pending.
    pub fn take(&mut self, now_ms: u64) -> Option<PushSummary>;
}
```

`newest_*` follow the attempt with the greatest `at_us` (ties: the later `note`). Monotonic milliseconds come from
`tokio::time::Instant` relative to the runtime's start (paused-time testable); arithmetic is `saturating_*`.
`merge` is the only way counts of two summaries are combined (the carry of §5.6 uses it); it never adds a send, so
the rate bounds of the scheduler are unchanged.

### 5.3 Live attempts (`alerts.rs`)

```rust
/// Pure: at_us >= started_us && now_us.saturating_sub(at_us) <= PUSH_MAX_ATTEMPT_AGE_MS * 1000.
#[must_use] pub fn is_live(at_us: u64, started_us: u64, now_us: u64) -> bool;
/// Called under the runtime mutex for each attempt the book recorded; must not block, await or do I/O.
pub(crate) type LiveAttemptSink = Arc<dyn Fn(Attempt) + Send + Sync>;
```

`AlertsRuntime::record` calls the sink for each attempt `book.record(attempt)` accepted (`Ok`) with
`is_live(attempt.at_us, book.started_us(), now_us)`; `AlertBook` gains the read-only accessor
`#[must_use] pub fn started_us(&self) -> u64` (the field stays private). The production sink locks the scheduler's `std::sync::Mutex` (poison → attempt dropped
for push only; alerts unaffected), calls `note`, and wakes the dispatcher through a `tokio::sync::Notify`. Replayed
attempts (journal time before the start) are therefore never pushed; a follower restart resuming from a cursor
does not re-push (attempts are recorded once).

**Lock order (binding for the whole push runtime).** The only nesting allowed is *alerts runtime mutex → scheduler
mutex* (the sink, called under the alerts mutex, takes the scheduler mutex). Never the reverse: no code holding the
scheduler mutex takes the alerts mutex, reads the alerts view or calls into `AlertsRuntime`. The dispatcher takes
the scheduler mutex alone, briefly, to call `due_at`/`take` and to fold carries, and never holds it (nor the
`JwtCache` mutex) across an `.await`, a store access (`flock`) or a transport call. The `JwtCache` mutex and the
dispatcher-state mutex are leaves: nothing else is taken while they are held. The store's `flock` is taken only
with no in-process mutex held.

### 5.4 Payload

```rust
/// Declarative Web Push JSON, ≤ MAX_PUSH_PLAINTEXT_BYTES.
pub fn alert_payload(summary: &PushSummary, previews: PushPreviews, rp_id: &str) -> Result<Vec<u8>, WebPushError>;
pub fn test_payload(rp_id: &str) -> Result<Vec<u8>, WebPushError>;
```

Exact shape (serde structs, field order as written):
`{"web_push":8030,"notification":{"title":"<title>","body":"<body>","navigate":"https://<rp_id>/","lang":"en"},"soos":{"v":1,"kind":"alerts"|"test","wrong_password":<u32>,"locked_out":<u32>,"source":"<source>"|null,"account":"<account>"|null,"last_unix_ms":<u64>|null}}`.

Text (fixed vocabulary; `n = wrong_password`, `m = locked_out`):

| Case | title | body |
|---|---|---|
| detailed, `n ≥ 1` | `Failed password on your PC` | `"<n> wrong password(s) — <source label>, <account label>"` + (`m ≥ 1`: `" and <m> attempt(s) while locked out"`) |
| detailed, `n = 0`, `m ≥ 1` | `Failed password on your PC` | `"<m> attempt(s) while locked out — <source label>, <account label>"` |
| generic (any counts) | `Security alert on your PC` | `Open soos for details` (and `source`, `account` are `null`, counts kept) |
| test | `soos test notification` | `Notifications from your PC work` |

`(s)` is rendered as the exact singular or plural (`1 wrong password`, `2 wrong passwords`). Source labels:
`lock_screen` → `lock screen`, `sudo` → `sudo`, `login` → `login`, `other` → `other`; account labels: `owner` →
`your account`, `root` → `root`, `other` → `another account`. `source`/`account` in `soos` are the newest attempt's
enum spellings. No time text (the phone shows the delivery time; `last_unix_ms` is for the worker). `rp_id` comes
from the validated configuration.

### 5.5 Transport seam and production transport

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]   // fixed texts
pub enum TransportError { Unavailable, Timeout, Protocol }

pub trait PushTransport: Send + Sync + 'static {
    fn deliver(&self, request: DeliveryRequest) -> BoxFuture<'_, Result<DeliveryReply, TransportError>>;
}

/// Production: connects to the sender socket per request.
pub struct UnixPushTransport { socket_path: PathBuf, owner_uid: u32 }
```

`UnixPushTransport::deliver`: `symlink_metadata(socket_path)` must be a socket owned by `owner_uid` (else
`Unavailable`, no connect); `tokio::net::UnixStream::connect` within `PUSH_CONNECT_UNIX_TIMEOUT_MS`
(`Unavailable`); `peer_cred().uid() == owner_uid` (else `Unavailable`); write the frame, read the 4-byte prefix and
the reply (`frame_len(…, MAX_PUSH_REPLY_BYTES)`), each I/O under `PUSH_FRAME_IO_TIMEOUT_MS` except the reply
prefix, which waits for the sender's outbound exchange; the **whole** `deliver` call (connect included) is wrapped in
one `tokio::time::timeout(PUSH_EXCHANGE_TIMEOUT_MS)` (`Timeout`), which by the §3.1 relation is longer than the
sender's worst case, so a reply the sender is still allowed to produce is never abandoned (no duplicate notification
through a premature retry); malformed reply → `Protocol`. No retry inside the transport.

### 5.6 Dispatcher task

`pub(crate) async fn run_dispatcher(push: Arc<PushRuntime>)` — spawned by `serve` in the supervised set, only when
push is enabled and the runtime is not `unavailable` at start. No push **error** ends it (every error is an
outcome below); like the follower, a **panic** of the task ends `serve` with `TaskPanicked` (supervised-set rule,
`server.rs:673`). Loop: wait for the `Notify` or the earliest of (`scheduler.due_at()`, the earliest retry, a
queued test); then, in this order: a queued test message; due retries; a due summary (`take`). For a message to
send: `store.load()` (error or `Ok(None)` → `last_delivery = failed`, audit `push delivery failed` once per
transition, nothing sent; an alert summary is folded into the carry of every subscription known from the last
successful load); 0 subscriptions → discard (no carry: nobody to tell); build the payload; for each subscription
**sequentially**: for an alert summary, `effective = merge(carry[endpoint], summary)` when a carry exists (else
`summary`), then the payload of `effective`; `encrypt` (random seam), `JwtCache::authorization(key,
endpoint.origin(), subject, now_s)`, a `DeliveryRequest { endpoint, authorization, ttl_s: PUSH_TTL_S, urgency: High,
topic: Some(PUSH_TOPIC) for alerts | Some(PUSH_TEST_TOPIC) for the test, body }`, `transport.deliver(..)`. Outcome per
subscription:

| Result | Action (alert summaries; a test message never touches carries or alert retries) |
|---|---|
| `Delivered` | `last_delivery = delivered`, `sender = reachable`, failure flags reset, **carry cleared** for that endpoint |
| `Gone` | `store.remove_endpoint` (audit `push subscription removed`), `last_delivery = gone`, carry dropped (subscription gone) |
| `Rejected` / `Refused` | keep, `last_delivery = rejected`, audit `push delivery failed` once per transition, no retry, **carry = `effective`** |
| `Retry` / `Err(Timeout)` / `Err(Protocol)` | schedule retry `k` (k < 2) of `effective` at `PUSH_RETRY_DELAYS_MS[k]` or `retry_after_s * 1000` if present; after the last → `last_delivery = failed`, audit once, **carry = `effective`** |
| `Err(Unavailable)` | `sender = unavailable` (audit `push sender unavailable` once per transition), then as `Retry` |
| local failure (encrypt, JWT, RNG, clock, payload too large) | `last_delivery = failed`, **carry = `effective`** |

**Carry** (F-10): the dispatcher state holds `carry: Vec<(PushEndpoint, PushSummary)>`, at most one entry per
endpoint and at most `MAX_PUSH_SUBSCRIPTIONS` entries (an entry for an endpoint no longer in the store is dropped at
the next load). The retry queue holds at most one entry per subscription (≤ `MAX_PUSH_SUBSCRIPTIONS`). When a new
alert summary is taken while an alert retry for a subscription is pending, the pending retry is cancelled and its
`effective` summary is folded into that subscription's carry **before** the new summary is merged, so the new
notification (same `Topic`) reports every attempt that was not yet delivered to that phone. Hence, per subscription,
the sum of the counts in the payloads it **accepted** (`Delivered`) plus its current carry equals the number of live
attempts noted since it was added, saturating at `u32::MAX` (test 54). A key replaced on disk (detected by a
different public key on load) clears the `JwtCache`. The dispatcher follows the lock order of §5.3.

### 5.7 Runtime and view

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "snake_case")]
pub enum PushState { Disabled, Active, Unavailable }
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "snake_case")]
pub enum PushUnavailableReason { StoreFailed, RngFailed, StoreMissing }
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "snake_case")]
pub enum LastDelivery { Delivered, Gone, Rejected, Failed }
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "snake_case")]
pub enum SenderState { Unknown, Reachable, Unavailable }

/// JSON of GET /api/push (exact key set).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PushView {
    pub state: PushState,
    pub reason: Option<PushUnavailableReason>,   // Some iff Unavailable
    pub public_key: Option<String>,              // 87-char b64url, None unless Active
    pub subscriptions: u32,                      // 0..=4, 0 unless Active
    /// One entry per stored subscription (≤ MAX_PUSH_SUBSCRIPTIONS), in store order; empty unless Active.
    /// Lets the owner spot a device they do not recognise (F-9). Never the endpoint path or a key.
    pub devices: Vec<PushDeviceView>,
    pub last_delivery: Option<LastDelivery>,     // None until the first attempt
    pub sender: SenderState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)] #[serde(rename_all = "snake_case")]
pub enum PushService { Apple, Google, Mozilla }   // from PushEndpoint::host(): web.push.apple.com / fcm.googleapis.com / updates.push.services.mozilla.com

/// JSON `{"service":"apple","created_unix_s":1759700000}` (exact key set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct PushDeviceView { pub service: PushService, pub created_unix_s: u64 }

pub struct PushSettings { pub store_path: PathBuf, pub subject: String, pub previews: PushPreviews, pub rp_id: String }
pub(crate) struct PushRuntime { /* settings, store, scheduler Mutex, Notify, JwtCache Mutex, state, gates, transport, random, clock */ }
```

`PushRuntime::setup` (in `serve`, after the alerts runtime): `load_or_create` → `Active`, else `Unavailable` with
`StoreFailed` (or `RngFailed` when the key draw failed) and audit `push notifications unavailable`; the live sink
is passed to `AlertsRuntime::setup` only when `Active`. `view()` reads the subscription count and devices with
`store.load()` (a load error turns the view `unavailable`/`store_failed`, an absent file `unavailable`/`store_missing`,
for that response without changing the stored state).

## 6. HTTP surface

### 6.1 Routes

| Route | Method | Funnel-public | Body | Handler |
|---|---|---|---|---|
| `/sw.js` | GET, HEAD (`405` + `Allow: GET, HEAD` otherwise) | yes (asset) | no | `asset(AssetId::ServiceWorker)` |
| `/api/push` | GET, HEAD | no | no | `200` + `PushView` (disabled view when off) |
| `/api/push/subscribe` | POST | no | yes (≤ 2 KiB) | §6.2 |
| `/api/push/unsubscribe` | POST | no | yes (≤ 2 KiB) | §6.3 |
| `/api/push/test` | POST | no | no (`413 body_not_allowed`, existing parser rule) | §6.4 |

Paths matched exactly (`/api/push/`, `/api/Push`, `/api/push/subscribe/`, `/sw.js/`, `/SW.js` → `404`). Constants
`PUSH_PATH`, `PUSH_SUBSCRIBE_PATH`, `PUSH_UNSUBSCRIBE_PATH`, `PUSH_TEST_PATH`, `SERVICE_WORKER_PATH` in `routes.rs`.
`accepts_body` returns true for exactly the four existing body routes plus subscribe and unsubscribe. `/sw.js` is
served with the unchanged mandatory headers (incl. `Cache-Control: no-store`, the unchanged CSP, `nosniff`).

```rust
/// Pure; the lock CSRF rules (`check_action_csrf`) with `action` ∈ {push-subscribe, push-unsubscribe, push-test};
/// any other action value is refused (MissingActionHeader).
pub fn check_push_csrf(head: &RequestHead, normalized_host: &str, action: &str) -> Result<(), CsrfError>;
```

The existing server gates run first (host `421`, classification `403`, Funnel capacity, Funnel host = `rp_id`,
Funnel session: anonymous → `403 login_required` **before any body read**).

### 6.2 `POST /api/push/subscribe` — gate order

1. `check_push_csrf(…, "push-subscribe")` → `403 forbidden`.
2. push disabled → `403 push_disabled`; push `unavailable` → `503 unavailable`.
3. shared route gate (`PUSH_ROUTE_MIN_INTERVAL_MS`) → `429 rate_limited`.
   (Steps 1–3 happen before the body is read.)
4. body read (existing `read_request_body`): malformed framing `400 bad_request`, > 8 KiB `413 body_too_large`,
   timeout/closed → closed without response; then > `MAX_PUSH_SUBSCRIBE_BODY_BYTES` → `413 body_too_large`.
5. JSON `{"endpoint":"…","expirationTime":null|<number>,"keys":{"p256dh":"…","auth":"…"}}` (`deny_unknown_fields`
   on both objects, `expirationTime` optional and ignored, the other keys mandatory) → `400 bad_request`.
6. `PushEndpoint::parse`: `HostNotAllowed` → `400 unsupported_push_service`; other errors → `400 bad_request`.
7. `parse_subscription_keys` → `400 bad_request`.
8. `store.upsert` (created_unix_s from the injected clock): `Added` → audit `push subscription added`, `200
   {"result":"subscribed"}`; `Replaced` → `200 {"result":"subscribed"}`; `TooMany` → `409 too_many_subscriptions`;
   any other store error (incl. `Missing`: the file was deleted while running) → `503 store_unavailable`.

### 6.3 `POST /api/push/unsubscribe`

Steps 1–4 as §6.2 with `push-unsubscribe`; body `{"endpoint":"…"}` (`deny_unknown_fields`) → `400 bad_request`;
endpoint invalid → `400 bad_request`; `remove_endpoint`: removed → audit `push subscription removed`; **both
removed and absent → `200 {"result":"unsubscribed"}`** (no oracle); store error → `503 store_unavailable`.

### 6.4 `POST /api/push/test`

1. `check_push_csrf(…, "push-test")` → `403 forbidden`. 2. disabled `403 push_disabled`; unavailable `503
unavailable`. 3. own gate `PUSH_TEST_MIN_INTERVAL_MS` → `429 rate_limited`. 4. `store.load()`: error or absent
file `503 store_unavailable`; 0 subscriptions → `409 no_subscriptions`. 5. queue one test message (a second queued
one replaces it), wake the dispatcher → `202 {"result":"test_queued"}`. The test is sent with `Topic: soos-test`
(`PUSH_TEST_TOPIC`), never `soos-alerts`, so it can never replace an alert still waiting at the push service for an
offline phone, and it never cancels, delays or absorbs an alert retry or carry. The result is observed through `GET
/api/push` (`last_delivery`). Test messages do not count towards `PUSH_MAX_PER_HOUR` (they are bounded by their own
gate: ≤ 360 per hour, owner-initiated, authenticated).

## 7. Error taxonomy (→ observable result)

| Failure | Result |
|---|---|
| config: push without alerts / without `rp_id` / bad subject / bad socket path / bad previews | exit 78 (`ConfigError`), nothing started |
| store absent at start | created (fresh key) |
| store invalid, insecure, unreadable at start; RNG failure on key creation | `unavailable` (`store_failed` / `rng_failed`), audit once, file untouched, POST routes `503 unavailable`, nothing sent |
| store error during a route | `503 store_unavailable` |
| store deleted while running | `unavailable` / `store_missing` in the view, routes `503 store_unavailable`, nothing recreated (§3.3) |
| store error in the dispatcher | message not sent, `last_delivery = failed`, audit once, alert counts carried (§5.6) |
| encryption / JWT / RNG / clock failure for one message | that subscription skipped for that message, `last_delivery = failed`, counts carried |
| sender socket missing, not ours, connect timeout, peer uid mismatch | `sender = unavailable`, retry policy |
| exchange timeout, malformed reply | retry policy |
| push service 2xx / 404·410 / 429·5xx / 3xx·other 4xx | delivered / subscription removed / retry / rejected (kept) |
| sender: endpoint not allowlisted, non-public address, empty resolution | `refused` (in `FilteringResolver`: `ureq::Error::Other(ResolveRefused)` → `send_error_outcome` → `Refused`, `status: None`, no connection attempted) → rejected (kept, no retry, counts carried) |
| sender: DNS error, connect/TLS error, timeout, any other `ureq::Error` | `retry` (`status: None`) |
| sender: malformed frame, oversized prefix, peer uid mismatch, frame I/O timeout | connection closed (malformed decoded frame: reply `refused`) |
| poisoned scheduler mutex | live attempts not pushed (alerts unaffected) |

No push **error** can change an alert, status, lock or unlock outcome, end `serve`, reach `PAM`, or delete a
subscription other than on `gone`. A **panic** inside the dispatcher task is not an error outcome: like a follower
panic it ends `serve` with `TaskPanicked` (supervised-set rule) and the service restarts under systemd
(`Restart=on-failure`); production push code contains no panicking construct (test 46).

## 8. `soos-push-sender`

### 8.1 Library (`crates/push-sender/src/lib.rs`)

```rust
/// The outbound policy; every field a constant of soos-push-protocol (test 13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SendPolicy { pub https_only: bool, pub max_redirects: u32, pub use_env_proxy: bool,
                        pub timeout_global_ms: u64, pub timeout_connect_ms: u64,
                        pub max_response_header_bytes: usize, pub max_response_body_bytes: usize }
impl Default for SendPolicy { /* true, 0, false, PUSH_SEND_TIMEOUT_MS, PUSH_CONNECT_TIMEOUT_MS, 16 KiB, 1 KiB */ }

/// DNS seam. Production: the system lookup through ureq's `DefaultResolver` (getaddrinfo, bounded by the
/// request's remaining timeout); tests inject a closure.
pub type AddressLookup = Arc<dyn Fn(&str) -> Result<Vec<std::net::SocketAddr>, LookupError> + Send + Sync>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)] pub enum LookupError { Failed, Timeout }

/// Pure filter: host must be a PUSH_HOSTS element (else Refused, lookup not called); lookup error → Retry;
/// empty → Refused; any non-public address → Refused; otherwise every address with port 443, IPv4 first (stable),
/// truncated to MAX_RESOLVED_ADDRESSES. Called exactly once per resolution.
pub fn filter_addresses(host: &str, lookup: &(dyn Fn(&str) -> Result<Vec<std::net::SocketAddr>, LookupError> + Send + Sync))
    -> Result<Vec<std::net::SocketAddr>, Outcome>;

/// Marker error carried inside `ureq::Error::Other` when the resolver refuses a request. Fixed text
/// ("push host address refused"), no field, never the host or an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub struct ResolveRefused;

/// The only ureq `Resolver` of the production agent (`ureq::unversioned::resolver::Resolver`, pinned by `=3.4.2`).
/// `resolve(uri, config, timeout)`: host = `uri.host()` (none → `ResolveRefused`); lookup = the injected
/// `AddressLookup` when present, else a closure calling `DefaultResolver::default().resolve(uri, config, timeout)`
/// (its `Error::Timeout` → `LookupError::Timeout`, any other error → `LookupError::Failed`); then `filter_addresses`:
/// `Ok(addrs)` → the `ResolvedSocketAddrs` array (≤ 16, so it always fits); `Err(Outcome::Refused)` →
/// `Err(ureq::Error::Other(Box::new(ResolveRefused)))`; `Err(Outcome::Retry)` → `Err(ureq::Error::HostNotFound)`.
/// The connector receives only these validated addresses (no second lookup).
#[derive(Clone)] pub struct FilteringResolver { lookup: Option<AddressLookup> }
impl fmt::Debug for FilteringResolver { /* "FilteringResolver" only */ }

/// Pure mapping of a failed `send` to the reply: `ureq::Error::Other(e)` with `e.is::<ResolveRefused>()` →
/// `Outcome::Refused`; every other error → `Outcome::Retry`; `status: None`, `retry_after_s: None` in both cases.
#[must_use] pub fn send_error_outcome(error: &ureq::Error) -> DeliveryReply;

/// One outbound attempt.
pub trait Deliverer: Send + Sync { fn deliver(&self, request: &DeliveryRequest) -> DeliveryReply; }
/// Production: a ureq Agent built **only** with `Agent::with_parts(config_from(policy), DefaultConnector::default(),
/// FilteringResolver { .. })` — never `Agent::new_with_config`, `new_with_defaults` or `Agent::from` (those use
/// ureq's unfiltered resolver) — and rustls (ring, webpki-roots). POST endpoint, headers `TTL`, `Urgency`, `Topic`
/// (when Some), `Content-Encoding: aes128gcm`, `Content-Type: application/octet-stream`, `Authorization`, fixed
/// `User-Agent: soos-push-sender`; reads at most 1 KiB of the response body and discards it; `classify_status` on
/// a response, `send_error_outcome` on an error.
pub struct UreqDeliverer { /* agent */ }
impl UreqDeliverer {
    /// Production: system lookup (`FilteringResolver { lookup: None }`).
    pub fn new(policy: SendPolicy) -> Self;
    /// Test seam (F-3): the same agent construction with `FilteringResolver { lookup: Some(lookup) }`.
    pub fn with_lookup(policy: SendPolicy, lookup: AddressLookup) -> Self;
}

/// Installs the only subscriber of the process (F-1): `tracing_subscriber::registry()` + `fmt::layer()` writing
/// to stderr without ANSI colours + the fixed filter `Targets::new().with_default(LevelFilter::INFO)
/// .with_target("ureq", LevelFilter::OFF).with_target("ureq_proto", LevelFilter::OFF)
/// .with_target("rustls", LevelFilter::OFF)`, installed with `tracing::subscriber::set_global_default`. Never
/// `.init()`/`.try_init()` (they install the `log` → `tracing` bridge `LogTracer` because the workspace
/// `tracing-subscriber` keeps its default `tracing-log` feature), never `LogTracer`, never `EnvFilter`/
/// `from_default_env`/`RUST_LOG`. With no `log` logger installed, every `log` record of `ureq`, `ureq-proto` and
/// `rustls` (including `ureq-proto`'s byte dump of each written request at `trace`) is discarded by the `log` crate
/// itself (its runtime maximum level stays `Off`). Error (a subscriber already set) → ignored, the process runs
/// without logs (fail-quiet, never fail-open).
pub fn install_logging();

/// Serves one accepted connection: SO_PEERCRED uid == own uid (else close), read prefix + frame under
/// PUSH_FRAME_IO_TIMEOUT_MS each (frame_len before allocation), decode (error → reply Refused), re-validate the
/// endpoint (decode does), deliver, write the reply under PUSH_FRAME_IO_TIMEOUT_MS, close.
pub fn serve_connection(stream: std::os::unix::net::UnixStream, own_uid: u32, deliverer: &dyn Deliverer);

/// Socket setup: $XDG_RUNTIME_DIR/soos-push must exist (RuntimeDirectory=) or is created 0700; owner checked
/// through the descriptor; a stale socket owned by the uid is replaced; any other file or a symlink → error;
/// bind; chmod 0600 (unlink on failure).
pub fn bind_socket(dir: &Path, uid: u32) -> Result<std::os::unix::net::UnixListener, SocketError>;
pub fn check_not_root(uid: u32, euid: u32) -> Result<(), SenderError>;
```

`main.rs`: `install_logging()` (stderr → journald, constant messages only, fixed filter, no `log` bridge) → refuse
root (exit 78) → `XDG_RUNTIME_DIR`
(absolute, else exit 78) → `bind_socket` → loop `accept` → `serve_connection` **sequentially** (one connection at
a time; a slow peer is bounded by the frame timeouts, a slow push service by `PUSH_SEND_TIMEOUT_MS`) until SIGTERM
(default disposition; nothing to clean but the socket, which `bind_socket` replaces next start). No other file is
opened by the sender's own code (the C library reads `/etc/resolv.conf`, `/etc/nsswitch.conf`, `/etc/hosts` for
the system lookup); **no environment variable other than `XDG_RUNTIME_DIR` is read** (the logging filter is fixed in
code, F-1); the ureq proxy is `None`.

### 8.2 Unit `packaging/soos-push-sender.service`

```ini
[Unit]
Description=soos push sender (outbound Web Push for the remote companion)
Documentation=https://github.com/Mysticaly622/soos/blob/main/Docs/REMOTE_COMPANION.md

[Service]
Type=exec
ExecStart=%h/.local/bin/soos-push-sender
Restart=on-failure
RestartSec=5
RestartPreventExitStatus=78
RuntimeDirectory=soos-push
RuntimeDirectoryMode=0700
NoNewPrivileges=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
LockPersonality=yes
MemoryDenyWriteExecute=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
SystemCallArchitectures=native
ProtectSystem=strict
ProtectHome=tmpfs
BindReadOnlyPaths=%h/.local/bin/soos-push-sender
TemporaryFileSystem=/run:ro
PrivateTmp=yes
PrivateDevices=yes
PrivateIPC=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectClock=yes
ProtectHostname=yes
RestrictNamespaces=yes
CapabilityBoundingSet=
SystemCallFilter=@system-service
SystemCallErrorNumber=EPERM
UMask=0077

[Install]
WantedBy=default.target
```

What each filesystem line does (systemd 262, `man systemd.exec`, checked on the owner's distribution; every
namespacing directive implies `PrivateUsers=` in a user unit, which needs `kernel.unprivileged_userns_clone=1`, set
on the owner's host):

- `ProtectHome=tmpfs`: read-only empty tmpfs over `/home`, `/root` and `/run/user`; nothing of `$HOME` is visible
  (not `~/.ssh`, browser profiles, `~/.config/soos` whatever `XDG_CONFIG_HOME` or `credentials_path` say).
- `BindReadOnlyPaths=%h/.local/bin/soos-push-sender`: the only file of `$HOME` made visible, read-only, so that
  `ExecStart=` (resolved inside the namespace) finds the binary.
- `TemporaryFileSystem=/run:ro`: read-only empty tmpfs over `/run`; this hides every runtime socket the sender never
  needs: `%t/soos-remote/remote.sock` (which trusts the `Tailscale-User-Login` header of any peer of the owner's
  uid), the session bus `%t/bus`, the system bus `/run/dbus/system_bus_socket`, `/run/tailscale/tailscaled.sock`,
  `/run/soos/daemon.sock`, `/run/docker.sock`, `/run/systemd/*`, and every other socket under `/run` (or `/var/run`,
  a symlink to it).
- `RuntimeDirectory=soos-push`: implies a bind mount of `%t/soos-push` (the sender's own socket directory) into the
  namespace on top of both tmpfs mounts; it is the only writable path besides the private `/tmp`.
- DNS keeps working: `/etc/resolv.conf`, `/etc/nsswitch.conf`, `/etc/hosts` stay readable under
  `ProtectSystem=strict`; `nss-resolve`/`nss-mymachines` find no socket under `/run` and report `UNAVAIL`, so
  `getaddrinfo` falls through to `files` then `dns` (on the owner's host `nameserver 100.100.100.100`, MagicDNS,
  reached over `AF_INET`). On a host whose `/etc/resolv.conf` is a symlink into `/run/systemd/resolve/`, the lookup
  fails (fail-closed: `retry`, then `failed`); the Docs §2d owner steps say so, and widening the unit is out of
  scope (it needs its own ADR).
- `ProtectProc=`, `ProcSubset=` and `ProtectControlGroups=` are **not** used: they are unsupported in per-user
  service managers (plan evaluation F-13; test 45 pins their absence).

The other hardening lines (`PrivateDevices`, `PrivateIPC`, `ProtectKernel*`,
`ProtectClock`, `ProtectHostname`, `RestrictNamespaces`, empty `CapabilityBoundingSet`, `SystemCallFilter=
@system-service` with `EPERM`) cost nothing for a process that only does DNS, TCP, TLS and one Unix listener.
Whether this exact sandbox starts on the owner's host, and DNS plus outbound HTTPS work under it, is the owner check
RMC74; a refused directive makes the unit fail to start (fail-closed: no push, the page shows "Push sender not
running").

## 9. Page (`assets/`)

- `sw.js` (new): `self.addEventListener("push", …)` always `event.waitUntil(self.registration.showNotification(title,
  {body, tag, lang: "en"}))` with `tag` = `"soos-test"` when the payload's `soos.kind` is `"test"`, else
  `"soos-alerts"` (a displayed test never replaces a displayed alert); title/body from `event.data.json().notification` inside `try`, with the
  fallback title `soos` and body `New security alert — open soos` when data is absent or unreadable;
  `notificationclick` → `event.notification.close()` and `event.waitUntil(clients.matchAll({type: "window"})…focus()`
  or `clients.openWindow("/"))`; `navigator.setAppBadge`/`clearAppBadge` are not used. No `fetch` listener, no
  `importScripts`, no `caches`, no storage, no absolute URL.
- `app.js`: when `"serviceWorker" in navigator && "PushManager" in window`, `navigator.serviceWorker.register("/sw.js",
  {scope: "/"})` at load; a "Notifications" card hidden while `GET /api/push` says `disabled`. The VAPID key is
  fetched with the state **before** any tap. Button **Enable notifications** → `async function
  enableNotifications()` whose first awaited call is `Notification.requestPermission()`, then
  `registration.pushManager.subscribe({userVisibleOnly: true, applicationServerKey: <key>})`, then `fetch
  ("/api/push/subscribe", {method: "POST", headers: {"X-Soos-Action": "push-subscribe", "Content-Type":
  "application/json"}, body: JSON.stringify(subscription.toJSON())})`. Permission `denied`/`default` → text
  "Notifications are blocked in the iPhone settings". Buttons **Send test notification** (`push-test`) and
  **Disable notifications** (`subscription.unsubscribe()` then `POST /api/push/unsubscribe` with `{endpoint}`). On
  load with permission `granted` and an existing subscription, the page compares the subscription's
  `options.applicationServerKey` bytes with the server's `public_key`: equal → it re-posts the subscription (keeps the
  server in sync); different (after `push reset` or a store recreated at restart) → it calls `unsubscribe()` locally,
  posts nothing, and shows "Notifications must be re-enabled on this phone" with the **Enable notifications** button
  (a new subscription needs the user gesture). `unavailable` with `store_missing` → "The notification store was
  removed on the PC — run `soos-remote push reset`". **Devices list** (F-9): one line per `devices` entry, "<Apple |
  Google | Mozilla> device, enabled <local date and time of created_unix_s>", with the hint "Not yours? Run
  `soos-remote push list` and `push remove N` on the PC, and, if a passkey is not yours either, `soos-remote passkeys remove N`" (removing a passkey ends
  the Funnel sessions it opened, Docs §2b). Without push support: "Notifications need the home-screen app (iOS 16.4 or later)". States
  shown: `last_delivery` (`rejected` → "The push service refused the last notification — see setup"), `sender:
  unavailable` → "Push sender not running on the PC", results `unsupported_push_service`, `too_many_subscriptions`,
  `no_subscriptions`, `push_disabled`. `textContent` only; no storage; the anonymous Funnel sign-in view never calls
  push routes.
- `index.html`: the card markup (`id="push"`), no inline script.

## 10. Latency

Not on the PAM path. Attempt → live record: immediate for trusted `pam_unix` lines, ≤ 2 s + one idle tick for
helper-only checks (level 1). Then `PUSH_COALESCE_MS` (3 s) + one exchange (typically < 1 s, bounded by `PUSH_EXCHANGE_TIMEOUT_MS` = 18 s).
**Budget: the first notification of a burst reaches the push service within 6 s of the attempt in the normal case**
(owner check RMC74 measures phone arrival, which also depends on Apple).

## 11. Invariants touched

| Invariant | How it holds |
|---|---|
| No passwords anywhere (`AGENTS.md`, O-2) | payload built from enums and counts only; needle tests (31, 33); no new journal field. |
| `soos-remote` opens no network socket (RMC-S2, 2026-10-05 item 2) | it only connects to a Unix socket; unit unchanged (RMC-S5, RMC-S26); `std::net` names stay out of `crates/remote/src` (the sender and protocol crates hold them). |
| Pure-Rust crypto in `soos-remote`, no OpenSSL/`ring` (RMC-S13) | `hmac`, `aes-gcm`, existing `p256` features; TLS only in the sender; `ring` confined to `soos-push-sender`. |
| Leaf crate (RMC-S4) | new manifests never contain `soos-remote`; `soos-remote` depends only on `soos-push-protocol`. |
| One CSPRNG (RMC-S16) | every random byte through `RandomSource`; the sender draws no randomness of its own beyond rustls's internal use of `ring`. |
| Clock read only in `main.rs`/`server.rs` | push code receives the injected `UnixClock`. |
| Authenticated only (O-4) | push API routes not Funnel-public; CSRF on every POST; anonymous Funnel refused before body reads. |
| Bounded (O-4) | §2.1, §3.1; sequential dispatcher and sender; retry queue ≤ 4; JWT cache ≤ 3; frames bounded before allocation. |
| Sender confinement (W-3) | §8.2: `$HOME` hidden except the binary, `/run` hidden except its own runtime directory, no capabilities, no new namespaces, `@system-service` syscalls; static test 45 pins the exact unit. |
| No secret in logs (W-14) | `push.rs`/`webpush.rs` without macros; sender: constant messages, fixed filter, no `log` bridge (tests 46, 47, 53). |
| Fail-closed | invalid store never overwritten; deleted store never silently re-keyed by the service; `unavailable` visible; every unknown status `rejected`; any private address refuses the request. |
| Test immutability (`AGENTS.md`) | W-10, W-15. |
| The crate-level forbid lint | both new crates carry it; added to the business-crate lists; no raw-memory API is needed (`SO_PEERCRED` through `nix`'s safe API). |

## 12. Test list for the tester (Phase 2)

Every test is a contract and must fail before Phase 4 (the items do not exist). Paused tokio time wherever timing
is asserted; no test sends a real network request or needs the sender unit.

### 12.1 `crates/push-protocol/tests/protocol_tests.rs` (new, pure)

1. `test_rwp_endpoint_allowlist_accepts_known_push_services` — real-shaped endpoints for the three hosts (Apple
   token path, FCM `/fcm/send/<token:with-colon>`, Mozilla `/wpush/v2/<b64url>`); `host()` and `origin()` exact.
2. `test_rwp_endpoint_refuses_ssrf_shapes` — every refusal of §2.2 with its exact `EndpointError`: `http://`,
   `HTTPS://`, userinfo, `:443`, `:8443`, IPv4 literal, `[::1]`, `web.push.apple.com.` (trailing dot),
   `web.push.apple.com.evil.example`, `evil.example/web.push.apple.com`, `WEB.PUSH.APPLE.COM`, percent-encoded host,
   `?q`, `#f`, `/a/../b`, `/./`, `//`, `\`, space, control byte, non-ASCII, path `/`, 1025 bytes (1024 accepted);
   `Display` and `Debug` of every error and `Debug` of a `PushEndpoint` never contain the input.
3. `test_rwp_public_address_predicate` — first and last address of every refused range of §2.3 refused; accepted:
   `17.188.143.78`, `216.239.38.55`, `199.232.169.91`, `2a04:4e42:6a::347`, `1.1.1.1`; refused: `100.100.100.100`,
   `fd7a:115c:a1e0::53`, `::ffff:17.188.143.78`, `64:ff9b::1101:1`, `2002:1111:1111::1`, `2001:0:1::1`.
4. `test_rwp_frame_round_trip_and_bounds` — request and reply round trip (exact JSON bytes of a fixed request);
   prefix big-endian; payload of exactly `MAX_PUSH_FRAME_BYTES` decodable size accepted by `frame_len`, +1 →
   `TooLarge`; unknown key, missing key, `v: 2`, padded or standard-alphabet body, body 0 or 4097 bytes,
   authorization 1025 bytes / not starting `vapid t=` / with a control byte, `ttl` 0 and `MAX_PUSH_TTL_S + 1`, topic
   33 chars or with `+`, reply `status` 99/600, `retry_after_s` with `delivered`, `retry_after_s` 0 or 301 → the
   matching `FrameError`.
5. `test_rwp_frame_decoding_never_panics` — proptest, arbitrary bytes ≤ 16 KiB into `decode_request`,
   `decode_reply`, `frame_len`, `PushEndpoint::parse`.
6. `test_rwp_status_classification` — the §2.5 table, edges (199, 200, 299, 300, 399, 403, 404, 409, 410, 413,
   429, 499, 500, 599) and `parse_retry_after` (`120`, `0`, `301`→300, `9999999999`→300, `99999999999` (11 digits),
   `-1`, `+5`, ` 5`, HTTP-date, empty → as specified).

### 12.2 `crates/push-sender/tests/sender_tests.rs` (new; no network)

7. `test_rwp_sender_filter_addresses` — injected `AddressLookup` (passed as `&*lookup`): all public → port 443,
   IPv4 first, order kept inside a family, truncated to 16; one private among public → `Refused`; empty →
   `Refused`; `LookupError` → `Retry`; a non-allowlisted host → `Refused` and the lookup is never called (counter);
   an allowlisted host → the lookup is called exactly once with exactly that host.
8. `test_rwp_sender_serves_one_request_with_fake_deliverer` — `UnixStream::pair()`: a valid frame → the fake
   `Deliverer` receives exactly the decoded request once; the reply frame equals `encode_reply` of the fake's reply.
9. `test_rwp_sender_refuses_bad_frames` — prefix above `MAX_PUSH_FRAME_BYTES` → closed without reading further and
   without calling the deliverer; malformed JSON → reply `refused`; non-allowlisted endpoint → reply `refused`,
   deliverer not called; a peer that sends 2 bytes and stalls → closed within `PUSH_FRAME_IO_TIMEOUT_MS` + 1 s
   (real time).
10. `test_rwp_sender_socket_setup` — temp dir: socket mode `0600`; created dir `0700`; a stale socket of the uid
    replaced; a regular file or a symlink at the socket path → error and untouched.
11. `test_rwp_sender_refuses_root` — `check_not_root(0, 1000)`, `(1000, 0)` → error; `(1000, 1000)` → ok.
12. `test_rwp_sender_policy_defaults` — `SendPolicy::default()` equals the §8.1 constants; `UreqDeliverer::new`
    and `UreqDeliverer::with_lookup` do not panic; `send_error_outcome(&ureq::Error::Other(Box::new(ResolveRefused)))`
    → `Refused`, `send_error_outcome(&ureq::Error::HostNotFound)` and of `ureq::Error::Timeout(..)` → `Retry`, all
    with `status: None` and `retry_after_s: None`; `Display`/`Debug` of `ResolveRefused` is fixed text.
52. `test_rwp_sender_deliverer_uses_filtering_resolver` (F-3; network-free) — `UreqDeliverer::with_lookup(
    SendPolicy::default(), lookup)` where `lookup` records each host it is asked for and returns a scripted answer;
    `deliver(&valid_request)` for an endpoint on `web.push.apple.com`: (a) lookup → `[127.0.0.1:443]` → exactly
    `DeliveryReply { outcome: Refused, status: None, retry_after_s: None }`, the lookup was called exactly once with
    `"web.push.apple.com"`, and the call returns within 2 s (real time; no connection is attempted to a refused
    address); (b) lookup → `[17.188.143.78:443, 10.0.0.1:443]` → `Refused`, called once; (c) lookup →
    `Err(LookupError::Failed)` → `Retry` with `status: None`, called once. A deliverer that ignored the injected
    lookup (ureq's own resolver) or mapped the refusal to `Retry` fails this test.

### 12.3 `crates/remote/tests/webpush_tests.rs` (new, pure)

13. `test_rwp_rfc8291_known_answer` — RFC 8291 Appendix A: `derive_keys` gives CEK `oIhVW04MRdy2XN9CiKLxTg` and
    NONCE `4h_95klXJ5E_qnoN`; `encrypt_with` gives the exact 144-byte body of research §3.1 (86 + 41 + 1 + 16; plan evaluation F-11).
14. `test_rwp_encrypt_inputs_and_bounds` — plaintext of `MAX_PUSH_PLAINTEXT_BYTES` accepted, +1 →
    `PlaintextTooLarge`; `parse_subscription_keys`: compressed point, off-curve point, 64/66 bytes, auth 15/17,
    padding `=`, `+`/`/` → `InvalidSubscriptionKeys`; failing `RandomSource` → `Random`, no output; two `encrypt`
    calls with different scripted randomness differ; a scripted all-zero then valid draw is retried.
15. `test_rwp_vapid_authorization_shape_and_signature` — fixed key and `now_s`: header and claims JSON exact, `aud`
    is the origin (no path, no slash), `exp = now_s + 43200`, `sub` exact; the signature (64 bytes) verifies with
    the `VerifyingKey`; value is `vapid t=<jwt>, k=<87 chars>`; `k` decodes to 65 bytes starting `0x04`;
    `now_s = 1_699_999_999` → `Clock`.
16. `test_rwp_jwt_cache_reuse_and_bounds` — same origin within 3599 s → identical token; at 3600 s → new token;
    three origins → three entries; `clear()` → new token.
17. `test_rwp_vapid_key_generation_and_redaction` — scripted randomness: zero scalar then valid → ok; four invalid
    draws → `Random`; `from_bytes(to_bytes())` round trip; `Debug` of `VapidKey` is `VapidKey(<redacted>)`.

### 12.4 `crates/remote/tests/push_tests.rs` (new, pure)

18. `test_rwp_scheduler_coalesces_and_rate_limits` — first note at `t` → `due_at = t + 3000`; five notes within 3 s
    → one summary with `wrong_password = 5`; next note right after a send → due 30 s after the send; the 21st send
    inside one hour waits until the oldest send leaves the window; counts saturate at `u32::MAX`; `newest_*`
    follow the greatest `at_us`; `PushSummary::merge`: counts add (saturating at `u32::MAX`), `first_unix_ms` min,
    `last_unix_ms` max, `newest_*` from the operand with the greater `last_unix_ms`, ties → `newer`.
19. `test_rwp_is_live_rule` — `at < started` → false; `at = started` → true; age exactly 300 s → true, +1 µs →
    false; `at > now` → true.
20. `test_rwp_alert_payload_exact` — exact JSON bytes for: detailed lock-screen/owner 1 and 3 attempts; detailed
    sudo/root wrong 2 + locked-out 1; detailed locked-out only; generic; test payload; `navigate` =
    `https://<rp_id>/`; `u32::MAX` counts stay ≤ 1024 bytes; the JSON parses as Declarative Web Push
    (`web_push == 8030`, non-empty title).
21. `test_rwp_store_round_trip_and_fail_closed` — `load_or_create` on absent → file `0600`, version 1, 0
    subscriptions, valid key; reload equal; refused and **never rewritten** (bytes and mtime unchanged): symlink,
    mode `0644`, foreign owner (via `with_file_owner_uid`-style uid parameter), 16 385 bytes, invalid JSON, unknown
    key, `version: 2`, 5 subscriptions, duplicate endpoint, invalid endpoint or keys inside, invalid private key;
    failing random source on an absent file → `Random` and no file. Recreation semantics (F-7): after
    `load_or_create`, delete the file → `load()` = `Ok(None)`; `upsert`, `remove_endpoint`, `remove_index` →
    `Missing` and **no file is created**; `reset` → file `0600` with a **different** valid key and 0 subscriptions;
    `reset` over an invalid (`0644`, bad JSON) file owned by the uid → replaced; `reset` with a symlink or a foreign
    owner → `Refused`, untouched; `reset` with a failing random source → `Random`, old bytes unchanged.
22. `test_rwp_store_subscription_operations` — `Added`, `Replaced` (same endpoint, new keys, `created_unix_s`
    kept), `TooMany` at the 5th distinct endpoint (store unchanged), `remove_endpoint` present/absent,
    `remove_index` 1-based, out of range false.
23. `test_rwp_subscribe_body_parsing` — `toJSON` shapes with `expirationTime: null` and a number accepted; unknown
    top-level or `keys` member, missing `keys`/`auth`, non-string endpoint, 2049-byte body → the specified error.
24. `test_rwp_cli_lines_never_print_secrets` — `push::subscription_line(index, &sub)` is exactly
    `"<index>  <host>  created <created_unix_s>"`; never contains the endpoint path, `p256dh` or `auth`.

### 12.5 `crates/remote/tests/push_server_tests.rs` (new; harness + tester-owned `FakeTransport` + `ScriptedJournal`)

The tester adds `crates/remote/tests/common/push.rs`: `FakeTransport` (records every `DeliveryRequest`, scripted
replies or errors per call, optional hold), `ua_keys()` (a fixed subscription key pair held by the test), and
`decrypt_aes128gcm(body, ua_private, auth)` (RFC 8291 receiver side, test-only).

25. `test_rwp_push_disabled_by_default` — default config: `GET /api/push` = disabled view (exact keys `state`,
    `reason`, `public_key`, `subscriptions`, `devices`, `last_delivery`, `sender`; `devices: []`), every push
    POST `403 push_disabled` without reading a body, no store file created, transport never called, alerts work.
26. `test_rwp_push_routes_require_authentication` — tailnet allowed identity `200`; foreign identity `403`;
    anonymous Funnel → `403 login_required` for all four push API routes, a subscribe body on anonymous Funnel is
    never read (the connection answers before the body is sent); Funnel session `200`; `/sw.js` on anonymous Funnel
    `200`, `text/javascript; charset=utf-8`, mandatory headers with the unchanged CSP (`harness::CSP`).
27. `test_rwp_subscribe_gates_and_validation` — wrong/missing action, cross-site, bad `Origin` → `403` before the
    body; second call within 1 s → `429`; 2049-byte body → `413`; `unsupported_push_service`; bad keys `400`;
    5th endpoint `409 too_many_subscriptions`; same endpoint → `200` replaced; store file `0600`; exactly one
    `push subscription added` audit line per added subscription, none per replacement.
28. `test_rwp_live_attempt_sends_one_coalesced_notification` — scripted journal: replayed attempts (journal time
    before the start) → no delivery ever; three live lock-screen failures within 1 s → after 3 s exactly one
    `DeliveryRequest` per subscription: endpoint, `ttl_s = 43200`, `urgency = high`, `topic = soos-alerts`;
    `Authorization` JWT verifies with the store's public key, `aud` = endpoint origin, `sub` = `https://<rp_id>`;
    `decrypt_aes128gcm` yields the exact detailed payload with `wrong_password = 3`; with `push_previews =
    "generic"` the generic payload.
29. `test_rwp_delivery_outcomes` — `gone` (404 and 410 cases) removes the subscription from the file and audits the
    removal; `rejected` keeps it, `last_delivery = rejected`, no retry; `retry` → retried at +5 s and +30 s then
    `failed` (3 calls total); `Retry-After 120` honoured, `retry_after_s` capped by the protocol; transport
    `Unavailable` → `sender = unavailable` and one `push sender unavailable` audit line for repeated failures; a new
    summary replaces a pending retry; `FakeTransport` replying `Refused` → `last_delivery = rejected`, no retry (one
    call), subscription kept.
30. `test_rwp_notification_rate_is_bounded` — 200 live attempts over 2 h of paused time: per subscription ≤ 20
    alert deliveries in any rolling hour, ≥ 30 s apart, and the decrypted counts sum to 200.
31. `test_rwp_test_notification` — no subscription `409 no_subscriptions`; with two, `202 test_queued` then exactly
    one delivery each with the test payload and `topic == Some("soos-test")` (alert deliveries in tests 28–30 carry
    `Some("soos-alerts")`); a test queued while an alert retry is pending neither cancels the retry nor changes the
    alert counts later delivered; again within 10 s `429`; with a body `413 body_not_allowed`.
32. `test_rwp_unsubscribe` — removes and audits; absent endpoint also `200 unsubscribed` without an audit line;
    invalid endpoint `400`.
33. `test_rwp_push_never_exposes_secrets` — needles: endpoint path token, `p256dh`, `auth`, the stored private key
    (b64url), every JWT and every ciphertext seen by `FakeTransport`, owner login, a password-looking journal user
    name: none appears in any response body (the VAPID **public** key appears only in `GET /api/push`), any SSE
    byte or any captured log line.
34. `test_rwp_push_failure_never_affects_other_features` — transport holding forever and failing alternately:
    `/api/status`, `/api/lock`, `/api/alerts`, status and alerts SSE events unchanged; the follower keeps recording;
    `serve` does not return.
35. `test_rwp_store_failure_is_unavailable` — insecure store at start → `GET /api/push` `unavailable`/`store_failed`
    with `public_key: null`; subscribe `503 unavailable` before the body; file untouched; failing random source with
    no file → `rng_failed`; exactly one `push notifications unavailable` audit line. Store deleted while running
    (F-7): `GET /api/push` → `unavailable`/`store_missing`, `public_key: null`, `devices: []`; subscribe, unsubscribe
    and test → `503 store_unavailable`; **no file is recreated** by the service; after `PushStore::reset` (as the CLI
    does) `GET /api/push` is `active` with the new public key. With two stored subscriptions, `devices` lists
    `{"service":"apple","created_unix_s":…}` entries in store order and never an endpoint path or key.
36. `test_rwp_unix_transport_round_trip` — `UnixPushTransport` against an in-test listener speaking the frame:
    exact request bytes; reply decoded; listener never answering → `Timeout` at `PUSH_EXCHANGE_TIMEOUT_MS` (18 s,
    paused time) and not before; a listener answering after 17 s (inside the bound) → the reply is used, no
    `Timeout`; missing socket → `Unavailable`; a symlink or regular file at the path → `Unavailable` without
    connecting; malformed reply → `Protocol`.
54. `test_rwp_undelivered_counts_carry_forward` (F-10) — one subscription, `FakeTransport` scripted: the first
    summary (3 live attempts) gets `Retry` on all three calls (retries exhausted → `failed`); 4 more attempts arrive
    later and that summary is `Delivered` → its decrypted payload has `wrong_password = 7`. Second case: a first
    summary of 3 attempts gets `Retry` once, then 2 more attempts produce a new summary before the +5 s retry is due → the pending
    retry is cancelled and the next delivered payload has the sum (5). Third case: a summary of 2 attempts
    gets `Rejected`, a later summary of 1 attempt is `Delivered` → its payload has `wrong_password = 3`. In every case, over the whole run, the sum of the counts in `Delivered`
    payloads equals the number of live attempts, and the number of alert deliveries respects test 30's bounds.

### 12.6 `crates/remote/tests/config_tests.rs` (new tests only)

37. `test_rwp_push_config_keys` — absent → `PushConfig::default()`; `push_notifications = true` without
    `password_alerts` → `PushRequiresAlerts`, without `rp_id` → `PushRequiresRpId`; subject accepted
    (`mailto:owner@proton.me`, `https://pc.tail1234.ts.net`, `https://github.com/Mysticaly622/soos`) and refused
    (`""`, `mailto:`, `mailto:a@localhost`, `mailto:a@x.invalid`, `mailto:mailto:a@b.co`, `https://x.test`,
    `https://h.local`, `http://pc.tail1234.ts.net`, `https://pc.tail1234.ts.net:8443`, 257 bytes, space, control
    byte) → `InvalidVapidSubject` whose message never contains the value; `push_socket_path` rules;
    `push_previews` `detailed`/`generic`/other.
38. `test_rwp_push_store_path_resolution` — `resolve_push_store_path` sibling `remote-push.json`; no parent →
    error.

### 12.7 `crates/remote/tests/routes_tests.rs` (new tests only)

39. `test_rwp_push_route_table` — §6.1 table: methods, `405` + `Allow`, near paths `404`, `/sw.js` asset;
    `accepts_body` true for subscribe/unsubscribe (with and without a query), false for `/api/push`,
    `/api/push/test`, `/sw.js`; the four existing body routes still true; `is_funnel_public` false for the four push
    API routes, true for `Asset(ServiceWorker)`.
40. `test_rwp_push_csrf_rules` — `check_push_csrf` mirrors the lock table for each of the three actions; an action
    value of another route (e.g. `lock`) or an unknown action argument → `MissingActionHeader`.
41. `test_rwp_service_worker_asset` — `asset(AssetId::ServiceWorker)`: content type `text/javascript;
    charset=utf-8`, body = `include_bytes!` of `assets/sw.js` (compared to the file read from disk).

### 12.8 Superseded / setup-only changes in existing tests (W-15)

Only `crates/remote/tests/common/harness.rs`, `crates/remote/tests/server_tests.rs` and
`crates/remote/tests/alerts_server_tests.rs` gain `push: PushConfig::default(),` in their `RemoteConfig` literals
(plus the `use` of `PushConfig`). No assertion of any existing test changes; the candid reviewer checks that `git
diff` removes or changes no other line of an existing test file.

### 12.9 `tests/invariants/src/remote_push_contract.rs` (new) + `mod remote_push_contract;`

42. `test_rmc_s32_push_crates_are_registered_and_isolated` — both crates are workspace members inheriting
    `version/edition/license/publish` and `[lints] workspace = true`; neither manifest contains `soos-remote`;
    `soos-push-protocol` has no `tokio`, `ureq`, `rustls`, `nix` dependency; `soos-remote` depends on
    `soos-push-protocol`, `hmac`, `aes-gcm` from the workspace and **not** on `ureq`/`rustls`/`soos-push-sender`;
    `test_business_crates_forbid_unsafe_code` and `scripts/candid_review.sh` list `push-protocol` and
    `push-sender`; both crates' `lib.rs` (and the sender's `main.rs`) start with the forbid attribute.
43. `test_rmc_s33_sender_tls_stack_without_openssl` — workspace `ureq` line exactly `{ version = "=3.4.2",
    default-features = false, features = ["rustls"] }`; when `cargo` is available, `cargo tree --offline --locked
    -p soos-push-sender -e normal` contains `rustls`, `ring`, `webpki-roots` and none of `openssl`, `openssl-sys`,
    `native-tls`, `aws-lc-rs`, `aws-lc-sys`, `rand`; `cargo tree -p soos-remote -e normal` contains none of `ureq`,
    `rustls`, `ring`; the workspace `p256` line is unchanged.
44. `test_rmc_s34_sender_policy_literals` — sender production code contains `.https_only(true)`,
    `.max_redirects(0)`, `.proxy(None)`, `timeout_global`, `http_status_as_error(false)`, `FilteringResolver`,
    `impl Resolver for FilteringResolver` (or the fully qualified trait path), `Agent::with_parts(`, `ResolveRefused`;
    contains none of `Agent::new_with_config`, `Agent::new_with_defaults`, `Agent::from(`, `ureq::post(`,
    `ureq::get(`, `danger`, `Dangerous`, `ServerCertVerifier`, `NoVerifier`, `native_certs`, `Command::new`,
    `std::process::Command`; every `env::var(`/`env::var_os(` argument is the literal `"XDG_RUNTIME_DIR"`.
45. `test_rmc_s35_sender_unit_is_sandboxed` — the non-empty, non-comment lines of
    `packaging/soos-push-sender.service` are **exactly** the line set of §8.2 (no line missing, none added), so in
    particular `ProtectHome=tmpfs`, `BindReadOnlyPaths=%h/.local/bin/soos-push-sender`, `TemporaryFileSystem=/run:ro`,
    `PrivateDevices=yes`, `CapabilityBoundingSet=`, `SystemCallFilter=@system-service` are present and none of
    `ProtectHome=read-only`, `ProtectHome=no`, `BindPaths=`, `ReadWritePaths=`, `ReadOnlyPaths=`,
    `SupplementaryGroups=`, `User=`, `Group=`, `DynamicUser=`, `AmbientCapabilities=`, `PrivateNetwork=`,
    `ProtectProc=` appears; `packaging/soos-remote.service` still has `RestrictAddressFamilies=AF_UNIX` as its only
    family line.
46. `test_rmc_s36_push_production_code_hygiene` — in both new crates' production code: no `.unwrap()`, `.expect(`,
    `panic!(`, `unreachable!(`, `todo!(`, `unimplemented!(`, `print!`/`println!`/`eprint!`/`eprintln!`/`dbg!(`, no
    `allow(clippy::unwrap_used…)`-style allowances; every `tracing` macro invocation in `crates/push-sender/src`
    has exactly one string-literal argument without `{`; `SystemTime::now` absent from
    `crates/remote/src/push.rs` and `webpush.rs` (the sender's logging set-up is test 53).
47. `test_rmc_s37_push_modules_never_log_and_types_are_redacted` — no `TRACING_MACROS` invocation in `push.rs` and
    `webpush.rs`; `audit.rs` declares the five fixed messages of W-14; `VapidKey` and `PushEndpoint` have no derived
    `Debug` and a manual `<redacted>` one; `DeliveryRequest`, `UaKeys`, `PushSubscription`, `PushStoreFile` have no
    `Debug` at all.
48. `test_rmc_s38_page_and_service_worker` — `assets/sw.js` exists and contains `addEventListener("push"`,
    `waitUntil(`, `showNotification(`, `notificationclick`, `openWindow(`, and none of `addEventListener("fetch"`,
    `importScripts`, `caches`, `localStorage`, `indexedDB`, `http://`, `https://`, `eval(`, `innerHTML`; `app.js`
    contains `/sw.js`, `serviceWorker.register(`, `Notification.requestPermission(`, `pushManager.subscribe(`,
    `userVisibleOnly: true`, `/api/push/subscribe`, `/api/push/unsubscribe`, `/api/push/test`, `push-subscribe`,
    `push-unsubscribe`, `push-test`, `Enable notifications`, `Send test notification`, `Disable notifications`,
    `home-screen app`, and in the body of `async function enableNotifications(` the first `await` is followed by
    `Notification.requestPermission(`; `index.html` has `id="push"`.
49. `test_rmc_s39_push_is_documented` — `Docs/REMOTE_COMPANION.md` §2d with the needles `push_notifications`,
    `soos-push-sender.service`, `systemctl --user enable --now soos-push-sender`, `home-screen`, `16.4`, `Show
    Previews`, `push_previews`, `vapid_subject`, `BadJwtToken`, `never the typed password`, `false notifications`,
    `web.push.apple.com`, `soos-remote push list`, `soos-remote push remove`, `soos-remote push reset`,
    `abstract`, `RUST_LOG`; the §9 list no longer names push notifications as out of scope but still names `live
    camera`; `AI/DECISIONS.md` has the ADR title.
50. `test_rmc_s40_installer_installs_the_sender_without_enabling_it` — `scripts/install_remote.sh` contains
    `-p soos-push-sender`, `packaging/soos-push-sender.service`, `systemd/user/soos-push-sender.service`,
    `.local/bin/soos-push-sender` (or `${BIN_DIR}/soos-push-sender`), `# push_notifications = false`; the uninstall
    path removes the sender binary and unit; `systemctl --user enable` and `tailscale` still appear only in
    `echo`/`printf` lines (existing RMC-S6 rule also enforces it).
51. `test_rmc_s41_push_keys_never_reach_the_sender` — `crates/push-sender/src` names none of `VapidKey`,
    `SigningKey`, `SecretKey`, `p256`, `aes_gcm`, `hmac`, `remote-push.json`, `remote-passkeys.json`, `remote.toml`;
    its manifest has no `p256`, `aes-gcm`, `hmac` dependency.
53. `test_rmc_s42_sender_logging_is_fixed_and_bridgeless` (F-1) — in `crates/push-sender/src` (production code):
    contains `set_global_default`, `Targets::new()`, `LevelFilter::INFO`, and `.with_target("ureq", LevelFilter::OFF)`,
    `.with_target("ureq_proto", LevelFilter::OFF)`, `.with_target("rustls", LevelFilter::OFF)`; contains none of
    `.init()`, `try_init`, `LogTracer`, `tracing_log`, `EnvFilter`, `from_default_env`, `RUST_LOG`, `log::set_logger`,
    `set_boxed_logger`, `log::set_max_level`; `main.rs` calls `install_logging()` before any other statement that can
    log; `crates/push-sender/Cargo.toml` declares no `log` and no `tracing-log` dependency, and declares
    `tracing-subscriber` exactly as `{ workspace = true }` (no added feature).

## 13. New matrix rows (after RMC59)

| ID | Criterion | Evidence |
|---|---|---|
| RMC60 | Opt-in: `push_notifications` defaults to `false`; requires `password_alerts` and `rp_id`; off ⇒ no file, no task, disabled view, `403 push_disabled`, nothing sent | tests 25, 37 |
| RMC61 | Endpoint allowlist (exact three hosts, https, no userinfo/port/query/fragment, path alphabet, ≤ 1 KiB) enforced at subscribe and at send | tests 1, 2, 9, 27 |
| RMC62 | SSRF: every resolved address must be public (tailnet, CGNAT, RFC 1918, ULA, mapped, NAT64, 6to4, Teredo… refused), connection only to validated addresses through the production `FilteringResolver` (refusal → `refused`, never retried), no redirects, no proxy, https only, 10 s bound | tests 3, 7, 12, 44, 52 |
| RMC63 | RFC 8291 `aes128gcm` encryption reproduces the RFC Appendix A vector; key inputs strictly validated; randomness only from the crate CSPRNG | tests 13, 14, 17 |
| RMC64 | VAPID ES256 JWT: exact header/claims, `aud` origin, `exp` 12 h, validated `sub`, raw signature verifies, token reused ≤ 1 h per origin | tests 15, 16, 37 |
| RMC65 | Store: `0600`, owner-checked, `O_NOFOLLOW`, ≤ 16 KiB, ≤ 4 subscriptions, locked atomic writes, invalid file never overwritten (`unavailable`); created only at service start or by `push reset`, a file deleted while running is reported `store_missing` and never silently re-keyed | tests 21, 22, 35 |
| RMC66 | Routes authenticated (tailnet or Funnel session, anonymous Funnel `403` before body), lock-style CSRF, rate gates, body bounds, no oracle on unsubscribe | tests 26, 27, 31, 32, 39, 40 |
| RMC67 | Trigger: only live attempts (never the 24 h replay), coalesced 3 s, ≥ 30 s spacing, ≤ 20 per hour, counts preserved (undelivered counts carried into the next notification of the same subscription) | tests 18, 19, 28, 30, 54 |
| RMC68 | Delivery outcomes: 2xx delivered; 404/410 subscription removed; 429/5xx/transport retried twice (Retry-After capped 300 s); 3xx/other 4xx/refused kept, not retried; sequential, bounded (whole exchange 18 s > sender worst case 17 s); the test notification uses its own `Topic: soos-test` | tests 6, 29, 31, 36 |
| RMC69 | O-2: payload from enums and counts only (detailed or generic), ≤ 1 KiB, Declarative Web Push shape; no endpoint, key, JWT, ciphertext, login or journal text in any response, event or log; the sender's dependency logs never reach the journal (no `log` bridge, fixed filter, no `RUST_LOG`) | tests 20, 24, 33, 47, 53 |
| RMC70 | Push failures never affect alerts, status, lock, unlock or `serve`; fixed-text audit events | tests 34, 35, 47 |
| RMC71 | Process split: `soos-remote` unit and `AF_UNIX` unchanged, keys and crypto never in the sender, sender sandboxed unit (`$HOME` hidden except its binary, `/run` hidden except its runtime directory, so no access to `remote.sock`, the session/system bus or the stores; no capabilities; `@system-service`), peer uid checked on both ends, frames bounded before allocation, root refused | tests 8–11, 36, 42, 45, 51 |
| RMC72 | Dependencies: `ureq =3.4.2` + rustls/ring/webpki-roots in the sender only; no OpenSSL/native-tls/aws-lc anywhere in either tree; `p256` line unchanged; `cargo deny check` clean | tests 42, 43; `cargo deny check` |
| RMC73 | Page and worker: service worker always shows a notification, user-gesture permission/subscribe, test and disable buttons, unchanged CSP, documentation §2d, installer | tests 26, 41, 48, 49, 50 |
| RMC74 | Hardware (owner): from the iPhone home-screen app (iOS ≥ 16.4) *Enable notifications* subscribes (endpoint host `web.push.apple.com`); *Send test notification* arrives (VAPID `sub` accepted, no `BadJwtToken`); with the app closed and the phone locked, a wrong password at the lock screen and with `sudo` produces one notification each within about 10 s; the sender unit starts under its exact §8.2 sandbox, DNS (MagicDNS / `nsswitch`) and outbound HTTPS work under it, and from inside it (`systemd-run --user -p` with the same properties, owner only) `ls ~`, `ls $XDG_RUNTIME_DIR` and `ls /run/dbus` show nothing but the bound paths; the page lists the phone under *devices* | Manual check (`Docs/REMOTE_COMPANION.md` §2d); agents never install, enable or start units, nor send real pushes |

## 14. ADR (summary; full text in `AI/DECISIONS.md`)

"[2026-10-06] Web Push Notifications for Failed-Password Alerts Through a Separate Sender Unit": W-1 to W-15, the
relayed-request/O-2 point of §0.1, the residual risks of §15 and the setup-only test amendment. Supersedes the "push
notifications" exclusion of the 2026-10-05 ADR item (5) and of the level-1 alerts ADR item (1).

## 15. Residual risks (ADR and `Docs/REMOTE_COMPANION.md` §8)

1. **Lock-screen exposure**: a detailed notification (source, account class, count) is readable on the locked
   iPhone unless "Show Previews: When Unlocked" is set or `push_previews = "generic"`.
2. **Third parties**: Apple (or Google/Mozilla for other browsers) sees delivery metadata (time, size, the PC's
   public IP, the VAPID public key and `sub`), never the content (end-to-end encrypted, RFC 8291). Push delivery is
   best effort; the in-app view stays the source of truth.
3. **False notifications (O-3) and what a compromised sender still reaches**: a process running as the owner can
   forge lock-screen-looking journal lines (level-1 accepted risk) and thereby trigger notifications, bounded to
   ≤ 20 per hour plus coalescing; it can also connect to the sender socket and make it POST to the three
   allowlisted hosts (it already has the owner's network access). The sender sandbox (§8.2) isolates the TLS/HTTP
   stack against bugs and parser exploits, not against a malicious owner-uid process. A **compromised sender** can
   still: open outbound connections to any address (by design; no IP filter exists for user units), read the
   world-readable system files under `/usr` and `/etc` (no secret of the owner lives there), connect to
   **abstract-namespace** Unix sockets of the host network namespace (they are not files, so no filesystem
   directive hides them, and `PrivateNetwork=` would remove the network the sender exists for), and answer
   `soos-remote` with forged replies (bounded frames; at worst a wrong `last_delivery` or a deleted subscription on a
   forged `gone`, which the owner re-enables). It can **not** read anything under `$HOME` (incl. `~/.ssh`,
   `~/.config/soos` wherever `XDG_CONFIG_HOME` points, under the home), nor reach `remote.sock`, the session or system
   bus, `tailscaled`'s socket or `soos-daemon`'s socket (all hidden under `/run`), nor hold any key: the VAPID key,
   subscription keys and plaintext never leave `soos-remote`.
4. **Key loss**: deleting `remote-push.json` or `push reset` replaces the VAPID key; every subscription stops
   working (`gone`/`rejected`) until notifications are re-enabled on the phone.
5. **Unaudited crates**: `ring` (C/assembly) and `rustls` run only in the sender; RustCrypto `p256`/`aes-gcm`/
   `hmac` (not independently audited) in `soos-remote`. `webpki-roots` is a compiled-in snapshot (updated with the
   crate).
6. **DNS**: resolution goes through Tailscale MagicDNS; a tailnet answer for an allowlisted name would be private
   and is refused (the push fails rather than reaching the tailnet).
7. **Sandbox on hardware**: the exact sender sandbox (`ProtectSystem=strict`, `ProtectHome=tmpfs` with the binary
   bound read-only, `TemporaryFileSystem=/run:ro`, implied `PrivateUsers=`, kernel/namespace/capability/syscall
   restrictions) is verified only by the owner (RMC74); if a directive is refused by the host's systemd, the unit
   fails to start (fail-closed: no push, the page shows "Push sender not running"). On a host whose resolver needs a
   socket under `/run` (for example `/etc/resolv.conf` linked into `/run/systemd/resolve/`), DNS fails and every push
   ends `failed`; widening the unit needs its own ADR.
8. **iOS behaviour**: Safari revokes the permission if a push ever arrives without a visible notification; the
   worker always shows one and the payload is declarative (iOS 18.4+ shows it even if the worker fails).
9. **Owner's own typos** are notified like any other failed attempt.
10. **Stolen or shared Funnel session (F-9)**: any authenticated caller (tailnet identity, or a Funnel passkey
    session, including a stolen session cookie) can register its own push subscription on an allowlisted service
    and then receive every alert (counts and classes only, never a password), and can occupy the 4 never-evicted
    slots so the owner's phone gets `409 too_many_subscriptions`. Mitigations: the page lists every registered
    device with its service and creation time (`devices` in `GET /api/push`), `soos-remote push list` prints
    number, host and creation time, `soos-remote push remove N` removes one, `soos-remote passkeys remove N` ends
    the Funnel sessions of a passkey; the `push subscription added` audit line marks each registration in the
    journal. Docs §2d tells the owner to check the device list after enabling and whenever a device is unknown.
11. **Dependency logging (F-1)**: `ureq`, `ureq-proto` and `rustls` contain `log` statements that would print the
    endpoint, the `Authorization` value and request bytes at `debug`/`trace`. They are inert because the sender
    installs no `log` logger and its filter is fixed in code; a future change that adds a `log` bridge or reads
    `RUST_LOG` would re-open the leak, which test 53 prevents.
12. **Undelivered counts**: counts are carried per subscription until delivered (§5.6), but a phone that never
    accepts a delivery again (for example a subscription the push service keeps rejecting) shows nothing; the in-app
    view stays authoritative and `last_delivery = rejected` is shown on the page.

## 16. Documentation drift

- `Docs/REMOTE_COMPANION.md` §9 lists push notifications as out of scope → replaced by §2d (the RMC-S7 needle
  `push notification` stays satisfied by §2d).
- ADR 2026-10-05 item (5) and the level-1 ADR item (1) → supersede notes (added with the new ADR).
- `Docs/REMOTE_COMPANION.md` §1 "never opens a network socket" → precise it: `soos-remote` never does; the optional
  `soos-push-sender` makes outbound HTTPS requests to three push hosts only.
- `AI/ARCHITECTURE.md` §13 (remote companion): the transport row ("no network socket at all") and invariant RC-1
  stay true for `soos-remote`; add one paragraph and a table row for `soos-push-sender` (the only network-capable
  component of the companion, outbound HTTPS to three hosts, separate sandboxed user unit) in the traceability
  phase. The 2026-09-12 IPC ADR ("zero network sockets") concerns the PAM/daemon boundary and is unaffected; the
  new ADR says so explicitly.

## 17. Exit criteria

Every W-decision maps to a type, constant or behaviour above and to at least one test; every new field and
collection has a stated bound (§2.1, §3.1, §5; `devices` ≤ 4, `carry` ≤ 4); every finding of the round-1 plan
evaluation is resolved (§18); the owner confirmation of §0.1 is recorded before Phase 2.

## 18. Round 2 — resolution of the plan evaluation (`AI/plan_evaluator_report.md`, round 1)

| Finding | Resolution | Sections | Tests |
|---|---|---|---|
| §0 precondition (relayed request vs O-2) | Gate restated: the owner must confirm in the owner's own conversation that notifications carry only time, source class, account class, kind and count, never the typed password; no script output or agent message counts; level 2 stops if declined | §0.1, ADR | — (Phase 2 gate) |
| F-1 MAJOR dependency logs | `install_logging`: registry + fmt layer + fixed `Targets` filter (`info`, `ureq`/`ureq_proto`/`rustls` off), `set_global_default`, never `.init()`/`try_init()`/`LogTracer`/`EnvFilter`/`RUST_LOG`; no `log`/`tracing-log` dependency; "(and the logging filter)" removed from §8.1; residual note | W-14, §1.1, §8.1, §11, §15.11 | 53 (new), 44, 46 |
| F-2 MAJOR sender sandbox | `ProtectHome=tmpfs` + `BindReadOnlyPaths=` of the binary only, `TemporaryFileSystem=/run:ro` (hides `remote.sock`, both buses, `tailscaled`, `soos-daemon`, docker), runtime directory bound by `RuntimeDirectory=`; `PrivateDevices`, `PrivateIPC`, `ProtectKernel*`, `ProtectControlGroups`, `ProtectClock`, `ProtectHostname`, `RestrictNamespaces`, empty `CapabilityBoundingSet`, `SystemCallFilter=@system-service` + `EPERM`; `ProtectProc=` omitted (unsupported in user managers, verified in the systemd 262 man page); precise residual reach (abstract sockets, network) | W-3, §8.2, §11, §15.3, §15.7, RMC71, RMC74 | 45 (exact line set) |
| F-3 MAJOR resolver wiring | `FilteringResolver` is the only resolver (`Agent::with_parts`), `UreqDeliverer::with_lookup` seam, `ResolveRefused` inside `ureq::Error::Other` → `send_error_outcome` → `Refused`; other errors → `Retry` | W-5, §1.1, §7, §8.1, RMC62 | 52 (new, network-free), 7, 12, 44 |
| F-4 timeout relation | `PUSH_EXCHANGE_TIMEOUT_MS` = 18 000 > 1 000 + 3 × 2 000 + 10 000; whole `deliver` under one timeout | §3.1, §5.5, §10, RMC68 | 36 |
| F-5 test topic | `PUSH_TEST_TOPIC = "soos-test"` (≠ `PUSH_TOPIC`, const-checked); service-worker tag per kind; a test never touches alert retries or carries | W-11, §3.1, §5.6, §6.4, §9 | 31 |
| F-6 lock order | alerts mutex → scheduler mutex only; dispatcher never holds a mutex across await, store `flock` or transport; `AlertBook::started_us()` accessor | §1.1, §5.3, §5.6 | (structural; 28, 34 exercise the paths) |
| F-7 store recreation | Only service start and `push reset` create the file; deleted while running → `Missing` / `store_missing`, never silently re-keyed; page detects a key change and asks to re-enable | W-8, §3.3, §5.1, §5.7, §6.2, §6.4, §7, §9, RMC65 | 21, 35 |
| F-8 §7 wording | "No push **error** …"; a dispatcher panic ends `serve` with `TaskPanicked`, as for the follower | §5.6, §7 | 46 |
| F-9 stolen-session subscription | Residual risk documented; `devices` (service + creation time, ≤ 4) in `GET /api/push` and on the page; Docs steps (`push list`, `push remove N`, `passkeys remove N`) | §5.7, §9, §15.10, test 49 needles | 25, 33, 35, 49 |
| F-10 lost counts | Per-subscription carry (≤ 4) folded with `PushSummary::merge` into the next notification; superseded retries folded before the new summary | W-13, §5.2, §5.6, §15.12, RMC67 | 18, 54 (new) |
