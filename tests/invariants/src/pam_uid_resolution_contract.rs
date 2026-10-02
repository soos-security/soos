//! Fail-closed PAM UID resolution and 2026-10-02 PAM review follow-ups (GitHub #300, #302,
//! #311; matrix rows PUR13-PUR20).
//!
//! - #300: `pam_soos.so` never substitutes the caller's real UID for an unresolvable
//!   PAM user; `libc::getuid` survives only in the null-handle `detached_uid_resolver`.
//! - #302: `uid=` never overrides PAM_USER; the docs and logs no longer recommend it as a
//!   way to skip the lookup.
//! - #300 docs: `uid_hint` is the authoritative target UID for a root peer.
//! - #311: single-allocation tagged encoders, fail-quiet docs, re-entrancy-safe panic
//!   location take, non-blocking nonce, logged security rejections, deadline ordering.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    Path::new(&manifest_dir)
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Source text before the first `#[cfg(test)]`, with `//` comment lines removed.
fn production_code(rel: &str) -> String {
    let src = read(rel);
    let prod = src.split("#[cfg(test)]").next().unwrap_or(&src).to_owned();
    prod.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The body of `fn <name>` (from its signature to the next top-level `\n}\n`).
fn function_body<'a>(src: &'a str, signature: &str) -> &'a str {
    let start = src
        .find(signature)
        .unwrap_or_else(|| panic!("`{signature}` must exist"));
    let end = src[start..]
        .find("\n}\n")
        .map_or(src.len(), |off| start + off + 3);
    &src[start..end]
}

/// PUR13 (#300): the only `getuid` of the PAM flow is the null-handle stand-in.
#[test]
fn test_pur_getuid_only_in_detached_resolver() {
    let lib = production_code("crates/pam/src/lib.rs");
    assert_eq!(
        lib.matches("getuid").count(),
        1,
        "crates/pam/src/lib.rs may call getuid exactly once (detached_uid_resolver)"
    );
    let detached = function_body(&lib, "fn detached_uid_resolver(");
    assert!(detached.contains("libc::getuid()"));
    let default = function_body(&lib, "fn default_uid_resolver(");
    assert!(
        default.contains("-> Option<u32>") && !default.contains("getuid"),
        "the libpam resolver must report failure as None, never as the caller's UID"
    );
}

/// PUR14 (#302): neither the code nor the module docs advise `uid=` to skip the lookup.
#[test]
fn test_pur_uid_argument_is_not_advertised_as_a_lookup_bypass() {
    let lib = read("crates/pam/src/lib.rs");
    assert!(!lib.contains("configure uid="), "lib.rs still advises uid=");
    let docs = read("Docs/PAM_MODULE.md");
    assert!(
        !docs.contains("to skip the lookup"),
        "Docs/PAM_MODULE.md still advises uid= to skip the lookup"
    );
    assert!(
        docs.contains("resolves to the same UID"),
        "Docs/PAM_MODULE.md must document the uid= / PAM_USER match rule (#302)"
    );
}

/// PUR15 (#300): `uid_hint` is documented as the authoritative target for root peers.
#[test]
fn test_pur_uid_hint_is_documented_as_authoritative_target() {
    let types = read("crates/protocol/src/types.rs");
    assert!(
        !types.contains("merely a consistency assertion"),
        "types.rs still calls uid_hint a mere consistency assertion"
    );
    assert!(types.contains("authoritative target UID"));
    let ipc = read("Docs/IPC_PROTOCOL.md");
    assert!(
        !ipc.contains("authoritatively cross-checked by the daemon using kernel `SO_PEERCRED`"),
        "Docs/IPC_PROTOCOL.md still describes uid_hint as a consistency check only"
    );
    assert!(ipc.contains("authoritative target UID"));
}

