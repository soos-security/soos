# Walkthrough 112 — Atomic Auth Rate-Limit Reservation and Client Message Tag

- **Date**: 2026-09-30
- **Issues**: GitHub #200 (DMN-10, rate-limit check/record not atomic), GitHub #204 (DMN-15,
  Request/Event frames without a discriminator)
- **Branch**: `fix/p2-daemon-ratelimit-wire-discriminator`
- **Matrix criteria**: DRW1–DRW6 (new component `daemon-ratelimit-wire-discriminator`)
- **ADR**: `AI/DECISIONS.md` [2026-09-30] "Client Message Tag Trailer and Atomic Auth Attempt
  Reservation"
- **Scope**: `soos-protocol` (new `message` module), `soos-daemon` dispatcher, the frame encoders
  of `soos-pam`, `soos-admin-cli` and `soos-gui`, new tests only, docs. No existing test changed.

---

## 1. Context

**DMN-10.** `ConnectionDispatcher::handle_request` checked the per-UID `Auth` limit with
`check_allowed` under the policy *read* lock at the start of a request and recorded the attempt
only after the consensus loop. Concurrent requests of one UID all passed the same remaining
attempt; requests that returned early (not enrolled, camera unavailable) or were cancelled never
consumed one. On `origin/main` the recorded result was already honored on the Allow path (an
`Allow` was downgraded when the late recording failed), but the non-atomic check/record remained:
two concurrent denied requests with `max_attempts = 1` were both evaluated.

**DMN-15.** `read_and_process` decoded every payload as both `Request` and `Event` and, when both
matched, chose `Request` iff `uid_hint == peer.uid`. The frame type depended on a heuristic, and
a sender could choose which handler ran (e.g. a root peer sending bytes that are also an `Event`).

## 2. Design (Phase 1)

### 2.1 Atomic reservation (#200)

Step `8-pre` now calls `AuthorizationEngine::record_attempt(uid, now)` under
`pipe.policy.write()` — check and record in one critical section — right after the deadline
check and before the camera wake, the enrollment lookup and any vision work. A rejected
reservation answers `ProtocolError` / `RateLimited` immediately. Step `8f` no longer records
anything and renders the consensus verdict as is. No new type; `check_allowed` stays public for
other callers.

### 2.2 Message tag trailer (#204)

New module `crates/protocol/src/message.rs`:

| Item | Purpose |
|---|---|
| `MESSAGE_TAG_REQUEST = 0xA0`, `MESSAGE_TAG_EVENT = 0xA1`, `MESSAGE_TAG_MIN = 0x80` | trailer bytes |
| `enum ClientMessage { Request(Request), Event(Event) }` | classified message |
| `enum FrameFormat { Tagged, Legacy }` | observability |
| `enum MessageError { Empty, UnknownTag(u8), Malformed, Ambiguous }` | every variant rejects |
| `encode_request`, `encode_event` | tagged, length-prefixed, `<= MAX_MESSAGE_SIZE` |
| `decode_client_message(payload)` | classification rule |

Why a **trailer** rather than a leading kind byte or a protocol v2 envelope:

- The message body stays byte-identical to codec v1 and `CURRENT_VERSION` stays `1`. Existing
  v1 readers (`decode::<Request>` uses `postcard::from_bytes`, which ignores trailing bytes)
  still read tagged frames, so the existing mock daemons of the PAM, admin and GUI test suites
  keep working unchanged.
- It is structurally unambiguous: a complete v1 `Request` or `Event` ends on the terminating
  byte of a `u64` varint (`deadline_monotonic_ns` / `timestamp_monotonic_ns`), whose high bit is
  clear. A last byte `>= 0x80` is therefore always a tag, and a last byte `< 0x80` is always a
  legacy frame. Tagged frames never need double decoding; only legacy frames do, and a legacy
  frame matching both types is rejected (`Ambiguous`).

The dispatcher replaces the double decode and the UID heuristic with `decode_client_message`;
any `MessageError` closes the connection without a response and without running a handler.

### 2.3 Compatibility and migration

- New daemon, old clients: untagged v1 frames are still served when unambiguous.
- New clients, old daemon: the trailer makes the old exact-decode fail, the connection is closed,
  and the PAM module returns `PAM_IGNORE` (password). `pam_soos.so` and `soos-daemon` ship in one
  package; upgrade both together.
- Responses are unchanged in both directions.

## 3. Tests first (Phase 2) — red evidence

New files only:

