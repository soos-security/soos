//! Management functions for GDM login PAM integration and disable flag toggling.
//!
//! `gdm enable` places `pam_soos.so` after every pre-credential gate of the
//! `gdm-password` auth stack (`pam_nologin`, `pam_succeed_if`, `pam_shells`,
//! `pam_faillock preauth`), copying the gates of a delegated stack in front of it,
//! so that a facial success never bypasses lockout or login restrictions
//! (review finding STO-03, GitHub #177; ADR 2026-09-30 "GDM PAM Stack Placement").

use serde::Serialize;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::args::GdmAction;
use crate::error::AdminCliError;
use crate::pam_stack::{delegated_auth, read_bounded_utf8, DelegatedAuth, PamLine};

pub use crate::pam_stack::{MAX_PAM_FILE_BYTES, MAX_PAM_INCLUDE_DEPTH, MAX_PAM_STACK_READS};

/// PAM rule inserted into the GDM service file by `gdm enable`. The explicit
/// `[success=done default=ignore]` control is the one used by every packaged soos
/// rule: a face match ends the auth phase with the accumulated result (so a failed
/// `required` gate before it still fails the login), anything else is ignored. No
/// `service=` argument: the module reads `PAM_SERVICE`, which drives `gdm.disable`.
pub const GDM_PAM_LINE: &str = "auth  [success=done default=ignore]  pam_soos.so timeout_ms=2500";

/// Rule written by releases before GitHub #177 (often at the top of the stack).
/// `gdm enable` removes it and re-inserts [`GDM_PAM_LINE`] at the safe position.
pub const LEGACY_GDM_PAM_LINE: &str = "auth  sufficient  pam_soos.so timeout_ms=2500";

/// First line of the block managed by `gdm enable`.
pub const GDM_BLOCK_BEGIN: &str = "# BEGIN soos-admin gdm enable (managed block, do not edit)";

/// Last line of the block managed by `gdm enable`.
pub const GDM_BLOCK_END: &str = "# END soos-admin gdm enable";

/// File name of the PAM module that must be installed before `gdm enable`.
pub const PAM_MODULE_FILE: &str = "pam_soos.so";

/// PAM module directories probed by `soos-admin gdm enable` (same list as `scripts/install.sh`).
pub const DEFAULT_PAM_MODULE_DIRS: &[&str] = &[
    "/lib/x86_64-linux-gnu/security",
    "/usr/lib/x86_64-linux-gnu/security",
    "/lib/aarch64-linux-gnu/security",
    "/usr/lib/aarch64-linux-gnu/security",
    "/usr/lib64/security",
    "/lib64/security",
    "/usr/lib/security",
    "/lib/security",
];

/// Suffix of the pristine copy kept next to an edited PAM file. `scripts/uninstall.sh`
/// restores every `*.soos-backup` found in `/etc/pam.d` (GitHub #166).
pub const PAM_BACKUP_SUFFIX: &str = ".soos-backup";

/// Returns the first `<dir>/pam_soos.so` that is a regular file (symlinks resolved).
#[must_use]
pub fn find_pam_module(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(PAM_MODULE_FILE))
        .find(|path| fs::metadata(path).is_ok_and(|m| m.is_file()))
}

/// Status summary of GDM PAM integration and disable flag.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GdmStatus {
    /// True if `pam_soos.so` is configured in the PAM service file.
    pub installed: bool,
    /// True if GDM biometric authentication is active (not disabled).
    pub enabled: bool,
    /// Path to the inspected PAM service file.
    pub pam_file: PathBuf,
    /// Path to the inspected disable flag file.
    pub disable_file: PathBuf,
    /// GitHub #331: name of the delegated stack file whose primary `pam_soos.so` rule GDM
    /// uses when the service file carries no soos rule of its own; `None` otherwise.
    pub shared_stack: Option<String>,
}

/// True when `content` holds an active (not commented-out) PAM rule whose module
/// field is `pam_soos.so` (bare name or absolute path). Comments, blank lines and
/// mentions in module arguments do not count (GitHub #236 / STO-20).
#[must_use]
pub fn has_active_pam_soos_rule(content: &str) -> bool {
    content
        .lines()
        .filter_map(PamLine::parse)
        .any(|line| line.module_name() == Some(PAM_MODULE_FILE))
}

/// Inspects current GDM integration and disable state.
///
/// `installed` requires an active `pam_soos.so` rule in the file
/// ([`has_active_pam_soos_rule`]) or, failing that, a shared primary `pam_soos.so` rule
/// reached through the delegated auth stack, found with exactly the analysis of
/// `gdm enable` (GitHub #331; reported in `shared_stack`). The file is read with the same
/// bound as `gdm enable` ([`MAX_PAM_FILE_BYTES`]), and an unreadable, oversized or
/// non-UTF-8 file, or any refusal of the shared-stack analysis, reports `installed: false`.
pub fn get_gdm_status(pam_file: &Path, disable_file: &Path) -> GdmStatus {
    let content = if pam_file.is_file() {
        read_bounded_utf8(pam_file).ok()
    } else {
        None
    };
    let (installed, shared_stack) = match content {
        Some(content) if has_active_pam_soos_rule(&content) => (true, None),
        Some(content) => {
            let shared = shared_soos_stack(&content, include_dir_of(pam_file));
            (shared.is_some(), shared)
        }
        None => (false, None),
    };

    let disabled = disable_file.exists() || Path::new("/etc/soos/disabled").exists();
    let enabled = installed && !disabled;

    GdmStatus {
        installed,
        enabled,
        pam_file: pam_file.to_path_buf(),
        disable_file: disable_file.to_path_buf(),
        shared_stack,
    }
}

/// Directory in which the include targets of `pam_file` are resolved.
fn include_dir_of(pam_file: &Path) -> &Path {
    pam_file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// Name of the stack holding the shared primary soos rule reached by `content`'s auth stack,
/// using exactly the analysis of `plan_gdm_enable` ([`pre_credential_scan`] and
/// `delegated_auth`). `None` on any refusal condition (continuation lines, unclassified rule,
/// no anchor, non-delegating anchor, unreadable or missing include, depth exceeded): fail
/// closed, never an error to the caller.
fn shared_soos_stack(content: &str, include_dir: &Path) -> Option<String> {
    refuse_continuations(content).ok()?;
    let pristine = strip_managed_rules(content).ok()?;
    let scan = pre_credential_scan(&pristine).ok()?;
    scan.anchor_rule.delegation()?;
    // A pre-anchor jump landing beyond the delegation bypasses the shared rule on that branch:
    // not reported as installed (fail closed; candid review 2026-10-05 suggestion).
    if scan.jump_skips_anchor() {
        return None;
    }
    match delegated_auth(include_dir, scan.delegated_lines()).ok()? {
        DelegatedAuth::SharedSoosRule { stack } => Some(stack),
        DelegatedAuth::Gates(_) => None,
    }
}

/// Options of [`configure_gdm_with_options`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GdmOptions {
    /// `restore` only: restore a backup that no longer matches the PAM file without its
    /// soos rules (GitHub #312, STO-NEW-7); the later changes are lost.
    pub force: bool,
}

/// Executes GDM configuration action (Status, Enable, Disable, Restore) with the default
/// [`GdmOptions`] (a stale backup is never restored).
///
/// `Enable` does not check that the module is installed: the `soos-admin` binary
/// calls [`find_pam_module`] first (tests run against temporary directories).
pub fn configure_gdm(
    action: &GdmAction,
    pam_file: &Path,
    disable_file: &Path,
) -> Result<GdmStatus, AdminCliError> {
    configure_gdm_with_options(action, pam_file, disable_file, GdmOptions::default())
}

/// [`configure_gdm`] with explicit [`GdmOptions`].
pub fn configure_gdm_with_options(
    action: &GdmAction,
    pam_file: &Path,
    disable_file: &Path,
    options: GdmOptions,
) -> Result<GdmStatus, AdminCliError> {
    match action {
        GdmAction::Status => Ok(get_gdm_status(pam_file, disable_file)),
        GdmAction::Disable => {
            if let Some(parent) = disable_file.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent).map_err(|e| {
                        AdminCliError::GdmConfig(format!(
                            "Failed to create directory '{}': {e}",
                            parent.display()
                        ))
                    })?;
                }
            }
            fs::write(disable_file, "disabled\n").map_err(|e| {
                AdminCliError::GdmConfig(format!(
                    "Failed to write disable flag '{}': {e}",
                    disable_file.display()
                ))
            })?;
            Ok(get_gdm_status(pam_file, disable_file))
        }
        GdmAction::Restore => {
            restore_gdm_pam_file(pam_file, options.force)?;
            Ok(get_gdm_status(pam_file, disable_file))
        }
        GdmAction::Enable => {
            // Place pam_soos.so first (backup + atomic replacement): if the stack is
            // refused, the disable flag stays in place.
            ensure_gdm_pam_line(pam_file)?;

            // Remove disable flag if present
            if disable_file.exists() {
                fs::remove_file(disable_file).map_err(|e| {
                    AdminCliError::GdmConfig(format!(
                        "Failed to remove disable flag '{}': {e}",
                        disable_file.display()
                    ))
                })?;
            }

            Ok(get_gdm_status(pam_file, disable_file))
        }
    }
}

/// Returns the path of the pristine backup kept next to `pam_file`.
#[must_use]
pub fn pam_backup_path(pam_file: &Path) -> PathBuf {
    let mut name = pam_file
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(PAM_BACKUP_SUFFIX);
    pam_file.with_file_name(name)
}

fn gdm_error(what: &str, path: &Path, err: &std::io::Error) -> AdminCliError {
    AdminCliError::GdmConfig(format!("{what} '{}': {err}", path.display()))
}

