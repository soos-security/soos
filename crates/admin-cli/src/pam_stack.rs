//! Minimal, bounded Linux-PAM service-file analysis used by `soos-admin gdm enable`
//! to place `pam_soos.so` after every pre-credential gate (review finding STO-03,
//! GitHub #177; ADR 2026-09-30 "GDM PAM Stack Placement").
//!
//! Only the `auth` phase is analysed. Nothing here writes to disk.

use std::fs;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use crate::error::AdminCliError;

/// Maximum nesting of `include` / `substack` / `@include` followed while looking for
/// the gates of a delegated auth stack. Deeper chains are refused (fail closed).
pub const MAX_PAM_INCLUDE_DEPTH: usize = 4;

/// Maximum size of any PAM service file read by `soos-admin gdm`.
pub const MAX_PAM_FILE_BYTES: u64 = 64 * 1024;

/// Gate modules copied in front of `pam_soos.so` when a delegated stack (or the
/// edited file after a delegation) runs them before its first credential module,
/// with a plain `required`/`requisite` control (`pam_faillock.so` only with `preauth`).
const GATE_MODULES: &[&str] = &[
    "pam_access.so",
    "pam_faillock.so",
    "pam_listfile.so",
    "pam_nologin.so",
    "pam_securetty.so",
    "pam_shells.so",
    "pam_succeed_if.so",
];

/// Modules that neither verify a credential nor gate the login; scanning continues.
const NEUTRAL_MODULES: &[&str] = &["pam_env.so", "pam_faildelay.so"];

/// Modules that may precede the insertion point in the edited file itself.
const PRE_CREDENTIAL_MODULES: &[&str] = &[
    "pam_access.so",
    "pam_env.so",
    "pam_faildelay.so",
    "pam_faillock.so",
    "pam_listfile.so",
    "pam_nologin.so",
    "pam_securetty.so",
    "pam_selinux_permit.so",
    "pam_shells.so",
    "pam_succeed_if.so",
];

/// Modules that verify a credential (password, Kerberos/LDAP/SSSD/Winbind account,
/// systemd-homed secret, fingerprint). `pam_soos.so` is inserted immediately before
/// the first of them (or before the delegation leading to it). Any other auth rule
/// met before this anchor, outside [`PRE_CREDENTIAL_MODULES`], is unclassified and
/// makes `gdm enable` refuse (ADR 2026-09-30 "GDM PAM Stack Placement").
pub const CREDENTIAL_MODULES: &[&str] = &[
    "pam_unix.so",
    "pam_sss.so",
    "pam_ldap.so",
    "pam_krb5.so",
    "pam_winbind.so",
    "pam_systemd_home.so",
    "pam_fprintd.so",
];

/// One active PAM rule of a service file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PamLine<'a> {
    /// `@include <name>` (Debian style, every phase).
    AtInclude(&'a str),
    /// `[-]<type> <control> <module> [args...]`.
    Rule {
        kind: &'a str,
        control: &'a str,
        module: &'a str,
        args: Vec<&'a str>,
    },
}

