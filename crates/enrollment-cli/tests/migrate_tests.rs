//! GitHub #287 (owner decision 2026-10-01): `soos-enroll migrate [--dry-run]` re-encrypts every
//! legacy unbound (v1) template and evidence snapshot to the AAD-bound (v2) envelope.
//!
//! Legacy fixtures are written through the unbound `encrypt_payload` codecs, exactly like the
//! store crates' `aad_migration_tests.rs` and `aad_binding_tests.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use clap::Parser;
use soos_biometric_store::{
    encrypt_payload, BiometricStore, BiometricTemplate, MasterKey, PayloadFormat,
};
use soos_enrollment_cli::args::{Cli, Commands, MigrateArgs, OutputFormat};
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::{
    format_migration_json, open_evidence_store_for_migration, open_template_store_for_migration,
    run_migration, EnrollmentService,
};
use soos_evidence_store::{EvidenceConfig, EvidenceStore};
use tempfile::TempDir;
use zeroize::Zeroizing;

fn template(uid: u32, value: f32) -> BiometricTemplate {
    BiometricTemplate::new(
        uid,
        "arcface_w600k_mbf".to_string(),
        "2.0.0".to_string(),
        1_700_000_000,
        Zeroizing::new(vec![value; 512]),
    )
    .unwrap()
}

struct Env {
    _temp: TempDir,
    store: Arc<BiometricStore>,
    key: MasterKey,
    evidence: EvidenceStore,
    evidence_key: soos_evidence_store::MasterKey,
    evidence_dir: PathBuf,
}

fn env() -> Env {
    let temp = TempDir::new().unwrap();
    let key = MasterKey::generate().unwrap();
    let store = Arc::new(BiometricStore::new(temp.path().join("bio"), key.clone()).unwrap());
    let evidence_dir = temp.path().join("evidence");
    let evidence_key = soos_evidence_store::MasterKey::generate().unwrap();
    let evidence = EvidenceStore::new(
        EvidenceConfig::enabled_with_dir(evidence_dir.clone(), temp.path().join("evidence.key")),
        evidence_key.clone(),
    );
    Env {
        _temp: temp,
        store,
        key,
        evidence,
        evidence_key,
        evidence_dir,
    }
}

fn write_legacy_template(env: &Env, uid: u32) -> Vec<u8> {
    let sealed = encrypt_payload(&env.key, &template(uid, 0.25).to_cbor().unwrap()).unwrap();
    fs::write(env.store.template_path(uid).unwrap(), &sealed).unwrap();
    sealed
}

fn write_legacy_snapshot(env: &Env) -> PathBuf {
    let stored = env
        .evidence
        .store_snapshot(
            1000,
            "PasswordFailed",
            b"frame",
            Some("2026-09-14"),
            Some(1),
        )
        .unwrap();
    let record = env.evidence.load_snapshot(&stored.path).unwrap();
    let legacy =
        soos_evidence_store::crypto::encrypt_payload(&env.evidence_key, &record.to_cbor().unwrap())
            .unwrap();
    fs::write(&stored.path, legacy).unwrap();
    stored.path
}

fn service(env: &Env) -> EnrollmentService {
    EnrollmentService::new_store_only(env.store.clone(), false)
}

#[test]
fn test_smi_cli_migrates_templates_and_evidence_then_reports_zero() {
    let env = env();
    write_legacy_template(&env, 1000);
    env.store.enroll(&template(1001, 0.5)).unwrap();
    let snapshot = write_legacy_snapshot(&env);

    let summary = service(&env)
        .migrate(&MigrateArgs::default(), Some(&env.evidence))
        .unwrap();
    assert!(!summary.dry_run);
    assert_eq!(summary.templates.migrated, 1);
    assert_eq!(summary.templates.already_current, 1);
    assert_eq!(summary.templates.failed, 0);
    assert_eq!(summary.evidence.migrated, 1);
    assert_eq!(summary.evidence.failed, 0);
    assert!(summary.evidence.skipped.is_none());
    assert!(!summary.has_failures());
    assert_eq!(
        env.store.template_format(1000).unwrap(),
        Some(PayloadFormat::BoundV2)
    );
    assert!(env.evidence.load_snapshot(&snapshot).is_ok());

    let again = service(&env)
        .migrate(&MigrateArgs::default(), Some(&env.evidence))
        .unwrap();
    assert_eq!(again.templates.migrated, 0, "second run migrates nothing");
    assert_eq!(again.templates.already_current, 2);
    assert_eq!(again.evidence.migrated, 0);
    assert_eq!(again.evidence.already_current, 1);
}

