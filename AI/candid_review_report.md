# Candid Review Report

- **Date**: 2026-09-29
- **Target Branch**: `chore/skills-ci-hardening`
- **Base (merge-base)**: `9ab7470`
- **Reviewed-Diff-Fingerprint**: `6262f1191df7208566917d93f846b950c51de1b1ceeefd8bc88a0955adc979e6`
- **Audited Files**: `.agents/skills/architect-agent/SKILL.md`, `.agents/skills/auditor-agent/SKILL.md`,
  `.agents/skills/candid-reviewer/SKILL.md`, `.agents/skills/dev-workflow/SKILL.md`,
  `.agents/skills/dev-workflow/references/project-facts.md`, `.agents/skills/developer-agent/SKILL.md`,
  `.agents/skills/plan-evaluator/SKILL.md`, `.agents/skills/tester-agent/SKILL.md`,
  `.agents/skills/traceability-agent/SKILL.md`, `.dockerignore`, `.githooks/pre-commit`,
  `.githooks/pre-push`, `.github/dependabot.yml`, `.github/workflows/ci.yml`,
  `.github/workflows/pr-title.yml`, `AI/ROLES_AND_WORKFLOW.md`,
  `AI/walkthroughs/77_skills_precision_and_ci_hardening.md`, `Docs/CI_CD_AND_SECURITY.md`,
  `Docs/COMMIT_CONVENTION.md`, `Docs/DEVELOPMENT_WORKFLOW.md`,
  `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `save.sh`, `scripts/candid_review.sh`,
  `scripts/candid_subagent.sh`, `scripts/pr_loop.sh`, `scripts/secret_scan.sh`

## 1. Executive Summary

Sixth independent review of the tooling/process change (no Rust source or test change). The three
fixes requested by the previous review were verified by construction and by execution in a scratch
repository:

- `scripts/secret_scan.sh` now lists names with `--diff-filter=ACMRT` in all three modes (staged,
  merge commit, regular commit). A symlink replaced by a regular file holding a GitHub token is
  blocked both staged and in `--range` mode.
- The per-file header is skipped only before the first `@@` hunk (`awk` state machine, line 127).
  Verified: a token in the second hunk of a multi-hunk diff, a content line `++ <token>`, and a new
  file whose first line is `+++ <token>` are all detected.
- The Dependabot group is named `actions`; the 72-character retitle rule is documented in
  `.github/dependabot.yml` and `Docs/CI_CD_AND_SECURITY.md`.

No new CRITICAL or MAJOR defect was found. Two MINOR and two SUGGESTION items are listed in §4;
none is a fail-open path in a gate that is not already documented as a repository-settings
limitation.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Commands from the candid-reviewer skill were run on the frozen `target/candid_diff.patch`:

- Test files touched (`tests?[/_.]|/tests/|fixtures`): **none**.
- Removed/changed checks (`^-[^-].*(assert|#[test]|...)`): 8 hits, all in removed Markdown prose of
  the old skill files (e.g. "asserts PAM fallback (RED)", "All tests MUST fail initially"). No Rust
  assertion was removed. The substance of each removed rule is retained in the new skills
  (verified: tester-agent lines 24–35 keep Red proof, truncated-frame, `PAM_IGNORE` and
  downstream-spy requirements; auditor-agent keeps the `// SAFETY:` rule).
- New escape hatches (`#[ignore`, `should_panic`, `tolerance`, `epsilon`): 4 hits, all Markdown
  (the grep commands themselves in the candid skill and the developer/tester prohibitions).
- Inline test modules (`mod tests`): 1 hit, the grep command in the candid skill.

No test was modified, weakened or deleted.

## 3. Deep Reasoning Audit

### Logic & Architecture
- **secret_scan.sh awk filter** — tried: multiple hunks in one file (token only in hunk 2), `++ `
  and `+++ ` content lines, typechange diffs (git emits two sections for one path), rename in the
  staged index with `diff.renames=true` (pathspec-limited diff shows the new path as a full add),
  binary content with `--text` (NUL bytes, no "Binary files differ"), a `-diff` gitattribute,
  Latin-1 bytes, an empty added file, an evil merge resolution, an empty range `HEAD..HEAD`, an
  unresolvable range, and a range without `..`. All detected / exit codes as documented (1 on
  finding, 0 clean, 2 on bad input). → PASS (typechange header nit in §4).
- **Full history scan** of this repository: clean (exit 0, ~29 s), so the CI full-history fallback
  and the extended file-name rules (`*.enc`, `*.key`, `*.onnx`, `.env`) do not break on existing
  history; `git ls-files` contains no file matching the name rules.
- **candid_review.sh** — nine business crates match `test_business_crates_forbid_unsafe_code`
  exactly; the `forbid` removal check captures the diff before `grep` (no SIGPIPE under
  `pipefail`); layer 1 passes on this tree. → PASS.
- **candid_subagent.sh** — fingerprint diff pins config-sensitive options; worktree snapshot uses a
  temporary index and aborts on failure; verdict matched only on verdict lines, any
  `CHANGES_REQUESTED` verdict line fails; `--rev` reads the committed report. → PASS.
- **pr_loop.sh** — waits for PR head == pushed SHA and at least one check run; `--watch --fail-fast`
  under `timeout`; authoritative poll of `CI Success` on the SHA (the check-runs endpoint returns
  the latest run per name by default, so a re-run does not leave a stale failure); merge with
  `--match-head-commit`, no `--admin`, no stash. If the watch starts before the CI jobs register,
  the later `CI Success` poll still blocks the merge (fail-closed). → PASS.
- **Docs vs code** — every repository path cited in skills/docs exists; every constant in
  `project-facts.md` matches the code (`grep -rn "pub const"`); `catch_c_entry`,
  `read_exact_counted`, `TruncatedResponse`, `parse_argv`/`parse_cstrs`, six `pam_sm_*` exports,
  `LOG_AUTHPRIV | LOG_ERR`, `BRANCH_TO_ISSUE`, `sync_issue.py --subissue/--comment/--auto`, and
  `save.sh -m` all exist. One inaccurate claim found (auditor `<svc>.disable`, §4).
- **.dockerignore `*`** — no Dockerfile (`Dockerfile`, `tests/docker/Dockerfile.*`) uses `COPY`/`ADD`;
  `run_tests.sh`, `run_matrix.sh` and `tests/distro/run_distro_validation.sh` bind-mount the workspace;
  a Dockerfile passed with `-f` is sent regardless of `.dockerignore`. → PASS.

### PAM Concurrency & Deadlines
- No file under `crates/pam` changed. Skills restate the enforced invariant (explicit deadline from
  the clamped `timeout_ms`) and correctly flag the "200–250ms" prose drift. → PASS.

### Panic Safety & Fail-Closed
- Gate scripts: pre-commit/pre-push invoke `./scripts/secret_scan.sh` and
  `./scripts/candid_subagent.sh` directly, so a missing script exits non-zero and blocks. The
  scanner exits 2 on any git failure. CI range computation aborts under `bash -e` if `merge-base`
  fails, and an empty `from` is rejected by the scanner. `ci-success` fails on any non-success
  result, including `skipped`/`cancelled`. → PASS.
- Tried: a `workflow_dispatch` run on a PR head SHA — it produces a `CI Success` check run without
  running layer 2 (step is `pull_request`-only). Requires write access, and the documented
  limitation already states that a PR can modify the gate scripts → MINOR (§4).

### Test Integrity & Anti-Weakening
- §2 listing reviewed; zero Rust test changes. New tester skill defines a "Contract Migration"
  path owned by the tester and justified by a backlog acceptance line; the developer skill forbids
  test edits outright. → PASS.

### Memory, Bounds & Secrets
- Scanner findings print only file and pattern name, never the match. Tokens are passed through
  `grep -qE` on a here-string (no temp file with secret content). `mktemp` names file is removed by
  `trap`. → PASS.

### Supply Chain & Automation
- `actionlint` v1.7.12: 0 findings. `zizmor --offline` v1.30.1: 0 findings. ShellCheck
  `--severity=error` on every `*.sh` and hook (including the new `pre-push` and `secret_scan.sh`):
  0 findings; `bash -n` on all: clean. New executables have mode `100755` in the patch.
- All third-party actions pinned by full SHA with tag comment; `permissions: contents: read`;
  `persist-credentials: false`; PR title, SHAs and `before` passed via `env:` only; `--locked` on
  all Cargo commands; per-job `timeout-minutes`; caches written only from `main`.
- `pr-title.yml`: title via `env:` + `printf`; git-generated subjects rejected before the hook's
  auto-accept; 72-char cap consistent with `save.sh` and `pr_loop.sh`. → PASS.

### English-Only Policy
- Layer 1 language audit passes; manual read of all skills, docs, walkthrough 77, hooks and
  scripts found no non-English text. → PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `.agents/skills/auditor-agent/SKILL.md:55` — claims a generic per-service flag
  `/etc/soos/<svc>.disable`; the code (`crates/pam/src/config.rs` `is_disabled`) honors only
  `/etc/soos/disabled`, `/etc/soos/gdm.disable` (GDM services) and a custom `disable_file`
  argument. Reword to match the code (as `project-facts.md` §3 already does).
- **[MINOR]** `.github/workflows/ci.yml:88` (with `:34`) — a `workflow_dispatch` run on a PR head
  SHA yields a green `CI Success` check run without the layer 2 fingerprint gate, and both
  branch protection and `pr_loop.sh` accept the latest `CI Success` run for the SHA. Needs write
  access; consider running layer 2 for `workflow_dispatch` too (`--rev "$GITHUB_SHA"` on non-main
  refs) or excluding `workflow_dispatch` from `ci-success`.
- **[SUGGESTION]** `scripts/secret_scan.sh:127` — for a typechange, git emits two sections for the
  same path; the second section's `+++ b/<path>` header is emitted as content. Harmless (at worst a
  false positive on a path that looks like a token); could reset `in_hunk` on `^diff --git`.
- **[SUGGESTION]** `scripts/secret_scan.sh:119-123` — binary content with NUL bytes prints bash's
  "ignored null byte in input" warning; cosmetic (piping through `tr -d '\0'` would silence it).

## 5. Final Verdict

**VERDICT: APPROVED**
