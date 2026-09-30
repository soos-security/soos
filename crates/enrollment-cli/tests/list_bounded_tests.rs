//! GitHub #235 (STO-19): `list` refuses an oversized template file instead of allocating it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "Contractual test suite utilizes direct assertions and unwrap"
)]

use std::io::Write;
use std::sync::Arc;

use tempfile::TempDir;
use zeroize::Zeroizing;

use soos_biometric_store::{
    BiometricStore, BiometricStoreError, BiometricTemplate, MasterKey, MAGIC_HEADER,
    TEMPLATE_EXTENSION,
};
use soos_enrollment_cli::args::ListArgs;
use soos_enrollment_cli::error::EnrollmentCliError;
use soos_enrollment_cli::service::EnrollmentService;

fn template(uid: u32, timestamp: u64) -> BiometricTemplate {
    BiometricTemplate::new(
        uid,
        "arcface_w600k_mbf".to_string(),
        "2.0.0".to_string(),
        timestamp,
        Zeroizing::new(vec![0.25f32; 512]),
    )
    .unwrap()
}

#[test]
fn test_list_refuses_oversized_template_file() {
    let temp = TempDir::new().unwrap();
    let dir = temp.path().join("biometrics");
    let store = Arc::new(BiometricStore::new(&dir, MasterKey::generate().unwrap()).unwrap());
    store.enroll(&template(1000, 1)).unwrap();

    let mut f = std::fs::File::create(dir.join(format!("1001{TEMPLATE_EXTENSION}"))).unwrap();
    f.write_all(MAGIC_HEADER).unwrap();
    f.write_all(&vec![0x5Au8; 10 * 1024 * 1024]).unwrap();
    drop(f);

    let service = EnrollmentService::new_store_only(store, false);
    let res = service.list(&ListArgs::default());
    assert!(
        matches!(
            res,
            Err(EnrollmentCliError::BiometricStore(
                BiometricStoreError::CorruptFile(_)
            ))
        ),
        "an oversized template must be refused as corrupt, got {res:?}"
    );
}

#[test]
fn test_list_reports_metadata_of_enrolled_templates() {
    let temp = TempDir::new().unwrap();
    let store = Arc::new(
        BiometricStore::new(temp.path().join("b"), MasterKey::generate().unwrap()).unwrap(),
    );
    for uid in [2002u32, 2001] {
        store.enroll(&template(uid, u64::from(uid))).unwrap();
    }
    let service = EnrollmentService::new_store_only(store, false);
    let list = service.list(&ListArgs::default()).unwrap();
    let uids: Vec<u32> = list.iter().map(|s| s.uid).collect();
    assert_eq!(uids, vec![2001, 2002]);
    assert!(list.iter().all(|s| s.embedding_dim == 512));
    assert!(list.iter().all(|s| s.model_id == "arcface_w600k_mbf"));
    assert_eq!(list.first().map(|s| s.enrollment_timestamp), Some(2001));
}
