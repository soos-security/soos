//! Static contracts of the UVC extension-unit IR emitter control (GitHub #378, PR 1 of #353;
//! spec `AI/architect_spec_ir_emitter_control.md`; matrix IRA22–IRA24):
//!
//! - the only new `unsafe` block is the `UVCIOC_CTRL_QUERY` ioctl in
//!   `crates/camera-v4l/src/uvc_xu.rs`, preceded by a `// SAFETY:` comment; the other new
//!   modules and the admin command contain no `unsafe` at all;
//! - the admin probe goes through the read-only scan, never the `v4l` crate directly;
//! - no `tracing` call of the new modules interpolates a payload or a control value;
//! - the mocks are compiled only with `mock-camera`, and `soos-frame` gains no IR type.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::PathBuf;

const UVC_XU: &str = "crates/camera-v4l/src/uvc_xu.rs";
const NEW_SAFE_MODULES: [&str; 5] = [
    "crates/camera-v4l/src/ir_emitter.rs",
    "crates/camera-v4l/src/ir_emitter_config.rs",
    "crates/camera-v4l/src/strobe.rs",
    "crates/camera-v4l/src/ir_mock.rs",
    "crates/admin-cli/src/ir_emitter.rs",
];
const ADR_TITLE: &str = "UVC Extension-Unit IR Emitter Control and Strobe Classification";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Lines that are not `//` comments.
fn code_lines(text: &str) -> Vec<(usize, &str)> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("//"))
        .collect()
}

fn has_unsafe_token(line: &str) -> bool {
    [
        "unsafe {",
        "unsafe{",
        "unsafe fn",
        "unsafe impl",
        "unsafe trait",
        "unsafe extern",
    ]
    .iter()
    .any(|t| line.contains(t))
}

/// IRA22: exactly one `unsafe` block in `uvc_xu.rs`, documented; none in the other modules.
#[test]
fn test_ira_unsafe_is_confined_to_the_xu_ioctl() {
    let text = read(UVC_XU);
    let lines: Vec<&str> = text.lines().collect();
    let sites: Vec<usize> = code_lines(&text)
        .into_iter()
        .filter(|(_, l)| has_unsafe_token(l))
        .map(|(n, _)| n)
        .collect();
    assert_eq!(
        sites.len(),
        1,
        "{UVC_XU}: one unsafe site expected, found {sites:?}"
    );
    let site = sites[0];
    let window = &lines[site.saturating_sub(12)..site];
    assert!(
        window
            .iter()
            .any(|l| l.trim_start().starts_with("// SAFETY:")),
        "{UVC_XU}: the unsafe block must be preceded by a // SAFETY: comment"
    );
    assert!(
        lines[site..].iter().take(8).any(|l| l.contains("ioctl")),
        "{UVC_XU}: the unsafe block is the ioctl call"
    );
    for rel in NEW_SAFE_MODULES {
        let text = read(rel);
        for (n, line) in code_lines(&text) {
            assert!(
                !has_unsafe_token(line),
                "{rel}:{}: no unsafe outside {UVC_XU}: {line}",
                n + 1
            );
        }
    }
    let lib = read("crates/camera-v4l/src/lib.rs");
    assert!(lib.contains("#![deny(clippy::undocumented_unsafe_blocks)]"));
    let admin = read("crates/admin-cli/src/lib.rs");
    assert!(admin.contains("#![forbid(unsafe_code)]"));
}

/// IRA23: the admin probe is the bounded read-only scan; the admin module never names the
/// `v4l` crate, streams frames or issues a raw SET_CUR.
#[test]
fn test_ira_admin_probe_is_read_only() {
    let admin = read("crates/admin-cli/src/ir_emitter.rs");
    let production = admin
        .find("#[cfg(test)]")
        .map_or(admin.as_str(), |p| &admin[..p]);
    assert!(production.contains("probe_xu_controls("));
    assert!(production.contains("XuReadOnly::new("));
    for forbidden in [
        "v4l::",
        "latest_frame",
        "MockCameraManager",
        "XuQuery::SetCur",
        "toml::",
        "thread::spawn",
        "std::thread",
    ] {
        let hit = production.match_indices(forbidden).any(|(pos, _)| {
            forbidden != "v4l::"
                || production[..pos]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
        });
        assert!(
            !hit,
            "crates/admin-cli/src/ir_emitter.rs must not use `{forbidden}`"
        );
    }
    let main = read("crates/admin-cli/src/main.rs");
    assert!(
        !main.contains("thread::spawn") && !main.contains("std::thread"),
        "soos-admin stays single-threaded (the signal mask of the device test is per thread)"
    );
    let xu = read(UVC_XU);
    assert!(
        xu.contains("WriteRefused"),
        "the read-only wrapper refuses writes"
    );
}

