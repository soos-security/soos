# Walkthrough 19 — Protocol: Fuzzing Harness and Property-Based Testing

> **Date**: 2026-09-13  
> **Target**: Issue #4 (`protocol` Fuzzing Harness & Property-Based Testing)  
> **Branch**: `test/protocol-fuzzing`  
> **Verification Matrix**: P1, P2, P3, P4, P5, P6  

---

## 1. Overview & Objectives

Issue #4 establishes comprehensive fuzzing and property-based verification for the binary IPC codec and message schemas in `crates/protocol/`.

Because `soos-protocol` deserializes data across the unprivileged PAM module and the root daemon, parser robustness is critical to security. Malformed, adversarial, truncated, or oversized payloads must be handled deterministically without crashes, unbounded memory allocations, or panics.

### Architectural Invariants Enforced
1. **Zero Panic on Arbitrary Input (`P5`)**: Feeding completely random, truncated, or adversarial byte streams to `decode::<Request>`, `decode::<Response>`, or `decode::<Event>` must never trigger panics or crashes.
2. **Strict Message Size Bounds (`P2`)**: Any frame claiming a declared payload size $> 4{,}096$ bytes (`MAX_MESSAGE_SIZE`) must be rejected immediately prior to body buffer allocation.
3. **Round-Trip Idempotency (`P1`)**: Any valid `Request`, `Response`, or `Event` structure serialized through `encode` must deserialize through `decode` into an identical structure.
4. **Service Length Bounds (`MAX_SERVICE_LEN`)**: PAM service names exceeding 64 bytes are rejected by `validate()` with `ValidationError::ServiceTooLong`.
5. **Zero Unsafe Code (`P6`)**: `#![forbid(unsafe_code)]` remains strictly enforced throughout the crate.

---

## 2. Multi-Agent TDD Implementation Cycle

### Phase 1 — Architect Sub-Agent
- Designed property testing strategies and fuzzing harness integration:
  - Extended `Request`, `Response`, and `Event` in `crates/protocol/src/types.rs` with `#[derive(PartialEq, Eq)]` to enable structural equivalence assertions.
  - Scaffolding `crates/protocol/fuzz/` containing `Cargo.toml`, libFuzzer targets, and execution documentation.

### Phase 1.5 — Plan Evaluator Sub-Agent
- Conducted independent compliance evaluation across the 6 architectural pillars:
  - Pillar 1: Architectural Alignment & Threat Model (**PASS**)
  - Pillar 2: PAM Real-Time Latency & Concurrency (**PASS**)
  - Pillar 3: Panic Safety & Fail-Closed Behavior (**PASS**)
  - Pillar 4: Dependency Isolation & Banned Crates (**PASS**)
  - Pillar 5: Data Confidentiality & Zeroization (**PASS**)
  - Pillar 6: Test Integrity & TDD Contracts (**PASS**)
- Verified plan with **`VALIDATION_VERDICT: APPROVED`**.

### Phase 2 — Tester Sub-Agent (TDD Red Phase)
- Authored contract test suite `crates/protocol/tests/property_tests.rs`:
  - `prop_request_roundtrip`: Arbitrary valid `Request` round-trip.
  - `prop_response_roundtrip`: Arbitrary valid `Response` round-trip.
  - `prop_event_roundtrip`: Arbitrary valid `Event` round-trip.
  - `prop_decode_request_never_panics`: Fuzzing `decode::<Request>` with arbitrary bytes ($0$ to $8{,}192$ bytes).
  - `prop_decode_response_never_panics`: Fuzzing `decode::<Response>` with arbitrary bytes.
  - `prop_decode_event_never_panics`: Fuzzing `decode::<Event>` with arbitrary bytes.
  - `prop_declared_size_bounds`: Verifying immediate `DeclaredSizeTooLarge` rejection.
  - `prop_truncated_buffer_bounds`: Verifying `BufferTooSmall` rejection.
  - `prop_oversized_service_validation`: Verifying `ServiceTooLong` rejection on oversized services.
- Verified test failure (TDD Red state): compilation failed prior to `PartialEq` implementation.

### Phase 3 — Auditor Sub-Agent
- Audited interfaces and code:
  - Verified `#![forbid(unsafe_code)]` maintained in `crates/protocol`.
  - Confirmed zero `unwrap()`, `expect()`, or panics in library code.
  - Verified that `Zeroize` and `ZeroizeOnDrop` implementations on `Response` remain functional and undisturbed.
  - Verified that no sensitive fields (passwords, embeddings, frames) were introduced.

### Phase 4 — Developer Sub-Agent (TDD Green Phase)
- Implemented `PartialEq, Eq` derive on `Request`, `Response`, and `Event`.
- Created standalone `cargo-fuzz` sub-crate in `crates/protocol/fuzz/`:
  - `fuzz_targets/decode_request.rs`: libFuzzer harness for `decode::<Request>`.
  - `fuzz_targets/decode_response.rs`: libFuzzer harness for `decode::<Response>`.
  - `fuzz_targets/decode_event.rs`: libFuzzer harness for `decode::<Event>`.
  - `crates/protocol/fuzz/README.md`: Execution instructions.
- All 9 property tests passed with 250 cases each (2,250 property runs).

### Phase 5 — Candid Reviewer Sub-Agent
- Independent diff audit confirmed:
  - Zero unsafe in business crates.
  - Zero unwrap/expect in production pathways.
  - Zero OpenCV / Nokhwa dependencies.
  - Strict English-only deliverables.

### Phase 6 — Traceability Sub-Agent
- Synchronized `AI/BACKLOG.md`:
  - Sub-issue #4.1: Checked off.
  - Sub-issue #4.2: Checked off.
  - Sub-issue #4.3: Checked off.
- Updated `AI/VERIFICATION_MATRIX.md`:
  - Criterion `P5` updated to `☑ Validated`.
- Updated `Docs/IPC_PROTOCOL.md` with Section 5 on fuzzing and property-based verification.
- Updated `scripts/sync_issue.py` with `test/protocol-fuzzing` branch mapping.

---

## 3. Verification Evidence

```text
running 9 tests
test tests::prop_truncated_buffer_bounds ... ok
test tests::prop_declared_size_bounds ... ok
test tests::prop_event_roundtrip ... ok
test tests::prop_response_roundtrip ... ok
test tests::prop_request_roundtrip ... ok
test tests::prop_oversized_service_validation ... ok
test tests::prop_decode_response_never_panics ... ok
test tests::prop_decode_request_never_panics ... ok
test tests::prop_decode_event_never_panics ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.44s
```

All architectural invariants and unit tests across all workspace crates pass cleanly.
