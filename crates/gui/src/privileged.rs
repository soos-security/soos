//! Privileged (Polkit `pkexec`) operations executed off the UI thread (review finding CAM-06).
//!
//! `pkexec` blocks until the user answers the Polkit dialog. Running it from a click handler
//! froze the whole window, so every privileged command now goes through [`TaskRunner`]: the UI
//! submits a [`PrivilegedAction`], a worker thread runs it through a [`PrivilegedExecutor`], and
//! the [`PrivilegedOutcome`] comes back over a channel drained by [`TaskRunner::poll`] each frame.
//! At most one privileged action runs at a time so the user never faces stacked dialogs.

#![forbid(unsafe_code)]

use std::fmt;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use soos_enrollment_cli::service::EnrolledUserSummary;
use zeroize::Zeroizing;

/// Upper bound on the `soos-enroll list --format json` output accepted from the helper.
pub const MAX_PROFILE_LIST_BYTES: usize = 1024 * 1024;

/// A privileged operation requested by the UI.
pub enum PrivilegedAction {
    /// `pkexec systemctl stop soos-daemon.service`.
    PauseDaemon,
    /// `pkexec systemctl start soos-daemon.service`.
    ResumeDaemon,
    /// `pkexec soos-enroll list --format json`.
    ListProfiles,
    /// Imports a fused embedding into the root-owned system store via `soos-enroll import`.
    ImportTemplate {
        /// Target user ID.
        uid: u32,
        /// Fused embedding (zeroized on drop, never printed).
        embedding: Zeroizing<Vec<f32>>,
    },
    /// `pkexec soos-enroll delete --uid <uid> --yes`.
    DeleteTemplate {
        /// Target user ID.
        uid: u32,
    },
}

impl fmt::Debug for PrivilegedAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PauseDaemon => f.write_str("PauseDaemon"),
            Self::ResumeDaemon => f.write_str("ResumeDaemon"),
            Self::ListProfiles => f.write_str("ListProfiles"),
            Self::ImportTemplate { uid, embedding } => f
                .debug_struct("ImportTemplate")
                .field("uid", uid)
                .field("embedding_dim", &embedding.len())
                .finish_non_exhaustive(),
            Self::DeleteTemplate { uid } => {
                f.debug_struct("DeleteTemplate").field("uid", uid).finish()
            }
        }
    }
}

/// Result of a [`PrivilegedAction`], delivered back to the UI thread.
#[derive(Debug)]
pub enum PrivilegedOutcome {
    /// Result of [`PrivilegedAction::PauseDaemon`].
    DaemonPaused(Result<(), String>),
    /// Result of [`PrivilegedAction::ResumeDaemon`].
    DaemonResumed(Result<(), String>),
    /// Result of [`PrivilegedAction::ListProfiles`].
    ProfilesListed(Result<Vec<EnrolledUserSummary>, String>),
    /// Result of [`PrivilegedAction::ImportTemplate`].
    TemplateImported {
        /// Target user ID.
        uid: u32,
        /// Import result.
        result: Result<(), String>,
    },
    /// Result of [`PrivilegedAction::DeleteTemplate`].
    TemplateDeleted {
        /// Target user ID.
        uid: u32,
        /// Deletion result.
        result: Result<(), String>,
    },
}

/// Executes privileged actions. May block for as long as the Polkit dialog is open.
pub trait PrivilegedExecutor: Send + Sync {
    /// Runs `action` to completion and reports its outcome.
    fn execute(&self, action: PrivilegedAction) -> PrivilegedOutcome;
}

/// Production executor invoking `pkexec` with fixed argument vectors (no shell).
#[derive(Debug, Default, Clone, Copy)]
pub struct PkexecExecutor;

fn pkexec_status(args: &[&str], what: &str) -> Result<(), String> {
    let status = Command::new("pkexec")
        .args(args)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to invoke pkexec: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to {what} (authorization denied or error, exit code {:?})",
            status.code()
        ))
    }
}

