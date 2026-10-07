//! Contract tests of the live camera view configuration of `soos-remote` (GitHub #345, ADR
//! 2026-10-07 "Live Camera View in `soos-remote` Through the Daemon Preview Channel",
//! architect spec `AI/architect_spec_remote_live_camera.md` §3.2, §4.1, test 36, matrix
//! RLC1).
//!
//! The spec maps test 36 to `config_tests.rs`; it lives in this new file so that the
//! existing configuration suite keeps compiling while the camera API does not exist yet
//! (the name is unchanged, traceability greps the test name).
//!
//! Pure parsing over text; nothing touches `$XDG_RUNTIME_DIR`, `$HOME` or `/etc`.

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

use std::path::Path;

use soos_remote::config::{parse_config, CameraConfig, CameraWidth, ConfigError, RemoteConfig};
use soos_remote::{
    ACTION_CAMERA_OPTIONS, ACTION_CAMERA_STOP, ACTION_CAMERA_STREAM, ACTION_CAMERA_VIEW,
    CAMERA_DAEMON_BACKOFF_MAX_MS, CAMERA_DAEMON_BACKOFF_MIN_MS, CAMERA_DAEMON_CLOSE_WAIT_MS,
    CAMERA_DAEMON_CONNECT_TIMEOUT_MS, CAMERA_DAEMON_IDLE_RETRIES, CAMERA_DAEMON_IO_TIMEOUT_MS,
    CAMERA_DAEMON_MAX_REQUESTS_PER_CONNECTION, CAMERA_DAEMON_RECONNECT_AFTER_MS,
    CAMERA_DAEMON_SERVICE, CAMERA_DAEMON_SOCKET_PATH, CAMERA_FIRST_FRAME_TIMEOUT_MS,
    CAMERA_FULL_WIDTH, CAMERA_HALF_WIDTH, CAMERA_MAX_BAD_FRAMES, CAMERA_MAX_SCRATCH_BYTES,
    CAMERA_MAX_SOURCE_HEIGHT, CAMERA_MAX_SOURCE_WIDTH, CAMERA_MIN_SOURCE_DIM,
    CAMERA_RATE_LIMITED_BACKOFF_MS, CAMERA_REFUSED_AUDIT_MIN_INTERVAL_MS, CAMERA_SESSION_CHECK_MS,
    CAMERA_STALL_TIMEOUT_MS, CAMERA_STREAM_BOUNDARY, CAMERA_VIEW_COOLDOWN_MS,
    CAMERA_VIEW_TOKEN_B64_LEN, CAMERA_VIEW_TOKEN_BYTES, CAMERA_VIEW_TOKEN_TTL_MS,
    DEFAULT_CAMERA_FPS, DEFAULT_CAMERA_MAX_VIEW_S, DEFAULT_CAMERA_QUALITY, MAX_CAMERA_FPS,
    MAX_CAMERA_JPEG_BYTES, MAX_CAMERA_MAX_VIEW_S, MAX_CAMERA_QUALITY, MAX_CAMERA_VIEWS,
    MIN_CAMERA_FPS, MIN_CAMERA_MAX_VIEW_S, MIN_CAMERA_QUALITY, PUSH_CAMERA_TOPIC,
};

const RUNTIME_DIR: &str = "/run/user/1000";
const LOGINS: &str = "allowed_logins = [\"owner@example.com\"]\n";
const RP: &str = "rp_id = \"pc.tail1234.ts.net\"\n";
const FUNNEL: &str = "allow_funnel = true\n";

fn parse(extra: &str) -> Result<RemoteConfig, ConfigError> {
    parse_config(&format!("{LOGINS}{extra}"), Some(Path::new(RUNTIME_DIR)))
}

fn camera(extra: &str) -> Result<CameraConfig, ConfigError> {
    parse(extra).map(|c| c.camera)
}

