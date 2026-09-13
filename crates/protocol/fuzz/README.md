# soos-protocol Fuzzing Harness

This directory provides LLVM libFuzzer integration via `cargo-fuzz` for fuzz testing the binary codec and message schemas in `soos-protocol`.

## Fuzz Targets

- **`decode_request`**: Sub-issue #4.1 — Feeds arbitrary mutated byte sequences into `decode::<Request>` and runs `req.validate()`. Asserts zero panics and memory safety across millions of iterations.
- **`decode_response`**: Sub-issue #4.2 — Feeds arbitrary mutated byte sequences into `decode::<Response>` and runs `resp.is_allow()`. Asserts zero panics.
- **`decode_event`**: Feeds arbitrary mutated byte sequences into `decode::<Event>`. Asserts zero panics.

## Prerequisites

Fuzzing with `cargo-fuzz` requires a Rust Nightly toolchain and `cargo-fuzz`:

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
```

## Running the Fuzzers

Navigate to `crates/protocol` and execute:

```bash
# Fuzz Request decoding (e.g. 1,000,000 runs or continuous)
cargo +nightly fuzz run decode_request -- -runs=1000000

# Fuzz Response decoding
cargo +nightly fuzz run decode_response -- -runs=1000000

# Fuzz Event decoding
cargo +nightly fuzz run decode_event -- -runs=1000000
```

## Property-Based Fuzzing in CI

In addition to `cargo-fuzz`, continuous property-based fuzzing is integrated into the default test suite via `proptest` in `crates/protocol/tests/property_tests.rs`. This executes on every standard `cargo test` run without requiring Nightly or `cargo-fuzz`.
