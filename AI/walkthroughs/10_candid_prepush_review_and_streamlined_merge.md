# Walkthrough 10 — Candid Pre-Push Code Review & Streamlined Merge

> Date: 2026-09-13  
> Phase: Quality Architecture & Zero-External-Dependency Autonomous Merge  

---

## Summary

Following exhaustion of external GitHub Copilot quota and user guidance, this update replaces the external review dependency with an independent **Candid Pre-Push Code Review** subagent/harness and streamlines the Pull Request merge process to auto-merge immediately once GitHub Actions CI checks are 100% green.

---

## 1. The Candid Pre-Push Review Architecture

### Motivation:
When developing code, the author or development agent operates under "author bias" — knowing the intent behind changes and potentially overlooking subtle logical errors, edge cases, or invariant violations. External bots also introduce network, quota, and latency bottlenecks.

### The Solution:
An impartial, context-free audit mechanism (`scripts/candid_review.sh`) integrated directly into `./save.sh`:
- **Cold Review of Raw Diff**: Evaluates only the diff against `origin/main` without author conversation history or justifications.
- **Automated Invariant Gating**:
  1. `#![forbid(unsafe_code)]`: Asserts zero `unsafe` in `crates/protocol`, `crates/policy`, `crates/vision`.
  2. Panic Safety: Asserts zero `.unwrap()` or `.expect()` in `crates/pam/src/`.
  3. Strict Concurrency Bound: Asserts zero Tokio or async dependencies in `crates/pam`.
  4. Dependency Whitelist: Asserts zero OpenCV or forbidden camera dependencies across the workspace.
  5. Shell Robustness: Asserts `bash -n` syntax validity across all shell scripts.
  6. Strict Language Policy: Audits all diff additions for English-only comments, docstrings, and technical documentation.

---

## 2. Streamlined PR Loop & CI Auto-Merge

`scripts/pr_loop.sh` has been refactored from 6 steps into a streamlined 4-step pipeline:

```
┌────────────────────────────────────────────────────────┐
│ Step 1: Local Quality Gates + Candid Pre-Push Review   │
│ (fmt, clippy, tests, deny, scripts/candid_review.sh)   │
└──────────────────────────┬─────────────────────────────┘
                           │
┌──────────────────────────▼─────────────────────────────┐
│ Step 2: Pull Request Verification or Creation          │
└──────────────────────────┬─────────────────────────────┘
                           │
┌──────────────────────────▼─────────────────────────────┐
│ Step 3: Monitoring CI Checks (GitHub Actions)          │
│ (Quality, Security, PAM Docker)                        │
└──────────────────────────┬─────────────────────────────┘
                           │ All Green
┌──────────────────────────▼─────────────────────────────┐
│ Step 4: Streamlined Auto-Merge to Main                 │
│ (Squash merge, branch cleanup, sync local main)        │
└────────────────────────────────────────────────────────┘
```

External review requests (Copilot GraphQL bot IDs, `@copilot review` comments, and review polling) have been completely removed. Pull Requests are merged automatically as soon as all GitHub Actions CI checks pass.

---

## 3. Verification

- `scripts/candid_review.sh`: Executable and passing with zero violations.
- `save.sh`: Updated with Step 5/5 candid review.
- `scripts/pr_loop.sh`: Validated with `bash -n`.
- `AGENTS.md` and `SKILL.md`: Synchronized in professional English.