/// Extra (not numbered in the spec, guards §3.2): every camera constant has the specified
/// value, so a drifting bound cannot hide behind a test that reads the constant.
#[test]
fn test_rlc_camera_constants_match_the_spec() {
    assert_eq!(MAX_CAMERA_VIEWS, 1);
    assert_eq!(DEFAULT_CAMERA_MAX_VIEW_S, 120);
    assert_eq!(MIN_CAMERA_MAX_VIEW_S, 10);
    assert_eq!(MAX_CAMERA_MAX_VIEW_S, 300);
    assert_eq!(DEFAULT_CAMERA_FPS, 5);
    assert_eq!(MIN_CAMERA_FPS, 1);
    assert_eq!(MAX_CAMERA_FPS, 10);
    assert_eq!(CAMERA_FULL_WIDTH, 640);
    assert_eq!(CAMERA_HALF_WIDTH, 320);
    assert_eq!(DEFAULT_CAMERA_QUALITY, 70);
    assert_eq!(MIN_CAMERA_QUALITY, 50);
    assert_eq!(MAX_CAMERA_QUALITY, 85);
    assert_eq!(CAMERA_MIN_SOURCE_DIM, 16);
    assert_eq!(CAMERA_MAX_SOURCE_WIDTH, 640);
    assert_eq!(CAMERA_MAX_SOURCE_HEIGHT, 480);
    assert_eq!(CAMERA_MAX_SCRATCH_BYTES, 921_600);
    assert_eq!(MAX_CAMERA_JPEG_BYTES, 524_288);
    assert_eq!(CAMERA_VIEW_TOKEN_BYTES, 32);
    assert_eq!(CAMERA_VIEW_TOKEN_B64_LEN, 43);
    assert_eq!(CAMERA_VIEW_TOKEN_TTL_MS, 10_000);
    assert_eq!(CAMERA_VIEW_COOLDOWN_MS, 10_000);
    assert_eq!(CAMERA_FIRST_FRAME_TIMEOUT_MS, 5_000);
    assert_eq!(CAMERA_STALL_TIMEOUT_MS, 5_000);
    assert_eq!(CAMERA_SESSION_CHECK_MS, 5_000);
    assert_eq!(CAMERA_MAX_BAD_FRAMES, 10);
    assert_eq!(CAMERA_REFUSED_AUDIT_MIN_INTERVAL_MS, 5_000);
    assert_eq!(CAMERA_DAEMON_SOCKET_PATH, "/run/soos/daemon.sock");
    assert_eq!(CAMERA_DAEMON_SERVICE, "soos-remote");
    assert_eq!(CAMERA_DAEMON_CONNECT_TIMEOUT_MS, 500);
    assert_eq!(CAMERA_DAEMON_IO_TIMEOUT_MS, 3_000);
    assert_eq!(CAMERA_DAEMON_RECONNECT_AFTER_MS, 25_000);
    assert_eq!(CAMERA_DAEMON_MAX_REQUESTS_PER_CONNECTION, 1_000);
    assert_eq!(CAMERA_DAEMON_BACKOFF_MIN_MS, 200);
    assert_eq!(CAMERA_DAEMON_BACKOFF_MAX_MS, 2_000);
    assert_eq!(CAMERA_RATE_LIMITED_BACKOFF_MS, 250);
    assert_eq!(CAMERA_DAEMON_CLOSE_WAIT_MS, 200);
    assert_eq!(CAMERA_DAEMON_IDLE_RETRIES, 1);
    assert_eq!(CAMERA_STREAM_BOUNDARY, "soosframe");
    assert_eq!(PUSH_CAMERA_TOPIC, "sooscamera");
    assert_eq!(ACTION_CAMERA_OPTIONS, "camera-options");
    assert_eq!(ACTION_CAMERA_VIEW, "camera-view");
    assert_eq!(ACTION_CAMERA_STREAM, "camera-stream");
    assert_eq!(ACTION_CAMERA_STOP, "camera-stop");
}

