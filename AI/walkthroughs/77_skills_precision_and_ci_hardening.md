# Walkthrough 77 — Agent Skills Precision & CI/CD Hardening

- **Date**: 2026-09-29
- **Issue**: none (tooling/process change) — **Branch**: `chore/skills-ci-hardening`
- **Matrix criteria**: none (no product behavior change)

---

## 1. Context & Objectives

A full review of `AI/`, `Docs/` and all 78 walkthroughs showed three classes of problems:

1. **Imprecise skills**: the eight `.agents/skills/*/SKILL.md` files restated generic rules,
   carried stale facts ("200–250ms" while the code uses a clamped 1000ms default and 2500ms for GDM;
   three different `forbid(unsafe_code)` crate lists), absolute `/home/<user>` links, tool-specific
   jargon, and French text. They did not encode the lessons from recurring defects (mock-only model
   tests, `Duration::ZERO` semantics, `"auto"` sentinels, clippy without `--all-features`).
2. **A review gate that could not fail**: `scripts/candid_subagent.sh` auto-generated an
   all-PASS `VERDICT: APPROVED` report when none existed, and only grepped for the verdict string,
   so a report left over from a previous pull request satisfied the gate.
3. **A slow, loosely secured CI**: one serial job (~8.5 minutes, 1.6 of them deleting runner
   software), no `target/` cache, an uncached Docker build that waited for the quality job,
   floating action tags, default token permissions, and no enforcement of the PR title convention
   that `Docs/COMMIT_CONVENTION.md` claimed.

## 2. Design

### Skills
- New shared reference `.agents/skills/dev-workflow/references/project-facts.md`: crate map with
  the authoritative `unsafe` policy, security/latency constants with their source location, runtime
  paths and modes, the verified model contract (BGR, NHWC, live class 1, pre-sigmoided SCRFD scores),
  the exact CI commands, and traceability conventions. It states the precedence rule
  (code constants are truth; prose drift is reported, not silently "fixed").
- Every skill now has a trigger-oriented description ("Use when ..."), inputs, a numbered
  procedure with concrete commands, walkthrough-sourced pitfalls, a fixed deliverable template and
  explicit exit criteria. The orchestrator defines a gate table between phases.
- The tester skill defines the only legitimate way an existing test changes (Contract Migration,
  owned by the tester, justified by a backlog acceptance line), which removes the ambiguity that
  previously made anti-weakening reviews unreliable.

### Review gate (layer 2)
- `scripts/candid_subagent.sh --prepare` writes `target/candid_diff.patch` and prints a
  **diff fingerprint**: SHA-256 of the diff from the merge-base with `origin/main` to a snapshot of
  the working tree (temporary index; untracked files included; the report excluded).
- The report must contain `Reviewed-Diff-Fingerprint: <sha256>` and `VERDICT: APPROVED`.
  The gate recomputes the fingerprint for the working tree (default) or a commit (`--rev`).
- The template auto-approval was removed; missing, template, stale and `CHANGES_REQUESTED`
  reports all fail.

### CI (`.github/workflows/ci.yml`)
- Six jobs: `lint` (no compilation), `clippy`, `test`, `security`, `pam-integration` (after lint),
  and the `CI Success` aggregate.
- Speed: parallel jobs, `Swatinem/rust-cache` (writes on `main` only), no debuginfo in CI,
  Buildx + GHA layer cache for the PAM sandbox, prebuilt cargo-deny, removal of the disk cleanup
  step, concurrency cancellation of superseded PR runs, `.dockerignore` with an empty context.
- Security: `permissions: contents: read`, SHA-pinned actions + Dependabot (7-day cooldown),
  `persist-credentials: false`, event data passed through `env:` only, `--locked` everywhere,
  per-job timeouts, daily advisory scan, per-commit secret scan of the PR/push range, the layer 2
  fingerprint check on the PR head (no bot exemption), and a separate `pr-title.yml` workflow that
  validates the PR title with `.githooks/commit-msg` (re-runs on `edited`).
- Concurrency: one group per PR (superseded runs cancelled); other runs are grouped per event and
  commit so runs on `main` are never cancelled.

### Local tooling
- `scripts/secret_scan.sh` (new) is shared by pre-commit (`--staged`), pre-push (`--unpushed`)
  and CI (`--range`). Range modes scan every commit individually and fail closed on an
  unresolvable range. It adds GitHub/AWS/Slack/`sk-` tokens, any PEM private key, `.env`, and — project-specific —
  encrypted biometric/evidence files, master keys and ONNX weights. Findings never echo the secret.
- `.githooks/pre-commit` runs layer 1 only (intermediate commits stay possible);
  `.githooks/pre-push` (new) refuses pushes to or deletion of `main`, scans every unpushed commit
  and enforces layer 2.
- `scripts/candid_review.sh`: the `unsafe` check now covers all nine business crates, flags a
  `#![forbid(unsafe_code)]` attribute that disappears from a file (relative to the merge-base), detects any async runtime in `crates/pam`, and syntax-checks hooks.
- `save.sh`: clippy/test/deny use the exact CI flags.
- `scripts/pr_loop.sh`: waits until the PR head is the pushed SHA and CI has started on it, then
  `gh pr checks --watch --fail-fast` (45-minute ceiling, above the longest job timeout, instead of a
  10-minute poll that was shorter than CI itself); then polls the `CI Success` check run of that
  exact SHA (created only once its dependencies finish) until it completes with `success`; merges with
  `--match-head-commit` and without `--admin`; never uses the shared git stash; handles `main`
  being checked out in another worktree.
