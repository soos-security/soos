//! Journal fixtures of the failed-password alert contract tests (ADR 2026-10-06
//! "Failed-Password Alerts in `soos-remote` From the System Journal", architect spec
//! `AI/architect_spec_remote_auth_alerts.md` §11).
//!
//! - [`JLine`]: a `journalctl --output=json` line built field by field, in order, with
//!   duplicate keys allowed (the shapes of research §5: string or byte-array values).
//! - Line builders for the three signal grammars of spec §2.4 and the noise of research §2.3.
//! - [`ScriptedJournal`]: a [`JournalSource`] driven by the test (settable probe result,
//!   recorded `FollowStart`s, lines pushed by the test, `end()` ends the current follower).
//!
//! Nothing here reads the real journal, spawns a process or touches the network.
//!
//! Include with `#[path = "common/journal.rs"] mod journal;`.

#![allow(
    dead_code,
    unused_imports,
    reason = "Shared test fixtures library used conditionally across test modules"
)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::{mpsc, watch};
use zeroize::Zeroizing;

use soos_remote::journal::{
    BoxFuture, FollowStart, JournalError, JournalLines, JournalSource, LineRead,
};

/// The service owner of every fixture (spec §2.4 `OwnerLogin`).
pub const OWNER: &str = "sooshost";
/// The service owner's uid (equal to the harness `UID`).
pub const OWNER_UID: u32 = 1000;
/// Another local account's uid (its lines are never signals).
pub const OTHER_UID: u32 = 1001;
/// The owner's real locker on the research host (owner-writable, outside `/usr/bin`).
pub const OWNER_LOCKER: &str = "/home/sooshost/.local/bin/swaylock-plugin";
/// A developer test binary that writes real `pam_unix(swaylock:auth)` lines (research §2.3).
pub const TEST_BINARY: &str =
    "/home/sooshost/.config/driftwm/rust/target/release/deps/drift_verrou-0123abcd";
pub const SUDO_EXE: &str = "/usr/bin/sudo";
pub const SU_EXE: &str = "/usr/bin/su";
pub const GDM_WORKER_EXE: &str = "/usr/lib/gdm-session-worker";
pub const POLKIT_HELPER_EXE: &str = "/usr/lib/polkit-1/polkit-agent-helper-1";

/// One microsecond-precision second.
pub const SECOND_US: u64 = 1_000_000;
/// One millisecond in microseconds.
pub const MS_US: u64 = 1_000;

// ---------------------------------------------------------------------------------------
// JSON line builder
// ---------------------------------------------------------------------------------------

/// A `journalctl -o json` object, fields kept in insertion order (duplicates allowed).
#[derive(Clone)]
pub struct JLine {
    pub fields: Vec<(String, Value)>,
}

impl JLine {
    /// A syslog line on `authpriv` (`SYSLOG_FACILITY=10`, `_TRANSPORT=syslog`) with a cursor
    /// derived from the time.
    pub fn new(realtime_us: u64, uid: u32, identifier: Option<&str>, message: &str) -> Self {
        let mut fields = vec![
            (
                "__CURSOR".to_string(),
                Value::String(cursor_for(realtime_us)),
            ),
            (
                "__REALTIME_TIMESTAMP".to_string(),
                Value::String(realtime_us.to_string()),
            ),
            (
                "__MONOTONIC_TIMESTAMP".to_string(),
                Value::String("4242".into()),
            ),
            (
                "_BOOT_ID".to_string(),
                Value::String("0123456789abcdef0123456789abcdef".into()),
            ),
            ("MESSAGE".to_string(), Value::String(message.to_string())),
            ("SYSLOG_FACILITY".to_string(), Value::String("10".into())),
            ("_UID".to_string(), Value::String(uid.to_string())),
            ("_TRANSPORT".to_string(), Value::String("syslog".into())),
        ];
        if let Some(identifier) = identifier {
            fields.push((
                "SYSLOG_IDENTIFIER".to_string(),
                Value::String(identifier.to_string()),
            ));
        }
        Self { fields }
    }

    /// Sets `key` (replacing every existing occurrence, or appending).
    pub fn with(mut self, key: &str, value: Value) -> Self {
        let mut replaced = false;
        self.fields.retain_mut(|(k, v)| {
            if k == key {
                if replaced {
                    return false;
                }
                *v = value.clone();
                replaced = true;
            }
            true
        });
        if !replaced {
            self.fields.push((key.to_string(), value));
        }
        self
    }

    /// Sets `key` to a JSON string.
    pub fn with_str(self, key: &str, value: &str) -> Self {
        self.with(key, Value::String(value.to_string()))
    }

    /// Sets `key` to the byte-array rendering `journalctl` uses for non-printable or
    /// non-UTF-8 values.
    pub fn with_bytes(self, key: &str, bytes: &[u8]) -> Self {
        self.with(
            key,
            Value::Array(bytes.iter().map(|b| Value::from(u64::from(*b))).collect()),
        )
    }

