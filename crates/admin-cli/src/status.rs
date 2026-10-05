//! Daemon health and status query subsystem.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use soos_protocol::codec::decode;
use soos_protocol::message::encode_request;
use soos_protocol::types::{
    Request, RequestKind, StatusResponse, CURRENT_VERSION, MAX_MESSAGE_SIZE, REQUEST_ID_LEN,
};

use crate::error::AdminCliError;

/// Upper bound of the `systemctl show` call made by `soos-admin status` (GitHub #331, P-2).
pub const SYSTEMCTL_SHOW_TIMEOUT_MS: u64 = 1000;

/// Bytes of `systemctl show` output read at most (three properties need < 200 bytes).
const MAX_SYSTEMCTL_OUTPUT_BYTES: u64 = 4096;

/// Poll interval of the `systemctl` child while waiting for it.
const SYSTEMCTL_POLL_INTERVAL_MS: u64 = 10;

/// Aggregated daemon health and runtime report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonStatusReport {
    pub socket_path: String,
    pub socket_ready: bool,
    pub camera_ready: bool,
    pub models_verified: bool,
    pub is_healthy: bool,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
    /// Whether the daemon pinned its memory with `mlockall` (`StatusResponse::memory_locked`,
    /// GitHub #201 / #287); `None` when the daemon cannot be contacted (unknown).
    pub memory_locked: Option<bool>,
    pub systemd_unit: String,
    pub systemd_active_state: String,
    pub systemd_sub_state: String,
}

impl DaemonStatusReport {
    /// Formats the report as a pretty-printed JSON document.
    ///
    /// Produced by `serde_json`, so the socket path and unit names are escaped (GitHub #232).
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self)
            .unwrap_or_else(|_| "{\"error\": \"report serialization failed\"}".to_string())
    }

    /// Formats the report as an aligned terminal summary table.
    #[must_use]
    pub fn format_table(&self) -> String {
        let mut out = String::new();
        out.push_str("====================================================\n");
        out.push_str("              SOOS DAEMON STATUS REPORT             \n");
        out.push_str("====================================================\n");
        out.push_str(&format!("Systemd Unit:        {}\n", self.systemd_unit));
        out.push_str(&format!(
            "Unit State:          {} ({})\n",
            self.systemd_active_state, self.systemd_sub_state
        ));
        out.push_str(&format!(
            "Daemon PID:          {}\n",
            self.pid
                .map_or_else(|| "N/A".to_string(), |p| p.to_string())
        ));
        out.push_str(&format!(
            "Daemon Uptime:       {}\n",
            self.uptime_secs
                .map_or_else(|| "N/A".to_string(), |u| format!("{u}s"))
        ));
        out.push_str(&format!("Socket Path:         {}\n", self.socket_path));
        out.push_str("----------------------------------------------------\n");
        out.push_str("Component Readiness:\n");
        out.push_str(&format!(
            "  Socket Listener:   {}\n",
            if self.socket_ready {
                "READY"
            } else {
                "OFFLINE"
            }
        ));
        out.push_str(&format!(
            "  Camera Capture:    {}\n",
            if self.camera_ready {
                "READY"
            } else {
                "OFFLINE"
            }
        ));
        out.push_str(&format!(
            "  ONNX Models:       {}\n",
            if self.models_verified {
                "VERIFIED"
            } else {
                "UNVERIFIED"
            }
        ));
        out.push_str(&format!(
            "  Swap Protection:   {}\n",
            match self.memory_locked {
                Some(true) => "LOCKED",
                Some(false) => "NOT LOCKED (memory may be swapped out)",
                None => "N/A",
            }
        ));
        out.push_str("----------------------------------------------------\n");
        out.push_str(&format!(
            "Overall Health:      {}\n",
            if self.is_healthy {
                "HEALTHY"
            } else {
                "UNHEALTHY"
            }
        ));
        out.push_str("====================================================\n");
        out
    }
}