/// PUR16 (#311 PAM-NEW-4): the docs and the ADR describe the fail-quiet daemon-absent
/// behaviour the code implements (no message at all).
#[test]
fn test_pur_daemon_absent_docs_describe_fail_quiet() {
    let docs = read("Docs/PAM_MODULE.md");
    assert!(
        !docs.contains("at most the single generic"),
        "Docs/PAM_MODULE.md still describes a generic daemon-absent message"
    );
    assert!(docs.contains("no message at all"));
    let adr = read("AI/DECISIONS.md");
    let item = adr
        .find("(5) *Daemon-absent message")
        .map(|i| &adr[i..adr[i..].find('\n').map_or(adr.len(), |e| i + e)])
        .expect("ADR item (5) exists");
    assert!(
        item.contains("fail quiet") && !item.contains("still produces the single generic"),
        "ADR item (5) must record the fail-quiet decision: {item}"
    );
}

/// PUR17 (#311 PAM-NEW-3): the tagged client encoders size first and serialize in place.
#[test]
fn test_pur_tagged_encoders_serialize_in_place() {
    let message = production_code("crates/protocol/src/message.rs");
    for forbidden in ["to_allocvec", "to_stdvec", "to_extend"] {
        assert!(
            !message.contains(forbidden),
            "message.rs must not serialize through `{forbidden}` (#311 PAM-NEW-3)"
        );
    }
    assert!(message.contains("encode_with_limit_and_trailer("));
    let codec = production_code("crates/protocol/src/codec.rs");
    let helper = function_body(&codec, "pub fn encode_with_limit_and_trailer<");
    assert!(
        helper.contains("postcard::ser_flavors::Size")
            && helper.contains("postcard::to_slice")
            && helper.matches("buf.zeroize()").count() >= 2,
        "the shared encoder must size first, serialize in place and zeroize on error"
    );
}

/// PUR18 (#311 PAM-NEW-6): the request nonce never blocks on an uninitialized CRNG.
#[test]
fn test_pur_request_nonce_uses_nonblocking_getrandom() {
    let ipc = production_code("crates/pam/src/ipc.rs");
    assert!(ipc.contains("libc::GRND_NONBLOCK"));
    assert!(
        !ipc.contains("getrandom::fill"),
        "getrandom::fill blocks until the CRNG is initialized"
    );
    let manifest = read("crates/pam/Cargo.toml");
    assert!(
        !manifest.contains("getrandom"),
        "soos-pam no longer needs the getrandom crate"
    );
}

/// PUR19 (#311 PAM-NEW-5): taking the panic location never panics (TLS teardown or a
/// re-entrant borrow), since it runs outside `catch_unwind`.
#[test]
fn test_pur_take_panic_location_is_panic_free() {
    let syslog = production_code("crates/pam/src/syslog.rs");
    let take = function_body(&syslog, "pub fn take_panic_location(");
    assert!(take.contains(".try_with(") && take.contains(".try_borrow_mut()"));
    assert!(!take.contains(".with(") && !take.contains(".borrow_mut()"));
}

/// PUR20 (#311 PAM-NEW-7): the deadline starts before any work of the flow, and the
/// security-relevant IPC rejections are logged.
#[test]
fn test_pur_deadline_ordering_and_security_rejection_logging() {
    let lib = production_code("crates/pam/src/lib.rs");
    let flow = function_body(&lib, "fn authenticate_flow<");
    let trigger = flow.find("fault_injection::trigger(").expect("trigger");
    let start = flow
        .find("ExchangeDeadline::start(")
        .expect("deadline start");
    let service = flow.find("with_pam_service(").expect("service item");
    let disabled = flow.find("is_disabled()").expect("disable flags");
    let resolve = flow.find("resolve_uid(").expect("uid resolution");
    assert!(
        trigger < start,
        "the fault-injection hook stays first (PHS11)"
    );
    assert!(
        start < service && service < disabled && disabled < resolve,
        "ExchangeDeadline::start must precede the PAM_SERVICE read, the disable-flag check \
         and the UID resolution"
    );
    assert!(flow.contains("security_log_message()") && flow.contains("log_warning("));
}