    /// Removes `key`.
    pub fn without(mut self, key: &str) -> Self {
        self.fields.retain(|(k, _)| k != key);
        self
    }

    /// Appends a second occurrence of `key` (a duplicate key in the object).
    pub fn duplicated(mut self, key: &str, value: Value) -> Self {
        self.fields.push((key.to_string(), value));
        self
    }

    /// The serialized line (no trailing newline).
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = String::from("{");
        for (i, (k, v)) in self.fields.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&serde_json::to_string(k).unwrap());
            out.push(':');
            out.push_str(&serde_json::to_string(v).unwrap());
        }
        out.push('}');
        out.into_bytes()
    }
}

/// A valid cursor (`[A-Za-z0-9=;_-]`) unique per journal time.
pub fn cursor_for(realtime_us: u64) -> String {
    format!("s=5eed0f00d;i={realtime_us:x};b=0123abcd;m=42;t={realtime_us:x};x=77aa")
}

/// `unix_chkpwd[PID]: password check failed for user (<name>)` as logged by the helper:
/// identifier and `_COMM` `unix_chkpwd`, no `_EXE` (research §3: absent in 54/54 entries).
pub fn chkpwd_line(uid: u32, name: &str, realtime_us: u64) -> JLine {
    JLine::new(
        realtime_us,
        uid,
        Some("unix_chkpwd"),
        &format!("password check failed for user ({name})"),
    )
    .with_str("_COMM", "unix_chkpwd")
}

/// The `pam_unix` failure message of Linux-PAM 1.7.3 (`support.c:803-836`).
pub fn pam_unix_message(service: &str, uid: u32, name: &str) -> String {
    format!(
        "pam_unix({service}:auth): authentication failure; logname={OWNER} uid={uid} euid=0 \
         tty=/dev/pts/3 ruser={OWNER} rhost=  user={name}"
    )
}

/// `pam_unix(<service>:auth): authentication failure; … user=<name>`; `exe` is `_EXE`
/// (`None`: field absent), `_COMM` is the basename of `exe` (truncated to 15 bytes like the
/// kernel task name).
pub fn pam_unix_line(
    service: &str,
    uid: u32,
    exe: Option<&str>,
    name: &str,
    realtime_us: u64,
) -> JLine {
    with_process(
        JLine::new(
            realtime_us,
            uid,
            Some(identifier_for(exe)),
            &pam_unix_message(service, uid, name),
        ),
        exe,
    )
}

/// `pam_faillock(<service>:auth): User <name> is temporarily locked out due to <n>
/// consecutive failed login attempts`.
pub fn faillock_locked_line(
    service: &str,
    uid: u32,
    exe: Option<&str>,
    name: &str,
    realtime_us: u64,
) -> JLine {
    with_process(
        JLine::new(
            realtime_us,
            uid,
            Some(identifier_for(exe)),
            &format!(
                "pam_faillock({service}:auth): User {name} is temporarily locked out due to 3 \
                 consecutive failed login attempts"
            ),
        ),
        exe,
    )
}

/// Any message logged by the process `exe` (`None`: no `_EXE`, no `_COMM`).
pub fn process_line(uid: u32, exe: Option<&str>, message: &str, realtime_us: u64) -> JLine {
    with_process(
        JLine::new(realtime_us, uid, Some(identifier_for(exe)), message),
        exe,
    )
}

fn identifier_for(exe: Option<&str>) -> &str {
    exe.and_then(|e| e.rsplit('/').next())
        .map(|base| base.trim_end_matches(" (deleted)"))
        .unwrap_or("pam")
}

fn with_process(line: JLine, exe: Option<&str>) -> JLine {
    match exe {
        Some(exe) => {
            let base = exe.rsplit('/').next().unwrap_or(exe);
            let comm: String = base.chars().take(15).collect();
            line.with_str("_EXE", exe).with_str("_COMM", &comm)
        }
        None => line,
    }
}

// ---------------------------------------------------------------------------------------
// Scripted journal source
// ---------------------------------------------------------------------------------------

/// What a scripted follower hands out.
pub enum Item {
    Line(Vec<u8>),
    Overlong,
    End,
}

pub struct JournalInner {
    /// Results handed out before `probe_default` (front first).
    pub probe_queue: Mutex<VecDeque<Result<(), JournalError>>>,
    pub probe_default: Mutex<Result<(), JournalError>>,
    /// Results of `follow` handed out before success (front first).
    pub follow_queue: Mutex<VecDeque<JournalError>>,
    pub probes: AtomicUsize,
    pub starts: Mutex<Vec<FollowStart>>,
    pub current: Mutex<Option<mpsc::UnboundedSender<Item>>>,
    /// Bumped on every probe and follow (observers wait on it).
    pub calls: watch::Sender<u64>,
}

