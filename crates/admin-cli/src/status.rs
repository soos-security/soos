//! Daemon health and status query subsystem.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use soos_protocol::codec::decode;
use soos_protocol::message::encode_request;
use soos_protocol::types::{
    Request, RequestKind, StatusResponse, CURRENT_VERSION, MAX_MESSAGE_SIZE, REQUEST_ID_LEN,
};

use crate::error::AdminCliError;

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
    let output = match Command::new("systemctl")
        .arg("show")
        .arg(unit_name)
        .arg("--property=ActiveState,SubState,MainPID")
        .output()
    {
        Ok(out) if out.status.success() => out.stdout,
        _ => return ("unknown".to_string(), "unknown".to_string(), None),
    };

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
