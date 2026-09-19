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

use soos_evidence_store::store::MAX_VALID_UID;
use soos_evidence_store::{EvidenceConfig, EvidenceStore, EvidenceStoreError};
use std::fs;
use std::os::unix::fs::symlink;
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;

/// Contractual Test #30.1: Symlink check before creating date-based directories.
/// Rejects symlink traversal when date directory is a symlink.
#[test]
fn test_evidence_store_rejects_symlink_date_directory() {
    let temp = TempDir::new().unwrap();
    let base_dir = temp.path().join("evidence");
    let key_path = temp.path().join("evidence.key");
    let escape_target = temp.path().join("escape_target");

    fs::create_dir_all(&base_dir).unwrap();
    fs::create_dir_all(&escape_target).unwrap();

    let config = EvidenceConfig {
        enabled: true,
        base_dir: base_dir.clone(),
        key_path,
        retention_days: 7,
        daily_cap_per_uid: 10,
    };

    let store = EvidenceStore::open(config).unwrap();

    // Create a symlink in base_dir pretending to be a date directory "2026-09-19"
    // pointing to escape_target outside evidence directory.
    let symlink_date_dir = base_dir.join("2026-09-19");
    symlink(&escape_target, &symlink_date_dir).unwrap();

    // Attempting to store a snapshot into the symlinked date directory must fail closed
    let result = store.store_snapshot(
        1000,
        "auth_failure",
        b"raw_frame_data",
        Some("2026-09-19"),
        None,
    );

    assert!(
        result.is_err(),
        "ACCEPTANCE CRITERION #30.1 VIOLATION: Storing snapshot into symlinked date directory must fail"
    );

    match result.unwrap_err() {
        EvidenceStoreError::InvalidPath(msg) => {
            assert!(
                msg.contains("symlink"),
                "Expected error message indicating symlink rejection, got: {msg}"
            );
        }
        other => panic!("Expected EvidenceStoreError::InvalidPath, got: {other:?}"),
    }

    // Verify zero files written into escape_target
    let escape_files: Vec<_> = fs::read_dir(&escape_target).unwrap().collect();
    assert!(
        escape_files.is_empty(),
        "Symlink traversal succeeded: files were written into target directory!"
    );
}

/// Contractual Test #30.2: File locking for concurrent retention rotation.
/// Concurrent retention rotations must not corrupt the evidence store or crash.
#[test]
fn test_concurrent_rotation_does_not_corrupt() {
    let temp = TempDir::new().unwrap();
    let base_dir = temp.path().join("evidence");
    let key_path = temp.path().join("evidence.key");
    fs::create_dir_all(&base_dir).unwrap();

    let config = EvidenceConfig {
        enabled: true,
        base_dir: base_dir.clone(),
        key_path,
        retention_days: 7,
        daily_cap_per_uid: 100,
    };

    let store = Arc::new(EvidenceStore::open(config).unwrap());

    // Populate multiple date directories
    // 2026-09-01, 2026-09-02, 2026-09-05 should be pruned (> 7 days old relative to 2026-09-20)
    // 2026-09-15, 2026-09-20 should be kept
    let old_dates = ["2026-09-01", "2026-09-02", "2026-09-05"];
    let keep_dates = ["2026-09-15", "2026-09-20"];

    for d in old_dates.iter().chain(keep_dates.iter()) {
        store
            .store_snapshot(1000, "rotation_test", b"frame", Some(d), None)
            .unwrap();
        assert!(base_dir.join(d).exists());
    }

    // Spawn 8 concurrent threads executing rotate_retention concurrently
    let mut handles = Vec::new();
    for _ in 0..8 {
        let store_clone = Arc::clone(&store);
        handles.push(thread::spawn(move || {
            store_clone.rotate_retention("2026-09-20")
        }));
    }

    let mut total_pruned_reported = 0;
    for handle in handles {
        let report_res = handle.join().expect("Thread panicked during rotation");
        assert!(
            report_res.is_ok(),
            "Concurrent rotation returned error: {:?}",
            report_res.err()
        );
        let report = report_res.unwrap();
        total_pruned_reported += report.directories_pruned;
    }

    // Since 3 old directories were pruned across all concurrent rotations,
    // the total pruned across threads must be at least 3 (distributed across threads)
    assert!(
        total_pruned_reported >= 3,
        "Expected at least 3 pruned directories reported across concurrent rotations, got {total_pruned_reported}"
    );

    // Verify filesystem state: old dates are deleted, keep dates remain intact
    for d in &old_dates {
        assert!(
            !base_dir.join(d).exists(),
            "Old date directory {d} should have been pruned"
        );
    }
    for d in &keep_dates {
        assert!(
            base_dir.join(d).exists(),
            "Recent date directory {d} must be retained"
        );
    }
}

/// Contractual Test #30.3: Validate UID parameter in store_snapshot().
/// Rejects negative (cast to unsigned) or excessively large UIDs.
#[test]
fn test_evidence_store_rejects_path_traversal_uid() {
    let temp = TempDir::new().unwrap();
    let base_dir = temp.path().join("evidence");
    let key_path = temp.path().join("evidence.key");
    fs::create_dir_all(&base_dir).unwrap();

    let config = EvidenceConfig {
        enabled: true,
        base_dir: base_dir.clone(),
        key_path,
        retention_days: 7,
        daily_cap_per_uid: 10,
    };

    let store = EvidenceStore::open(config).unwrap();

    // 1. Test u32::MAX (which represents -1 as signed 32-bit integer or (uid_t)-1)
    let err_max = store.store_snapshot(u32::MAX, "test", b"image", None, None);
    assert!(
        err_max.is_err(),
        "ACCEPTANCE CRITERION #30.3 VIOLATION: u32::MAX must be rejected"
    );
    match err_max.unwrap_err() {
        EvidenceStoreError::InvalidUid(uid) => assert_eq!(uid, u32::MAX),
        other => panic!("Expected EvidenceStoreError::InvalidUid, got: {other:?}"),
    }

    // 2. Test negative integer cast to u32
    let neg_uid = (-1000i32) as u32;
    let err_neg = store.store_snapshot(neg_uid, "test", b"image", None, None);
    assert!(
        err_neg.is_err(),
        "ACCEPTANCE CRITERION #30.3 VIOLATION: negative UID cast to u32 must be rejected"
    );
    match err_neg.unwrap_err() {
        EvidenceStoreError::InvalidUid(uid) => assert_eq!(uid, neg_uid),
        other => panic!("Expected EvidenceStoreError::InvalidUid, got: {other:?}"),
    }

    // 3. Test UID exceeding MAX_VALID_UID
    let too_large = MAX_VALID_UID + 1;
    let err_large = store.store_snapshot(too_large, "test", b"image", None, None);
    assert!(
        err_large.is_err(),
        "ACCEPTANCE CRITERION #30.3 VIOLATION: UID exceeding MAX_VALID_UID must be rejected"
    );
    match err_large.unwrap_err() {
        EvidenceStoreError::InvalidUid(uid) => assert_eq!(uid, too_large),
        other => panic!("Expected EvidenceStoreError::InvalidUid, got: {other:?}"),
    }

    // 4. Verify valid POSIX UIDs are accepted
    assert!(store
        .store_snapshot(0, "root", b"image", None, None)
        .is_ok());
    assert!(store
        .store_snapshot(1000, "user", b"image", None, None)
        .is_ok());
    assert!(store
        .store_snapshot(65534, "nobody", b"image", None, None)
        .is_ok());
    assert!(store
        .store_snapshot(MAX_VALID_UID, "max_valid", b"image", None, None)
        .is_ok());
}
