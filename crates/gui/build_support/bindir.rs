//! Build-time validation of `SOOS_BINDIR` (GitHub #318), shared by `crates/gui/build.rs` and
//! `crates/gui/tests/program_path_tests.rs`.
//!
//! `scripts/install.sh --build` exports `SOOS_BINDIR=<prefix>/bin`, so the GUI's privileged
//! actions run `soos-enroll` from where that installer put it. The value ends up in an argument
//! vector run as root through `pkexec`, so only a normalized absolute directory made of
//! `[A-Za-z0-9._+-]` components is accepted; anything else fails the build.

/// Directory of `soos-enroll` when `SOOS_BINDIR` is unset (every package installs it there).
pub const DEFAULT_BINDIR: &str = "/usr/bin";

/// Longest accepted `SOOS_BINDIR`, in bytes.
pub const MAX_BINDIR_LEN: usize = 256;

/// Checks that `dir` is a safe, normalized absolute directory: it starts with `/`, is at most
/// [`MAX_BINDIR_LEN`] bytes, has no empty, `.` or `..` component (so no `//` and no trailing
/// `/`, and `/` alone is refused), and every component uses only `[A-Za-z0-9._+-]`.
pub fn validate_bindir(dir: &str) -> Result<(), &'static str> {
    if dir.len() > MAX_BINDIR_LEN {
        return Err("SOOS_BINDIR is longer than 256 bytes");
    }
    let Some(rest) = dir.strip_prefix('/') else {
        return Err("SOOS_BINDIR must be an absolute path");
    };
    for component in rest.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err("SOOS_BINDIR must be normalized (no empty, '.' or '..' component)");
        }
        if !component
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
        {
            return Err("SOOS_BINDIR may only contain [A-Za-z0-9._+-] and '/'");
        }
    }
    Ok(())
}

/// Absolute `soos-enroll` path for an optional `SOOS_BINDIR` (`None` means [`DEFAULT_BINDIR`]).
pub fn enroll_program_for(bindir: Option<&str>) -> Result<String, &'static str> {
    let dir = bindir.unwrap_or(DEFAULT_BINDIR);
    validate_bindir(dir)?;
    Ok(format!("{dir}/soos-enroll"))
}
