# Walkthrough 15 — Dual Issue and Sub-Issue Synchronization

> Date: 2026-09-13  
> Phase: Task Tracking & GitHub Synchronization  

---

## Summary

To ensure seamless traceability between local project documentation and remote GitHub tracking, this update introduces automated **Dual Issue and Sub-Issue Synchronization**:
1. **Local Tracking**: Automatically marks completed sub-issues (`- [x] **#X.Y**`) in `AI/BACKLOG.md`.
2. **GitHub Issue Tracking**: Automatically checks off sub-issues in the corresponding GitHub Issue body via GitHub API (`gh api`), posts progress comments, and associates Pull Requests with `Closes #<github-id>`.

---

## 1. Synchronization Architecture

```
Developer / Traceability Sub-Agent completes sub-issue (e.g. #1.1)
                                │
                                ▼
                   scripts/sync_issue.py --subissue 1.1
                                │
          ┌─────────────────────┴─────────────────────┐
          ▼                                           ▼
   AI/BACKLOG.md                              GitHub Issue #8
 - [x] **#1.1** Scaffold policy...          - [x] **#1.1** Scaffold policy...
                                            Comment: "Sub-issue #1.1 completed."
```

### Backlog Issue to GitHub Issue Mapping
- Issue #1 (policy crate) $\rightarrow$ GitHub Issue #8
- Issue #2 (daemon skeleton) $\rightarrow$ GitHub Issue #9
- Issue #3 (pam ipc client) $\rightarrow$ GitHub Issue #10
- Issue #4 (protocol fuzz) $\rightarrow$ GitHub Issue #11
- Issue #5 (camera v4l) $\rightarrow$ GitHub Issue #12
- ... through Issue #16 $\rightarrow$ GitHub Issue #23

---

## 2. Integrated Workflow

- **`scripts/sync_issue.py` / `scripts/sync_issue.sh`**:
  - `--subissue X.Y`: Checks off sub-issue locally in `AI/BACKLOG.md` and remotely on GitHub with a progress comment.
  - `--auto`: Inspects current branch, checks off all branch sub-issues, and prepares `Closes #ID`.
- **`save.sh` & `scripts/pr_loop.sh`**:
  - Automatically appends `Closes #<github-issue-id>` to PR descriptions so GitHub automatically closes the issue upon merge.
- **`traceability-agent`**:
  - Mandated to invoke `sync_issue.py` upon task realization.

---

## 3. Verification

- `python3 scripts/sync_issue.py --help`: Validated.
- Syntax and script permissions: Tested and clean.
