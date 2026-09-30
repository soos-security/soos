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
| `score < threshold \|\| !score.is_finite()` | `Verdict::Deny` | `ReasonClass::ScoreBelowThreshold` | `PAM_IGNORE` |
| `!session_valid` | `Verdict::ProtocolError` | `ReasonClass::UidMismatch` | `PAM_IGNORE` |
| Rate limit reached | `Verdict::ProtocolError` | `ReasonClass::RateLimited` | `PAM_IGNORE` |

Non-finite floating-point scores (`f32::INFINITY`, `f32::NEG_INFINITY`, `f32::NAN`) are strictly classified as non-authorizing denials.

The per-capture `Allow` row above is a necessary, not sufficient, condition: the daemon only
authorizes once the multi-frame consensus of section 5 is reached.

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

### Security Floor for Operator Configuration (GitHub #170, PAD-04)
`build()` only checks the mathematical domain, so `pad_threshold = 0.0` is in range yet accepts
every frame as live. Operator-supplied thresholds (`[pipeline.thresholds]` in
`/etc/soos/daemon.toml`) are therefore built with `ThresholdConfigBuilder::build_with_security_floor()`,
which additionally refuses:

| Setting | Floor constant | Value | Rationale |
|---|---|---|---|
| `match_threshold` | `ThresholdConfig::MIN_MATCH_THRESHOLD` | `0.40` | Lower cosine thresholds accept unrelated faces |
| `pad_threshold` | `ThresholdConfig::MIN_PAD_THRESHOLD` | `0.50` | `0.0` / negative values disable anti-spoofing |

`soos-daemon` maps the error to `DaemonError::Config` and refuses to start (fail closed; there is
no override flag). `ThresholdConfig::new_raw` performs no validation and is never used outside
`soos-policy` (invariant `pad_contract::test_thresholds_never_built_unvalidated_outside_policy`).

---

## 4. Deterministic Per-UID Rate Limiting (`RateLimiter`)

Implements a sliding-window rate limiter tracking requests per target UID.
Memory growth is strictly bounded to prevent Denial of Service (DoS) through spoofed UIDs:
- **Configurable Capacity**: `RateLimitConfig::max_tracked_uids` (default: 1,024 UIDs).
- **Dual Eviction Strategy**:
  1. On capacity saturation, expired entries across all UIDs are pruned first (`prune_stale`).
  2. If capacity remains saturated, the least recently used (LRU) UID—determined by oldest active attempt timestamp—is evicted.
- **Fail-Closed Guarantees**: A capacity of zero immediately rejects attempts with `PolicyError::RateLimitExceeded`.

```rust
use soos_policy::{RateLimiter, RateLimitConfig};

let config = RateLimitConfig::new(5, 60_000_000_000) // 5 attempts per 60s
    .with_max_tracked_uids(1024);
let mut limiter = RateLimiter::new(config);

let now_monotonic_ns = 1_000_000_000;
limiter.check_and_record(1000, now_monotonic_ns)?;
```

---

## 5. Multi-Frame PAD Consensus (`PadAggregator`)