/// Queries daemon component readiness and systemd unit status.
///
/// If the daemon socket cannot be contacted, returns an offline report
/// rather than failing hard, displaying the systemd unit status and
/// offline indicators.
///
/// # Errors
///
/// Returns `AdminCliError` only on irrecoverable codec errors.
pub fn query_status(
    socket_path: &Path,
    unit_name: &str,
) -> Result<DaemonStatusReport, AdminCliError> {
    let (active_state, sub_state, sys_pid) = inspect_systemd_unit(unit_name);

    match query_daemon_socket(socket_path) {
        Ok(status_resp) => {
            let active = if active_state == "unknown" {
                "active".to_string()
            } else {
                active_state
            };
            let sub = if sub_state == "unknown" {
                "running".to_string()
            } else {
                sub_state
            };

            Ok(DaemonStatusReport {
                socket_path: socket_path.display().to_string(),
                socket_ready: status_resp.socket_ready,
                camera_ready: status_resp.camera_ready,
                models_verified: status_resp.models_verified,
                is_healthy: status_resp.is_healthy,
                pid: Some(status_resp.pid),
                uptime_secs: Some(status_resp.uptime_secs),
                memory_locked: Some(status_resp.memory_locked),
                systemd_unit: unit_name.to_string(),
                systemd_active_state: active,
                systemd_sub_state: sub,
            })
        }
        Err(_) => Ok(DaemonStatusReport {
            socket_path: socket_path.display().to_string(),
            socket_ready: false,
            camera_ready: false,
            models_verified: false,
            is_healthy: false,
            pid: sys_pid,
            uptime_secs: None,
            memory_locked: None,
            systemd_unit: unit_name.to_string(),
            systemd_active_state: active_state,
            systemd_sub_state: sub_state,
        }),
    }
}

/// Contacts the daemon Unix Domain Socket to retrieve component health status.
fn query_daemon_socket(socket_path: &Path) -> Result<StatusResponse, AdminCliError> {
    let mut stream =
        UnixStream::connect(socket_path).map_err(|e| AdminCliError::SocketConnect {
            path: socket_path.display().to_string(),
            source: e,
        })?;

    let timeout = Duration::from_millis(500);
    stream
        .set_read_timeout(Some(timeout))
        .map_err(AdminCliError::SocketIo)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(AdminCliError::SocketIo)?;

    let mut request_id = [0u8; REQUEST_ID_LEN];
    getrandom::fill(&mut request_id)?;

    let req = Request {
        version: CURRENT_VERSION,
        kind: RequestKind::Status,
        request_id,
        uid_hint: 0,
        service: "soos-admin".to_string(),
        deadline_monotonic_ns: 0,
    };

    let encoded_req = encode_request(&req)?;
    stream
        .write_all(&encoded_req)
        .map_err(AdminCliError::SocketIo)?;
    stream.flush().map_err(AdminCliError::SocketIo)?;

    let mut len_bytes = [0u8; 4];
    stream
        .read_exact(&mut len_bytes)
        .map_err(AdminCliError::SocketIo)?;
    let declared_size = usize::try_from(u32::from_be_bytes(len_bytes))
        .map_err(|_| AdminCliError::UnexpectedResponse("invalid length prefix".to_string()))?;

    if declared_size > MAX_MESSAGE_SIZE || declared_size == 0 {
        return Err(AdminCliError::UnexpectedResponse(format!(
            "invalid declared response size {declared_size}"
        )));
    }

    let mut body = vec![0u8; declared_size];
    stream
        .read_exact(&mut body)
        .map_err(AdminCliError::SocketIo)?;

    let total_capacity = declared_size.saturating_add(4);
    let mut full = Vec::with_capacity(total_capacity);
    full.extend_from_slice(&len_bytes);
    full.extend_from_slice(&body);

    let resp: StatusResponse = decode(&full)?;
    Ok(resp)
}

/// Inspects systemd unit state via `systemctl show` with safe fallback.
fn inspect_systemd_unit(unit_name: &str) -> (String, String, Option<u32>) {
    inspect_systemd_unit_with(
        OsStr::new("systemctl"),
        unit_name,
        Duration::from_millis(SYSTEMCTL_SHOW_TIMEOUT_MS),
    )
}

