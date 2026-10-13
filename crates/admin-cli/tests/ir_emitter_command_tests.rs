//! `soos-admin camera ir-emitter probe|configure` (GitHub #378, PR 1 of #353; matrix
//! IRA17–IRA21). Everything runs against a fake V4L2 probe, a temporary sysfs tree and the
//! mock UVC extension-unit backend: no device, no root, no sudo.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use clap::Parser;
use soos_admin_cli::args::{CameraAction, Cli, Commands, IrEmitterAction, IrEmitterModeArg};
use soos_admin_cli::camera::CameraEnvironment;
use soos_admin_cli::ir_emitter::{
    check_ir_node, configure_ir_emitter, ensure_same_node, probe_ir_emitter,
    test_emitter_on_device, write_ir_emitter_section, ConfigureRequest, IrEmitterEnv, WriteOutcome,
    IR_EMITTER_BLOCK_BEGIN, IR_EMITTER_BLOCK_END, MAX_DEVICE_TEST_QUERIES,
};
use soos_admin_cli::AdminCliError;
use soos_camera_v4l::diagnostics::{
    ProbeFailure, V4lDeviceProbe, V4lNodeDetails, V4L2_CAP_DEVICE_CAPS, V4L2_CAP_STREAMING,
    V4L2_CAP_VIDEO_CAPTURE,
};
use soos_camera_v4l::ir_emitter_config::{
    read_daemon_ir_emitter_config, IrEmitterMode, IrEmitterSettings, UsbDeviceId,
    XuEmitterDescriptor,
};
use soos_camera_v4l::ir_mock::MockXuControl;
use soos_camera_v4l::uvc_xu::{XuQuery, PROBE_MAX_SELECTOR};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CAPS: u32 = V4L2_CAP_VIDEO_CAPTURE | V4L2_CAP_STREAMING | V4L2_CAP_DEVICE_CAPS;
const ORIGINAL: [u8; 3] = [9, 9, 9];

#[derive(Default)]
struct FakeProbe {
    table: BTreeMap<PathBuf, V4lNodeDetails>,
}

impl FakeProbe {
    fn node(mut self, path: &Path, card: &str, driver: &str, fourcc: &[u8; 4]) -> Self {
        self.table.insert(
            path.to_path_buf(),
            V4lNodeDetails {
                driver: driver.to_string(),
                card_name: card.to_string(),
                bus_info: "usb-0000:00:14.0-6".to_string(),
                device_caps: CAPS,
                fourccs: vec![*fourcc],
                // IR modules stream small frames; the RGB sibling streams HD.
                frame_sizes: if fourcc == b"GREY" {
                    vec![(640, 360)]
                } else {
                    vec![(1280, 720)]
                },
            },
        );
        self
    }
}

impl V4lDeviceProbe for FakeProbe {
    fn details(&self, dev_path: &Path) -> Result<V4lNodeDetails, ProbeFailure> {
        self.table
            .get(dev_path)
            .cloned()
            .ok_or(ProbeFailure::NotFound)
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    camera: CameraEnvironment,
    probe: FakeProbe,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let sysfs = dir.path().join("sys/class/video4linux");
        let usb = dir.path().join("sys/usb/1-6");
        let iface = usb.join("1-6:1.2");
        std::fs::create_dir_all(&iface).unwrap();
        std::fs::write(usb.join("idVendor"), "04f2\n").unwrap();
        std::fs::write(usb.join("idProduct"), "b6d9\n").unwrap();
        for node in ["video0", "video2", "video4"] {
            std::fs::create_dir_all(sysfs.join(node)).unwrap();
        }
        // video0 (RGB) and video2 (IR) belong to the USB device; video4 has no USB parent.
        std::os::unix::fs::symlink(&iface, sysfs.join("video0/device")).unwrap();
        std::os::unix::fs::symlink(&iface, sysfs.join("video2/device")).unwrap();
        let dev = dir.path().join("dev");
        std::fs::create_dir_all(&dev).unwrap();
        let probe = FakeProbe::default()
            .node(
                &dev.join("video0"),
                "Integrated Camera: Integrated C",
                "uvcvideo",
                b"YUYV",
            )
            .node(
                &dev.join("video2"),
                "Integrated Camera: Integrated I",
                "uvcvideo",
                b"GREY",
            )
            .node(
                &dev.join("video4"),
                "Integrated Camera: Integrated I",
                "uvcvideo",
                b"GREY",
            )
            .node(&dev.join("video6"), "Loopback IR", "v4l2 loopback", b"GREY");
        let camera = CameraEnvironment {
            sysfs_dir: sysfs,
            dev_dir: dev.clone(),
            by_id_dir: dir.path().join("by-id"),
        };
        Self { dir, camera, probe }
    }

