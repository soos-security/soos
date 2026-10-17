# Walkthrough 221 — zbus 6 for RUSTSEC-2026-0370, RUSTSEC-2026-0371 and RUSTSEC-2026-0372

- **Date**: 2026-10-17
- **Issue**: GitHub-only #416, no backlog id. **Branch**: `fix/zbus-6`. It is not registered in `scripts/sync_issue.py`; the commit message carries `Closes #416`.
- **Matrix criteria**: no new row. The text of PAU17, RMC17 and RMC19 now names zbus 6 (zbus 5 until #416); their tests are unchanged apart from the owner-approved version pins below.
- **Numbering**: 221 was reserved for this change.

## 1. Context & Objectives

Since 2026-10-17, `cargo deny --locked check` fails on `main` and on every pull request. RustSec published three advisories against crates of the zbus 5 stack:

- **RUSTSEC-2026-0370**: `zvariant` 5.15.0 is unmaintained.
- **RUSTSEC-2026-0371**: `zbus_names` 4.3.4 is unmaintained.
- **RUSTSEC-2026-0372**: `zvariant_derive` 5.15.0 is unmaintained.

All three were merged into `zbus` 6.0.0 (`zvariant` became `zbus::wire`, `zbus_names` became `zbus::names`, the derives moved into `zbus_macros`). `deny.toml` keeps `ignore = []`, so the fix is the migration, not a waiver. The goal is identical behaviour of the two logind clients: the presence auto-unlock of `soos-daemon` (ADR 2026-10-02 "Presence Auto-Unlock Through logind") and the lock status, lock, opt-in unlock and offline probes of `soos-remote` (ADR 2026-10-05 "Remote Companion `soos-remote`", ADR 2026-10-06, security hardening 2026-10-12).

## 2. Architect Design

The workspace dependency becomes `zbus = { version = "6", default-features = false, features = ["tokio"] }`. In zbus 6 the `tokio` feature enables only the connection layer (`comms` plus Tokio); `proxy`, `service`, `object-manager`, `tracing`, `p2p`, `ibus` and `unixexec` stay off, so the proxy macros, the object server and the blocking API are not even compiled (the RMC-S9 / presence C2 needles stay vacuous by construction).

API changes, each mechanical except the last:

| zbus 5 | zbus 6 | Change in soos |
|---|---|---|
| `zbus::zvariant::{Value, OwnedValue, OwnedObjectPath, ObjectPath, Type, DynamicType}` | crate root: `Variant`, `OwnedVariant`, `OwnedObjectPath`, `ObjectPath`, `Type`, `DynamicType` (the old module is a deprecated alias shim) | Renamed in `presence/logind.rs`, `remote/logind.rs`, `remote/session.rs`, `remote/offline.rs` and the imports of the two mapping test files. Same types, same encoding. |
| `connection::Builder::address(..)` returns `Result<Builder>` | infallible; an invalid address is reported by `build()` | The `.map_err(..BusUnavailable)?` after `address(..)` is removed. A bad address now fails inside `build()`, which both connect paths already map to `BusUnavailable` (the pinned address is valid, so this path is not reached in practice). |
| `Builder::max_queued(usize)` | `max_queued(NonZeroUsize)` | `MAX_QUEUED_MESSAGES` becomes a `NonZeroUsize` const built by a const `match` (no `unwrap`, value still 16). |
| An error reply always becomes `Error::MethodError(name, ..)` | an error reply with a standard `org.freedesktop.DBus.Error.*` name becomes `Error::FDO(Box<fdo::Error>)`; other names stay `MethodError` | **Behavioural trap.** New `method_error_name(&zbus::Error) -> Option<String>` in `presence::logind` and `soos_remote::logind`: `MethodError` gives its name, `FDO` (other than the `ZBus` fall-through) gives `DBusError::name()`, anything else `None`. The three call sites (`ZbusLogind::call`, `ZbusSessionSource::call`, `ZbusNetworkSource::reply`) use it instead of matching `MethodError` only. |

Without the last row, `org.freedesktop.DBus.Error.UnknownObject` (one of the two `VANISHED_ERRORS`: "the session vanished between two calls") would have become a transport failure that drops the connection, a polkit `AccessDenied` on `UnlockSession` would have been reported as `BusUnavailable` instead of `call(<name>)`, and `ServiceUnknown` from NetworkManager in the offline probe would have dropped the connection instead of returning `SourceError::Call`. None of these turns a failure into a success, but each is a behaviour change, and no existing test covered it (all logind tests are bus-free).

Unchanged: the pinned `SYSTEM_BUS_ADDRESS`, `DBUS_CALL_TIMEOUT_MS` (zbus `method_timeout` and the per-call `tokio::time::timeout`), `DBUS_CONNECT_TIMEOUT_MS`, the queue bound 16, every D-Bus destination, path, interface, method and body, the `VANISHED_ERRORS` lists, every error mapping and every reset-on-failure rule. The zbus 6 internal method timeout still yields `Error::InputOutput(TimedOut)` (`runtime/timeout.rs`), a transport failure as before.

MSRV: zbus, zbus_macros and zbus_utils 6.0.0 declare `rust-version = 1.89`, zruntime 0.2.0 declares 1.88 and tokio 1.53.3 declares 1.71, all within the workspace `rust-version = 1.92`.

## 3. Plan Evaluation

Condensed for a dependency migration. Alternatives rejected:

- Ignoring the advisories in `deny.toml`: forbidden by policy while a maintained successor exists.
- Keeping the deprecated `zbus::zvariant` aliases with `#[allow(deprecated)]`: hides the migration and breaks once the shim is removed.
- Adding the `proxy` feature to use typed `fdo` proxies: widens the compiled surface the RMC-S9 rules forbid.
- Matching `Error::FDO` variants one by one (`UnknownObject`, `AccessDenied`, ...): the error names are already compared as strings by `VANISHED_ERRORS`; one name extraction keeps the existing string logic exact for every name.

Verdict: APPROVED.

## 4. Tester Contract

The existing bus-free mapping tests (`presence_logind_mapping_tests`, `session_tests`, the presence and remote suites) are the contract for the renames. The error-reply classification was uncovered, so two new test files pin it with hand-built error replies (no bus):

- `crates/daemon/tests/presence_logind_error_name_tests.rs`
- `crates/remote/tests/logind_error_name_tests.rs`

Each asserts that nine standard names (including `UnknownObject`, `AccessDenied`, `ServiceUnknown`, `NoReply` and a `Spawn.*` name) arrive as `Error::FDO` and keep their exact name, that logind names arrive as `Error::MethodError` and keep their exact name, and that `InvalidReply`, an I/O error and `FDO(ZBus(..))` are not method errors. They were written before `method_error_name` existed. Red evidence: with a classifier that matches `Error::MethodError` only (the zbus 5 logic), `test_standard_error_reply_keeps_its_exact_name` fails at its first name (`UnknownObject`) while the two other tests pass.

Contract migrations (owner-approved on 2026-10-17, applied by the lead in commit `96523e9`, not by this branch's developer phase):

- `presence_unlock_contract::test_pau_zbus_is_declared_once_in_the_workspace`: `version = "5"` → `"6"`.
- `presence_unlock_contract::test_pau_lockfile_pins_zbus_5_without_libdbus`: `version = "5.` → `"6.` (test name unchanged).
- `portable_crates_contract::test_mac_guard_query_sees_linux_only_crates`: the removed `zvariant` witness of `soos-daemon` → `zbus_macros`.
- `remote_companion_contract::test_rmc_architecture_and_strategy_documents_describe_the_companion`: needle `` `zbus` 5 `` → `` `zbus` 6 ``.

Import-only changes (allowed by the owner): `presence_logind_mapping_tests.rs` and `session_tests.rs` import `zbus::{ObjectPath, OwnedVariant, Variant}` instead of the deprecated `zbus::zvariant` aliases; their helper signatures follow the rename and no assertion changes.

Stricter guard (addition only): `FORBIDDEN_PORTABLE_DEPENDENCIES` gains `zbus_utils` and `zruntime`, the two new crates of the zbus 6 stack; `zbus_names` and `zvariant` stay as defensive entries; `Docs/MACOS_PORT.md` lists both new names.

## 5. Auditor Constraints

1. **No new privilege, no new bus.** Both crates still connect only through `Builder::address(SYSTEM_BUS_ADDRESS)`; zbus 6 adds no environment lookup on this path (`address()` takes the string as given). No object server, well-known name, signal match, proxy or blocking API is compiled. Met.
2. **Fail closed.** Every failure path still returns an error variant; no error is turned into a success. `UnknownObject` / `NoSuchSession` stay "session vanished" (`Ok(None)`, the presence worker never unlocks a session it cannot re-read), `AccessDenied` stays `call(<name>)` (the error text is still bounded by `MAX_LOGIND_ERROR_LEN` and the remote log still truncates the name). Met through `method_error_name` and its tests.
3. **Bounds.** Call timeout, connect timeout and queue bound are the same constants; the queue type change cannot express 0. Met.
4. **No panic in production paths.** No `unwrap`/`expect`/`panic!` added (the presence invariant scans `crates/daemon/src/presence`); the `NonZeroUsize` const uses a const `match`. Met.
5. **No sensitive data logged.** The FDO detail text is never read; only the error name reaches `PresenceLogindError::call` and the remote `debug!` line, exactly as before. Met.
6. **Supply chain.** zbus 6 adds 12 crates in all (MIT or MIT/Apache-2.0; 22 for zbus 5). Lockfile delta: removed `zvariant`, `zvariant_derive`, `zvariant_utils`, `zbus_names`, `async-broadcast`, `async-recursion`, `enumflags2`, `enumflags2_derive`, `event-listener`, `event-listener-strategy`, `futures-lite`, `parking`; added `zbus_utils` 6.0.0 and `zruntime` 0.2.0; updated `zbus` and `zbus_macros` 5.19.0 → 6.0.0 and `tokio` 1.53.1 → 1.53.3 (required by zbus 6; patch release, no `AsyncFd` use in the workspace, so its new deprecations do not apply). No new duplicate version, no `deny.toml` change. Met.
7. **MSRV.** No dependency raises the MSRV above 1.92 (§2); `clippy -D warnings` reports no `incompatible_msrv`. Met; the CI System ONNX Runtime job confirms on 1.92.0.

Clearance: CLEARED.

## 6. Implementation

- `Cargo.toml`, `Cargo.lock`: zbus 6 (§5.6).
- `crates/daemon/src/presence/logind.rs`, `crates/remote/src/logind.rs`, `crates/remote/src/session.rs`, `crates/remote/src/offline.rs`: §2.
- Tests: §4.
- Docs: `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` (`zbus` 6, 12 crates), `AI/ARCHITECTURE.md` (verified versions zbus 6.0.0, tokio 1.53.3), `Docs/MACOS_PORT.md` (guard list), `AI/MOCK_STRATEGY.md` (`zbus::OwnedVariant` maps), `AI/VERIFICATION_MATRIX.md` (PAU17, RMC17, RMC19 text).

No ADR change: the dependency decision (zbus, no default features, Tokio only, two crates) is unchanged; ADR 2026-10-02 keeps its historical "`zbus` 5, 22 new crates" wording, superseded for the version by this walkthrough.

## 7. Candid Review

See `AI/candid_review_report.md` (bound to the final diff fingerprint).

## 8. Verification Results

Run with `LIBONNXRUNTIME_NO_PKG_CONFIG=1`, `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=4`, toolchain 1.98.1:

- `cargo deny --locked check`: `advisories ok, bans ok, licenses ok, sources ok`.
- `cargo tree --locked -i zvariant`, `-i zvariant_derive`, `-i zbus_names`: no matching package.
- `cargo tree --locked -p soos-daemon -e features -i zbus`: only the `tokio`, `comms` and `zruntime` features of zbus 6.
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean (no `deprecated`, no `incompatible_msrv`).
- `cargo test --locked -p soos-daemon -p soos-remote -p soos-invariants --all-targets --all-features -- --test-threads=4`: 1939 passed, 0 failed, 2 ignored.
- `cargo test --locked --workspace --all-targets --all-features -- --test-threads=4`: 4107 passed, 0 failed, 6 ignored across 437 test binaries.

## 9. Known Limitations / Follow-ups

- The logind clients are still tested without a bus; the error-reply classification is pinned at the `zbus::Error` level, not through a live `call_method`.
- `zbus::zvariant` is a deprecated shim; nothing in the workspace uses it any more.