/// Test 36 (RLC1): the six camera keys: defaults (off, 120 s, 5 fps, 640, q70), accepted
/// bounds, every §4.1 error with its fixed text and check order, range keys validated even
/// with `camera_view = false`, `0` never accepted, wrong TOML types are `Syntax`, and
/// `camera_view` does not require push.
#[test]
fn test_rlc_camera_config_keys() {
    // Defaults.
    let default = CameraConfig::default();
    assert_eq!(
        default,
        CameraConfig {
            enabled: false,
            funnel: false,
            max_view_s: 120,
            fps: 5,
            width: CameraWidth::Full,
            quality: 70,
        }
    );
    assert_eq!(CameraWidth::default(), CameraWidth::Full);
    assert_eq!(
        camera(""),
        Ok(default.clone()),
        "absent keys keep the view off"
    );
    assert_eq!(
        camera(RP),
        Ok(default.clone()),
        "rp_id alone never enables it"
    );
    assert_eq!(
        camera("camera_view = false\ncamera_view_funnel = false\n"),
        Ok(default.clone())
    );

    // camera_view requires rp_id, not push.
    assert_eq!(
        camera("camera_view = true\n"),
        Err(ConfigError::CameraRequiresRpId)
    );
    let on = camera(&format!("{RP}camera_view = true\n")).unwrap();
    assert!(on.enabled);
    assert!(!on.funnel, "tailnet only by default");

    // camera_view_funnel requires camera_view, then allow_funnel.
    assert_eq!(
        camera(&format!("{RP}{FUNNEL}camera_view_funnel = true\n")),
        Err(ConfigError::CameraFunnelRequiresCameraView)
    );
    assert_eq!(
        camera(&format!(
            "{RP}{FUNNEL}camera_view = false\ncamera_view_funnel = true\n"
        )),
        Err(ConfigError::CameraFunnelRequiresCameraView)
    );
    assert_eq!(
        camera(&format!(
            "{RP}camera_view = true\ncamera_view_funnel = true\n"
        )),
        Err(ConfigError::CameraFunnelRequiresFunnel)
    );
    let both = camera(&format!(
        "{RP}{FUNNEL}camera_view = true\ncamera_view_funnel = true\n"
    ))
    .unwrap();
    assert!(both.enabled && both.funnel);

    // Ranges: accepted bounds.
    let c =
        camera("camera_max_view_s = 10\ncamera_fps = 1\ncamera_width = 320\ncamera_quality = 50\n")
            .unwrap();
    assert_eq!(
        (c.max_view_s, c.fps, c.width, c.quality),
        (10, 1, CameraWidth::Half, 50)
    );
    let c = camera(
        "camera_max_view_s = 300\ncamera_fps = 10\ncamera_width = 640\ncamera_quality = 85\n",
    )
    .unwrap();
    assert_eq!(
        (c.max_view_s, c.fps, c.width, c.quality),
        (300, 10, CameraWidth::Full, 85)
    );

    // Ranges: refused values (validated with camera_view absent, i.e. false), 0 included.
    for v in [0u32, 9, 301, 100_000, u32::MAX] {
        assert_eq!(
            camera(&format!("camera_max_view_s = {v}\n")),
            Err(ConfigError::CameraMaxViewOutOfRange),
            "{v}"
        );
    }
    for v in [0u32, 11, 60, u32::MAX] {
        assert_eq!(
            camera(&format!("camera_fps = {v}\n")),
            Err(ConfigError::CameraFpsOutOfRange),
            "{v}"
        );
    }
    for v in [0u32, 160, 319, 321, 480, 639, 641, 1280, u32::MAX] {
        assert_eq!(
            camera(&format!("camera_width = {v}\n")),
            Err(ConfigError::InvalidCameraWidth),
            "{v}"
        );
    }
    for v in [0u8, 1, 49, 86, 100, 255] {
        assert_eq!(
            camera(&format!("camera_quality = {v}\n")),
            Err(ConfigError::CameraQualityOutOfRange),
            "{v}"
        );
    }
    // Also refused when the view is enabled.
    assert_eq!(
        camera(&format!("{RP}camera_view = true\ncamera_fps = 0\n")),
        Err(ConfigError::CameraFpsOutOfRange)
    );

    // Wrong TOML types and out-of-type values are Syntax.
    for bad in [
        "camera_view = \"yes\"\n",
        "camera_view = 1\n",
        "camera_view_funnel = \"true\"\n",
        "camera_fps = \"5\"\n",
        "camera_fps = 5.0\n",
        "camera_fps = -1\n",
        "camera_max_view_s = \"120\"\n",
        "camera_width = \"640\"\n",
        "camera_quality = 300\n",
        "camera_quality = -1\n",
        "camera_quality = \"70\"\n",
        "camera_fpss = 5\n",
        "camera = true\n",
    ] {
        assert_eq!(camera(bad), Err(ConfigError::Syntax), "{bad:?}");
    }

    // Check order: rp_id → funnel needs view → funnel needs allow_funnel → max view → fps →
    // width → quality.
    assert_eq!(
        camera("camera_view = true\ncamera_view_funnel = true\ncamera_fps = 0\n"),
        Err(ConfigError::CameraRequiresRpId)
    );
    assert_eq!(
        camera(&format!(
            "{RP}camera_view_funnel = true\ncamera_max_view_s = 0\n"
        )),
        Err(ConfigError::CameraFunnelRequiresCameraView)
    );
    assert_eq!(
        camera(&format!(
            "{RP}camera_view = true\ncamera_view_funnel = true\ncamera_max_view_s = 0\n"
        )),
        Err(ConfigError::CameraFunnelRequiresFunnel)
    );
    assert_eq!(
        camera("camera_max_view_s = 0\ncamera_fps = 0\ncamera_width = 1\ncamera_quality = 1\n"),
        Err(ConfigError::CameraMaxViewOutOfRange)
    );
    assert_eq!(
        camera("camera_fps = 0\ncamera_width = 1\ncamera_quality = 1\n"),
        Err(ConfigError::CameraFpsOutOfRange)
    );
    assert_eq!(
        camera("camera_width = 1\ncamera_quality = 1\n"),
        Err(ConfigError::InvalidCameraWidth)
    );

    // Fixed texts that never echo a value.
    let texts = [
        (
            ConfigError::CameraRequiresRpId,
            "camera_view requires rp_id",
        ),
        (
            ConfigError::CameraFunnelRequiresCameraView,
            "camera_view_funnel requires camera_view",
        ),
        (
            ConfigError::CameraFunnelRequiresFunnel,
            "camera_view_funnel requires allow_funnel",
        ),
        (
            ConfigError::CameraMaxViewOutOfRange,
            "camera_max_view_s out of range",
        ),
        (ConfigError::CameraFpsOutOfRange, "camera_fps out of range"),
        (
            ConfigError::InvalidCameraWidth,
            "camera_width must be 320 or 640",
        ),
        (
            ConfigError::CameraQualityOutOfRange,
            "camera_quality out of range",
        ),
    ];
    for (err, text) in texts {
        assert_eq!(err.to_string(), text);
    }
    for (input, value) in [
        ("camera_max_view_s = 4242\n", "4242"),
        ("camera_fps = 4243\n", "4243"),
        ("camera_width = 4244\n", "4244"),
        ("camera_quality = 86\n", "86"),
    ] {
        let err = camera(input).unwrap_err();
        assert!(!err.to_string().contains(value), "{err} echoes {value}");
    }

    // Camera keys do not disturb the rest of the configuration.
    let full = parse(&format!(
        "{RP}camera_view = true\ncamera_fps = 2\ncamera_width = 320\n"
    ))
    .unwrap();
    assert_eq!(full.auth.rp_id.as_deref(), Some("pc.tail1234.ts.net"));
    assert_eq!(
        full.camera,
        CameraConfig {
            enabled: true,
            fps: 2,
            width: CameraWidth::Half,
            ..CameraConfig::default()
        }
    );
}
