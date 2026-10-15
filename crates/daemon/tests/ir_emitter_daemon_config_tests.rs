//! `soos-daemon` loads `[camera.ir_emitter]` fail-closed (GitHub #378, PR 1 of #353; matrix
//! IRA16): absent means off, every invalid value refuses start-up without echoing the value,
//! and `mode = "auto"` is accepted with a start-up warning (the emitter is not driven yet).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use soos_camera_v4l::ir_emitter_config::{IrEmitterMode, UsbDeviceId};
use soos_daemon::config::DaemonConfig;
use soos_daemon::error::DaemonError;

const SECTION: &str = r#"
[camera.ir_emitter]
mode = "auto"
device = "04f2:b6d9"
unit = 14
selector = 6
on = [1, 3, 3, 0, 0, 0, 0, 0, 0]
off = [1, 3, 0, 0, 0, 0, 0, 0, 0]
"#;

fn rejected(toml: &str) -> String {
    match DaemonConfig::from_toml_str(toml) {
        Err(DaemonError::Config(msg)) => msg,
        Err(other) => panic!("must yield DaemonError::Config, got {other:?}"),
        Ok(_) => panic!("`{toml}` must be refused, but was accepted"),
    }
}

/// IRA16: no `[camera]` table: the emitter stays off and nothing is reported.
#[test]
fn test_ira_daemon_default_is_off() {
    let config = DaemonConfig::from_toml_str("[pipeline]\n").unwrap();
    assert_eq!(config.pipeline.ir_emitter.mode, IrEmitterMode::Off);
    assert!(config.pipeline.ir_emitter.descriptor.is_none());
    assert!(!config.warnings.iter().any(|w| w.contains("ir_emitter")));
    assert_eq!(
        DaemonConfig::runtime_default().pipeline.ir_emitter.mode,
        IrEmitterMode::Off
    );
}

/// IRA16: a valid section is loaded, with a warning that the daemon does not drive it yet.
#[test]
fn test_ira_daemon_loads_valid_section_with_warning() {
    let config = DaemonConfig::from_toml_str(SECTION).unwrap();
    assert_eq!(config.pipeline.ir_emitter.mode, IrEmitterMode::Auto);
    let d = config.pipeline.ir_emitter.descriptor.as_ref().unwrap();
    assert_eq!(
        d.device,
        UsbDeviceId {
            vendor: 0x04f2,
            product: 0xb6d9
        }
    );
    let warning = config
        .warnings
        .iter()
        .find(|w| w.contains("[camera.ir_emitter]"))
        .expect("a start-up warning names the section");
    assert!(!warning.contains("04f2"), "the warning never echoes values");

    let off = DaemonConfig::from_toml_str(&SECTION.replace("\"auto\"", "\"off\"")).unwrap();
    assert_eq!(off.pipeline.ir_emitter.mode, IrEmitterMode::Off);
    assert!(!off
        .warnings
        .iter()
        .any(|w| w.contains("[camera.ir_emitter]")));
}

/// IRA16: every invalid value refuses start-up, naming the section, never the value.
#[test]
fn test_ira_daemon_refuses_invalid_section() {
    let cases = [
        SECTION.replace("mode = \"auto\"", "mode = \"sometimes\""),
        format!("{SECTION}power = 1\n"),
        SECTION.replace("unit = 14", "unit = 0"),
        SECTION.replace("selector = 6", "selector = 300"),
        SECTION.replace("device = \"04f2:b6d9\"", "device = \"zz:top\""),
        SECTION.replace("off = [1, 3, 0, 0, 0, 0, 0, 0, 0]", "off = [1]"),
        SECTION.replace(
            "on = [1, 3, 3, 0, 0, 0, 0, 0, 0]",
            &format!("on = [{}]", vec!["1"; 65].join(", ")),
        ),
        SECTION.replace("unit = 14\n", ""),
        "[camera]\nir_emitter = \"on\"\n".to_string(),
    ];
    for case in cases {
        let msg = rejected(&case);
        assert!(msg.contains("[camera"), "{msg}");
        assert!(
            !msg.contains("sometimes") && !msg.contains("zz:top"),
            "{msg}"
        );
    }
}

/// IRA16b (candid review of #378): an unknown `[camera]` key does not stop the daemon: it is
/// ignored like unknown keys of every other table and reported by name, never by value.
#[test]
fn test_ira_daemon_loads_with_unknown_camera_key() {
    let config = DaemonConfig::from_toml_str(
        "[camera]\nidle_timeout_secs = 4242\n\n[pipeline]\nidle_timeout_secs = 7\n",
    )
    .expect("an unknown [camera] key must not refuse start-up");
    assert_eq!(config.pipeline.ir_emitter.mode, IrEmitterMode::Off);
    assert_eq!(config.pipeline.camera.idle_timeout.as_secs(), 7);
    let warning = config
        .warnings
        .iter()
        .find(|w| w.contains("[camera] idle_timeout_secs"))
        .expect("the ignored key is reported");
    assert!(!warning.contains("4242"), "{warning}");

    let with_section = DaemonConfig::from_toml_str(&format!("[camera]\nexposure = 3\n{SECTION}"))
        .expect("the section still loads next to an unknown key");
    assert_eq!(with_section.pipeline.ir_emitter.mode, IrEmitterMode::Auto);
}
