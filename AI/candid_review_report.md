# Candid Review Report

- **Date**: 2026-10-15
- **Target Branch**: `dependabot/github_actions/actions-f112735df3` (PR #394)
- **Base (merge-base)**: `6f32ef8`
- **Reviewed-Diff-Fingerprint**: `3a0764d96024434d5a873cc719b060dc56213177667a96643a8a742ed0a3a01d`
- **Audited Files**: `.github/workflows/ci.yml` (1 insertion / 1 deletion), `.github/workflows/fuzz.yml` (1 insertion / 1 deletion)

## 1. Change

Dependabot bumps two pinned actions:

- `taiki-e/install-action` from v2.87.22 to v2.87.26 in the `Security (cargo-deny)` job of
  `ci.yml`. The installed tool stays `cargo-deny@0.20.2` (checksum-verified by the action).
- `actions/upload-artifact` from v7.0.1 to v7.0.2 in the failure-only `Upload crash reproducers`
  step of `fuzz.yml`. Inputs (`name`, `path`) are unchanged.

Both actions stay pinned by full commit SHA with a version comment.

## 2. Supply Chain

- `f7e5d7c961414b23f5b25b2da9294395d08513ad` was checked against GitHub: it is the commit the
  upstream tag `v2.87.26` points to (lightweight tag, commit "Release 2.87.26", 2026-10-06).
  The upstream range v2.87.22..v2.87.26 only updates tool manifests (none for cargo-deny), the
  changelog, upstream CI workflows and the manifest codegen; `main.sh` and `action.yml` are not
  touched, so the cargo-deny install path is unchanged.
- `cf430e030ddbb5b0abf93d22962f4752f3646cd9` was checked against GitHub: it is the commit the
  upstream tag `v7.0.2` points to (lightweight tag, merge of "prepare v7.0.2 release",
  2026-10-07). Release notes: improved retry handling on HTTP 429 (honouring `Retry-After`) and
  `@actions/artifact` 6.3.1. No security advisory, no new inputs.
- Patch releases on the same major lines; no new permissions, inputs, secrets or tools; the
  workflows' `permissions:` blocks are unchanged and no `${{ github.event.* }}` reaches `run:`.

## 3. Invariants

No Rust, PAM, IPC, daemon or packaging code changes; no test changes; no logging, panic or unsafe
surface. English only.

**VERDICT: APPROVED**
