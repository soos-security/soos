//! Contract tests that drive `SoosPam` through a REAL Linux-PAM handle created with
//! `pam_start`, with a capturing conversation function.
//!
//! - GitHub #176 (review PAM-05): the module reads the `PAM_SERVICE` item, so the
//!   installed line `auth sufficient pam_soos.so timeout_ms=2500` (no `service=`) is
//!   disabled by `gdm.disable` and forwards the real service name to the daemon.
//! - GitHub #174 (review PAM-03): the text shown to the unauthenticated user never
//!   reveals the internal `ReasonClass` (no presentation-attack / camera-state oracle).
//!
//! Every pathway asserts the `PAM_IGNORE` fallback (never `PAM_SUCCESS` on failure).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Contractual integration tests use assertions, unwrap, and expect"
)]

use std::collections::BTreeSet;
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::Mutex;
use std::thread;

use pam_soos::config::{parse_cstrs, PamConfig};
use pam_soos::{PamHandle, PamResultCode, SoosPam};
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, Response, Verdict, CURRENT_VERSION};
use tempfile::tempdir;

/// Linux-PAM `PAM_CONV_ERR`.
const PAM_CONV_ERR: c_int = 19;

#[repr(C)]
struct PamMessageC {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamConvC {
    conv: Option<
        extern "C" fn(c_int, *mut *const PamMessageC, *mut *mut c_void, *mut c_void) -> c_int,
    >,
    appdata_ptr: *mut c_void,
}

#[link(name = "pam")]
extern "C" {
    fn pam_start(
        service_name: *const c_char,
        user: *const c_char,
        pam_conversation: *const PamConvC,
        pamh: *mut *mut PamHandle,
    ) -> c_int;
    fn pam_end(pamh: *mut PamHandle, pam_status: c_int) -> c_int;
}

/// Conversation function recording every message into the `Mutex<Vec<String>>` passed as
/// `appdata_ptr`. Never panics (it runs across the C ABI) and returns `PAM_CONV_ERR` so no
/// response memory is handed back to the module.
extern "C" fn capture_conv(
    num_msg: c_int,
    msg: *mut *const PamMessageC,
    _resp: *mut *mut c_void,
    appdata_ptr: *mut c_void,
) -> c_int {
    if msg.is_null() || appdata_ptr.is_null() {
        return PAM_CONV_ERR;
    }
    // SAFETY: appdata_ptr is the `&Mutex<Vec<String>>` registered by `with_pam_handle`,
    // which outlives the handle.
    let sink = unsafe { &*appdata_ptr.cast::<Mutex<Vec<String>>>() };
    for i in 0..usize::try_from(num_msg).unwrap_or(0) {
        // SAFETY: Linux-PAM passes `num_msg` message pointers.
        let m = unsafe { *msg.add(i) };
        if m.is_null() {
            continue;
        }
        // SAFETY: m points to a PamMessageC built by pam-bindings with a NUL-terminated msg.
        let text = unsafe { CStr::from_ptr((*m).msg) }
            .to_string_lossy()
            .into_owned();
        if let Ok(mut guard) = sink.lock() {
            guard.push(text);
        }
    }
    PAM_CONV_ERR
}

/// Opens a real PAM handle for `service`, runs `f`, closes the handle and returns `f`'s
/// result together with every conversation message the module emitted.
fn with_pam_handle<T>(service: &str, f: impl FnOnce(&mut PamHandle) -> T) -> (T, Vec<String>) {
    let sink: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let conv = PamConvC {
        conv: Some(capture_conv),
        appdata_ptr: std::ptr::from_ref(&sink).cast_mut().cast(),
    };
    let service_c = CString::new(service).unwrap();
    let user_c = CString::new("soos-contract-user").unwrap();
    let mut pamh: *mut PamHandle = std::ptr::null_mut();
    // SAFETY: valid NUL-terminated strings, a conv struct that outlives the handle, and
    // an out-pointer to a local.
    let rc = unsafe { pam_start(service_c.as_ptr(), user_c.as_ptr(), &conv, &mut pamh) };
    assert_eq!(rc, 0, "pam_start failed with {rc}");
    assert!(!pamh.is_null());

    // SAFETY: pam_start succeeded, pamh is a live handle until pam_end below.
    let out = f(unsafe { &mut *pamh });

    // SAFETY: pamh came from pam_start and is ended exactly once.
    unsafe { pam_end(pamh, 0) };
    let messages = sink.into_inner().unwrap();
    (out, messages)
}

/// One-shot mock daemon answering `(verdict, reason)` and returning the request's service.
fn spawn_daemon(
    listener: UnixListener,
    verdict: Verdict,
    reason: ReasonClass,
) -> thread::JoinHandle<Option<String>> {
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().ok()?;
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).ok()?;
        let size = u32::from_be_bytes(len_buf) as usize;
        let mut full_req = len_buf.to_vec();
        let mut body = vec![0u8; size];
        stream.read_exact(&mut body).ok()?;
        full_req.extend_from_slice(&body);
        let req: Request = decode(&full_req).ok()?;
        let resp = Response {
            version: CURRENT_VERSION,
            request_id: req.request_id,
            verdict,
            reason_class: reason,
            issued_monotonic_ns: 1000,
            expires_monotonic_ns: 2000,
        };
        stream.write_all(&encode(&resp).ok()?).ok()?;
        Some(req.service.clone())
    })
}

