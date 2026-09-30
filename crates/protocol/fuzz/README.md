# soos-protocol Fuzzing Harness

This directory provides LLVM libFuzzer integration via `cargo-fuzz` for fuzz testing the binary codec and message schemas in `soos-protocol`.

## Fuzz Targets

- **`decode_request`**: Sub-issue #4.1 — Feeds arbitrary mutated byte sequences into `decode::<Request>` and runs `req.validate()`. Asserts zero panics and memory safety across millions of iterations.
- **`decode_response`**: Sub-issue #4.2 — Feeds arbitrary mutated byte sequences into `decode::<Response>` and runs `resp.is_allow()`. Asserts zero panics.
- **`decode_event`**: Feeds arbitrary mutated byte sequences into `decode::<Event>`. Asserts zero panics.
- **`decode_preview`**: GitHub #227 — Feeds arbitrary bytes into `decode_preview::<PreviewResponse>` (2 MiB preview limit). Asserts zero panics and that a decoded frame never exceeds `MAX_PREVIEW_MESSAGE_SIZE`.
- **`decode_client_message`**: GitHub #204 — Feeds arbitrary payloads into `message::decode_client_message`. Asserts zero panics and that a tagged classification always comes from a reserved trailer byte (`>= 0x80`) while a legacy one never does.

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

## Nightly CI

`.github/workflows/fuzz.yml` runs every target nightly (and on manual dispatch) for a fixed
`-max_total_time` budget and uploads crash reproducers from `fuzz/artifacts/` as workflow
artifacts. The harness is its own Cargo workspace (`[workspace]` table in `fuzz/Cargo.toml`,
committed `fuzz/Cargo.lock`): without it cargo refuses to build it inside the root workspace.
`fuzz/target`, `fuzz/corpus` and `fuzz/artifacts` are git-ignored.

## Property-Based Fuzzing in CI

In addition to `cargo-fuzz`, continuous property-based fuzzing is integrated into the default test suite via `proptest` in `crates/protocol/tests/property_tests.rs`. This executes on every standard `cargo test` run without requiring Nightly or `cargo-fuzz`.
