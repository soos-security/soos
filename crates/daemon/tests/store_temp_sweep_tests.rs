//! GitHub #291 (SGU7): `soos-daemon` sweeps the temporary files abandoned by an interrupted
//! store write once at startup, through the stores' own bounded, symlink-safe sweeps.
//!
//! `pipeline::sweep_orphaned_store_temp_files` runs both sweeps (biometric templates and
//! evidence snapshots) and never fails the startup; `initialize_pipeline` calls it once, after
//! both stores are opened.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions"
)]

use std::fs::File;
use std::path::Path;
use std::time::{Duration, SystemTime};

use soos_daemon::pipeline::sweep_orphaned_store_temp_files;
use tempfile::TempDir;

fn plant_old(dir: &Path, name: &str) {
    let path = dir.join(name);
    std::fs::write(&path, b"orphan").unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(600))
        .unwrap();
}

#[test]
fn test_sgu_daemon_startup_sweep_cleans_both_stores_and_keeps_decoys() {
    let temp = TempDir::new().unwrap();
    let bio_dir = temp.path().join("biometrics");
    let biometric = soos_biometric_store::BiometricStore::new(
        &bio_dir,
        soos_biometric_store::MasterKey::generate().unwrap(),
    )
    .unwrap();
    let ev_base = temp.path().join("evidence");
    let evidence = soos_evidence_store::EvidenceStore::new(
        soos_evidence_store::EvidenceConfig::enabled_with_dir(
            ev_base.clone(),
            temp.path().join("ev.key"),
        ),
        soos_evidence_store::MasterKey::generate().unwrap(),
    );
    let day = ev_base.join("2026-09-30");
    std::fs::create_dir_all(&day).unwrap();

    plant_old(&bio_dir, "1000.tmp.4242.17");
    plant_old(&bio_dir, "1000.tmp.4242.17.keep");
    plant_old(
        &day,
        ".tmp.0f8c2a4e-1b3d-4c5e-8f60-718293a4b5c6.4242.00ff00ff00ff00ff",
    );
    plant_old(&day, ".tmp.keep-me");

    sweep_orphaned_store_temp_files(&biometric, &evidence);

    assert!(!bio_dir.join("1000.tmp.4242.17").exists());
    assert!(bio_dir.join("1000.tmp.4242.17.keep").exists());
    assert!(!day
        .join(".tmp.0f8c2a4e-1b3d-4c5e-8f60-718293a4b5c6.4242.00ff00ff00ff00ff")
        .exists());
    assert!(day.join(".tmp.keep-me").exists());
}

/// A failing sweep (symlinked evidence base) never prevents the other sweep nor the startup.
#[test]
fn test_sgu_daemon_startup_sweep_failure_is_not_fatal() {
    let temp = TempDir::new().unwrap();
    let bio_dir = temp.path().join("biometrics");
    let biometric = soos_biometric_store::BiometricStore::new(
        &bio_dir,
        soos_biometric_store::MasterKey::generate().unwrap(),
    )
    .unwrap();
    let real = temp.path().join("real-evidence");
    std::fs::create_dir(&real).unwrap();
    let ev_base = temp.path().join("evidence");
    std::os::unix::fs::symlink(&real, &ev_base).unwrap();
    let evidence = soos_evidence_store::EvidenceStore::new(
        soos_evidence_store::EvidenceConfig::enabled_with_dir(ev_base, temp.path().join("ev.key")),
        soos_evidence_store::MasterKey::generate().unwrap(),
    );
    plant_old(&bio_dir, "1000.tmp.1.2");

    sweep_orphaned_store_temp_files(&biometric, &evidence);

    assert!(!bio_dir.join("1000.tmp.1.2").exists());
}

#[test]
fn test_sgu_initialize_pipeline_sweeps_once_after_opening_both_stores() {
    let src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/pipeline.rs")).unwrap();
    let body_start = src.find("pub fn initialize_pipeline(").unwrap();
    let body = &src[body_start..];
    let body = &body[..body.find("\n}\n").unwrap()];
    let call = body
        .find("sweep_orphaned_store_temp_files(")
        .expect("initialize_pipeline must sweep the orphaned store temp files");
    assert_eq!(body.matches("sweep_orphaned_store_temp_files(").count(), 1);
    let evidence_opened = body.find("EvidenceStore::new(").unwrap();
    let biometric_opened = body.find("BiometricStore::new(").unwrap();
    assert!(call > evidence_opened && call > biometric_opened);
}