The daemon evaluates several camera captures per authentication request. Authorizing on the first
capture that passed PAD and matching gave an attacker one independent liveness trial per capture
(GitHub #147 / review finding PAD-02). `pad_consensus.rs` replaces that rule with a zero-I/O,
clockless aggregator created once per request:

| Item | Location | Value |
|---|---|---|
| `DEFAULT_PAD_CONSENSUS_REQUIRED` (`k`) | `crates/policy/src/pad_consensus.rs` | 3 consecutive passing captures |
| `DEFAULT_PAD_CONSENSUS_WINDOW` (`n`) | `crates/policy/src/pad_consensus.rs` | 5 retained classifications |
| `MAX_PAD_CONSENSUS_WINDOW` | `crates/policy/src/pad_consensus.rs` | 32 (memory bound) |
| `FRAME_POLL_INTERVAL_MS` | `crates/daemon/src/pipeline.rs` | 10 ms between capture checks |

`PadConsensusConfig::new(window, required)` rejects `required == 0`, `required > window` and
`window > MAX_PAD_CONSENSUS_WINDOW` with `PolicyError::InvalidConsensus`.

### Frame classification (`FrameEvaluation` → `FrameClass`)

| Condition (evaluated in order) | `FrameClass` | Effect on the run |
|---|---|---|
| `face_count == 0` | `NoFace` | resets the consecutive run |
| `face_count > 1` | `MultipleFaces` | resets the consecutive run |
| `!pad_live \|\| !pad_score.is_finite() \|\| pad_score < pad_threshold` | `Spoof` | **sticky veto** for the request |
| `!match_score.is_finite() \|\| match_score < match_threshold` | `NoMatch` | resets the consecutive run |
| otherwise | `Passing` | extends the consecutive run |

### Aggregate decision (`ConsensusDecision::verdict()`)

| Condition | `Verdict` | `ReasonClass` |
|---|---|---|
| Any `Spoof` capture recorded in the request | `Deny` | `PadFailed` |
| Trailing run of `Passing` captures `>= k` (and no spoof) | `Allow` | `FaceMatch` |
| Last capture `NoFace` / `MultipleFaces` / `NoMatch` | `Deny` | `NoFace` / `MultipleFaces` / `ScoreBelowThreshold` |
| No capture recorded, or run shorter than `k` when the budget expires | `Unavailable` | `Timeout` |

There is deliberately no reset method: a spoof veto cannot be cleared within a request. The
aggregator only ever stores the last `n` classifications and saturating counters.

```rust
use soos_policy::{ConsensusDecision, FrameEvaluation, PadAggregator, ThresholdConfig};

let mut aggregator = PadAggregator::with_defaults(ThresholdConfig::default());
for _ in 0..3 {
    aggregator.record(&FrameEvaluation::live(0.97, 0.90));
}
assert_eq!(aggregator.decision(), ConsensusDecision::Allow);
aggregator.record(&FrameEvaluation::spoof(0.05));
assert_eq!(aggregator.decision(), ConsensusDecision::SpoofVetoed);
```

### Daemon integration

`crates/daemon/src/dispatcher.rs` (step 8e) records one `FrameEvaluation` per **distinct, fresh**
camera capture (new sequence number, age `<= MAX_FRAME_AGE_NS`), stops as soon as the decision is
`Allow` or `SpoofVetoed`, and otherwise polls every `FRAME_POLL_INTERVAL_MS` until the decision
budget expires (the poll never sleeps past the budget). The budget is a single
`soos_daemon::inference::RequestDeadline` computed once per request: the smaller of the client
deadline and the request start plus `connection_timeout`, each minus `RESPONSE_WRITE_MARGIN_MS`
(50 ms) reserved for writing the response and the PAM read; `DECISION_BUDGET_MS` applies only when
the client sends no deadline. Each capture is evaluated on the Tokio blocking pool behind the
`InferenceGate` semaphore (`MAX_CONCURRENT_INFERENCES` = 1); an inference starts only if the
measured latency estimate (bounded EMA, initially `DEFAULT_INFERENCE_ESTIMATE_MS` = 80 ms) fits the
remaining budget and the slot is obtained before the last feasible start. Otherwise the loop
finalizes with the current consensus (`Unavailable`/`Timeout` when no capture was evaluated); a
panicking inference job fails closed with `Unavailable`/`InternalError` (GitHub #158, #159).
Exactly one rate-limit attempt is recorded per request, atomically and before the loop
(`AuthorizationEngine::record_attempt` under the policy write lock, before the camera wake and any
vision work; GitHub #200 / DMN-10): a rejected reservation answers `ProtocolError`/`RateLimited`
at once, concurrent requests cannot overshoot `max_attempts`, and nothing is recorded after the
loop.

At daemon start (before the socket is bound) `soos_daemon::pipeline::warmed_inference_gate`
runs every vision stage `WARMUP_PASSES` (2) times on blank synthetic inputs and seeds the
estimate with the last pass (`InferenceEstimator::seed`), so the first `Auth` request is
admitted against a measured latency instead of the 80 ms default (GitHub #276). The cold first
pass is discarded; a failed warm-up keeps the default and never blocks start-up.

Latency: with a 30 fps camera the third distinct capture is available about 67 ms after the first,
so consensus adds roughly two capture intervals plus two inference passes to the previous
single-capture latency, well within `DECISION_BUDGET_MS` (900 ms) and the 2500 ms GDM budget.

---

## 6. Security & Quality Invariants

- `#![forbid(unsafe_code)]` at crate root.
- Zero `unwrap()` or `expect()` in production pathways.
- Bounded memory allocations with strict LRU capacity limits and automatic eviction of expired attempts.
- Clockless determinism: zero syscalls or async runtimes in the policy layer.
- Multi-frame consensus: no single capture can authorize; any spoof capture vetoes the request.