| File | Tests |
|---|---|
| `crates/protocol/tests/client_message_tests.rs` | 13 unit tests + 4 proptests (DRW3–DRW5) |
| `crates/daemon/tests/wire_discriminator_tests.rs` | 6 dispatcher tests (DRW4–DRW6) |
| `crates/daemon/tests/rate_limit_reservation_tests.rs` | 5 dispatcher tests (DRW1–DRW2) |
| `crates/pam/tests/wire_tag_tests.rs` | PAM `Auth` and `PasswordFailed` frames are tagged |
| `crates/admin-cli/tests/wire_tag_tests.rs` | `status` and `test-pam` frames are tagged |
| `crates/gui/tests/wire_tag_tests.rs` | preview frame is tagged |

The ambiguous legacy fixture is a 37-byte `Request` (`uid_hint = 0`, empty service,
`request_id = [00, 01, 05, 1E, 'a' x 28]`) that is also an exact `Event`
(`request_id = None`, `uid = Some(5)`, a 30-byte service, `timestamp = 0`).

Red runs:

- Protocol suite against a stub `message` module (untagged encoders, decoder always
  `Malformed`): 10 of 17 failed (every classification and tag assertion).
- Daemon, PAM, admin and GUI suites against the `origin/main` production code:
  `test_204_ambiguous_legacy_frame_is_rejected_and_connection_closed` failed (the event handler ran
  and the following Status request was served), the three tagged-frame dispatcher tests failed
  (tagged frames were rejected as malformed), every `wire_tag_tests` test failed (untagged
  frames), and in `rate_limit_reservation_tests`
  `test_200_concurrent_denied_auths_with_one_attempt_yield_one_evaluation` (both requests
  evaluated), `test_200_attempt_ending_before_vision_work_is_recorded` (second request not rate
  limited) and `test_200_attempt_is_reserved_before_vision_work` (no attempt tracked while the
  request was in flight) failed.
- `test_204_unknown_message_tag_is_rejected`, `test_204_legacy_untagged_status_request_still_served`
  and `test_200_allow_then_rate_limited_with_one_attempt` pass on both sides: they are regression
  guards (compatibility and the sequential limit). The matching-face concurrency test
  (`..._allow_at_most_once`) guards the pre-existing Allow downgrade; its first draft used four
  clients and failed only on the per-UID connection cap (`DEFAULT_MAX_CONNECTIONS_PER_UID` = 2),
  so it now uses two.

## 4. Audit (Phase 3)

- No `unwrap`/`expect` added to production code; `decode_client_message` uses `split_last` and
  returns typed errors.
- Allocation stays bounded: the daemon still rejects a declared size above `MAX_MESSAGE_SIZE`
  before allocating; `encode_tagged` checks the size with the trailer included; the intermediate
  serialized body is wrapped in `Zeroizing`.
- Logs name only the `MessageError` (no payload bytes, service or UID from the rejected frame);
  the log text avoids the words checked by `logging_audit_test`.
- No error becomes an `Allow`: a rejected frame closes the connection (PAM `PAM_IGNORE`), a
  rejected reservation is `ProtocolError` / `RateLimited`.
- `#![forbid(unsafe_code)]` unchanged in `soos-protocol`; no Tokio added to the PAM module (it only
  switches encoder function).

## 5. Implementation (Phase 4)

- `crates/protocol/src/message.rs` (new) and re-exports in `crates/protocol/src/lib.rs`.
- `crates/daemon/src/dispatcher.rs`: step 4 uses `decode_client_message`; step `8-pre` reserves
  the attempt under the write lock; step `8f` no longer records.
- Encoders: `crates/pam/src/ipc.rs` (`encode_request`, `encode_event`),
  `crates/admin-cli/src/status.rs`, `crates/admin-cli/src/test_pam.rs`,
  `crates/gui/src/ipc_camera.rs` (`encode_request`).
- Fuzz target `crates/protocol/fuzz/fuzz_targets/decode_client_message.rs` (registered in the fuzz
  `Cargo.toml`; the fuzz crate is outside the workspace and is not built by CI).

## 6. Documentation

- `Docs/IPC_PROTOCOL.md` §2 pointer, §5 fuzz target list, new §12 (message tag and
  classification) and §13 (atomic reservation).
- `Docs/POLICY_CRATE.md`: the attempt is recorded before the loop, not after it.
- `crates/protocol/fuzz/README.md`: new target.

## 7. Behavior change to note

Every admitted `Auth` request now consumes an attempt, including requests that end before the
vision work (not enrolled, camera unavailable). With the default 5 attempts per 60 s this only
matters for repeated failures, which fall back to the password anyway.

## 8. Follow-ups

- Run the new libFuzzer target (`cargo +nightly fuzz run decode_client_message`) on a host with
  `cargo-fuzz`; it was not compiled here because the fuzz crate is not a workspace member.
- Once every deployed client tags its frames, a later change may stop accepting untagged legacy
  frames entirely (removes the last double decode).
