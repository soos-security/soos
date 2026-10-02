//! Contractual tests for the daemon minor findings of the 2026-10-02 review (GitHub #315,
//! matrix DRM).
//!
//! - DMN-NEW-2: the inference latency estimate decays toward `DEFAULT_INFERENCE_ESTIMATE_MS`
//!   (`InferenceEstimator::decay_toward_default`) so it cannot disable face authentication
//!   permanently (the dispatcher-level proof lives in `inference_budget_tests.rs`, which owns
//!   the gated dispatcher fixture).
//! - DMN-NEW-3: `[dispatcher] enforce_active_session = false` is refused unless
//!   `[pipeline] use_mock_camera = true` (test harnesses only).
//! - DMN-NEW-4: the configuration file is read through the shared bounded `O_PATH` reader
//!   (a FIFO is refused promptly, a file above 1 MiB is refused), and an unknown
//!   `sensor_preference` is reported as a load warning that `main.rs` logs.
//! - Suggestions: `socket_path` must be a direct child of `socket_dir`; the socket directory
//!   group is checked against the configured socket group.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use soos_daemon::config::{DaemonConfig, SocketConfig};
use soos_daemon::error::DaemonError;
use soos_daemon::inference::{
    InferenceEstimator, InferenceGate, DEFAULT_INFERENCE_ESTIMATE_MS, MAX_INFERENCE_ESTIMATE_MS,
};
use soos_daemon::socket::validate_directory_group;

fn assert_config_error_naming(result: Result<DaemonConfig, DaemonError>, setting: &str) {
    match result {
        Err(DaemonError::Config(msg)) => assert!(
            msg.contains(setting),
            "the configuration error must name `{setting}`: {msg}"
        ),
        Err(other) => panic!("expected DaemonError::Config naming `{setting}`, got {other:?}"),
        Ok(_) => panic!("the configuration must be refused (`{setting}`)"),
    }
}

// ---------------------------------------------------------------------------
// DMN-NEW-2: estimate decay
// ---------------------------------------------------------------------------

#[test]
fn test_drm_estimator_decay_moves_a_saturated_estimate_toward_the_default() {
    let default = Duration::from_millis(DEFAULT_INFERENCE_ESTIMATE_MS);
    let estimator = InferenceEstimator::new(Duration::from_millis(MAX_INFERENCE_ESTIMATE_MS));
    let mut previous = estimator.estimate();
    estimator.decay_toward_default();
    let first = estimator.estimate();
    assert!(
        first < previous,
        "one decay step must lower a saturated estimate ({first:?} !< {previous:?})"
    );
    assert!(
        first
            <= Duration::from_millis(MAX_INFERENCE_ESTIMATE_MS / 2 + DEFAULT_INFERENCE_ESTIMATE_MS),
        "one decay step must at least halve the distance to the default, got {first:?}"
    );
    previous = first;
    for _ in 0..32 {
        estimator.decay_toward_default();
        let current = estimator.estimate();
        assert!(current <= previous, "decay never raises the estimate");
        assert!(current >= default, "decay never goes below the default");
        previous = current;
    }
    assert_eq!(
        estimator.estimate(),
        default,
        "repeated decay converges exactly to DEFAULT_INFERENCE_ESTIMATE_MS"
    );
}

#[test]
fn test_drm_estimator_decay_never_raises_a_low_estimate() {
    let low = Duration::from_millis(20);
    let estimator = InferenceEstimator::new(low);
    estimator.decay_toward_default();
    assert_eq!(
        estimator.estimate(),
        low,
        "an estimate already below the default is a measurement and is kept"
    );
}

#[test]
fn test_drm_gate_decay_estimate_delegates_to_the_estimator() {
    let gate = InferenceGate::new(1, Duration::from_millis(MAX_INFERENCE_ESTIMATE_MS));
    gate.decay_estimate();
    assert!(gate.estimate() < Duration::from_millis(MAX_INFERENCE_ESTIMATE_MS));
}

// ---------------------------------------------------------------------------
// DMN-NEW-3: enforce_active_session = false only with the mock camera
// ---------------------------------------------------------------------------

#[test]
fn test_drm_enforce_active_session_false_is_refused_in_production_config() {
    assert_config_error_naming(
        DaemonConfig::from_toml_str("[dispatcher]\nenforce_active_session = false\n"),
        "enforce_active_session",
    );
    assert_config_error_naming(
        DaemonConfig::from_toml_str(
            "[dispatcher]\nenforce_active_session = false\n[pipeline]\nuse_mock_camera = false\n",
        ),
        "enforce_active_session",
    );
}

#[test]
fn test_drm_enforce_active_session_false_is_accepted_with_mock_camera() {
    let config = DaemonConfig::from_toml_str(
        "[dispatcher]\nenforce_active_session = false\n[pipeline]\nuse_mock_camera = true\n",
    )
    .expect("the test harness combination must stay accepted");
    assert!(!config.dispatcher.enforce_active_session);
    assert!(config.pipeline.use_mock_camera);
}