/// Kills and reaps `child` (never leaves a zombie behind).
fn kill_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Test seam of the bounded `systemctl show` call (GitHub #331, P-2).
///
/// Runs `<program> show <unit> --property=ActiveState,SubState,MainPID` with stdin and
/// stderr null and stdout piped. Stdout is drained on a helper thread (at most
/// `MAX_SYSTEMCTL_OUTPUT_BYTES + 1` bytes) so a child filling the pipe never blocks on it;
/// the child is polled with `try_wait` every `SYSTEMCTL_POLL_INTERVAL_MS` until `timeout`,
/// then killed and reaped. A reader still running at the deadline (a descendant keeps the
/// pipe open) is detached. On a timeout, a non-zero exit, a spawn or wait error, or output
/// above `MAX_SYSTEMCTL_OUTPUT_BYTES`, the result is `("unknown", "unknown", None)`.
pub(crate) fn inspect_systemd_unit_with(
    program: &OsStr,
    unit_name: &str,
    timeout: Duration,
) -> (String, String, Option<u32>) {
    let unknown = || ("unknown".to_string(), "unknown".to_string(), None);
    // An unrepresentable deadline is treated as already expired (fail closed to unknown).
    let deadline = Instant::now().checked_add(timeout);
    let remaining = || {
        deadline.map_or(Duration::ZERO, |d| {
            d.saturating_duration_since(Instant::now())
        })
    };

    let mut child = match Command::new(program)
        .arg("show")
        .arg(unit_name)
        .arg("--property=ActiveState,SubState,MainPID")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return unknown(),
    };
    let Some(stdout) = child.stdout.take() else {
        kill_and_reap(&mut child);
        return unknown();
    };
    let (sender, receiver) = mpsc::channel();
    let reader = thread::Builder::new()
        .name("soos-admin-systemctl".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .take(MAX_SYSTEMCTL_OUTPUT_BYTES.saturating_add(1))
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            // The receiver may be gone (deadline passed): nothing to report then.
            let _ = sender.send(result);
        });
    if reader.is_err() {
        kill_and_reap(&mut child);
        return unknown();
    }

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => {
                kill_and_reap(&mut child);
                return unknown();
            }
        }
        let left = remaining();
        if left.is_zero() {
            kill_and_reap(&mut child);
            return unknown();
        }
        thread::sleep(left.min(Duration::from_millis(SYSTEMCTL_POLL_INTERVAL_MS)));
    };

    let output = match receiver.recv_timeout(remaining()) {
        Ok(Ok(bytes)) => bytes,
        _ => return unknown(),
    };
    let oversized =
        u64::try_from(output.len()).map_or(true, |len| len > MAX_SYSTEMCTL_OUTPUT_BYTES);
    if !status.success() || oversized {
        return unknown();
    }

    let text = String::from_utf8_lossy(&output);
    let mut active = "unknown".to_string();
    let mut sub = "unknown".to_string();
    let mut pid = None;

    for line in text.lines() {
        if let Some(val) = line.strip_prefix("ActiveState=") {
            active = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("SubState=") {
            sub = val.trim().to_string();
        } else if let Some(val) = line.strip_prefix("MainPID=") {
            if let Ok(parsed) = val.trim().parse::<u32>() {
                if parsed > 0 {
                    pid = Some(parsed);
                }
            }
        }
    }

    (active, sub, pid)
}