/// Inserts the managed block (gates + [`GDM_PAM_LINE`]) into `pam_file`.
///
/// The pristine content (without any soos rule) is first saved to
/// [`pam_backup_path`] (never overwritten when a backup already exists: it holds the
/// pristine pre-soos state), then the new content is written to a temporary file in
/// the same directory, fsynced and renamed over the original, so a crash never leaves
/// a truncated PAM file. Symlinks, non-regular, oversized and non-UTF-8 files, stacks
/// without a credential anchor, stacks with an unclassified auth rule before that
/// anchor (in the file or in a delegated stack) and stacks whose jumps would change
/// are refused
/// without any modification. A `pam_soos.so` rule written by the administrator is
/// left untouched.
fn ensure_gdm_pam_line(pam_file: &Path) -> Result<(), AdminCliError> {
    ensure_gdm_pam_line_with(pam_file, &mut || {})
}

/// GitHub #333: `ensure_gdm_pam_line` with a hook run once, right before the rename of the PAM
/// file (after the temporary file is written and synced). Production passes a no-op.
///
/// The PAM file is opened once (`O_NOFOLLOW | O_NOCTTY | O_NONBLOCK | O_CLOEXEC`): its type,
/// size, bytes, mode and owner all come from that descriptor ([`PamFileSnapshot`]) and the
/// planned content is computed from exactly those bytes. Right before the rename the file is
/// read again and must still be the same inode with the same bytes, mode and owner; otherwise
/// nothing is written and the enable fails with "changed concurrently" (as `gdm restore` since
/// GitHub #318). A change made in the few system calls between that re-check and `rename(2)`
/// is an inherent residual. A backup created by an aborted run is removed again, unless the
/// file now holds a soos managed block (a concurrent `gdm enable` won and relies on it).
fn ensure_gdm_pam_line_with(
    pam_file: &Path,
    before_rename: &mut dyn FnMut(),
) -> Result<(), AdminCliError> {
    ensure_gdm_pam_line_with_hooks(pam_file, before_rename, &mut backup_post_link_steps)
}

/// Production post-link steps of [`publish_backup`]: remove this run's temporary file (a
/// missing name is fine) and fsync the directory (GitHub #333, #335).
fn backup_post_link_steps(tmp: &Path, dir: &Path) -> std::io::Result<()> {
    match fs::remove_file(tmp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    fs::File::open(dir)?.sync_all()
}

/// GitHub #335: [`ensure_gdm_pam_line_with`] with a second hook that replaces the post-link
/// steps of the backup publication. The hook receives `(own_tmp_path, dir)` right after the
/// backup hard link succeeded; production passes [`backup_post_link_steps`] (remove this run's
/// temporary file, fsync the directory). An error from it fails the enable before the PAM
/// file is written, and the backup this run created goes through [`discard_created_backup`].
fn ensure_gdm_pam_line_with_hooks(
    pam_file: &Path,
    before_rename: &mut dyn FnMut(),
    post_link: &mut dyn FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), AdminCliError> {
    let Some(snapshot) = read_pam_file_snapshot(pam_file, GdmOperation::Enable)? else {
        return Err(AdminCliError::GdmConfig(format!(
            "PAM file '{}' does not exist",
            pam_file.display()
        )));
    };
    let Ok(content) = std::str::from_utf8(&snapshot.bytes) else {
        return Err(AdminCliError::GdmConfig(format!(
            "PAM file '{}' is not valid UTF-8; refusing to edit it",
            pam_file.display()
        )));
    };
    let include_dir = include_dir_of(pam_file);

    let Some(plan) = plan_gdm_enable(content, include_dir)? else {
        return Ok(());
    };

    // Never propagate group/world write permission to the rewritten file or its backup. The
    // snapshot keeps the unmasked mode, so an unchanged 0o664 file passes the re-check.
    let mode = snapshot.mode & 0o7755;
    let owner = (snapshot.uid, snapshot.gid);
    let mut recheck = || {
        before_rename();
        ensure_pam_file_unchanged(pam_file, Some(&snapshot), GdmOperation::Enable)
    };

    match plan {
        EnablePlan::Insert { pristine, updated } => {
            let backup = pam_backup_path(pam_file);
            let created = publish_backup(
                pam_file,
                &backup,
                pristine.as_bytes(),
                mode,
                owner,
                post_link,
            )?;
            write_atomic_checked(pam_file, updated.as_bytes(), mode, owner, &mut recheck).map_err(
                |err| match created {
                    Some(identity) => discard_created_backup(pam_file, &backup, identity, err),
                    None => err,
                },
            )
        }
        // GitHub #331: never creates a backup and leaves an existing one byte-identical, so
        // `gdm restore` keeps returning the pristine pre-soos bytes (and keeps refusing a
        // stale backup without `--force`).
        EnablePlan::RemoveRedundant { updated } => {
            write_atomic_checked(pam_file, updated.as_bytes(), mode, owner, &mut recheck)
        }
    }
}

/// Directory holding `path` (`.` for a bare file name).
fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// Upper bound on the fresh temporary names tried before giving up (GitHub #335).
const MAX_TEMP_NAME_ATTEMPTS: u32 = 16;

/// Creates a new `0600` file `<dir>/.<file_name>.soos-tmp-<pid>-<n>` with `create_new`
/// (`O_CREAT | O_EXCL`: never opens or truncates an existing file, never follows a symlink at
/// that name), trying `n` in `0..MAX_TEMP_NAME_ATTEMPTS` on `AlreadyExists` (GitHub #335).
///
/// Returns the path actually created and its handle; any other error is returned at once, and
/// after the last attempt the `AlreadyExists` error is returned. A stale file left by a
/// crashed run with a reused PID only consumes one attempt and is never removed or modified.
fn create_temp_sibling(dir: &Path, file_name: &str) -> std::io::Result<(PathBuf, fs::File)> {
    let pid = std::process::id();
    let mut last = std::io::Error::from(std::io::ErrorKind::AlreadyExists);
    for n in 0..MAX_TEMP_NAME_ATTEMPTS {
        let path = dir.join(format!(".{file_name}.soos-tmp-{pid}-{n}"));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// Removes `path` only when it is still the regular file with exactly `identity`
/// (`symlink_metadata`, no follow); best effort, every error is ignored (GitHub #335).
fn remove_own_file(path: &Path, identity: (u64, u64)) {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.is_file() && (meta.dev(), meta.ino()) == identity {
            let _ = fs::remove_file(path);
        }
    }
}

/// Publishes `bytes` as the backup without ever replacing an existing name (GitHub #333): the
/// bytes go to an exclusive temporary file in the same directory ([`create_temp_sibling`],
/// mode, owner, `fsync`), which is hard-linked to `backup` (`linkat(2)` fails with `EEXIST` on
/// any existing name, a symlink included, and never follows it). Only the temporary file this
/// call created is ever removed; a stale temporary file of another run is left alone
/// (GitHub #335).
///
/// After a successful link, `post_link(tmp, dir)` runs once (production:
/// [`backup_post_link_steps`]). If it fails, this run's temporary file is removed again (only
/// while it is still the created inode) and the backup this call created goes through
/// [`discard_created_backup`], so a failed `enable` leaves no new backup unless the re-read PAM
/// file holds a soos managed block or cannot be re-read (GitHub #335).
///
/// Returns the `(dev, ino)` of the backup this call created, or `None` when a backup already
/// existed (it is kept untouched: it holds the pristine pre-soos state).
fn publish_backup(
    pam_file: &Path,
    backup: &Path,
    bytes: &[u8],
    mode: u32,
    owner: (u32, u32),
    post_link: &mut dyn FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<Option<(u64, u64)>, AdminCliError> {
    // Fast path only; correctness relies on the link below.
    match fs::symlink_metadata(backup) {
        Ok(_) => return Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(gdm_error("Failed to inspect PAM backup", backup, &e)),
    }
    let write_error =
        |e: &std::io::Error| gdm_error("Failed to write PAM file atomically", backup, e);
    let dir = parent_dir(backup);
    let file_name = backup
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (tmp, file) = create_temp_sibling(dir, &file_name).map_err(|e| write_error(&e))?;
    let identity = match link_new_file(file, &tmp, backup, bytes, mode, owner) {
        Ok(Some(identity)) => identity,
        Ok(None) => {
            // A backup appeared concurrently: it is kept; only this run's temporary file goes.
            return match fs::remove_file(&tmp) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(write_error(&e)),
                _ => Ok(None),
            };
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(write_error(&e));
        }
    };
    if let Err(e) = post_link(&tmp, dir) {
        remove_own_file(&tmp, identity);
        return Err(discard_created_backup(
            pam_file,
            backup,
            identity,
            write_error(&e),
        ));
    }
    Ok(Some(identity))
}

/// Writes `bytes` to `file` (the new temporary file `tmp`) and hard-links it to `target`;
/// `None` when `target` already exists.
fn link_new_file(
    mut file: fs::File,
    tmp: &Path,
    target: &Path,
    bytes: &[u8],
    mode: u32,
    owner: (u32, u32),
) -> std::io::Result<Option<(u64, u64)>> {
    file.write_all(bytes)?;
    std::os::unix::fs::fchown(&file, Some(owner.0), Some(owner.1))?;
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    file.sync_all()?;
    let metadata = file.metadata()?;
    drop(file);
    match fs::hard_link(tmp, target) {
        Ok(()) => Ok(Some((metadata.dev(), metadata.ino()))),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(e) => Err(e),
    }
}

/// After a failed PAM-file write, removes the backup this run created (`identity`), and
/// returns `err`, possibly with a note on the backup (GitHub #333).
///
/// The backup is kept when the PAM file now holds a soos managed block (a concurrent
/// `gdm enable` won the rename and relies on it) or cannot be re-read; it is removed only when
/// it is still the regular file with exactly `identity`. A replaced backup is left alone (a
/// replacement between that check and the removal is a stated, root-only residual).
fn discard_created_backup(
    pam_file: &Path,
    backup: &Path,
    identity: (u64, u64),
    err: AdminCliError,
) -> AdminCliError {
    let note = |err: AdminCliError, reason: &str| match err {
        AdminCliError::GdmConfig(msg) => AdminCliError::GdmConfig(format!(
            "{msg} The backup '{}' created by this run could not be removed: {reason}",
            backup.display()
        )),
        other => other,
    };
    match read_pam_file_snapshot(pam_file, GdmOperation::Enable) {
        Ok(Some(current)) if holds_managed_block(&current.bytes) => return err,
        Ok(_) => {}
        Err(AdminCliError::GdmConfig(reason)) => return note(err, &reason),
        Err(other) => return note(err, &other.to_string()),
    }
    match fs::symlink_metadata(backup) {
        Ok(meta) if meta.is_file() && (meta.dev(), meta.ino()) == identity => {
            if let Err(e) = fs::remove_file(backup) {
                return note(err, &e.to_string());
            }
            // Best effort: the removal itself already happened.
            let _ = fs::File::open(parent_dir(backup)).and_then(|d| d.sync_all());
            err
        }
        Ok(_) => note(err, "it was replaced"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => err,
        Err(e) => note(err, &e.to_string()),
    }
}

/// True when `bytes` hold a soos managed block (a line equal to [`GDM_BLOCK_BEGIN`] once
/// trimmed, as [`strip_managed_rules`] detects it).
fn holds_managed_block(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes)
        .lines()
        .any(|line| line.trim() == GDM_BLOCK_BEGIN)
}

/// Result of [`plan_gdm_enable`] when the file must be rewritten.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EnablePlan {
    /// Insert the managed block.
    Insert {
        /// Content without any soos rule (what the backup must hold).
        pristine: String,
        /// Content with the managed block at the safe position.
        updated: String,
    },
    /// GitHub #331: the delegated stack already reaches a shared primary soos rule; write the
    /// file without its managed rules. Never creates a backup.
    RemoveRedundant {
        /// Content without any soos managed rule.
        updated: String,
    },
}

/// Refuses a PAM file using line continuations (the stack order cannot be told).
fn refuse_continuations(content: &str) -> Result<(), AdminCliError> {
    if content.lines().any(|l| l.trim_end().ends_with('\\')) {
        return Err(AdminCliError::GdmConfig(
            "PAM file uses line continuations; refusing to edit it automatically".into(),
        ));
    }
    Ok(())
}

/// The pre-credential part of a GDM service file, as `gdm enable` classifies it.
struct AnchorScan<'a> {
    /// Lines of the pristine content, line endings included.
    lines: Vec<&'a str>,
    /// Index in `lines` of the first credential or delegating rule.
    anchor_index: usize,
    /// That rule.
    anchor_rule: PamLine<'a>,
    /// Number of pre-credential auth rules before the anchor.
    ordinal: usize,
    /// `(ordinal, largest jump)` of every pre-credential rule with a `[...=N]` jump.
    jumps: Vec<(usize, usize)>,
}