- The fingerprint diff pins every git option that user configuration could alter (prefixes,
  algorithm, renames, order file, quoting) and runs from the repository root, so it is identical
  on every machine and in CI; any git failure aborts instead of hashing an empty diff.

## 3. Candid Review

The first independent review (fresh sub-agent, following the new skill) returned
**CHANGES_REQUESTED** with four MAJOR findings, all fixed in production code:

1. `secret_scan.sh` failed open on an invalid range (git error hidden in process substitution) →
   revisions are resolved up front and every git failure exits 2.
2. Range scanning compared only the range endpoints, missing a secret added then removed →
   each commit is scanned individually.
3. `pr_loop.sh` could merge on the previous head's green checks → CI is bound to the pushed SHA
   (`CI Success` check run of that commit).
4. The reviewer's "mechanical" test-change listing missed inline `#[cfg(test)]` modules and
   untracked files → it now greps the frozen `target/candid_diff.patch`.

MINOR findings fixed: git-config-dependent fingerprint, silent snapshot failure, `forbid` removal
false positives, silent pre-push scan skip, `sk-` pattern boundary and case-insensitive file names,
scanner self-exclusion, `main` checked out in another worktree, cancellable `main` runs, deletion of
`main` passing the hook, PR title not re-validated on retitle, T1–T8 → T1–T9 wording. The
spoofable Dependabot actor exemption (flagged by zizmor) was removed. The second review returned **CHANGES_REQUESTED** with one MAJOR finding: `CI Success` is created
only after its `needs:` complete, so waiting for it up front always timed out → `pr_loop.sh` now
waits for any check run of the SHA, watches, then polls `CI Success` to completion. Its minors were
also fixed: literal pathspecs in the secret scanner, merge commits scanned against their first
parent, fail-closed content reads, 45-minute watch ceiling, full-history fallback in CI, verdict
matched only on verdict lines, PR titles limited to 72 characters (GitHub appends ` (#NNN)`).
The third review was **APPROVED** with four MINOR findings, then fixed: the `forbid` removal check
no longer uses `grep -q` under `pipefail` (exit 141 on large diffs), binary files are content-scanned
(`--text`), `save.sh`/`pr_loop.sh` reject subjects over 72 characters before pushing (matching
`pr-title.yml`), and the `pr_loop.sh` header states the 45-minute ceiling. The fourth review returned **CHANGES_REQUESTED** (MAJOR): in UTF-8 locales GNU grep silently skips
lines containing invalid UTF-8, so a token next to binary or Latin-1 bytes passed → the scanner now
runs with `LC_ALL=C`. Minors fixed with it: the no-new-commit `--push-pr` path also enforces the
72-character subject, `--inter-hunk-context=0` is pinned in the fingerprint diff, and any
`CHANGES_REQUESTED` verdict line fails the gate even if an `APPROVED` line exists. The fifth review returned **CHANGES_REQUESTED** (MAJOR): `--diff-filter=ACMR` skipped type
changes, so a symlink replaced by a file holding a secret passed → `ACMRT`. Minors fixed with it:
added content lines starting with `++ ` were dropped as headers (the per-file header is now skipped
only before the first hunk), and the Dependabot group was renamed to keep titles within 72
characters. A sixth review was run on the final diff (see `AI/candid_review_report.md`).

## 4. Verification Results

| Check | Result |
|---|---|
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked --workspace --all-targets --all-features` | pass (479 tests) |
| `actionlint` (v1.7.12, with ShellCheck on `run:` blocks) | 0 findings |
| `zizmor --offline` (v1.30.1) on `.github/` | 0 findings |
| ShellCheck `--severity=error` on all scripts and hooks | 0 findings |
| `secret_scan.sh` on a synthetic repo (token, PEM key, `.enc`, `.PEM`, add-then-remove, bad range, kebab-case text) | findings exit 1; bad range exit 2; clean exit 0; full repository history clean |
| Fingerprint stability: working tree vs commit, subdirectory, hostile git config (`noprefix`, `histogram`, `copies`) | identical; one later edit ⇒ gate fails |
| Stale report from PR #140 against this branch | rejected ("no Reviewed-Diff-Fingerprint") |
| `./run_tests.sh` with the empty Docker build context | T1–T9 pass (1m34s) |
| Test build without debuginfo | 2.9 GB target dir vs 3.3 GB with debuginfo |

## 5. Known Limitations / Follow-ups

- Repository settings are outside the code: requiring the `CI Success` and `PR Title` checks on
  `main` (branch protection or a ruleset) and a CODEOWNERS entry for `scripts/`, `.githooks/` and
  `.github/` are recommended; until then a PR can modify the gate scripts that check it.
- `AGENTS.md`, `AI/ARCHITECTURE.md` and `AI/DECISIONS.md` still state the historical 200–250ms PAM
  deadline and MiniFASNet live class 0; an ADR entry recording the current values (clamped 1000ms
  default, 2500ms GDM, live class 1) should be added.
- `rust-toolchain.toml` tracks `stable`; pinning an exact version would remove CI-only clippy
  drift at the cost of manual bumps.
