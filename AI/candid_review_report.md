# Candid Review Report

- **Date**: 2026-10-15 (re-review after the rebase onto `origin/main` `515ada3`; first review 2026-10-13)
- **Target Branch**: `chore/ui-motion-spec`
- **Base (merge-base)**: `515ada3` (previously `6f32ef8`)
- **Reviewed-Diff-Fingerprint**: `01d34256f783c74aa6f889fa1b35250edf149c36969692d4b7bd5106023d81e6`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_ui_motion.md`,
  `AI/auditor_constraints_ui_motion.md`, `AI/tester_contract_motion.md`, `AI/walkthroughs/203_ui_motion_spec.md`,
  `Docs/GUI_APPLICATION.md`, `Docs/README.md`, `Docs/REMOTE_COMPANION.md`, `Docs/UI_MOTION.md`,
  `Docs/prototypes/ui_motion_prototype.html`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/ui_motion_spec_contract.rs` (`AI/plan_evaluator_report.md` excluded by the tool)

## 1. Executive Summary

GitHub #377 (PR 1 of #358) is documentation, specification, a static HTML prototype and one new invariant module;
no production code, manifest, lockfile, workflow or script changes. The diff matches the #377 scope and the #358
owner decisions of 2026-10-13: brand ADR amendment (motion allowed, still no gradients or decorative shadows),
spring overshoot at most 4 % (2.84 % analytic; CSS `linear()` peak 2.71 %; `cubic-bezier` fallback peak 3.02 %),
prototype approval (UIM7) gating the implementation PRs, GUI reduced motion from the XDG portal over zbus plus
`--reduce-motion` / `SOOS_REDUCE_MOTION`, dark variant out of scope. The auditor's C-1 (zbus scope) was re-checked
independently and cleared (C-16 added for the later GUI invariant). `cargo fmt --check`, `cargo clippy -p
soos-invariants --all-targets -D warnings` and `cargo test -p soos-invariants` (611 passed) are green. Two MINOR
documentation findings (the ADR splice since resolved), no CRITICAL or MAJOR.

**Rebase re-review (2026-10-15).** The branch was rebased from `8967ff5` onto `origin/main` `515ada3` (main gained
#383, the push-test deflake with matrix section `rwp-delivery-deflake`, and #394, an actions bump). Checked
independently:
- `git range-diff origin/chore/ui-motion-spec...HEAD`: the only content change in the branch commit outside the
  two report files (`AI/candid_review_report.md`, `AI/plan_evaluator_report.md`, whose base text changed on main)
  is the `AI/VERIFICATION_MATRIX.md` hunk context.
- Per-file comparison of the branch's added/removed lines before and after the rebase: identical for
  `AI/DECISIONS.md`, the spec, auditor constraints, tester contract, walkthrough 203, `Docs/GUI_APPLICATION.md`,
  `Docs/README.md`, `Docs/REMOTE_COMPANION.md`, `Docs/UI_MOTION.md`, the prototype, `tests/invariants/src/lib.rs`
  and `tests/invariants/src/ui_motion_spec_contract.rs`.
- `AI/VERIFICATION_MATRIX.md`: the file's first lines equal `origin/main`'s file byte for byte (main's RWPF section,
  row RWPF1, intact; nothing lost from main); the branch adds exactly `---`, a blank line and the unchanged
  `ui-motion-spec` section (UIM1–UIM12, 12 rows, no duplicate ids across RWPF/UIM). The `---` separator follows the
  file's existing convention between component sections.
- `cargo test --locked -p soos-invariants` on the rebased tree: 611 passed, 0 failed.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: `tests/invariants/src/lib.rs` (+8 lines: doc comment, `#[cfg(test)]`,
  `mod ui_motion_spec_contract;`) and the new file `tests/invariants/src/ui_motion_spec_contract.rs`.
- Removed or changed assertions (`^-` with `assert`/`#[test]`/...): none.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, `tolerance`, `epsilon`): none.
- Inline `mod tests` changes: none.
- Justification: additive module only (UIM-S1 to UIM-S6 plus a helper self-test). No existing test is modified.
  The planned PAU17 migration is recorded for a later PR only (tester contract B.2a) and is not applied here.

## 3. Deep Reasoning Audit

### Logic & Architecture
- Tried: does the spring meet the owner bound at every rate? UIM-S3 computes ζ = 0.75, analytic overshoot,
  continuous settle (0.1 % from 400 ms), discrete closed-form stepping at 30/60/144 Hz with a re-unsettle panic,
  `linear()` error and peak, fallback peak (recomputed independently: 1.0302). PASS.
- Tried: a token drift between the doc, `style.css` and `theme.rs`. UIM-S2 pins the table and cross-checks any
  token already present in either file (forward-compatible), exits = 0.7 × entry. PASS.
