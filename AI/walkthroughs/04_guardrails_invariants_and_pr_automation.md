# Walkthrough — Security Invariants, Pre-Commit Guardrails, and PR Automation

> Date: 2026-09-13  
> Phase: Security Guardrails & Automated PR Loop  

---

## Summary

Implemented architectural invariant tests, local Git hook protection, and automated Pull Request creation:
- **Architectural Invariant Tests (`tests/invariants/`)**: Fail-closed integration tests validating that business crates forbid unsafe code, the PAM module has no `unwrap`/`expect` or Tokio dependencies, OpenCV is completely banned, and no sensitive credentials leak into IPC structs.
- **Pre-Commit Hook (`.githooks/pre-commit`)**: Physically blocks direct commits on `main` and scans staged files for private keys and tokens.
- **Push & PR Automation (`save.sh --push-pr`)**: Runs all quality gates, triggers pre-commit validation, pushes to origin, and opens a Pull Request via GitHub CLI (`gh`).

---

## Deliverables

| File | Purpose |
|---|---|
| [`tests/invariants/Cargo.toml`](file:///home/hadrien/soos/tests/invariants/Cargo.toml) | Test harness for architectural security invariants |
| [`tests/invariants/src/lib.rs`](file:///home/hadrien/soos/tests/invariants/src/lib.rs) | 5 automated fail-closed invariant test suites |
| [`.githooks/pre-commit`](file:///home/hadrien/soos/.githooks/pre-commit) | Local git hook blocking main commits and secret leaks |
| [`save.sh`](file:///home/hadrien/soos/save.sh) | Integrated `--push-pr` flag for automated PR generation |

---

## Verification Results

- `cargo test --all-targets`: 25 passed (5 invariants + 4 PAM + 16 protocol).
- Invariant 1: `#![forbid(unsafe_code)]` in all business crates.
- Invariant 2: Zero `unwrap()` or `expect()` in PAM production code.
- Invariant 3: Zero Tokio dependencies in PAM crate.
- Invariant 4: Zero OpenCV references across all `Cargo.toml` files.
- Invariant 5: No password, secret, embedding, or frame fields in IPC schemas.
