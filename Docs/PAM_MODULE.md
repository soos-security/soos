# PAM Module Specification — `pam_soos.so`

## 1. Overview & Identity

`pam_soos.so` is a Linux-PAM dynamic shared library (`cdylib`) implementing local facial biometric verification.
It acts as a lightweight, unprivileged synchronous IPC client to `soos-daemon`.

- **Crate**: `crates/pam` (`soos-pam`)
- **Shared Object**: `pam_soos.so`
- **Bindings Engine**: `pam-bindings` 0.3.0 ([crates.io/crates/pam-bindings](https://crates.io/crates/pam-bindings))

---

## 2. Architectural Security Principles

1. **Zero Tokio / Asynchronous Runtime**:
   The module executes entirely within the memory and thread space of the calling PAM application (`sudo`, `login`, `gdm`, `swaylock`). Starting an asynchronous runtime (Tokio) inside a PAM shared library is strictly prohibited due to thread lifecycle hazards, signal interference, and process termination deadlocks. All socket operations use synchronous standard library primitives (`std::os::unix::net::UnixStream`).

2. **Strict Latency Budget & Non-Blocking Connection**:
   - Authentication request: one cumulative deadline derived from the clamped `timeout_ms` (default 1000 ms, range 10–5000 ms) covering connect, send request and await verdict; every `read()` / `write()` re-arms the socket timeout with the budget left (see §6).
   - Telemetry notification (`event=password-failed`): strict 20ms ceiling.
   - Connection to the daemon socket utilizes non-blocking `connect()` with `libc::poll` to guarantee the client never hangs on frozen daemons or full backlogs.

3. **Fail-Closed Default (`PAM_IGNORE`)**:
   Any network timeout, protocol error, daemon unavailability, or crash degrades silently to `PAM_IGNORE` (Linux-PAM return code 25). `PAM_SUCCESS` is rendered exclusively upon receiving `Verdict::Allow`. Under no circumstances can an error or panic yield `PAM_SUCCESS`.

4. **Panic Safety & Syslog Alerting**:
   Every entrypoint and argument parsing routine (`parse_argv`, `parse_cstrs`) is wrapped in `catch_unwind(AssertUnwindSafe(...))` via `catch_c_entry`. Caught panics trigger bounded alerting to the system authentication facility (`LOG_AUTHPRIV | LOG_ERR`) and return `PAM_IGNORE`. Stderr output is suppressed to prevent graphical display manager crashes.
   This guarantee depends on the build profile: `catch_unwind` only catches anything when the artifact is compiled with `panic = "unwind"`. The root `Cargo.toml` therefore pins `[profile.release] panic = "unwind"` (ADR 2026-09-29, review finding PAM-01 / GitHub #148); `tests/invariants` fails if any profile reintroduces `abort` or if a packaging script selects another profile, and Docker matrix case T10 loads the release-built shared object with the opt-in `fault-injection` feature (`fault_inject=panic` / `fault_inject=overflow` PAM arguments, see `crates/pam/src/fault_injection.rs`) to prove that an in-module panic yields `PAM_IGNORE` and password fallback rather than killing the PAM host. The feature is never a default and never enabled by packaging or install scripts; production builds ignore the `fault_inject=` argument entirely.

5. **Zero Secrets in Memory or Logs**:
   The module never inspects, captures, or transmits passwords. Syslog panic messages are strictly sanitized and never contain user data or IPC payloads. Requests, responses, nonces, and buffers implement `Zeroize` or `Zeroizing` for automatic, deterministic memory erasure on drop.

---

## 3. Integration with `pam-bindings` 0.3.0

The module implements the `PamHooks` trait provided by `pam-bindings`:

```rust
use pam_bindings::constants::{PamFlag, PamResultCode};
use pam_bindings::module::{PamHandle, PamHooks};

pub struct SoosPam;

impl PamHooks for SoosPam {
    fn sm_authenticate(
        pamh: &mut PamHandle,
        args: Vec<&CStr>,
        _flags: PamFlag,
    ) -> PamResultCode {
        // Authenticates user via synchronous IPC to daemon
    }

    fn sm_setcred(
        _pamh: &mut PamHandle,
        _args: Vec<&CStr>,
        _flags: PamFlag,
    ) -> PamResultCode {
        PamResultCode::PAM_IGNORE
    }
}
```

Standard PAM C ABI entrypoints (`pam_sm_authenticate`, `pam_sm_setcred`, `pam_sm_acct_mgmt`, `pam_sm_chauthtok`, `pam_sm_open_session`, `pam_sm_close_session`) are exported as `extern "C"` functions delegating to `SoosPam`.

---

## 4. Syslog Panic Logging

Caught panics are dispatched to syslog via POSIX `libc::syslog(LOG_AUTHPRIV | LOG_ERR, ...)`:

```text
soos-pam: authentication panic caught at crates/pam/src/lib.rs:92:9: <panic payload>
```

- Location (file, line, column) is captured via a thread-local panic hook.
- All embedded nul characters (`\0`) are replaced to prevent C string truncations.
- Format specifier `"%s"` is used to eliminate format string injection attacks.

---

## 5. PAM Stack Configuration Example

```pam
# Placed AFTER faillock preauth, BEFORE pam_unix
auth [success=done default=ignore] pam_soos.so timeout_ms=250

# Fallback to standard password authentication
auth [success=done default=bad]    pam_unix.so try_first_pass

# Optional telemetry notification if password failed
auth optional                      pam_soos.so event=password-failed timeout_ms=20
```

---

## 6. Non-Blocking Connect & Memory Zeroization

- **Non-Blocking Connect**: `connect_with_timeout` initializes an `AF_UNIX` socket with `SOCK_NONBLOCK` and calls `libc::connect`. In case of `EINPROGRESS`, `libc::poll` is executed with the remaining time budget (`remaining_budget`), ensuring that unresponsive daemons or full socket listen queues cannot block `pam_soos.so` past its configured `timeout_ms`. Once connected, the socket descriptor is converted into a standard `UnixStream` with non-blocking disabled, so subsequent reads/writes use `SO_RCVTIMEO` / `SO_SNDTIMEO`.
- **Cumulative Read/Write Deadline** (review PAM-02, GitHub #173): `SO_RCVTIMEO` / `SO_SNDTIMEO` bound a single syscall, not an exchange. `read_exact_before_deadline` and `write_all_before_deadline` therefore re-arm the socket timeout with the budget left (from one `Deadline` created at the start of `authenticate` / `notify_event`) before **every** `read()` / `write()`, retry `EINTR` only while budget remains, and a verdict whose last byte arrives after the deadline is discarded (`IpcError::Timeout` → `PAM_IGNORE`). A daemon that drip-feeds its response can no longer extend the PAM wait beyond `timeout_ms`.

---

## 7. Service Name and Disable Flags

- **Service resolution** (review PAM-05, GitHub #176): an explicit `service=<name>` argument wins. Otherwise the module reads the `PAM_SERVICE` item (`pam_get_item`) of the handle — e.g. `gdm-password`, `sudo`, `login` — validated as UTF-8, trimmed and bounded to `MAX_SERVICE_LEN` (64 bytes). The resolved name is sent to the daemon in `Request.service` / `Event.service` and drives the flags below. The lines installed by `soos-admin gdm enable` and the packaging snippets pass no `service=`, so this lookup is what makes `gdm.disable` effective.
- **Disable flags** (checked before any socket activity; a match returns `PAM_IGNORE` silently):
  - `/etc/soos/disabled` — disables every service.
  - `/etc/soos/gdm.disable` — disables every service whose name contains `gdm` (written by `soos-admin gdm disable`).
  - `/etc/soos/<service>.disable` — disables one service; only honored for names made of `[A-Za-z0-9._-]` that do not start with `.`, so a crafted service name can never reference a path outside `/etc/soos`.
  - `disabled` and `disable_if_file=<path>` PAM arguments.

---

## 8. User-Facing Messages

Feedback is sent through `PAM_TEXT_INFO` to the **unauthenticated** user and therefore depends on the verdict class only, never on `ReasonClass` (review PAM-03, GitHub #174):

| Outcome | Text |
|---|---|
| Request started | `[soos] Looking for face...` |
| `Verdict::Allow` | `[soos] Face recognized. Unlocking...` |
| `Verdict::Deny` (any reason, including a PAD / anti-spoofing rejection) | `[soos] Face not recognized.` |
| `Verdict::Unavailable`, `Verdict::ProtocolError`, or any IPC error | `[soos] Face verification unavailable.` |

Distinguishing a spoof rejection from a non-match would give a presentation attacker an oracle to iterate spoof material; distinguishing camera or model state would disclose device state to whoever sits at the locked machine. The detailed `ReasonClass` stays in the daemon logs only.
- **Memory Zeroization**: The `soos-protocol` `Request` and `Response` structs implement `zeroize::Zeroize` and `Drop`. In addition, intermediate buffers and cryptographic nonces (`request_id`, `len_buf`, `full_buf`, `encoded`) are wrapped in `Zeroizing` wrappers to guarantee prompt memory erasure when dropped.

