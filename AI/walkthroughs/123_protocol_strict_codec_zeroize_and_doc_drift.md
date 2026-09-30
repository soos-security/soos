# Walkthrough 123 — Strict Codec Decoding, Single-Buffer Encoding and IPC Doc Drift

- **Date**: 2026-09-30
- **Issues**: GitHub #224 (PAM-12), #225 (PAM-13), #226 (PAM-14)
- **Branch**: `fix/p2-protocol-codec-zeroize-docs`
- **Matrix criteria**: PCZ1–PCZ5 (new)
- **ADR**: 2026-09-30 "Strict Codec v1 Decoding; Message Discriminator Deferred to Protocol v2"

---

## 1. Findings

1. **PAM-12 (#224)**: `decode_with_limit` ended with the lenient `postcard::from_bytes`, so a
   valid `Allow` response followed by garbage inside the declared length decoded as `Ok(Allow)`
   on the PAM client, while the daemon used `take_from_bytes` and rejected leftover bytes. The
   v1 wire format also has no message-type discriminator: the daemon decodes each payload as
   both `Request` and `Event` and breaks ties with `uid_hint == SO_PEERCRED uid`.
2. **PAM-13 (#225)**: `encode_with_limit` serialized into an intermediate
   `postcard::to_allocvec` buffer, copied it into the frame and dropped it without zeroization,
   leaving the request nonce in freed heap of the login process.
3. **PAM-14 (#226)**: `Docs/IPC_PROTOCOL.md` named `CodecError::PayloadCorrupted`,
   `PROTOCOL_VERSION` and `ZeroizeOnDrop` (none exist), omitted `StatusResponse`,
   `PreviewResponse`, `MAX_PREVIEW_MESSAGE_SIZE` and `encode_preview` / `decode_preview`; the
   protocol crate diagram drew fixed widths (`uid_hint:u32`, `service_len:u8`, `*_ns:u64`)
   although postcard uses varints; `crates/pam/src/lib.rs` still stated a 200–250 ms budget;
   `tests/docker/mock_daemon.py` sets `REASON_SCORE_BELOW_THRESHOLD = 1`, which is
   `ReasonClass::NoFace` (`ScoreBelowThreshold` is 3).

## 2. Red evidence (tests written first)

- `cargo test -p soos-protocol --test strict_codec_tests` failed to compile:
  `unresolved import soos_protocol::codec::decode_payload` and
  `no variant named TrailingBytes found for enum CodecError`.
- `cargo test -p soos-invariants protocol_codec_contract`: 0 passed, 8 failed, among them
  `the codec must not use the lenient postcard::from_bytes`,
  `codec.rs must not serialize through to_allocvec`,
  `Docs/IPC_PROTOCOL.md names PayloadCorrupted`,
  `crates/protocol/src/lib.rs draws uid_hint:u32 as a fixed width`,
  `crates/pam/src/lib.rs restates the obsolete fixed PAM deadline 200–250`,
  `wire payloads must be decoded through soos_protocol::codec ... ["crates/daemon/src/dispatcher.rs"]`.
- `crates/pam/tests/strict_decode_tests.rs` depends on `CodecError::TrailingBytes` and did not
  compile either; on the old codec its PAM-level scenario is the review's scratch repro
  (`Allow` + 3 bytes decoded as `Ok(Allow)`).

## 3. Implementation

- `crates/protocol/src/codec.rs`
  - New `CodecError::TrailingBytes { unconsumed }` and `pub fn decode_payload<T>(&[u8])`
    (`postcard::take_from_bytes`, non-empty rest rejected). `decode_with_limit` calls it after
    the unchanged prefix and bound checks. Bytes after the declared frame are still ignored, so
    callers passing a larger read buffer keep working.
  - `encode_with_limit` computes the size with `postcard::serialize_with_flavor(msg,
    ser_flavors::Size::default())`, rejects oversize before allocating, allocates
    `vec![0; 4 + len]` once, writes the prefix and serializes in place with
    `postcard::to_slice`. A size mismatch or serialization error zeroizes the partial frame and
    fails closed. Wire bytes are identical to the previous encoder
    (`encode_is_length_prefix_followed_by_postcard_payload`).
- `crates/daemon/src/dispatcher.rs`: the two `postcard::take_from_bytes` calls are replaced by
  `decode_payload::<Request>` / `decode_payload::<Event>`; the tie-break is unchanged.
- Docs: `Docs/IPC_PROTOCOL.md` §2 rewritten from the code (postcard varint widths, both size
  limits, every codec error, strict decoding, the v1 discrimination limitation, a wire-index
  table of every enum), §3 gains `version` fields, `StatusResponse`, `PreviewResponse` and the
  real zeroization model (manual `Zeroize` + `Drop`), §8 documents the single-buffer encoder.
  `crates/protocol/src/lib.rs` frame diagram and `crates/pam/src/lib.rs` latency line fixed.

## 4. Audit

- No `unwrap` / `expect` / indexing in production code: `split_at_mut_checked`, `get`,
  `checked_add`, `u32::try_from`.
- Allocation is bounded by the limit and happens after the size check (smaller than before:
  one buffer instead of two, no 2 MiB intermediate for previews).
- Errors never become `PAM_SUCCESS`: a trailing-bytes frame maps to `IpcError::Codec` and
  `PAM_IGNORE` (`strict_decode_tests::test_allow_with_trailing_bytes_inside_frame_returns_ignore`).
- `#![forbid(unsafe_code)]` unchanged in `soos-protocol`; no new dependency (`postcard` stays
  declared by `soos-daemon`, now unused there; removing it would touch `Cargo.lock`, left as a
  follow-up).
- No log line added; no secret, frame or embedding exposed.

## 5. Verification

Quality gate: `cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets
--all-features -- -D warnings`, `cargo test --locked --workspace --all-targets --all-features
--no-fail-fast`, `./scripts/candid_review.sh`.

## 6. Not done here (needs a decision)

- **Tagged `Message` envelope (#224)**: changes the first payload byte of every frame, so it
  needs a `CURRENT_VERSION` bump, a coordinated PAM / daemon / admin-cli / gui upgrade and a
  rewrite of the hand-written encoder and request parser in `tests/docker/mock_daemon.py`.
- **`tests/docker/mock_daemon.py` `REASON_SCORE_BELOW_THRESHOLD = 1` → `3` (#226)**: a
  pre-existing Docker test fixture; the change is proposed, not applied, under the test
  integrity rule. The PAM module ignores `reason_class`, so no assertion depends on it today.
- **Matrix PA2 wording** ("> 250ms"): the row is not rewritten here (append-only edits to the
  shared matrix); the enforced rule is ADR 2026-09-30 "PAM Deadline Derived From Clamped
  `timeout_ms`".

## Integration: user-approved fixture fix (2026-09-30)

`tests/docker/mock_daemon.py` `REASON_SCORE_BELOW_THRESHOLD` is now 3 (`ReasonClass::ScoreBelowThreshold`);
1 is `NoFace`.

## Correction (2026-09-30, candid review findings 2 and 7)

PCZ5 is split. The mock fixture part (#226) is `✅ Verified` by the new invariant
`protocol_codec_contract::test_mock_daemon_wire_indices_match_protocol_enums`, which checks every
`REASON_*` / `VERDICT_*` constant of `tests/docker/mock_daemon.py` against the protocol enums (red
with the former value 1). The tagged envelope (#224) moves to PCZ6 and stays pending. Matrix PA2
now states the clamped-`timeout_ms` deadline instead of "> 250ms", so the #226 recommendation is
complete.
