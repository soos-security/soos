# Walkthrough — Autonomous PR Loop, Copilot Review, and Auto-Merge

> Date: 2026-09-13  
> Phase: Complete PR Automation & Auto-Merge Pipeline  

---

## Summary

Created an autonomous development pipeline (`scripts/pr_loop.sh` and `./save.sh --auto-merge`) that eliminates human overhead between feature completion and branch merge:
- Runs full quality gates (`fmt`, `clippy -D warnings`, `test`, `deny check`).
- Pushes topic branch to GitHub and opens a Pull Request.
- Solicits automatic review from GitHub Copilot (`@copilot review`).
- Monitors CI jobs in real time until all checks pass.
- Verifies that Copilot has finished reviewing and that zero blocking comments exist.
- Performs an automated squash-merge into `main`, deletes the remote branch, and pulls latest `main` locally.

---

## Deliverables

| File | Purpose |
|---|---|
| [`scripts/pr_loop.sh`](file:///home/hadrien/soos/scripts/pr_loop.sh) | Autonomous PR orchestration script |
| [`save.sh`](file:///home/hadrien/soos/save.sh) | Integrated `--auto-merge` delegation to `pr_loop.sh` |
| [`AGENTS.md`](file:///home/hadrien/soos/AGENTS.md) | Enforces autonomous loop rule until merge |