    fn env(&self, euid: u32) -> IrEmitterEnv<'_> {
        IrEmitterEnv {
            camera: &self.camera,
            probe: &self.probe,
            euid,
        }
    }

    fn dev(&self, node: &str) -> PathBuf {
        self.camera.dev_dir.join(node)
    }

    fn config(&self) -> PathBuf {
        self.dir.path().join("daemon.toml")
    }

    fn request(&self) -> ConfigureRequest {
        ConfigureRequest {
            device: self.dev("video2"),
            mode: IrEmitterMode::Off,
            unit: 14,
            selector: 6,
            on: vec![1, 3, 3],
            off: vec![1, 3, 0],
            config_path: self.config(),
            dry_run: false,
            test_on_device: false,
        }
    }
}

fn xu() -> MockXuControl {
    MockXuControl::new()
        .with_control(14, 6, 0x03, ORIGINAL.to_vec())
        .with_control(14, 2, 0x01, vec![0x55])
}

fn writes(mock: &MockXuControl) -> usize {
    mock.requests()
        .iter()
        .filter(|(_, _, q)| *q == XuQuery::SetCur)
        .count()
}

fn descriptor() -> XuEmitterDescriptor {
    XuEmitterDescriptor::new(
        UsbDeviceId {
            vendor: 0x04f2,
            product: 0xb6d9,
        },
        14,
        6,
        vec![1, 3, 3],
        vec![1, 3, 0],
    )
    .unwrap()
}

fn ir_err(result: Result<impl std::fmt::Debug, AdminCliError>) -> String {
    match result {
        Err(AdminCliError::IrEmitter(msg)) => msg,
        other => panic!("expected AdminCliError::IrEmitter, got {other:?}"),
    }
}

/// IRA17: the node gate needs root, an Infrared uvcvideo capture node; it reports the USB id.
#[test]
fn test_ira_node_gate_requires_root_and_an_ir_uvc_node() {
    let fx = Fixture::new();
    let msg = ir_err(check_ir_node(&fx.env(1000), &fx.dev("video2")));
    assert!(msg.contains("root"), "{msg}");
    let msg = ir_err(check_ir_node(&fx.env(0), &fx.dev("video0")));
    assert!(msg.to_lowercase().contains("infrared"), "{msg}");
    let msg = ir_err(check_ir_node(&fx.env(0), &fx.dev("video6")));
    assert!(!msg.is_empty());
    let msg = ir_err(check_ir_node(&fx.env(0), &fx.dev("video9")));
    assert!(!msg.is_empty());
    let node = check_ir_node(&fx.env(0), &fx.dev("video2")).unwrap();
    assert_eq!(node.node, fx.dev("video2"));
    assert_eq!(
        node.usb_id,
        Some(UsbDeviceId {
            vendor: 0x04f2,
            product: 0xb6d9
        })
    );
    let no_usb = check_ir_node(&fx.env(0), &fx.dev("video4")).unwrap();
    assert_eq!(no_usb.usb_id, None);
}