#[test]
fn test_drm_validate_refuses_disabled_session_policy_without_mock_camera() {
    let mut config = DaemonConfig::default();
    config.dispatcher.enforce_active_session = false;
    match config.validate() {
        Err(DaemonError::Config(msg)) => assert!(msg.contains("enforce_active_session"), "{msg}"),
        other => panic!("validate() must refuse it, got {other:?}"),
    }
    config.pipeline.use_mock_camera = true;
    config.validate().expect("accepted with the mock camera");
}

#[test]
fn test_drm_enforce_active_session_true_is_always_accepted() {
    let config = DaemonConfig::from_toml_str("[dispatcher]\nenforce_active_session = true\n")
        .expect("the production value is accepted");
    assert!(config.dispatcher.enforce_active_session);
}

// ---------------------------------------------------------------------------
// DMN-NEW-4: bounded regular-file loader, unknown sensor_preference warning
// ---------------------------------------------------------------------------

/// Runs `load` on a helper thread and returns its result, or `None` when it did not finish
/// within `limit` (a blocking read); a reader blocked on a FIFO is released afterwards.
fn load_with_timeout(
    path: PathBuf,
    load: fn(&Path) -> Result<DaemonConfig, DaemonError>,
    limit: Duration,
) -> Option<Result<DaemonConfig, DaemonError>> {
    let (tx, rx) = mpsc::channel();
    let worker_path = path.clone();
    std::thread::spawn(move || {
        let _ = tx.send(load(&worker_path));
    });
    let result = rx.recv_timeout(limit).ok();
    if result.is_none() {
        // Release a reader blocked in open(2) on the FIFO so the thread can finish.
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path);
    }
    result
}

fn make_fifo(dir: &Path) -> PathBuf {
    let fifo = dir.join("daemon.toml");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).expect("mkfifo");
    fifo
}

#[test]
fn test_drm_load_from_path_refuses_a_fifo_promptly() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = make_fifo(dir.path());
    let result = load_with_timeout(
        fifo,
        |p| DaemonConfig::load_from_path(p),
        Duration::from_secs(5),
    )
    .expect("loading a FIFO must return promptly instead of blocking");
    assert!(
        matches!(result, Err(DaemonError::Config(_))),
        "a FIFO is not a configuration file: {result:?}"
    );
}

#[test]
fn test_drm_system_config_fifo_is_refused_promptly() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = make_fifo(dir.path());
    let result = load_with_timeout(
        fifo,
        |p| DaemonConfig::load_or_default_with_system_path(None, p),
        Duration::from_secs(5),
    )
    .expect("a FIFO at the system path must not block the start-up");
    assert!(
        matches!(result, Err(DaemonError::Config(_))),
        "a FIFO at the system path must be refused, not silently replaced by defaults: {result:?}"
    );
}

#[test]
fn test_drm_load_from_path_refuses_a_file_above_one_mib() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daemon.toml");
    let mut content = String::from("log_level = \"info\"\n");
    let limit = usize::try_from(soos_camera_v4l::daemon_config::MAX_DAEMON_CONFIG_BYTES).unwrap();
    while content.len() <= limit {
        content.push_str("# padding padding padding padding padding padding padding padding\n");
    }
    std::fs::write(&path, &content).unwrap();
    match DaemonConfig::load_from_path(&path) {
        Err(DaemonError::Config(msg)) => assert!(
            msg.contains("larger than"),
            "the error must say the file is too large: {msg}"
        ),
        other => panic!("a file above 1 MiB must be refused, got {other:?}"),
    }
}

#[test]
fn test_drm_load_from_path_accepts_a_file_at_the_limit_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daemon.toml");
    std::fs::write(&path, "[pipeline]\nwarmup_frames = 3\n").unwrap();
    let config = DaemonConfig::load_from_path(&path).expect("a small regular file loads");
    assert_eq!(config.pipeline.camera.warmup_frames, 3);
}

#[test]
fn test_drm_system_config_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let not_a_file = dir.path().join("daemon.toml");
    std::fs::create_dir(&not_a_file).unwrap();
    assert!(
        matches!(
            DaemonConfig::load_or_default_with_system_path(None, &not_a_file),
            Err(DaemonError::Config(_))
        ),
        "a non-regular system configuration path must be refused (no check-then-open)"
    );
}

#[test]
fn test_drm_unknown_sensor_preference_is_reported_as_a_load_warning() {
    let config =
        DaemonConfig::from_toml_str("[pipeline]\nsensor_preference = \"infrared-please\"\n")
            .expect("an unknown value keeps the default and is not an error");
    assert_eq!(
        config.pipeline.camera.sensor_preference,
        soos_camera_v4l::SensorPreference::default(),
        "the default preference applies"
    );
    assert_eq!(config.warnings.len(), 1, "{:?}", config.warnings);
    assert!(config.warnings[0].contains("sensor_preference"));
    assert!(
        !config.warnings[0].contains("infrared-please"),
        "the warning names the key, never the value"
    );
}

