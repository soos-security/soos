//! Contractual tests for the opt-in `fault-injection` feature of `pam_soos.so`
//! (review finding PAM-01 / TCI-01, GitHub #148).
//!
//! The feature adds the PAM argument `fault_inject=<panic|overflow>` which deliberately panics
//! inside the `catch_unwind` region of the module. It exists only so that the Docker matrix
//! (T10) can load the *release-built* shared object and prove that a panic degrades to
//! `PAM_IGNORE` instead of aborting the host process. These tests pin the hook's contract:
//!
//! - with the feature: every entry path (C ABI, `PamHooks`, `authenticate_with_config`) maps
//!   an injected panic to `PAM_IGNORE`, never to `PAM_SUCCESS`;
//! - without the feature: the argument is ignored and the field stays `None`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use pam_soos::config::{parse_cstrs, PamConfig};
use std::ffi::CStr;

/// The `fault_inject` field is always present and defaults to `None` in every build.
#[test]
fn test_default_config_has_no_fault_injection() {
    let config = PamConfig::default();
    assert_eq!(config.fault_inject, None);
}

/// Unknown or empty modes never arm the hook, regardless of the feature.
#[test]
fn test_fault_inject_unknown_mode_is_ignored() {
    let args: Vec<&CStr> = vec![c"fault_inject=bogus", c"fault_inject=", c"fault_inject"];
    let config = parse_cstrs(args);
    assert_eq!(config.fault_inject, None);
}

#[cfg(not(feature = "fault-injection"))]
mod without_feature {
    use super::*;

    /// Without the feature the argument is inert: production builds cannot be armed.
    #[test]
    fn test_fault_inject_argument_without_feature_is_ignored() {
        let args: Vec<&CStr> = vec![c"fault_inject=panic", c"fault_inject=overflow"];
        let config = parse_cstrs(args);
        assert_eq!(config.fault_inject, None);
    }
}

#[cfg(feature = "fault-injection")]
mod with_feature {
    use super::*;
    use pam_bindings::constants::PamResultCode;
    use pam_bindings::module::{PamHandle, PamHooks};
    use pam_soos::config::FaultInject;
    use pam_soos::{pam_sm_authenticate, SoosPam, PAM_IGNORE};
    use std::ptr;

    /// `fault_inject=panic` and `fault_inject=overflow` are parsed into the typed enum.
    #[test]
    fn test_parse_cstrs_fault_inject_modes() {
        let panic_cfg = parse_cstrs(vec![c"fault_inject=panic"]);
        assert_eq!(panic_cfg.fault_inject, Some(FaultInject::Panic));

        let overflow_cfg = parse_cstrs(vec![c"fault_inject=overflow"]);
        assert_eq!(overflow_cfg.fault_inject, Some(FaultInject::Overflow));
    }

    /// C ABI: an injected explicit panic inside the module returns PAM_IGNORE (25).
    #[test]
    fn test_fault_inject_panic_returns_pam_ignore_via_c_abi() {
        let args = [c"fault_inject=panic", c"socket=/tmp/soos_fault_unused.sock"];
        let argv = [args[0].as_ptr().cast::<u8>(), args[1].as_ptr().cast::<u8>()];
        let code = pam_sm_authenticate(ptr::null_mut(), 0, 2, argv.as_ptr());
        assert_eq!(code, PAM_IGNORE);
    }

    /// C ABI: an injected arithmetic overflow (overflow-checks) returns PAM_IGNORE (25).
    #[test]
    fn test_fault_inject_overflow_returns_pam_ignore_via_c_abi() {
        let args = [
            c"fault_inject=overflow",
            c"socket=/tmp/soos_fault_unused.sock",
        ];
        let argv = [args[0].as_ptr().cast::<u8>(), args[1].as_ptr().cast::<u8>()];
        let code = pam_sm_authenticate(ptr::null_mut(), 0, 2, argv.as_ptr());
        assert_eq!(code, PAM_IGNORE);
    }

    /// `PamHooks::sm_authenticate` path maps the injected panic to PAM_IGNORE as well.
    #[test]
    fn test_fault_inject_via_pam_hooks_returns_pam_ignore() {
        let dummy_ptr = 0x1000 as *mut PamHandle;
        // SAFETY: PamHandle is an opaque type; fault_injection::trigger runs before any libpam
        // access in authenticate_flow (ordering pinned by PHS11), so this dummy pointer is only
        // carried, never read.
        let pamh = unsafe { &mut *dummy_ptr };
        let args: Vec<&CStr> = vec![c"fault_inject=panic"];
        let code = SoosPam::sm_authenticate(pamh, args, 0);
        assert_eq!(code, PamResultCode::PAM_IGNORE);
    }

    /// The hook fires before any socket activity: an `allow`-style daemon is never reached,
    /// so an injected panic can never be converted into PAM_SUCCESS.
    #[test]
    fn test_fault_inject_never_yields_success() {
        for mode in [FaultInject::Panic, FaultInject::Overflow] {
            let config = PamConfig {
                fault_inject: Some(mode),
                ..Default::default()
            };
            let code = SoosPam::authenticate_with_config(None, &config);
            assert_eq!(code, PamResultCode::PAM_IGNORE, "mode {mode:?}");
        }
    }

    /// The hook is repeatable within one process (the panic hook and thread-local location
    /// are reset between calls), mirroring successive PAM conversations in one login process.
    #[test]
    fn test_fault_inject_is_repeatable_in_same_process() {
        let arg = c"fault_inject=panic";
        let argv = [arg.as_ptr().cast::<u8>()];
        for _ in 0..3 {
            assert_eq!(
                pam_sm_authenticate(ptr::null_mut(), 0, 1, argv.as_ptr()),
                PAM_IGNORE
            );
        }
    }
}