impl<'a> PamLine<'a> {
    /// Parses one line; `None` for blank lines, comments and malformed lines.
    pub(crate) fn parse(line: &'a str) -> Option<Self> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        if let Some(rest) = line.strip_prefix("@include") {
            let name = rest.trim();
            return (!name.is_empty() && rest.starts_with(char::is_whitespace))
                .then_some(Self::AtInclude(name));
        }
        let (kind, rest) = line.split_once(char::is_whitespace)?;
        let rest = rest.trim_start();
        let (control, rest) = if rest.starts_with('[') {
            let (inside, after) = rest.split_once(']')?;
            let control = rest.get(..inside.len().checked_add(1)?)?;
            (control, after.trim_start())
        } else {
            let (control, after) = rest.split_once(char::is_whitespace)?;
            (control, after.trim_start())
        };
        let mut words = rest.split_whitespace();
        let module = words.next()?;
        Some(Self::Rule {
            kind: kind.strip_prefix('-').unwrap_or(kind),
            control,
            module,
            args: words.collect(),
        })
    }

    /// True for rules evaluated by `pam_authenticate`. The type keyword is
    /// case-insensitive, as in libpam (pam.conf(5)).
    pub(crate) fn is_auth(&self) -> bool {
        match self {
            Self::AtInclude(_) => true,
            Self::Rule { kind, .. } => kind.eq_ignore_ascii_case("auth"),
        }
    }

    /// Name of the delegated stack for `@include`, `include` and `substack`
    /// (control keyword compared case-insensitively; the target name is not).
    pub(crate) fn delegation(&self) -> Option<&'a str> {
        match self {
            Self::AtInclude(name) => Some(name),
            Self::Rule {
                control, module, ..
            } if control.eq_ignore_ascii_case("include")
                || control.eq_ignore_ascii_case("substack") =>
            {
                Some(module)
            }
            Self::Rule { .. } => None,
        }
    }

    /// True when the control is a plain `required` or `requisite` keyword
    /// (case-insensitive): the rule enforces its verdict on the whole stack.
    pub(crate) fn is_enforcing(&self) -> bool {
        matches!(self, Self::Rule { control, .. }
            if control.eq_ignore_ascii_case("required")
                || control.eq_ignore_ascii_case("requisite"))
    }

    /// Basename of the module (`/lib/security/pam_unix.so` -> `pam_unix.so`).
    pub(crate) fn module_name(&self) -> Option<&'a str> {
        match self {
            Self::Rule { module, .. } if self.delegation().is_none() => {
                Some(module.rsplit('/').next().unwrap_or(module))
            }
            _ => None,
        }
    }

    /// Largest numeric jump (`[success=2 ...]`) of the control, if any. Every
    /// `key=value` pair is inspected whatever the case of its key.
    pub(crate) fn max_jump(&self) -> Option<usize> {
        let Self::Rule { control, .. } = self else {
            return None;
        };
        let inside = control.strip_prefix('[')?.strip_suffix(']')?;
        inside
            .split_whitespace()
            .filter_map(|pair| pair.split_once('=').map(|(_, v)| v))
            .filter_map(|v| v.parse::<usize>().ok())
            .max()
    }

    fn has_arg(&self, wanted: &str) -> bool {
        matches!(self, Self::Rule { args, .. } if args.contains(&wanted))
    }

    /// True when the rule may sit before the insertion point of the edited file.
    pub(crate) fn is_pre_credential(&self) -> bool {
        let Some(name) = self.module_name() else {
            return false;
        };
        if name == "pam_faillock.so" {
            return self.has_arg("preauth");
        }
        PRE_CREDENTIAL_MODULES.contains(&name)
    }

    /// True when the rule is one of the [`CREDENTIAL_MODULES`].
    pub(crate) fn is_credential(&self) -> bool {
        self.module_name()
            .is_some_and(|name| CREDENTIAL_MODULES.contains(&name))
    }

    /// Operator-facing description of the rule (`<control> <module>`).
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::AtInclude(name) => format!("@include {name}"),
            Self::Rule {
                control, module, ..
            } => format!("'{module}' (control '{control}')"),
        }
    }

    /// Gate rule to copy in front of `pam_soos.so`, normalized
    /// (`auth  <control>  <module> <args>`, keywords in lowercase), when the control
    /// is a plain `required`/`requisite` keyword.
    fn as_guard(&self) -> Option<String> {
        let Self::Rule {
            control,
            module,
            args,
            ..
        } = self
        else {
            return None;
        };
        let name = self.module_name()?;
        if !GATE_MODULES.contains(&name) || !self.is_enforcing() {
            return None;
        }
        if name == "pam_faillock.so" && !self.has_arg("preauth") {
            return None;
        }
        let control = control.to_ascii_lowercase();
        let mut guard = format!("auth  {control}  {module}");
        for arg in args {
            guard.push(' ');
            guard.push_str(arg);
        }
        Some(guard)
    }

    fn is_neutral(&self) -> bool {
        self.module_name()
            .is_some_and(|name| NEUTRAL_MODULES.contains(&name))
    }
}

