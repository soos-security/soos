//! Shared harness for the PAM conversation contract tests (GitHub #220, #221, #227):
//! a REAL Linux-PAM handle created with `pam_start` and a capturing conversation
//! function, plus a one-shot mock daemon that records the request it received.

#![allow(
    dead_code,
    reason = "Each integration test binary uses a different subset of the shared harness"
)]

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::sync::Mutex;
use std::thread;

use pam_soos::PamHandle;
use soos_protocol::codec::{decode, encode};
use soos_protocol::types::{ReasonClass, Request, Response, Verdict, CURRENT_VERSION};

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

/// Conversation function recording every message into the `Mutex<Vec<String>>` passed
/// as `appdata_ptr`. Never panics (it runs across the C ABI) and returns `PAM_CONV_ERR`
/// so no response memory is handed back to the module.
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
pub fn with_pam_handle<T>(service: &str, f: impl FnOnce(&mut PamHandle) -> T) -> (T, Vec<String>) {
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

/// What the one-shot mock daemon observed.
#[derive(Debug, Clone)]
pub struct SeenRequest {
    pub uid_hint: u32,
    pub service: String,
}

/// One-shot mock daemon answering `(verdict, reason)`; returns the decoded request.
pub fn spawn_daemon(
    listener: UnixListener,
    verdict: Verdict,
    reason: ReasonClass,
) -> thread::JoinHandle<Option<SeenRequest>> {
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
        Some(SeenRequest {
            uid_hint: req.uid_hint,
            service: req.service.clone(),
        })
    })
}

/// Text announcing that the camera lookup started.
pub const LOOKING_FOR_FACE: &str = "[soos] Looking for face...";
