//! Contract tests of GitHub #416 for the classification of a logind or bus call failure.
//!
//! zbus 6 reports an error reply whose name is a standard `org.freedesktop.DBus.Error.*`
//! name as `zbus::Error::FDO` and every other name as `zbus::Error::MethodError`; zbus 5
//! reported both as `MethodError`. The companion must keep treating every error reply
//! as a refusal carrying its exact name (for example `UnknownObject`, a vanished
//! session, and `ServiceUnknown`, NetworkManager not running), and every other failure
//! as a transport failure.
//!
//! No bus is needed: the error replies are hand-built messages.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use soos_remote::logind::method_error_name;

/// The `zbus::Error` that zbus 6 builds from an error reply named `name`.
fn reply_error(name: &str) -> zbus::Error {
    let call = zbus::Message::method_call("/org/freedesktop/login1", "UnlockSession")
        .destination("org.freedesktop.login1")
        .interface("org.freedesktop.login1.Manager")
        .build(&("4",))
        .expect("method call");
    let reply = zbus::Message::error(&call.header(), name)
        .build(&("refused",))
        .expect("error reply");
    zbus::Error::from(reply)
}

/// Standard bus error names, reported by zbus 6 as `Error::FDO`.
const STANDARD_NAMES: [&str; 9] = [
    "org.freedesktop.DBus.Error.UnknownObject",
    "org.freedesktop.DBus.Error.AccessDenied",
    "org.freedesktop.DBus.Error.ServiceUnknown",
    "org.freedesktop.DBus.Error.NoReply",
    "org.freedesktop.DBus.Error.UnknownMethod",
    "org.freedesktop.DBus.Error.UnknownProperty",
    "org.freedesktop.DBus.Error.InvalidArgs",
    "org.freedesktop.DBus.Error.Failed",
    "org.freedesktop.DBus.Error.Spawn.ExecFailed",
];

/// logind error names, reported by zbus 6 as `Error::MethodError`.
const LOGIND_NAMES: [&str; 3] = [
    "org.freedesktop.login1.NoSuchSession",
    "org.freedesktop.login1.NotInControl",
    "org.example.Unrelated",
];

#[test]
fn test_standard_error_reply_keeps_its_exact_name() {
    for name in STANDARD_NAMES {
        let error = reply_error(name);
        assert!(
            matches!(error, zbus::Error::FDO(_)),
            "zbus 6 reports {name} as Error::FDO: {error:?}"
        );
        assert_eq!(method_error_name(&error).as_deref(), Some(name), "{name}");
    }
}

#[test]
fn test_logind_error_reply_keeps_its_exact_name() {
    for name in LOGIND_NAMES {
        let error = reply_error(name);
        assert!(
            matches!(error, zbus::Error::MethodError(..)),
            "zbus 6 reports {name} as Error::MethodError: {error:?}"
        );
        assert_eq!(method_error_name(&error).as_deref(), Some(name), "{name}");
    }
}

#[test]
fn test_transport_failures_are_not_method_errors() {
    let failures = [
        zbus::Error::InvalidReply,
        zbus::Error::InputOutput(std::sync::Arc::new(std::io::Error::from(
            std::io::ErrorKind::BrokenPipe,
        ))),
        zbus::Error::FDO(Box::new(zbus::fdo::Error::ZBus(zbus::Error::InvalidReply))),
    ];
    for failure in failures {
        assert_eq!(method_error_name(&failure), None, "{failure:?}");
    }
}
