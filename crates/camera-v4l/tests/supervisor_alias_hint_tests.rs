//! By-id alias lookup of a plain `/dev/videoN` capture path (GitHub #289, matrix rows
//! CAG1-CAG2).
//!
//! When `camera_device` names a kernel node (`/dev/video2`) instead of a by-id link, the capture
//! supervisor looks up the node's persistent `/dev/v4l/by-id/` alias (through the single bounded
//! scanner `SystemCameraEnumerator::by_id_aliases`) and feeds its name to the classifier. The
//! lookup only fills a missing by-id name, so it can only move a node to `Infrared` (stricter IR
//! PAD policy), never the reverse. The tests use a fake by-id directory: no real device is read.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite uses assertions, unwrap and indexing"
)]

use soos_camera_v4l::{
    classify_sensor_with_hints, plan_capture_with_hints, supervisor_alias_hints,
    supervisor_sensor_hints, CameraConfig, PixelFormat, SensorHints, SensorType,
};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

/// A scratch `/dev` with fake capture nodes and a `v4l/by-id` alias directory.
struct FakeDev {
    _root: tempfile::TempDir,
    dev: PathBuf,
    by_id: PathBuf,
}

impl FakeDev {
    fn new(nodes: &[&str]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let dev = root.path().join("dev");
        let by_id = dev.join("v4l").join("by-id");
        std::fs::create_dir_all(&by_id).unwrap();
        for node in nodes {
            std::fs::write(dev.join(node), b"").unwrap();
        }
        Self {
            _root: root,
            dev,
            by_id,
        }
    }

    fn node(&self, name: &str) -> PathBuf {
        self.dev.join(name)
    }

    /// Creates a relative udev-style link `by-id/<alias> -> ../../<node>`.
    fn alias(&self, alias: &str, node: &str) -> PathBuf {
        let link = self.by_id.join(alias);
        symlink(Path::new("../..").join(node), &link).unwrap();
        link
    }
}

fn hints(device_path: &Path, sizes: &[(u32, u32)]) -> SensorHints {
    supervisor_sensor_hints(device_path, sizes.to_vec())
}

#[test]
fn test_cag_plain_node_path_takes_its_by_id_alias_name() {
    let fake = FakeDev::new(&["video0", "video2"]);
    fake.alias("usb-Acme_HD_Camera_0001-video-index0", "video0");
    fake.alias("usb-Acme_Face_IR_Camera_0001-video-index0", "video2");

    // An IR node streaming a colour format under a neutral card name: without the alias the
    // supervisor stamps it Rgb; with the alias name the by-id IR token makes it Infrared.
    let node = fake.node("video2");
    let before = hints(&node, &[(640, 480)]);
    assert_eq!(before.by_id_name, None, "a plain node path carries no name");
    let card = "Integrated Camera: Integrated C";
    let formats = [PixelFormat::Yuyv, PixelFormat::Mjpeg];
    assert_eq!(
        classify_sensor_with_hints(card, &formats, &before),
        SensorType::Rgb
    );

    let after = supervisor_alias_hints(before, &node, &fake.by_id);
    assert_eq!(
        after.by_id_name.as_deref(),
        Some("usb-Acme_Face_IR_Camera_0001-video-index0")
    );
    assert_eq!(after.frame_sizes, vec![(640, 480)], "frame sizes are kept");
    let plan = plan_capture_with_hints(card, &formats, &after, &CameraConfig::default()).unwrap();
    assert_eq!(plan.sensor_type, SensorType::Infrared);

    // The colour node's alias carries no IR token: still Rgb (no spurious upgrade).
    let colour = fake.node("video0");
    let colour_hints = supervisor_alias_hints(hints(&colour, &[]), &colour, &fake.by_id);
    assert_eq!(
        colour_hints.by_id_name.as_deref(),
        Some("usb-Acme_HD_Camera_0001-video-index0")
    );
    assert_eq!(
        classify_sensor_with_hints(card, &formats, &colour_hints),
        SensorType::Rgb
    );
}

