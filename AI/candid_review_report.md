# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `chore/readme-redesign`
- **Base (merge-base)**: `aa25f78`
- **Reviewed-Diff-Fingerprint**: `a49bd92b6f909255653a1c9fe424b6fcfbcd6e826e681c2da731b876189b95ed`
- **Audited Files**: `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_readme_redesign.md`,
  `AI/auditor_constraints_readme_redesign.md`, `AI/walkthroughs/216_readme_redesign.md`,
  `Docs/DEVELOPMENT_WORKFLOW.md`, `Docs/README.md`, `README.md`, `tests/invariants/src/lib.rs`,
  `tests/invariants/src/readme_contract.rs`

## 1. Executive Summary

Re-review after the rebase of `chore/readme-redesign` from `9e82795` onto `origin/main` `aa25f78`
(main gained #411, #417 zbus 6 migration, #413 lazy lock-widget buffer and #402 setup user phase).

Rebase integrity: `git range-diff 9e82795..3a1ce5c aa25f78..03f5ff1` shows only context changes.
Per file, the delta between the old and new branch tips equals main's own delta (identical
`git patch-id`) for every file except `AI/DECISIONS.md`, where both sides appended an ADR at the end;
the conflict was resolved by keeping main's ADRs and appending the branch ADR unchanged (the branch's
`+`/`-` lines against each base are byte-identical; no conflict marker remains). The branch's own
changes are unchanged by the rebase.

Gates: `cargo test --locked -p soos-invariants -- --test-threads=4` → 739 passed, 0 failed;
`cargo clippy --locked -p soos-invariants --all-targets -- -D warnings` → clean.

However, the semantic check against #402 (merged just before) finds that the new README's security
and feature claims now contradict the recommended install path: `scripts/setup.sh` writes
`allow_unlock = true`, `camera_view = true` and `camera_view_funnel = true` by default (ADR
2026-10-15 "Setup User Phase", item (3)), and the README's own Quick Start (text from #402) says the
companion is set up "with every feature on". The README Security model still says "Remote unlock is
off by default". Since the ADR of this branch makes truthfulness the security property of the README,
this is a MAJOR finding. Verdict: CHANGES_REQUESTED.

## 2. Test Changes

Mechanical listing from `target/candid_diff.patch`:

- Test files touched: `tests/invariants/src/lib.rs` (adds `#[cfg(all(test, unix))] mod readme_contract;`
  and its doc comment only), `tests/invariants/src/readme_contract.rs` (new file, RDM1–RDM8).
- Removed or changed assertions (`^-` with `assert`/`#[test]`/`proptest!`/`should_panic`): none.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.

No pre-existing test assertion is changed by the branch. The only test change main brought in
(SET3 in `setup_contract.rs`, owner-approved 2026-10-15) belongs to #402 and is recorded in its ADR;
it is not part of this diff.

## 3. Deep Reasoning Audit

### Logic & Architecture

- Section order, Mermaid block, badges (5 ≤ 6, no release badge, `rust-1.98.1` equals
  `rust-toolchain.toml`), link resolution: verified by RDM1–RDM8, all green on the rebased tree. PASS.
- Quick Start after rebase contains #402's companion paragraph; the `<details>` wrapping of the
  installer paragraph is intact and existing contracts (onboarding, installer, setup SET17, GDM, UPG7,
  licence, RAC10) pass. PASS.
- Interaction with #402 (setup user phase defaults): the security model, Why, Features, Mermaid and FAQ
  describe remote unlock and camera view as opt-in / off by default, while the one-command setup the
  README recommends turns them on (and camera view over Funnel). FINDING (MAJOR, see §4).
- Feature row "One-command setup" omits that setup now also sets up the phone companion. FINDING (MINOR).
- Interaction with #417 (zbus 6), #413 (lock-widget lazy buffer), #411 (LockState refusal test): no
  README claim depends on zbus versions, buffer counts or reserve semantics; the lock-widget row only
  names swaylock-plugin and hyprlock, still accurate. PASS.

### PAM Concurrency & Deadlines

No runtime code changed. README statements (blocking, no Tokio, deadline from `timeout_ms`, default
1000 ms) match `crates/pam/src/config.rs` and the ADR. PASS.

### Panic Safety & Fail-Closed

No production code. README fail-closed wording (`PAM_SUCCESS` only on a fresh `Allow`, else
`PAM_IGNORE`) matches the invariants. Test code uses the standard invariant `#![allow]`. PASS.

### Test Integrity & Anti-Weakening

New tests can fail against plausible wrong READMEs (missing file link, banned word, reordered
sections, missing needles; RDM2/RDM4 self-check the helpers with negative cases). No existing assertion
touched. PASS.

### Memory, Bounds & Secrets

Test reads two files, runs `git tag --list` with `GIT_*` removed, writes a scratch tree under
`target/readme_contract/`; resolver refuses absolute and escaping paths. README contains only the three
synthetic PNGs and placeholder identities (`alice`, `you@example.com`, `mypc.tail1234.ts.net` from main).
PASS.

### Supply Chain & Automation

No `Cargo.*`, workflow, script or dependency change in this diff. PASS.

### English-Only Policy

All added text is English. PASS.

## 4. Detailed Findings & Action Items

- **[MAJOR]** `README.md:285` — "Remote unlock is off by default and needs a passkey check on every
  request" is no longer true for the path the README recommends: since #402, `scripts/setup.sh`
  (`COMPANION_KEYS`, defaults printed at `scripts/setup.sh:2283-2284`) writes `allow_unlock = true`,
  `camera_view = true` and `camera_view_funnel = true`, and the README's own Quick Start says "with every
  feature on". Required correction: state that the code and manual defaults are off, but the
  one-command setup turns remote unlock, camera view and camera view over Funnel on when the companion
  is configured (each switchable off in soos-gui or with `--remote-unlock off` / `--camera-view off` /
  `--no-remote`).
- **[MAJOR]** `README.md:337-339` (FAQ "Does any image leave my machine?"), `README.md:37` (Why),
  `README.md:53` (Features), `README.md:233,246` (Mermaid labels) — camera view and unlock are described
  as "opt-in" and the camera view as streaming "through your Tailscale network"; with the setup default
  `camera_view_funnel = true` the stream can go over Tailscale Funnel (public HTTPS endpoint) after a
  passkey check. Required correction: align the wording with the setup defaults (on with the companion
  setup, off in the manual path) and mention Funnel.
- **[MINOR]** `README.md:49` — the "One-command setup" feature row does not mention that setup also
  configures the phone companion (#402) when given a Tailscale login. Add it.
- **[SUGGESTION]** Consider an RDM needle tying the security section's default wording to the setup
  defaults so a later setup change cannot silently invalidate the README again.

## 5. Final Verdict

**VERDICT: CHANGES_REQUESTED**
