//! `[camera.ir_emitter]` parsing, validation and rendering (GitHub #378, PR 1 of #353;
//! matrix IRA1–IRA4). Every invalid value is refused (fail closed) with an error that never
//! echoes the offending value.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use proptest::prelude::*;
use soos_camera_v4l::ir_emitter_config::{
    ignored_camera_keys, ir_emitter_section_present, parse_camera_table, parse_ir_emitter_text,
    read_daemon_ir_emitter_config, usb_device_id_of_node, IrEmitterConfigError, IrEmitterMode,
    IrEmitterReadError, IrEmitterSettings, UsbDeviceId, XuEmitterDescriptor,
    MAX_IGNORED_CAMERA_KEYS,
};
use soos_camera_v4l::uvc_xu::XU_MAX_PAYLOAD_BYTES;
use std::path::Path;

const VALID: &str = r#"
[ir_emitter]
mode = "auto"
device = "04F2:b6d9"
unit = 14
selector = 6
on = [1, 3, 3, 0, 0, 0, 0, 0, 0]
off = [1, 3, 0, 0, 0, 0, 0, 0, 0]
"#;

fn camera(text: &str) -> toml::Table {
    text.parse::<toml::Table>().expect("test TOML parses")
}

fn parse(text: &str) -> Result<IrEmitterSettings, IrEmitterConfigError> {
    parse_camera_table(&camera(text))
}

