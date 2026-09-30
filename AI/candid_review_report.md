# Candid Review Report

- **Date**: 2026-09-30
- **Target Branch**: `fix/p2-batch-a`
- **Base (merge-base)**: `e2f602c`
- **Reviewed-Diff-Fingerprint**: `61e6fd6f215fcad8542c4a4107e416062b1cfa76f263433dcefb9a925e14172f`
- **Audited Files**: the 222 files of `target/candid_diff.patch` (merge-base `e2f602c` to the working tree at `4875556`, clean tree apart from this report). The prior approval (fingerprint `476cc146...82d1`, tree at `e37f8e2`, 221 files) is kept below; the only change since is commit `4875556` (`tests/docker/mock_daemon.py`), reviewed in §3 "Mock daemon tag delta". This review covers the **combination delta** in depth: the merge resolutions of `ba3139c`, `5e19432`, `a7855c9` and commit `e37f8e2` (`AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `Docs/DAEMON.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md`, `Docs/README.md`, `tests/invariants/src/quality_followups.rs`), plus the multi-owner files `.agents/skills/dev-workflow/references/project-facts.md`, `AI/ARCHITECTURE.md`, `Cargo.toml`, `Cargo.lock`, `crates/daemon/src/config.rs`, `crates/daemon/src/dispatcher.rs`, `crates/daemon/src/pipeline.rs`, `Docs/MEMORY_PROTECTION_AND_SWAP.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `packaging/soos-daemon.service`, `scripts/download_models.sh`, `tests/invariants/src/lib.rs`. Every other file is byte-identical to an already approved batch head (verified below).

## 1. Executive Summary

`fix/p2-batch-a` = `origin/main` (`e2f602c`) + three batch branches merged with `--no-ff`, plus one reconciliation commit. Each batch was approved by a full candid review against the same merge-base `e2f602c`:

| Batch branch | Head | Approved fingerprint | Report |
|---|---|---|---|
| `fix/p2-daemon-storage-batch` | `1c7419c` | `84f5843c5a7c16d342dc0e7f0ecc72d6430601e8952ca6dbe139149864329fc4` | `wf_73c3a8b4-111-2/AI/candid_review_report.md` — VERDICT: APPROVED |
| `chore/p2-install-quality-batch` | `205efed` | `272640caa3ad7988f9a2a617564557ad8dbc2ea0a0c5ab37e45eb6c7f99e6051` | `wf_73c3a8b4-111-3/AI/candid_review_report.md` — VERDICT: APPROVED |
| `fix/p2-vision-pad-batch` | `8c3d512` | `a1bbf2ba1e4f48ba75a030b89fdd4ca65d9578d02f0ad7f1f892b8eb9fa5724a` | `wf_73c3a8b4-111-4/AI/candid_review_report.md` — VERDICT: APPROVED |

The branch refs merged here (`1c7419c`, `205efed`, `8c3d512`) are exactly the heads checked out in the three approved worktrees. The combination delta is small: six conflict resolutions that are unions of both sides (or a strict superset of a removal), and a documentation/contract reconciliation to the user-decided 2500 ms daemon connection timeout. No fix from any batch was dropped, the workspace builds, and every quality gate passes. One MINOR stale matrix wording remains. Verdict: APPROVED.

## 2. Test Changes (mechanical listing)

Pre-existing (on `origin/main`) test files modified by the combined diff:
`crates/admin-cli/src/test_pam.rs`, `crates/daemon/tests/{config_tests,hardening_tests,systemd_test}.rs` (daemon batch); `crates/daemon/tests/pad_wiring_tests.rs`, `crates/enrollment-cli/tests/pad_wiring_tests.rs`, `crates/vision/tests/pad_tests.rs`, `tests/fixtures/mod.rs`, `tests/invariants/src/matrix_citations.rs`, `run_tests.sh`, `tests/docker/*` (install/quality batch); `crates/inference-ort/tests/{landmark_tests,landmarks_tests,zeroize_tests}.rs` (vision batch); `tests/invariants/src/lib.rs` (all three).

- Each of these files except `tests/invariants/src/lib.rs` is touched by exactly one batch and is **byte-identical** at `HEAD` to that batch's approved head (`git diff --quiet <batch> HEAD -- <file>` for every single-owner file). Their changes were listed and justified in the corresponding approved report (each file name appears in the owning report's test-change section).
- `tests/invariants/src/lib.rs`: the resolution only unions the `mod` declarations of the three batches (`daemon_docs_contract`; `onboarding_packaging_contract`, `installer_templates_contract`, `maintainer_hygiene_contract`, `fixtures_contract`, `lexing_contract`, `candid_review_contract`, `quality_followups`, `toolchain_pin_contract`, `licence_contract`, `dependency_tooling_contract`; `pad_startup_contract`, `vision_attestation_contract`, `vision_threshold_contract`, `ort_output_zeroize_contract`), each with its original `#[cfg]` gate. No module was dropped (all run: 215 invariant tests).
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`) in the combined diff: none.
- Only test edit made after the approvals: `e37f8e2` changes `quality_followups::test_gdm_timeout_documents_daemon_connection_timeout_cap`. This file/test **does not exist on `origin/main`** (it is a branch-local contract of the install batch, QFU5). Before `e37f8e2` the test could not pass on the combined tree: the daemon batch replaced `connection_timeout: Duration::from_millis(1000)` by `DEFAULT_CONNECTION_TIMEOUT_MS = 2500` (user decision, ADR "Daemon Default Connection Timeout Raised to 2500 ms"). The migration asserts the exact new constant line `pub const DEFAULT_CONNECTION_TIMEOUT_MS: u64 = 2500;` and requires the GDM section to mention `2500 ms` instead of `1000 ms`; the other two needles and the ADR check are unchanged. The assertion is no weaker (it pins the value exactly, and would fail on any other default); it tracks a documented user decision. Justified contract migration, not weakening.

## 3. Deep Reasoning Audit

### Combination delta

1. **No fix dropped (mechanical).** For every file changed by more than one batch (15 files), I re-ran the 3-way merge independently (`git merge-file` of the three batch versions over `e2f602c`) and compared with `a7855c9`: 9 files merge cleanly and are identical to the resolution; the 6 conflicted ones were diffed hunk by hunk. A line-level check also confirmed that every line added by any batch in those files is present in the result.
2. **`Cargo.toml`** — resolution keeps the install batch's exact pin `ort = "=2.0.0-rc.13"` with its comment and applies the vision batch's removal of the `ndarray` workspace dependency. `git grep ndarray` finds no crate manifest using it; `ndarray` stays in `Cargo.lock` only as a transitive dependency of `ort` (`cargo tree -i ndarray`), and `vision_attestation_contract` asserting its absence from `soos-inference-ort` passes. `Cargo.lock` differs from the install head by exactly one line (the removed `ndarray` entry of `soos-inference-ort`); all `--locked` builds succeed.
3. **`crates/daemon/src/pipeline.rs`** — both independent additions are kept verbatim: the daemon batch's `MAX_WARMUP_DIMENSION`, `warm_up_vision_stages`, `blank_rgb`, `warmed_inference_gate` (bounded allocation, results discarded, no sensitive data) and the vision batch's `registry_config_for` (intra-op threads). Both are wired: `main.rs` calls `warmed_inference_gate`; `initialize_pipeline` calls `registry_config_for` and then `validate_pad_detector` (vision batch PAD startup contract). The warm-up uses the post-vision-batch `detector()/pad()/extractor()` APIs and compiles under clippy `-D warnings`. Scenario tried: warm-up running before the PAD output-contract validation could mask a bad PAD model — no, validation happens in `initialize_pipeline`, which fails closed before the gate is built; warm-up errors are ignored and never influence a verdict.
4. **`scripts/download_models.sh`** — resolution is identical to the install side: the whole `resolve_download_url` fallback table is removed (manifest is the single URL source, GitHub #208). The vision batch only deleted four legacy entries of that table, so the result is a strict superset of both removals. `models/manifest.toml` carries direct `.onnx` HTTPS URLs for the three attested models, so no model loses its download path.
5. **`AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`** — pure unions of both sides' ADRs and matrix sections (conflict markers replaced by the `---` section separators only); no ADR or row lost.
6. **`e37f8e2`** — `Docs/DAEMON.md` gains the `inference_intra_threads` key (vision batch key now documented, required by `daemon_docs_contract::test_daemon_doc_documents_every_daemon_toml_key`); `Docs/README.md` indexes `DAEMON.md`; `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 now states the 2500 ms default, ~2450 ms daemon time for GDM and the connection-slot trade-off (accurate vs `config.rs:31,139`); the superseded GDM ADR is annotated rather than rewritten (ADR register stays append-only); DSF3 cites resolvable evidence paths (required by the install batch's `matrix_citations` contract).

### Mock daemon tag delta

Scope: `git log e37f8e2..HEAD` lists exactly one commit, `4875556 test(docker): strip the client frame tag in the mock daemon`; `git diff --stat e37f8e2 4875556` touches only `tests/docker/mock_daemon.py` (+14/-1), and the working tree has no other modification besides this report. The file count of the frozen patch grows from 221 to 222 accordingly.

1. **Constants match the protocol.** `MESSAGE_TAG_REQUEST = 0xA0`, `MESSAGE_TAG_EVENT = 0xA1`, `MESSAGE_TAG_MIN = 0x80` are byte-identical to `crates/protocol/src/message.rs:34,37,41`. The PAM module emits tagged frames only (`crates/pam/src/ipc.rs:359` `encode_request`, `:449` `encode_event`, which append the tag via `encode_tagged`).
2. **Trailer stripping mirrors `decode_client_message`.** The mock strips the last byte iff it is `>= 0x80`, exactly the Rust rule (`message.rs:151`). A complete legacy v1 postcard frame always ends on a varint terminating byte (`< 0x80`) or a fixed-width/UTF-8 byte of the last field; for both the `Request` and `Event` layouts the last field is a varint, so a legacy frame is never stripped and is still classified as before.
3. **No request misclassified as event.** With tag `0xA0`, `classify_event` is not called at all (`event = ... if frame_tag != MESSAGE_TAG_REQUEST else None`), so a tagged Request always reaches the `request` path and receives a reply. Tried: a request body that happens to parse as an Event layout — it is still routed to `request` because the tag short-circuits classification. With tag `0xA1`, the body must still pass the exact-length `classify_event` parse; a malformed event falls back to the `request` path, which only yields a spurious reply to a fire-and-forget sender (harmless for the mock). Request-id extraction (`body[2:34]`) is unaffected because the tag is a trailer, not a prefix.
4. **No assertion changed.** No file under `tests/docker/` other than the mock is modified; `test_suite.sh` T11 still asserts `grep -c '^event kind=password-failed'` (lines 610, 623) and the record line format produced at `mock_daemon.py:270` is unchanged. The fix corrects the mock's frame parsing so the existing T11 assertion observes the event the PAM module already sends; it does not relax any check. The orchestrator reports the Docker PAM matrix T1–T15 re-run green after this commit (not re-executed here, Docker excluded from this review).
5. **Other pillars.** Test-only Python helper: no production code, PAM deadline, panic path, secret or dependency is affected; it records only the classification line, never payload bytes. Comments are English. An unknown tag in `0x80..=0x9F`/`0xA2..` is stripped and then classified (the Rust decoder rejects it with `UnknownTag`); the mock is more lenient but the PAM module never emits such tags, so this has no test impact (SUGGESTION only, finding 2).

Verdict for this delta: **APPROVED** (no CRITICAL/MAJOR finding).

### Logic & Architecture
Scenarios: a merge silently keeping only one side of a conflicted function; the daemon default drifting between code, docs and contract. → PASS (see delta items 1–3, 6). One stale wording, finding 1.

### PAM Concurrency & Deadlines
No PAM crate file is in any conflict or in `e37f8e2`; `crates/pam` is identical to the approved batch versions. Raising the daemon default to 2500 ms does not touch the PAM module deadline (console/`sudo` keep the 1000 ms module default). → PASS.

### Panic Safety & Fail-Closed
Merged `pipeline.rs` code: `blank_rgb` uses `try_from(..).unwrap_or(0)` and saturating arithmetic (no panic); warm-up failure keeps the default estimate. No new path to `Allow`/`PAM_SUCCESS`. → PASS.

### Test Integrity & Anti-Weakening
See §2. No test existing on `origin/main` is modified beyond the approved batches; the single post-approval test edit is on a branch-local contract and is an exact-value migration. → PASS.

### Memory, Bounds & Secrets
Warm-up allocation bounded by `MAX_WARMUP_DIMENSION` (1920² × 3 bytes), all-zero synthetic inputs, nothing logged but latencies. → PASS.

### Supply Chain & Automation
`ort` exact pin kept; one fewer direct dependency (`ndarray`); lock file consistent (`--locked` everywhere). Download script: manifest-only HTTPS URLs, size cap, `mktemp` staging (install batch, unchanged by the merge). → PASS.

### English-Only Policy
All delta text (docs, comments, commit message) is English. → PASS.

### Gates executed on the combined tree
- `cargo fmt --all -- --check` → exit 0
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` → exit 0
- `cargo test --locked -p soos-invariants --all-features` → exit 0, 215 passed, 0 failed
- `cargo test --workspace --locked` → exit 0, 1468 passed, 0 failed, 0 ignored

## 4. Detailed Findings & Action Items

1. **[MINOR]** `AI/VERIFICATION_MATRIX.md:1026` — row QFU5 still describes the contract as "gains no daemon time beyond `[dispatcher] connection_timeout_ms` (default 1000 ms)", while the migrated test and `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 now pin the 2500 ms default. Update the row wording (e.g. "states how `timeout_ms=2500` relates to `connection_timeout_ms` (default 2500 ms)") in a follow-up; it does not affect behaviour or any gate.
2. **[SUGGESTION]** `tests/docker/mock_daemon.py:261` — the mock strips any trailer `>= 0x80`, while `decode_client_message` rejects tags other than `0xA0`/`0xA1` (`UnknownTag`). Optionally drop such frames in the mock for parity; no current test depends on it.

## 5. Final Verdict

**VERDICT: APPROVED**
