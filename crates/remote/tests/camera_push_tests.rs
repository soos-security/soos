//! Contract test of the camera-view Web Push payload of `soos-remote` (GitHub #345, ADR
//! 2026-10-07 "Live Camera View in `soos-remote` Through the Daemon Preview Channel",
//! architect spec `AI/architect_spec_remote_live_camera.md` §10.1, test 39, matrix RLC13,
//! RLC15).
//!
//! The spec maps test 39 to `push_tests.rs`; it lives in this new file so that the existing
//! push suite keeps compiling while the camera API does not exist yet (name unchanged).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use serde_json::Value;

use soos_remote::push::{camera_payload, test_payload};
use soos_remote::{MAX_PUSH_PLAINTEXT_BYTES, PUSH_CAMERA_TOPIC};

const RP_ID: &str = "pc.tail1234.ts.net";

/// Test 39 (RLC13, RLC15): fixed Declarative Web Push text (`soos camera view` / `The live
/// camera view of your PC was started`), `navigate` = `https://<rp_id>/`, `soos.kind =
/// "camera"`, zero counts, no source, account, time or image; at most 1 KiB; one generic
/// payload whatever the alert preview setting (the function takes no preview argument).
#[test]
fn test_rlc_camera_payload() {
    assert_eq!(PUSH_CAMERA_TOPIC, "sooscamera");
    assert!(PUSH_CAMERA_TOPIC.bytes().all(|b| b.is_ascii_alphanumeric()));

    let bytes = camera_payload(RP_ID).expect("camera payload renders");
    assert!(bytes.len() <= MAX_PUSH_PLAINTEXT_BYTES, "{}", bytes.len());
    assert!(bytes.len() <= 1024);
    let json: Value = serde_json::from_slice(&bytes).expect("valid JSON");

    assert_eq!(json["web_push"], 8030, "Declarative Web Push");
    let n = &json["notification"];
    assert_eq!(n["title"], "soos camera view");
    assert_eq!(n["body"], "The live camera view of your PC was started");
    assert_eq!(n["navigate"], "https://pc.tail1234.ts.net/");
    assert_eq!(n["lang"], "en");
    for key in ["image", "icon", "badge", "data"] {
        assert!(n.get(key).is_none(), "no {key} in the notification");
    }

    let soos = &json["soos"];
    assert_eq!(soos["kind"], "camera");
    assert_eq!(soos["v"], 1);
    assert_eq!(soos["wrong_password"], 0);
    assert_eq!(soos["locked_out"], 0);
    for key in ["source", "account", "last_unix_ms"] {
        assert!(
            soos.get(key).is_none_or(Value::is_null),
            "no {key} in a camera payload: {json}"
        );
    }

    // navigate follows rp_id; the text never does.
    let other: Value =
        serde_json::from_slice(&camera_payload("box.tail9.ts.net").unwrap()).unwrap();
    assert_eq!(
        other["notification"]["navigate"],
        "https://box.tail9.ts.net/"
    );
    assert_eq!(other["notification"]["title"], n["title"]);
    assert_eq!(other["notification"]["body"], n["body"]);

    // Deterministic, and distinct from the test notification.
    assert_eq!(camera_payload(RP_ID).unwrap(), bytes);
    let test: Value = serde_json::from_slice(&test_payload(RP_ID).unwrap()).unwrap();
    assert_ne!(test["soos"]["kind"], soos["kind"]);

    // No identity, time or pixel-related word leaks into the text.
    let text = String::from_utf8(bytes).unwrap();
    for needle in ["owner@", "jpeg", "frame", "token", "width", "sequence"] {
        assert!(
            !text.to_ascii_lowercase().contains(needle),
            "{needle} in {text}"
        );
    }
}