impl<'a> AnchorScan<'a> {
    /// The lines from the anchor onward (what a delegated-stack scan starts from).
    fn delegated_lines(&self) -> &[&'a str] {
        self.lines.get(self.anchor_index..).unwrap_or_default()
    }

    /// Auth-rule position at which the `[...=N]` jump of the rule at `from` lands.
    fn jump_target(from: usize, jump: usize) -> usize {
        from.saturating_add(jump).saturating_add(1)
    }

    /// True when a pre-anchor jump lands on or beyond the anchor: inserting or removing
    /// rules at the insertion point would change its target.
    fn jump_crosses_anchor(&self) -> bool {
        self.jumps
            .iter()
            .any(|&(from, jump)| Self::jump_target(from, jump) >= self.ordinal)
    }

    /// True when a pre-anchor jump lands beyond the anchor, so that branch of the stack skips
    /// the delegation (and any shared rule it reaches).
    fn jump_skips_anchor(&self) -> bool {
        self.jumps
            .iter()
            .any(|&(from, jump)| Self::jump_target(from, jump) > self.ordinal)
    }
}

/// The refusal of a `[...=N]` jump whose target an edit would change.
fn jump_crossing_error() -> AdminCliError {
    AdminCliError::GdmConfig(
        "a [...=N] jump before the insertion point would change target; \
         refusing to edit the PAM file automatically"
            .into(),
    )
}

/// Classifies the auth rules of `pristine` up to its credential or delegating anchor. The
/// single source of truth of `gdm enable` and `gdm status` (GitHub #331).
///
/// # Errors
///
/// Refuses an unclassified auth rule before the anchor and a file without an anchor
/// (line continuations are refused on the whole file first, [`refuse_continuations`]).
fn pre_credential_scan(pristine: &str) -> Result<AnchorScan<'_>, AdminCliError> {
    let lines: Vec<&str> = pristine.split_inclusive('\n').collect();
    let mut anchor = None;
    let mut ordinal = 0_usize;
    let mut jumps = Vec::new();
    for (index, raw) in lines.iter().enumerate() {
        let Some(rule) = PamLine::parse(raw) else {
            continue;
        };
        if !rule.is_auth() {
            continue;
        }
        if rule.delegation().is_some() || rule.is_credential() {
            anchor = Some((index, rule));
            break;
        }
        if rule.is_pre_credential() {
            if let Some(jump) = rule.max_jump() {
                jumps.push((ordinal, jump));
            }
            ordinal = ordinal.saturating_add(1);
            continue;
        }
        // Fail closed: an unclassified rule before the credential anchor may be a
        // lockout or login gate that a face match must never skip.
        return Err(AdminCliError::GdmConfig(format!(
            "the PAM file runs the unclassified auth rule {} before its credential module \
             or shared auth stack; soos cannot tell whether it is a lockout or login gate, \
             so pam_soos.so is not inserted automatically \
             (see Docs/DISTRIBUTION_DEPLOYMENT.md section 2.1)",
            rule.describe()
        )));
    }
    let Some((anchor_index, anchor_rule)) = anchor else {
        return Err(AdminCliError::GdmConfig(
            "no credential module (pam_unix.so, pam_sss.so, ...) or shared auth stack \
             (include/substack/@include) found in the PAM file; refusing to guess where \
             pam_soos.so belongs"
                .into(),
        ));
    };
    Ok(AnchorScan {
        lines,
        anchor_index,
        anchor_rule,
        ordinal,
        jumps,
    })
}

/// Computes the rewritten GDM service file, or `None` when nothing must change.
fn plan_gdm_enable(content: &str, include_dir: &Path) -> Result<Option<EnablePlan>, AdminCliError> {
    refuse_continuations(content)?;
    let pristine = strip_managed_rules(content)?;
    if has_active_pam_soos_rule(&pristine) {
        return Ok(None);
    }

    let scan = pre_credential_scan(&pristine)?;
    let jump_crosses = scan.jump_crosses_anchor();
    // GitHub #333: the predicate `gdm status` uses, so both commands agree.
    let jump_skips = scan.jump_skips_anchor();
    let AnchorScan {
        lines,
        anchor_index,
        anchor_rule,
        jumps,
        ..
    } = scan;
    // GitHub #331: the delegated scan runs before the jump check, but its error surfaces
    // after it (unchanged priority). A shared primary soos rule short-circuits: nothing is
    // inserted. When nothing is removed either, no jump target can move. Removing managed
    // rules does move every jump that crosses them (the block sits at the insertion point and
    // a crossing jump coexisting with it was written with its rules counted), so the jump
    // check applies then (candid review 2026-10-05).
    let delegated = anchor_rule
        .delegation()
        .map(|_| delegated_auth(include_dir, lines.get(anchor_index..).unwrap_or_default()));
    if let Some(Ok(DelegatedAuth::SharedSoosRule { stack })) = &delegated {
        if pristine != content && jump_crosses {
            return Err(jump_crossing_error());
        }
        // GitHub #333: a pre-anchor jump landing beyond the delegation skips the shared rule
        // on that branch; `gdm status` reports `installed: false`, so enable refuses.
        if jump_skips {
            return Err(AdminCliError::GdmConfig(format!(
                "a [...=N] jump in the PAM file lands beyond the shared auth stack '{stack}', so \
                 that branch never reaches its pam_soos.so rule; nothing was written and GDM \
                 face login is not enabled automatically \
                 (see Docs/DISTRIBUTION_DEPLOYMENT.md section 2.1)"
            )));
        }
        if pristine == content {
            return Ok(None);
        }
        return Ok(Some(EnablePlan::RemoveRedundant { updated: pristine }));
    }
    let has_jump = !jumps.is_empty();
    // A jump from a rule before the insertion point that lands on or beyond it would
    // silently change target once rules are inserted.
    if jump_crosses {
        return Err(jump_crossing_error());
    }

    // Only an earlier rule with a plain `required`/`requisite` control enforces the
    // gate; an `optional` (or any other) copy earlier in the file does not count.
    // When any `[...=N]` jump precedes the anchor, an earlier copy may be skipped
    // (e.g. `pam_succeed_if ... ingroup vip` jumping over `requisite pam_nologin.so`),
    // so nothing is de-duplicated: running a gate twice is harmless (GitHub #278).
    let already_present: Vec<String> = if has_jump {
        Vec::new()
    } else {
        lines
            .iter()
            .take(anchor_index)
            .filter_map(|raw| PamLine::parse(raw))
            .filter(|rule| rule.is_auth() && rule.is_enforcing())
            .filter_map(|rule| normalized_rule(&rule))
            .collect()
    };
    let gates = match delegated {
        Some(result) => match result? {
            DelegatedAuth::Gates(gates) => gates,
            // Handled above; kept total without a panic path.
            DelegatedAuth::SharedSoosRule { .. } => return Ok(None),
        },
        None => Vec::new(),
    };

    let mut updated = String::with_capacity(pristine.len().saturating_add(512));
    for raw in lines.iter().take(anchor_index) {
        updated.push_str(raw);
    }
    updated.push_str(GDM_BLOCK_BEGIN);
    updated.push('\n');
    for gate in gates {
        let duplicate = PamLine::parse(&gate)
            .and_then(|rule| normalized_rule(&rule))
            .is_some_and(|norm| already_present.contains(&norm));
        if !duplicate {
            updated.push_str(&gate);
            updated.push('\n');
        }
    }
    updated.push_str(GDM_PAM_LINE);
    updated.push('\n');
    updated.push_str(GDM_BLOCK_END);
    updated.push('\n');
    for raw in lines.iter().skip(anchor_index) {
        updated.push_str(raw);
    }

    if updated == content {
        return Ok(None);
    }
    Ok(Some(EnablePlan::Insert { pristine, updated }))
}

