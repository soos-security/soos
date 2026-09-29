# Candid Review Report

- **Date**: 2026-09-29
- **Target Branch**: `chore/full-project-review`
- **Base (merge-base)**: `fbb99c4`
- **Reviewed-Diff-Fingerprint**: `f39645898983c12b9830b62798bd8937e700380ca50f06024eccbcee63fcea75`
- **Audited Files**: `AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md` (new file, 2,241 lines; the patch is 2,246 lines and touches nothing else)

## 1. Executive Summary

The diff adds one documentation artifact: the consolidated full-project review report
(136 verified findings across 8 areas, 1 refuted finding, three improvement plans with
verifier critiques, a cross-area duplicate table and a correction-pass checklist bound to
GitHub issues #143–#270). No Rust, shell, workflow, packaging or manifest file changes, so
the code-oriented pillars (PAM deadlines, panic safety, memory bounds, supply chain) are
trivially unaffected; the review therefore concentrated on (a) whether the report misstates
the code it cites, especially in every CRITICAL finding, (b) internal consistency of the
report (statistics, issue numbers, checklist, Markdown structure), and (c) publication
hygiene (English-only, no secrets, no personal data, no biometric data, no session-user
absolute paths).

Result: the report is internally consistent (statistics table reproduces exactly from the
findings, all 136 findings appear in Appendix B with matching severity/issue/title, all
issue numbers #143–#269 are used and #270 is the tracking issue that exists on GitHub with
the `review-2026-09-29` label), well-formed (5/5 `<details>` pairs, no unclosed code fences,
uniform table column counts), English-only (Layer 1 Audit 7 passes), and every CRITICAL
finding plus twelve MAJOR/MINOR findings were re-verified against the code at the cited
locations and hold. Only MINOR/SUGGESTION observations remain (line-number drift inside
reviewer "Locations" that the embedded verifier notes already correct, and references to
non-repository scratch reproductions). **VERDICT: APPROVED.**

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Run on `target/candid_diff.patch`:

- `grep -E '^diff --git' | grep -E 'tests?[/_.]|/tests/|fixtures'` → no test files touched.
- `grep -nE '^-[^-].*(assert|#\[test\]|...)'` → no removed or changed assertions (the patch
  is a single new-file diff; it contains zero removed lines).
- `grep -nE '^\+.*(#\[ignore|#\[cfg\(any\(\)\)\]|should_panic|tolerance|epsilon)'` → 12 hits,
  all prose lines inside the Markdown report (recommendations that mention `#[ignore]`
  real-model tests); no `.rs` file is present in the patch (`+++` headers list only the
  `.md` file).
- `grep -nE '^[-+].*mod tests'` → 2 hits (patch lines 2050 and 2053), both prose in the
  TCI-11 finding quoting `#[cfg(test)] mod tests;` as an example; no Rust module touched.

No test was added, modified, weakened or deleted. Nothing to justify.

## 3. Deep Reasoning Audit

### Logic & Architecture

Scenarios attempted:

1. **Statistics vs findings.** Parsed every `#### <ID> — <title>` heading with its
   `GitHub issue: #N` and `**SEVERITY**` line (136 findings). Per-area counts reproduce the
   §1.1 table exactly: PAD 2/4/7/2=15, CAM 2/7/7/1=17, ONB 2/8/5/1=16, PAM 1/4/9/3=17,
   DMN 1/7/8/3=19, STO 2/9/10/1=22, VIS 1/2/10/2=15, TCI 1/5/8/1=15; totals 12/46/64/14=136.
   "Refuted: 1" matches Appendix A (PAD-15, and PAD ids skip 15 accordingly). → PASS
2. **Issue numbers.** Distinct issues used: 127 (#143..#269, no gaps). Six issues are shared
   by cross-area duplicates (#146 ×5, #143, #148, #153, #156, #175 ×2), exactly the rows of
   the §1.4 duplicate table. #270 is the tracking issue named in the header; `gh issue view
   270` → "[Review 2026-09-29] Full project review — tracking", labels `type:chore,
   review-2026-09-29`; #143 and #269 titles match CAM-01 and VIS-15. → PASS
3. **Checklist completeness.** Appendix B has 136 rows; every row's ID, severity, issue and
   title match its finding (titles containing `|` are escaped as `\|`); no finding is missing
   and no row lacks a finding. → PASS
4. **Headline claims vs body.** §1.2/§1.3 headline statements (class index 2 at
   `pipeline.rs:189-193` / `service.rs:773`, `panic = "abort"` at `Cargo.toml:126`,
   `build_deb.sh:100` staging via `install.sh --destdir`, PreviewFrame at
   `dispatcher.rs:346-360`, 136,619,444-byte ArcFace) were each re-checked (see Panic Safety
   and Memory sections below). → PASS
5. **Repository references.** All file references are repository-relative; the only
   absolute paths are runtime FHS paths (`/var/lib/soos/...`, `/etc/soos/...`,
   `/run/soos/...`) and the quoted defect `/home/hadrien/...` in TCI-07, which reports an
   existing tracked-file violation rather than introducing one. Remote confirmed as
   `github.com:Mysticaly622/soos`, matching the §3.3 bootstrap URL. → PASS

### PAM Concurrency & Deadlines

No PAM code changes. Spot-checked the report's PAM claims instead:

- PAM-02: `crates/pam/src/ipc.rs:107-131` `read_exact_counted` loops on `stream.read` with
  no `set_read_timeout`; the only refreshes are at :313-315 (header) and :353-355 (body). → matches code.
- PAM-05: `crates/pam/src/config.rs:71-76` gates `/etc/soos/gdm.disable` on
  `self.service` containing "gdm"; `DEFAULT_SERVICE = "pam_soos"` (:18); `service=` is the
  only setter (:135-142); the sole `get_item` call is `Conv` at `lib.rs:46`; the installed
  GDM line at `crates/admin-cli/src/gdm.rs:97,105` carries no `service=`. → matches code.
- TCI-02: every packaging template uses `timeout_ms=250`; `tests/invariants/src/lib.rs:603,
  623,654,671` hard-code the literal; `DEFAULT_TIMEOUT_MS = 1000`, clamp 10–5000;
  `DECISION_BUDGET_MS = 900`. → matches code.
→ PASS (report does not misstate the PAM code).

### Panic Safety & Fail-Closed

- PAM-01 / TCI-01 (CRITICAL): `Cargo.toml:122-128` `[profile.release]` with
  `panic = "abort"` at line 126 and `overflow-checks = true`; `crates/pam/Cargo.toml:13`
  `crate-type = ["cdylib", "rlib"]`; `catch_unwind` at `crates/pam/src/lib.rs:84,167,291`;
  `Docs/SECURITY_AND_QUALITY_GUIDELINES.md:60` ("caught by `catch_unwind`") and :63 ("if any
  unexpected panic escapes `catch_unwind`"). The report's reasoning (no unwinding under
  abort, so catch_unwind cannot catch) is correct Rust semantics; the verifier's line
  correction for the guidelines (:60/:63 rather than :134-153) is itself correct. → PASS
- STO-02 (CRITICAL): `crates/enrollment-cli/src/service.rs:679-686` base64-encodes the raw
  RGB frame, joins `soos-debug.html` to `current_dir()` and calls `std::fs::write`;
  `html_report.rs:152-164` embeds the raw buffer via `putImageData`; `main.rs:32`
  `check_privileges(true)`; leftover comment block at :659-674 and `.unwrap_or_default()` at
  :678; `AI/ARCHITECTURE.md:20` anti-pattern and :245 "securely deleted immediately". → PASS
→ PASS

### Test Integrity & Anti-Weakening

No test code in the patch (section 2). The report itself repeatedly respects the
zero-weakening rule (ONB-01 and PAD-07 explicitly require a backlog/matrix spec change
before `test_install_script_creates_required_directories` or
`test_manifest_v2_model_count_and_checksum_attestation` are touched; VIS-07 flags NGM8's
acceptance test as re-point-not-delete). → PASS

### Memory, Bounds & Secrets

Secrets / personal data / biometric data scan of the report:

- No e-mail addresses, tokens, key material or private URLs (regex scan for `@`, `ghp_`,
  `AKIA`, `-----BEGIN`, `sk-`). `password123` / `hunter2` are throwaway probe strings used
  in a container repro and a redaction probe, not credentials.
- No raw frames, embeddings, crops or hashes of biometric templates; the report explicitly
  argues (PAD-06, §3.1 critique) against committing face crops.
- No `/home/willi363` or other session-user path; `~/.cargo/registry/...` references are
  home-relative crate sources; `scratchpad/...` references are relative labels for
  out-of-repo reproductions (see SUGGESTION below).
- The host observation "136,619,444 bytes, sha256 ffe014..." was re-run here:
  `/var/lib/soos/models/arcface_w600k_mbf.onnx` is 136,619,444 bytes with sha256
  `ffe014a45c9488506719d37fd578ece6661bb385535b36e8039975fa5d4683db`, equal to
  `models/manifest.toml:31`, whose description still reads "ArcFace MobileFaceNet w600k"
  with `input_shape = [1, 3, 112, 112]` (VIS-03 holds).

Code spot-checks of bounds/secret findings:

- CAM-01 / DMN-02 (CRITICAL/MAJOR): `crates/daemon/src/dispatcher.rs:347` PreviewFrame
  branch calls `pipe.camera.notify_activity()` (:349) and clones `frame.data` (:375) before
  Step 6 `verify_peer_credentials` (:422) and Step 6b session check (:445, gated on
  `RequestKind::Auth`); socket mode `0o660`, group `soos` (`config.rs:30-32`);
  `AI/ARCHITECTURE.md:303` lists frames over IPC as a pitfall. → matches code.
- PAD-02 (CRITICAL): loop `while auth_start.elapsed() < total_budget` (:628); PadFailed arm
  only `debug!` + `(0.0, 1u8, false)` (:685-692); `engine.evaluate` (:743); early return on
  Allow (:750); rate limiter recorded once at :757 or :786; `sleep(25ms)` (:778). → matches code.
- PAD-01 / CAM-09 / DMN-01 / STO-01 / VIS-01 (CRITICAL): `crates/daemon/src/pipeline.rs:
  189-193` `OrtPadDetector::new_with_class_index(pad_session, config.vision.pad_threshold,
  2)`; `crates/enrollment-cli/src/service.rs:773` `new_with_class_index(pad_session, 0.80,
  2)`; `crates/inference-ort/src/pad.rs:81-82` doc "Class 1: Genuine Live" and
  `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX: usize = 1`; `pad.rs:146` `probs.get(live_class_index)`;
  `crates/gui/src/main.rs:92` `OrtPadDetector::new(pad_session, 0.80)`; a workspace grep
  confirms these are the only production construction sites. → matches code.
- ONB-01 (CRITICAL): `scripts/install.sh:235-247` generates `master.key` gated only on
  file absence (no `DESTDIR` guard; only the `chown` is DESTDIR-gated); `build_deb.sh:100-104`,
  `build_arch.sh:97-101`, `packaging/debian/rules:14-19` stage via `install.sh --destdir`;
  `packaging/debian/postinst:41` skips generation when the file exists. → matches code.
- ONB-02 (CRITICAL): `packaging/pam/fedora/soos/README` declares no features;
  `system-auth:7,16,24` use `{?with-faillock:...}` (not authselect's
  `{include if "with-faillock"}` syntax); `Docs/PACKAGING_AND_PROVISIONING.md:84` documents
  plain `authselect select custom/soos`; `tests/distro/fedora_rhel_test.sh:236-249` only
  greps template text and :254 runs `authselect check || true`. The container-based lockout
  reproduction (pamtester in fedora:40) was not re-executed here; the static claims and the
  syntax argument are correct. → matches code.
- STO-04, VIS-02, CAM-05/DMN-07, DMN-04, PAM-03: `store.rs:43-46` best-effort `0o700` chmod
  on pre-existing dirs and `args.rs:204-206` FHS prefixes incl. `/tmp` `/home`;
  `color.rs:156-159` `Decoder::new(..).decode()` with no `read_info`/size limit and only
  byte-length comparison; `main.rs:63` sole `set_camera_ready(true)` writer; single global
  semaphore at `dispatcher.rs:121-127` with per-request `timeout` at :136;
  `lib.rs:118-129` three distinct `PAM_TEXT_INFO` texts vs `types.rs:158` "Must NEVER be
  exposed". → all match code.
→ PASS

### Supply Chain & Automation

No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. `./scripts/
candid_review.sh` (Layer 1) was executed on the current tree: all seven audits pass,
including the English-only audit on this report. → PASS

### English-Only Policy

Full-text scan for French function words and diacritics: no French content. Non-ASCII
characters are typographic (`·`, `—`, `→`, `§`, `☐`, `≤`, `≥`), one `İ` (U+0130) that is the
subject of the STO-13 redaction probe, and status glyphs (`✅`, `⬜`). File and directory names
(`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`) are English. → PASS

## 4. Detailed Findings & Action Items

- **[MINOR]** `AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md:325` (and similarly PAD-04
  :361, STO-03 :1453, STO-04 :1465, STO-06 :1477, STO-08 :1501, STO-09 :1513, STO-10 :1525,
  STO-11 :1537, STO-13 :1573, STO-14 :1585, STO-15 :1597, STO-16 :1609, STO-18 :1633,
  STO-19 :1645, STO-20 :1657, STO-21 :1669, STO-22 :1681, TCI-01 :1923) — the reviewer
  "Locations"/"Evidence" lines carry line numbers that do not match the code (e.g. PAD-01
  cites `crates/gui/src/main.rs:288`, the call is at :92; STO-04 cites `store.rs:294-300`,
  the code is at :43-46; TCI-01 cites `SECURITY_AND_QUALITY_GUIDELINES.md:134-153`, the
  rationale is at :60/:63). In every case the embedded verifier notes already state the
  correct lines and §2 declares that verifier notes prevail, so no reader is misled about
  the code; but the correction pass should normalise the "Locations" bullets to the
  verifier-corrected lines (or note "see verifier notes") so that the checklist and the
  GitHub issues cite accurate locations. No action required before merge.
- **[MINOR]** `AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md:1717` — VIS-01 cites the
  constant at `crates/inference-ort/src/pad.rs:80` (reviewer and verifier); the constant is
  at :82 (doc comment at :81). Cosmetic; the other four duplicates of this finding cite
  :81-82 correctly.
- **[SUGGESTION]** `AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md:936`, `:951`, `:1059`,
  `:1457`, `:1924` — reproductions are referenced as `scratchpad/...` paths that do not
  exist in the repository. The surrounding text describes the reproduction well enough to
  redo it, so this is acceptable for a review artifact; consider committing the small
  repro sources under `tests/` (or an `AI/reviews/repros/` folder) in the correction pass
  when a fix PR relies on them as acceptance evidence.
- **[SUGGESTION]** `AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md:1996`, `:1999` — the
  `/home/hadrien/...` strings are quotations of the tracked defect (TCI-07) and are the only
  `/home/` occurrences; Layer 1 accepts them. Keep them as-is until TCI-07 removes the
  source files, then drop the quotation to keep the docs free of user-home paths per
  project-facts §6.

## 5. Final Verdict

Documentation-only diff. The review report is internally consistent, English-only, free of
secrets, personal data, biometric data and session-user paths, and every CRITICAL finding
(and a dozen MAJOR/MINOR findings) accurately describes the code at the cited locations
once the embedded verifier corrections are taken into account. Residual observations are
cosmetic line-number drift and non-repository reproduction references.

**VERDICT: APPROVED**
