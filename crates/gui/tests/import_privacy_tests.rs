//! Contractual tests for the GUI template import path and store selection (review findings
//! CAM-08 / STO-12, GitHub #156).
//!
//! Contract:
//! - Import finalize pipes the fused embedding to `soos-enroll import --uid <uid> --file -`
//!   over the helper's stdin; no file is ever created under `temp_dir()`.
//! - A failing or early-exiting helper is reported as an error and never hangs the worker.
//! - The GUI no longer falls back silently to `temp_dir()/soos-gui-master.key` /
//!   `soos-gui-biometrics`: an inaccessible system store selects the Polkit mode (no local
//!   store at all), and a local store exists only through the explicit `--dev-store <DIR>`
//!   developer mode, which carries a visible banner.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Parser;
use soos_gui::args::GuiArgs;
use soos_gui::privileged::{import_helper_args, import_template_with};
use soos_gui::store_mode::{resolve_gui_store, GuiStore};
use tempfile::tempdir;

const GUI_IMPORT_PREFIX: &str = ".soos_gui_import_";

fn temp_import_files() -> Vec<String> {
    std::fs::read_dir(std::env::temp_dir())
        .map(|it| {
            it.filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with(GUI_IMPORT_PREFIX))
                .collect()
        })
        .unwrap_or_default()
}

/// Writes a fake helper recording its argv, its stdin and the GUI import files visible in
/// `temp_dir()` while it runs (i.e. while the Polkit prompt would be open).
fn fake_helper(dir: &Path, exit_code: i32) -> PathBuf {
    let script = dir.join("helper.sh");
    let body = format!(
        "printf '%s\\n' \"$@\" > '{out}/args'\n\
         cat > '{out}/stdin'\n\
         ls -a '{tmp}' | grep '^{prefix}' > '{out}/tmpfiles' || true\n\
         exit {exit_code}\n",
        out = dir.display(),
        tmp = std::env::temp_dir().display(),
        prefix = GUI_IMPORT_PREFIX,
    );
    std::fs::write(&script, body).unwrap();
    script
}

#[test]
fn test_import_helper_args_use_stdin() {
    assert_eq!(
        import_helper_args(1000),
        vec!["soos-enroll", "import", "--uid", "1000", "--file", "-"]
    );
}

#[test]
fn test_import_finalize_pipes_embedding_without_temp_file() {
    let dir = tempdir().unwrap();
    let script = fake_helper(dir.path(), 0);
    let before = temp_import_files();
    let embedding: Vec<f32> = (0..512u16).map(|i| f32::from(i) / 1024.0).collect();

    let mut cmd = Command::new("/bin/sh");
    cmd.arg(&script);
    import_template_with(cmd, 1000, &embedding).expect("import must succeed");

    let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
    assert_eq!(args, "soos-enroll\nimport\n--uid\n1000\n--file\n-\n");
    let piped: Vec<f32> =
        serde_json::from_slice(&std::fs::read(dir.path().join("stdin")).unwrap()).unwrap();
    assert_eq!(
        piped, embedding,
        "the embedding must arrive on the helper stdin"
    );

    let during = std::fs::read_to_string(dir.path().join("tmpfiles")).unwrap();
    let new_during: Vec<&str> = during
        .lines()
        .filter(|n| !before.iter().any(|b| b == n))
        .collect();
    assert!(
        new_during.is_empty(),
        "no GUI import file may exist under temp_dir while the helper runs: {new_during:?}"
    );
    let after = temp_import_files();
    assert!(
        after.iter().all(|n| before.contains(n)),
        "no GUI import file may be created under temp_dir: {after:?}"
    );
}

#[test]
fn test_import_reports_helper_failure() {
    let dir = tempdir().unwrap();
    let script = fake_helper(dir.path(), 3);
    let mut cmd = Command::new("/bin/sh");
    cmd.arg(&script);
    assert!(import_template_with(cmd, 1000, &[0.5f32; 512]).is_err());
}