/// IRA18: the probe is read-only, lists the controls and says whether the configured
/// descriptor matches this node.
#[test]
fn test_ira_probe_is_read_only_and_reports_controls() {
    let fx = Fixture::new();
    std::fs::write(
        fx.config(),
        IrEmitterSettings {
            mode: IrEmitterMode::Auto,
            descriptor: Some(descriptor()),
        }
        .to_toml_section(),
    )
    .unwrap();
    let mut mock = xu();
    let report = probe_ir_emitter(
        &fx.env(0),
        &fx.dev("video2"),
        14..=14,
        &fx.config(),
        &mut mock,
    )
    .unwrap();
    assert_eq!(writes(&mock), 0, "the probe never writes");
    assert_eq!(report.controls.len(), 2);
    assert!(report.configured_matches_node);
    let json = report.to_json();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["usb_id"], "04f2:b6d9");
    assert_eq!(value["controls"].as_array().unwrap().len(), 2);
    let table = report.format_table();
    assert!(
        table.contains("090909") || table.contains("09 09 09"),
        "{table}"
    );
    assert!(mock
        .requests()
        .iter()
        .all(|(u, s, _)| *u == 14 && (1..=PROBE_MAX_SELECTOR).contains(s)));

    let msg = ir_err(probe_ir_emitter(
        &fx.env(1000),
        &fx.dev("video2"),
        14..=14,
        &fx.config(),
        &mut xu(),
    ));
    assert!(msg.contains("root"), "{msg}");
}

/// IRA18: a probe with a missing configuration still works and reports no match.
#[test]
fn test_ira_probe_without_configuration() {
    let fx = Fixture::new();
    let mut mock = xu();
    let report = probe_ir_emitter(
        &fx.env(0),
        &fx.dev("video2"),
        14..=14,
        &fx.config(),
        &mut mock,
    )
    .unwrap();
    assert!(!report.configured_matches_node);
    assert_eq!(writes(&mock), 0);
}

/// IRA19: `--dry-run` renders the section and touches neither the file nor the device.
#[test]
fn test_ira_configure_dry_run_writes_nothing() {
    let fx = Fixture::new();
    let mut mock = xu();
    let request = ConfigureRequest {
        dry_run: true,
        ..fx.request()
    };
    let report = configure_ir_emitter(&fx.env(0), &request, &mut mock).unwrap();
    assert!(report.section.contains("[camera.ir_emitter]"));
    assert!(report.section.contains("04f2:b6d9"));
    assert_eq!(report.written, None);
    assert!(report.device_test.is_none());
    assert!(!fx.config().exists());
    assert_eq!(writes(&mock), 0);
}

/// IRA19: configure writes the managed block, preserves the rest, and replaces its own block.
#[test]
fn test_ira_configure_writes_and_replaces_managed_block() {
    let fx = Fixture::new();
    let original = "# operator comment\n[pipeline]\nsensor_preference = \"ir\"\n";
    std::fs::write(fx.config(), original).unwrap();
    let mut mock = xu();
    let report = configure_ir_emitter(&fx.env(0), &fx.request(), &mut mock).unwrap();
    assert_eq!(report.written, Some(WriteOutcome::Updated));
    assert_eq!(
        writes(&mock),
        0,
        "configure without --test-on-device never writes the device"
    );
    let text = std::fs::read_to_string(fx.config()).unwrap();
    assert!(text.starts_with(original), "{text}");
    assert_eq!(text.matches(IR_EMITTER_BLOCK_BEGIN).count(), 1);
    assert_eq!(text.matches(IR_EMITTER_BLOCK_END).count(), 1);
    let settings = read_daemon_ir_emitter_config(&fx.config()).unwrap();
    assert_eq!(settings.descriptor, Some(descriptor()));

    assert_eq!(settings.mode, IrEmitterMode::Off, "the default mode is off");

    let auto = ConfigureRequest {
        mode: IrEmitterMode::Auto,
        test_on_device: true,
        ..fx.request()
    };
    let mut mock = xu();
    configure_ir_emitter(&fx.env(0), &auto, &mut mock).unwrap();
    assert_eq!(mock.value(14, 6).unwrap(), ORIGINAL.to_vec());
    let text = std::fs::read_to_string(fx.config()).unwrap();
    assert!(text.starts_with(original));
    assert_eq!(text.matches(IR_EMITTER_BLOCK_BEGIN).count(), 1);
    assert_eq!(
        read_daemon_ir_emitter_config(&fx.config()).unwrap().mode,
        IrEmitterMode::Auto
    );
}