#[test]
fn test_smi_cli_dry_run_changes_nothing() {
    let env = env();
    let sealed = write_legacy_template(&env, 1000);
    let snapshot = write_legacy_snapshot(&env);
    let snapshot_bytes = fs::read(&snapshot).unwrap();

    let args = MigrateArgs {
        dry_run: true,
        ..MigrateArgs::default()
    };
    let summary = service(&env).migrate(&args, Some(&env.evidence)).unwrap();
    assert!(summary.dry_run);
    assert_eq!(summary.templates.migrated, 1, "reported as would-migrate");
    assert_eq!(summary.evidence.migrated, 1);
    assert_eq!(
        fs::read(env.store.template_path(1000).unwrap()).unwrap(),
        sealed
    );
    assert_eq!(fs::read(&snapshot).unwrap(), snapshot_bytes);
}

#[test]
fn test_smi_cli_reports_per_file_failure_and_continues() {
    let env = env();
    write_legacy_template(&env, 1000);
    let junk = b"SOOSBIO1 junk that never authenticates".to_vec();
    fs::write(env.store.template_path(1001).unwrap(), &junk).unwrap();

    let summary = service(&env)
        .migrate(&MigrateArgs::default(), Some(&env.evidence))
        .unwrap();
    assert_eq!(summary.templates.migrated, 1);
    assert_eq!(summary.templates.failed, 1);
    assert_eq!(summary.templates.failures.len(), 1);
    assert_eq!(summary.templates.failures[0].item, "1001");
    assert!(summary.has_failures());
    assert_eq!(
        fs::read(env.store.template_path(1001).unwrap()).unwrap(),
        junk
    );
}

#[test]
fn test_smi_cli_without_evidence_key_skips_evidence_and_creates_nothing() {
    let env = env();
    write_legacy_template(&env, 1000);
    let temp = TempDir::new().unwrap();
    let key_path = temp.path().join("evidence.key");

    let opened =
        open_evidence_store_for_migration(&temp.path().join("evidence"), &key_path).unwrap();
    assert!(opened.is_none(), "a missing evidence key means no evidence");
    assert!(!key_path.exists(), "the evidence key is never created");

    let summary = service(&env)
        .migrate(&MigrateArgs::default(), opened.as_ref())
        .unwrap();
    assert_eq!(summary.templates.migrated, 1);
    assert!(summary.evidence.skipped.is_some());
    assert_eq!(summary.evidence.migrated, 0);
    assert!(!summary.has_failures());
}

#[test]
fn test_smi_cli_opens_existing_evidence_key() {
    let env = env();
    let snapshot = write_legacy_snapshot(&env);
    let key_path = env.evidence_dir.with_file_name("evidence.key");
    fs::write(&key_path, env.evidence_key.as_bytes()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();

    let opened = open_evidence_store_for_migration(&env.evidence_dir, &key_path)
        .unwrap()
        .expect("existing key opens the evidence store");
    let summary = service(&env)
        .migrate(&MigrateArgs::default(), Some(&opened))
        .unwrap();
    assert_eq!(summary.evidence.migrated, 1);
    assert!(env.evidence.load_snapshot(&snapshot).is_ok());
}

#[test]
fn test_smi_cli_migrate_requires_root() {
    let env = env();
    let service = EnrollmentService::new_store_only(env.store.clone(), true);
    let res = service.migrate(&MigrateArgs::default(), None);
    if nix::unistd::geteuid().is_root() {
        assert!(res.is_ok());
    } else {
        assert!(matches!(res, Err(EnrollmentCliError::RootRequired)));
    }
}

#[test]
fn test_smi_cli_json_summary_is_valid_and_carries_no_embedding() {
    let env = env();
    let foreign = encrypt_payload(&env.key, &template(1000, 0.123_456).to_cbor().unwrap()).unwrap();
    fs::write(env.store.template_path(1001).unwrap(), foreign).unwrap();
    write_legacy_template(&env, 1002);

    let summary = service(&env)
        .migrate(&MigrateArgs::default(), None)
        .unwrap();
    let json = format_migration_json(&summary);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["dry_run"], false);
    assert_eq!(value["templates"]["migrated"], 1);
    assert_eq!(value["templates"]["already_current"], 0);
    assert_eq!(value["templates"]["failed"], 1);
    assert_eq!(value["templates"]["failures"][0]["item"], "1001");
    assert!(value["evidence"]["skipped"].is_string());
    assert!(!json.contains("0.123"), "no embedding value: {json}");
    assert!(!json.contains("0.25"), "no embedding value: {json}");
}

