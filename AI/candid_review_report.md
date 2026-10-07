# Candid Review Report

- **Date**: 2026-10-07
- **Target Branch**: `dependabot/github_actions/actions-af36512759` (PR #341)
- **Base (merge-base)**: `e5a549e`
- **Reviewed-Diff-Fingerprint**: `160ab48e83a2dc8e01a4f6089fd7e7d7367f9dc3c9b99ae35ae5b555fa1246fe`
- **Audited Files**: `.github/workflows/ci.yml` (1 insertion / 1 deletion)

## 1. Change

Dependabot bumps `taiki-e/install-action` from v2.87.21 to v2.87.22 in the `Security
(cargo-deny)` job. The action stays pinned by full commit SHA with a version comment, and the
installed tool stays `cargo-deny@0.20.2` (checksum-verified by the action).

## 2. Supply Chain

- The new SHA `83ac0ad63c0167e6f06796fab0fce28db1bf3db0` was checked against GitHub: it is the
  commit the upstream tag `v2.87.22` points to (lightweight tag, commit "Release 2.87.22",
  2026-09-29), so the pin and its comment agree.
- Patch release of the same major line; no new permissions, inputs, secrets or tools.

## 3. Invariants

No Rust, PAM, IPC, daemon or packaging code changes; no logging, panic or unsafe surface. English
only.

**VERDICT: APPROVED**