fn with_line(key: &str, line: &str) -> String {
    VALID
        .lines()
        .map(|l| {
            if l.starts_with(&format!("{key} =")) {
                line.to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn without(key: &str) -> String {
    VALID
        .lines()
        .filter(|l| !l.starts_with(&format!("{key} =")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn id(vendor: u16, product: u16) -> UsbDeviceId {
    UsbDeviceId { vendor, product }
}

/// IRA1: a complete auto section parses into the exact descriptor (VID:PID normalised).
#[test]
fn test_ira_valid_section_parses_to_descriptor() {
    let settings = parse(VALID).unwrap();
    assert_eq!(settings.mode, IrEmitterMode::Auto);
    let d = settings.descriptor.expect("descriptor");
    assert_eq!(d.device, id(0x04f2, 0xb6d9));
    assert_eq!(d.device.to_string(), "04f2:b6d9");
    assert_eq!((d.unit, d.selector), (14, 6));
    assert_eq!(d.on, vec![1, 3, 3, 0, 0, 0, 0, 0, 0]);
    assert_eq!(d.off, vec![1, 3, 0, 0, 0, 0, 0, 0, 0]);
}

/// IRA1: no section, an empty `[camera]` and `mode = "off"` alone are all "off" (default).
#[test]
fn test_ira_absent_section_defaults_to_off() {
    assert_eq!(parse("").unwrap(), IrEmitterSettings::default());
    assert_eq!(IrEmitterSettings::default().mode, IrEmitterMode::Off);
    assert!(IrEmitterSettings::default().descriptor.is_none());
    let off = parse("[ir_emitter]\nmode = \"off\"\n").unwrap();
    assert_eq!(off.mode, IrEmitterMode::Off);
    assert!(off.descriptor.is_none());
    let implicit = parse("[ir_emitter]\n").unwrap();
    assert_eq!(implicit.mode, IrEmitterMode::Off);
}

/// IRA2: unknown keys are refused inside `[camera.ir_emitter]`.
#[test]
fn test_ira_unknown_keys_are_refused() {
    let err = parse(&format!("{VALID}\npower = 3\n")).unwrap_err();
    assert!(
        matches!(err, IrEmitterConfigError::UnknownKey { table: "camera.ir_emitter", ref key } if key == "power"),
        "{err:?}"
    );
}

/// IRA2b (candid review of #378): other keys of `[camera]` are ignored like unknown keys of
/// every other daemon table, and reported by name only (sanitized, bounded), never by value.
#[test]
fn test_ira_other_camera_keys_are_ignored_and_reported() {
    let settings = parse("idle_timeout_secs = 10\nstrobe = true\n").unwrap();
    assert_eq!(settings, IrEmitterSettings::default());
    let with_section = parse(&format!("exposure = 3\n{VALID}")).unwrap();
    assert_eq!(with_section.mode, IrEmitterMode::Auto);

    let table = camera("idle_timeout_secs = 10\nstrobe = \"secret-value\"\n");
    let ignored = ignored_camera_keys(&table);
    assert_eq!(
        ignored,
        vec!["idle_timeout_secs".to_string(), "strobe".to_string()]
    );
    assert!(ignored.iter().all(|k| !k.contains("secret")));
    assert!(ignored_camera_keys(&camera(VALID)).is_empty());

    let long = "k".repeat(500);
    let table = camera(&format!("\"\\u001b[31m{long}\" = 1\n"));
    let ignored = ignored_camera_keys(&table);
    assert_eq!(ignored.len(), 1);
    assert!(ignored[0].chars().count() <= 64);
    assert!(!ignored[0].contains('\u{1b}'));

    let many: String = (0..40).map(|i| format!("k{i:02} = {i}\n")).collect();
    assert_eq!(
        ignored_camera_keys(&camera(&many)).len(),
        MAX_IGNORED_CAMERA_KEYS
    );
}

/// IRA2: an unknown key name is sanitized and bounded in the error.
#[test]
fn test_ira_unknown_key_name_is_sanitized_and_bounded() {
    let long = "k".repeat(500);
    let err = parse(&format!("{VALID}\n\"\\u001b[31m{long}\" = 1\n")).unwrap_err();
    let IrEmitterConfigError::UnknownKey { key, .. } = &err else {
        panic!("{err:?}");
    };
    assert!(key.chars().count() <= 64, "{} chars", key.chars().count());
    assert!(!key.contains('\u{1b}'));
    assert!(!err.to_string().contains('\u{1b}'));
}

/// IRA2: a mode other than "off"/"auto" is refused without echoing it.
#[test]
fn test_ira_invalid_mode_is_refused_without_echo() {
    for bad in ["\"strict\"", "\"AUTO\"", "\"\"", "1", "true"] {
        let err = parse(&with_line("mode", &format!("mode = {bad}"))).unwrap_err();
        assert!(
            matches!(
                err,
                IrEmitterConfigError::InvalidMode
                    | IrEmitterConfigError::WrongType { key: "mode", .. }
            ),
            "{bad}: {err:?}"
        );
        assert!(!err.to_string().contains("strict"));
    }
}

/// IRA2: `mode = "auto"` needs every descriptor key.
#[test]
fn test_ira_auto_requires_every_key() {
    for key in ["device", "unit", "selector", "on", "off"] {
        let err = parse(&without(key)).unwrap_err();
        assert_eq!(err, IrEmitterConfigError::MissingKey { key }, "{key}");
    }
}

/// IRA2: unit and selector must be 1..=255.
#[test]
fn test_ira_unit_and_selector_out_of_range_are_refused() {
    for key in ["unit", "selector"] {
        for bad in ["0", "-1", "256", "65536"] {
            let err = parse(&with_line(key, &format!("{key} = {bad}"))).unwrap_err();
            assert_eq!(
                err,
                IrEmitterConfigError::OutOfRange { key },
                "{key} = {bad}"
            );
        }
        let err = parse(&with_line(key, &format!("{key} = \"14\""))).unwrap_err();
        assert!(
            matches!(err, IrEmitterConfigError::WrongType { .. }),
            "{err:?}"
        );
        assert!(parse(&with_line(key, &format!("{key} = 255"))).is_ok());
        assert!(parse(&with_line(key, &format!("{key} = 1"))).is_ok());
    }
}

/// IRA2: payloads above 64 bytes are refused; exactly 64 is accepted.
#[test]
fn test_ira_payload_over_64_bytes_is_refused() {
    assert_eq!(XU_MAX_PAYLOAD_BYTES, 64);
    let list = |n: usize, v: u8| {
        format!(
            "[{}]",
            (0..n).map(|_| v.to_string()).collect::<Vec<_>>().join(", ")
        )
    };
    let ok = with_line("on", &format!("on = {}", list(64, 1)));
    let ok = ok.replace(
        "off = [1, 3, 0, 0, 0, 0, 0, 0, 0]",
        &format!("off = {}", list(64, 0)),
    );
    assert!(parse(&ok).is_ok());
    let bad = with_line("on", &format!("on = {}", list(65, 1)));
    let bad = bad.replace(
        "off = [1, 3, 0, 0, 0, 0, 0, 0, 0]",
        &format!("off = {}", list(65, 0)),
    );
    let err = parse(&bad).unwrap_err();
    assert_eq!(
        err,
        IrEmitterConfigError::PayloadTooLong { key: "on", max: 64 }
    );
}

/// IRA2: empty payloads, bytes outside 0..=255, wrong element types, mismatched lengths and
/// identical payloads are refused.
#[test]
fn test_ira_malformed_payloads_are_refused() {
    let err = parse(&with_line("on", "on = []")).unwrap_err();
    assert_eq!(err, IrEmitterConfigError::EmptyPayload { key: "on" });
    let err = parse(&with_line("off", "off = [1, 3, 0, 0, 0, 0, 0, 0, 256]")).unwrap_err();
    assert_eq!(
        err,
        IrEmitterConfigError::PayloadByteOutOfRange { key: "off" }
    );
    let err = parse(&with_line("off", "off = [1, 3, 0, 0, 0, 0, 0, 0, -1]")).unwrap_err();
    assert_eq!(
        err,
        IrEmitterConfigError::PayloadByteOutOfRange { key: "off" }
    );
    let err = parse(&with_line("on", "on = [\"1\"]")).unwrap_err();
    assert!(
        matches!(
            err,
            IrEmitterConfigError::PayloadByteOutOfRange { key: "on" }
                | IrEmitterConfigError::WrongType { key: "on", .. }
        ),
        "{err:?}"
    );
    let err = parse(&with_line("on", "on = \"010303\"")).unwrap_err();
    assert!(
        matches!(err, IrEmitterConfigError::WrongType { key: "on", .. }),
        "{err:?}"
    );
    let err = parse(&with_line("off", "off = [1, 3]")).unwrap_err();
    assert_eq!(err, IrEmitterConfigError::PayloadLengthMismatch);
    let err = parse(&with_line("off", "off = [1, 3, 3, 0, 0, 0, 0, 0, 0]")).unwrap_err();
    assert_eq!(err, IrEmitterConfigError::IdenticalPayloads);
}

/// IRA2: a malformed VID:PID is refused.
#[test]
fn test_ira_invalid_device_id_is_refused() {
    for bad in [
        "\"04f2b6d9\"",
        "\"04f2:b6d\"",
        "\"04f2:b6d90\"",
        "\"g4f2:b6d9\"",
        "\"04f2-b6d9\"",
        "\" 04f2:b6d9\"",
        "\"\"",
    ] {
        let err = parse(&with_line("device", &format!("device = {bad}"))).unwrap_err();
        assert_eq!(err, IrEmitterConfigError::InvalidDeviceId, "{bad}");
    }
    assert_eq!(UsbDeviceId::parse("ABCD:ef01"), Some(id(0xabcd, 0xef01)));
    assert_eq!(UsbDeviceId::parse("abcd:ef0"), None);
    assert_eq!(UsbDeviceId::parse("+bcd:ef01"), None);
}

/// IRA2: with mode = "off", present keys are still validated (fail closed on garbage).
#[test]
fn test_ira_off_mode_still_validates_present_keys() {
    let off = with_line("mode", "mode = \"off\"");
    let settings = parse(&off).unwrap();
    assert_eq!(settings.mode, IrEmitterMode::Off);
    assert!(settings.descriptor.is_some());
    let err = parse(&with_line("unit", "unit = 0").replace("mode = \"auto\"", "mode = \"off\""))
        .unwrap_err();
    assert_eq!(err, IrEmitterConfigError::OutOfRange { key: "unit" });
    let partial = parse("[ir_emitter]\nmode = \"off\"\nunit = 3\n").unwrap();
    assert!(partial.descriptor.is_none());
}

/// IRA2: `ir_emitter` that is not a table is refused.
#[test]
fn test_ira_ir_emitter_must_be_a_table() {
    let err = parse("ir_emitter = 1\n").unwrap_err();
    assert!(
        matches!(
            err,
            IrEmitterConfigError::WrongType {
                key: "ir_emitter",
                ..
            }
        ),
        "{err:?}"
    );
}

/// IRA3: the descriptor constructor applies the same rules as the parser.
#[test]
fn test_ira_descriptor_constructor_validates() {
    let d = id(1, 2);
    assert!(XuEmitterDescriptor::new(d, 1, 1, vec![1], vec![0]).is_ok());
    assert_eq!(
        XuEmitterDescriptor::new(d, 0, 1, vec![1], vec![0]).unwrap_err(),
        IrEmitterConfigError::OutOfRange { key: "unit" }
    );
    assert_eq!(
        XuEmitterDescriptor::new(d, 1, 0, vec![1], vec![0]).unwrap_err(),
        IrEmitterConfigError::OutOfRange { key: "selector" }
    );
    assert_eq!(
        XuEmitterDescriptor::new(d, 1, 1, vec![], vec![]).unwrap_err(),
        IrEmitterConfigError::EmptyPayload { key: "on" }
    );
    assert_eq!(
        XuEmitterDescriptor::new(d, 1, 1, vec![1; 65], vec![0; 65]).unwrap_err(),
        IrEmitterConfigError::PayloadTooLong { key: "on", max: 64 }
    );
    assert_eq!(
        XuEmitterDescriptor::new(d, 1, 1, vec![1, 2], vec![0]).unwrap_err(),
        IrEmitterConfigError::PayloadLengthMismatch
    );
    assert_eq!(
        XuEmitterDescriptor::new(d, 1, 1, vec![5], vec![5]).unwrap_err(),
        IrEmitterConfigError::IdenticalPayloads
    );
}

/// IRA3: the VID:PID check fails closed on an unknown or different device.
#[test]
fn test_ira_device_identity_check_fails_closed() {
    let d = XuEmitterDescriptor::new(id(0x04f2, 0xb6d9), 14, 6, vec![1], vec![0]).unwrap();
    assert!(d.check_device(Some(id(0x04f2, 0xb6d9))).is_ok());
    assert_eq!(
        d.check_device(None).unwrap_err(),
        IrEmitterConfigError::DeviceIdUnavailable
    );
    assert_eq!(
        d.check_device(Some(id(0x04f2, 0xb6da))).unwrap_err(),
        IrEmitterConfigError::DeviceMismatch {
            configured: id(0x04f2, 0xb6d9),
            observed: id(0x04f2, 0xb6da),
        }
    );
}

/// IRA4: the rendered section round-trips through the parser (both modes).
#[test]
fn test_ira_rendered_section_round_trips() {
    let settings = parse(VALID).unwrap();
    let text = settings.to_toml_section();
    assert!(text.contains("[camera.ir_emitter]"), "{text}");
    let doc: toml::Table = text.parse().unwrap();
    let camera = doc.get("camera").and_then(toml::Value::as_table).unwrap();
    assert_eq!(parse_camera_table(camera).unwrap(), settings);

    let off = IrEmitterSettings {
        mode: IrEmitterMode::Off,
        descriptor: settings.descriptor.clone(),
    };
    let doc: toml::Table = off.to_toml_section().parse().unwrap();
    let camera = doc.get("camera").and_then(toml::Value::as_table).unwrap();
    assert_eq!(parse_camera_table(camera).unwrap(), off);
}

/// IRA4: the client reader goes through the shared bounded reader and parses `[camera]` only.
#[test]
fn test_ira_client_reader_reads_camera_section() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("daemon.toml");
    let body = format!(
        "[pipeline]\nsensor_preference = \"ir\"\n\n{}",
        VALID.replace("[ir_emitter]", "[camera.ir_emitter]")
    );
    std::fs::write(&path, body).unwrap();
    let settings = read_daemon_ir_emitter_config(&path).unwrap();
    assert_eq!(settings.mode, IrEmitterMode::Auto);

    std::fs::write(&path, "[pipeline]\n").unwrap();
    assert_eq!(
        read_daemon_ir_emitter_config(&path).unwrap(),
        IrEmitterSettings::default()
    );

    std::fs::write(&path, "[camera.ir_emitter]\nmode = \"strict\"\n").unwrap();
    assert_eq!(
        read_daemon_ir_emitter_config(&path).unwrap_err(),
        IrEmitterReadError::Invalid(IrEmitterConfigError::InvalidMode)
    );
    std::fs::write(&path, "not toml [").unwrap();
    assert_eq!(
        read_daemon_ir_emitter_config(&path).unwrap_err(),
        IrEmitterReadError::Malformed
    );
    assert!(matches!(
        read_daemon_ir_emitter_config(&dir.path().join("missing.toml")).unwrap_err(),
        IrEmitterReadError::Read(_)
    ));
}

/// IRA4: text entry points for clients that must not parse TOML themselves.
#[test]
fn test_ira_text_entry_points() {
    let full = VALID.replace("[ir_emitter]", "[camera.ir_emitter]");
    let text = format!("[pipeline]\nwarmup_frames = 0\n{full}");
    assert_eq!(
        parse_ir_emitter_text(&text).unwrap().mode,
        IrEmitterMode::Auto
    );
    assert!(ir_emitter_section_present(&text).unwrap());
    assert!(!ir_emitter_section_present("[pipeline]\n").unwrap());
    assert!(!ir_emitter_section_present("[camera]\n").unwrap());
    assert!(ir_emitter_section_present("camera.ir_emitter.mode = \"off\"\n").unwrap());
    assert_eq!(
        parse_ir_emitter_text("[pipeline\n").unwrap_err(),
        IrEmitterReadError::Malformed
    );
    assert_eq!(
        ir_emitter_section_present("[pipeline\n").unwrap_err(),
        IrEmitterReadError::Malformed
    );
    assert_eq!(
        parse_ir_emitter_text("camera = 3\n").unwrap_err(),
        IrEmitterReadError::Malformed
    );
    assert_eq!(
        parse_ir_emitter_text("").unwrap(),
        IrEmitterSettings::default()
    );
}

fn sysfs_with_usb(root: &Path, node: &str, vendor: &str, product: &str) {
    let usb = root.join("usb/1-6");
    let iface = usb.join("1-6:1.0");
    std::fs::create_dir_all(&iface).unwrap();
    std::fs::write(usb.join("idVendor"), vendor).unwrap();
    std::fs::write(usb.join("idProduct"), product).unwrap();
    let class = root.join("class");
    std::fs::create_dir_all(class.join(node)).unwrap();
    std::os::unix::fs::symlink(&iface, class.join(node).join("device")).unwrap();
}

/// IRA3: the USB id of a node is read from sysfs, bounded, and fails closed on bad content.
#[test]
fn test_ira_usb_id_is_read_from_sysfs_bounded() {
    let dir = tempfile::tempdir().unwrap();
    sysfs_with_usb(dir.path(), "video2", "04f2\n", "B6D9\n");
    let class = dir.path().join("class");
    assert_eq!(
        usb_device_id_of_node(&class, "video2"),
        Some(id(0x04f2, 0xb6d9))
    );
    assert_eq!(usb_device_id_of_node(&class, "video3"), None);
    assert_eq!(usb_device_id_of_node(&class, "../usb"), None);
    assert_eq!(usb_device_id_of_node(&class, "video2/.."), None);

    let dir = tempfile::tempdir().unwrap();
    sysfs_with_usb(dir.path(), "video0", "04f2", "zzzz");
    assert_eq!(
        usb_device_id_of_node(&dir.path().join("class"), "video0"),
        None
    );

    let dir = tempfile::tempdir().unwrap();
    sysfs_with_usb(dir.path(), "video0", &"0".repeat(4096), "b6d9");
    assert_eq!(
        usb_device_id_of_node(&dir.path().join("class"), "video0"),
        None
    );
}

proptest! {
    /// IRA2: arbitrary TOML-ish input never panics, and anything accepted with mode auto
    /// carries a fully valid descriptor.
    #[test]
    fn prop_ira_parser_never_panics(
        unit in any::<i64>(),
        selector in any::<i64>(),
        on in proptest::collection::vec(any::<i64>(), 0..80),
        off in proptest::collection::vec(any::<i64>(), 0..80),
        device in "[0-9a-fA-F:x ]{0,12}",
    ) {
        let mut table = toml::Table::new();
        let mut ir = toml::Table::new();
        ir.insert("mode".into(), toml::Value::String("auto".into()));
        ir.insert("device".into(), toml::Value::String(device));
        ir.insert("unit".into(), toml::Value::Integer(unit));
        ir.insert("selector".into(), toml::Value::Integer(selector));
        ir.insert("on".into(), toml::Value::Array(on.into_iter().map(toml::Value::Integer).collect()));
        ir.insert("off".into(), toml::Value::Array(off.into_iter().map(toml::Value::Integer).collect()));
        table.insert("ir_emitter".into(), toml::Value::Table(ir));
        if let Ok(settings) = parse_camera_table(&table) {
            let d = settings.descriptor.expect("auto implies a descriptor");
            prop_assert!(d.unit >= 1 && d.selector >= 1);
            prop_assert!(!d.on.is_empty() && d.on.len() <= XU_MAX_PAYLOAD_BYTES);
            prop_assert_eq!(d.on.len(), d.off.len());
            prop_assert_ne!(d.on, d.off);
        }
    }
}