/// `"<module basename> <args>"` of a non-delegating rule, for duplicate detection.
fn normalized_rule(rule: &PamLine<'_>) -> Option<String> {
    let name = rule.module_name()?;
    let PamLine::Rule { args, .. } = rule else {
        return None;
    };
    Some(
        std::iter::once(name)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Removes the managed block and any [`LEGACY_GDM_PAM_LINE`] / bare
/// [`GDM_PAM_LINE`] rule, preserving every other byte.
fn strip_managed_rules(content: &str) -> Result<String, AdminCliError> {
    let squash = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let legacy = squash(LEGACY_GDM_PAM_LINE);
    let current = squash(GDM_PAM_LINE);
    let mut out = String::with_capacity(content.len());
    let mut in_block = false;
    for raw in content.split_inclusive('\n') {
        let trimmed = raw.trim();
        if in_block {
            if trimmed == GDM_BLOCK_END {
                in_block = false;
            }
            continue;
        }
        if trimmed == GDM_BLOCK_BEGIN {
            in_block = true;
            continue;
        }
        if trimmed == GDM_BLOCK_END {
            return Err(AdminCliError::GdmConfig(
                "unbalanced soos managed block in the PAM file; restore it first".into(),
            ));
        }
        let squashed = squash(trimmed);
        if squashed == legacy || squashed == current {
            continue;
        }
        out.push_str(raw);
    }
    if in_block {
        return Err(AdminCliError::GdmConfig(
            "unterminated soos managed block in the PAM file; restore it first".into(),
        ));
    }
    Ok(out)
}

/// Restores `pam_file` from its [`pam_backup_path`] copy (bytes, mode and owner),
/// atomically, then removes the backup. Fails without any change when the backup is
/// missing, a symlink, not a regular file or larger than [`MAX_PAM_FILE_BYTES`].
/// The backup is checked and read through one `O_NOFOLLOW` descriptor, so the checks
/// and the bytes restored concern the same file.
///
/// Without `force`, a stale backup is refused and nothing changes (GitHub #312,
/// STO-NEW-7): the current PAM file, once its soos managed rules are removed, must be
/// byte-for-byte the backup, so that a change made after `gdm enable` is never discarded.
fn restore_gdm_pam_file(pam_file: &Path, force: bool) -> Result<(), AdminCliError> {
    restore_gdm_pam_file_with(pam_file, force, &mut || {})
}

/// [`restore_gdm_pam_file`] with a hook run between the comparison and the rename (a test
/// seam: production passes a no-op).
///
/// GitHub #318: the PAM file is read once (`O_NOFOLLOW`, bounded) into a [`PamFileSnapshot`];
/// that snapshot is what the stale-backup check compares, and right before the rename (after
/// the temporary file is written and synced) the file is read again and must still be the
/// same inode with the same bytes (or still be absent). Otherwise nothing is written and the
/// restore fails with "changed concurrently". `force` skips the stale-backup check only.
fn restore_gdm_pam_file_with(
    pam_file: &Path,
    force: bool,
    before_rename: &mut dyn FnMut(),
) -> Result<(), AdminCliError> {
    let backup = pam_backup_path(pam_file);
    let (bytes, backup_meta) = read_backup_bounded(&backup)?;
    let snapshot = read_pam_file_snapshot(pam_file, GdmOperation::Restore)?;
    if let Some(current) = &snapshot {
        if !force && !backup_matches_current(pam_file, current, &bytes)? {
            return Err(AdminCliError::GdmConfig(format!(
                "'{}' was changed after `gdm enable`: restoring '{}' would discard those \
                 changes. Remove the soos rules by hand, or re-run `gdm restore --force` to \
                 restore the backup anyway",
                pam_file.display(),
                backup.display()
            )));
        }
    }
    let mode = backup_meta.permissions().mode() & 0o7755;
    write_atomic_checked(
        pam_file,
        &bytes,
        mode,
        (backup_meta.uid(), backup_meta.gid()),
        &mut || {
            before_rename();
            ensure_pam_file_unchanged(pam_file, snapshot.as_ref(), GdmOperation::Restore)
        },
    )?;
    fs::remove_file(&backup).map_err(|e| gdm_error("Failed to remove PAM backup", &backup, &e))?;
    let dir = pam_file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| gdm_error("Failed to sync PAM directory", dir, &e))
}

/// Which `gdm` command performs the checked rename (message wording and compared fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GdmOperation {
    /// `gdm enable` (GitHub #333).
    Enable,
    /// `gdm restore` (GitHub #318).
    Restore,
}

impl GdmOperation {
    /// The command as the operator types it after `soos-admin`.
    fn command(self) -> &'static str {
        match self {
            Self::Enable => "gdm enable",
            Self::Restore => "gdm restore",
        }
    }

    /// Verb of the "not a regular file" refusal (kept texts of each command).
    fn refusal_verb(self) -> &'static str {
        match self {
            Self::Enable => "use",
            Self::Restore => "replace",
        }
    }
}

/// The PAM file as one `gdm` command read it: identity, exact bytes and (GitHub #333) the
/// mode (`st_mode & 0o7777`, unmasked) and owner of the same descriptor.
#[derive(Debug)]
struct PamFileSnapshot {
    dev: u64,
    ino: u64,
    bytes: Vec<u8>,
    mode: u32,
    uid: u32,
    gid: u32,
}

impl PamFileSnapshot {
    /// Whether `other` is the same file for `operation`: identity and bytes for restore
    /// (GitHub #318), plus mode and owner for enable (GitHub #333), so that a concurrent
    /// `chmod`/`chown` aborts instead of being reverted.
    fn matches(&self, other: &Self, operation: GdmOperation) -> bool {
        let same_file = self.dev == other.dev && self.ino == other.ino && self.bytes == other.bytes;
        match operation {
            GdmOperation::Restore => same_file,
            GdmOperation::Enable => {
                same_file
                    && self.mode == other.mode
                    && self.uid == other.uid
                    && self.gid == other.gid
            }
        }
    }
}

/// Reads `pam_file` through one `O_NOFOLLOW | O_NOCTTY | O_NONBLOCK | O_CLOEXEC` descriptor,
/// bounded to [`MAX_PAM_FILE_BYTES`]; `None` when it does not exist. A symbolic link or a
/// non-regular file is refused before any byte is read (a FIFO never blocks).
fn read_pam_file_snapshot(
    pam_file: &Path,
    operation: GdmOperation,
) -> Result<Option<PamFileSnapshot>, AdminCliError> {
    let not_regular = || {
        AdminCliError::GdmConfig(format!(
            "Refusing to {} '{}': not a regular file (symlink or special file)",
            operation.refusal_verb(),
            pam_file.display()
        ))
    };
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(pam_file)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => return Err(not_regular()),
        Err(e) => return Err(gdm_error("Failed to open PAM file", pam_file, &e)),
    };
    let metadata = file
        .metadata()
        .map_err(|e| gdm_error("Failed to inspect PAM file", pam_file, &e))?;
    if !metadata.is_file() {
        return Err(not_regular());
    }
    let too_large = || {
        AdminCliError::GdmConfig(format!(
            "PAM file '{}' exceeds {MAX_PAM_FILE_BYTES} bytes",
            pam_file.display()
        ))
    };
    if metadata.len() > MAX_PAM_FILE_BYTES {
        return Err(too_large());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(MAX_PAM_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| gdm_error("Failed to read PAM file", pam_file, &e))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_PAM_FILE_BYTES {
        return Err(too_large());
    }
    Ok(Some(PamFileSnapshot {
        dev: metadata.dev(),
        ino: metadata.ino(),
        bytes,
        mode: metadata.mode() & 0o7777,
        uid: metadata.uid(),
        gid: metadata.gid(),
    }))
}

/// Fails with "changed concurrently" unless `pam_file` is still `expected`
/// ([`PamFileSnapshot::matches`] for `operation`), or still absent when `expected` is `None`
/// (GitHub #318, GitHub #333).
fn ensure_pam_file_unchanged(
    pam_file: &Path,
    expected: Option<&PamFileSnapshot>,
    operation: GdmOperation,
) -> Result<(), AdminCliError> {
    let unchanged = match (read_pam_file_snapshot(pam_file, operation), expected) {
        (Ok(None), None) => true,
        (Ok(Some(now)), Some(expected)) => now.matches(expected, operation),
        _ => false,
    };
    if unchanged {
        return Ok(());
    }
    let command = operation.command();
    Err(AdminCliError::GdmConfig(format!(
        "'{}' changed concurrently while `{command}` was running; nothing was written. \
         Check the file and re-run `soos-admin {command}`",
        pam_file.display()
    )))
}

/// Whether the compared PAM file without its soos managed rules is exactly `backup`
/// (GitHub #312).
///
/// A file that is not valid UTF-8 is refused (never rewritten lossily); an unbalanced
/// managed block never matches.
fn backup_matches_current(
    pam_file: &Path,
    current: &PamFileSnapshot,
    backup: &[u8],
) -> Result<bool, AdminCliError> {
    let Ok(current) = std::str::from_utf8(&current.bytes) else {
        return Err(AdminCliError::GdmConfig(format!(
            "PAM file '{}' is not valid UTF-8; refusing to edit it",
            pam_file.display()
        )));
    };
    Ok(strip_managed_rules(current).is_ok_and(|pristine| pristine.as_bytes() == backup))
}

