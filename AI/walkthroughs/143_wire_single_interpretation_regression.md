# Walkthrough 143 — Wire Format Single-Interpretation Regression Contract

- **Date**: 2026-09-30
- **Issue**: GitHub #224 (review finding PAM-12)
- **Branch**: `fix/p2-protocol-codec`
- **Matrix criteria**: PCX1–PCX4 (new component `wire-single-interpretation-regression`)
- **ADR**: none new; ADR 2026-09-30 "Strict Codec Accepts Only the Matching Client Message Tag Trailer" already records the design

---

## 1. Finding and Current State

PAM-12 reported three problems against base `fbb99c4`:

| Point | Reported | State on `main` (`abd6016`) |
|---|---|---|
| Client-side lenient decoding | `decode_with_limit` ended with `postcard::from_bytes`, accepting a valid `Allow` followed by garbage inside the declared length | Fixed: `codec::decode_payload` uses `take_from_bytes` and rejects any remainder with `CodecError::TrailingBytes` (matrix PCZ1–PCZ2, walkthrough 123) |
| Two decoders of different strictness | The daemon used its own strict `take_from_bytes` path | Fixed: the dispatcher classifies through `message::decode_client_message`, which decodes through `codec::decode_payload_exact`; no crate outside `soos-protocol` calls `postcard` (invariant `protocol_codec_contract::test_no_crate_decodes_wire_payloads_with_postcard_directly`) |
| No message-type discriminator; `uid_hint == peer.uid` heuristic | A root peer with `uid_hint != 0` could see an ambiguous Request consumed as an Event | Fixed by GitHub #204: every in-tree client (`pam_soos.so`, `soos-admin`, `soos-gui`) sends `encode_request` / `encode_event` frames ending with a one-byte tag trailer; the heuristic is gone; untagged legacy frames are accepted only when they decode as exactly one type (matrix BBX1–BBX4) |

The only #224 item still open in the matrix was PCZ6 (the discriminator), deferred to a
protocol v2 `enum Message` envelope. The #204 trailer delivers the discriminator without a
version bump, so no production change was needed. What was missing was the evidence the
review asked for: a test expressing "exactly one interpretation".

## 2. Specification

No type, error or constant changes. The contract pinned by the new tests:

1. A frame from `encode_request` decodes with `codec::decode::<Request>` and fails with
   `decode::<Event>`, `decode::<Response>` and `decode::<StatusResponse>`; symmetrically for
   `encode_event`.
2. `Response` and `StatusResponse` frames never decode as each other (a `StatusResponse` is at
   most 20 bytes, a `Response` at least 37, and the decoder is strict).
3. The tagged form of the PAM-12 collision is classified as a `Request`; a tagged `Request`
   with an unknown extra field is `MessageError::Malformed`, never re-routed.
4. `decode::<Response>` tolerates no remainder at all: the client tag tolerance of
   `decode_payload` never reaches the PAM client, which returns `PAM_IGNORE`.
5. The daemon answers that tagged collision from a peer whose UID differs from `uid_hint`.

## 3. Tests (all new files, no pre-existing test touched)

| File | Tests |
|---|---|
| `crates/protocol/tests/wire_exclusivity_tests.rs` | `prop_pcx_tagged_request_decodes_as_no_other_type`, `prop_pcx_tagged_event_decodes_as_no_other_type`, `prop_pcx_response_and_status_response_are_mutually_exclusive`, `prop_pcx_response_rejects_every_single_trailing_byte` (512 cases each), `test_pcx_tagged_ambiguous_request_has_exactly_one_interpretation`, `test_pcx_tagged_request_with_unknown_extra_field_is_malformed_not_event`, `test_pcx_response_with_any_client_tag_is_rejected` |
| `crates/pam/tests/pcx_client_tag_tolerance_tests.rs` | `test_pcx_allow_followed_by_a_client_tag_returns_ignore`, `test_pcx_direct_authenticate_rejects_allow_with_a_client_tag` |
| `crates/daemon/tests/pcx_wire_routing_tests.rs` | `test_pcx_tagged_ambiguous_request_with_foreign_uid_hint_gets_a_response` |

### Red evidence (mutation checks)

The tests pass on `main` because the fix is already there. To prove they bite, the
production code was mutated locally and reverted:

- `decode_payload` tolerating any single byte `>= 0x80` (tag tolerance leaking to every type):
  4 of 7 protocol tests fail and both PAM tests fail (the tampered `Allow` is accepted).
- `decode_payload` restored to the pre-#224 lenient `postcard::from_bytes`: the same 4 protocol
  tests fail.
- `message.rs` `decode_exact` made lenient: `test_pcx_tagged_request_with_unknown_extra_field_is_malformed_not_event` fails.

## 4. Audit

Test-only change. No production code, no `unwrap` outside `#[cfg(test)]`/test crates, no new
dependency, no logging, no socket permission change. PAM tests use temporary sockets under
`tempfile` directories and assert `PAM_IGNORE`, never `PAM_SUCCESS`.

## 5. Open Points

- Untagged legacy frames are still accepted when they decode as exactly one type (pinned by
  the pre-existing `wire_discriminator_tests::test_204_legacy_untagged_status_request_still_served`).
  All in-tree clients tag their frames; dropping legacy acceptance is a separate decision and
  would require changing that pre-existing test.
- Matrix row PCZ6 still reads "Pending"; this walkthrough and PCX1–PCX4 record its closure
  (shared-file edits are append-only in this batch).
