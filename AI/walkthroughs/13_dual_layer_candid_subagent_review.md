# Walkthrough 13 — Dual-Layer Candid Sub-Agent Review Architecture

> Date: 2026-09-13  
> Phase: AI Harness & Autonomous Review Sub-Agent Architecture  

---

## Summary

In response to the requirement that code changes must be audited by an **AI sub-agent reasoning with a fresh, unpolluted perspective** rather than relying solely on deterministic scripts, this update implements a **Dual-Layer Candid Review Architecture**:
1. **Layer 1: Deterministic Static Invariant Gate (`scripts/candid_review.sh`)**:
   - Ultra-fast regex/grep static analysis asserting zero `unsafe` in business crates, zero `unwrap`/`expect`/`panic!` in PAM, zero Tokio in PAM, zero OpenCV/Nokhwa, shell syntax validity, PAM output isolation (no prints), and strict English policy.
2. **Layer 2: AI Sub-Agent Reasoning Gate (`.agents/skills/candid-reviewer/SKILL.md` & `AI/candid_review_report.md`)**:
   - A dedicated **Candid Reviewer Sub-Agent** operating with clean context and zero author bias on the raw diff (`target/candid_diff.patch`).
   - Deeply analyzes logic, state transitions, PAM real-time deadlines, panic safety, test integrity, and memory bounds across 5 core pillars.
   - Outputs a formal structured report in `AI/candid_review_report.md` with an explicit `VERDICT: APPROVED` or `VERDICT: CHANGES_REQUESTED`.

Both layers are orchestrated seamlessly via `scripts/candid_subagent.sh`, which is integrated directly into `.githooks/pre-commit`, `./save.sh`, and `dev-workflow`.

---

## 1. The 5 Reasoning Pillars of the Candid Reviewer Sub-Agent

Documented in `.agents/skills/candid-reviewer/SKILL.md`:
- **Pillar 1: Logic & Architectural Soundness**: Validates state machines, edge cases, off-by-one errors, and alignment with `AI/BACKLOG.md`.
- **Pillar 2: PAM Concurrency & Real-Time Deadlines**: Validates synchronous execution, 200–250ms timeouts, absence of Tokio, and zero terminal pollution.
- **Pillar 3: Panic Safety & Fail-Closed Behavior**: Verifies all FFI entries are wrapped in `catch_unwind` and systematically return `PAM_IGNORE` on any error or panic.
- **Pillar 4: Strict Test Integrity (Zero Weakening)**: Asserts that pre-existing tests were not weakened, altered, or bypassed to match faulty production code.
- **Pillar 5: Memory Safety, Bounds & Secrets**: Asserts bounded allocations (4,096 bytes), zeroization of sensitive buffers, and absence of passwords or embeddings in schemas or logs.

---

## 2. Updated TDD Lifecycle

The `dev-workflow` skill now includes Phase 5 before merging:
```
Phase 1: Architect (Specification & Types)
   ↓
Phase 2: Tester (TDD Red Phase — Tests Must Fail)
   ↓
Phase 3: Auditor (Static Invariant Audit)
   ↓
Phase 4: Developer (TDD Green Phase — Zero Test Weakening)
   ↓
Phase 5: Candid Reviewer Sub-Agent (Fresh Reasoning Audit on Raw Diff)
   ↓
Autonomous PR & Auto-Merge Loop (./save.sh --auto-merge)
```

---

## 3. Verification Results

All automated gates passed cleanly:
```text
── Candid Review Layer 1: Deterministic Invariant Checks ──
[OK]    Candid Review PASSED: All architectural invariants verified!

── Candid Review Layer 2: AI Sub-Agent Reasoning Gate ──
[INFO]  Raw diff size: 81 line(s) saved to target/candid_diff.patch
[OK]    Dual-Layer Candid Review PASSED!
[OK]    ✓ Layer 1: All deterministic architectural invariants verified
[OK]    ✓ Layer 2: AI Sub-Agent reasoning audit approved (AI/candid_review_report.md)
```
