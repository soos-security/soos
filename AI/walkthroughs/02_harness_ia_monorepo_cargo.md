# Walkthrough — AI Harness Setup + Cargo Monorepo Workspace

> Date: 2026-09-13  
> Phase: Foundation — Professional AI Harness & Compilable Skeleton  

---

## Summary

Complete setup of AI-driven development infrastructure for `soos`:
- **AI Harness**: Persistent rules, verification matrix, 4-phase TDD workflow.
- **Cargo Monorepo**: Workspace with 2 initial crates (`protocol`, `pam`), 20 passing tests.
- **Quality Pipeline**: `fmt` + `clippy -D warnings` + `test` + automated commit.

---

## Modified & Delivered Files

| File | Purpose | Status |
|---|---|---|
| [`AGENTS.md`](file:///home/hadrien/soos/AGENTS.md) | Persistent rules loaded automatically by AI agents | **NEW** |
| [`.gitignore`](file:///home/hadrien/soos/.gitignore) | Ignores `target/`, ONNX models, biometric data | **NEW** |
| [`AI/DECISIONS.md`](file:///home/hadrien/soos/AI/DECISIONS.md) | ADR register: naming, codec, and panic safety decisions | **MODIFIED** |
| [`AI/VERIFICATION_MATRIX.md`](file:///home/hadrien/soos/AI/VERIFICATION_MATRIX.md) | Component acceptance criteria checklist | **NEW** |
| [`Cargo.toml`](file:///home/hadrien/soos/Cargo.toml) | Workspace root, `resolver = "2"`, centralized dependencies | **NEW** |
| [`rust-toolchain.toml`](file:///home/hadrien/soos/rust-toolchain.toml) | Pinned stable toolchain with clippy and rustfmt | **NEW** |
| [`crates/protocol/Cargo.toml`](file:///home/hadrien/soos/crates/protocol/Cargo.toml) | Protocol crate: IPC types, Postcard/Serde codec | **NEW** |
| [`crates/protocol/src/lib.rs`](file:///home/hadrien/soos/crates/protocol/src/lib.rs) | Protocol crate entry point, `#![forbid(unsafe_code)]` | **NEW** |
| [`crates/protocol/src/types.rs`](file:///home/hadrien/soos/crates/protocol/src/types.rs) | Request, Response, Verdict, ReasonClass, Event schemas | **NEW** |
| [`crates/protocol/src/codec.rs`](file:///home/hadrien/soos/crates/protocol/src/codec.rs) | Bounded binary codec + 16 unit tests | **NEW** |
| [`crates/pam/Cargo.toml`](file:///home/hadrien/soos/crates/pam/Cargo.toml) | PAM crate producing cdylib | **NEW** |
| [`crates/pam/src/lib.rs`](file:///home/hadrien/soos/crates/pam/src/lib.rs) | Skeleton PAM module returning `PAM_IGNORE` + `catch_unwind` + 4 tests | **NEW** |
| [`.agents/skills/dev-workflow/SKILL.md`](file:///home/hadrien/soos/.agents/skills/dev-workflow/SKILL.md) | Multi-agent TDD 4-phase workflow skill | **NEW** |

---

## Validation Results

### `cargo test` — 20/20 Passed
- **Protocol (16 tests)**:
  - Round-trip serialization: Request, Response, Event.
  - Rejection: oversized service name, declared size > 4096 bytes, unsupported protocol version.
  - Validation: 256-bit request IDs, `should_ignore()` helper, `is_allow()` checks.
  - Fault tolerance: empty buffers, short buffers, truncated payloads.
- **PAM (4 tests)**:
  - `authenticate_returns_pam_ignore` (fail-closed invariant).
  - `setcred_returns_pam_ignore`.
  - `pam_ignore_has_correct_value` (Linux-PAM constant match).
  - `panic_safety_returns_pam_ignore` (`catch_unwind` safety barrier).

### Quality Pipeline
- `cargo fmt --check`: Passed.
- `cargo clippy --all-targets -- -D warnings`: 0 warnings.
- `./save.sh`: Automated commit `a5bad02` created.