/// IRA19: a missing configuration file is created with mode 0644.
#[test]
fn test_ira_configure_creates_missing_file() {
    use std::os::unix::fs::PermissionsExt as _;
    let fx = Fixture::new();
    let report = configure_ir_emitter(&fx.env(0), &fx.request(), &mut xu()).unwrap();
    assert_eq!(report.written, Some(WriteOutcome::Created));
    let mode = std::fs::metadata(fx.config()).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o644);
}

/// IRA20: refusals leave the configuration byte-for-byte unchanged.
#[test]
fn test_ira_configure_refusals_leave_file_unchanged() {
    let fx = Fixture::new();
    let hand_written = "[camera.ir_emitter]\nmode = \"off\"\n";
    std::fs::write(fx.config(), hand_written).unwrap();
    let msg = ir_err(configure_ir_emitter(&fx.env(0), &fx.request(), &mut xu()));
    assert!(msg.contains("outside"), "{msg}");
    assert_eq!(std::fs::read_to_string(fx.config()).unwrap(), hand_written);

    let unbalanced = format!("[pipeline]\n{IR_EMITTER_BLOCK_BEGIN}\n");
    std::fs::write(fx.config(), &unbalanced).unwrap();
    ir_err(configure_ir_emitter(&fx.env(0), &fx.request(), &mut xu()));
    assert_eq!(std::fs::read_to_string(fx.config()).unwrap(), unbalanced);

    // Payload length differs from GET_LEN of the control.
    std::fs::write(fx.config(), "[pipeline]\n").unwrap();
    let long = ConfigureRequest {
        on: vec![1, 3, 3, 0],
        off: vec![1, 3, 0, 0],
        ..fx.request()
    };
    let msg = ir_err(configure_ir_emitter(&fx.env(0), &long, &mut xu()));
    assert!(msg.contains('3') && msg.contains('4'), "{msg}");
    // Read-only control.
    let ro = ConfigureRequest {
        selector: 2,
        on: vec![1],
        off: vec![0],
        ..fx.request()
    };
    ir_err(configure_ir_emitter(&fx.env(0), &ro, &mut xu()));
    // Invalid descriptor (identical payloads).
    let same = ConfigureRequest {
        off: vec![1, 3, 3],
        ..fx.request()
    };
    ir_err(configure_ir_emitter(&fx.env(0), &same, &mut xu()));
    // Node without a USB id.
    let no_usb = ConfigureRequest {
        device: fx.dev("video4"),
        ..fx.request()
    };
    ir_err(configure_ir_emitter(&fx.env(0), &no_usb, &mut xu()));
    // Not root.
    ir_err(configure_ir_emitter(
        &fx.env(1000),
        &fx.request(),
        &mut xu(),
    ));
    assert_eq!(
        std::fs::read_to_string(fx.config()).unwrap(),
        "[pipeline]\n"
    );
}

/// IRA20: `--mode auto` is refused unless the device test ran and succeeded in the same run.
#[test]
fn test_ira_auto_requires_a_successful_device_test() {
    let fx = Fixture::new();
    std::fs::write(fx.config(), "[pipeline]\n").unwrap();
    let auto = ConfigureRequest {
        mode: IrEmitterMode::Auto,
        ..fx.request()
    };
    let mut mock = xu();
    let msg = ir_err(configure_ir_emitter(&fx.env(0), &auto, &mut mock));
    assert!(msg.contains("--test-on-device"), "{msg}");
    assert_eq!(writes(&mock), 0);
    let mut failing = xu();
    failing.fail_set_after(1);
    let tested = ConfigureRequest {
        test_on_device: true,
        ..auto
    };
    ir_err(configure_ir_emitter(&fx.env(0), &tested, &mut failing));
    assert_eq!(
        std::fs::read_to_string(fx.config()).unwrap(),
        "[pipeline]\n"
    );
}

