# Candid Review Report

- **Date**: 2026-09-30
- **Target Branch**: `fix/p2-batch-b`
- **Base (merge-base)**: `41d933d` (`origin/main`, PR #283)
- **Reviewed-Diff-Fingerprint**: `9c21946e9d649d1a05056bd6f5c3a0e82257507d60d90d25e3ee4d620acd4b43`
- **Audited Files**: `.github/workflows/fuzz.yml`, `AI/{ARCHITECTURE,DECISIONS,VERIFICATION_MATRIX}.md`, `AI/walkthroughs/{73,108,109,110,121,122,123,124,126,127}_*.md`, `Docs/{BIOMETRIC_STORE_CRATE,CAMERA_V4L_CRATE,DISTRIBUTION_DEPLOYMENT,ENROLLMENT_CLI,EVIDENCE_STORE_CRATE,GUI_APPLICATION,IPC_PROTOCOL,PAM_MODULE}.md`, `crates/admin-cli/{Cargo.toml,src/{args,gdm,main,pam_stack,status,test_pam}.rs,tests/{cli_deadline_json_tests,gdm_followup_tests}.rs}`, `crates/biometric-store/{src/{lib,store,template}.rs,tests/bounded_read_tests.rs}`, `crates/camera-v4l/{src/{capture,config,deep_grey,lib,mock,resolver,sensor,v4l_impl}.rs,tests/{capture_validation_tests,enumeration_tests,hardware_smoke_tests,sensor_hint_classification_tests}.rs}`, `crates/daemon/{src/{dispatcher,lib,preview,preview_image}.rs,tests/preview_downscale_tests.rs}`, `crates/enrollment-cli/{src/{args,html_report,lib,main,service,shred}.rs,tests/{cli_hygiene_tests,device_resolution_hermetic_tests,enroll_fresh_frames_tests,import_stdin_overwrite_tests,list_bounded_tests,shred_tests}.rs}`, `crates/evidence-store/{src/store.rs,tests/{opaque_suffix_tests,permissions_tests}.rs}`, `crates/gui/{src/{app,privileged}.rs,tests/{import_privacy_tests,ipc_camera_failure_tests}.rs}`, `crates/pam/{src/{ipc,lib}.rs,tests/{common/mod,deadline_uid_tests,pam_bindings_tests,pam_feedback_tests,pam_silent_tests,strict_decode_tests}.rs}`, `crates/protocol/fuzz/{.gitignore,Cargo.lock,Cargo.toml,README.md,fuzz_targets/decode_preview.rs}`, `crates/protocol/src/{codec,lib,message,types}.rs`, `crates/protocol/tests/{preview_property_tests,strict_codec_tests,tag_trailer_codec_tests}.rs`, `tests/docker/mock_daemon.py`, `tests/invariants/src/{camera_docs_contract,cli_hygiene_contract,enroll_list_metadata_contract,lib,pam_feedback_contract,pam_handle_guard_removal_contract,pam_response_freshness_contract,protocol_codec_contract}.rs`

## 1. Executive Summary

`fix/p2-batch-b` combines two branches that each passed a full candid review:

| Branch | Tip | Report | Fingerprint | Verdict |
|---|---|---|---|---|
| `fix/p2-camera-batch` | `8dcbd55` | `wf_73c3a8b4-111-1/AI/candid_review_report.md` | `35ecf354c9a52684fccec4beae601e5d5d6498b8af82c3569b40cd53e7197366` | APPROVED |
| `fix/p2-pam-cli-batch` | `7a7cd72` | `wf_73c3a8b4-111-5/AI/candid_review_report.md` | `311a7cd8507ca11f5a2456af403eea7fd6062ad4c9b920a53f6814515987d81f` | APPROVED |

Both tips are ancestors of `HEAD` (`git branch --contains`). They were merged (`aefb23d`, `fe166cc`),
then approved `origin/main` `41d933d` (PR #283) was merged in (`0104c24`), and `83c02fd` resolves the
interaction of the strict codec (#224, from the pam-cli batch) with the client message tag trailer
(#204, from main). This review covers only that combination delta. The two approved per-branch
diffs were not re-reviewed.

Outcome: no CRITICAL or MAJOR finding. Every conflict resolution keeps the fixes from both sides.
`83c02fd` keeps PAM and CLI response decoding fully strict and accepts exactly one trailing byte,
only the tag of the decoded client type. One MINOR documentation finding: three facts from main
were lost in the hand-merge of `Docs/IPC_PROTOCOL.md`.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- `grep '^-… assert|#[test]…'` on the frozen patch lists only `crates/enrollment-cli/tests/shred_tests.rs`
  (deleted with `src/shred.rs`, user-approved item (d) in the pam-cli report) and the args-string
  assertion of `crates/gui/tests/import_privacy_tests.rs` (approved item (c) in the same report).
  No other assertion was removed.
- New escape hatches: `#[ignore]` + `SOOS_HW_TESTS=1` on the camera hardware tests (approved in the
  camera report), and one `prop_assume!` added by `83c02fd` (see below). No `#[cfg(any())]`,
  `should_panic`, tolerance or epsilon was added.
- **Tests that exist on `origin/main` (41d933d)**: for every file in `41d933d..HEAD`, I rebuilt the
  three-way merge with `git merge-file` (base `e2f602c` = merge-base of the batches and main, ours
  `fe166cc`, theirs `41d933d`) and compared it with `0104c24` and then with `HEAD`. Every test file
  that exists on main merged without conflict and is byte-identical to the automatic merge, and `83c02fd`
  did not touch any of them. Their delta against main is therefore exactly the approved batch delta
  (for example, `tests/docker/mock_daemon.py`: only the approved `REASON_SCORE_BELOW_THRESHOLD = 3`).
  The only conflicted test file is `tests/invariants/src/lib.rs`, a union of module declarations.
  The `--union` re-merge matches it line for line, apart from blank lines.
  The pre-existing main test `client_message_tests::test_tagged_frame_stays_readable_by_v1_decoders`
  is unchanged. It failed after the combination and passes now because of a production fix, not a test edit.
- **Branch-new tests changed by `83c02fd`** (introduced by the approved pam-cli batch, absent from main):
  - `crates/protocol/tests/strict_codec_tests.rs::prop_request_strict_roundtrip`: adds
    `prop_assume!(extra != [MESSAGE_TAG_REQUEST])`. This excludes exactly one input, and that input
    is now specified as accepted by #204 on main ("a v1 reader `decode::<Request>` ignores the
    trailer"). Rejection of every other suffix still holds: 1..=64 random bytes, and `[0xA0, x]`
    passes the assumption because it is two bytes. The excluded case is pinned positively by
    `tag_trailer_codec_tests` and its neighbours negatively (BBX1–BBX3). Justified. Not a weakening of the #224 intent.
  - `tests/invariants/src/protocol_codec_contract.rs`: `dispatcher.rs contains "decode_payload"` becomes
    `dispatcher.rs contains "decode_client_message"`, plus a new assertion that production
    `message.rs` uses `decode_payload_exact` and not `postcard::take_from_bytes`. The old assertion
    could not survive the kept #204 dispatcher path. The replacement still binds the daemon to the
    strict protocol decoders and adds a check on the classifier body. Justified. Equivalent or stronger.
- **New test file** `crates/protocol/tests/tag_trailer_codec_tests.rs` has 10 tests and 1 proptest. They would fail on
  plausible wrong implementations: "accept any single byte" fails BBX2 (all 255 non-tag bytes, cross-type tags);
  "accept any tag after any type" fails `test_response_never_accepts_a_client_message_tag`;
  "strip up to one tag, then lenient" fails BBX3 and the doubled-tag test.

## 3. Deep Reasoning Audit

### Logic & Architecture
- **Dispatcher conflict (`0104c24`)**: main's `decode_client_message` classification (#204 / DMN-15) is kept.
  The batch's `uid_hint == peer.uid` double-decode heuristic is removed. `CodecError` stays imported and is still
  used (`dispatcher.rs:1269`). The batch's other #224 dispatcher hunks merged cleanly. The earlier
  camera × pam-cli conflict (`fe166cc`) was only the `use` line, resolved as the union. PASS.
- **`crates/protocol/src/lib.rs`**: the doc comment merges both sides (strict decoding + tag trailer). PASS.
- **Fuzz** `Cargo.toml` / `README.md` / `fuzz.yml`: both the `decode_preview` (#227) and `decode_client_message`
  (#204) bins are declared and both target files exist, and the nightly matrix now runs all five targets. PASS.
- **Docs**: the `CAMERA_V4L_CRATE.md` resolution keeps the batch's corrected defaults (`idle_timeout` 10s,
  `auto_format` true, `PreferIr`, S_PARM text) over main's stale list. The camera docs invariant
  parses `warmup_frames` (default: 20). `EVIDENCE_STORE_CRATE.md` keeps the batch's `.opaque.enc`
  layout plus legacy `.webp.enc`. The `IPC_PROTOCOL.md` hand-merge keeps main's "Request kinds" table,
  persistent-connection loop, §12 tag trailer, §13 rate-limit reservation, the fuzz-target line and the
  §2 tag sentence. It keeps the batch's #226 rewrite (constants in §2, `ValidationError::ServiceTooLong`,
  typed `StatusResponse`/`PreviewResponse`). The stale "v1 limitation / uid_hint" discrimination text is
  correctly replaced. Three facts from main were lost, see Finding 1. `DECISIONS.md` and the matrix
  match the `--union` re-merge, apart from the corrected QFU5 wording (default 2500 ms, consistent with main's
  `DEFAULT_CONNECTION_TIMEOUT_MS`).
- **`83c02fd` `decode_payload`**: `match rest { [] => Ok, [tag] if Some(*tag) == client_message_tag::<T>() => Ok, _ => TrailingBytes }`.
  - Can a `Response` ever accept a trailing byte? No. `client_message_tag::<Response>()` is `None`, so
    `Some(b) == None` never holds. The same applies to `StatusResponse` and `PreviewResponse`, and to any wrapper type (the
    `TypeId` differs), which fails closed. PASS.
  - Can arbitrary trailing bytes slip through? Only a remainder of exactly length 1 equal to the `T`'s
    tag is accepted. Cross-type attempt: a tagged `Event` decoded as `Request` always leaves a remainder ending in
    `0xA1`, never `[0xA0]`. The remainder cannot be empty either: postcard would have to consume `0xA1` as a
    varint continuation byte (high bit set) and would then need more input. PASS.
  - `decode_client_message` → `decode_exact` → `decode_payload_exact` with zero remainder, so
    `body || A0 || A0` strips one tag and fails on the second: `Malformed` (`test_client_message_with_a_doubled_tag_is_rejected`).
    Legacy frames also go through the exact decoder. PASS.
  - **PAM client strictness**: production callers of `codec::decode` are `pam/src/ipc.rs:483`
    (`Response`), `admin-cli/src/test_pam.rs:187` (`Response`), `admin-cli/src/status.rs:219`
    (`StatusResponse`) and `gui/src/ipc_camera.rs` (`Response`, `PreviewResponse`). They are all never-tagged types, so
    they stay fully strict. No production code decodes a `Request`/`Event` through `decode_payload`. The relaxation
    serves only mock servers, fuzz targets and v1 compatibility. PASS.
  - **`T: 'static` bound** on `decode`/`decode_preview`/`decode_with_limit`/`decode_payload`: every wire
    type is owned. `DeserializeOwned` already rules out borrowed deserialization. The workspace compiles and
    clippy passes. It is a public API tightening with no in-tree breakage. PASS.
  - Tag constants have a single source (`message::MESSAGE_TAG_*`, imported by `codec`). PASS.

### PAM Concurrency & Deadlines
Not touched by the delta. `pam/src/ipc.rs` only merged cleanly. No new blocking call. PASS.

### Panic Safety & Fail-Closed
`client_message_tag` and the new match contain no `unwrap`, indexing or panics. A slice pattern is not
indexing. Every rejection is a `CodecError` / `MessageError`, and the daemon closes the connection without a
handler. No new path leads to `Allow`/`PAM_SUCCESS`. PASS.

### Test Integrity & Anti-Weakening
See §2. The two branch-new test edits are forced by an acceptance criterion from main (#204) and are
compensated by stricter pinned tests. No test that exists on main was modified beyond the approved batch sets. PASS.

### Memory, Bounds & Secrets
There are no new allocations in the decode path (`take_from_bytes` on the bounded payload slice). `encode_tagged`
is unchanged (`Zeroizing` payload, trailer counted in `MAX_MESSAGE_SIZE`). Nothing is logged. PASS.

### Supply Chain & Automation
`fuzz.yml` only adds a matrix entry: no new action, no interpolation in `run:`, permissions unchanged.
The fuzz `Cargo.lock` comes from the approved batch. PASS.

### English-Only Policy
All code, comments, docs, ADR and matrix text in the delta are English. PASS.

**Execution evidence** (this review):
- `cargo test --locked -p soos-protocol -p soos-invariants --all-features`: all suites green (0 failed).
- `cargo test --locked -p soos-pam -p soos-daemon -p soos-admin-cli`: all suites green. These suites held
  most of the 43 combination failures.
- `cargo clippy --locked -p soos-protocol --all-targets --all-features -- -D warnings`: clean.
- Docker was not run (per instructions).

## 4. Detailed Findings & Action Items

1. **[MINOR]** `Docs/IPC_PROTOCOL.md:100`, §`StatusResponse` / §`PreviewResponse`: the hand-merge
   dropped three facts that main (#283) had added:
   - the `connection_timeout` default (`DEFAULT_CONNECTION_TIMEOUT_MS` = 2500 ms, "so the GDM line
     `timeout_ms=2500` gets its full budget while console/sudo stays capped by its 1000 ms PAM deadline");
   - "It carries no frame, template, embedding or UID data" (`StatusResponse`);
   - "the struct zeroizes on drop" (`PreviewResponse`).

   The first fact is still documented in `Docs/DAEMON.md` and `Docs/DISTRIBUTION_DEPLOYMENT.md`, and no invariant
   depends on these sentences, so this does not block. Suggested follow-up: re-add them to the batch-side
   typed field descriptions.

No CRITICAL or MAJOR findings.

## 5. Final Verdict

**VERDICT: APPROVED**