/// A [`JournalSource`] driven by the test.
#[derive(Clone)]
pub struct ScriptedJournal(pub Arc<JournalInner>);

impl Default for ScriptedJournal {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptedJournal {
    /// Probe succeeds, `follow` succeeds, no line yet.
    pub fn new() -> Self {
        Self(Arc::new(JournalInner {
            probe_queue: Mutex::new(VecDeque::new()),
            probe_default: Mutex::new(Ok(())),
            follow_queue: Mutex::new(VecDeque::new()),
            probes: AtomicUsize::new(0),
            starts: Mutex::new(Vec::new()),
            current: Mutex::new(None),
            calls: watch::channel(0).0,
        }))
    }

    /// Every later probe answers `result` (after the queued ones).
    pub fn set_probe(&self, result: Result<(), JournalError>) {
        *self.0.probe_default.lock().unwrap() = result;
    }

    /// The next probes answer `results` in order, then the default.
    pub fn queue_probes(&self, results: &[Result<(), JournalError>]) {
        self.0
            .probe_queue
            .lock()
            .unwrap()
            .extend(results.iter().copied());
    }

    pub fn probes(&self) -> usize {
        self.0.probes.load(Ordering::SeqCst)
    }

    /// Every `FollowStart` received so far, in order.
    pub fn starts(&self) -> Vec<FollowStart> {
        self.0.starts.lock().unwrap().clone()
    }

    pub fn follows(&self) -> usize {
        self.0.starts.lock().unwrap().len()
    }

    /// Hands `bytes` to the running follower; `false` when no follower is running.
    pub fn push_bytes(&self, bytes: Vec<u8>) -> bool {
        match self.0.current.lock().unwrap().as_ref() {
            Some(tx) => tx.send(Item::Line(bytes)).is_ok(),
            None => false,
        }
    }

    /// Hands one line to the running follower (panics without one).
    pub fn push(&self, line: &JLine) {
        assert!(
            self.push_bytes(line.bytes()),
            "no running follower to hand the line to"
        );
    }

    /// Hands `LineRead::Overlong` to the running follower.
    pub fn push_overlong(&self) {
        let sent = match self.0.current.lock().unwrap().as_ref() {
            Some(tx) => tx.send(Item::Overlong).is_ok(),
            None => false,
        };
        assert!(sent, "no running follower");
    }

    /// Ends the running follower (`LineRead::End`).
    pub fn end(&self) {
        if let Some(tx) = self.0.current.lock().unwrap().take() {
            let _ = tx.send(Item::End);
        }
    }

    /// Whether a follower is currently attached.
    pub fn has_follower(&self) -> bool {
        self.0
            .current
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|tx| !tx.is_closed())
    }
}

/// Human-readable rendering of a `FollowStart` (it has no `Debug`).
pub fn describe_start(start: &FollowStart) -> String {
    match start {
        FollowStart::Since { unix_s } => format!("Since({unix_s})"),
        FollowStart::AfterCursor(cursor) => format!("AfterCursor({})", cursor.as_str()),
    }
}

struct ScriptedLines {
    rx: mpsc::UnboundedReceiver<Item>,
}

impl JournalLines for ScriptedLines {
    fn next_line(&mut self) -> BoxFuture<'_, LineRead> {
        Box::pin(async move {
            match self.rx.recv().await {
                Some(Item::Line(bytes)) => LineRead::Line(Zeroizing::new(bytes)),
                Some(Item::Overlong) => LineRead::Overlong,
                Some(Item::End) | None => LineRead::End,
            }
        })
    }
}

impl JournalSource for ScriptedJournal {
    fn probe(&self) -> BoxFuture<'_, Result<(), JournalError>> {
        Box::pin(async move {
            self.0.probes.fetch_add(1, Ordering::SeqCst);
            let queued = self.0.probe_queue.lock().unwrap().pop_front();
            let result = queued.unwrap_or_else(|| *self.0.probe_default.lock().unwrap());
            self.0.calls.send_modify(|c| *c += 1);
            result
        })
    }

    fn follow(
        &self,
        start: FollowStart,
    ) -> BoxFuture<'_, Result<Box<dyn JournalLines>, JournalError>> {
        Box::pin(async move {
            self.0.starts.lock().unwrap().push(start);
            self.0.calls.send_modify(|c| *c += 1);
            if let Some(err) = self.0.follow_queue.lock().unwrap().pop_front() {
                return Err(err);
            }
            let (tx, rx) = mpsc::unbounded_channel();
            *self.0.current.lock().unwrap() = Some(tx);
            Ok(Box::new(ScriptedLines { rx }) as Box<dyn JournalLines>)
        })
    }
}
