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

To eliminate this threat surface, `soos` implements a coordinated, multi-layer memory hygiene and swap protection model:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                          soos-daemon / soos-pipeline                        │
│                                                                             │
│  Layer 1: Process-Wide Memory Pinning (mlockall)                            │
│  - Prevents all process memory pages from being paged to swap               │
│  - Flags: MCL_CURRENT | MCL_FUTURE                                          │
│  - Requires CAP_IPC_LOCK (granted by systemd unit User=root)                 │
│                                                                             │
│  Layer 2: Granular Buffer Locking (mlock / LockedBuffer)                    │
│  - Granular page locking for sensitive buffers                              │
│  - Best-effort fallback when unprivileged                                   │
│                                                                             │
│  Layer 3: Immediate Memory Zeroization (Zeroize / ZeroizeOnDrop)            │
│  - Floating-point vectors, byte buffers, and keys wiped with zeros on drop  │
│  - Explicit frame and template scrubbing immediately post-verification      │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Implementation Details

### Process-Wide Memory Locking (`mlockall`)
During daemon startup (`soos-daemon`), the process attempts to lock its address space:
```rust
soos_daemon::mlock::mlock_process_address_space();
```
- Calls kernel system call `libc::mlockall(MCL_CURRENT | MCL_FUTURE)`.
- Enforces that both the initial process text/heap and all future dynamic allocations (such as ONNX Runtime inference buffers and image frames) remain permanently resident in physical RAM.
- If executed in an unprivileged test environment or Docker container lacking `CAP_IPC_LOCK`, the call returns `false` without panicking, ensuring graceful degradation.

### Granular Buffer Locking (`LockedBuffer<T>`)
For targeted protection of key material and decrypted embeddings:
```rust
use soos_daemon::mlock::LockedBuffer;

let locked_key = LockedBuffer::new(master_key_bytes);
// Underlying memory pages locked via mlock(2)
// On drop: munlock(2) is called and memory is zeroized via Zeroize
```

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
- `tests/hardening_tests.rs`: tests `mlock_slice`, `munlock_slice`, `mlock_process_address_space`, and `LockedBuffer` lifecycle.
- `crates/inference-ort/tests/zeroize_tests.rs`: validates zeroization of `BiometricEmbedding`.
- `crates/camera-v4l/tests/frame_zeroize_tests.rs`: validates zeroization of `Frame`.
- `tests/invariants/src/lib.rs`: enforces absence of unsafe code in business crates and absence of stream pollution.
