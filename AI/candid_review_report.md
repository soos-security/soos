# Candid Review Report

- **Date**: 2026-09-21
- **Target Branch / Commit**: `docs/nextgen-model-documentation`
- **Audited Files**:
  - `AI/ARCHITECTURE.md` (§1, §7, footnotes)
  - `AI/BACKLOG.md` (Issue #45 sub-issues, Phase 18 NGM table)
  - `AI/VERIFICATION_MATRIX.md` (PAD1, PAD6, NGM17)
  - `AI/plan_evaluator_report.md` (new)
  - `AI/walkthroughs/63_nextgen_model_documentation.md` (new)
  - `Docs/INFERENCE_ORT_CRATE.md` (overview items 1–2, FaceDetector description)
  - `Docs/VISION_CRATE.md` (§2.2, §2.3 headers)
  - `scripts/sync_issue.py` (issue/branch mappings)

---

## 1. Executive Summary

This PR is a **documentation-only update** synchronizing five project artefacts with the 3-model AI pipeline implemented in Issues #36–#44. No production Rust code, no test modifications, no Cargo dependency changes. All changes are additive documentation amendments.

The changes correctly replace the legacy 4-model pipeline description (UltraFace Slim 320 + separate landmark_5point + MobileFaceNet 128D + MiniFASNet) with the current 3-model architecture (SCRFD 500M KPS unified detection+landmarks + ArcFace w600k MBF 512D + MiniFASNetV2 80×80).

---

## 2. Deep Reasoning Audit

### Logic & Architecture

- **[Pass]** `AI/ARCHITECTURE.md` §1 table correctly replaces the inference row. The prohibition column now explicitly mentions "separate landmark model (absorbed into SCRFD)" — preventing regressions if new AI sessions are proposed.
- **[Pass]** §7 latency budget is internally consistent: 5 + 40 + 30 + 35 + 20 = 130ms total, well under the 150ms deadline, with correct margin for OS scheduling.
- **[Pass]** The footnotes correctly add academic references for SCRFD (ICLR 2022), ArcFace (CVPR 2019), and MiniFASNetV2 (CVPR 2019). Old `[^ultraface]` and `[^mobilefacenet]` footnotes removed.
- **[Pass]** `AI/VERIFICATION_MATRIX.md` NGM17 criterion is consistent with the ADR register in `AI/DECISIONS.md` and the Phase 18 table in `AI/BACKLOG.md`.
- **[Pass]** PAD1 update correctly specifies MiniFASNetV2 details: model ID `minifasnet_v2_pad`, 80×80 BGR input, `[1, 3]` output, `live_class_index = 0`, Apache-2.0 license.
- **[Pass]** PAD6 latency budget reference updated from 35ms to 30ms, consistent with the new §7 budget table.
- **[Pass]** `scripts/sync_issue.py` mappings (`45: 111` and `"docs/nextgen-model-documentation": 45`) are consistent with the GitHub issue number referenced in the backlog and the branch name.
- **[Observation]** `AI/BACKLOG.md` historical entries for Issues #6, #19 retain references to legacy model names (UltraFace, MobileFaceNet) — these are correct as historical problem statements and MUST NOT be edited.

### PAM Concurrency & Deadlines

- **[Pass]** Zero PAM source code modified. No impact on `crates/pam/`.
- **[Pass]** No new async operations, no Tokio usage, no socket operations introduced.
- **[Pass]** No `println!`, `eprintln!`, or `dbg!` added.

### Panic Safety & Fallback

- **[Pass]** Documentation-only change. No code modifications means no new panic surfaces.
- **[Pass]** The walkthrough correctly documents that no security invariants are affected.

### Test Integrity & Anti-Weakening

- **[Pass]** Zero tests modified. The VERIFICATION_MATRIX.md updates only reflect evidence already verified in previous PRs (#36–#44).
- **[Pass]** NGM17 criterion is defined as a documentation audit, appropriately cross-referencing production tests (`test_enrollment_cli_model_ids_match_manifest`, `test_enrollment_cli_legacy_model_ids_absent`) as indirect validation.
- **[Pass]** No acceptance criteria weakened.

### Memory & Secret Bounds

- **[Pass]** Not applicable — documentation only.
- **[Pass]** No sensitive data (passwords, embeddings, keys) mentioned or introduced in documentation in ways that could leak.

---

## 3. Detailed Findings & Action Items

- **[SUGGESTION]** `AI/ARCHITECTURE.md:189` — "ArcFace w600k MobileFaceNet ONNX" could be confusing. The model is more precisely described as "ArcFace-trained w600k MobileFaceNet backbone ONNX". This is technically accurate (MobileFaceNet is the backbone, w600k is the training set, ArcFace is the loss function). No action required — the naming matches `AI/DECISIONS.md` ADR entry exactly.

No critical, major, or minor blocking findings.

---

## 4. Final Verdict

**VERDICT: APPROVED**

All documentation changes are accurate, consistent with the ADR register (`AI/DECISIONS.md`), and faithful to the implementation verified in Issues #36–#44. Zero production code modified. Zero test integrity violations. English policy fully respected across all five artefacts and the walkthrough. The PR is ready for autonomous merge.