/// Outcome of scanning an auth stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scan {
    /// The lines ended without reaching a credential module: keep scanning the caller.
    Continue,
    /// A credential module was reached: gates end here.
    Stop,
}

/// Collects, in evaluation order, the gate rules run by `lines` (the edited file from
/// its delegating anchor onward, resolved inside `dir`) before the first credential
/// module.
///
/// # Errors
///
/// Refuses (fails closed) when an unclassified rule, a conditional or non-plain gate,
/// an unresolvable include target, an include chain deeper than
/// [`MAX_PAM_INCLUDE_DEPTH`] or line continuations are met before the credential
/// module, and when no credential module is reached at all.
pub(crate) fn delegated_gates(dir: &Path, lines: &[&str]) -> Result<Vec<String>, AdminCliError> {
    let mut gates = Vec::new();
    match scan_lines(
        dir,
        "the edited PAM file",
        lines.iter().copied(),
        0,
        &mut gates,
    )? {
        Scan::Stop => Ok(gates),
        Scan::Continue => Err(AdminCliError::GdmConfig(
            "the auth stack reaches no known credential module (pam_unix.so, pam_sss.so, ...); \
             refusing to guess where pam_soos.so belongs"
                .into(),
        )),
    }
}

fn scan_stack(
    dir: &Path,
    name: &str,
    depth: usize,
    gates: &mut Vec<String>,
) -> Result<Scan, AdminCliError> {
    if depth > MAX_PAM_INCLUDE_DEPTH {
        return Err(AdminCliError::GdmConfig(format!(
            "PAM include chain deeper than {MAX_PAM_INCLUDE_DEPTH} levels at '{name}'; refusing to guess the stack order"
        )));
    }
    let valid_name = !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !valid_name {
        return Err(AdminCliError::GdmConfig(format!(
            "cannot resolve the included auth stack '{name}' inside the PAM directory; \
             refusing to guess which gates it runs"
        )));
    }
    let path = dir.join(name);
    let content = match read_bounded_utf8(&path) {
        Ok(content) => content,
        // Fail closed: a target missing from the PAM directory may be resolved by
        // libpam from a vendor directory (never searched here) or make the stack
        // fail; either way its gates are unknown.
        Err(ReadError::NotFound) => {
            return Err(AdminCliError::GdmConfig(format!(
                "the included auth stack '{name}' was not found inside the PAM directory \
                 (vendor directories such as /usr/lib/pam.d are not searched); refusing to \
                 guess which gates it runs (see Docs/DISTRIBUTION_DEPLOYMENT.md section 2.1)"
            )))
        }
        Err(ReadError::Other(msg)) => return Err(AdminCliError::GdmConfig(msg)),
    };
    let label = format!("the shared auth stack '{name}'");
    scan_lines(dir, &label, content.lines(), depth, gates)
}

fn scan_lines<'a>(
    dir: &Path,
    label: &str,
    lines: impl Iterator<Item = &'a str>,
    depth: usize,
    gates: &mut Vec<String>,
) -> Result<Scan, AdminCliError> {
    for raw in lines {
        if raw.trim_end().ends_with('\\') {
            return Err(AdminCliError::GdmConfig(format!(
                "{label} uses line continuations; refusing to guess the stack order"
            )));
        }
        let Some(line) = PamLine::parse(raw) else {
            continue;
        };
        if !line.is_auth() {
            continue;
        }
        if let Some(target) = line.delegation() {
            let next = depth.saturating_add(1);
            if scan_stack(dir, target, next, gates)? == Scan::Stop {
                return Ok(Scan::Stop);
            }
            continue;
        }
        if line.is_credential() {
            return Ok(Scan::Stop);
        }
        if let Some(guard) = line.as_guard() {
            if !gates.contains(&guard) {
                gates.push(guard);
            }
            continue;
        }
        if line.is_neutral() {
            continue;
        }
        return Err(AdminCliError::GdmConfig(format!(
            "{label} runs the unclassified auth rule {} before its credential module; \
             soos cannot tell whether it is a lockout or login gate, so pam_soos.so is not \
             inserted automatically (see Docs/DISTRIBUTION_DEPLOYMENT.md section 2.1)",
            line.describe()
        )));
    }
    Ok(Scan::Continue)
}

