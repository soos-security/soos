# Memory Protection & Swap Hardening Strategy

This document outlines the memory protection and swap defense-in-depth architecture implemented for the `soos` biometric PAM suite.

---

## 1. Threat Model & Swap Exposure

In modern Linux operating systems, the virtual memory subsystem dynamically migrates memory pages from physical RAM to swap space (unencrypted swap partition, swap file, or compressed zram) under system memory pressure.

In a biometric authentication system:
- **AES-256 Master Encryption Keys** (`MasterKey`)
- **Decrypted Biometric Templates & Embeddings** (`BiometricTemplate`, `BiometricEmbedding`)
- **Raw Camera Frames** (`Frame`)
- **Intermediate Face Crops** (`PipelineOutput`)

If any of these sensitive data buffers are paged to unencrypted swap space:
1. Forensic disk recovery could extract user biometric templates or master cryptographic keys long after process shutdown or system reboot.
2. Cold boot and swap file inspection attacks bypass file permission boundaries (`0600`).
3. Core dumps or memory hibernation images could persist secrets indefinitely.

---

## 2. Multi-Layer Defense Architecture

To reduce this threat surface, `soos` combines process-wide page locking with zeroization. Only the two layers below are wired in production (GitHub #201, review finding DMN-12; ADR 2026-09-30 "Swap Protection Is `mlockall` Only"):

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          soos-daemon / soos-pipeline                        │
│                                                                             │
│  Layer 1: Process-Wide Memory Pinning (mlockall)                            │
│  - Prevents all process memory pages from being paged to swap               │
│  - Flags: MCL_CURRENT | MCL_FUTURE                                          │
│  - Requires CAP_IPC_LOCK (granted by systemd unit User=root)                 │
│                                                                             │
│  Layer 2: Immediate Memory Zeroization (Zeroize / ZeroizeOnDrop)            │
│  - Floating-point vectors, byte buffers, and keys wiped with zeros on drop  │
│  - Explicit frame and template scrubbing immediately post-verification      │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Implementation Details

### Process-Wide Memory Locking (`mlockall`)
During daemon startup (`soos-daemon`), the process attempts to lock its address space:
```rust
soos_daemon::mlock::enable_swap_protection(&health);
```
- Calls kernel system call `libc::mlockall(MCL_CURRENT | MCL_FUTURE)` through `mlock_process_address_space`.
- Enforces that both the initial process text/heap and all future dynamic allocations (such as ONNX Runtime inference buffers and image frames) remain permanently resident in physical RAM.
- Success is logged at `info` level and recorded as `HealthState::memory_locked() == true`.
- A refusal (no `CAP_IPC_LOCK`, an insufficient `RLIMIT_MEMLOCK`, a container, or a future `CapabilityBoundingSet` without `CAP_IPC_LOCK`) does not stop the daemon, but it is logged at `warn` level ("Swap protection disabled: mlockall refused ...") and recorded as `HealthState::memory_locked() == false`. Keys, templates and frames are then swappable: no other page-locking layer compensates. Operators who need the guarantee must keep `CAP_IPC_LOCK` (the shipped unit runs as root) or use encrypted swap / no swap.
- `memory_locked` is informational and does not change `is_healthy`. It is carried by `StatusResponse::memory_locked` (GitHub #201, walkthrough 140) and shown by `soos-admin status` as `Swap Protection: LOCKED`, `NOT LOCKED (memory may be swapped out)` or `N/A` when the daemon cannot be contacted (`DaemonStatusReport::memory_locked: Option<bool>`, `null` in the JSON output; GitHub #287, walkthrough 147).

### `LockedBuffer<T>` and `mlock_slice` (available, not wired)
`crates/daemon/src/mlock.rs` also provides `mlock_slice` / `munlock_slice` and the RAII wrapper `LockedBuffer<T>` (locks the pages of one buffer with `mlock(2)`, unlocks and zeroizes on drop). They are tested primitives but are **not wired** around any production buffer: the master keys live in `soos-biometric-store` / `soos-evidence-store`, the decrypted templates in `soos-biometric-store` and the live embeddings in `soos-inference-ort`, all `#![forbid(unsafe_code)]` crates that cannot call `mlock` and do not depend on the daemon. Wiring them would require an allocator-level design and a new ADR; until then, do not describe `LockedBuffer` as an active protection.

### Automatic Zeroization on Deallocation
All sensitive data structures implement `zeroize::Zeroize` and wipe their internal byte or float representations on drop:
- `BiometricEmbedding`: wraps `Zeroizing<Vec<f32>>`, scrubbing the 512D ArcFace float arrays.
- `Frame`: implements `Zeroize` and `Drop`, scrubbing raw camera capture pixels.
- `PipelineOutput`: implements `Zeroize` and `Drop`, scrubbing intermediate 112x112 RGB crops.
- `MasterKey`: implements `Zeroize` and `ZeroizeOnDrop`, scrubbing 256-bit encryption keys.
- `Response`: implements `Zeroize` and `Drop`, resetting verdict to `Deny` and zeroing nonces.

### Immediate Post-Verification Scrubbing
In `crates/daemon/src/dispatcher.rs`, camera frames and enrolled templates are not held until the end of the connection:
```rust
// Security hardening: immediately scrub and discard raw camera capture and enrolled template
drop(frame);
drop(enrolled_template);
```
They are explicitly dropped immediately after cosine similarity computation, ensuring that camera pixels and enrolled vectors are cleared from memory before response serialization or socket transmission.

---

## 4. Verification & Testing

Memory protection and zeroization are validated by automated unit and integration tests:
- `crates/daemon/tests/hardening_tests.rs`: tests `mlock_slice`, `munlock_slice`, `mlock_process_address_space`, and `LockedBuffer` lifecycle.
- `crates/daemon/tests/swap_protection_tests.rs`: the `mlockall` outcome is recorded in `HealthState::memory_locked` and a refusal is logged at `warn` level (GitHub #201).
- `tests/invariants/src/daemon_docs_contract.rs`: this document may only present `LockedBuffer` as an active layer once production code uses it.
- `crates/inference-ort/tests/zeroize_tests.rs`: validates zeroization of `BiometricEmbedding`.
- `crates/camera-v4l/tests/frame_zeroize_tests.rs`: validates zeroization of `Frame`.
- `tests/invariants/src/lib.rs`: enforces absence of unsafe code in business crates and absence of stream pollution.
