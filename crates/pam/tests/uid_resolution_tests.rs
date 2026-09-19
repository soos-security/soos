//! UID resolution and buffer safety tests for pam_soos.
//!
//! Sub-issue #29.1: Dynamic buffer growth for getpwnam_r
//! - Start with 1024, retry with sysconf(_SC_GETPW_R_SIZE_MAX) or double on ERANGE
//! - Cap at 64KB to prevent OOM
//! - Acceptance: UID resolution works with LDAP/AD backends
//! - TDD: test_getpwnam_r_handles_erange_retry

use pam_soos::{resolve_username_to_uid, resolve_username_to_uid_with_bounds};

#[test]
fn test_getpwnam_r_handles_erange_retry() {
    // Starting with an intentionally tiny buffer (4 bytes) forces getpwnam_r
    // to return ERANGE on standard Linux systems for "root".
    // The implementation must catch ERANGE, double the buffer size across retries,
    // and successfully resolve root's UID (0).
    let resolved = resolve_username_to_uid_with_bounds("root", 4, 65536);
    assert_eq!(
        resolved,
        Some(0),
        "Dynamic buffer growth must recover from ERANGE and resolve root UID to 0"
    );
}

#[test]
fn test_getpwnam_r_caps_at_max_buffer_size() {
    // If max_size is set too small (e.g. 4 bytes), the retry loop cannot grow
    // large enough to hold root's passwd entry, and must cleanly fail-closed (return None).
    let resolved = resolve_username_to_uid_with_bounds("root", 4, 8);
    assert_eq!(
        resolved, None,
        "Buffer growth exceeding max_size must fail closed and return None"
    );
}

#[test]
fn test_getpwnam_r_nominal_resolution() {
    // Standard resolution for root using the default dynamic sizing
    let resolved = resolve_username_to_uid("root");
    assert_eq!(
        resolved,
        Some(0),
        "Nominal resolution for root user must return Some(0)"
    );
}

#[test]
fn test_getpwnam_r_nonexistent_user_returns_none() {
    let resolved = resolve_username_to_uid("nonexistent_soos_user_xyz_999");
    assert_eq!(
        resolved, None,
        "Resolving nonexistent username must cleanly return None"
    );
}
