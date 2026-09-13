# Architectural Decision Records (ADR) Register

This file records immutable technical decisions to ensure consistency and prevent architectural drift or hallucination across development sessions. All decisions derive strictly from `ARCHITECTURE.md`.

## Active Decisions

* **[2026-09-12] IPC (Inter-Process Communication):** Exclusively use local Unix Domain Sockets (`SOCK_SEQPACKET` or framed `SOCK_STREAM`) with strict `SO_PEERCRED` kernel validation. Zero network sockets.
* **[2026-09-12] PAM Module (`pam_soos.so`):** Must **never** start an asynchronous runtime (Tokio). Exclusively use blocking standard library primitives (`std::os::unix::net::UnixStream`) with a strict 200–250ms timeout.
* **[2026-09-12] Panic Safety:** The PAM module catches all panics via `catch_unwind`. In any error or panic condition, it systematically returns `PAM_IGNORE` to silently fall back to password authentication.
* **[2026-09-12] Camera & Hardware Access:** The root daemon is the exclusive owner of the `/dev/video*` capture device. Use the `v4l` crate (not `nokhwa`) to utilize kernel MMAP buffers in production.
* **[2026-09-12] Artificial Intelligence Engine:** Use `ort` (ONNX Runtime) in CPU execution mode. Importing or linking OpenCV is strictly forbidden.
* **[2026-09-13] Project Naming:** The project is named `soos`. The PAM shared object is `pam_soos.so`, the privileged background process is `soos-daemon`. Legacy references to "ZTLH" or "pam_ztlh" are deprecated.
* **[2026-09-13] Codec & Framing:** Use `postcard` + `serde` for IPC payload serialization. JSON is forbidden. Maximum payload size is capped at 4,096 bytes with big-endian length prefix.
* **[2026-09-13] Commit Standard & Language:** Conventional Commits 1.0.0 is strictly enforced across the repository via `.githooks/commit-msg`. All codebase deliverables (code, docs, PRs, commits, walkthroughs) are authored in 100% English.