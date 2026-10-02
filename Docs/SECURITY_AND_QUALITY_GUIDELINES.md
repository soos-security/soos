# Security & Code Quality Guidelines

This document specifies the technical security guidelines, code-quality gates, and hardening configurations implemented across the `soos` workspace.

Because `soos` develops a biometric Linux PAM module (`pam_soos.so`) that runs directly inside the address space of privileged host processes (`login`, `gdm`, `sudo`, `polkit-1`, `sshd`), standard application-level robustness is insufficient. High-assurance systems programming practices derived from the **Rust Reference**, **The Rustonomicon**, **The Cargo Book**, and **Clippy Restriction Guidelines** are strictly enforced.

---

## 1. Threat Model & PAM Execution Constraints

```
┌─────────────────────────────────────────────────────────────┐
│ Privileged Host Process (e.g. login, sudo, gdm-password)    │
│                                                             │
│  ┌─────────────────────────┐   ┌──────────────────────────┐ │
│  │ pam_unix.so (fallback)  │   │ pam_soos.so (C ABI)      │ │
│  └─────────────────────────┘   └─────────────┬────────────┘ │
└──────────────────────────────────────────────┼──────────────┘
                                               │ Blocking IPC
                                               │ (Unix Domain Socket)
                                               │ Mode: 0660
                                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Privileged Background Daemon: soos-daemon (root:soos)       │
│ - Exclusive owner of /dev/video* via v4l crate              │
│ - Isolated ONNX Runtime CPU inference engine                │
│ - Peer validation via kernel SO_PEERCRED                    │
└─────────────────────────────────────────────────────────────┘
```

### Critical Invariants
1. **Never Panic Across FFI**: An unwinding panic across an `extern "C"` boundary without `catch_unwind` triggers an abort or undefined behavior. Every PAM FFI entry point must catch all panics and systematically return `PAM_IGNORE`.
2. **Never Return PAM_SUCCESS on Failure**: Any failure (timeout, network glitch, missing socket, protocol error, corrupted buffer) must silently degrade to `PAM_IGNORE` so Linux-PAM can proceed to traditional password authentication (`pam_unix.so`).
3. **No Terminal/Stream Pollution**: PAM modules share `stdout` and `stderr` descriptors with the host process. Emitting debugging prints (`println!`, `eprintln!`, `dbg!`) can crash graphical display managers (GDM, SDDM) or corrupt scripts calling `sudo`.
4. **Strict Concurrency & Latency Deadline**: The PAM module must **never** start an asynchronous runtime (Tokio) and every blocking PAM operation has an explicit deadline derived from the clamped `timeout_ms` (default 1000 ms, range 10–5000 ms); nothing in the PAM path is ever unbounded.

---

## 2. Compiler Profile Hardening

Configured centrally in root `Cargo.toml`:

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "unwind"
overflow-checks = true
strip = "symbols"