/// Error returned by [`read_bounded`].
#[derive(Debug, thiserror::Error)]
pub enum BoundedReadError {
    /// The source produced more than the allowed number of bytes.
    #[error("output exceeds {limit} bytes")]
    Oversized {
        /// Maximum accepted size in bytes.
        limit: usize,
    },
    /// Reading the source failed.
    #[error("failed to read output: {0}")]
    Io(#[from] std::io::Error),
}

/// Reads `reader` to its end, consuming at most `limit + 1` bytes.
///
/// The bound is enforced while reading (the source is wrapped in [`Read::take`]), so an
/// oversized or never-ending output cannot grow the buffer beyond `limit + 1` bytes.
///
/// # Errors
///
/// [`BoundedReadError::Oversized`] when more than `limit` bytes are available;
/// [`BoundedReadError::Io`] when reading fails.
pub fn read_bounded<R: Read>(reader: R, limit: usize) -> Result<Vec<u8>, BoundedReadError> {
    let mut buf = Vec::new();
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    reader.take(cap).read_to_end(&mut buf)?;
    if buf.len() > limit {
        return Err(BoundedReadError::Oversized { limit });
    }
    Ok(buf)
}

fn list_profiles() -> Result<Vec<EnrolledUserSummary>, String> {
    let mut child = Command::new("pkexec")
        .args(["soos-enroll", "list", "--format", "json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to invoke pkexec: {e}"))?;
    // The bound is enforced while reading (candid review finding 7): the helper's stdout is
    // never buffered beyond `MAX_PROFILE_LIST_BYTES + 1` bytes.
    let read = match child.stdout.take() {
        Some(stdout) => read_bounded(stdout, MAX_PROFILE_LIST_BYTES),
        None => Err(BoundedReadError::Io(std::io::Error::other(
            "helper stdout was not captured",
        ))),
    };
    if read.is_err() {
        // Do not wait for a helper that may still be writing into a pipe nobody reads.
        let _ = child.kill();
    }
    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for pkexec: {e}"))?;
    let stdout = match read {
        Ok(bytes) => bytes,
        Err(BoundedReadError::Oversized { .. }) => {
            return Err("soos-enroll returned an oversized profile list".to_string());
        }
        Err(BoundedReadError::Io(e)) => {
            return Err(format!("Failed to read the soos-enroll profile list: {e}"));
        }
    };
    if !status.success() {
        return Err("Loading profiles via Polkit was denied or failed".to_string());
    }
    serde_json::from_slice(&stdout)
        .map_err(|_| "soos-enroll returned a malformed profile list".to_string())
}

/// Argument vector of the import helper: `soos-enroll import --uid <uid> --file -`.
///
/// `--file -` makes `soos-enroll` read the embedding from its standard input, so the
/// plaintext template never touches the filesystem (GitHub #156).
pub fn import_helper_args(uid: u32) -> Vec<String> {
    vec![
        "soos-enroll".to_string(),
        "import".to_string(),
        "--uid".to_string(),
        uid.to_string(),
        "--file".to_string(),
        "-".to_string(),
    ]
}

/// Serializes `embedding` as a JSON float array into a zeroizing buffer reserved once at
/// the exact encoded size (measured by a counting pass that stores nothing), so the buffer
/// never reallocates and no freed intermediate copy of the embedding is left unzeroized.
///
/// # Errors
///
/// A user-facing message when serialization or the allocation fails (never the values).
pub fn embedding_json(embedding: &[f32]) -> Result<Zeroizing<Vec<u8>>, String> {
    let mut counter = ByteCounter(0);
    serde_json::to_writer(&mut counter, embedding)
        .map_err(|e| format!("Failed to serialize embedding: {e}"))?;
    let mut json = Zeroizing::new(Vec::new());
    json.try_reserve_exact(counter.0)
        .map_err(|e| format!("Failed to allocate the embedding buffer: {e}"))?;
    serde_json::to_writer(&mut *json, embedding)
        .map_err(|e| format!("Failed to serialize embedding: {e}"))?;
    Ok(json)
}

/// `Write` sink that only counts the bytes written (nothing is stored).
struct ByteCounter(usize);

impl Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(buf.len())
            .ok_or_else(|| std::io::Error::other("encoded size overflow"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Runs `command` (production: `pkexec`) with [`import_helper_args`] and pipes the embedding
/// as JSON to its standard input.
///
/// The serialized JSON lives only in a zeroizing buffer and in the pipe; no file is created.
/// The child is always reaped, even when writing to its stdin fails (it may exit early, e.g.
/// on a denied Polkit prompt).
///
/// # Errors
///
/// A user-facing message when the helper cannot be spawned, the embedding cannot be
/// delivered, or the helper exits unsuccessfully. Embedding values are never included.
pub fn import_template_with(
    mut command: Command,
    uid: u32,
    embedding: &[f32],
) -> Result<(), String> {
    let json = embedding_json(embedding)?;
    let mut child = command
        .args(import_helper_args(uid))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to invoke pkexec: {e}"))?;
    let delivered = match child.stdin.take() {
        // Dropping `stdin` at the end of this arm closes the pipe (EOF for the helper).
        Some(mut stdin) => stdin.write_all(&json).and_then(|()| stdin.flush()),
        None => Err(std::io::Error::other("helper stdin was not captured")),
    };
    drop(json);
    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for pkexec: {e}"))?;
    if !status.success() {
        return Err(format!(
            "Failed to import the template into the system store (authorization denied or \
             error, exit code {:?})",
            status.code()
        ));
    }
    delivered.map_err(|e| format!("Failed to pass the embedding to soos-enroll: {e}"))
}

fn import_template(uid: u32, embedding: &[f32]) -> Result<(), String> {
    import_template_with(Command::new("pkexec"), uid, embedding)
}

impl PrivilegedExecutor for PkexecExecutor {
    fn execute(&self, action: PrivilegedAction) -> PrivilegedOutcome {
        match action {
            PrivilegedAction::PauseDaemon => PrivilegedOutcome::DaemonPaused(pkexec_status(
                &["systemctl", "stop", "soos-daemon.service"],
                "pause soos-daemon",
            )),
            PrivilegedAction::ResumeDaemon => PrivilegedOutcome::DaemonResumed(pkexec_status(
                &["systemctl", "start", "soos-daemon.service"],
                "resume soos-daemon",
            )),
            PrivilegedAction::ListProfiles => PrivilegedOutcome::ProfilesListed(list_profiles()),
            PrivilegedAction::ImportTemplate { uid, embedding } => {
                PrivilegedOutcome::TemplateImported {
                    uid,
                    result: import_template(uid, &embedding),
                }
            }
            PrivilegedAction::DeleteTemplate { uid } => {
                let uid_arg = uid.to_string();
                PrivilegedOutcome::TemplateDeleted {
                    uid,
                    result: pkexec_status(
                        &["soos-enroll", "delete", "--uid", &uid_arg, "--yes"],
                        "delete the template from the system store",
                    ),
                }
            }
        }
    }
}

/// Error returned by [`TaskRunner::submit`].
#[derive(Debug, thiserror::Error)]
pub enum TaskRunnerError {
    /// Another privileged action is still running.
    #[error("another privileged operation is already in progress")]
    Busy,
    /// The worker thread could not be spawned.
    #[error("failed to start the privileged operation worker: {0}")]
    Spawn(#[from] std::io::Error),
}

/// Callback waking the UI when an outcome is ready (e.g. `egui::Context::request_repaint`).
pub type Notify = Arc<dyn Fn() + Send + Sync>;

/// Runs one privileged action at a time on a background thread.
pub struct TaskRunner {
    executor: Arc<dyn PrivilegedExecutor>,
    notify: Notify,
    tx: Sender<PrivilegedOutcome>,
    rx: Receiver<PrivilegedOutcome>,
    in_flight: Option<JoinHandle<()>>,
}

impl TaskRunner {
    /// Creates a runner using `executor`, calling `notify` whenever an outcome is delivered.
    pub fn new(executor: Arc<dyn PrivilegedExecutor>, notify: Notify) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            executor,
            notify,
            tx,
            rx,
            in_flight: None,
        }
    }

    /// Starts `action` on a worker thread and returns immediately.
    ///
    /// # Errors
    ///
    /// [`TaskRunnerError::Busy`] while another action runs; [`TaskRunnerError::Spawn`] if the
    /// worker thread cannot be created.
    pub fn submit(&mut self, action: PrivilegedAction) -> Result<(), TaskRunnerError> {
        if self.in_flight.is_some() {
            return Err(TaskRunnerError::Busy);
        }
        let executor = Arc::clone(&self.executor);
        let notify = Arc::clone(&self.notify);
        let tx = self.tx.clone();
        let handle = thread::Builder::new()
            .name("soos-gui-privileged".to_string())
            .spawn(move || {
                let outcome = executor.execute(action);
                let _ = tx.send(outcome);
                notify();
            })?;
        self.in_flight = Some(handle);
        Ok(())
    }

    /// Returns whether a privileged action is still running.
    pub fn is_busy(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Drains delivered outcomes without blocking. Call once per frame from the UI thread.
    pub fn poll(&mut self) -> Vec<PrivilegedOutcome> {
        let finished = self.in_flight.as_ref().is_some_and(JoinHandle::is_finished);
        let outcomes: Vec<PrivilegedOutcome> = self.rx.try_iter().collect();
        if !outcomes.is_empty() || finished {
            if finished && outcomes.is_empty() {
                tracing::error!("privileged operation worker terminated without an outcome");
            }
            self.in_flight = None;
        }
        outcomes
    }
}