fn base_config(socket_path: &Path, flag_dir: &Path) -> PamConfig {
    PamConfig {
        timeout_ms: 1000,
        socket_path: socket_path.to_path_buf(),
        uid: Some(1000),
        flag_dir: flag_dir.to_path_buf(),
        ..Default::default()
    }
}

/// Runs one authentication with a daemon answering `(verdict, reason)`; returns the PAM
/// code, the service seen by the daemon and the conversation messages.
fn run_with_daemon(
    service: &str,
    config_for: impl FnOnce(&Path, &Path) -> PamConfig,
    verdict: Verdict,
    reason: ReasonClass,
) -> (PamResultCode, Option<String>, Vec<String>) {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("daemon.sock");
    let flags = tmp.path().join("flags");
    std::fs::create_dir(&flags).unwrap();
    let listener = UnixListener::bind(&sock).unwrap();
    let daemon = spawn_daemon(listener, verdict, reason);
    let config = config_for(&sock, &flags);
    let (code, messages) = with_pam_handle(service, |h| {
        SoosPam::authenticate_with_config(Some(h), &config)
    });
    let seen = daemon.join().unwrap();
    (code, seen, messages)
}

const ALL_REASONS: [ReasonClass; 13] = [
    ReasonClass::FaceMatch,
    ReasonClass::NoFace,
    ReasonClass::MultipleFaces,
    ReasonClass::ScoreBelowThreshold,
    ReasonClass::PadFailed,
    ReasonClass::CameraUnavailable,
    ReasonClass::ModelUnavailable,
    ReasonClass::StaleFrame,
    ReasonClass::Timeout,
    ReasonClass::RateLimited,
    ReasonClass::UidMismatch,
    ReasonClass::MalformedRequest,
    ReasonClass::InternalError,
];

