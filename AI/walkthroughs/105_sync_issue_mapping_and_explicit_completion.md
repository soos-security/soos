# Walkthrough 105 — Issue Sync Mapping Self-Check and Explicit Completion

- **Date**: 2026-09-30
- **Issue**: Review finding TCI-05 (GitHub #188)
- **Branch**: `fix/sync-issue-mapping`
- **Matrix criteria**: SIT1–SIT5 (new)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`) confirmed three defects in
`scripts/sync_issue.py` and its callers:

1. **Two backlog issues numbered #22.** `AI/BACKLOG.md` had `### Issue #22 — feat(camera-v4l):
   Automatic format negotiation` and `### Issue #22: admin-cli — Visual Debugging GUI`, both with
   sub-issues `#22.1`–`#22.4`. `BRANCH_TO_ISSUE` sent `feat/camera-format-negotiation`,
   `feat/admin-debug-gui` and `feat/guided-enrollment-production-unlock` to #22, and
   `BACKLOG_TO_GITHUB[22] = 61`, so GUI branches ticked the camera issue on GitHub.
2. **Sync after push.** `save.sh` ran `git commit`, `git push` and only then
   `sync_issue.py --auto`, which rewrote `AI/BACKLOG.md` in the working tree. The edit was never
   part of the PR and the dirty tree made `scripts/pr_loop.sh` refuse to update `main`.
3. **Complete-all on any push.** `--auto` checked off every sub-issue of the branch with no
   completion check, and `|| true` hid every failure.

## 2. Mapping Verification (read-only)

Every `BACKLOG_TO_GITHUB` value was compared with the GitHub issue titles (`gh issue list --state
all`, `gh api repos/.../issues/<n>`; nothing on GitHub was edited):

- Backlog #1–#16 → GitHub #8–#23, #17–#35 → #56–#74 (titles prefixed `[17]`…`[35]`), #36–#45 →
  #102–#111, #46 → #132, #47 → #134, #48 → #136, #49 → #138: titles match.
- `22: 61` is the **camera** issue (`[22] feat(camera-v4l): Automatic format negotiation and NV12
  support`). No GitHub issue exists for the GUI work.
- `50: 140` pointed at **pull request** #140 (`fix(gui): camera auto-resolution, video feed
  fallback and packaging`), not an issue. The mapping is removed; the matrix header now reads
  `Issue #50 / PR #140; no GitHub issue`.

## 3. Changes

### `AI/BACKLOG.md`, `AI/VERIFICATION_MATRIX.md`
- The GUI heading is renumbered `### Issue #51` with sub-issues `#51.1`–`#51.4` and a numbering
  note; the P1 table row follows. The camera `#22` and the code comments that cite `#22.2`–`#22.4`
  (camera tests) are unchanged.
- Matrix header `guided-enrollment-production-unlock` now cites `Issue #51 … no GitHub issue`.

### `scripts/sync_issue.py`
- `BRANCH_TO_ISSUE`: both GUI branches → 51. `BACKLOG_TO_GITHUB`: `50: 140` removed.
- `--check`: offline self-check (no `gh`, no git) that fails on duplicated `### Issue #N`
  headings, duplicated sub-issue ids, two backlog ids sharing one GitHub target, invalid targets and
  table entries naming an unknown backlog id. Every syncing mode runs it first.
- `--auto` never writes the backlog: it reports the branch's open sub-issues and mirrors only the
  already-checked ones to GitHub. An unregistered branch is an announced no-op.
- Completion is explicit (`--subissue`, `--issue N --complete-all`) and scoped to the heading's
  own section. `--local-only` skips GitHub; `--backlog` selects the file (default: resolved from
  the script location); `--print-github-issue` prints the mapped issue.
- GitHub calls use `shutil.which("gh")` (the hard-coded personal home-directory fallback is gone) with a 30 s
  timeout. The body is only PATCHed after a successful read, and the comment only posted after a
  successful PATCH. Failure exits 2 with `GitHub sync failed`; self-check or usage failure exits 1.

### `save.sh`, `scripts/pr_loop.sh`
- `save.sh` runs `sync_issue.py --check` (fatal) and `--auto --local-only` **before** `git add .`.
- After the push, both scripts run `--auto --branch "$CURRENT_BRANCH"` without `|| true`; a failure
  prints a warning and the re-run command, and the loop continues (the branch is already pushed).

### GitHub-only branches
Review-finding branches (GitHub issue, no backlog id) remain intentionally unregistered: `--auto`
says so and does nothing, and the commit message carries `Closes #N`. This is documented in
`Docs/DEVELOPMENT_WORKFLOW.md` §8, the `dev-workflow` and `traceability-agent` skills and
`project-facts.md`.

## 4. Tests (written first)

`tests/invariants/src/sync_issue_contract.rs` (9 tests). Scratch copies go under
`target/sync_issue_contract/`; the gh-failure case puts a fake `gh` that always exits 1 first on
`PATH`, so no test touches the network.

| Test | Criterion |
|---|---|
| `test_sync_issue_check_passes_on_repository` | SIT1 |
| `test_sync_issue_check_rejects_duplicate_github_target` | SIT2 (copy with `BACKLOG_TO_GITHUB[23] = BACKLOG_TO_GITHUB[22]`) |
| `test_sync_issue_check_rejects_unknown_backlog_id` | SIT2 (copy with a branch → #999) |
| `test_sync_issue_check_rejects_duplicate_backlog_heading` | SIT2 |
| `test_sync_issue_auto_local_only_never_rewrites_backlog` | SIT3 |
| `test_sync_issue_auto_unregistered_branch_is_a_visible_noop` | SIT3 |
| `test_sync_issue_gh_failure_is_visible_and_non_destructive` | SIT4 |
| `test_save_sh_runs_sync_check_before_staging` | SIT5 |
| `test_release_scripts_never_silence_sync_failures` | SIT5 |

**Red evidence**: before the implementation all 9 failed on their assertions (argparse rejected
`--check` / `--backlog`, `save.sh:429` used `|| true`, `save.sh` had no `--check`). With the new
script but before the renumbering, `--check` on the repository reported:

```
[FAIL] sync_issue self-check: duplicate backlog heading #22 (2 '### Issue #22' headings)
[FAIL] sync_issue self-check: duplicate sub-issue #22.1 (2 occurrences)
...
[FAIL] sync_issue self-check: branch 'feat/admin-debug-gui' maps to unknown backlog issue #51
```

**Green**: all 9 pass; `--check` prints `49 GitHub targets, 55 branches, backlog headings and
sub-issues unique`.

## 5. Out of Scope / Follow-ups

- TCI-08 (GitHub #239): `refactor/` and `docs/` historical branch names remain in
  `BRANCH_TO_ISSUE`; they are valid lookups for old branches and are not renamed here.
- TCI-07 (GitHub #238): the root scratch files (`create_issues.py`, `patch_sync.py`, …) are not
  removed here; only the hard-coded home-directory `gh` fallback inside `sync_issue.py` is.
