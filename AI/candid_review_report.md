# Candid Review Report

- **Date**: 2026-10-17
- **Target Branch**: `fix/zbus-6`
- **Base (merge-base)**: `6eb0746`
- **Reviewed-Diff-Fingerprint**: `e91e6fbcd64d1dd1a9bba9bb765b40c2a077182c359a0891ab28b5d3fc42ef8e`
- **Audited Files**: `AI/ARCHITECTURE.md`, `AI/MOCK_STRATEGY.md`, `AI/VERIFICATION_MATRIX.md`, `AI/walkthroughs/221_zbus_6_migration.md`, `Cargo.lock`, `Cargo.toml`, `Docs/MACOS_PORT.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `crates/daemon/src/presence/logind.rs`, `crates/daemon/tests/presence_logind_error_name_tests.rs` (new), `crates/daemon/tests/presence_logind_mapping_tests.rs`, `crates/remote/src/logind.rs`, `crates/remote/src/offline.rs`, `crates/remote/src/session.rs`, `crates/remote/tests/logind_error_name_tests.rs` (new), `crates/remote/tests/session_tests.rs`, `tests/invariants/src/portable_crates_contract.rs`, `tests/invariants/src/presence_unlock_contract.rs`, `tests/invariants/src/remote_companion_contract.rs`

## 1. Executive Summary

The diff migrates the workspace `zbus` dependency from 5.19.0 to 6.0.0 (removing `zvariant`,
`zvariant_derive`, `zvariant_utils`, `zbus_names` and eight async helper crates; adding
`zbus_utils` 6.0.0 and `zruntime` 0.2.0; `tokio` moves 1.53.1 -> 1.53.3 because zbus 6 requires
it). The code changes are type renames (`OwnedValue`/`Value` -> `OwnedVariant`/`Variant`,
`zbus::zvariant::{Type,DynamicType}` -> `zbus::{Type,DynamicType}`), the infallible
`Builder::address` (the address error is now reported by `build()`), `max_queued` taking a
`NonZeroUsize`, and a new `method_error_name` classifier that compensates for zbus 6 reporting
standard `org.freedesktop.DBus.Error.*` error replies as `Error::FDO` instead of
`Error::MethodError`.

Every behavioural claim was verified against the zbus 6.0.0 and zbus 5.19.0 / zvariant 5.15.0
sources and with a throwaway probe built against zbus 6.0.0. D-Bus behaviour of both logind
clients and the NetworkManager reader is preserved: same calls, same timeouts and bounds, same
error-name strings, same reset-on-transport-failure and fail-closed paths. No CRITICAL or MAJOR
finding.

## 2. Test Changes (mechanical listing from step 3, with justification per change)

Step-3 greps on `target/candid_diff.patch`:

- Removed/changed assertion lines: exactly one hit by the grep (`presence_unlock_contract.rs`,
  `version = "5"` -> `version = "6"`). A manual read of all test hunks finds four assertion
  changes in total, all in commit `96523e9` and all owner-approved pin updates:
  1. `test_pau_zbus_is_declared_once_in_the_workspace`: `version = "5"` -> `version = "6"`.
  2. `test_pau_lockfile_pins_zbus_5_without_libdbus`: `starts_with("version = \"5.")` -> `"6."`;
     the single-version and the `dbus`/`libdbus-sys` ban checks are unchanged.
  3. `test_mac_guard_query_sees_linux_only_crates`: daemon witness `zvariant` -> `zbus_macros`
     (`zvariant` no longer exists in the graph; `zbus_macros` is a real member of the graph and is
     in the forbidden list, so the witness still makes the check non-vacuous).
  4. `test_rmc_architecture_and_strategy_documents_describe_the_companion`: needle `` `zbus` 5 ``
     -> `` `zbus` 6 ``; the doc line still carries the other checked needles (test passes).
- Uncommitted change to `FORBIDDEN_PORTABLE_DEPENDENCIES`: 31 -> 33 entries (`zbus_utils`,
  `zruntime` added; `zbus_names`/`zvariant` kept as defensive entries). This makes the guard
  stricter, it does not weaken it. The uniqueness assertion over the list still holds.
- `presence_logind_mapping_tests.rs` and `session_tests.rs`: only imports, helper signatures and
  the module doc change (`OwnedValue` -> `OwnedVariant`); no assertion, input value or test body
  changes.
- New escape hatches (`#[ignore]`, `should_panic`, tolerance, epsilon): none.
- Inline `mod tests` changes: none.
- New tests: `crates/daemon/tests/presence_logind_error_name_tests.rs` and
  `crates/remote/tests/logind_error_name_tests.rs` (3 tests each). Each one first asserts the zbus 6
  variant (`FDO` vs `MethodError`) the hand-built reply produces, then asserts that the exact name
  is recovered, and that `InvalidReply`, `InputOutput` and `FDO(ZBus(_))` give `None`. They would
  fail against the naive port (matching only `MethodError`) for all nine standard names.

