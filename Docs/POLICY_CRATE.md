# Policy Crate Specification (`soos-policy`)

## 1. Overview & Architecture

The `soos-policy` crate provides pure authorization logic and per-UID rate limiting for the `soos` biometric PAM ecosystem.
It strictly operates as a **Zero I/O** component:
- Zero filesystem access
- Zero network sockets
- Zero asynchronous runtimes
- Zero clock syscalls (accepts monotonic timestamps as arguments)

```
                       ┌──────────────────────────────┐
                       │         soos-daemon          │
                       └──────────────┬───────────────┘
                                      │ In-Memory Call
                                      ▼
                       ┌──────────────────────────────┐
                       │         soos-policy          │
                       │                              │
                       │  ┌────────────────────────┐  │
                       │  │   RateLimiter (Sliding)│  │
                       │  └───────────┬────────────┘  │
                       │              ▼               │
                       │  ┌────────────────────────┐  │
                       │  │   AuthorizationEngine  │  │
                       │  │   - ThresholdConfig    │  │
                       │  │   - AuthContext        │  │
                       │  └───────────┬────────────┘  │
                       └──────────────┼───────────────┘
                                      ▼
                      (Verdict, ReasonClass)
```

---

## 2. Request State Matrix Alignment

Adheres strictly to `AI/ARCHITECTURE.md` §3 Request State Matrix:

| Condition | `Verdict` | `ReasonClass` | System Effect |
|---|---|---|---|
| `score >= threshold && pad_passed && face_count == 1 && session_valid` | `Verdict::Allow` | `ReasonClass::FaceMatch` | `PAM_SUCCESS` |
| `face_count == 0` | `Verdict::Deny` | `ReasonClass::NoFace` | `PAM_IGNORE` |
| `face_count > 1` | `Verdict::Deny` | `ReasonClass::MultipleFaces` | `PAM_IGNORE` |
| `!pad_passed` | `Verdict::Deny` | `ReasonClass::PadFailed` | `PAM_IGNORE` |
| `score < threshold \|\| score.is_nan()` | `Verdict::Deny` | `ReasonClass::ScoreBelowThreshold` | `PAM_IGNORE` |
| `!session_valid` | `Verdict::ProtocolError` | `ReasonClass::UidMismatch` | `PAM_IGNORE` |
| Rate limit reached | `Verdict::ProtocolError` | `ReasonClass::RateLimited` | `PAM_IGNORE` |

---

## 3. Threshold Configuration (`ThresholdConfig`)

Biometric and Presentation Attack Detection (PAD) thresholds are configured via `ThresholdConfigBuilder`:

```rust
use soos_policy::ThresholdConfig;

let config = ThresholdConfig::builder()
    .match_threshold(0.70) // Sourced from MobileFaceNet literature (FAR <= 0.1%)
    .pad_threshold(0.85)   // Sourced from NIST SP 800-63B guidelines
    .build()?;
```

### Invariant Checks
- Range: `[0.0, 1.0]`
- Rejects `NaN` and `Infinite` values with `PolicyError::InvalidThreshold`

---

## 4. Deterministic Per-UID Rate Limiting (`RateLimiter`)

Implements a sliding-window rate limiter tracking requests per target UID.
Memory growth is bounded through `prune_stale()`:

```rust
use soos_policy::{RateLimiter, RateLimitConfig};

let config = RateLimitConfig::new(5, 60_000_000_000); // 5 attempts per 60s
let mut limiter = RateLimiter::new(config);

let now_monotonic_ns = 1_000_000_000;
limiter.check_and_record(1000, now_monotonic_ns)?;
```

---

## 5. Security & Quality Invariants

- `#![forbid(unsafe_code)]` at crate root.
- Zero `unwrap()` or `expect()` in production pathways.
- Bounded memory allocations with automatic eviction of expired attempts.
