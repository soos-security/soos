# Walkthrough 14 — Master Sub-Agent Orchestration Architecture

> Date: 2026-09-13  
> Phase: AI Harness & Multi-Sub-Agent Orchestration  

---

## Summary

In response to the user's objective to unify the entire development lifecycle under a single skill invocation, this update establishes the **Master Sub-Agent Orchestration Architecture**. The `dev-workflow` skill now acts as the master conductor that seamlessly coordinates 6 specialized sub-agents, each with dedicated skills in `.agents/skills/`:

```
dev-workflow (Master Orchestrator)
 ├── Phase 1: architect-agent    (Scaffolding, Bounded Schemas, Interfaces)
 ├── Phase 2: tester-agent       (TDD Red Phase, Contract Tests, Anti-Weakening)
 ├── Phase 3: auditor-agent      (Static Security, Panics, Unsafe, Output Isolation)
 ├── Phase 4: developer-agent    (TDD Green Phase, Minimalist Implementation)
 ├── Phase 5: candid-reviewer    (Cold Diff Reasoning Audit, Report Verdict)
 ├── Phase 6: traceability-agent (Verification Matrix, Docs, Walkthrough)
 └── Phase 7: pr_loop / save.sh  (Autonomous CI Monitoring & Auto-Merge)
```

---

## 1. The Sub-Agent Ensemble

1. **[architect-agent](file:///home/hadrien/soos/.agents/skills/architect-agent/SKILL.md)**: Scaffolds crates, specifies bounded types, error enums, and trait abstractions.
2. **[tester-agent](file:///home/hadrien/soos/.agents/skills/tester-agent/SKILL.md)**: Authors failing tests first (Red Phase), enforces fail-closed `PAM_IGNORE` fallback, and establishes the immutable test contract.
3. **[auditor-agent](file:///home/hadrien/soos/.agents/skills/auditor-agent/SKILL.md)**: Audits designs for panics (`unwrap`/`expect`), `#![forbid(unsafe_code)]`, zero stdout prints in PAM, and credential protection.
4. **[developer-agent](file:///home/hadrien/soos/.agents/skills/developer-agent/SKILL.md)**: Implements production code to achieve green tests without ever weakening pre-written tests.
5. **[candid-reviewer](file:///home/hadrien/soos/.agents/skills/candid-reviewer/SKILL.md)**: Impartially audits raw diffs on 5 critical pillars and authors `AI/candid_review_report.md`.
6. **[traceability-agent](file:///home/hadrien/soos/.agents/skills/traceability-agent/SKILL.md)**: Synchronizes `AI/VERIFICATION_MATRIX.md`, technical docs in `Docs/`, and writes sequential walkthroughs.

---

## 2. Zero-Friction User Experience

The user only needs to invoke:
> *"Use `dev-workflow` for Issue #N: <name>"*

The orchestrator manages the entire delegation sequence through merge without human intervention.
