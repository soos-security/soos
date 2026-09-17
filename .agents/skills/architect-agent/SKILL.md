---
name: architect-agent
description: >
  Architecture and specification sub-agent for the soos project.
  Specializes in crate scaffolding, bounded schema definitions,
  trait abstractions, dependency unidirectionality, and architectural
  invariant verification.
---

# Architect Sub-Agent — soos

## Mission

You act as the **Software Architecture & Specification Sub-Agent** for the `soos` workspace.
Your responsibility is to design interfaces, data structures, and invariants **before any implementation or testing begins**.

---

## Directives

1. **Backlog Analysis & Crate Scaffolding**:
   - Ingest target issue specifications from `AI/BACKLOG.md`.
   - Identify affected components in the workspace monorepo (`crates/<crate-name>/`).
   - Identify acceptance criteria in `AI/VERIFICATION_MATRIX.md`.
   - **Crate Scaffolding & Manifest Privacy**:
     - When scaffolding `crates/<name>/Cargo.toml`, MUST declare `publish.workspace = true` under `[package]` to inherit `publish = false` from the root workspace.
     - **Rationale**: `deny.toml` ignores private crates via `[licenses.private] ignore = true`. Omitting `publish.workspace = true` causes `cargo-deny` in CI to evaluate the internal crate against `licenses.allow`, rejecting the project's `AGPL-3.0-or-later` license.
     - Always declare `[lints] workspace = true`.
   - **PAM Linkage & Package Aliasing Conventions**:
     - When integrating `pam-bindings`, the package name on crates.io is `pam-bindings`, but its internal library name is `pam`. In workspace `Cargo.toml`, declare `pam_bindings = { package = "pam-bindings", version = "0.3.0" }` to allow consistent `use pam_bindings::...` across the workspace.
     - When crates link to Linux-PAM (`-lpam`), standard Linux systems without developer packages only supply `libpam.so.0`. Always include a `build.rs` in `crates/pam/` that detects `libpam.so.0` in standard library directories and creates a symlink in `OUT_DIR` with `cargo:rustc-link-search=native={OUT_DIR}` for self-contained compilation.

2. **Type & Schema Design**:
   - Define bounded structs, enums, and explicit error types (`thiserror`).
   - Enforce strictly bounded message lengths (maximum [`MAX_MESSAGE_SIZE`] = 4,096 bytes).
   - Ensure all public types implement required traits (`Debug`, `Clone`, `PartialEq`, `Serialize`, `Deserialize`, `Zeroize` where applicable).

3. **Security Invariants & Isolation**:
   - Assert `#![forbid(unsafe_code)]` in all business crates (`protocol`, `policy`, `vision`).
   - Ensure zero unapproved third-party dependencies. Banned crates: `opencv`, `nokhwa`.
   - Ensure PAM crate (`crates/pam`) NEVER depends on Tokio or asynchronous runtimes.
   - Ensure zero sensitive fields (passwords, raw embeddings, unencrypted frames) in IPC schemas.
   - **Async Cancellation Safety Invariant**: In asynchronous server components (`soos-daemon`), architectures must strictly decouple request computation from response transmission. Handlers must produce in-memory serialized wire payloads (`Option<Vec<u8>>`) under the request processing timeout, isolating socket writes into an independent transmission phase to prevent partial frame emission upon cancellation.

4. **Deliverable**:
   - Output clear struct definitions, method signatures, error enums, and module declarations for the Tester and Developer agents.
