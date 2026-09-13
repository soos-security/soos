# Walkthrough — Copilot Dual-Trigger & Robust PR Automation

> Date: 2026-09-13  
> Phase: Reliable AI Review Orchestration  

---

## Summary

Resolved unreliable review triggering by establishing a dual-trigger mechanism for GitHub Copilot in `scripts/pr_loop.sh`:
- **GraphQL Mutation**: Programmatically assigns Copilot to the PR reviewers section via `requestReviews(botIds: ["BOT_kgDOCnlnWA"])`.
- **Comment Trigger**: Posts `@copilot review` to initiate the review agent immediately.
- **Stash Safety**: Preserves untracked files during post-merge checkout of `main`.