/// IRA20: a symbolic link configuration is refused (its target is never replaced).
#[test]
fn test_ira_configure_refuses_symlinked_config() {
    let fx = Fixture::new();
    let target = fx.dir.path().join("real.toml");
    std::fs::write(&target, "[pipeline]\n").unwrap();
    std::os::unix::fs::symlink(&target, fx.config()).unwrap();
    ir_err(write_ir_emitter_section(
        &fx.config(),
        &IrEmitterSettings {
            mode: IrEmitterMode::Auto,
            descriptor: Some(descriptor()),
        },
    ));
    assert!(std::fs::symlink_metadata(fx.config())
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "[pipeline]\n");
}

/// IRA20: a result that would not load in soos-daemon is never written.
#[test]
fn test_ira_configure_refuses_text_that_would_not_parse() {
    let fx = Fixture::new();
    let broken = "[pipeline\n";
    std::fs::write(fx.config(), broken).unwrap();
    ir_err(configure_ir_emitter(&fx.env(0), &fx.request(), &mut xu()));
    assert_eq!(std::fs::read_to_string(fx.config()).unwrap(), broken);
}

/// IRA21: the explicit device test writes on, off, verifies both and restores the original,
/// with the termination signals blocked meanwhile, within the query bound.
#[test]
fn test_ira_device_test_restores_original_with_signals_blocked() {
    use nix::sys::signal::{SigSet, Signal};
    let before = SigSet::thread_get_mask().unwrap();
    assert!(!before.contains(Signal::SIGINT), "test precondition");
    let mut mock = xu();
    let mut seen_blocked = Vec::new();
    let report = test_emitter_on_device(&mut mock, &descriptor(), &mut || {
        let mask = SigSet::thread_get_mask().unwrap();
        seen_blocked.push(
            [
                Signal::SIGINT,
                Signal::SIGTERM,
                Signal::SIGHUP,
                Signal::SIGQUIT,
                Signal::SIGTSTP,
            ]
            .iter()
            .all(|s| mask.contains(*s)),
        );
    })
    .unwrap();
    assert!(!seen_blocked.is_empty() && seen_blocked.iter().all(|b| *b));
    assert!(report.on_readback_ok && report.off_readback_ok && report.restored);
    assert_eq!(mock.value(14, 6).unwrap(), ORIGINAL.to_vec());
    assert!(mock.requests().len() <= MAX_DEVICE_TEST_QUERIES);
    let set_values = writes(&mock);
    assert_eq!(set_values, 3, "on, off, restore");
    let after = SigSet::thread_get_mask().unwrap();
    assert!(
        !after.contains(Signal::SIGINT),
        "the previous mask is restored"
    );
}

/// IRA21: when a write fails mid-test the original value is still restored.
#[test]
fn test_ira_device_test_restores_after_failure() {
    let mut mock = xu();
    mock.fail_set_after(1);
    let result = test_emitter_on_device(&mut mock, &descriptor(), &mut || {});
    let msg = ir_err(result);
    assert!(!msg.is_empty());
    assert!(
        writes(&mock) >= 2,
        "a restore was attempted after the failure"
    );
    assert!(mock.requests().len() <= MAX_DEVICE_TEST_QUERIES);
}

/// IRA21: configure only reaches the device with --test-on-device.
#[test]
fn test_ira_configure_test_on_device_flag() {
    let fx = Fixture::new();
    let mut mock = xu();
    let request = ConfigureRequest {
        test_on_device: true,
        dry_run: true,
        ..fx.request()
    };
    let report = configure_ir_emitter(&fx.env(0), &request, &mut mock).unwrap();
    let test = report.device_test.expect("device test ran");
    assert!(test.restored);
    assert_eq!(mock.value(14, 6).unwrap(), ORIGINAL.to_vec());
    assert_eq!(writes(&mock), 3);
}