/// Opens `backup` with `O_NOFOLLOW | O_NOCTTY | O_NONBLOCK | O_CLOEXEC`, checks on the open
/// descriptor that it is a regular file of at most [`MAX_PAM_FILE_BYTES`], and reads
/// it through that same descriptor (bounded). Returns the bytes and the descriptor's
/// metadata (mode and owner to restore).
fn read_backup_bounded(backup: &Path) -> Result<(Vec<u8>, fs::Metadata), AdminCliError> {
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(backup)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(AdminCliError::GdmConfig(format!(
                "PAM backup '{}' does not exist",
                backup.display()
            )));
        }
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => {
            return Err(AdminCliError::GdmConfig(format!(
                "Refusing to use '{}': not a regular file (symlink or special file)",
                backup.display()
            )));
        }
        Err(e) => return Err(gdm_error("Failed to open PAM backup", backup, &e)),
    };
    let metadata = file
        .metadata()
        .map_err(|e| gdm_error("Failed to inspect PAM backup", backup, &e))?;
    if !metadata.is_file() {
        return Err(AdminCliError::GdmConfig(format!(
            "Refusing to use '{}': not a regular file (symlink or special file)",
            backup.display()
        )));
    }
    if metadata.len() > MAX_PAM_FILE_BYTES {
        return Err(AdminCliError::GdmConfig(format!(
            "PAM backup '{}' exceeds {MAX_PAM_FILE_BYTES} bytes",
            backup.display()
        )));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(MAX_PAM_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| gdm_error("Failed to read PAM backup", backup, &e))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_PAM_FILE_BYTES {
        return Err(AdminCliError::GdmConfig(format!(
            "PAM backup '{}' exceeds {MAX_PAM_FILE_BYTES} bytes",
            backup.display()
        )));
    }
    Ok((bytes, metadata))
}

/// Writes `bytes` to `target` atomically: exclusive temporary file in the same
/// directory (created with `mode`, owned like the original), `fsync`, `rename`,
/// then `fsync` of the directory. On failure only the temporary file this call created
/// ([`create_temp_sibling`]) is removed; a stale temporary file of another run is never
/// removed or modified, and when no fresh name can be created nothing is removed
/// (GitHub #335).
///
/// `before_rename` runs once the temporary file is written and synced, right before the
/// rename; an error from it removes the temporary file and is returned as is, leaving
/// `target` untouched (GitHub #318, #333).
fn write_atomic_checked(
    target: &Path,
    bytes: &[u8],
    mode: u32,
    owner: (u32, u32),
    before_rename: &mut dyn FnMut() -> Result<(), AdminCliError>,
) -> Result<(), AdminCliError> {
    let dir = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (tmp, file) = create_temp_sibling(dir, &file_name)
        .map_err(|e| gdm_error("Failed to write PAM file atomically", target, &e))?;

    write_and_rename(file, &tmp, target, bytes, mode, owner, before_rename).map_err(|failure| {
        let _ = fs::remove_file(&tmp);
        match failure {
            WriteFailure::Io(e) => gdm_error("Failed to write PAM file atomically", target, &e),
            WriteFailure::Refused(err) => err,
        }
    })
}

/// Why [`write_and_rename`] stopped.
enum WriteFailure {
    /// A system call failed.
    Io(std::io::Error),
    /// The pre-rename check refused the rename.
    Refused(AdminCliError),
}

impl From<std::io::Error> for WriteFailure {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Writes `bytes` to `file` (the new temporary file `tmp`), syncs it, runs `before_rename`,
/// renames `tmp` over `target` and fsyncs the directory of `target`.
fn write_and_rename(
    mut file: fs::File,
    tmp: &Path,
    target: &Path,
    bytes: &[u8],
    mode: u32,
    owner: (u32, u32),
    before_rename: &mut dyn FnMut() -> Result<(), AdminCliError>,
) -> Result<(), WriteFailure> {
    file.write_all(bytes)?;
    std::os::unix::fs::fchown(&file, Some(owner.0), Some(owner.1))?;
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    file.sync_all()?;
    drop(file);
    before_rename().map_err(WriteFailure::Refused)?;
    fs::rename(tmp, target)?;
    fs::File::open(parent_dir(target))?.sync_all()?;
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Unit tests of the restore race seam use direct assertions"
)]
mod restore_race_tests {
    //! GitHub #318 (row VCO4): `gdm restore` re-checks the PAM file right before the rename
    //! and aborts without writing when it changed after the comparison.

    use super::*;

    const PRISTINE: &str = "\
#%PAM-1.0
auth    requisite       pam_nologin.so
auth    required        pam_unix.so
account required        pam_unix.so
";

    struct Fixture {
        dir: tempfile::TempDir,
        pam_file: PathBuf,
    }

    fn enabled() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let pam_file = dir.path().join("gdm-password");
        fs::write(&pam_file, PRISTINE).unwrap();
        let disable = dir.path().join("gdm.disable");
        configure_gdm(&GdmAction::Enable, &pam_file, &disable).unwrap();
        assert!(pam_backup_path(&pam_file).exists());
        Fixture { dir, pam_file }
    }

    fn leftover_temp_files(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains("soos-tmp"))
            .collect()
    }

    fn assert_aborted(f: &Fixture, result: Result<(), AdminCliError>, expected: &str) {
        let err = result.expect_err("a concurrent change must abort the restore");
        let msg = err.to_string();
        assert!(msg.contains("changed concurrently"), "{msg}");
        assert!(
            msg.contains("re-run"),
            "the error must suggest re-running: {msg}"
        );
        assert_eq!(
            fs::read_to_string(&f.pam_file).unwrap(),
            expected,
            "the concurrent change must be kept, nothing written"
        );
        assert_eq!(
            fs::read_to_string(pam_backup_path(&f.pam_file)).unwrap(),
            PRISTINE,
            "the backup must be kept"
        );
        assert!(leftover_temp_files(f.dir.path()).is_empty());
    }

    /// VCO4: an edit between the comparison and the rename aborts the restore.
    #[test]
    fn test_vco_gdm_restore_aborts_when_file_changes_before_rename() {
        let f = enabled();
        let mut changed = String::new();
        let pam_file = f.pam_file.clone();
        let result = restore_gdm_pam_file_with(&f.pam_file, false, &mut || {
            let mut content = fs::read_to_string(&pam_file).unwrap();
            content.push_str("session optional        pam_keyinit.so force revoke\n");
            fs::write(&pam_file, &content).unwrap();
            changed = content;
        });
        assert_aborted(&f, result, &changed);
    }

    /// VCO4: `--force` skips the stale-backup comparison, never the concurrency check.
    #[test]
    fn test_vco_gdm_restore_force_still_aborts_on_concurrent_change() {
        let f = enabled();
        let pam_file = f.pam_file.clone();
        let result = restore_gdm_pam_file_with(&f.pam_file, true, &mut || {
            fs::write(&pam_file, "auth required pam_deny.so\n").unwrap();
        });
        assert_aborted(&f, result, "auth required pam_deny.so\n");
    }

    /// VCO4: a file replaced by another inode with identical bytes is a concurrent change too.
    #[test]
    fn test_vco_gdm_restore_aborts_when_file_is_replaced_by_same_bytes() {
        let f = enabled();
        let before = fs::read_to_string(&f.pam_file).unwrap();
        let pam_file = f.pam_file.clone();
        let other = f.dir.path().join("replacement");
        let result = restore_gdm_pam_file_with(&f.pam_file, false, &mut || {
            fs::write(&other, fs::read(&pam_file).unwrap()).unwrap();
            fs::rename(&other, &pam_file).unwrap();
        });
        assert_aborted(&f, result, &before);
    }

    /// VCO4: a PAM file created after the comparison found none is never overwritten.
    #[test]
    fn test_vco_gdm_restore_aborts_when_missing_file_appears() {
        let f = enabled();
        fs::remove_file(&f.pam_file).unwrap();
        let pam_file = f.pam_file.clone();
        let result = restore_gdm_pam_file_with(&f.pam_file, false, &mut || {
            fs::write(&pam_file, "auth required pam_unix.so\n").unwrap();
        });
        assert_aborted(&f, result, "auth required pam_unix.so\n");
    }

    /// VCO4: without a concurrent change the restore still succeeds.
    #[test]
    fn test_vco_gdm_restore_without_concurrent_change_restores() {
        let f = enabled();
        let mut calls = 0_u32;
        restore_gdm_pam_file_with(&f.pam_file, false, &mut || calls += 1).unwrap();
        assert_eq!(calls, 1, "the hook runs exactly once, before the rename");
        assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), PRISTINE);
        assert!(!pam_backup_path(&f.pam_file).exists());
        assert!(leftover_temp_files(f.dir.path()).is_empty());
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "Unit tests of the enable race seam use direct assertions"
)]
mod enable_race_tests {
    //! GitHub #333 (row GHF1): `gdm enable` re-checks the PAM file right before the rename
    //! (identity, bytes, mode and owner) and aborts without writing when it changed after the
    //! read. A backup created by the aborted run is removed again; an existing backup, a
    //! symlink at the backup path and a backup replaced meanwhile are never touched.

    use super::*;
    use std::collections::BTreeSet;
    use std::os::unix::fs::symlink;

    const PRISTINE: &str = "\
#%PAM-1.0
auth    requisite       pam_nologin.so
auth    required        pam_unix.so
account required        pam_unix.so
";

    /// Shared stack without any soos rule (enable inserts a managed block).
    const PLAIN_SHARED: &str = "\
auth       required                    pam_faillock.so preauth
auth       sufficient                  pam_unix.so nullok
auth       required                    pam_deny.so
";

    /// Shared stack whose primary soos rule makes a managed block redundant.
    const SOOS_SHARED: &str = "\
auth       required                    pam_faillock.so preauth
auth       [success=done default=ignore]  pam_soos.so
auth       sufficient                  pam_unix.so nullok
auth       required                    pam_deny.so
";

    const DELEGATING: &str = "\
#%PAM-1.0
auth       requisite   pam_nologin.so
auth       include     shared-auth
account    include     shared-auth
";

    struct Fixture {
        dir: tempfile::TempDir,
        pam_file: PathBuf,
    }

