# Walkthrough 76 — Claude Code Project Skills Link

> **Branch**: `chore/claude-skills-link`  
> **Status**: Completed  

---

## 1. Overview

The multi-agent workflow skills (`dev-workflow`, `architect-agent`, `tester-agent`, `auditor-agent`, `developer-agent`, `candid-reviewer`, `plan-evaluator`, `traceability-agent`) are versioned under `.agents/skills/`. Claude Code only discovers project skills under `.claude/skills/`, so none of them were available in Claude Code sessions.

---

## 2. Changes Made

- Added the relative symbolic link `.claude/skills -> ../.agents/skills`.
  - A relative target keeps the link valid in every clone and git worktree.
  - `.agents/skills/` remains the single source of truth; no skill file is duplicated.
- Documented the link in `Docs/DEVELOPMENT_WORKFLOW.md` (§5, "Agent Role Skills").

---

## 3. Verification

- `ls .claude/skills/` lists all eight skills.
- `git ls-files -s .claude/skills` reports mode `120000` (symbolic link).
- A new Claude Code session started at the repository root lists the skills as project skills.