## 3. Deep Reasoning Audit

### Logic & Architecture

- **Error-reply classification.** zbus 5 `From<Message> for Error` always built
  `MethodError(name, detail, msg)`. zbus 6 builds the same value and passes it through
  `fdo::Error::from` (macro-generated in `zbus_macros-6.0.0/src/error.rs`): a name equal to a
  `fdo::Error` variant's fully qualified name becomes that variant, wrapped in `Error::FDO`; any
  other name returns `fdo::Error::ZBus(original)`, which `From<fdo::Error> for Error` unwraps back
  to the original `MethodError`. `DBusError::name` for a `fdo::Error` variant returns
  `ErrorName::from_static_str_unchecked(fqn)` with the same `fqn` the conversion matched, so the
  name is round-trip exact (including the `Spawn.*` names). `method_error_name` excludes
  `fdo::Error::ZBus(_)`, so a wrapped transport error can never be taken for a refusal. Result:
  the set of failures mapped to `CallFailure::Method(name)` (daemon, remote) and to
  `SourceError::Call` (offline) is exactly the zbus 5 `MethodError` set, with the same name
  strings, so `VANISHED_ERRORS` (`UnknownObject`, `NoSuchSession`) and the bounded error text
  behave exactly as before. PASS.
- **No other FDO producer on the client path.** Searched zbus 6 for `Error::FDO` construction
  outside tests and the object server: only the `From` impls above and `proxy/mod.rs` (no proxy
  is used). `Error::DBus` is produced only by the object server (none registered). Reply errors
  are produced only by `socket_reader.rs:139` (`Type::Error => Err(msg.clone().into())`),
  identical to zbus 5. PASS.
- **Method timeout.** zbus 6 still reports a `method_timeout` expiry as
  `Error::InputOutput(ErrorKind::TimedOut)` (`runtime/timeout.rs`), so it stays a transport
  failure with a connection reset, as in zbus 5; the outer `tokio::time::timeout` paths are
  untouched. PASS.
- **`Builder::address`.** zbus 6 records an invalid address and reports it from `build()`
  (`connection/builder.rs:205-214`); the `build()` error arm already maps to `BusUnavailable` in
  both `ZbusLogind::connect` and `connect_system_bus`, inside the same connect timeout. The
  addresses themselves (`self.address()`, `SYSTEM_BUS_ADDRESS`) are unchanged. PASS.
- **`max_queued`.** The bound is still 16 (`NonZeroUsize`), built in a `const` with a
  non-panicking fallback. PASS.
- **Reply decoding.** Probe against zbus 6.0.0: a `Properties.Get` reply of signature `v`
  deserialized as `OwnedVariant` gives `Bool(true)` directly (not a nested variant), and an
  `a{sv}` `GetAll` reply gives `Structure([U32, ObjectPath])` for `User`, exactly the shapes that
  `as_bool`, `first_field` and the `u32`/`bool` `TryFrom` conversions expect.
  `ov_try_from!`/`value_try_from!` in zbus 6 `wire/owned_variant.rs`/`from_variant.rs` are
  token-for-token the zvariant 5.15.0 implementations (strict variant match, `IncorrectType`
  otherwise). PASS.
