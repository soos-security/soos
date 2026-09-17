# Walkthrough 37: Persisted Async Cancellation Safety & Socket Write Decoupling Skills

> **Topic**: Post-Issue #21 Learning & Skill Persistence (`/learn`)  
> **Branch**: `chore/skills-async-cancel-invariants`  
> **Status**: Completed & Verified  

---

## 1. Context & Rationale

Following the completion and verification of **Issue #21 (GitHub #60)** (`fix(daemon): Async cancellation safety on socket writes`), we identified recurring architectural hazards and patterns in Tokio-based systems with framed streaming sockets:
1. **Async Cancellation Hazard**: Wrapping async connection loops with `stream.write_all` in the same timeout that bounds pipeline inference allows timeout cancellations to drop `write_all` mid-transmission, polluting the socket buffer with partial frames and causing client deserialization failure.
2. **Client-Side Truncation Detection**: Standard `Read::read_exact` flattens premature disconnects into generic `UnexpectedEof`, losing the critical distinction between zero-byte EOF and partial frame truncation.
3. **Fail-Closed Fallback Invariant**: Any partial framing or client-side detection failure must systematically fallback to `PAM_IGNORE` rather than granting authentication or crashing.

To ensure all future sub-agents automatically enforce these architectural invariants and testing patterns, the directives were persisted across the project's agent skills.

---

## 2. Updated Skills

### A. Developer Sub-Agent (`.agents/skills/developer-agent/SKILL.md`)
Added **Section 8: Async Cancellation Safety & Decoupled Socket Writes**:
- **Decouple Processing from Transmission**: Never wrap an async connection handler containing `stream.write_all` inside the single timeout governing request processing.
- **In-Memory Wire Serialization Phase**: Run request reading, credential checking, and verification to completion, encoding the wire frame into `Option<Vec<u8>>` under `connection_timeout` with 0 socket writes.
- **Dedicated Transmission Phase**: Write the pre-encoded response via `write_all` and `flush` after processing completes, protected by an independent write timeout.
- **Counted Frame Reading in Clients**: Synchronous stream clients (`pam_soos.so`) must use byte-counted reading (`read_exact_counted`) and validate `total_received == expected_total`.

### B. Architect Sub-Agent (`.agents/skills/architect-agent/SKILL.md`)
Added **Async Cancellation Safety Invariant** to Directive 3:
- Enforces two-phase decoupled architectures in server components (`soos-daemon`) during interface specification.

### C. Auditor Sub-Agent (`.agents/skills/auditor-agent/SKILL.md`)
Added **Async Cancellation & Write Isolation Audit** to Directive 4:
- Directs static pre-implementation audit to verify that `write_all` is never wrapped inside request processing timeouts and that client stream readers validate frame completeness.

### D. Tester Sub-Agent (`.agents/skills/tester-agent/SKILL.md`)
Added **Async Cancellation & Truncated Framing Tests** to Directive 3:
- Directs test authors to include adversarial tests ensuring daemon timeouts drop connections with 0 bytes sent without partial response framing, and client stream readers detect truncated frames and fail closed to `PAM_IGNORE`.

---

## 3. Verification & Compliance

1. **Git Cleanliness**: All 4 skill files updated with clean diffs.
2. **Language Policy**: 100% English deliverables, strictly formatted in GitHub Flavored Markdown.
3. **Workspace Invariants**: Verified that no production Rust code or tests were weakened or altered.