#[test]
fn test_cag_alias_lookup_edge_cases_keep_or_fill_only() {
    let fake = FakeDev::new(&["video0", "video1", "video4"]);
    fake.alias("usb-Vendor_Plain_Cam-video-index0", "video0");
    fake.alias("usb-Vendor_IR_Cam-video-index0", "video0");
    fake.alias("usb-Dangling_IR_Cam-video-index0", "video9");
    let named = fake.alias("usb-Vendor_Named_Cam-video-index0", "video1");

    // Several aliases resolve to one node: the one carrying an IR token wins (stricter).
    let node0 = fake.node("video0");
    let multi = supervisor_alias_hints(hints(&node0, &[]), &node0, &fake.by_id);
    assert_eq!(
        multi.by_id_name.as_deref(),
        Some("usb-Vendor_IR_Cam-video-index0")
    );

    // A path that is already a by-id link keeps its own name (no second lookup, no swap).
    let own = hints(&named, &[(1280, 720)]);
    assert_eq!(
        own.by_id_name.as_deref(),
        Some("usb-Vendor_Named_Cam-video-index0")
    );
    let kept = supervisor_alias_hints(own.clone(), &named, &fake.by_id);
    assert_eq!(kept, own);

    // A name already present is never replaced, even by an IR-token alias of the same node.
    let preset = SensorHints {
        by_id_name: Some("usb-Preset_Cam-video-index0".to_string()),
        frame_sizes: Vec::new(),
    };
    assert_eq!(
        supervisor_alias_hints(preset.clone(), &node0, &fake.by_id),
        preset
    );

    // No alias for the node, a dangling alias only, a missing node, a missing directory and an
    // empty path: hints unchanged.
    let node4 = fake.node("video4");
    let none = hints(&node4, &[(400, 400)]);
    assert_eq!(
        supervisor_alias_hints(none.clone(), &node4, &fake.by_id),
        none
    );
    let missing = fake.node("video9");
    let missing_hints = hints(&missing, &[]);
    assert_eq!(
        supervisor_alias_hints(missing_hints.clone(), &missing, &fake.by_id),
        missing_hints
    );
    let no_dir = fake.dev.join("no-such-dir");
    assert_eq!(
        supervisor_alias_hints(none.clone(), &node0, &no_dir),
        none,
        "an unreadable by-id directory yields no alias"
    );
    let empty = hints(Path::new(""), &[]);
    assert_eq!(
        supervisor_alias_hints(empty.clone(), Path::new(""), &fake.by_id),
        empty
    );
}

#[test]
fn test_cag_alias_lookup_is_bounded_on_a_crowded_directory() {
    let fake = FakeDev::new(&["video0"]);
    for i in 0..300 {
        std::fs::write(fake.by_id.join(format!("noise-{i:03}")), b"").unwrap();
    }
    let node = fake.node("video0");
    // The single scanner reads at most MAX_BY_ID_ENTRIES entries: the call returns and never
    // invents a name for a node that has no alias.
    let out = supervisor_alias_hints(hints(&node, &[]), &node, &fake.by_id);
    assert_eq!(out.by_id_name, None);
}

#[test]
fn test_cag_alias_lookup_never_downgrades_infrared() {
    let cards = [
        "Integrated Camera: Integrated C",
        "Integrated_IR_Camera",
        "USB2.0 FHD UVC WebCam",
        "Acme HD Camera: Acme HD Camera",
    ];
    let format_sets: [&[PixelFormat]; 5] = [
        &[PixelFormat::Mjpeg, PixelFormat::Yuyv],
        &[PixelFormat::Grey],
        &[PixelFormat::Yuyv, PixelFormat::Grey],
        &[PixelFormat::Rgb24],
        &[],
    ];
    let size_sets: [&[(u32, u32)]; 3] = [&[], &[(400, 400)], &[(1280, 720), (640, 480)]];
    let alias_sets: [&[&str]; 5] = [
        &[],
        &["usb-Vendor_Integrated_Camera-video-index0"],
        &["usb-Acme_HD_IR_Camera_0001-video-index2"],
        &[
            "usb-Vendor_infrared_cam-video-index0",
            "usb-Vendor_Cam-video-index0",
        ],
        &[
            "usb-RGB_Only_Webcam-video-index0",
            "usb-Colour_Cam-video-index1",
        ],
    ];

    let mut checked = 0usize;
    for aliases in alias_sets {
        let fake = FakeDev::new(&["video3"]);
        for alias in aliases {
            fake.alias(alias, "video3");
        }
        let node = fake.node("video3");
        for card in cards {
            for formats in format_sets {
                for sizes in size_sets {
                    let before = hints(&node, sizes);
                    let after = supervisor_alias_hints(before.clone(), &node, &fake.by_id);
                    assert_eq!(after.frame_sizes, before.frame_sizes);
                    if let Some(name) = after.by_id_name.as_deref() {
                        assert!(
                            aliases.contains(&name),
                            "{name} is one of the node's aliases"
                        );
                    } else {
                        assert!(aliases.is_empty());
                    }
                    let without = classify_sensor_with_hints(card, formats, &before);
                    let with = classify_sensor_with_hints(card, formats, &after);
                    if without == SensorType::Infrared {
                        assert_eq!(
                            with,
                            SensorType::Infrared,
                            "{card} {formats:?} {sizes:?} {aliases:?}: the alias lookup must \
                             never turn an Infrared node into {with:?}"
                        );
                    }
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 5 * 4 * 5 * 3);
}