    fn fixture(content: &str, mode: u32) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let pam_file = dir.path().join("gdm-password");
        fs::write(&pam_file, content).unwrap();
        fs::set_permissions(&pam_file, fs::Permissions::from_mode(mode)).unwrap();
        Fixture { dir, pam_file }
    }

    /// A GDM file holding a managed block over a shared stack that now carries a primary
    /// soos rule (`EnablePlan::RemoveRedundant`), with the backup of the first enable.
    fn redundant_block() -> Fixture {
        let f = fixture(DELEGATING, 0o644);
        let shared = f.dir.path().join("shared-auth");
        fs::write(&shared, PLAIN_SHARED).unwrap();
        ensure_gdm_pam_line(&f.pam_file).unwrap();
        assert!(fs::read_to_string(&f.pam_file)
            .unwrap()
            .contains(GDM_BLOCK_BEGIN));
        assert!(pam_backup_path(&f.pam_file).exists());
        fs::write(&shared, SOOS_SHARED).unwrap();
        f
    }

    fn entries(dir: &Path) -> BTreeSet<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    fn names(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    fn e1(pam_file: &Path) -> String {
        format!(
            "GDM configuration error: '{}' changed concurrently while `gdm enable` was running; \
             nothing was written. Check the file and re-run `soos-admin gdm enable`",
            pam_file.display()
        )
    }

    /// Runs the enable with `hook` before the rename; returns the result and the hook calls.
    fn enable_with(pam_file: &Path, mut hook: impl FnMut()) -> (Result<(), AdminCliError>, u32) {
        let mut calls = 0_u32;
        let result = ensure_gdm_pam_line_with(pam_file, &mut || {
            calls += 1;
            hook();
        });
        (result, calls)
    }

    /// Replaces `path` atomically (another inode) with `content`.
    fn replace_file(path: &Path, content: &[u8]) {
        let other = path.with_extension("concurrent-writer");
        fs::write(&other, content).unwrap();
        fs::rename(&other, path).unwrap();
    }

    fn append_line(path: &Path) -> String {
        let mut content = fs::read_to_string(path).unwrap();
        content.push_str("session optional        pam_keyinit.so force revoke\n");
        fs::write(path, &content).unwrap();
        content
    }

    /// GHF1: an edit between the read and the rename aborts the Insert; the concurrent bytes
    /// are kept, the backup this run created is removed, no temporary file is left.
    #[test]
    fn test_ghf1_enable_aborts_when_file_is_edited_before_rename() {
        let f = fixture(PRISTINE, 0o644);
        let pam_file = f.pam_file.clone();
        let mut changed = String::new();
        let (result, calls) = enable_with(&f.pam_file, || changed = append_line(&pam_file));
        assert_eq!(calls, 1, "the hook runs exactly once, before the rename");
        let err = result.expect_err("a concurrent edit must abort gdm enable");
        assert_eq!(err.to_string(), e1(&f.pam_file));
        assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), changed);
        assert!(
            !pam_backup_path(&f.pam_file).exists(),
            "the backup created by the aborted run must be removed"
        );
        assert_eq!(entries(f.dir.path()), names(&["gdm-password"]));
    }

    /// GHF1: a file replaced by another inode with identical bytes is a concurrent change.
    #[test]
    fn test_ghf1_enable_aborts_when_file_is_replaced_by_same_bytes() {
        let f = fixture(PRISTINE, 0o644);
        let pam_file = f.pam_file.clone();
        let (result, calls) = enable_with(&f.pam_file, || {
            let bytes = fs::read(&pam_file).unwrap();
            replace_file(&pam_file, &bytes);
            fs::set_permissions(&pam_file, fs::Permissions::from_mode(0o644)).unwrap();
        });
        assert_eq!(calls, 1);
        let err = result.expect_err("a replaced inode must abort gdm enable");
        assert_eq!(err.to_string(), e1(&f.pam_file));
        assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), PRISTINE);
        assert!(!pam_backup_path(&f.pam_file).exists());
        assert_eq!(entries(f.dir.path()), names(&["gdm-password"]));
    }

    /// GHF1: a PAM file deleted between the read and the rename is never re-created.
    #[test]
    fn test_ghf1_enable_aborts_when_file_is_deleted_before_rename() {
        let f = fixture(PRISTINE, 0o644);
        let pam_file = f.pam_file.clone();
        let (result, calls) = enable_with(&f.pam_file, || fs::remove_file(&pam_file).unwrap());
        assert_eq!(calls, 1);
        let err = result.expect_err("a vanished file must abort gdm enable");
        assert_eq!(err.to_string(), e1(&f.pam_file));
        assert!(
            !f.pam_file.exists(),
            "the deleted file must not be re-created"
        );
        assert!(!pam_backup_path(&f.pam_file).exists());
        assert!(
            entries(f.dir.path()).is_empty(),
            "nothing may be left behind"
        );
    }

    /// GHF1 / plan evaluator R2-1: a concurrent `chmod` aborts instead of being reverted to
    /// the mode read at the start.
    #[test]
    fn test_ghf1_enable_aborts_on_chmod_before_rename() {
        let f = fixture(PRISTINE, 0o644);
        let pam_file = f.pam_file.clone();
        let (result, calls) = enable_with(&f.pam_file, || {
            fs::set_permissions(&pam_file, fs::Permissions::from_mode(0o600)).unwrap();
        });
        assert_eq!(calls, 1);
        let err = result.expect_err("a concurrent chmod must abort gdm enable");
        assert_eq!(err.to_string(), e1(&f.pam_file));
        assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), PRISTINE);
        assert_eq!(
            fs::metadata(&f.pam_file).unwrap().permissions().mode() & 0o7777,
            0o600,
            "the concurrent mode is kept"
        );
        assert!(!pam_backup_path(&f.pam_file).exists());
        assert_eq!(entries(f.dir.path()), names(&["gdm-password"]));
    }

    /// GHF1 / plan evaluator R2-1: an unchanged group-writable file (`0o664`) is not a
    /// concurrent change; the rewritten file and its backup drop group/world write (`0o644`).
    #[test]
    fn test_ghf1_enable_of_group_writable_file_passes_the_recheck() {
        let f = fixture(PRISTINE, 0o664);
        let (result, calls) = enable_with(&f.pam_file, || {});
        assert_eq!(calls, 1, "the re-check runs once before the rename");
        result.expect("an unchanged 0o664 file must not be reported as changed");
        assert!(fs::read_to_string(&f.pam_file)
            .unwrap()
            .contains(GDM_PAM_LINE));
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode(&f.pam_file), 0o644);
        assert_eq!(mode(&pam_backup_path(&f.pam_file)), 0o644);
    }

    /// GHF1: RemoveRedundant aborts on a concurrent edit and never touches the backup.
    #[test]
    fn test_ghf1_enable_remove_redundant_aborts_and_keeps_backup() {
        let f = redundant_block();
        let backup = pam_backup_path(&f.pam_file);
        let backup_bytes = fs::read(&backup).unwrap();
        let backup_ino = fs::metadata(&backup).unwrap().ino();
        let before = entries(f.dir.path());
        let pam_file = f.pam_file.clone();
        let mut changed = String::new();
        let (result, calls) = enable_with(&f.pam_file, || changed = append_line(&pam_file));
        assert_eq!(calls, 1);
        let err = result.expect_err("a concurrent edit must abort the block removal");
        assert_eq!(err.to_string(), e1(&f.pam_file));
        assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), changed);
        assert_eq!(
            fs::read(&backup).unwrap(),
            backup_bytes,
            "backup bytes kept"
        );
        assert_eq!(
            fs::metadata(&backup).unwrap().ino(),
            backup_ino,
            "backup inode kept"
        );
        assert_eq!(entries(f.dir.path()), before, "no temporary file left");
    }

    /// GHF1 / plan evaluator R2-4: with a pre-existing backup the run creates none, so an
    /// abort leaves that backup byte-identical (same inode) and adds no E1b suffix.
    #[test]
    fn test_ghf1_enable_abort_keeps_preexisting_backup() {
        let f = fixture(PRISTINE, 0o644);
        let backup = pam_backup_path(&f.pam_file);
        fs::write(&backup, "#%PAM-1.0\n# older pristine copy\n").unwrap();
        let backup_ino = fs::metadata(&backup).unwrap().ino();
        let pam_file = f.pam_file.clone();
        let mut changed = String::new();
        let (result, calls) = enable_with(&f.pam_file, || changed = append_line(&pam_file));
        assert_eq!(calls, 1);
        let err = result.expect_err("a concurrent edit must abort gdm enable");
        assert_eq!(err.to_string(), e1(&f.pam_file));
        assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), changed);
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "#%PAM-1.0\n# older pristine copy\n"
        );
        assert_eq!(fs::metadata(&backup).unwrap().ino(), backup_ino);
        assert_eq!(
            entries(f.dir.path()),
            names(&["gdm-password", "gdm-password.soos-backup"])
        );
    }

    /// GHF1 / plan evaluator R2-4: a symlink planted at the backup path is never followed,
    /// replaced or removed (`linkat` fails with `EEXIST`), whether the run aborts or not.
    #[test]
    fn test_ghf1_enable_never_follows_or_removes_a_backup_symlink() {
        for abort in [true, false] {
            let f = fixture(PRISTINE, 0o644);
            let victim = f.dir.path().join("victim");
            fs::write(&victim, "victim\n").unwrap();
            let backup = pam_backup_path(&f.pam_file);
            symlink(&victim, &backup).unwrap();
            let pam_file = f.pam_file.clone();
            let (result, calls) = enable_with(&f.pam_file, || {
                if abort {
                    append_line(&pam_file);
                }
            });
            assert_eq!(calls, 1, "abort={abort}");
            if abort {
                let err = result.expect_err("a concurrent edit must abort gdm enable");
                assert_eq!(err.to_string(), e1(&f.pam_file));
            } else {
                result.expect("an existing backup name never blocks the enable");
                assert!(fs::read_to_string(&f.pam_file)
                    .unwrap()
                    .contains(GDM_PAM_LINE));
            }
            let meta = fs::symlink_metadata(&backup).unwrap();
            assert!(meta.file_type().is_symlink(), "abort={abort}: symlink kept");
            assert_eq!(fs::read_link(&backup).unwrap(), victim);
            assert_eq!(fs::read_to_string(&victim).unwrap(), "victim\n");
            assert_eq!(
                entries(f.dir.path()),
                names(&["gdm-password", "gdm-password.soos-backup", "victim"]),
                "abort={abort}: no temporary file left"
            );
        }
    }

    /// GHF1 / plan evaluator R2-4: a backup replaced after this run linked it is not this
    /// run's inode: it is left alone and the error carries the E1b "it was replaced" suffix.
    #[test]
    fn test_ghf1_enable_abort_leaves_a_replaced_backup_and_reports_it() {
        let f = fixture(PRISTINE, 0o644);
        let backup = pam_backup_path(&f.pam_file);
        let pam_file = f.pam_file.clone();
        let backup_in_hook = backup.clone();
        let (result, calls) = enable_with(&f.pam_file, || {
            replace_file(&backup_in_hook, b"#%PAM-1.0\n# replaced concurrently\n");
            append_line(&pam_file);
        });
        assert_eq!(calls, 1);
        let msg = result
            .expect_err("a concurrent edit must abort gdm enable")
            .to_string();
        assert!(msg.starts_with(&e1(&f.pam_file)), "{msg}");
        assert!(
            msg.ends_with(&format!(
                " The backup '{}' created by this run could not be removed: it was replaced",
                backup.display()
            )),
            "{msg}"
        );
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "#%PAM-1.0\n# replaced concurrently\n",
            "a backup that is not this run's inode is never removed"
        );
    }

    /// GHF1 / plan evaluator R2-2 (the spec leaves the choice open; the tester contract pins
    /// the safe option): when the losing run's re-read finds a soos managed block (a
    /// concurrent `gdm enable` won the rename), its backup is the one the winner relies on and
    /// is not removed.
    #[test]
    fn test_ghf1_losing_enable_keeps_backup_when_file_holds_managed_block() {
        let f = fixture(PRISTINE, 0o644);
        let winner = format!(
            "#%PAM-1.0\nauth    requisite       pam_nologin.so\n{GDM_BLOCK_BEGIN}\n\
             {GDM_PAM_LINE}\n{GDM_BLOCK_END}\nauth    required        pam_unix.so\n\
             account required        pam_unix.so\n"
        );
        let pam_file = f.pam_file.clone();
        let winner_bytes = winner.clone();
        let (result, calls) = enable_with(&f.pam_file, || {
            replace_file(&pam_file, winner_bytes.as_bytes());
        });
        assert_eq!(calls, 1);
        let msg = result
            .expect_err("the losing run must abort with changed concurrently")
            .to_string();
        assert!(msg.starts_with(&e1(&f.pam_file)), "{msg}");
        assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), winner);
        assert_eq!(
            fs::read_to_string(pam_backup_path(&f.pam_file)).unwrap(),
            PRISTINE,
            "the winner's managed block needs the pristine backup: it must be kept"
        );
    }

    /// GHF1: the hook runs exactly once when the file is rewritten (Insert and
    /// RemoveRedundant) and never when nothing is written.
    #[test]
    fn test_ghf1_enable_hook_runs_once_per_write_and_never_without_write() {
        let f = fixture(PRISTINE, 0o644);
        let (result, calls) = enable_with(&f.pam_file, || {});
        result.unwrap();
        assert_eq!(calls, 1, "Insert: one re-check before the rename");
        assert!(fs::read_to_string(&f.pam_file)
            .unwrap()
            .contains(GDM_PAM_LINE));
        let ino = fs::metadata(&f.pam_file).unwrap().ino();
        let (result, calls) = enable_with(&f.pam_file, || {});
        result.unwrap();
        assert_eq!(calls, 0, "nothing to write: no rename, no hook");
        assert_eq!(fs::metadata(&f.pam_file).unwrap().ino(), ino);

        let r = redundant_block();
        let backup = pam_backup_path(&r.pam_file);
        let backup_bytes = fs::read(&backup).unwrap();
        let (result, calls) = enable_with(&r.pam_file, || {});
        result.unwrap();
        assert_eq!(calls, 1, "RemoveRedundant: one re-check before the rename");
        assert_eq!(fs::read_to_string(&r.pam_file).unwrap(), DELEGATING);
        assert_eq!(fs::read(&backup).unwrap(), backup_bytes);
    }

    /// GHF1: the restore message stays byte-identical (VCO4 wording).
    #[test]
    fn test_ghf1_restore_message_is_unchanged() {
        let f = fixture(PRISTINE, 0o644);
        ensure_gdm_pam_line(&f.pam_file).unwrap();
        let pam_file = f.pam_file.clone();
        let err = restore_gdm_pam_file_with(&f.pam_file, false, &mut || {
            append_line(&pam_file);
        })
        .expect_err("a concurrent edit must abort gdm restore");
        assert_eq!(
            err.to_string(),
            format!(
                "GDM configuration error: '{}' changed concurrently while `gdm restore` was \
                 running; nothing was written. Check the file and re-run `soos-admin gdm restore`",
                f.pam_file.display()
            )
        );
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "Unit tests of the backup publication seam use direct assertions"
)]
mod backup_publish_tests {
    //! GitHub #335 (rows GBP1-GBP5): temporary files are named `.<file>.soos-tmp-<pid>-<n>`
    //! with up to 16 fresh names, a stale file (legacy `-<pid>` or `-<pid>-<n>`) never blocks
    //! `gdm enable` / `gdm restore` and is never removed or modified, and a failure after the
    //! backup hard link removes the backup this run created through `discard_created_backup`.

