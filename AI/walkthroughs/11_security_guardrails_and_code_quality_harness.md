# Walkthrough 11 — Security Guardrails & Code Quality Harness

> Date: 2026-09-13  
> Phase: Pre-Implementation Security Hardening & AI Harness Hardening  

---

## Summary

In preparation for implementing the core authentication and daemon crates, this update establishes an extensive suite of pre-implementation security guardrails and code-quality gates. Grounded in official Rust security documentation (**The Rust Reference**, **The Rustonomicon**, **The Cargo Book**, and **Clippy Restriction Guidelines**), the workspace now centrally enforces compiler profile hardening, strict clippy restriction lints, expanded supply-chain bans, pre-commit candid review gating, and full English-only deliverable compliance.

---

## 1. Compiler Profile Hardening

Configured centrally in root `Cargo.toml`:
- `overflow-checks = true` enabled in both `[profile.release]` and `[profile.dev]`. This eliminates silent two's-complement integer wrapping in release builds, preventing buffer or size-calculation bypasses.
- `opt-level = 3`, `lto = true`, and `codegen-units = 1` for whole-program optimization.
- `panic = "abort"` in release mode as defense-in-depth against unwinding outside `catch_unwind`.
- `strip = "symbols"` to minimize exposed shared object metadata.

---

## 2. Centralized Workspace Lints (`[workspace.lints]`)

Defined in root `Cargo.toml` and inherited across all crates (`[lints] workspace = true`):
- **Panics & Stubs (Deny)**: `unwrap_used`, `expect_used`, `panic`, `panic_in_result_fn`, `unimplemented`, `todo`, `unreachable`.
- **Memory & Unsafe Isolation (Deny)**: `undocumented_unsafe_blocks` (requires `// SAFETY:` rationale on all unsafe blocks), `mem_forget`.
- **PAM Terminal Isolation (Deny)**: `print_stdout`, `print_stderr`, `dbg_macro`.
- **Accountability & Quality (Deny)**: `allow_attributes_without_reason` (any `#[allow(...)]` must specify `reason = "..."`), `fallible_impl_from`.
- **Numerical & Bounds Safety (Warn)**: `indexing_slicing`, `arithmetic_side_effects`, `cast_possible_truncation`, `cast_possible_wrap`, `cast_sign_loss`.

---

## 3. Supply Chain Hardening (`deny.toml`)

- Explicitly banned `nokhwa` (unapproved camera crate) alongside `opencv`.
- Cleaned up license matching (`unused-allowed-license = "allow"`).
- Translated all residual French comments to English.

---

## 4. AI Harness Upgrades

- **`scripts/candid_review.sh`**:
  - Added detection of `panic!`, `todo!`, `unimplemented!`, `unreachable!`, and `dbg!` in production code.
  - Added detection of `println!`, `eprintln!`, `print!`, `eprint!` in `crates/pam/src`.
  - Expanded English-only scanner to inspect `.yml`, `.toml`, `Dockerfile`, `.gitignore`.
  - Elevated non-English detections to blocking errors.
- **`.githooks/pre-commit`**:
  - Automatically executes `scripts/candid_review.sh` on staged files, ensuring no commit bypasses the architectural audit.
- **`run_tests.sh`**:
  - Fixed crate package name flag from `-p pam` to `-p soos-pam`.
- **`.github/workflows/ci.yml`**:
  - Translated all comments to English.
  - Added `./scripts/candid_review.sh` execution step to the `quality` job.
- **`tests/invariants`**:
  - Added Invariant 6: Assert `nokhwa` ban in all `Cargo.toml` files.
  - Added Invariant 7: Assert `overflow-checks = true` in workspace `Cargo.toml`.
  - Added Invariant 8: Assert zero `println!` or `eprintln!` in `crates/pam/src`.

---

## 5. Verification Results

All automated gates passed cleanly:
1. `cargo clippy --all-targets --all-features -- -D warnings`: 0 warnings, 0 errors.
2. `cargo test --all-targets`: 8 invariant tests, 4 pam tests, 16 protocol tests (all passed).
3. `cargo deny check`: Advisories ok, bans ok, licenses ok, sources ok.
4. `./scripts/candid_review.sh`: All 8 audits passed.
