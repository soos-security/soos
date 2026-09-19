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
   - Authentication request: 200–250ms total execution budget (connect, send request, await verdict).
   - Telemetry notification (`event=password-failed`): strict 20ms ceiling.
   - Connection to the daemon socket utilizes non-blocking `connect()` with `libc::poll` to guarantee the client never hangs on frozen daemons or full backlogs.

3. **Fail-Closed Default (`PAM_IGNORE`)**:
   Any network timeout, protocol error, daemon unavailability, or crash degrades silently to `PAM_IGNORE` (Linux-PAM return code 25). `PAM_SUCCESS` is rendered exclusively upon receiving `Verdict::Allow`. Under no circumstances can an error or panic yield `PAM_SUCCESS`.

4. **Panic Safety & Syslog Alerting**:
   Every entrypoint and argument parsing routine (`parse_argv`, `parse_cstrs`) is wrapped in `catch_unwind(AssertUnwindSafe(...))` via `catch_c_entry`. Caught panics trigger bounded alerting to the system authentication facility (`LOG_AUTHPRIV | LOG_ERR`) and return `PAM_IGNORE`. Stderr output is suppressed to prevent graphical display manager crashes.

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
- **Memory Zeroization**: The `soos-protocol` `Request` and `Response` structs implement `zeroize::Zeroize` and `Drop`. In addition, intermediate buffers and cryptographic nonces (`request_id`, `len_buf`, `full_buf`, `encoded`) are wrapped in `Zeroizing` wrappers to guarantee prompt memory erasure when dropped.