- Tried: contradiction with the brand and browser-support ADRs. The ADR amends item (3) of the 2026-10-06 brand
  ADR without superseding it, keeps no-gradient / no-decorative-shadow / CSP / routes / eleven-file asset set, and
  uses feature detection only (no UA logic, no polyfill). PASS.
- Tried: the zbus scope widening as a hidden weakening. Owner decision 4 of #358 names "XDG portal ... over zbus";
  the exception is session bus, read-only, two keys, one connection, no object server/name/match, 200 ms deadline,
  failure = no preference. PAU17 migration adds one `ALLOWED` path and keeps every other assertion. PASS, with
  the gap that the negative (no system bus, no logind) is implicit: bound by auditor C-16 (MINOR finding 2).
- Text integrity of the ADR item (6): FINDING (MINOR 1), resolved: re-review of the new fingerprint (word diff of
  `AI/DECISIONS.md` against the previous review shows only the sentence move) confirms item (6) now ends with
  "then full motion; reduced makes every helper return its target and `Style::animation_time` 0." followed by the
  zbus-scope paragraph ending "No process is spawned instead.", and item (7) "Progressive enhancement" reads intact.
  `cargo test -p soos-invariants` re-run: 611 passed.

### PAM Concurrency & Deadlines
- No change under `crates/pam` or any crate; the later portal reader stays in `soos-gui`, which is not in the
  `pam_soos.so` graph (`lexing_contract` and PAU PAM assertion untouched). PASS (n/a).

### Panic Safety & Fail-Closed
- Test-only module; module-level `allow` with `reason`. No production path. The specified web behaviour keeps the
  native `window.confirm` plus passkey, no optimistic "Unlocked", state written before animation (prototype
  `render()` also writes text/class/`data-state`/`disabled` together). PASS.

### Test Integrity & Anti-Weakening
- Tried: can UIM-S4 pass on a prototype that loads a remote font or image? It forbids `http(s)://`, `src=`,
  `href=`, `url(`, `data:`, `blob:`, `@import`, `@font-face`, `<img>`, `<link>`, network/storage/camera APIs and
  `/home/`, and checks every hex colour against `theme.rs`. A plausible wrong prototype fails. PASS.
- Tried: an overshoot of 5 % would fail UIM-S3 (`MAX_OVERSHOOT`), a non-settling spring panics. PASS.
- Rebase: the four step-3 greps re-run on the new patch give the same result (test files touched: `lib.rs` and the
  new module only; no removed assertion, no escape hatch, no inline `mod tests` change). Main's #383 test changes
  are in the base, not in this diff. PASS.

### Memory, Bounds & Secrets
- Prototype (57 337 bytes, under the 96 KiB bound) grep: no external resource, no network, storage, camera API,
  image, face or person picture (camera stage is a flat placeholder); alert mock shows time/source/"wrong password"
  only, never a typed password; non-ASCII only U+00A0, U+00B7, U+2026. Colours and SVG paths come from the
  repository (`theme.rs`, `crates/remote/assets/index.html`), nothing from a private brand pack. Script writes
  `style.transform` only inside the prototype (C-15 forbids copying it into assets). PASS.

### Supply Chain & Automation
- No `Cargo.*`, `deny.toml`, `.github/`, `scripts/` or `.githooks/` change. Later `zbus`/`tokio` for `soos-gui`
  are already in `Cargo.lock` once each; C-4 binds features. PASS.

### English-Only Policy
- All new Markdown, the prototype text and test strings are English. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR — RESOLVED in this fingerprint]** `AI/DECISIONS.md:135` — the zbus-scope paragraph was spliced into the middle of item (6)'s
  precedence sentence: "... No process is spawned instead., then full motion; reduced makes every helper ...".
  The GUI reduced-motion chain (portal, then `enable-animations`, then full motion) is now broken across the
  inserted text in an append-only ADR register. Move the inserted paragraph after "then full motion; ... 0." and
  drop the stray "., " (the spec D-13 and `Docs/UI_MOTION.md` §8 state the chain correctly). Requires a new
  `--prepare` and review of the one-line change.
- **[MINOR]** `AI/tester_contract_motion.md:148` — the planned GUI zbus invariant lists only `ObjectServer`,
  `request_name` and match rules; it does not forbid `Connection::system` / `Builder::system`,
  `DBUS_SYSTEM_BUS_ADDRESS`, `system_bus_socket` or `login1`. Bound for `feat/gui-motion` by auditor constraint
  C-16 in `AI/auditor_constraints_ui_motion.md`; mirror it in the tester contract when that PR is specified.
- **[SUGGESTION]** `Docs/prototypes/ui_motion_prototype.html:561` — with Slow ×4 the simulated pending lasts 6 s;
  the hint now says so, acceptable for a review aid.

## 5. Final Verdict

No CRITICAL or MAJOR finding after the search above; MINOR 1 is resolved, MINOR 2 is bound by auditor C-16 for the later PR.

**VERDICT: APPROVED**
