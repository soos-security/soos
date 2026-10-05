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
use crate::pam_stack::{delegated_auth, read_bounded_utf8, DelegatedAuth, PamLine, ReadError};

pub use crate::pam_stack::{MAX_PAM_FILE_BYTES, MAX_PAM_INCLUDE_DEPTH};

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
    let metadata = regular_file_metadata(pam_file, "PAM file")?;
    let content = match read_bounded_utf8(pam_file) {
        Ok(content) => content,
        Err(ReadError::NotFound) => {
            return Err(AdminCliError::GdmConfig(format!(
                "PAM file '{}' does not exist",
                pam_file.display()
            )))
        }
        Err(ReadError::Other(msg)) => return Err(AdminCliError::GdmConfig(msg)),
    };
    let include_dir = include_dir_of(pam_file);

    let Some(plan) = plan_gdm_enable(&content, include_dir)? else {
        return Ok(());
    };

    // Never propagate group/world write permission to the rewritten file or its backup.
    let mode = metadata.permissions().mode() & 0o7755;
    let owner = (metadata.uid(), metadata.gid());

    match plan {
        EnablePlan::Insert { pristine, updated } => {
            let backup = pam_backup_path(pam_file);
            match fs::symlink_metadata(&backup) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    write_atomic(&backup, pristine.as_bytes(), mode, owner)?;
                }
                Err(e) => return Err(gdm_error("Failed to inspect PAM backup", &backup, &e)),
            }
            write_atomic(pam_file, updated.as_bytes(), mode, owner)
        }
        // GitHub #331: never creates a backup and leaves an existing one byte-identical, so
        // `gdm restore` keeps returning the pristine pre-soos bytes (and keeps refusing a
        // stale backup without `--force`).
        EnablePlan::RemoveRedundant { updated } => {
            write_atomic(pam_file, updated.as_bytes(), mode, owner)
        }
    }
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
    if let Some(Ok(DelegatedAuth::SharedSoosRule { .. })) = delegated {
        if pristine == content {
            return Ok(None);
        }
        if jump_crosses {
            return Err(jump_crossing_error());
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

/// Metadata of `path`, refusing symlinks and non-regular files.
fn regular_file_metadata(path: &Path, what: &str) -> Result<fs::Metadata, AdminCliError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(AdminCliError::GdmConfig(format!(
                "{what} '{}' does not exist",
                path.display()
            )));
        }
        Err(e) => return Err(gdm_error(&format!("Failed to inspect {what}"), path, &e)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AdminCliError::GdmConfig(format!(
            "Refusing to use '{}': not a regular file (symlink or special file)",
            path.display()
        )));
    }
    Ok(metadata)
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
    let snapshot = read_pam_file_snapshot(pam_file)?;
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
            ensure_pam_file_unchanged(pam_file, snapshot.as_ref())
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

/// The PAM file as `gdm restore` compared it: its identity and its exact bytes.
#[derive(Debug, PartialEq, Eq)]
struct PamFileSnapshot {
    dev: u64,
    ino: u64,
    bytes: Vec<u8>,
}

/// Reads `pam_file` through one `O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC` descriptor, bounded to
/// [`MAX_PAM_FILE_BYTES`]; `None` when it does not exist. A symbolic link or a non-regular
/// file is refused.
fn read_pam_file_snapshot(pam_file: &Path) -> Result<Option<PamFileSnapshot>, AdminCliError> {
    let not_regular = || {
        AdminCliError::GdmConfig(format!(
            "Refusing to replace '{}': not a regular file (symlink or special file)",
            pam_file.display()
        ))
    };
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
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
    }))
}

/// Fails with "changed concurrently" unless `pam_file` is still exactly `expected` (same
/// device, inode and bytes), or still absent when `expected` is `None` (GitHub #318).
fn ensure_pam_file_unchanged(
    pam_file: &Path,
    expected: Option<&PamFileSnapshot>,
) -> Result<(), AdminCliError> {
    let current = read_pam_file_snapshot(pam_file);
    if matches!(&current, Ok(now) if now.as_ref() == expected) {
        return Ok(());
    }
    Err(AdminCliError::GdmConfig(format!(
        "'{}' changed concurrently while `gdm restore` was running; nothing was written. \
         Check the file and re-run `soos-admin gdm restore`",
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

/// Opens `backup` with `O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC`, checks on the open
/// descriptor that it is a regular file of at most [`MAX_PAM_FILE_BYTES`], and reads
/// it through that same descriptor (bounded). Returns the bytes and the descriptor's
/// metadata (mode and owner to restore).
fn read_backup_bounded(backup: &Path) -> Result<(Vec<u8>, fs::Metadata), AdminCliError> {
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
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
/// then `fsync` of the directory. The temporary file is removed on failure.
fn write_atomic(
    target: &Path,
    bytes: &[u8],
    mode: u32,
    owner: (u32, u32),
) -> Result<(), AdminCliError> {
    write_atomic_checked(target, bytes, mode, owner, &mut || Ok(()))
}

/// [`write_atomic`] that runs `before_rename` once the temporary file is written and synced,
/// right before the rename; an error from it removes the temporary file and is returned as
/// is, leaving `target` untouched (GitHub #318).
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
    let tmp = dir.join(format!(".{file_name}.soos-tmp-{}", std::process::id()));

    write_and_rename(&tmp, target, dir, bytes, mode, owner, before_rename).map_err(|failure| {
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

fn write_and_rename(
    tmp: &Path,
    target: &Path,
    dir: &Path,
    bytes: &[u8],
    mode: u32,
    owner: (u32, u32),
    before_rename: &mut dyn FnMut() -> Result<(), AdminCliError>,
) -> Result<(), WriteFailure> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(tmp)?;
    file.write_all(bytes)?;
    std::os::unix::fs::fchown(&file, Some(owner.0), Some(owner.1))?;
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    file.sync_all()?;
    drop(file);
    before_rename().map_err(WriteFailure::Refused)?;
    fs::rename(tmp, target)?;
    fs::File::open(dir)?.sync_all()?;
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
