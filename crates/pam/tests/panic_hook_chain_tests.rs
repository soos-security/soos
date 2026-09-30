//! Contract test for the pam_soos panic hook (review PAM-09, GitHub #263, row PHY1).
//!
//! Calling a PAM entry point must not take over the process panic hook: a panic raised
//! OUTSIDE any soos entry point after `pam_sm_authenticate` returned must still reach the
//! hook that was installed before the module was first used.
//!
//! This file deliberately holds a single test: the panic hook is process-global and the
//! module installs its own hook at most once per process, so the prior hook must be set
//! before any other code of this test binary calls into the module.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "Contractual integration test uses assertions and deliberate panics"
)]

use std::panic::{catch_unwind, set_hook};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use pam_soos::{pam_sm_authenticate, PAM_IGNORE};

#[test]
fn test_panic_outside_entry_point_still_reaches_prior_hook() {
    let seen = Arc::new(AtomicUsize::new(0));
    let recorder = Arc::clone(&seen);
    set_hook(Box::new(move |_info| {
        recorder.fetch_add(1, Ordering::SeqCst);
    }));

    let arg = c"socket=/nonexistent/soos-panic-hook-chain.sock";
    let argv = [arg.as_ptr().cast::<u8>()];
    let code = pam_sm_authenticate(std::ptr::null_mut(), 0, 1, argv.as_ptr());
    assert_eq!(
        code, PAM_IGNORE,
        "absent daemon must fall back to PAM_IGNORE"
    );

    let outside = catch_unwind(|| panic!("host panic outside pam_soos"));
    assert!(outside.is_err());
    assert_eq!(
        seen.load(Ordering::SeqCst),
        1,
        "a panic outside every soos entry point must reach the host's prior panic hook"
    );

    // A second entry point call does not re-take the hook either.
    let code = pam_sm_authenticate(std::ptr::null_mut(), 0, 1, argv.as_ptr());
    assert_eq!(code, PAM_IGNORE);
    let outside = catch_unwind(|| panic!("second host panic"));
    assert!(outside.is_err());
    assert_eq!(seen.load(Ordering::SeqCst), 2);
}
