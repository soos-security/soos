//! Minimal, bounded Linux-PAM service-file analysis used by `soos-admin gdm enable`
//! to place `pam_soos.so` after every pre-credential gate (review finding STO-03,
//! GitHub #177; ADR 2026-09-30 "GDM PAM Stack Placement").
//!
//! Only the `auth` phase is analysed. Nothing here writes to disk.

use std::fs;
use std::io::Read;
use std::path::Path;

use crate::error::AdminCliError;

/// Maximum nesting of `include` / `substack` / `@include` followed while looking for
/// the gates of a delegated auth stack. Deeper chains are refused (fail closed).
pub const MAX_PAM_INCLUDE_DEPTH: usize = 4;

/// Maximum size of any PAM service file read by `soos-admin gdm`.
pub const MAX_PAM_FILE_BYTES: u64 = 64 * 1024;

/// Gate modules copied in front of `pam_soos.so` when a delegated stack runs them
/// before its first credential module (`pam_faillock.so` only with `preauth`).
const GATE_MODULES: &[&str] = &[
    "pam_faillock.so",
    "pam_nologin.so",
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

    /// True for rules evaluated by `pam_authenticate`.
    pub(crate) fn is_auth(&self) -> bool {
        match self {
            Self::AtInclude(_) => true,
            Self::Rule { kind, .. } => *kind == "auth",
        }
    }

    /// Name of the delegated stack for `@include`, `include` and `substack`.
    pub(crate) fn delegation(&self) -> Option<&'a str> {
        match self {
            Self::AtInclude(name) => Some(name),
            Self::Rule {
                control, module, ..
            } if *control == "include" || *control == "substack" => Some(module),
            Self::Rule { .. } => None,
        }
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

    /// Largest numeric jump (`[success=2 ...]`) of the control, if any.
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

    /// Gate rule to copy in front of `pam_soos.so`, normalized
    /// (`auth  <control>  <module> <args>`), when the control is a plain
    /// `required`/`requisite` keyword.
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
        if !GATE_MODULES.contains(&name) || !matches!(*control, "required" | "requisite") {
            return None;
        }
        if name == "pam_faillock.so" && !self.has_arg("preauth") {
            return None;
        }
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

/// Outcome of scanning a delegated stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scan {
    /// The file ended without reaching a credential module: keep scanning the caller.
    Continue,
    /// A credential, conditional or unknown rule was reached: gates end here.
    Stop,
}

/// Collects, in evaluation order, the gate rules that the delegated stack `name`
/// (resolved inside `dir`) runs before its first credential module.
pub(crate) fn delegated_gates(dir: &Path, name: &str) -> Result<Vec<String>, AdminCliError> {
    let mut gates = Vec::new();
    scan_stack(dir, name, 1, &mut gates)?;
    Ok(gates)
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
        // Absolute or unusual include targets are not resolved: gates end here.
        return Ok(Scan::Stop);
    }
    let path = dir.join(name);
    let content = match read_bounded_utf8(&path) {
        Ok(content) => content,
        // A missing include makes Linux-PAM fail the stack anyway: nothing to guard.
        Err(ReadError::NotFound) => return Ok(Scan::Stop),
        Err(ReadError::Other(msg)) => return Err(AdminCliError::GdmConfig(msg)),
    };
    for raw in content.lines() {
        if raw.trim_end().ends_with('\\') {
            return Ok(Scan::Stop);
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
        if let Some(guard) = line.as_guard() {
            if !gates.contains(&guard) {
                gates.push(guard);
            }
            continue;
        }
        if line.is_neutral() {
            continue;
        }
        return Ok(Scan::Stop);
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
pub(crate) fn read_bounded_utf8(path: &Path) -> Result<String, ReadError> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ReadError::NotFound),
        Err(e) => {
            return Err(ReadError::Other(format!(
                "Failed to open PAM file '{}': {e}",
                path.display()
            )))
        }
    };
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
