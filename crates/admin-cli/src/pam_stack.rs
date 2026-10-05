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

/// Maximum number of included stack files opened by one delegated-stack analysis (one
/// `gdm enable` or one `gdm status`), in addition to [`MAX_PAM_INCLUDE_DEPTH`]; reaching the
/// next open refuses (fail closed). GitHub #333.
pub const MAX_PAM_STACK_READS: usize = 32;

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

    /// True for an active `auth` rule of `pam_soos.so` (bare name or path) that can grant the
    /// whole auth phase on a face match and is driven by `PAM_SERVICE` (GitHub #331):
    /// - no argument starting with `event=` (the password-failed hook is not primary) and no
    ///   argument starting with `service=` (the `gdm.disable` flag must keep applying);
    /// - control `sufficient` (case-insensitive), or a bracketed control whose `success` value
    ///   is `done` or a decimal jump `N >= 1`, and whose every other `key=value` action is
    ///   `ignore` (keys and values compared case-insensitively).
    ///
    /// Anything else (`required`, `requisite`, `optional`, `success=ok`, `default=die`, a
    /// duplicate or missing `success`, ...) is not primary. The caller has already checked
    /// `is_auth()`.
    pub(crate) fn is_primary_soos_rule(&self) -> bool {
        let Self::Rule { control, args, .. } = self else {
            return false;
        };
        if self.module_name() != Some("pam_soos.so") {
            return false;
        }
        if args
            .iter()
            .any(|arg| arg.starts_with("event=") || arg.starts_with("service="))
        {
            return false;
        }
        if control.eq_ignore_ascii_case("sufficient") {
            return true;
        }
        let Some(inside) = control
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        else {
            return false;
        };
        let mut success_seen = false;
        for token in inside.split_whitespace() {
            let Some((key, value)) = token.split_once('=') else {
                return false;
            };
            if key.eq_ignore_ascii_case("success") {
                if success_seen || !is_primary_success_action(value) {
                    return false;
                }
                success_seen = true;
            } else if !value.eq_ignore_ascii_case("ignore") {
                return false;
            }
        }
        success_seen
    }
}

impl PamLine<'_> {
    /// GitHub #333: the decimal jump `N >= 1` of the `success` action of a primary soos rule
    /// (`None` for `sufficient` or `success=done`). Only meaningful when `is_primary_soos_rule()`.
    pub(crate) fn primary_success_jump(&self) -> Option<usize> {
        let Self::Rule { control, .. } = self else {
            return None;
        };
        let inside = control.strip_prefix('[')?.strip_suffix(']')?;
        inside
            .split_whitespace()
            .filter_map(|token| token.split_once('='))
            .find(|(key, _)| key.eq_ignore_ascii_case("success"))
            .and_then(|(_, value)| {
                if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                value.parse::<usize>().ok().filter(|jump| *jump >= 1)
            })
    }
}

/// GitHub #333: true when a `[success=n]` jump taken by a rule of a shared stack file lands on
/// an auth rule of that same file, `rest` being the raw lines of the file after the rule.
///
/// The jump counts the following auth handlers (`-auth` included: a rule whose module is
/// missing stays in the chain); `account`, `password` and `session` rules, comments and blank
/// lines are skipped. A continued line, a malformed line, an unknown type keyword or an auth
/// delegation (`include`, `substack`, `@include`) inside the span makes the target
/// unverifiable (`false`), as does the end of the file: libpam turns a jump past the end of
/// the chain into `PAM_PERM_DENIED` ("bad jump in stack").
fn jump_lands_in_stack(rest: &[&str], n: usize) -> bool {
    let mut skipped = 0_usize;
    for raw in rest {
        if raw.trim_end().ends_with('\\') {
            return false;
        }
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(line) = PamLine::parse(raw) else {
            return false;
        };
        // `@include` splices every group, auth included.
        let PamLine::Rule { kind, .. } = &line else {
            return false;
        };
        if ["account", "password", "session"]
            .iter()
            .any(|other| kind.eq_ignore_ascii_case(other))
        {
            continue;
        }
        if !kind.eq_ignore_ascii_case("auth") || line.delegation().is_some() {
            return false;
        }
        if skipped == n {
            return true;
        }
        skipped = match skipped.checked_add(1) {
            Some(next) => next,
            None => return false,
        };
    }
    false
}

/// Number of stack files one [`delegated_auth`] analysis may still open (GitHub #333).
struct ScanBudget {
    reads_left: usize,
}