/// Final conversation text (the verdict feedback following "Looking for face...").
fn final_message(messages: &[String]) -> String {
    messages.last().cloned().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// GitHub #176 — PAM_SERVICE item
// ---------------------------------------------------------------------------

/// Without `service=`, the daemon receives the PAM_SERVICE item of the handle.
#[test]
fn test_pam_service_item_forwarded_to_daemon_when_no_service_argument() {
    let (code, seen, _) = run_with_daemon(
        "gdm-soos-contract",
        base_config,
        Verdict::Allow,
        ReasonClass::FaceMatch,
    );
    assert_eq!(code, PamResultCode::PAM_SUCCESS);
    assert_eq!(seen.as_deref(), Some("gdm-soos-contract"));
}

/// The installed GDM line has no `service=`: `<flag_dir>/gdm.disable` must still disable
/// it through the PAM_SERVICE item, before any socket activity.
#[test]
fn test_gdm_disable_flag_honored_through_pam_service_item() {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    listener.set_nonblocking(true).unwrap();
    std::fs::write(tmp.path().join("gdm.disable"), "").unwrap();
    let config = base_config(&sock, tmp.path());

    let (code, messages) = with_pam_handle("gdm-soos-contract", |h| {
        SoosPam::authenticate_with_config(Some(h), &config)
    });

    assert_eq!(
        code,
        PamResultCode::PAM_IGNORE,
        "gdm.disable must make the GDM line fall back to the password"
    );
    assert!(
        listener.accept().is_err(),
        "a disabled service must never contact the daemon"
    );
    assert!(messages.is_empty(), "a disabled service stays silent");
}

/// The per-service flag `<flag_dir>/<PAM_SERVICE>.disable` disables that service only.
#[test]
fn test_per_service_disable_flag_honored_through_pam_service_item() {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("daemon.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    listener.set_nonblocking(true).unwrap();
    std::fs::write(tmp.path().join("soos-contract-sudo.disable"), "").unwrap();
    let config = base_config(&sock, tmp.path());

    let (code, _) = with_pam_handle("soos-contract-sudo", |h| {
        SoosPam::authenticate_with_config(Some(h), &config)
    });

    assert_eq!(code, PamResultCode::PAM_IGNORE);
    assert!(listener.accept().is_err());
}

/// An explicit `service=` argument keeps precedence over the PAM_SERVICE item.
#[test]
fn test_explicit_service_argument_overrides_pam_service_item() {
    let (code, seen, _) = run_with_daemon(
        "gdm-soos-contract",
        |sock, flags| {
            let arg = CString::new("service=sudo").unwrap();
            let mut config = parse_cstrs([arg.as_c_str()]);
            config.timeout_ms = 1000;
            config.socket_path = sock.to_path_buf();
            config.uid = Some(1000);
            config.flag_dir = flags.to_path_buf();
            config
        },
        Verdict::Allow,
        ReasonClass::FaceMatch,
    );
    assert_eq!(code, PamResultCode::PAM_SUCCESS);
    assert_eq!(seen.as_deref(), Some("sudo"));
}

// ---------------------------------------------------------------------------
// GitHub #174 — no ReasonClass oracle in the user-facing conversation
// ---------------------------------------------------------------------------

/// Every `Verdict::Deny` produces the same neutral text whatever the `ReasonClass`,
/// in particular a PAD (spoof) rejection is indistinguishable from a non-match.
#[test]
fn test_deny_feedback_is_identical_for_every_reason_class() {
    let mut texts = BTreeSet::new();
    for reason in ALL_REASONS {
        let (code, _, messages) =
            run_with_daemon("soos-contract", base_config, Verdict::Deny, reason);
        assert_eq!(
            code,
            PamResultCode::PAM_IGNORE,
            "Deny/{reason:?} must ignore"
        );
        let text = final_message(&messages);
        assert!(
            !text.to_lowercase().contains("spoof"),
            "Deny/{reason:?} leaked the PAD outcome: {text:?}"
        );
        texts.insert(text);
    }
    assert_eq!(
        texts.len(),
        1,
        "Deny feedback must not depend on ReasonClass, got {texts:?}"
    );
}

/// Every non-Allow, non-Deny outcome (Unavailable, ProtocolError, IPC error) collapses
/// to one generic text that reveals neither camera state nor the failure class, and the
/// whole failure surface uses at most two texts (denied / unavailable).
#[test]
fn test_unavailable_feedback_is_a_single_generic_text() {
    let mut unavailable = BTreeSet::new();
    for verdict in [Verdict::Unavailable, Verdict::ProtocolError] {
        for reason in ALL_REASONS {
            let (code, _, messages) =
                run_with_daemon("soos-contract", base_config, verdict, reason);
            assert_eq!(
                code,
                PamResultCode::PAM_IGNORE,
                "{verdict:?}/{reason:?} must ignore"
            );
            unavailable.insert(final_message(&messages));
        }
    }

    // IPC failure (daemon offline).
    let tmp = tempdir().unwrap();
    let offline = base_config(&tmp.path().join("absent.sock"), tmp.path());
    let (code, messages) = with_pam_handle("soos-contract", |h| {
        SoosPam::authenticate_with_config(Some(h), &offline)
    });
    assert_eq!(code, PamResultCode::PAM_IGNORE);
    unavailable.insert(final_message(&messages));

    assert_eq!(
        unavailable.len(),
        1,
        "non-Deny failures must share one generic text, got {unavailable:?}"
    );
    for text in &unavailable {
        let lower = text.to_lowercase();
        assert!(
            !lower.contains("camera") && !lower.contains("spoof"),
            "failure text leaks internal state: {text:?}"
        );
    }

    let (_, _, deny) = run_with_daemon(
        "soos-contract",
        base_config,
        Verdict::Deny,
        ReasonClass::ScoreBelowThreshold,
    );
    let mut all = unavailable;
    all.insert(final_message(&deny));
    assert!(all.len() <= 2, "at most two failure texts, got {all:?}");
}