    use super::*;
    use std::collections::BTreeSet;

    const PRISTINE: &str = "\
#%PAM-1.0
auth    requisite       pam_nologin.so
auth    required        pam_unix.so
account required        pam_unix.so
";

    const STALE: &[u8] = b"stale temporary file of a crashed run\n";

    const INJECTED: &str = "injected post-link failure";

    struct Fixture {
        dir: tempfile::TempDir,
        pam_file: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let pam_file = dir.path().join("gdm-password");
        fs::write(&pam_file, PRISTINE).unwrap();
        fs::set_permissions(&pam_file, fs::Permissions::from_mode(0o644)).unwrap();
        Fixture { dir, pam_file }
    }

    fn entries(dir: &Path) -> BTreeSet<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    fn names<S: AsRef<str>>(list: &[S]) -> BTreeSet<String> {
        list.iter().map(|s| s.as_ref().to_string()).collect()
    }

    /// Legacy temporary name of `file_name` (before GitHub #335), with this process's PID.
    fn legacy_tmp(file_name: &str) -> String {
        format!(".{file_name}.soos-tmp-{}", std::process::id())
    }

    /// GitHub #335 temporary name number `n` of `file_name`, with this process's PID.
    fn numbered_tmp(file_name: &str, n: u32) -> String {
        format!(".{file_name}.soos-tmp-{}-{n}", std::process::id())
    }

    /// The stale-file sets each GBP1/GBP2/GBP5 test plants: the legacy name alone (red
    /// against the current code), the `-0` name alone, and both.
    fn stale_sets(file_name: &str) -> Vec<Vec<String>> {
        vec![
            vec![legacy_tmp(file_name)],
            vec![numbered_tmp(file_name, 0)],
            vec![legacy_tmp(file_name), numbered_tmp(file_name, 0)],
        ]
    }

    /// Plants stale files; returns their `(path, inode)` for the byte-identity checks.
    fn plant(dir: &Path, stale: &[String]) -> Vec<(PathBuf, u64)> {
        stale
            .iter()
            .map(|name| {
                let path = dir.join(name);
                fs::write(&path, STALE).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
                let ino = fs::metadata(&path).unwrap().ino();
                (path, ino)
            })
            .collect()
    }

    fn assert_stale_untouched(planted: &[(PathBuf, u64)], context: &str) {
        for (path, ino) in planted {
            let meta = fs::symlink_metadata(path)
                .unwrap_or_else(|e| panic!("{context}: stale {} removed: {e}", path.display()));
            assert!(meta.is_file(), "{context}: {}", path.display());
            assert_eq!(meta.ino(), *ino, "{context}: stale file replaced");
            assert_eq!(
                fs::read(path).unwrap(),
                STALE,
                "{context}: stale file modified"
            );
        }
    }

    fn post_link_error(backup: &Path) -> String {
        format!(
            "GDM configuration error: Failed to write PAM file atomically '{}': {INJECTED}",
            backup.display()
        )
    }

    /// Runs the enable through `ensure_gdm_pam_line_with_hooks`; returns the result and the
    /// call counts of the before-rename and post-link hooks.
    fn enable_with_post_link(
        pam_file: &Path,
        mut post_link: impl FnMut(&Path, &Path) -> std::io::Result<()>,
    ) -> (Result<(), AdminCliError>, u32, u32) {
        let mut before_calls = 0_u32;
        let mut post_calls = 0_u32;
        let result =
            ensure_gdm_pam_line_with_hooks(pam_file, &mut || before_calls += 1, &mut |tmp, dir| {
                post_calls += 1;
                post_link(tmp, dir)
            });
        (result, before_calls, post_calls)
    }