/// `done`, or a decimal jump `N >= 1` written with digits only (no sign, not empty).
fn is_primary_success_action(value: &str) -> bool {
    if value.eq_ignore_ascii_case("done") {
        return true;
    }
    !value.is_empty()
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<usize>().is_ok_and(|jump| jump >= 1)
}

/// Outcome of scanning an auth stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Scan {
    /// The lines ended without reaching a credential module: keep scanning the caller.
    Continue,
    /// A credential module was reached: gates end here.
    Stop,
    /// A primary `pam_soos.so` rule was reached first (GitHub #331); payload = stack name.
    SharedSoos(String),
}

/// What the auth stack delegated from the GDM file runs before its first credential module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DelegatedAuth {
    /// Gate rules to copy in front of the managed block.
    Gates(Vec<String>),
    /// The delegated stack reaches an active primary `pam_soos.so` rule (in the named stack
    /// file) before any credential module: GDM already authenticates through it.
    SharedSoosRule {
        /// Name of the stack file holding the shared rule (a validated include target).
        stack: String,
    },
}

/// Scans `lines` (the edited file from its delegating anchor onward, resolved inside `dir`)
/// in evaluation order up to the first credential module or primary `pam_soos.so` rule,
/// collecting the gate rules met before it.
///
/// # Errors
///
/// Refuses (fails closed) when an unclassified rule, a conditional or non-plain gate,
/// an unresolvable include target, an include chain deeper than
/// [`MAX_PAM_INCLUDE_DEPTH`], more than [`MAX_PAM_STACK_READS`] stack files opened in total,
/// a shared `[success=N]` soos rule whose jump target is not verifiable, or line
/// continuations are met before the credential module, and when no credential module is
/// reached at all.
pub(crate) fn delegated_auth(dir: &Path, lines: &[&str]) -> Result<DelegatedAuth, AdminCliError> {
    let mut gates = Vec::new();
    let mut budget = ScanBudget {
        reads_left: MAX_PAM_STACK_READS,
    };
    match scan_lines(
        dir,
        "the edited PAM file",
        None,
        lines,
        0,
        &mut gates,
        &mut budget,
    )? {
        Scan::Stop => Ok(DelegatedAuth::Gates(gates)),
        Scan::SharedSoos(stack) => Ok(DelegatedAuth::SharedSoosRule { stack }),
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
    budget: &mut ScanBudget,
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
    // GitHub #333: every open counts (a missing or refused target too, a repeated include
    // twice), so the analysis reads at most MAX_PAM_STACK_READS files whatever its width.
    budget.reads_left = match budget.reads_left.checked_sub(1) {
        Some(left) => left,
        None => {
            return Err(AdminCliError::GdmConfig(format!(
                "the auth stack includes more than {MAX_PAM_STACK_READS} stack files \
                 (limit MAX_PAM_STACK_READS) at '{name}'; refusing to guess the stack order"
            )))
        }
    };
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
    let lines: Vec<&str> = content.lines().collect();
    scan_lines(dir, &label, Some(name), &lines, depth, gates, budget)
}

/// `stack` is the validated name of the stack file being scanned (`None` for the edited
/// file itself, where a primary soos rule is unreachable and stays unclassified).
fn scan_lines(
    dir: &Path,
    label: &str,
    stack: Option<&str>,
    lines: &[&str],
    depth: usize,
    gates: &mut Vec<String>,
    budget: &mut ScanBudget,
) -> Result<Scan, AdminCliError> {
    for (index, raw) in lines.iter().enumerate() {
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
            match scan_stack(dir, target, next, gates, budget)? {
                Scan::Continue => continue,
                decisive => return Ok(decisive),
            }
        }
        if line.is_credential() {
            return Ok(Scan::Stop);
        }
        if let Some(name) = stack {
            if line.is_primary_soos_rule() {
                if let Some(n) = line.primary_success_jump() {
                    let rest = lines.get(index.saturating_add(1)..).unwrap_or_default();
                    if !jump_lands_in_stack(rest, n) {
                        return Err(AdminCliError::GdmConfig(format!(
                            "the shared auth stack '{name}' runs pam_soos.so with a \
                             [success={n}] jump that does not land on a rule of the same file \
                             (it runs past the end of the file or over an include, substack, \
                             @include, malformed or continued line); soos cannot tell where a \
                             face match leads, so pam_soos.so is not inserted automatically \
                             (see Docs/DISTRIBUTION_DEPLOYMENT.md section 2.1)"
                        )));
                    }
                }
                return Ok(Scan::SharedSoos(name.to_string()));
            }
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
/// The file is opened with `O_NOCTTY | O_NONBLOCK | O_CLOEXEC` (a FIFO never blocks the
/// open, a terminal never becomes the controlling terminal)
/// and its type is checked on the open descriptor: anything but a regular file is
/// refused before a single byte is read (GitHub #278). Symlinks are followed, as
/// libpam does (authselect ships `system-auth` as a symlink).
pub(crate) fn read_bounded_utf8(path: &Path) -> Result<String, ReadError> {
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC)
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
        clippy::arithmetic_side_effects,
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

    // ------------------------------------------------------------------
    // GitHub #331 (matrix IGF17): primary pam_soos.so rule classification
    // ------------------------------------------------------------------

    fn primary(line: &str) -> bool {
        let rule = PamLine::parse(line).unwrap();
        assert!(rule.is_auth(), "fixture {line:?} must be an auth rule");
        rule.is_primary_soos_rule()
    }

    #[test]
    fn test_igf17_packaged_soos_rules_are_primary() {
        for line in [
            "auth  [success=4 default=ignore]  pam_soos.so",
            "auth\t[success=done default=ignore]\tpam_soos.so",
            "auth\t[success=2 default=ignore]\tpam_soos.so",
            "auth        [success=done default=ignore]                pam_soos.so",
            "auth sufficient pam_soos.so",
        ] {
            assert!(primary(line), "{line:?} must be primary");
        }
    }

    #[test]
    fn test_igf17_qualifying_edge_forms_are_primary() {
        for line in [
            "auth SUFFICIENT pam_soos.so",
            "auth [Success=DONE Default=Ignore] pam_soos.so",
            "auth [success=done ignore=ignore default=ignore] pam_soos.so",
            "auth [success=done default=ignore] /usr/lib/security/pam_soos.so",
            "-auth [success=1 default=ignore] pam_soos.so timeout_ms=1500",
            "auth [success=done default=ignore] pam_soos.so debug",
        ] {
            assert!(primary(line), "{line:?} must be primary");
        }
    }

    #[test]
    fn test_igf17_non_primary_soos_rules_are_rejected() {
        for line in [
            "auth optional pam_soos.so event=password-failed timeout_ms=20",
            "auth [success=done default=ignore] pam_soos.so event=password-failed",
            "auth [success=done default=ignore] pam_soos.so service=sudo",
            "auth sufficient pam_soos.so service=gdm-password",
            "auth optional pam_soos.so",
            "auth required pam_soos.so",
            "auth requisite pam_soos.so",
            "auth [success=ok default=ignore] pam_soos.so",
            "auth [success=done default=die] pam_soos.so",
            "auth [success=done default=bad] pam_soos.so",
            "auth [success=0 default=ignore] pam_soos.so",
            "auth [success=-1 default=ignore] pam_soos.so",
            "auth [success= default=ignore] pam_soos.so",
            "auth [default=ignore] pam_soos.so",
            "auth [success=done success=bad default=ignore] pam_soos.so",
            "auth [success=done success=2 default=ignore] pam_soos.so",
            "auth [success=done new_authtok_reqd=done default=ignore] pam_soos.so",
            "auth sufficient pam_unix.so",
            "auth [success=done default=ignore] pam_soos_other.so",
        ] {
            assert!(!primary(line), "{line:?} must not be primary");
        }
    }

    // ------------------------------------------------------------------
    // GitHub #331 (matrix IGF14/IGF18): delegated_auth outcome
    // ------------------------------------------------------------------

    fn pam_dir(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            fs::write(dir.path().join(name), content).unwrap();
        }
        dir
    }

    #[test]
    fn test_igf14_delegated_auth_reports_the_stack_of_the_shared_rule() {
        let dir = pam_dir(&[
            ("outer", "auth required pam_shells.so\nauth include inner\n"),
            (
                "inner",
                "auth required pam_faillock.so preauth\n\
                 auth [success=4 default=ignore] pam_soos.so\n\
                 auth [success=1 default=bad] pam_unix.so\n\
                 auth optional pam_soos.so event=password-failed timeout_ms=20\n\
                 auth [default=die] pam_faillock.so authfail\n\
                 auth optional pam_permit.so\n\
                 auth required pam_env.so\n",
            ),
        ]);
        let lines = [
            "auth include outer\n",
            "auth optional pam_gnome_keyring.so\n",
        ];
        assert_eq!(
            delegated_auth(dir.path(), &lines).unwrap(),
            DelegatedAuth::SharedSoosRule {
                stack: "inner".to_string()
            }
        );
    }

    #[test]
    fn test_igf14_delegated_auth_without_soos_rule_returns_gates() {
        let dir = pam_dir(&[(
            "inner",
            "auth required pam_faillock.so preauth\nauth sufficient pam_unix.so\n",
        )]);
        let lines = ["auth include inner\n"];
        assert_eq!(
            delegated_auth(dir.path(), &lines).unwrap(),
            DelegatedAuth::Gates(vec!["auth  required  pam_faillock.so preauth".to_string()])
        );
    }

    #[test]
    fn test_igf18_delegated_auth_credential_before_soos_rule_is_not_shared() {
        let dir = pam_dir(&[(
            "inner",
            "auth sufficient pam_unix.so\nauth [success=done default=ignore] pam_soos.so\n",
        )]);
        let lines = ["auth include inner\n"];
        assert_eq!(
            delegated_auth(dir.path(), &lines).unwrap(),
            DelegatedAuth::Gates(Vec::new())
        );
    }

    #[test]
    fn test_igf18_delegated_auth_keeps_refusals() {
        let unclassified = pam_dir(&[(
            "inner",
            "auth required pam_mystery.so\nauth [success=done default=ignore] pam_soos.so\n",
        )]);
        assert!(delegated_auth(unclassified.path(), &["auth include inner\n"]).is_err());

        let non_primary = pam_dir(&[(
            "inner",
            "auth optional pam_soos.so\nauth sufficient pam_unix.so\n",
        )]);
        assert!(delegated_auth(non_primary.path(), &["auth include inner\n"]).is_err());

        let missing = pam_dir(&[]);
        assert!(delegated_auth(missing.path(), &["auth include inner\n"]).is_err());

        let no_credential = pam_dir(&[("inner", "auth required pam_env.so\n")]);
        assert!(delegated_auth(no_credential.path(), &["auth include inner\n"]).is_err());
    }

    #[test]
    fn test_igf18_delegated_auth_keeps_the_depth_bound() {
        // d0 -> d1 -> ... -> d5: the shared rule sits beyond MAX_PAM_INCLUDE_DEPTH.
        let files: Vec<(String, String)> = (0..=MAX_PAM_INCLUDE_DEPTH)
            .map(|i| (format!("d{i}"), format!("auth include d{}\n", i + 1)))
            .chain(std::iter::once((
                format!("d{}", MAX_PAM_INCLUDE_DEPTH + 1),
                "auth [success=done default=ignore] pam_soos.so\nauth sufficient pam_unix.so\n"
                    .to_string(),
            )))
            .collect();
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(n, c)| (n.as_str(), c.as_str()))
            .collect();
        let dir = pam_dir(&refs);
        assert!(delegated_auth(dir.path(), &["auth include d0\n"]).is_err());
    }
}