#[test]
fn test_import_does_not_hang_when_helper_ignores_stdin() {
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", "exit 1"]);
    assert!(import_template_with(cmd, 1000, &[0.5f32; 512]).is_err());
}

#[test]
fn test_import_and_store_code_never_use_temp_dir() {
    let privileged = include_str!("../src/privileged.rs");
    assert!(
        !privileged.contains("temp_dir"),
        "privileged.rs must not stage the embedding under temp_dir"
    );
    let main = include_str!("../src/main.rs");
    let store_mode = include_str!("../src/store_mode.rs");
    for (name, src) in [("main.rs", main), ("store_mode.rs", store_mode)] {
        assert!(
            !src.contains("temp_dir")
                && !src.contains("soos-gui-master.key")
                && !src.contains("soos-gui-biometrics"),
            "{name} must not fall back to a /tmp key or store"
        );
    }
}

#[test]
fn test_accessible_system_store_is_used_directly() {
    let dir = tempdir().unwrap();
    let store = resolve_gui_store(
        &dir.path().join("master.key"),
        &dir.path().join("biometrics"),
        None,
    )
    .unwrap();
    assert!(matches!(store, GuiStore::System(_)));
    assert!(store.local().is_some());
    assert!(!store.uses_polkit());
    assert!(store.banner().is_none());
}

#[test]
fn test_inaccessible_system_store_selects_polkit_without_tmp_fallback() {
    let legacy_key = std::env::temp_dir().join("soos-gui-master.key");
    let legacy_store = std::env::temp_dir().join("soos-gui-biometrics");
    let key_existed = legacy_key.exists();
    let store_existed = legacy_store.exists();

    let store = resolve_gui_store(
        Path::new("/proc/soos-no-such-dir/master.key"),
        Path::new("/proc/soos-no-such-dir/biometrics"),
        None,
    )
    .unwrap();
    assert!(matches!(store, GuiStore::Polkit));
    assert!(
        store.local().is_none(),
        "no local store without --dev-store"
    );
    assert!(store.uses_polkit());
    if !key_existed {
        assert!(!legacy_key.exists(), "no key may be created under temp_dir");
    }
    if !store_existed {
        assert!(
            !legacy_store.exists(),
            "no store may be created under temp_dir"
        );
    }
}

#[test]
fn test_dev_store_is_explicit_and_announced() {
    let dir = tempdir().unwrap();
    let dev = dir.path().join("dev");
    let store = resolve_gui_store(
        Path::new("/proc/soos-no-such-dir/master.key"),
        Path::new("/proc/soos-no-such-dir/biometrics"),
        Some(&dev),
    )
    .unwrap();
    match &store {
        GuiStore::Developer { dir: d, .. } => assert_eq!(d, &dev),
        _ => panic!("--dev-store must select the developer store"),
    }
    assert!(store.local().is_some());
    assert!(
        !store.uses_polkit(),
        "developer mode never touches the system store"
    );
    assert!(dev.join("master.key").is_file());
    assert!(dev.join("biometrics").is_dir());
    let banner = store.banner().expect("developer mode must show a banner");
    assert!(banner.contains("DEVELOPER"), "{banner}");
    assert!(banner.contains(&dev.display().to_string()), "{banner}");
    assert!(banner.contains("PAM"), "{banner}");

    assert!(
        resolve_gui_store(
            Path::new("/proc/x/master.key"),
            Path::new("/proc/x/bio"),
            Some(Path::new("relative/dev")),
        )
        .is_err(),
        "a relative --dev-store path must be refused"
    );
}

#[test]
fn test_gui_args_accept_dev_store() {
    let args = GuiArgs::try_parse_from(["soos-gui", "--dev-store", "/home/u/soos-dev"]).unwrap();
    assert_eq!(args.dev_store, Some(PathBuf::from("/home/u/soos-dev")));
    let args = GuiArgs::try_parse_from(["soos-gui"]).unwrap();
    assert_eq!(args.dev_store, None, "developer mode must be opt-in");
}