/// IRA17: command-line shape: hex payloads, bounded unit/selector, defaults.
#[test]
fn test_ira_cli_arguments() {
    let cli = Cli::try_parse_from([
        "soos-admin",
        "camera",
        "ir-emitter",
        "configure",
        "/dev/video2",
        "--unit",
        "14",
        "--selector",
        "6",
        "--on",
        "0x010303",
        "--off",
        "010300",
    ])
    .unwrap();
    let Commands::Camera(camera) = cli.command else {
        panic!("camera");
    };
    let CameraAction::IrEmitter(ir) = camera.action else {
        panic!("ir-emitter");
    };
    let IrEmitterAction::Configure(args) = ir.action else {
        panic!("configure");
    };
    assert_eq!(args.on.0, vec![1, 3, 3]);
    assert_eq!(args.off.0, vec![1, 3, 0]);
    assert_eq!((args.unit, args.selector), (14, 6));
    assert_eq!(args.mode, IrEmitterModeArg::Off);
    assert!(!args.dry_run && !args.test_on_device);
    assert_eq!(args.config, PathBuf::from("/etc/soos/daemon.toml"));

    let base = [
        "soos-admin",
        "camera",
        "ir-emitter",
        "configure",
        "/dev/video2",
    ];
    for bad in [
        vec![
            "--unit",
            "0",
            "--selector",
            "6",
            "--on",
            "01",
            "--off",
            "00",
        ],
        vec![
            "--unit",
            "256",
            "--selector",
            "6",
            "--on",
            "01",
            "--off",
            "00",
        ],
        vec!["--unit", "1", "--selector", "6", "--on", "0", "--off", "00"],
        vec![
            "--unit",
            "1",
            "--selector",
            "6",
            "--on",
            "zz",
            "--off",
            "00",
        ],
        vec!["--unit", "1", "--selector", "6", "--on", "", "--off", "00"],
        vec![
            "--unit",
            "1",
            "--selector",
            "6",
            "--on",
            &"00".repeat(65),
            "--off",
            "00",
        ],
    ] {
        let mut argv: Vec<&str> = base.to_vec();
        argv.extend(bad.iter().copied());
        assert!(Cli::try_parse_from(&argv).is_err(), "{argv:?}");
    }

    let cli = Cli::try_parse_from([
        "soos-admin",
        "camera",
        "ir-emitter",
        "probe",
        "/dev/video2",
        "--unit",
        "14",
        "--json",
    ])
    .unwrap();
    let Commands::Camera(camera) = cli.command else {
        panic!("camera");
    };
    let CameraAction::IrEmitter(ir) = camera.action else {
        panic!("ir-emitter");
    };
    let IrEmitterAction::Probe(args) = ir.action else {
        panic!("probe");
    };
    assert_eq!(args.unit, Some(14));
    assert!(args.json);
}

/// IRA21b (candid review of #378): the documented bound is the real maximum of a
/// `configure --test-on-device` run (2 pre-checks, 3 to open, on/read/off/read, restore and the
/// drop-time retry) and it is enforced in code.
#[test]
fn test_ira_configure_device_test_stays_within_the_enforced_bound() {
    assert_eq!(MAX_DEVICE_TEST_QUERIES, 11);
    let fx = Fixture::new();
    let request = ConfigureRequest {
        test_on_device: true,
        dry_run: true,
        ..fx.request()
    };
    let mut mock = xu();
    configure_ir_emitter(&fx.env(0), &request, &mut mock).unwrap();
    assert_eq!(
        mock.requests().len(),
        10,
        "nominal run: {:?}",
        mock.requests()
    );

    // Worst case: both payloads written and read back, then the restore and its drop-time
    // retry fail.
    let mut worst = xu();
    worst.fail_set_after(2);
    ir_err(configure_ir_emitter(&fx.env(0), &request, &mut worst));
    assert_eq!(
        worst.requests().len(),
        MAX_DEVICE_TEST_QUERIES,
        "{:?}",
        worst.requests()
    );
}

/// IRA17b (candid review of #378): the opened descriptor must be the node that was classified
/// (same device number); an unknown or different device number is refused.
#[test]
fn test_ira_opened_descriptor_must_be_the_classified_node() {
    assert!(ensure_same_node(Some(0x5100), 0x5100).is_ok());
    let msg = ir_err(ensure_same_node(Some(0x5100), 0x5102));
    assert!(msg.contains("changed"), "{msg}");
    ir_err(ensure_same_node(None, 0x5100));
}