- **Scope.** Daemon and remote are the only zbus consumers (`git grep`); no consumer was missed.
  The RMC17 source needles (`Builder::address(SYSTEM_BUS_ADDRESS)`, `.max_queued(`, no proxy,
  object server or signal stream) still hold; the invariant suite passes. PASS.

### PAM Concurrency & Deadlines

`crates/pam` is not touched and does not depend on zbus (PAU17 invariant passes). PASS.

### Panic Safety & Fail-Closed

No `unwrap`/`expect`/indexing added in production code; the `const` uses a `match`. Every error
still maps to `BusUnavailable`/`Timeout`/`Malformed`/`Call`, and nothing maps to an unlock or a
"session present" result that did not do so before. Scenario tried: a zbus 6 `FDO(ZBus(InvalidReply))`
-> `None` -> transport + reset (test-covered). PASS.

### Test Integrity & Anti-Weakening

See §2. The only assertion changes are the four owner-approved pin updates in `96523e9`. The
forbidden-crate list got stricter. Tests run: `cargo test --locked -p soos-invariants -p soos-remote`
(707 invariant tests plus all remote suites) and `cargo test --locked -p soos-daemon` all green
with `--test-threads=4`; `cargo clippy --locked -p soos-daemon -p soos-remote --all-targets -- -D warnings`
clean. PASS.

### Memory, Bounds & Secrets

Bounds are unchanged: queue 16, `DBUS_CALL_TIMEOUT_MS`, `DBUS_CONNECT_TIMEOUT_MS`,
`MAX_LOGIND_ERROR_LEN` / `truncated_error_name`. No new logging. The error description (`detail`)
is not read by `method_error_name`; only the name is kept. PASS.

### Supply Chain & Automation

`cargo deny --locked check` -> `advisories ok, bans ok, licenses ok, sources ok`. Lockfile: 13
packages removed (including the three RUSTSEC-2026-0370/0371/0372 crates), 2 added (`zbus_utils`,
`zruntime`, both from crates.io), `tokio` 1.53.1 -> 1.53.3 (patch bump required by zbus 6,
documented in `AI/ARCHITECTURE.md`). MSRV: zbus 6.0.0 / zbus_macros / zbus_utils declare 1.89,
zruntime 1.88, tokio 1.53.3 declares 1.71, all at or below the workspace 1.92. Features: still
`default-features = false, features = ["tokio"]`; no `default-rt`, `p2p`, `unixexec` or process
features are enabled. No workflow or script change. PASS.

### English-Only Policy

All added text (code, comments, docs, walkthrough) scanned; English only. PASS.

## 4. Detailed Findings & Action Items

- **[SUGGESTION]** `crates/daemon/src/presence/logind.rs:240` and `crates/remote/src/logind.rs:143`:
  `method_error_name` is duplicated word for word in both crates, along with two identical test
  files. It is the same pattern as the existing duplicated `call()` mapping, so this is acceptable
  for now. A future shared helper would give it one source of truth.
- **[SUGGESTION]** `tests/invariants/src/presence_unlock_contract.rs:249`: the test name
  `test_pau_lockfile_pins_zbus_5_without_libdbus` (also cited in `AI/VERIFICATION_MATRIX.md`
  PAU17) now asserts zbus 6. Renaming it is optional and needs owner approval because it is a test
  change.
- **[SUGGESTION]** `AI/DECISIONS.md`: the ADR 2026-10-02 text still says `zbus` 5 (it is a
  historical record and is left untouched). If the project wants an ADR trail for a dependency
  major bump, it could add a short dated note there that cites #416 and the three advisories.
- **[SUGGESTION]** The new tests check `method_error_name` in isolation. No test drives the
  `call()` arms themselves (no reset for an error reply, reset for a transport failure). Those
  arms are a direct `match` on the classifier and were read by hand, so the risk is low.

## 5. Final Verdict

**VERDICT: APPROVED**
