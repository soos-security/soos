# Tester Contract — GitHub #339 follow-up: Web Push Notifications for Failed-Password Alerts (feature level 2)

- **Date**: 2026-10-06
- **Branch**: `feat/remote-auth-alerts` (level 1 implemented, nothing committed, pushed or stashed; O-5)
- **Spec**: `AI/architect_spec_remote_web_push.md` round 2 (plan-evaluator `VALIDATION_VERDICT: APPROVED`, MINOR F-11 to F-19)
- **ADR**: "[2026-10-06] Web Push Notifications for Failed-Password Alerts Through a Separate Sender Unit" (`AI/DECISIONS.md`)
- **Scope reminder (O-2, spec §0.1)**: every test encodes the safe subset only (time, source class, account class,
  kind, count). No test, fixture or fake carries, stores or displays a typed password; test 33 asserts that no
  endpoint token, key, JWT, ciphertext, login or journal user name reaches a response, an SSE byte or a log line.

## Files

| File | Kind | Tests |
|---|---|---|
| `crates/push-protocol/{Cargo.toml,src/lib.rs}` (new) | **scaffold only**: manifest with the spec §1.1 dependencies, `lib.rs` = doc + `#![forbid(unsafe_code)]`, no item | — |
| `crates/push-sender/{Cargo.toml,src/lib.rs}` (new) | **scaffold only**: manifest without `ureq` (needs the online lock update of F-18), `lib.rs` = doc + forbid attribute; no `main.rs` | — |
| `Cargo.toml` | members `crates/push-protocol`, `crates/push-sender`; `soos-push-protocol` workspace path entry (spec §1.1) | — |
| `Cargo.lock` | the two path packages only (offline `cargo check`) | — |
| `crates/push-protocol/tests/protocol_tests.rs` (new) | pure | constants guard, 1–6 (5 = two proptests) |
| `crates/push-sender/tests/sender_tests.rs` (new) | pure + `UnixStream::pair` threads + `TempDir`; no network | 7–12, 52 |
| `crates/remote/tests/common/push.rs` (new) | fixture: `UaFixture`, `decrypt_aes128gcm` (RFC 8291 receiver), `verify_vapid`, `FakeTransport`, endpoint/body builders | — |
| `crates/remote/tests/webpush_tests.rs` (new) | pure | constants guard, 13–17 |
| `crates/remote/tests/push_tests.rs` (new) | pure + `TempDir` store | constants guard, 18–22, 24 |
| `crates/remote/tests/push_server_tests.rs` (new) | end-to-end, paused frozen clock, `ScriptedJournal` + `FakeTransport` | constants guard, 23, 25–36, 54 |
| `crates/remote/tests/config_tests.rs` (appended `mod push_contract`) | pure | 37, 38 |
| `crates/remote/tests/routes_tests.rs` (appended `mod push_contract`) | pure | 39–41 |
| `tests/invariants/src/remote_push_contract.rs` (new) + `mod` line in `tests/invariants/src/lib.rs` | static | 42–51, 53 (RMC-S32–RMC-S42) |

## Plan-evaluator findings encoded in the tests

| Finding | Where |
|---|---|
| F-11 RFC 8291 Appendix A body is **144** bytes | test 13 asserts `expected.len() == 144`, `body.len() == 144` and the exact bytes |
| F-12 "new summary replaces a pending retry" unreachable as worded | tests 29 (last case) and 54 case 2 drive it through a `Retry` with `retry_after_s = 120`, due later than the next summary (+30 s); the delivered payload has the sum (5) and the replaced retry is never sent |
| F-13 `ProtectControlGroups=` unsupported in user managers | test 45: removed from the exact line set (37 lines) and listed as forbidden |
| F-15 `SubscriberInitExt::set_default()` installs the `log` bridge | test 53 forbids `SubscriberInitExt`, `.set_default(`/`set_default(`; requires `tracing::subscriber::set_global_default(` inside the body of `install_logging` and `install_logging();` as the first statement of `main` |
| F-16 cumulative frame deadline | test 9: a peer trickling the prefix (1 byte / 1.5 s) is dropped within `PUSH_FRAME_IO_TIMEOUT_MS + 1 s`; a valid prefix then a payload trickled 1 byte / s is dropped within `2 × PUSH_FRAME_IO_TIMEOUT_MS + 1 s` (a per-read timeout implementation was checked to fail: "a trickled prefix held the sender for 4.5 s") |
| F-14, F-17, F-18, F-19 | no test change (spec, Docs §8, RMC74 and walkthrough items); F-18 noted below |