    /// Checks the hook arguments: this run's temporary file of the backup, still present and
    /// holding the pristine bytes, in the PAM directory, with the backup already linked.
    fn assert_hook_args(f_dir: &Path, backup: &Path, tmp: &Path, dir: &Path) {
        assert_eq!(dir, f_dir, "the hook receives the PAM directory");
        assert_eq!(
            tmp.parent(),
            Some(f_dir),
            "own temporary file in the same dir"
        );
        let tmp_name = tmp.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            tmp_name.starts_with(".gdm-password.soos-backup.soos-tmp-"),
            "{tmp_name}"
        );
        assert_eq!(fs::read_to_string(tmp).unwrap(), PRISTINE);
        assert_eq!(fs::read_to_string(backup).unwrap(), PRISTINE);
        assert_eq!(
            fs::metadata(tmp).unwrap().ino(),
            fs::metadata(backup).unwrap().ino(),
            "the hook runs right after the hard link"
        );
    }

    /// GBP1: a stale backup temporary file (legacy `-<pid>`, `-<pid>-0`, or both) never makes
    /// `gdm enable` fail; it is left byte-identical and no temporary file of this run remains.
    #[test]
    fn test_gbp1_stale_backup_temp_file_does_not_block_enable() {
        for stale in stale_sets("gdm-password.soos-backup") {
            let context = format!("stale={stale:?}");
            let f = fixture();
            let planted = plant(f.dir.path(), &stale);
            ensure_gdm_pam_line(&f.pam_file)
                .unwrap_or_else(|e| panic!("{context}: enable must succeed: {e}"));
            assert!(
                fs::read_to_string(&f.pam_file)
                    .unwrap()
                    .contains(GDM_PAM_LINE),
                "{context}"
            );
            assert_eq!(
                fs::read_to_string(pam_backup_path(&f.pam_file)).unwrap(),
                PRISTINE,
                "{context}"
            );
            assert_stale_untouched(&planted, &context);
            let mut expected = vec![
                "gdm-password".to_string(),
                "gdm-password.soos-backup".to_string(),
            ];
            expected.extend(stale.iter().cloned());
            assert_eq!(
                entries(f.dir.path()),
                names(&expected),
                "{context}: no temporary file of this run remains"
            );
        }
    }

    /// GBP1 (exhaustion): with the 16 numbered backup temporary names taken, `gdm enable`
    /// fails with an `AlreadyExists` I/O error, removes nothing and creates no backup.
    #[test]
    fn test_gbp1_exhausted_backup_temp_names_fail_without_removing_anything() {
        let f = fixture();
        let stale: Vec<String> = (0..16)
            .map(|n| numbered_tmp("gdm-password.soos-backup", n))
            .collect();
        let planted = plant(f.dir.path(), &stale);
        let backup = pam_backup_path(&f.pam_file);
        let msg = ensure_gdm_pam_line(&f.pam_file)
            .expect_err("16 taken temporary names must fail the enable")
            .to_string();
        assert!(
            msg.starts_with(&format!(
                "GDM configuration error: Failed to write PAM file atomically '{}': ",
                backup.display()
            )),
            "{msg}"
        );
        let expected_io = std::io::Error::from(std::io::ErrorKind::AlreadyExists).to_string();
        assert!(
            msg.contains("File exists") || msg.contains(&expected_io),
            "the AlreadyExists I/O error is reported: {msg}"
        );
        assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), PRISTINE);
        assert!(!backup.exists(), "no backup is created");
        assert_stale_untouched(&planted, "exhaustion");
        let mut expected = vec!["gdm-password".to_string()];
        expected.extend(stale.iter().cloned());
        assert_eq!(entries(f.dir.path()), names(&expected));
    }

    /// GBP2: a stale PAM-file temporary file (`write_atomic_checked`) never makes
    /// `gdm enable` fail and is left byte-identical.
    #[test]
    fn test_gbp2_stale_pam_temp_file_does_not_block_enable() {
        for stale in stale_sets("gdm-password") {
            let context = format!("stale={stale:?}");
            let f = fixture();
            let planted = plant(f.dir.path(), &stale);
            ensure_gdm_pam_line(&f.pam_file)
                .unwrap_or_else(|e| panic!("{context}: enable must succeed: {e}"));
            assert!(
                fs::read_to_string(&f.pam_file)
                    .unwrap()
                    .contains(GDM_PAM_LINE),
                "{context}"
            );
            assert_eq!(
                fs::read_to_string(pam_backup_path(&f.pam_file)).unwrap(),
                PRISTINE,
                "{context}"
            );
            assert_stale_untouched(&planted, &context);
            let mut expected = vec![
                "gdm-password".to_string(),
                "gdm-password.soos-backup".to_string(),
            ];
            expected.extend(stale.iter().cloned());
            assert_eq!(
                entries(f.dir.path()),
                names(&expected),
                "{context}: no temporary file of this run remains"
            );
        }
    }

    /// GBP3: a post-link failure (no managed block in the PAM file) fails the enable with the
    /// atomic-write error, removes the backup this run created, leaves the PAM file
    /// byte-identical and removes this run's temporary file, whether or not the failing step
    /// removed it; the PAM-file write is never reached.
    #[test]
    fn test_gbp3_post_link_failure_removes_created_backup() {
        for hook_removes_tmp in [true, false] {
            let context = format!("hook_removes_tmp={hook_removes_tmp}");
            let f = fixture();
            let backup = pam_backup_path(&f.pam_file);
            let f_dir = f.dir.path().to_path_buf();
            let hook_backup = backup.clone();
            let (result, before_calls, post_calls) =
                enable_with_post_link(&f.pam_file, |tmp, dir| {
                    assert_hook_args(&f_dir, &hook_backup, tmp, dir);
                    if hook_removes_tmp {
                        fs::remove_file(tmp)?;
                    }
                    Err(std::io::Error::other(INJECTED))
                });
            assert_eq!(post_calls, 1, "{context}: the post-link hook runs once");
            assert_eq!(
                before_calls, 0,
                "{context}: the PAM-file write is not reached"
            );
            let err = result.expect_err("a post-link failure must fail gdm enable");
            assert_eq!(err.to_string(), post_link_error(&backup), "{context}");
            assert_eq!(
                fs::read_to_string(&f.pam_file).unwrap(),
                PRISTINE,
                "{context}"
            );
            assert!(
                !backup.exists(),
                "{context}: the backup created by this run is removed"
            );
            assert_eq!(
                entries(f.dir.path()),
                names(&["gdm-password"]),
                "{context}: no temporary file of this run remains"
            );
        }
    }

    /// GBP3: a pre-existing backup is never removed: the link is never attempted (fast path),
    /// so the post-link hook is not reached and the enable succeeds.
    #[test]
    fn test_gbp3_preexisting_backup_never_reaches_post_link_hook() {
        let f = fixture();
        let backup = pam_backup_path(&f.pam_file);
        fs::write(&backup, "#%PAM-1.0\n# older pristine copy\n").unwrap();
        let ino = fs::metadata(&backup).unwrap().ino();
        let (result, before_calls, post_calls) =
            enable_with_post_link(&f.pam_file, |_tmp, _dir| {
                Err(std::io::Error::other(INJECTED))
            });
        result.expect("an existing backup is kept and the enable proceeds");
        assert_eq!(post_calls, 0, "no link, no post-link step");
        assert_eq!(before_calls, 1);
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "#%PAM-1.0\n# older pristine copy\n"
        );
        assert_eq!(fs::metadata(&backup).unwrap().ino(), ino);
        assert_eq!(
            entries(f.dir.path()),
            names(&["gdm-password", "gdm-password.soos-backup"])
        );
    }

    /// GBP3b: when the post-link step fails after a concurrent `gdm enable` wrote its managed
    /// block, the backup is the one the winner relies on: it is kept byte-identical.
    #[test]
    fn test_gbp3b_post_link_failure_keeps_backup_when_file_holds_managed_block() {
        let f = fixture();
        let backup = pam_backup_path(&f.pam_file);
        let winner = format!(
            "#%PAM-1.0\nauth    requisite       pam_nologin.so\n{GDM_BLOCK_BEGIN}\n\
             {GDM_PAM_LINE}\n{GDM_BLOCK_END}\nauth    required        pam_unix.so\n\
             account required        pam_unix.so\n"
        );
        let pam_file = f.pam_file.clone();
        let winner_bytes = winner.clone();
        let (result, before_calls, post_calls) = enable_with_post_link(&f.pam_file, |tmp, _| {
            fs::remove_file(tmp)?;
            let other = pam_file.with_extension("concurrent-writer");
            fs::write(&other, winner_bytes.as_bytes())?;
            fs::rename(&other, &pam_file)?;
            Err(std::io::Error::other(INJECTED))
        });
        assert_eq!(post_calls, 1);
        assert_eq!(before_calls, 0, "the PAM-file write is not reached");
        let err = result.expect_err("a post-link failure must fail gdm enable");
        assert_eq!(err.to_string(), post_link_error(&backup));
        assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), winner);
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            PRISTINE,
            "the winner's managed block needs the pristine backup: it must be kept"
        );
        assert_eq!(
            entries(f.dir.path()),
            names(&["gdm-password", "gdm-password.soos-backup"])
        );
    }

    /// GBP4: a backup replaced before the post-link failure is not this run's inode: it is
    /// left byte-identical and the error carries the `discard_created_backup` note.
    #[test]
    fn test_gbp4_post_link_failure_leaves_a_replaced_backup_and_reports_it() {
        let f = fixture();
        let backup = pam_backup_path(&f.pam_file);
        let hook_backup = backup.clone();
        let (result, before_calls, post_calls) = enable_with_post_link(&f.pam_file, |tmp, _| {
            fs::remove_file(tmp)?;
            let other = hook_backup.with_extension("concurrent-writer");
            fs::write(&other, b"#%PAM-1.0\n# replaced concurrently\n")?;
            fs::rename(&other, &hook_backup)?;
            Err(std::io::Error::other(INJECTED))
        });
        assert_eq!(post_calls, 1);
        assert_eq!(before_calls, 0, "the PAM-file write is not reached");
        let msg = result
            .expect_err("a post-link failure must fail gdm enable")
            .to_string();
        assert!(msg.starts_with(&post_link_error(&backup)), "{msg}");
        assert!(
            msg.contains("could not be removed: it was replaced"),
            "{msg}"
        );
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "#%PAM-1.0\n# replaced concurrently\n",
            "a backup that is not this run's inode is never removed"
        );
        assert_eq!(fs::read_to_string(&f.pam_file).unwrap(), PRISTINE);
        assert_eq!(
            entries(f.dir.path()),
            names(&["gdm-password", "gdm-password.soos-backup"])
        );
    }

    /// GBP5: a stale PAM-file temporary file never makes `gdm restore` fail and is left
    /// byte-identical.
    #[test]
    fn test_gbp5_stale_pam_temp_file_does_not_block_restore() {
        for stale in stale_sets("gdm-password") {
            let context = format!("stale={stale:?}");
            let f = fixture();
            ensure_gdm_pam_line(&f.pam_file).unwrap();
            let planted = plant(f.dir.path(), &stale);
            restore_gdm_pam_file(&f.pam_file, false)
                .unwrap_or_else(|e| panic!("{context}: restore must succeed: {e}"));
            assert_eq!(
                fs::read_to_string(&f.pam_file).unwrap(),
                PRISTINE,
                "{context}"
            );
            assert!(!pam_backup_path(&f.pam_file).exists(), "{context}");
            assert_stale_untouched(&planted, &context);
            let mut expected = vec!["gdm-password".to_string()];
            expected.extend(stale.iter().cloned());
            assert_eq!(
                entries(f.dir.path()),
                names(&expected),
                "{context}: no temporary file of this run remains"
            );
        }
    }
}
