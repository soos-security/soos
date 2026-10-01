# Plan Evaluation Report

- **Date**: 2026-10-01
- **Issue**: GitHub #278 — SFace embedding model replaces ArcFace ResNet34, `match_threshold = 0.50` (owner decisions 2026-10-01; no backlog id)
- **Branch**: `feat/sface-embedding-model`
- **Base commit**: `11c967e`
- **Plan evaluated**: architect spec of `AI/walkthroughs/162_sface_embedding_model.md` §3 with the owner answers Q1–Q11 of 2026-10-01

## 1. Coverage Matrix

| Owner decision / acceptance line | Spec element | Status |
|---|---|---|
| Adopt SFace under a new manifest id, pinned revision, Apache-2.0 file | §3.2 manifest entry, §3.9 retired file | Covered (SFC1) |
| Existing templates become Foreign → `Unavailable` → password fallback | §3.4 binding by id and dimension, alias removed, §3.8 | Covered (SFC7–SFC10) |
| Never a cross-model comparison | §3.4 daemon 8d, `verify`, GUI, import, migrate | Covered (SFC7, SFC9, SFC13–SFC15) |
| `match_threshold` default 0.50, floor 0.40 (Q9) | §3.5 | Covered (SFC11, SFC12) |
| Q2 retired attestation file | §3.9 `models/retired_models.toml` | Covered (SFC1, SFC17) |
| Q3 alias removal | §3.4 | Covered (SFC8) |
| Q4 import refusal, migrate / list notice | §3.6 | Covered (SFC13, SFC14) |
| Q5 verify / GUI refusal before capture | §3.4, §3.7 | Covered (SFC15; GUI by code review, no GUI test harness for the panel) |
| Q6 manifest version 2.0.0 | §3.2 | Covered (`model_id_tests`) |
| Q7 mock default 128 | §3.3 | Covered (SFC4) |
| Q8 unattested-file notice | §3.9 | Covered (SFC17) |
| Q11 ORT warning noise, Error only for that session | added §3.3 registry `ERROR_ONLY_LOG_MODELS` | Covered (SFC18) |
| Training-data risk recorded as owner-accepted | §3.13 ADR | Covered (SFC16) |

## 2. Facts Verified Against Code and the Real File

| Fact cited by plan | Location | Actual value | Match |
|---|---|---|---|
| SFace input / output | real file, ORT 1.20 and `embedding_real_model_tests` | `data [1,3,112,112]`, `fc1 [1,128]` | yes |
| In-graph normalization | real file protobuf | `Sub(127.5)`, `Mul(0.0078125)` | yes |
| OpenCV recipe | `face_recognize.cpp` | `blobFromImage(.., 1, 112x112, 0, swapRB=true)` | yes |
| Alignment template | `crates/vision/src/align.rs` vs `face_recognize.cpp` | identical 5 points | yes |
| SHA-256 / size | `sha256sum`, HF `x-linked-etag` / `x-linked-size` | `0ba9fbfa...4c34e79`, 38,696,353 | yes |
| Import model default | `crates/enrollment-cli/src/args.rs:200` | literal `"arcface_w600k_mbf"` (GUI relies on it) | finding F2 |
| Import outcome dimension | `service.rs` `store_imported` | hard-coded `embedding_dim: 512` | finding F3 |
| GUI threshold | `crates/gui/src/app.rs:807` | literal `0.70f32` | finding F4 |
| Dimension mismatch in daemon | `dispatcher.rs` consensus arm | cosine error → score `0.0` → `Deny` | finding F5 |
| Rate limit | `dispatcher.rs` 8-pre | every request reserves an attempt before 8d | noted (walkthrough corrected) |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario: an ArcFace template labelled `mobilefacenet` / `1.0.0` keeps the alias and is compared with SFace probes. Result: the alias is removed (F1).
- Result: PASS after F1.

### Pillar 2 — PAM deadline & concurrency
- Failure scenario: a slower model breaks the budget. SFace is about 3.5x faster than ArcFace; no PAM change. PASS.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario: new binary with the old installed manifest. `get_or_load_session("sface_2021dec")` fails, the daemon refuses to start, PAM returns `PAM_IGNORE`. PASS.
- Failure scenario: a 512-D vector labelled `sface_2021dec` (old mock or forged import) reaches the cosine. Dimension binding refuses it (`Unavailable`), import refuses it. PASS after F5.

### Pillar 4 — Dependencies
- No new crate; `ort` logging API already in `ort 2.0.0-rc.13`. PASS.

### Pillar 5 — Data confidentiality
- New messages carry model ids, UIDs and dimensions only; the migrate summary JSON carries UIDs only. PASS.

### Pillar 6 — Test integrity
- Every pre-existing change is in the owner-approved list (walkthrough 162 §4). One additional test was found during Phase 4 and was **not** edited (finding F6). PASS with F6 reported.

## 4. Findings

- **[CRITICAL] F1** Keeping the legacy alias with SFace loaded would compare ArcFace vectors with SFace probes — the plan removes it (owner Q3).
- **[MAJOR] F2** `import --model-id` default literal would label every GUI enrollment ArcFace — the plan derives it from `MODEL_ID_EMBEDDING`.
- **[MINOR] F3** `store_imported` reports a hard-coded 512 — fixed to `IMPORT_EMBEDDING_DIM`.
- **[MAJOR] F4** GUI verdict literal 0.70 would disagree with the 0.50 default — removed, pinned by SFC12.
- **[MAJOR] F5** Id-only binding scores a wrong-length vector as `Deny` instead of refusing it — dimension binding added.
- **[MINOR] F6** `import_json_presize_tests::test_import_json_decodes_into_a_buffer_that_never_grows` uses the literal 257 as "a length below the import dimension"; with 128 it fails. Not in the approved list: reported to the owner, not edited.

## 5. Verdict
VALIDATION_VERDICT: APPROVED
