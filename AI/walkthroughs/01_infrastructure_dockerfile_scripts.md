# Walkthrough — Development Infrastructure (Dockerfile, run_tests.sh, save.sh)

> Date: 2026-09-12  
> Phase: Foundation — Local Development Environment Setup  

---

## Deliverables

| File | Purpose | Lines |
|---|---|---:|
| [`Dockerfile`](file:///home/hadrien/soos/Dockerfile) | Ubuntu 24.04 image with Rust, PAM, pamtester, and mock user | ~90 |
| [`run_tests.sh`](file:///home/hadrien/soos/run_tests.sh) | Ephemeral Docker container orchestration, release compilation, 3 pamtester assertions | ~160 |
| [`save.sh`](file:///home/hadrien/soos/save.sh) | Pipeline `cargo fmt` → `clippy -D warnings` → `test` → automated git commit | ~200 |

---

## File Responsibilities

### Dockerfile
- Based on **Ubuntu 24.04** with `build-essential`, `libpam0g-dev`, `libclang-dev`, `pamtester`.
- Installs stable Rust via `rustup` with clippy and rustfmt components.
- Creates test user `testuser` with password `password123`.
- Configures test PAM service `/etc/pam.d/test-soos` with stack:
  ```pam
  auth [success=done default=ignore] pam_soos.so timeout_ms=250
  auth required                      pam_unix.so
  ```
- Source code is mounted at runtime via bind mount, never baked into the image.

### run_tests.sh
Executes 3 tests in the ephemeral container:

| Test | Description | Invariant Validated |
|---|---|---|
| **T1** | Loads `.so` + correct password -> success via `pam_unix` | Module returns `PAM_IGNORE`, C ABI intact |
| **T2** | Incorrect password -> rejected | Module does not mask normal authentication failures |
| **T3** | Absent `.so` module -> system remains functional | PAM stack resilience and fault tolerance |

Container is destroyed automatically (`--rm`) following execution.

### save.sh
Sequential fail-fast pipeline:
1. `cargo fmt` — formats code in place.
2. `cargo clippy -- -D warnings` — zero warning policy.
3. `cargo test` — full workspace test suite.
4. `git add .` — stages changes.
5. Generates conventional commit message based on modified files.
6. `git commit` — local commit only.

---

## Next Steps
Construct Cargo workspace with `protocol` and `pam` crates to achieve an end-to-end verifiable PAM sandbox.
