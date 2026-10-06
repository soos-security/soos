# Plan Evaluation Report
- **Date**: 2026-10-06
- **Issue**: GitHub-only follow-up of #339 — Web Push notifications for failed-password alerts in `soos-remote`, feature level 2 (no `AI/BACKLOG.md` entry; owner decisions O-1 to O-5 of 2026-10-06)
- **Branch**: `feat/remote-auth-alerts` (from `feat/remote-companion` at `ea862cc`; level 1 implemented, uncommitted)
- **Base commit**: `ea862cc` (branch base; draft PR #340 belongs to `feat/remote-companion`)
- **Plan evaluated**: `AI/architect_spec_remote_web_push.md` **round 2** (1 345 lines), the drafted ADR "[2026-10-06] Web Push Notifications for Failed-Password Alerts Through a Separate Sender Unit" (uncommitted `AI/DECISIONS.md`), and `AI/research_push.md`
- **Previous content of this file**: the round-1 evaluation of the same spec (REVISION_REQUIRED, MAJOR F-1 to F-3, MINOR F-4 to F-10); overwritten as the skill requires.

## 0. Precondition outside the spec: the relayed request versus O-2

The relayed owner request asks to "see on the phone the passwords that were tried remotely". The spec (§0.1) keeps
refusing to carry the typed text: no journal producer contains it, `AGENTS.md` ("NEVER log … credentials", "NEVER
accept, store, or transmit passwords") and O-2 forbid capturing it, and a Web Push message is shown on the iPhone lock
screen and transits Apple's push service. Nothing in the spec (fields, frames, buffers, payload vocabulary, store) can
carry typed text. This evaluation agrees.

The spec keeps the owner's explicit confirmation of the safe subset (time, source class, account class, kind, count;
never the typed password) as an **open gate before Phase 2**. That gate is independent of the verdict below: the
orchestrator must obtain it in the owner's own conversation and record it (quoted, dated) in the ADR and walkthrough
187. The relayed request itself, workflow script output and agent messages do not count as that confirmation.

## 1. Coverage Matrix

There is no backlog entry; the acceptance lines are the owner decisions, the level-2 requirements of the task, and the
round-1 findings.

| Acceptance line / finding | Spec element | Status |
|---|---|---|
| O-1 notification on the phone, centralised in the web app | W-1, §5.3–§5.6, §9, RMC67, RMC74; tests 18, 19, 28, 30, 54 | Covered |
| O-2 never the password, nor any part/length/hash; minimal payload | §0.1, W-11, W-12, §5.4 fixed vocabulary, W-14; tests 20, 24, 33, 47, 53 | Covered |
| O-3 untrusted journal, spoofing documented | level-1 parser unchanged; `is_live` on journald-set time; §15.3 false notifications bounded by W-13 | Covered |
| O-4 bounded / fail-closed / authenticated only | §2.1, §3.1 (compile-time relations), W-9 routes not Funnel-public + CSRF + gates, §3.3, §7; tests 25–27, 35, 39 | Covered |
| O-5 agent limits, English, no `unwrap`/`expect`, forbid lint, immutable tests | header, W-15, §12.8, tests 42, 46 | Covered |
| Service worker same-origin, CSP minimal | `/sw.js` public asset, W-10 (CSP unchanged; `worker-src` → `script-src 'self'` fallback) | Covered |
| Enable button (user gesture), `requestPermission`, `pushManager.subscribe` with VAPID key | §9, test 48 | Covered |
| Subscription storage bounded, 0600, authenticated create/remove | W-8, §3.3, §5.1, §6.2, §6.3; tests 21, 22, 27, 32, 35 | Covered |
| Unsubscribe; test notification | §6.3, §6.4; tests 31, 32 | Covered |
| VAPID RFC 8292 ES256 | §4; tests 15, 16, 37 | Covered |
| RFC 8291 `aes128gcm` | §4 key schedule; test 13 (RFC Appendix A) | Covered (**F-11**: wrong body length cited) |
| Crates: HMAC-based HKDF, rustls/TLS, cargo-deny licences, no OpenSSL | W-6, W-7, §1.1; tests 42, 43 | Covered (F-18 lock-file note) |
| ADR: separate sender process versus relaxing `AF_UNIX` | W-3, §8.2, §15.3 | Covered (F-13, F-14 sandbox details) |
| Endpoint allowlist, https only, no redirects, no private IPs | W-5, §2.2, §2.3, §8.1; tests 1–3, 7, 9, 12, 44, 52 | Covered |
| Bounded timeouts, retries, rate limits; 404/410 removal | §2.1, §3.1, W-13, §5.6; tests 6, 29, 30, 36, 54 | Covered (F-12, F-16, F-17) |
| Trigger: each new level-1 alert, coalesced and rate-limited | W-1, W-13 | Covered |
| New ADR, spec, matrix rows, walkthrough 187, Docs owner steps | §13 RMC60–RMC74, §14, §16, test 49 | Covered |
| Round-1 F-1 (dependency logs) | W-14, §8.1 `install_logging`, test 53 | **Resolved** (test-power gap F-15) |
| Round-1 F-2 (sender sandbox) | §8.2, §15.3, §15.7, test 45, RMC74 | **Resolved** (F-13, F-14) |
| Round-1 F-3 (resolver wiring) | W-5, §8.1 `FilteringResolver`/`with_lookup`/`send_error_outcome`, test 52 | **Resolved** |
| Round-1 F-4 … F-10 | §3.1 relation 17 000 < 18 000; `PUSH_TEST_TOPIC`; lock order §5.3; store recreation §3.3; §7 wording; `devices` + §15.10; carry §5.6 | **Resolved** (F-12 on test 54 wording) |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| ureq `Agent::with_parts(config, connector, resolver)` exists | `ureq-3.4.2/src/agent.rs:130` | `pub fn with_parts(config: Config, connector: impl Connector, resolver: impl Resolver) -> Self` | Yes |
| `Resolver` trait: `resolve(&self, &Uri, &Config, NextTimeout) -> Result<ResolvedSocketAddrs, Error>`; requires `Debug` | `ureq-3.4.2/src/unversioned/resolver.rs:29–41` | as cited; `Debug + Send + Sync + 'static` (spec gives a manual `Debug`) | Yes |
| `ResolvedSocketAddrs` capped at 16 | `resolver.rs:54,59` | `ArrayVec<SocketAddr, 16>` | Yes |
| `ureq::Error::Other(Box<dyn Error + Send + Sync>)`, `HostNotFound`, `Timeout(_)` | `ureq-3.4.2/src/error.rs:36,39,182` | present | Yes (`e.is::<ResolveRefused>()` workable via `downcast_ref`) |
| The resolver is called once per request without a proxy and the connector uses only its addresses | `ureq-3.4.2/src/run.rs:381–397` (`agent.resolver.resolve` only when no proxy / no-proxy / SOCKS4), `transport/connect.rs:64` (second resolve only for a CONNECT proxy) | as cited; `proxy(None)` disables the second path | Yes |
| Config builder: `https_only`, `max_redirects`, `proxy(Option)`, `http_status_as_error`, `timeout_global`, `timeout_connect`, `max_response_header_size` | `ureq-3.4.2/src/config.rs:491–763` | present | Yes |
| ureq's `rustls` dependency has `default-features = false` and features `logging, std, tls12`; ureq feature `rustls` = `rustls-no-provider` + `_ring` + `rustls-webpki-roots` | `ureq-3.4.2/Cargo.toml:101–112, 194–202` | as cited (no `aws-lc-rs`) | Yes |
| `tracing-subscriber` default features include `tracing-log` | `tracing-subscriber-0.3.23/Cargo.toml:64` | `default = ["smallvec","fmt","ansi","tracing-log","std"]` | Yes |
| `.init()`/`.try_init()` install `LogTracer` | `tracing-subscriber-0.3.23/src/util.rs:61–96` | as cited | Yes |
| **`SubscriberInitExt::set_default()` also installs `LogTracer`** | `tracing-subscriber-0.3.23/src/util.rs:39–46` | `let _ = tracing_log::LogTracer::init();` | **Not covered by test 53 → F-15** |
| `filter::Targets` available with default features; matching is by target prefix | `tracing-subscriber-0.3.23/src/filter/mod.rs:35`, `directive.rs:182,250` | `starts_with` | Yes (`ureq` would also cover `ureq_proto`; the explicit entries are harmless) |
| RFC 8291 Appendix A body "145 bytes" (spec test 13, research §3.1) | decoded `DGv6ra1n…a-fN` (192 base64url chars) | **144 bytes** (86-byte header + 41-byte plaintext + 1 delimiter + 16 tag) | **No → F-11** |
| `deny.toml` allows `ISC`, `CDLA-Permissive-2.0` (`ring`, `webpki-roots`) | `deny.toml` `[licenses] allow` | present | Yes |
| `rustls`, `rustls-webpki`, `webpki-roots`, `ring` in `Cargo.lock` | `Cargo.lock` | **absent** (`ring 0.17.14`/`untrusted 0.9.0` only in the local registry cache; `rustls`, `webpki-roots` not cached) | Spec says ureq is locked (true), not the TLS crates → F-18 (informational) |
| `RandomSource` seam | `crates/remote/src/auth.rs:40` | `Arc<dyn Fn(&mut [u8]) -> Result<(), RandomError> + Send + Sync>` | Yes |
| `UnixClock` (ms) injected; `started_us = clock() * 1000` | `server.rs:114`, `alerts.rs:841` | as cited | Yes |
| `AlertBook::started_us` private field | `alerts.rs:365` | private; spec adds an accessor | Yes |
| `identity::is_valid_host_name`, `MAX_SOCKET_PATH_LEN = 107`, `STORE_LOCK_TIMEOUT_MS = 500`, `resolve_alerts_ack_path`, `with_file_owner_uid`, `check_action_csrf` (private) | `identity.rs:87`, `lib.rs:53`, `lib.rs:226`, `config.rs:531`, `server.rs:182`, `routes.rs:200` | as cited | Yes |
| Supervised task panic ends `serve` with `TaskPanicked` | `server.rs:262,673,690` | as cited | Yes (§5.6/§7 wording now correct) |
| Workspace lines `p256 … features = ["ecdsa"]`, `aes-gcm = "0.10"`, `tracing-subscriber … ["fmt","env-filter"]`; `hmac 0.12.1` locked | root `Cargo.toml:65,81,85`; `Cargo.lock:1568` | as cited | Yes |
| systemd 262 on the owner's host; `kernel.unprivileged_userns_clone = 1` | `systemctl --version`, `sysctl` | `systemd 262 (262-1-arch)`, `1` | Yes |
| `/etc/resolv.conf` is a regular file with `nameserver 100.100.100.100` (+ `fd7a:115c:a1e0::53`); `hosts: mymachines resolve [!UNAVAIL=return] files myhostname dns` | `/etc/resolv.conf`, `/etc/nsswitch.conf` (read only) | as cited | Yes (DNS under `TemporaryFileSystem=/run:ro` plausible) |
| `ProtectProc=` unsupported in user managers | `man systemd.exec` (262) | "only available for system services and is not supported for … per-user instances" | Yes |
| `ProtectHome=tmpfs` + `BindReadOnlyPaths=`, `TemporaryFileSystem=`, `PrivateDevices=`, `PrivateIPC=`, `ProtectKernel*`, `ProtectHostname=`, `ProtectClock=` available to user units with implied `PrivateUsers=` | `man systemd.exec` (262) | as cited | Yes |
| **`ProtectControlGroups=` in a user unit** | `man systemd.exec` (262) | "only available for system services and **is not supported** for services running in per-user instances" — same wording as `ProtectProc=` | **No → F-13** |
| `RuntimeDirectory=` bound into the namespace over `ProtectHome=tmpfs` and `TemporaryFileSystem=/run:ro` | `man systemd.exec` (262) | not stated by the man page (only the symlink parameter is said to be created after `BindPaths=`/`TemporaryFileSystem=`) | **Unverified → F-14** |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- PAM ↔ daemon boundary untouched; `soos-remote` keeps `AF_UNIX` only and its unit unchanged; all keys and crypto stay in
  `soos-remote` (test 51). PASS.
- Failure scenario (round-1 F-2): a parser exploit in the sender reads `$HOME` or reaches `remote.sock`/the buses.
  Round 2 hides `$HOME` (`ProtectHome=tmpfs` + the binary bound read-only) and `/run` (`TemporaryFileSystem=/run:ro`),
  and §15.3 states the remaining reach precisely. PASS for the design.
- Failure scenario: the exact line set pinned by test 45 contains a directive the user manager does not support
  (`ProtectControlGroups=`, F-13) or the runtime directory is not visible inside the namespace (F-14) → the unit does
  not start or the socket is unreachable. Fail-closed (no push, "Push sender not running"), but an immutable exact-line
  test would then pin a broken unit. **FINDING (MINOR, F-13, F-14)**.
- Failure scenario: a compromised sender connects to an abstract Unix socket of the host network namespace, for
  example Xwayland's `@/tmp/.X11-unix/X0`, and injects or reads X11 input of X clients. §15.3 names abstract sockets
  generically only. **FINDING (MINOR, F-17)**.

### Pillar 2 — PAM deadline & concurrency
- Not on the PAM path. Lock order is now stated (alerts → scheduler only; no mutex across `.await`, `flock` or the
  transport; leaf mutexes). PASS.
- Timing: `PUSH_EXCHANGE_TIMEOUT_MS` 18 000 > 1 000 + 3 × 2 000 + 10 000 for a single client. Failure scenario: another
  owner-uid process holds the sequential sender (or trickles a 12 KiB frame byte by byte, if "under
  `PUSH_FRAME_IO_TIMEOUT_MS` each" is implemented as a per-syscall read timeout) → the request of `soos-remote` sits in
  the listen backlog, `soos-remote` times out at 18 s and retries, the sender later accepts the abandoned connection
  whose request bytes are already buffered and sends it → duplicate notification; or push is stalled for a long time.
  Only an owner-uid process can do this (socket `0600`, peer uid checked). **FINDING (MINOR, F-16)**.

### Pillar 3 — Panic safety & fail-closed
- Store invalid/insecure/absent-while-running, RNG failure, clock before 2023, sender absent, malformed reply, 3xx,
  unknown status, private DNS answer, resolver refusal: each maps to `unavailable`/`failed`/`rejected`/`refused` with
  nothing sent and counts carried (§7). A deleted store is never silently re-keyed. PASS.
- §7 now distinguishes a push *error* from a dispatcher panic (`TaskPanicked`). PASS.
- ureq's `DefaultResolver` contains `unwrap` (after `ensure_valid_url`); sender-side only, the process restarts. PASS.

### Pillar 4 — Dependencies
- `ureq =3.4.2`, `default-features = false, ["rustls"]` → rustls 0.23 (`logging`, `std`, `tls12`), `ring`,
  `webpki-roots`; no `aws-lc-rs`, OpenSSL or `native-tls` in the normal graph (resolver 2 keeps `ort-sys`'s
  build-dependency features apart); licences allowed. PASS.
- `rustls`, `rustls-webpki`, `webpki-roots` are not in `Cargo.lock` nor in the local registry cache: the developer
  needs one online lock update before the `--locked`/`--offline` gates and test 43 can pass. **Informational (F-18)**.
- HKDF as RFC 8291 HMAC calls over `hmac 0.12` (already locked), `aes-gcm 0.10`, `p256` `ecdsa` line untouched. PASS.

### Pillar 5 — Data confidentiality
- Password (O-2): payload only from enums and counts through a fixed vocabulary; no route, frame, buffer or store field
  can carry typed text. PASS.
- Round-1 F-1: `install_logging` uses `registry()` + `fmt::layer()` + fixed `Targets` + `tracing::subscriber::
  set_global_default` (no `log` bridge, no environment filter); `log`'s runtime maximum stays `Off`, so the `ureq-proto`
  byte dump and rustls logs are discarded. Verified against `tracing-subscriber 0.3.23`. PASS for the design.
- Failure scenario (test power): a developer writes `registry().with(layer).with(filter).set_default()` (kept guard)
  instead of `tracing::subscriber::set_global_default(...)`. `SubscriberInitExt::set_default` calls
  `LogTracer::init()` (`util.rs:43`), re-opening the `log` → `tracing` bridge; test 53 only forbids `.init()`,
  `try_init`, `LogTracer`, `tracing_log` and requires the substring `set_global_default`, which can appear elsewhere
  (comment). The bridge would then forward `ureq`/`rustls` records — the fixed `Targets` filter still turns them off,
  so this is defence in depth only. **FINDING (MINOR, F-15)**.
- Endpoint, keys, JWT, ciphertext never in responses, SSE or logs (test 33); redacted `Debug`; no `Debug` on
  secret-bearing types (test 47). PASS.

### Pillar 6 — Test integrity and power
- Existing tests: setup-only `push: PushConfig::default(),` in three `RemoteConfig` literals (precedent A-12 and the
  setup-migration memory note); CSP, `RequestHead`, route answers unchanged. PASS.
- Round-1 F-3: test 52 fails if `UreqDeliverer` ignores the injected lookup (ureq's resolver) or maps the refusal to
  `Retry`; test 44 pins `Agent::with_parts(` and forbids the unfiltered constructors. PASS.
- Failure scenario (crypto KAT): test 13 asserts "the exact 145-byte body"; the RFC vector is **144** bytes. A tester
  who encodes a length assertion of 145 writes an immutable test no correct implementation can pass. **FINDING
  (MINOR, F-11)** — must be corrected before Phase 2.
- Failure scenario (scheduler): test 54 case 2 says "a first summary … gets `Retry` once, then 2 more attempts produce
  a new summary **before the +5 s retry is due**". The next summary is due at `last_sent + PUSH_MIN_INTERVAL_MS`
  (30 s), so it can never be taken before a 5 s retry: the scenario is unreachable as written. The same applies to the
  phrase "a new summary replaces a pending retry" in test 29 unless the retry delay exceeds 30 s. **FINDING (MINOR,
  F-12)** — must be reworded before Phase 2.
- Crypto: test 13 (CEK, NONCE, exact bytes) and test 15 (signature verifies, exact claims) would fail on a wrong
  `key_info` order, a DER signature or an `aud` with a path. PASS.

## 4. Findings

No CRITICAL or MAJOR finding. The three round-1 MAJOR findings are resolved and their resolutions were checked against
the pinned dependency sources (ureq 3.4.2, tracing-subscriber 0.3.23) and the systemd 262 man page.

- **[MINOR] F-11 — Wrong length of the RFC 8291 Appendix A body.** The vector decodes to **144** bytes (86 + 41 + 1 +
  16), not 145 (spec test 13, research §3.1). Required change (binding on the tester before Phase 2, and on the
  architect for the spec text): test 13 compares the exact decoded bytes of the RFC string and, if it asserts a
  length, asserts 144. Correct "145" in the spec and in `AI/research_push.md`.
- **[MINOR] F-12 — Test 54 case 2 (and the "replaces a pending retry" case of test 29) is unreachable as worded.**
  Required change before Phase 2: drive the cancellation through a pending retry later than the next due summary,
  e.g. `Retry` with `Retry-After: 120` (or the second retry at +30 s after the first, i.e. +35 s, while the next
  summary is due at +30 s), then assert the cancelled `effective` is folded into the delivered count (sum 5).
- **[MINOR] F-13 — `ProtectControlGroups=yes` is documented as not supported in per-user managers** (same wording the
  spec used to drop `ProtectProc=`). Remove it from §8.2 and from test 45's exact line set (or record in RMC74 that the
  owner's systemd accepts it and what it does there). Also note in §8.2 that `ProtectClock=` implies `DeviceAllow=` and
  `PrivateDevices=` implies `DevicePolicy=closed`, which rely on cgroup device control the user manager may not
  enforce (the mount part still applies).
- **[MINOR] F-14 — Visibility of `RuntimeDirectory=` under `ProtectHome=tmpfs` + `TemporaryFileSystem=/run:ro` is not
  documented by the man page.** The spec asserts it; it is plausible but unverified, and test 45 forbids `BindPaths=`,
  which would be the obvious fix. Required change: RMC74 explicitly checks that `soos-remote` reaches
  `$XDG_RUNTIME_DIR/soos-push/push.sock` (the test notification proves it) and §8.2 names the fallback (an explicit
  `BindPaths=%t/soos-push`, adopted through a documented contract migration of test 45 if needed).
- **[MINOR] F-15 — Test 53 does not forbid the bridge-installing `SubscriberInitExt::set_default()`.** Add
  `SubscriberInitExt` and `.set_default(` to the forbidden needles of test 53 (the free function
  `tracing::subscriber::set_global_default` remains the required form), and require that the `set_global_default` call
  is in the body of `install_logging`.
- **[MINOR] F-16 — Sender frame deadlines and queueing.** State that the sender bounds each frame by one cumulative
  deadline (prefix + payload ≤ `PUSH_FRAME_IO_TIMEOUT_MS` each as a deadline, not a per-`read` timeout), so a trickling
  owner-uid peer cannot hold the sequential sender for longer than about 4 s; and add to §15.3 that a concurrent
  owner-uid client can delay `soos-remote`'s exchange past 18 s, after which the sender may still send the abandoned
  request (possible duplicate notification). Extend test 9 with a peer that trickles one byte every second.
- **[MINOR] F-17 — Abstract-namespace sockets reachable by a compromised sender.** Name the concrete case in §15.3 and
  Docs §8 (Xwayland's abstract X11 socket on the owner's desktop). Optional cheap hardening for a later ADR: the
  sender applies a Landlock scope restricting abstract Unix sockets (`LANDLOCK_SCOPE_ABSTRACT_UNIX_SOCKET`, kernel
  ≥ 6.12; owner host 7.2) after binding its listener.
- **[MINOR] F-18 — `rustls`, `rustls-webpki`, `webpki-roots` are not yet in `Cargo.lock`.** The developer must run one
  online lock update (a plain `cargo check -p soos-push-sender` without `--locked`, which adds only the new entries)
  before the `--locked` gates and test 43 (`--offline`) can pass; record the new lock entries in the
  walkthrough and re-run `cargo deny check`.
- **[MINOR] F-19 — `Retry-After` is not honoured across summaries.** After a `429` with `Retry-After: 120`, the next
  alert summary (due 30 s after the previous send) cancels the retry and is sent at once to the same push service.
  Either defer the next delivery to that subscription until the `Retry-After` instant (carrying the counts) or record
  the behaviour in §15; Apple counts such calls against the rate limit.

## 5. Verdict

The round-2 spec resolves every round-1 finding: the sender installs no `log` bridge and uses a fixed filter, the
sandbox hides `$HOME` and `/run` with precise residual reach, and the filtering resolver is the only resolver of the
production agent with a network-free test that fails on wrong wiring or mapping. O-2 is preserved end to end, every new
route is authenticated and CSRF-checked, every collection and frame is bounded, and every failure is fail-closed. The
remaining findings are MINOR. F-11 and F-12 are binding corrections to the test wording that the tester must apply
when writing tests 13, 29 and 54 (and the architect in the spec text). F-13 to F-19 are to be applied in the spec, the
tests or the residual-risk section before or during Phase 2.

Independent of this verdict, Phase 2 must not start before the owner's own confirmation of the safe subset (§0 above,
spec §0.1) is recorded.

VALIDATION_VERDICT: APPROVED
