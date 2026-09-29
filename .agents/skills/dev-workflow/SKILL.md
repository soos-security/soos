---
name: dev-workflow
description: >
  Master orchestrator for soos development. Use when asked to implement,
  fix or deliver a backlog issue (e.g. "Issue #50", "#12", a branch name) end
  to end: topic branch → architect → plan-evaluator → tester → auditor →
  developer → candid-reviewer → traceability → quality gates → PR → CI →
  squash merge. Coordinates the specialized sub-agent skills and enforces the
  gate between every phase.
---

# Master Development Workflow — soos

Shared facts (crate map, constants, commands, conventions):
[`references/project-facts.md`](references/project-facts.md). Read it first.

## 0. Preconditions

- Ingest `AGENTS.md`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, the target issue in `AI/BACKLOG.md`,
  its rows in `AI/VERIFICATION_MATRIX.md`, `AI/MOCK_STRATEGY.md`, `AI/ROLES_AND_WORKFLOW.md`,
  `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `Docs/DEVELOPMENT_WORKFLOW.md`,
  `Docs/COMMIT_CONVENTION.md`, `Docs/IPC_PROTOCOL.md`, `Docs/CI_CD_AND_SECURITY.md`, and the
  `Docs/*` page of each affected crate.
- Working tree clean and up to date: `git fetch origin && git status --short` is empty.
- `git config core.hooksPath` prints `.githooks` (run `git config core.hooksPath .githooks` if not).

## 1. Phase Pipeline and Gates

| Phase | Skill | Output | Gate to continue |
|---|---|---|---|
| 0 Branch | — | `<type>/<name>` from BACKLOG | branch registered in `scripts/sync_issue.py` |
| 1 Spec | `architect-agent` | Architect Spec | every acceptance line mapped |
| 1.5 Plan gate | `plan-evaluator` | `AI/plan_evaluator_report.md` | `VALIDATION_VERDICT: APPROVED` |
| 2 Red | `tester-agent` | tests + contract table | tests fail on assertions/specified API only |
| 3 Audit | `auditor-agent` | constraint list | `Clearance: CLEARED` |
| 4 Green | `developer-agent` | implementation | fmt + clippy + test + candid layer 1 green |
| 5 Review | `candid-reviewer` (fresh context) | `AI/candid_review_report.md` | `./scripts/candid_subagent.sh` passes |
| 6 Trace | `traceability-agent` | matrix, backlog, Docs, walkthrough NN | self-check green |
| 7 Release | `./save.sh --auto-merge -m "<conventional message>"` | PR merged | CI `CI Success` green |

A failed gate loops back to the phase that owns the defect (plan → architect, weak test → tester,
failing code → developer). Never skip a phase because the change "looks small"; for a docs-only
or CI-only change, phases 1–4 may be condensed into one written plan, but 5–7 always run.

## 2. Phase Details

### Phase 0 — Topic branch
- Resolve ambiguous numbers (`#12` may be backlog #12 or GitHub #12) with `BACKLOG_TO_GITHUB` /
  `BRANCH_TO_ISSUE` in `scripts/sync_issue.py`.
- Branch name from the issue's `> **Branch**:` line; allowed prefixes `feat/ fix/ test/ chore/`.
  `git switch -c <type>/<name> origin/main`.
- Register `"<type>/<name>": <backlog_id>` in `BRANCH_TO_ISSUE` if absent. Do **not** run
  `sync_issue.py --auto` now — it checks off every sub-issue of the branch.

### Phases 1–4
Invoke each skill in order and keep their deliverables in the conversation/plan; they feed the
walkthrough. The developer must run the exact CI commands (with `--locked --all-features`).

### Phase 5 — Candid review
Run the reviewer as an independent sub-agent when the harness supports it, giving it only: the
skill, the branch name and the instruction to start with `./scripts/candid_subagent.sh --prepare`.
If it returns CHANGES_REQUESTED, fix production code and run a **new** review (the fingerprint
changes with every code change; a stale report is rejected by the gate, the pre-push hook and CI).
Phase 6 edits only docs, but they are part of the diff: run `--prepare` + gate again after Phase 6
if any file changed (docs are reviewed for accuracy and English policy too).

### Phase 6 — Traceability
Per the `traceability-agent` skill, then `python3 scripts/sync_issue.py --auto` once all sub-issues
are complete.

### Phase 7 — Release loop
```bash
./save.sh --auto-merge -m "<type>(<scope>): <imperative summary ≤ 72 chars>"
```
- `save.sh` runs fmt, clippy, tests, cargo-deny and both candid layers; commits; `scripts/pr_loop.sh`
  pushes (pre-push hook re-verifies the review fingerprint), opens the PR, watches CI with fail-fast,
  and squash-merges with `--match-head-commit` so nothing unreviewed can be merged.
- The PR title becomes the squash commit subject and is validated by CI against
  `.githooks/commit-msg`.
- While `save.sh --auto-merge` / `pr_loop.sh` runs, do not switch branches or edit the working tree.
- If CI fails: read the failing job log (`gh run view --log-failed`), fix on the same branch, re-run
  Phase 5 (new fingerprint), then re-run `./save.sh --auto-merge`.

## 3. Environment Notes

- Sandboxed harnesses: branch creation, `cargo fetch` of new dependencies, `gh` and pushes need
  write access to `.git` and network — request the harness's elevated/unsandboxed execution for
  those commands only.
- `gh` may live in `~/.local/bin`; prepend `PATH="$HOME/.local/bin:$PATH"` if not found.
  Query issues with explicit fields: `gh issue view <id> --json title,body,number,state`.
- Worktrees share the git stash: never use bare `git stash`/`git stash pop`.
- Write repository files directly in the working tree, never in an agent-private artifact folder.

## 4. Completion

Done means: PR squash-merged, local `main` fast-forwarded (`git log -1 origin/main` shows the
squash commit), issue checkboxes synced, walkthrough NN present. Under an autonomous `/goal` run,
end the final summary with `<!-- GOAL_COMPLETE -->` after verifying the merge.