/// Error of [`read_bounded_utf8`].
pub(crate) enum ReadError {
    /// The file does not exist.
    NotFound,
    /// Any other failure, already formatted for the operator.
    Other(String),
}

/// Reads at most [`MAX_PAM_FILE_BYTES`] of a UTF-8 PAM file.
///
/// The file is opened with `O_NONBLOCK | O_CLOEXEC` (a FIFO never blocks the open)
/// and its type is checked on the open descriptor: anything but a regular file is
/// refused before a single byte is read (GitHub #278). Symlinks are followed, as
/// libpam does (authselect ships `system-auth` as a symlink).
pub(crate) fn read_bounded_utf8(path: &Path) -> Result<String, ReadError> {
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ReadError::NotFound),
        Err(e) => {
            return Err(ReadError::Other(format!(
                "Failed to open PAM file '{}': {e}",
                path.display()
            )))
        }
    };
    let metadata = file.metadata().map_err(|e| {
        ReadError::Other(format!(
            "Failed to inspect PAM file '{}': {e}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(ReadError::Other(format!(
            "Refusing to read PAM file '{}': not a regular file (FIFO, device, socket or directory)",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    file.take(MAX_PAM_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| {
            ReadError::Other(format!("Failed to read PAM file '{}': {e}", path.display()))
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_PAM_FILE_BYTES {
        return Err(ReadError::Other(format!(
            "PAM file '{}' exceeds {MAX_PAM_FILE_BYTES} bytes",
            path.display()
        )));
    }
    String::from_utf8(bytes).map_err(|_| {
        ReadError::Other(format!(
            "PAM file '{}' is not valid UTF-8; refusing to edit it",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "unit tests use direct assertions"
    )]
    use super::*;

    #[test]
    fn test_parse_bracket_control_and_optional_type() {
        let line = PamLine::parse("-auth [success=2 default=ignore] pam_systemd_home.so").unwrap();
        assert!(line.is_auth());
        assert_eq!(line.module_name(), Some("pam_systemd_home.so"));
        assert_eq!(line.max_jump(), Some(2));
        assert_eq!(line.delegation(), None);
    }

    #[test]
    fn test_parse_include_forms() {
        assert_eq!(
            PamLine::parse("@include common-auth").and_then(|l| l.delegation()),
            Some("common-auth")
        );
        assert_eq!(
            PamLine::parse("auth substack password-auth").and_then(|l| l.delegation()),
            Some("password-auth")
        );
        assert_eq!(PamLine::parse("# auth required pam_unix.so"), None);
        assert_eq!(PamLine::parse("@includecommon-auth"), None);
    }

    #[test]
    fn test_guard_requires_plain_control_and_preauth() {
        let preauth = PamLine::parse("auth required pam_faillock.so preauth silent").unwrap();
        assert_eq!(
            preauth.as_guard().as_deref(),
            Some("auth  required  pam_faillock.so preauth silent")
        );
        let authfail = PamLine::parse("auth [default=die] pam_faillock.so authfail").unwrap();
        assert_eq!(authfail.as_guard(), None);
        let jump =
            PamLine::parse("auth [success=1 default=ignore] pam_succeed_if.so uid < 1000").unwrap();
        assert_eq!(jump.as_guard(), None, "conditional gates are never copied");
    }
}