#[test]
fn test_smi_cli_parses_migrate_flags() {
    let cli = Cli::try_parse_from([
        "soos-enroll",
        "migrate",
        "--dry-run",
        "--format",
        "json",
        "--evidence-dir",
        "/var/lib/soos/evidence",
        "--evidence-key-file",
        "/var/lib/soos/evidence.key",
    ])
    .unwrap();
    match cli.command {
        Commands::Migrate(args) => {
            assert!(args.dry_run);
            assert_eq!(args.format, OutputFormat::Json);
            assert_eq!(
                args.evidence_dir.as_deref(),
                Some(std::path::Path::new("/var/lib/soos/evidence"))
            );
            assert_eq!(
                args.evidence_key_file.as_deref(),
                Some(std::path::Path::new("/var/lib/soos/evidence.key"))
            );
        }
        other => panic!("unexpected command {other:?}"),
    }

    let bare = Cli::try_parse_from(["soos-enroll", "migrate"]).unwrap();
    match bare.command {
        Commands::Migrate(args) => {
            assert!(!args.dry_run, "a real migration needs no flag");
            assert_eq!(args.format, OutputFormat::Table);
        }
        other => panic!("unexpected command {other:?}"),
    }
}

#[test]
fn test_smi_cli_binary_refuses_migrate_without_root() {
    if nix::unistd::geteuid().is_root() {
        return; // The refusal is only observable as a non-root user.
    }
    let out = Command::new(env!("CARGO_BIN_EXE_soos-enroll"))
        .args(["migrate", "--dry-run"])
        .output()
        .expect("spawn soos-enroll");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Root privileges"));
}

#[test]
fn test_smi_cli_without_master_key_skips_templates_and_creates_nothing() {
    let temp = TempDir::new().unwrap();
    let bio_dir = temp.path().join("biometrics");
    let key_path = temp.path().join("soos").join("master.key");

    let opened = open_template_store_for_migration(&bio_dir, &key_path).unwrap();
    assert!(opened.is_none(), "a missing master key means no templates");
    assert!(!key_path.exists(), "the master key is never created");
    assert!(
        !temp.path().join("soos").exists(),
        "no key directory created"
    );
    assert!(!bio_dir.exists(), "no biometrics directory created");

    let summary = run_migration(&MigrateArgs::default(), None, None, false).unwrap();
    assert!(summary.templates.skipped.is_some());
    assert_eq!(summary.templates.migrated, 0);
    assert!(summary.evidence.skipped.is_some());
    assert!(!summary.has_failures());
}

#[test]
fn test_smi_cli_without_biometrics_dir_skips_templates_and_creates_nothing() {
    let temp = TempDir::new().unwrap();
    let bio_dir = temp.path().join("biometrics");
    let key_path = temp.path().join("master.key");
    MasterKey::load_or_create(&key_path).unwrap();

    let opened = open_template_store_for_migration(&bio_dir, &key_path).unwrap();
    assert!(
        opened.is_none(),
        "no biometrics directory means no templates"
    );
    assert!(
        !bio_dir.exists(),
        "the biometrics directory is never created"
    );
}

#[test]
fn test_smi_cli_opens_existing_template_store_and_migrates() {
    let env = env();
    write_legacy_template(&env, 1000);
    let key_path = env.evidence_dir.with_file_name("master.key");
    fs::write(&key_path, env.key.as_bytes()).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();
    let bio_dir = env.evidence_dir.with_file_name("bio");

    let store = open_template_store_for_migration(&bio_dir, &key_path)
        .unwrap()
        .expect("existing key and directory open the store");
    let summary = run_migration(&MigrateArgs::default(), Some(&store), None, false).unwrap();
    assert_eq!(summary.templates.migrated, 1);
    assert_eq!(
        env.store.template_format(1000).unwrap(),
        Some(PayloadFormat::BoundV2)
    );
}

#[test]
fn test_smi_cli_run_migration_requires_root() {
    let res = run_migration(&MigrateArgs::default(), None, None, true);
    if nix::unistd::geteuid().is_root() {
        assert!(res.is_ok());
    } else {
        assert!(matches!(res, Err(EnrollmentCliError::RootRequired)));
    }
}