#[cfg(test)]
mod ghf_tests {
    //! GitHub #333: total read budget of the delegated-stack analysis (row GHF3) and the
    //! structural check of a shared `[success=N]` jump target (row GHF5).
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::arithmetic_side_effects,
        reason = "unit tests use direct assertions"
    )]
    use super::*;

    fn pam_dir(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            fs::write(dir.path().join(name), content).unwrap();
        }
        dir
    }

    /// The edited file's delegated lines: `count` includes of `leaf`, then `tail`.
    fn includes(count: usize, leaf: &str, tail: &str) -> Vec<String> {
        let mut lines: Vec<String> = (0..count)
            .map(|_| format!("auth include {leaf}\n"))
            .collect();
        lines.push(tail.to_string());
        lines
    }

    fn run(dir: &tempfile::TempDir, lines: &[String]) -> Result<DelegatedAuth, AdminCliError> {
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        delegated_auth(dir.path(), &refs)
    }

    fn shared(stack: &str) -> DelegatedAuth {
        DelegatedAuth::SharedSoosRule {
            stack: stack.to_string(),
        }
    }

    // ------------------------------------------------------------------
    // GHF3 — MAX_PAM_STACK_READS
    // ------------------------------------------------------------------

    #[test]
    fn test_ghf3_max_pam_stack_reads_is_32() {
        assert_eq!(MAX_PAM_STACK_READS, 32);
    }

    /// GHF3: 32 sibling includes of a neutral leaf, then a credential: accepted.
    #[test]
    fn test_ghf3_delegated_auth_accepts_32_stack_reads() {
        let dir = pam_dir(&[("leaf", "auth required pam_env.so\n")]);
        let lines = includes(MAX_PAM_STACK_READS, "leaf", "auth sufficient pam_unix.so\n");
        assert_eq!(run(&dir, &lines).unwrap(), DelegatedAuth::Gates(Vec::new()));
    }

    /// GHF3: the 33rd open is refused with E3, whatever the width or repetition.
    #[test]
    fn test_ghf3_delegated_auth_refuses_the_33rd_stack_read() {
        let dir = pam_dir(&[("leaf", "auth required pam_env.so\n")]);
        let lines = includes(
            MAX_PAM_STACK_READS + 1,
            "leaf",
            "auth sufficient pam_unix.so\n",
        );
        let err = run(&dir, &lines).expect_err("33 stack reads must be refused");
        let msg = err.to_string();
        assert!(msg.contains("more than 32 stack files"), "{msg}");
        assert!(msg.contains("MAX_PAM_STACK_READS"), "{msg}");
        assert!(msg.contains("refusing to guess the stack order"), "{msg}");
    }

    /// GHF3: nested reads count too (a leaf including another file costs two opens), and a
    /// shared rule reached on the 33rd open is refused (fail closed, never "installed").
    #[test]
    fn test_ghf3_nested_reads_count_and_a_late_shared_rule_is_refused() {
        let dir = pam_dir(&[
            ("pair", "auth include leaf\n"),
            ("leaf", "auth required pam_env.so\n"),
            (
                "soos",
                "auth [success=done default=ignore] pam_soos.so\nauth sufficient pam_unix.so\n",
            ),
        ]);
        // 16 x (pair -> leaf) = 32 opens, then the shared stack would be the 33rd.
        let mut lines = includes(16, "pair", "auth include soos\n");
        assert!(
            run(&dir, &lines).is_err(),
            "the shared stack opened 33rd must be refused"
        );
        // 15 x 2 = 30 opens, then the shared stack is the 31st: accepted.
        lines = includes(15, "pair", "auth include soos\n");
        assert_eq!(run(&dir, &lines).unwrap(), shared("soos"));
    }

    // ------------------------------------------------------------------
    // GHF5 — target of a shared [success=N] rule
    // ------------------------------------------------------------------

    #[test]
    fn test_ghf5_primary_success_jump_reads_the_decimal_jump() {
        let jump = |line: &str| PamLine::parse(line).unwrap().primary_success_jump();
        assert_eq!(jump("auth [success=4 default=ignore] pam_soos.so"), Some(4));
        assert_eq!(jump("auth [Success=2 Default=Ignore] pam_soos.so"), Some(2));
        assert_eq!(
            jump("-auth [default=ignore success=1] pam_soos.so timeout_ms=1500"),
            Some(1)
        );
        assert_eq!(jump("auth [success=done default=ignore] pam_soos.so"), None);
        assert_eq!(jump("auth sufficient pam_soos.so"), None);
    }

    fn e5(dir: &tempfile::TempDir, n: usize) {
        let err = delegated_auth(dir.path(), &["auth include inner\n"])
            .expect_err("a jump target outside the file must be refused (E5)");
        let msg = err.to_string();
        assert!(msg.contains(&format!("[success={n}]")), "{msg}");
        assert!(
            msg.contains("does not land on a rule of the same file"),
            "{msg}"
        );
        assert!(msg.contains("the shared auth stack 'inner'"), "{msg}");
    }

    fn accepted(dir: &tempfile::TempDir) {
        assert_eq!(
            delegated_auth(dir.path(), &["auth include inner\n"]).unwrap(),
            shared("inner")
        );
    }

    /// GHF5: the original IGF14 fixture (before OA-1): the jump runs past the end of the
    /// chain (libpam "bad jump", PAM_PERM_DENIED).
    #[test]
    fn test_ghf5_jump_past_the_end_of_the_file_is_refused() {
        let dir = pam_dir(&[(
            "inner",
            "auth required pam_faillock.so preauth\n\
             auth [success=4 default=ignore] pam_soos.so\n\
             auth [success=1 default=bad] pam_unix.so\n",
        )]);
        e5(&dir, 4);
    }

    /// GHF5: the target is the `(n + 1)`-th auth rule after the soos rule (off-by-one guard).
    #[test]
    fn test_ghf5_target_must_be_the_rule_after_the_n_skipped_ones() {
        let two_after = pam_dir(&[(
            "inner",
            "auth [success=2 default=ignore] pam_soos.so\n\
             auth sufficient pam_unix.so\n\
             auth required pam_deny.so\n",
        )]);
        e5(&two_after, 2);
        let three_after = pam_dir(&[(
            "inner",
            "auth [success=2 default=ignore] pam_soos.so\n\
             auth sufficient pam_unix.so\n\
             auth required pam_deny.so\n\
             auth required pam_permit.so\n",
        )]);
        accepted(&three_after);
    }

    /// GHF5: a delegation inside the jump span (include, substack, @include, any case).
    #[test]
    fn test_ghf5_jump_over_a_delegation_is_refused() {
        for delegation in [
            "auth include other",
            "auth substack other",
            "@include other",
            "auth INCLUDE other",
        ] {
            let inner = format!(
                "auth [success=2 default=ignore] pam_soos.so\n\
                 auth sufficient pam_unix.so\n\
                 {delegation}\n\
                 auth required pam_permit.so\n\
                 auth required pam_env.so\n"
            );
            let dir = pam_dir(&[("inner", &inner), ("other", "auth required pam_env.so\n")]);
            e5(&dir, 2);
        }
    }

    /// GHF5: a malformed line, an unknown type keyword or a continued line inside the span.
    #[test]
    fn test_ghf5_malformed_unknown_or_continued_line_in_span_is_refused() {
        for bad in [
            "auth required",
            "auth [success=1 pam_unix.so",
            "authx required pam_permit.so",
            "foo required pam_permit.so",
            "auth required pam_permit.so \\",
        ] {
            let inner = format!(
                "auth [success=2 default=ignore] pam_soos.so\n\
                 auth sufficient pam_unix.so\n\
                 {bad}\n\
                 auth required pam_permit.so\n\
                 auth required pam_env.so\n"
            );
            let dir = pam_dir(&[("inner", &inner)]);
            e5(&dir, 2);
        }
    }

    /// GHF5: `-auth` rules are counted (they stay in the chain), other management groups,
    /// comments and blank lines are not.
    #[test]
    fn test_ghf5_counting_rules_of_the_jump_span() {
        // Counting `-auth`: lands on pam_permit; not counting it would run past the end.
        let dash = pam_dir(&[(
            "inner",
            "auth [success=2 default=ignore] pam_soos.so\n\
             -auth [success=1 default=ignore] pam_systemd_home.so\n\
             auth sufficient pam_unix.so\n\
             auth required pam_permit.so\n",
        )]);
        accepted(&dash);
        // Non-auth lines are not counted: the only auth rule after the jump is skipped.
        let others = pam_dir(&[(
            "inner",
            "auth [success=1 default=ignore] pam_soos.so\n\
             auth sufficient pam_unix.so\n\
             account required pam_unix.so\n\
             -session optional pam_systemd.so\n\
             password required pam_unix.so\n\
             Session required pam_env.so\n",
        )]);
        e5(&others, 1);
        // Comments, blank lines and other groups are skipped on the way to the target.
        let skipped = pam_dir(&[(
            "inner",
            "auth [success=1 default=ignore] pam_soos.so\n\
             # a comment\n\
             \n\
             auth sufficient pam_unix.so\n\
             account required pam_unix.so\n\
             auth required pam_permit.so\n",
        )]);
        accepted(&skipped);
    }

    /// GHF5: `success=done` and `sufficient` are unaffected by the target check.
    #[test]
    fn test_ghf5_done_and_sufficient_need_no_target() {
        for rule in [
            "auth [success=done default=ignore] pam_soos.so",
            "auth sufficient pam_soos.so",
        ] {
            let dir = pam_dir(&[("inner", &format!("{rule}\n"))]);
            accepted(&dir);
        }
    }

    /// GHF5: the packaged Arch `system-auth` (`[success=4]` lands on `optional pam_permit.so`)
    /// and the Debian pam-auth-update rewrite (`[success=2]` lands on `required pam_permit.so`)
    /// keep counting as shared.
    #[test]
    fn test_ghf5_packaged_jump_targets_are_accepted() {
        let arch = include_str!("../../../packaging/pam/arch/system-auth");
        let dir = pam_dir(&[("inner", arch)]);
        accepted(&dir);
        let debian = pam_dir(&[(
            "inner",
            "auth\t[success=2 default=ignore]\tpam_soos.so\n\
             auth\t[success=1 default=ignore]\tpam_unix.so nullok\n\
             auth\trequisite\t\t\tpam_deny.so\n\
             auth\trequired\t\t\tpam_permit.so\n\
             auth\toptional\t\t\tpam_cap.so\n",
        )]);
        accepted(&debian);
    }
}