[profile.dev]
overflow-checks = true
```

### Rationale

| Flag | Value | Security Justification |
|---|---|---|
| `overflow-checks` | `true` (Release & Dev) | In default Rust release builds, integer overflows wrap modulo $2^N$ (`overflow-checks = false`). In authentication logic where buffer lengths and counts are validated, wrapping can create critical logic bypasses or out-of-bounds access. Enforcing `overflow-checks = true` turns an overflow into a panic, which `catch_unwind` in `pam_soos.so` converts into `PAM_IGNORE` — this only holds because the profile unwinds (see `panic` below). |
| `lto` | `true` | Cross-crate Link-Time Optimization optimizes call graphs across crate boundaries, eliminating unused symbols and hardening indirect calls. |
| `codegen-units` | `1` | Maximizes compiler optimization visibility and eliminates codegen discrepancies across compilation units. |
| `panic` | `"unwind"` | **Mandatory for panic safety of `pam_soos.so`** (ADR 2026-09-29, review finding PAM-01 / GitHub #148). Under `panic = "abort"` every `catch_unwind` in the module is a no-op: a panic (including an overflow trapped by `overflow-checks`) aborts the PAM host process — gdm, sudo, login, the screen locker — instead of degrading to `PAM_IGNORE` and password fallback (ARCHITECTURE.md invariant 5). The panic strategy cannot be set per package, and every packaging path builds with `[profile.release]`, so the whole workspace unwinds. Rust already aborts when a panic escapes an `extern "C"` function, so the defense-in-depth that `abort` used to provide at the FFI boundary is retained for free. `tests/invariants::test_release_profile_unwinds_so_pam_catch_unwind_is_effective` pins this value and Docker case T10 proves it on the release-built artifact. |
| `strip` | `"symbols"` | Strips internal debug symbols from the compiled shared object to minimize exposed metadata. |

### Proving panic safety on the shipped artifact

Unit and integration tests run under `[profile.test]`, which always unwinds, so they cannot
detect a wrong release panic strategy. The `soos-pam` crate therefore has an opt-in Cargo
feature `fault-injection` (never a default feature, never enabled by `scripts/build_*.sh`,
`packaging/**`, `scripts/install.sh` or CI builds — enforced by
`tests/invariants::test_pam_fault_injection_feature_is_opt_in_and_never_packaged`). It adds the
PAM argument `fault_inject=<panic|overflow>` that panics inside the `catch_unwind` region
before any socket activity. `tests/docker/test_suite.sh` case T10 builds this variant with the
same `[profile.release]` into `target/fault-injection/`, loads it as `pam_soos_fault.so` and
asserts that the PAM host process is not killed (exit 134 = SIGABRT) and that password fallback
still works (valid password accepted, invalid password rejected).

---

## 3. Workspace-Wide Lints & Restriction Rules

Declared centrally in root `Cargo.toml` and inherited by all crates via `[lints] workspace = true`:

### Rust Compiler Lints (`[workspace.lints.rust]`)
- `unsafe_op_in_unsafe_fn = "deny"`: Requires explicit `unsafe {}` blocks even inside `unsafe fn`, making every unsafe operation clearly visible and auditable.
- `unused_must_use = "deny"`: Forbids ignoring `Result` or `Option` return values.
- `rust_2018_idioms = "deny"`: Enforces modern Rust conventions (explicit lifetimes, dyn trait objects).

### Clippy Security & Restriction Lints (`[workspace.lints.clippy]`)

| Lint | Severity | Security Rationale |
|---|---|---|
| `unwrap_used` | `deny` | Prevents unhandled errors causing panics in production code. |
| `expect_used` | `deny` | Same as `unwrap_used`. Explicit error bubbling (`?`) is required. |
| `panic` | `deny` | Forbids explicit `panic!()` in library and PAM production code. |
| `panic_in_result_fn` | `deny` | Functions returning `Result` must return `Err`, never panic. |
| `unimplemented` / `todo` / `unreachable` | `deny` | Prevents unfinished stubs from being accidentally deployed to production. |
| `undocumented_unsafe_blocks` | `deny` | Every `unsafe` block must be documented with a `// SAFETY:` rationale explaining why invariants hold. |
| `mem_forget` | `deny` | Prevents memory leaks in long-running daemon and PAM processes. |
| `dbg_macro` | `deny` | Prevents committing `dbg!()` statements which could leak credentials or memory contents. |
| `print_stdout` / `print_stderr` | `deny` | Forbids polluting host display managers or terminal output in PAM/library code. |
| `allow_attributes_without_reason` | `deny` | Enforces accountability: any `#[allow(...)]` must specify `reason = "..."`. |
| `fallible_impl_from` | `deny` | `From` trait implementations must never fail; use `TryFrom` instead. |
| `indexing_slicing` | `warn` | Direct slice indexing (`s[i]`) can panic out of bounds; favor `.get()` or bounded iterators. |
| `arithmetic_side_effects` | `warn` | Flags potential integer overflows or division by zero; favor `checked_*` methods. |
| `cast_possible_truncation` / `cast_possible_wrap` / `cast_sign_loss` | `warn` | Catches dangerous `as` casts that could silently truncate or alter sign. |

---

## 4. Supply Chain & Dependency Audit (`deny.toml`)

Automated via `cargo-deny`:
- **Banned Crates**:
  - `opencv`: Strictly prohibited (threat of native C++ vulnerabilities, memory footprint).
  - `nokhwa`: Strictly prohibited (root daemon directly owns `/dev/video*` via `v4l` crate).
- **Advisories**: `yanked = "deny"`, zero unreviewed security advisories.
- **Licenses**: Only permissive open source licenses (`MIT`, `Apache-2.0`, `BSD-3-Clause`, `ISC`) for third-party dependencies. The project itself is `AGPL-3.0-or-later` (root `LICENSE`, `[workspace.package]` of `Cargo.toml`); workspace crates are private and exempt from the dependency allow-list (`[licenses.private]`).
- **Sources**: Only official `crates.io` registry is permitted; arbitrary git dependencies are blocked.
- **Audited daemon-only D-Bus client**: `zbus` 5 (`default-features = false`, feature `tokio` only, MIT) is declared once in `[workspace.dependencies]` and used only by `soos-daemon` for the presence auto-unlock (systemd-logind `LockedHint`, `LidClosed`, `UnlockSession`; GitHub #323). It adds 22 crates (MIT or MIT/Apache-2.0, no new duplicate version, no `deny.toml` skip), links no C `libdbus`, and is never a dependency of `pam_soos.so` (invariant `presence_unlock_contract`). The daemon connects only to the pinned address `unix:path=/run/dbus/system_bus_socket`, never to an environment-derived bus, without object server, well-known name or signal match.

---

## 5. Automated Verification Gates

Quality and security gates are enforced at multiple levels (details in
[`CI_CD_AND_SECURITY.md`](CI_CD_AND_SECURITY.md)):

```
┌─────────────────────────────────────────────────────────────┐
│ 1. Git Hooks (.githooks/)                                   │
│    - pre-commit: anti-commit to main, secret scanner,       │
│      scripts/candid_review.sh (deterministic invariants)    │
│    - commit-msg: Conventional Commits 1.0.0                 │
│    - pre-push: anti-push to main, secret scanner,           │
│      fingerprint-bound candid review report                 │
├─────────────────────────────────────────────────────────────┤
│ 2. Local Quality Script (./save.sh) — same flags as CI      │
│    - cargo fmt                                              │
│    - cargo clippy --locked --workspace --all-targets        │
│      --all-features -- -D warnings                          │
│    - cargo test --locked --workspace --all-targets          │
│      --all-features (incl. architectural tests)             │
│    - cargo deny --locked check (supply chain audit)         │
│    - scripts/candid_subagent.sh (dual-layer candid review)  │
├─────────────────────────────────────────────────────────────┤
│ 3. Continuous Integration (.github/workflows/ci.yml)        │
│    - lint (fmt, ShellCheck, candid layers 1+2, per-commit   │
│      secrets), clippy, test, security (cargo-deny, daily),  │
│      pam-integration + authselect-profile + pam-rollback    │
│      (Docker),                                              │
│      ci-success aggregate;                                  │
│      pr-title.yml (Conventional Commits on the PR title)    │
│    - read-only token, SHA-pinned actions, --locked builds   │
└─────────────────────────────────────────────────────────────┘
```
