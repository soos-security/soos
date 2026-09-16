# Walkthrough 31 — Production Hardening & Zeroization Audit

## Overview
- **Issue**: Issue #16 (GitHub #23): Production Hardening — zeroization audit, swap protection, cargo-deny
- **Branch**: `chore/production-hardening`
- **Scope**: Memory zeroization audit across pipelines, swap protection (`mlock`), systemd hardening validation, and `cargo-deny` duplicate ban enforcement

---

## 1. Key Accomplishments

### 1.1 Memory Zeroization Audit (`#16.1`)
- **`soos-inference-ort`**: `BiometricEmbedding` wraps `Zeroizing<Vec<f32>>` and implements `zeroize::Zeroize`. When an embedding vector is dropped, all 128D/512D float values are scrubbed with zeros.
- **`soos-camera-v4l`**: `Frame` implements `zeroize::Zeroize` and `Drop`. Camera pixel buffers are scrubbed upon deallocation.
- **`soos-vision`**: `PipelineOutput` implements `zeroize::Zeroize` and `Drop`, scrubbing intermediate 112x112 aligned face crops upon deallocation.
- **`soos-daemon`**: `ConnectionDispatcher::handle_request` explicitly drops `frame` and `enrolled_template` immediately following cosine similarity computation, avoiding residual capture memory during response generation and socket I/O.
- **Cryptographic Keys**: `MasterKey` in both `soos-biometric-store` and `soos-evidence-store` automatically zeroizes on drop during daemon termination.

### 1.2 Swap Protection (`#16.2`)
- **`soos-daemon::mlock` Subsystem**:
  - `mlock_process_address_space()`: invokes `libc::mlockall(MCL_CURRENT | MCL_FUTURE)` on daemon startup, locking text, heap, and future dynamic allocations into physical RAM when running with `CAP_IPC_LOCK`.
  - `mlock_slice(&[u8])` / `munlock_slice(&[u8])`: granular buffer memory locking.
  - `LockedBuffer<T>`: RAII guard that pins sensitive memory into physical RAM and executes `munlock` followed by `zeroize()` upon deallocation.
  - Graceful degradation: handles unprivileged execution environments (such as CI containers or user testing) without panicking.
- **Documentation**: Authored `Docs/MEMORY_PROTECTION_AND_SWAP.md` detailing the Linux `mlock` page-locking strategy and capability requirements.

### 1.3 Systemd Hardening Validation (`#16.3`)
- Tested all sandboxing directives in `packaging/soos-daemon.service`:
  - `MemoryDenyWriteExecute=yes`
  - `RestrictSUIDSGID=yes`
  - `SystemCallArchitectures=native`
  - `NoNewPrivileges=yes`
  - `PrivateTmp=yes`
  - `ProtectHome=yes`
  - `ProtectSystem=strict`
  - `DevicePolicy=closed`
  - `DeviceAllow=/dev/video* rw`
  - `RestrictAddressFamilies=AF_UNIX`
  - `LockPersonality=yes`
  - `UMask=0077`, `User=root`, `Group=root`, `RuntimeDirectory=soos`, `ReadWritePaths=/var/lib/soos /run/soos`
- Automated test asserts that forbidden directives (e.g. `AF_INET`, `AF_NETLINK`) are absent.

### 1.4 `cargo-deny` Audit Enforcement (`#16.4`)
- Elevated `multiple-versions = "deny"` in `deny.toml`, strictly blocking duplicate dependencies.
- Added explicit, documented skip exemptions for unavoidable transitive build/dev dependencies (`bindgen 0.65`, `tempfile 3.27`).
- Verified that `cargo deny check` passes with zero advisories, zero bans, zero license issues, and zero source violations.

---

## 2. Test Verification Evidence

```bash
cargo test --all-targets --all-features
cargo deny check
./scripts/candid_review.sh
```

All suites passed with zero failures:
- `zeroize_tests::test_biometric_embedding_zeroize_trait`: PASS
- `frame_zeroize_tests::test_frame_zeroize_trait`: PASS
- `hardening_tests::test_mlock_slice_and_munlock_slice_lifecycle`: PASS
- `hardening_tests::test_locked_buffer_raii_wrapper`: PASS
- `hardening_tests::test_mlock_process_address_space_call`: PASS
- `hardening_tests::test_cargo_deny_bans_duplicate_versions`: PASS
- `hardening_tests::test_systemd_hardening_directives_complete`: PASS
- All 12 `soos-invariants` architectural checks: PASS
- `cargo-deny`: 100% compliant (`advisories ok, bans ok, licenses ok, sources ok`)
- Pre-push candid review: APPROVED

---

## 3. Artifacts & Documentation
- `Docs/MEMORY_PROTECTION_AND_SWAP.md`: Technical documentation of swap protection architecture.
- `AI/DECISIONS.md`: Added ADR on Production Hardening & Memory Hygiene.
- `AI/VERIFICATION_MATRIX.md`: Appended Component `production-hardening` criteria H1–H4 (all Verified).
- `AI/candid_review_report.md`: Independent audit report with `VERDICT: APPROVED`.