/// IRA23: no tracing call in the new modules interpolates a payload or a control value.
#[test]
fn test_ira_no_payload_in_logs() {
    let mut files = vec![UVC_XU];
    files.extend(NEW_SAFE_MODULES);
    let forbidden = [
        "payload",
        "original",
        "descriptor.on",
        "descriptor.off",
        "value",
        "data",
        "frame",
    ];
    let mut scanned = 0;
    for rel in files {
        let text = read(rel);
        let mut in_call = false;
        for (n, line) in code_lines(&text) {
            let t = line.trim_start();
            if [
                "tracing::",
                "trace!(",
                "debug!(",
                "info!(",
                "warn!(",
                "error!(",
            ]
            .iter()
            .any(|m| t.contains(m))
            {
                in_call = true;
                scanned += 1;
            }
            if in_call {
                for word in forbidden {
                    assert!(
                        !line.contains(&format!("{{{word}"))
                            && !line.contains(&format!("= %{word}"))
                            && !line.contains(&format!("= ?{word}"))
                            && !line.contains(&format!(", {word})"))
                            && !line.contains(&format!(", {word},")),
                        "{rel}:{}: a log call must not carry `{word}`: {line}",
                        n + 1
                    );
                }
                if t.contains(");") {
                    in_call = false;
                }
            }
        }
    }
    // The scan may legitimately find zero calls; it must at least have read the files.
    let _ = scanned;
}

/// IRA24: the mocks are compiled only with `mock-camera`; `soos-frame` gains no IR type.
#[test]
fn test_ira_mocks_are_feature_gated_and_frame_is_untouched() {
    let lib = read("crates/camera-v4l/src/lib.rs");
    let pos = lib.find("pub mod ir_mock;").expect("ir_mock module");
    let before = lib[..pos].lines().last().unwrap_or("");
    assert!(
        before.contains("#[cfg(feature = \"mock-camera\")]"),
        "ir_mock must be declared behind #[cfg(feature = \"mock-camera\")]"
    );
    for module in [
        "pub mod uvc_xu;",
        "pub mod ir_emitter;",
        "pub mod ir_emitter_config;",
        "pub mod strobe;",
    ] {
        assert!(lib.contains(module), "lib.rs must declare `{module}`");
    }
    for rel in [
        "crates/frame/src/lib.rs",
        "crates/frame/src/frame.rs",
        "crates/frame/src/sensor.rs",
    ] {
        let text = read(rel);
        for word in ["Emitter", "Strobe", "Luma"] {
            assert!(!text.contains(word), "{rel} must not gain IR type `{word}`");
        }
    }
    let manifest = read("crates/frame/Cargo.toml");
    assert!(
        !manifest.contains("default = [\"ir\"") && !manifest.contains("\"ir\"]"),
        "an `ir` feature of soos-frame is never a default"
    );
}

/// IRA24: the decision is recorded.
#[test]
fn test_ira_adr_is_recorded() {
    let decisions = read("AI/DECISIONS.md");
    assert!(
        decisions.contains(ADR_TITLE),
        "AI/DECISIONS.md must record `{ADR_TITLE}`"
    );
    let docs = read("Docs/CAMERA_V4L_CRATE.md");
    for needle in [
        "UVCIOC_CTRL_QUERY",
        "[camera.ir_emitter]",
        "ir-emitter probe",
        "--test-on-device",
    ] {
        assert!(
            docs.contains(needle),
            "Docs/CAMERA_V4L_CRATE.md must mention `{needle}`"
        );
    }
}