## Tester Contract

Red evidence. In the hand-off state the new test targets do not compile because exactly the specified API is
missing: `soos-push-protocol` (unresolved imports of every §2 item), `soos-push-sender` (unresolved §8.1 items and the
`ureq` crate), `soos-remote` (`no field push on RemoteConfig` — 20 sites, the new `ConfigError` variants,
`soos_remote::push`, `soos_remote::webpush`, `soos_push_protocol` not a dependency yet). Because the three W-15
setup lines add `push:` to the shared harness, every `soos-remote` test target that includes `common/harness.rs`
is compile-red until `RemoteConfig.push` exists (same situation as level 1 with `alerts:`).
Assertion-level red was observed with a **temporary signature stub** (wrong constants and values, removed before
hand-off, `crates/remote/src` restored byte for byte, checked by SHA-256): every new behaviour test failed under it;
the captured first failure of each test is quoted below. Invariant tests compile today and fail on their assertions.
Achievability check: `protocol_tests` (8/8), `sender_tests` (7/7), `webpush_tests` (6/6) and `push_tests`
(7/7, one only on a stub error text) were also run **green against a throwaway reference implementation** of
the spec (removed; nothing of it remains), which found and fixed three over-constraints in the tests (see "Test
corrections made during Phase 2").

| Test (path::name) | Acceptance line / matrix ID | Red evidence (failure message) |
|---|---|---|
| `protocol_tests::test_rwp_protocol_constants_match_the_spec` | §2.1 / RMC61, RMC71 | FAILED under the signature stub (first constant assertion) |
| `protocol_tests::test_rwp_endpoint_allowlist_accepts_known_push_services` | W-5 a, §2.2 / RMC61 | FAILED under the signature stub (`host()` assertion) |
| `protocol_tests::test_rwp_endpoint_refuses_ssrf_shapes` | W-5 a, §2.2 / RMC61 | FAILED under the signature stub (first refusal case accepted) |
| `protocol_tests::test_rwp_public_address_predicate` | W-5 b, §2.3 / RMC62 | FAILED under the signature stub (first refused range accepted) |
| `protocol_tests::test_rwp_frame_round_trip_and_bounds` | §2.4 / RMC71 | FAILED under the signature stub (prefix assertion) |
| `protocol_tests::test_rwp_frame_decoding_never_panics` (+ `…_on_json_shapes`) | §2.4 / RMC71 | property (no-panic) tests: pass against any non-panicking decoder; compile-red until the decoders exist |
| `protocol_tests::test_rwp_status_classification` | §2.5 / RMC68 | FAILED under the signature stub (first classification) |
| `sender_tests::test_rwp_sender_filter_addresses` | W-5 b, §8.1 / RMC62 | compile-red (`filter_addresses`, `AddressLookup`, `LookupError` missing) |
| `sender_tests::test_rwp_sender_serves_one_request_with_fake_deliverer` | §8.1 / RMC71 | compile-red (`serve_connection`, `Deliverer`) |
| `sender_tests::test_rwp_sender_refuses_bad_frames` | §8.1, F-16 / RMC61, RMC71 | compile-red; power checked: a per-read-timeout variant fails ("held the sender for 4.5 s") |
| `sender_tests::test_rwp_sender_socket_setup` | §8.1 / RMC71 | compile-red (`bind_socket`) |
| `sender_tests::test_rwp_sender_refuses_root` | §8.1 / RMC71 | compile-red (`check_not_root`) |
| `sender_tests::test_rwp_sender_policy_defaults` | §8.1, F-3 / RMC62 | compile-red (`SendPolicy`, `UreqDeliverer`, `send_error_outcome`, `ResolveRefused`, crate `ureq`) |
| `sender_tests::test_rwp_sender_deliverer_uses_filtering_resolver` (test 52) | W-5, F-3 / RMC62 | compile-red (`UreqDeliverer::with_lookup`) |
| `webpush_tests::test_rwp_crypto_constants_match_the_spec` | §3.1 / RMC63 | stub: `assertion left == right failed, left: 0` |
| `webpush_tests::test_rwp_rfc8291_known_answer` | W-7, F-11 / RMC63 | stub: `unwrap() on Err(InvalidSubscriptionKeys)` |
| `webpush_tests::test_rwp_encrypt_inputs_and_bounds` | §4 / RMC63 | stub: `fixture keys rejected: InvalidSubscriptionKeys` |
| `webpush_tests::test_rwp_vapid_authorization_shape_and_signature` | §4, RFC 8292 / RMC64 | stub: `unwrap() on Err(InvalidKey)` |
| `webpush_tests::test_rwp_jwt_cache_reuse_and_bounds` | §4 / RMC64 | stub: `unwrap() on Err(InvalidKey)` |
| `webpush_tests::test_rwp_vapid_key_generation_and_redaction` | §4 / RMC63 | stub: `unwrap() on Err(Random)` |
| `push_tests::test_rwp_push_constants_match_the_spec` | §3.1 / RMC65, RMC67 | stub: `assertion left == right failed, left: 0` |
| `push_tests::test_rwp_scheduler_coalesces_and_rate_limits` | W-13, §5.2, F-10 / RMC67 | stub: `assertion left == right failed, left: Some(0)` |
| `push_tests::test_rwp_is_live_rule` | §5.3 / RMC67 | stub: `assertion failed: is_live(started, started, started)` |
| `push_tests::test_rwp_alert_payload_exact` | W-11, W-12, §5.4 / RMC69 | stub: `assertion left == right failed, left: ""` |
| `push_tests::test_rwp_store_round_trip_and_fail_closed` | W-8, §3.3, §5.1, F-7 / RMC65 | stub: `unwrap() on Err(Io)` |
| `push_tests::test_rwp_store_subscription_operations` | §5.1 / RMC65 | stub: `unwrap() on Err(Io)` |
| `push_tests::test_rwp_cli_lines_never_print_secrets` | F-9, O-2 / RMC69 | stub: `fixture keys rejected: InvalidSubscriptionKeys` |
| `push_server_tests::test_rwp_runtime_constants_match_the_spec` | §3.1 / RMC66, RMC68 | stub: `assertion left == right failed, left: 0` |
| `push_server_tests::test_rwp_subscribe_body_parsing` (test 23) | §6.2 steps 4–7 / RMC61, RMC66 | stub: `expected 200 subscribed, left: (413, "body_not_allowed")` |
| `push_server_tests::test_rwp_push_disabled_by_default` | W-2 / RMC60 | stub: `GET /api/push: not_found` |
| `push_server_tests::test_rwp_push_routes_require_authentication` | O-4 / RMC66, RMC73 | stub: `GET /api/push: not_found` |
| `push_server_tests::test_rwp_subscribe_gates_and_validation` | §6.2 / RMC61, RMC65, RMC66 | stub: `expected 403 forbidden, left: (413, "body_not_allowed")` |
| `push_server_tests::test_rwp_live_attempt_sends_one_coalesced_notification` | W-1, W-11, §5.6 / RMC67, RMC69 | stub: `expected 200 subscribed, left: (413, "body_not_allowed")` |
| `push_server_tests::test_rwp_delivery_outcomes` | W-13, §5.6, F-12 / RMC68 | stub: `expected 200 subscribed, left: (413, "body_not_allowed")` |
| `push_server_tests::test_rwp_notification_rate_is_bounded` | W-13, O-3 / RMC67 | stub: `expected 200 subscribed, left: (413, "body_not_allowed")` |
| `push_server_tests::test_rwp_test_notification` | §6.4, F-5 / RMC68 | stub: `expected 409 no_subscriptions, left: (404, "not_found")` |
| `push_server_tests::test_rwp_unsubscribe` | §6.3 / RMC66 | stub: `expected 200 subscribed, left: (413, "body_not_allowed")` |
| `push_server_tests::test_rwp_push_never_exposes_secrets` | O-2, W-14 / RMC69 | stub: `assertion left == right failed, left: 413` |
| `push_server_tests::test_rwp_push_failure_never_affects_other_features` | §7 / RMC70 | stub: `expected 200 subscribed, left: (413, "body_not_allowed")` |
| `push_server_tests::test_rwp_store_failure_is_unavailable` | §3.3, §5.7, F-7, F-9 / RMC65, RMC70 | stub: `unwrap() on Err(Io)` |
| `push_server_tests::test_rwp_unix_transport_round_trip` | §5.5, F-4 / RMC68, RMC71 | stub: `unwrap() on Err(Ttl)` |
| `push_server_tests::test_rwp_undelivered_counts_carry_forward` (test 54) | W-13, §5.6, F-10, F-12 / RMC67 | stub: `expected 200 subscribed, left: (413, "body_not_allowed")` |
| `config_tests::push_contract::test_rwp_push_config_keys` | W-2, W-12, §3.2, §3.3 / RMC60, RMC64 | stub: `unwrap() on Err(Syntax)` |
| `config_tests::push_contract::test_rwp_push_store_path_resolution` | §3.2 / RMC65 | stub: `assertion left == right failed, left: Err(Syntax)` |
| `routes_tests::push_contract::test_rwp_push_route_table` | §6.1 / RMC66, RMC73 | stub: `assertion left == right failed, left: ""` |
| `routes_tests::push_contract::test_rwp_push_csrf_rules` | §6.1 / RMC66 | stub: `assertion left == right failed, left: ""` |
| `routes_tests::push_contract::test_rwp_service_worker_asset` | §6.1, §9 / RMC73 | stub: `assertion left == right failed, left: ""` |
| `remote_push_contract::test_rmc_s32_push_crates_are_registered_and_isolated` | RMC-S32 / RMC71 | `[workspace.dependencies] hmac = "0.12"` missing |
| `remote_push_contract::test_rmc_s33_sender_tls_stack_without_openssl` | RMC-S33 / RMC72 | `[workspace.dependencies] ureq exact line` missing |
| `remote_push_contract::test_rmc_s34_sender_policy_literals` | RMC-S34 / RMC62 | `the sender code must contain .https_only(true)` |
| `remote_push_contract::test_rmc_s35_sender_unit_is_sandboxed` | RMC-S35, F-13 / RMC71 | `Cannot read packaging/soos-push-sender.service` |
| `remote_push_contract::test_rmc_s36_push_production_code_hygiene` | RMC-S36 / O-5 | `crates/remote/src/push.rs must exist (spec §1.1)` |
| `remote_push_contract::test_rmc_s37_push_modules_never_log_and_types_are_redacted` | RMC-S37 / RMC69, RMC70 | `crates/remote/src/push.rs must exist (spec §1.1)` |
| `remote_push_contract::test_rmc_s38_page_and_service_worker` | RMC-S38 / RMC73 | `Cannot read crates/remote/assets/sw.js` |
| `remote_push_contract::test_rmc_s39_push_is_documented` | RMC-S39 / RMC73 | `Docs/REMOTE_COMPANION.md has a ## 2d. section` |
| `remote_push_contract::test_rmc_s40_installer_installs_the_sender_without_enabling_it` | RMC-S40 / RMC73 | `install_remote.sh must contain -p soos-push-sender` |
| `remote_push_contract::test_rmc_s41_push_keys_never_reach_the_sender` | RMC-S41 / RMC71 | guard: passes on the empty scaffold (nothing named yet); it fails as soon as the sender names a key type, a crypto crate or a store file |
| `remote_push_contract::test_rmc_s42_sender_logging_is_fixed_and_bridgeless` | RMC-S42, F-1, F-15 / RMC69 | `the sender's logging must contain set_global_default` |

### Migrated existing tests

| Test | Old assertion | New assertion | Mandating acceptance line |
|---|---|---|---|
| `crates/remote/tests/common/harness.rs` `Harness::start_with` (`RemoteConfig` literal) | — (setup only) | adds `push: soos_remote::config::PushConfig::default(),`; no assertion changed | W-15, spec §12.8 |
| `crates/remote/tests/server_tests.rs` passkey-store harness (`RemoteConfig` literal) | — (setup only) | same line | W-15, spec §12.8 |
| `crates/remote/tests/alerts_server_tests.rs` `start_alerts` (`RemoteConfig` literal) | — (setup only) | same line | W-15, spec §12.8 |
| `tests/invariants/src/lib.rs::test_business_crates_forbid_unsafe_code` | list of 10 business crates | `"push-protocol"`, `"push-sender"` appended (strengthened, nothing removed) | spec §1.1, test 42 |

The fully qualified path follows the level-1 precedent (`soos_remote::config::AlertsConfig::default()`), so no `use`
line changes. No other line of an existing test file was changed (the appended `mod push_contract` blocks are new).

### Test corrections made during Phase 2 (before hand-off, against the throwaway reference)

1. Test 2: the needle `user` was dropped from the "error text never echoes the input" check (a fixed text such as
   "push endpoint has user info" is legitimate); the input fragments `pw@`, `%2E`, `abc`, `evil`, `8443`, `::1`,
   `17.188` remain.
2. Test 23: the case `"expirationTime":"soon"` was dropped (spec §6.2: `expirationTime` is optional and ignored; its
   type is not specified).
3. `subscribe_body`/`UaFixture` seed 1 checked to render with `-` or `_` in base64url so that the
   standard-alphabet refusal of test 14 is meaningful.

### Contract choices where the spec is silent (binding on Phase 4)

- `ServerState::with_push(settings: PushSettings, transport: Arc<dyn PushTransport>) -> Self`, ignored unless
  `config.push.enabled` (like `with_password_alerts`); `PushSettings { store_path, subject, previews, rp_id }` as
  §5.7.
- `PushTransport::deliver` returns `Pin<Box<dyn Future<Output = Result<DeliveryReply, TransportError>> + Send + '_>>`
  (the `BoxFuture` alias of `journal.rs`).
- `UnixPushTransport::new(socket_path: PathBuf, owner_uid: u32) -> Self`.
- `push::subscription_line(index: usize, sub: &PushSubscription) -> String`.
- `bind_socket(dir, uid)`: `dir` is the socket directory itself (`$XDG_RUNTIME_DIR/soos-push`); the socket is
  `dir/push.sock`; an existing directory not owned by `uid` or a stale socket of another owner is an error.
- `SocketError`, `SenderError`: any error type (tests use `is_err()` only).
- The disabled and the `unavailable` views report `sender: "unknown"` until an exchange happened.
- Decoders: every key of the request **and** of the reply is mandatory (`status` and `retry_after_s` may be
  `null` but not absent), unknown keys refused (`deny_unknown_fields` + presence checks).
- `encode_request`/`encode_reply` refuse invalid values with the decoder's `FrameError` (the encoder validates too).
- `PushSummary.first_unix_ms`/`last_unix_ms` are `at_us / 1000` of the attempts; the generic payload keeps
  `last_unix_ms` (only `source`/`account` become `null`).
- `push_socket_path` absent with push enabled may resolve to `None` or to the default path in `PushConfig`; without
  a runtime directory and without an explicit `push_socket_path` it is `ConfigError::NoRuntimeDir`.
- An oversized store file may be `Refused` or `Malformed` (both fail closed, file untouched).
- Test 23 is end-to-end (route answers): the spec defines the subscribe body only through §6.2 steps 4–7.

### Flakiness check

The new tests cannot run before Phase 4 (compile-red). The real-time tests of `sender_tests` (9, 52) passed against
the throwaway reference with margins of at least 1 s (the trickle cases end at about 2 s for a 3 s / 5 s bound).
The developer must run the new `soos-remote` suites 10 times in a row (`for i in $(seq 10); do cargo test --locked
-p soos-remote --all-features --test push_server_tests -q || break; done`) and the sender suite 10 times; the
server tests use only paused time with `FrozenClock`, no wall-clock assertion.

## Open points

1. **Relayed request versus O-2 (spec §0.1, plan-evaluator §0, §5)**: the owner's explicit confirmation of the safe
   subset (time, source class, account class, kind, count — never the typed password) is still recorded as **open**.
   Phase 2 was run because the owner's relayed request asks to "launch both in sequence"; nothing in these tests
   depends on the answer except the feature itself. The same request also asks to see the passwords that were
   tried: no test, fixture or seam carries typed text, `AGENTS.md` forbids it, and a request for it needs an
   `AGENTS.md` amendment and its own ADR. The orchestrator must still obtain and record the confirmation (quoted,
   dated) in the ADR and walkthrough 189.
2. **`AGENTS.md` workspace tree (existing invariant GitHub #239)**: registering the two crates makes
   `maintainer_hygiene_contract::test_workspace_maps_list_every_member_and_real_tests_dirs` fail ("AGENTS.md workspace
   tree must list Cargo member 'crates/push-protocol'"). Spec §1.1 omits this file; Phase 4/6 must add
   `push-protocol/` and `push-sender/` to the tree in `AGENTS.md` (and any other workspace map that invariant checks).
3. **F-18**: `rustls`, `rustls-webpki`, `webpki-roots` are not in `Cargo.lock` nor in the offline registry cache; the
   developer adds `ureq = { workspace = true }` to the sender, the workspace `ureq` line, runs one online lock update,
   then the `--locked`/`--offline` gates and test 43.
4. Phase 4 also owns: `crates/remote/assets/sw.js` (tests 26, 41, 48), the page strings of test 48, the sender
   `main.rs` (tests 42, 53), `packaging/soos-push-sender.service` (test 45), `scripts/install_remote.sh` and
   `scripts/candid_review.sh` (tests 42, 50), `Docs/REMOTE_COMPANION.md` §2d/§9 (test 49), the five audit events in
   `audit.rs` (test 47), and `hmac`/`aes-gcm`/`soos-push-protocol` in `crates/remote/Cargo.toml` (the test fixture
   `common/push.rs` decrypts with `aes_gcm`).

## Contract Migrations (2026-10-06, applied after the developer phase, owner-requested feature)

1. `tests/invariants/src/remote_passkey_contract.rs` `test_rmc_s18_page_uses_modal_webauthn_without_storage`:
   `serviceWorker` removed from the forbidden tokens of `app.js`, because the Web Push ADR (owner request) needs
   one service worker. Replaced by a stricter positive rule: exactly one `.register(` call in `app.js`, the
   same-origin `/sw.js` with scope `/`. Every other forbidden token is unchanged.
2. `crates/remote/tests/config_tests.rs` `test_rwp_push_config_keys`: fixture arithmetic defect. `"/" + n + "/s"`
   is `n + 3` bytes, so the at-limit path now uses `MAX_SOCKET_PATH_LEN - 3` (was `- 4`, which built 106 bytes while
   asserting 107) and the over-limit path `MAX_SOCKET_PATH_LEN - 2` (was `- 3`, which was exactly at the limit).
   The assertions and `MAX_SOCKET_PATH_LEN` are unchanged.
3. `crates/remote/tests/push_tests.rs` (lines 102-103) and `crates/remote/tests/push_server_tests.rs` (lines
   577-578, 1047, 1383, doc comment 1361): the pinned topic values `soos-alerts` / `soos-test` become
   `soosalerts` / `soostest`. Evidence (owner's iPhone, 2026-10-06): `web.push.apple.com` answered
   `400 {"reason":"BadWebPushTopic"}` to `Topic: soos-test` and `201` to `Topic: soostest` with the same VAPID key,
   payload and RFC 8291 encryption; after the change the service reported `last_delivery: delivered`. The
   assertions still check exact values. New regression test `test_rwp_topics_are_ascii_alphanumeric_for_apple`
   (fails against `soos-test`); `crates/remote/src/lib.rs` const-checks both topics. The service-worker display tags
   `soos-alerts` / `soos-test` in `sw.js` are not push topics and are unchanged; `push-protocol` keeps accepting the
   RFC 8030 alphabet (`protocol_tests.rs` still uses `soos-alerts` to test that check).
4. `crates/remote/tests/push_server_tests.rs` `test_rwp_delivery_outcomes` (single-subscription `410` block): CI on
   2026-10-07 (run 37537551565) observed `(subscriptions 0, last_delivery null)` because the gone subscription is
   removed on the blocking pool and the outcome is recorded after that await; one `pump()` did not wait for it on a
   slower runner. The block now waits with the new bounded helper `wait_view` (polls `GET /api/push` until the
   expected state, at most about 2 s of real time) before the unchanged exact assertion
   `(subscriptions, last_delivery) == (0, "gone")`. No assertion changed.