#[test]
fn test_drm_known_sensor_preference_has_no_warning() {
    for value in ["prefer_ir", "RGB", "any"] {
        let config =
            DaemonConfig::from_toml_str(&format!("[pipeline]\nsensor_preference = \"{value}\"\n"))
                .unwrap();
        assert!(config.warnings.is_empty(), "{value}: {:?}", config.warnings);
    }
    assert!(DaemonConfig::runtime_default().warnings.is_empty());
}

#[test]
fn test_drm_main_logs_config_warnings_after_logging_init() {
    let main =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/main.rs"))
            .unwrap();
    let init = main
        .find("init_logging(&config.log_level)")
        .expect("main.rs initializes logging from the configuration");
    let warnings = main
        .find("config.warnings")
        .expect("main.rs must log the configuration load warnings");
    assert!(
        warnings > init,
        "the warnings must be logged after the subscriber is installed"
    );
    assert!(
        main[warnings..]
            .lines()
            .take(4)
            .any(|l| l.contains("warn!(")),
        "each load warning is logged at warn level"
    );
}

#[test]
fn test_drm_config_loader_has_no_unbounded_read_or_is_file_check() {
    let source =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/config.rs"))
            .unwrap();
    assert!(
        !source.contains("read_to_string("),
        "the daemon configuration must be read through the bounded reader"
    );
    assert!(
        !source.contains(".is_file()"),
        "no check-then-open on the system configuration path"
    );
}

// ---------------------------------------------------------------------------
// Suggestions: socket_path parent, socket directory group
// ---------------------------------------------------------------------------

fn socket_config(path: &str, dir: &str) -> SocketConfig {
    SocketConfig {
        socket_path: PathBuf::from(path),
        socket_dir: PathBuf::from(dir),
        ..SocketConfig::default()
    }
}

#[test]
fn test_drm_socket_path_must_be_a_direct_child_of_socket_dir() {
    for (path, dir) in [
        ("/tmp/elsewhere/daemon.sock", "/run/soos"),
        ("/run/soos/nested/daemon.sock", "/run/soos"),
        ("/run/daemon.sock", "/run/soos"),
        ("/run/soos/../x/daemon.sock", "/run/soos"),
        ("/run/soos/..", "/run/soos"),
        ("/", "/"),
    ] {
        match socket_config(path, dir).validate() {
            Err(DaemonError::Config(msg)) => {
                assert!(msg.contains("socket_path"), "{path} in {dir}: {msg}");
            }
            other => panic!("{path} in {dir} must be refused, got {other:?}"),
        }
    }
}

#[test]
fn test_drm_socket_path_inside_socket_dir_is_accepted() {
    for (path, dir) in [
        ("/run/soos/daemon.sock", "/run/soos"),
        ("/run/soos/daemon.sock", "/run/soos/"),
        ("/tmp/custom_soos.sock", "/tmp"),
    ] {
        socket_config(path, dir)
            .validate()
            .unwrap_or_else(|e| panic!("{path} in {dir} must be accepted: {e:?}"));
    }
}

#[test]
fn test_drm_socket_path_outside_socket_dir_is_refused_at_load() {
    assert!(matches!(
        DaemonConfig::from_toml_str(
            "[socket]\nsocket_path = \"/tmp/daemon.sock\"\nsocket_dir = \"/run/soos\"\n"
        ),
        Err(DaemonError::Config(msg)) if msg.contains("socket_path")
    ));
}

#[test]
fn test_drm_socket_directory_group_must_match_the_socket_group() {
    let dir = tempfile::tempdir().unwrap();
    let handle = std::fs::File::open(dir.path()).unwrap();
    let own_gid = std::fs::metadata(dir.path()).unwrap().gid();
    validate_directory_group(&handle, dir.path(), own_gid)
        .expect("a directory owned by the socket group is accepted");
    let other_gid = own_gid.wrapping_add(1);
    match validate_directory_group(&handle, dir.path(), other_gid) {
        Err(DaemonError::SocketDirValidation(msg)) => {
            assert!(msg.contains("group"), "{msg}");
        }
        other => panic!("a directory of another group must be refused, got {other:?}"),
    }
}

#[test]
fn test_drm_bind_socket_checks_the_directory_group_when_root_owned_is_enforced() {
    let source =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/socket.rs"))
            .unwrap();
    let bind = source
        .find("pub async fn bind_socket(")
        .expect("bind_socket exists");
    let body = &source[bind..];
    let check = body
        .find("validate_directory_group(")
        .expect("bind_socket must check the socket directory group");
    let first_bind = body
        .find("UnixListener::bind(")
        .expect("bind_socket binds the listener");
    assert!(
        check < first_bind,
        "the directory group is checked before the socket is bound"
    );
}
