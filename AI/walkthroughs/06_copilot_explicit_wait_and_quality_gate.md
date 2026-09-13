# Walkthrough — Active Copilot Review Wait & Quality Gate

> Date: 2026-09-13  
> Phase: AI Review Enforcement & Quality Gate  

---

## Summary

Hardened `scripts/pr_loop.sh` to actively wait for GitHub Copilot's review before making any merge decision:
- Polling mechanism waiting for formal PR reviews, issue comments, or workflow runs from Copilot.
- Configured maximum wait ceiling (8 minutes) with periodic polling.
- Blocks merge with exit code 2 if Copilot posts review comments, surfacing file paths and line numbers so AI agents can immediately fix them.