#[cfg(test)]
mod systemctl_bound_tests {
    //! GitHub #331 P-2 (matrix IGF11): `soos-admin status` bounds its `systemctl show` call.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "unit tests use direct assertions"
    )]

    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use std::time::Instant;

    const SHORT_TIMEOUT: Duration = Duration::from_millis(300);
    const SLACK: Duration = Duration::from_secs(1);

    /// Fake `systemctl` programs, written once before any of them is spawned (avoids
    /// `ETXTBSY` from a concurrent fork inheriting a write descriptor).
    fn fakes() -> &'static PathBuf {
        static DIR: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
        &DIR.get_or_init(|| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().to_path_buf();
            let scripts: &[(&str, &str)] = &[
                ("hang", "#!/bin/sh\nexec sleep 30\n"),
                (
                    "pipe-holder",
                    "#!/bin/sh\nsleep 30 &\necho ActiveState=active\n",
                ),
                (
                    "ok",
                    "#!/bin/sh\nprintf 'ActiveState=active\\nSubState=running\\nMainPID=42\\n'\n",
                ),
                (
                    "args",
                    "#!/bin/sh\n[ \"$1\" = show ] && [ \"$2\" = soos-daemon.service ] && \
                     [ \"$3\" = --property=ActiveState,SubState,MainPID ] && [ $# -eq 3 ] || exit 9\n\
                     printf 'ActiveState=activating\\nSubState=start\\nMainPID=7\\n'\n",
                ),
                (
                    "fail",
                    "#!/bin/sh\nprintf 'ActiveState=active\\nSubState=running\\nMainPID=42\\n'\nexit 3\n",
                ),
                (
                    "huge",
                    "#!/bin/sh\nprintf 'ActiveState=active\\nSubState=running\\nMainPID=42\\n'\n\
                     head -c 8192 /dev/zero | tr '\\000' 'x'\necho\n",
                ),
            ];
            for (name, body) in scripts {
                let file = path.join(name);
                std::fs::write(&file, body).unwrap();
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            (dir, path)
        })
        .1
    }

    fn run(name: &str, timeout: Duration) -> ((String, String, Option<u32>), Duration) {
        let program = fakes().join(name);
        let start = Instant::now();
        let out = inspect_systemd_unit_with(program.as_os_str(), "soos-daemon.service", timeout);
        (out, start.elapsed())
    }

    fn unknown() -> (String, String, Option<u32>) {
        ("unknown".to_string(), "unknown".to_string(), None)
    }

    #[test]
    fn test_igf11_systemctl_show_timeout_is_one_second() {
        assert_eq!(SYSTEMCTL_SHOW_TIMEOUT_MS, 1000);
    }

    #[test]
    fn test_igf11_hanging_systemctl_is_killed_and_reported_unknown() {
        let (out, elapsed) = run("hang", SHORT_TIMEOUT);
        assert_eq!(out, unknown());
        assert!(
            elapsed < SHORT_TIMEOUT + SLACK,
            "a hanging systemctl must be bounded by the timeout, took {elapsed:?}"
        );
    }

    #[test]
    fn test_igf11_descendant_holding_stdout_does_not_block() {
        let (out, elapsed) = run("pipe-holder", SHORT_TIMEOUT);
        assert!(
            elapsed < SHORT_TIMEOUT + SLACK,
            "a descendant keeping stdout open must not block the call, took {elapsed:?}"
        );
        assert!(
            out == unknown() || out.0 == "active",
            "either unknown or parsed, got {out:?}"
        );
    }

    #[test]
    fn test_igf11_normal_output_is_parsed() {
        let (out, _) = run("ok", Duration::from_millis(SYSTEMCTL_SHOW_TIMEOUT_MS));
        assert_eq!(out, ("active".to_string(), "running".to_string(), Some(42)));
    }

    #[test]
    fn test_igf11_arguments_are_unchanged() {
        let (out, _) = run("args", Duration::from_millis(SYSTEMCTL_SHOW_TIMEOUT_MS));
        assert_eq!(
            out,
            ("activating".to_string(), "start".to_string(), Some(7))
        );
    }

    #[test]
    fn test_igf11_non_zero_exit_is_unknown() {
        let (out, _) = run("fail", Duration::from_millis(SYSTEMCTL_SHOW_TIMEOUT_MS));
        assert_eq!(out, unknown());
    }

    #[test]
    fn test_igf11_oversized_output_is_unknown() {
        let (out, _) = run("huge", Duration::from_millis(SYSTEMCTL_SHOW_TIMEOUT_MS));
        assert_eq!(out, unknown());
    }

    #[test]
    fn test_igf11_missing_program_is_unknown() {
        let start = Instant::now();
        let out = inspect_systemd_unit_with(
            OsStr::new("/nonexistent/soos-test-systemctl"),
            "soos-daemon.service",
            SHORT_TIMEOUT,
        );
        assert_eq!(out, unknown());
        assert!(start.elapsed() < SHORT_TIMEOUT + SLACK);
    }
}
