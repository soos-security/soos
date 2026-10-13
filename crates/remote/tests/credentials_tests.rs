//! Contract tests of the passkey credential store of `soos-remote` (ADR 2026-10-06
//! "Tailscale Funnel Access and In-House Passkey Authentication for `soos-remote`",
//! architect spec §4.4, tests 27–29, matrix RMC35).
//!
//! Every file lives in a `TempDir`; nothing touches `~/.config`.

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

#[path = "common/passkey.rs"]
mod passkey;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use passkey::*;
use soos_remote::auth::{RandomError, RandomSource};
use soos_remote::credentials::{
    encode_store, parse_store, CredentialStore, PasskeyFile, PasskeyRecord, StoreError,
};
use soos_remote::{
    MAX_CREDENTIAL_ID_BYTES, MAX_CREDENTIAL_STORE_BYTES, MAX_PASSKEYS, STORE_LOCK_TIMEOUT_MS,
    USER_HANDLE_BYTES,
};

const HANDLE: [u8; USER_HANDLE_BYTES] = [0x3c; USER_HANDLE_BYTES];

fn record(auth: &Authenticator, sign_count: u32) -> PasskeyRecord {
    PasskeyRecord {
        credential_id: auth.credential_id.clone(),
        public_key: auth.public_key(),
        sign_count,
        backup_eligible: auth.synced,
        backup_state: auth.synced,
        created_unix_s: 1_700_000_000,
        transports: Default::default(),
    }
}

fn keys() -> Vec<Authenticator> {
    vec![
        Authenticator::owner(),
        Authenticator::device(0x51, b"device-0051"),
        Authenticator::device(0x52, b"device-0052"),
        Authenticator::device(0x53, b"device-0053"),
        Authenticator::device(0x54, b"device-0054"),
    ]
}

fn file_of(auths: &[Authenticator]) -> PasskeyFile {
    PasskeyFile {
        user_handle: HANDLE,
        passkeys: auths.iter().map(|a| record(a, 0)).collect(),
    }
}

fn scripted_random() -> RandomSource {
    let counter = Arc::new(AtomicU8::new(1));
    Arc::new(move |buf: &mut [u8]| -> Result<(), RandomError> {
        for b in buf.iter_mut() {
            *b = counter.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    })
}

fn store_path(dir: &Path) -> PathBuf {
    dir.join("remote-passkeys.json")
}

fn write_store(path: &Path, auths: &[Authenticator], mode: u32) {
    let passkeys: Vec<StoredPasskey> = auths.iter().map(StoredPasskey::of).collect();
    write_file_mode(path, store_json(&HANDLE, &passkeys).as_bytes(), mode);
}

/// Test 27 (RMC35): encode/parse round trip, the exact on-disk format of spec §4.4, and every
/// refusal (unknown key, version, record count, duplicate id, invalid key or id, size).
#[test]
fn test_rmc_credential_store_roundtrip_and_validation() {
    let all = keys();
    let two = file_of(&all[..2]);
    let encoded = encode_store(&two);
    assert!(encoded.len() <= MAX_CREDENTIAL_STORE_BYTES);
    assert!(std::str::from_utf8(&encoded).is_ok());
    assert!(parse_store(&encoded) == Ok(two.clone()), "round trip");

    // The hand-written format of spec §4.4 parses to the same value.
    let passkeys: Vec<StoredPasskey> = all[..2].iter().map(StoredPasskey::of).collect();
    let text = store_json(&HANDLE, &passkeys);
    assert!(
        parse_store(text.as_bytes()) == Ok(two.clone()),
        "spec format"
    );
    let json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["user_handle"], b64url(&HANDLE));
    assert_eq!(
        json["passkeys"][0]["credential_id"],
        all[0].credential_id_b64()
    );
    assert_eq!(
        json["passkeys"][0]["public_key"],
        b64url(&all[0].public_key())
    );
    assert_eq!(json["passkeys"][1]["sign_count"], 0);

    // Zero records and MAX_PASSKEYS records are valid.
    let empty = file_of(&[]);
    assert!(parse_store(&encode_store(&empty)) == Ok(empty.clone()));
    let full = file_of(&all[..MAX_PASSKEYS]);
    assert!(parse_store(&encode_store(&full)) == Ok(full.clone()));
    // The largest legal store fits the bound.
    let big: Vec<Authenticator> = (0..MAX_PASSKEYS)
        .map(|i| {
            let mut id = vec![0xee; MAX_CREDENTIAL_ID_BYTES];
            id[0] = i as u8;
            Authenticator::device(0x60 + i as u8, &id)
        })
        .collect();
    let big_file = file_of(&big);
    let big_encoded = encode_store(&big_file);
    assert!(big_encoded.len() <= MAX_CREDENTIAL_STORE_BYTES);
    assert!(parse_store(&big_encoded) == Ok(big_file));

    let malformed = |text: &str| parse_store(text.as_bytes()).err();
    let rec = |a: &Authenticator| {
        format!(
            "{{\"credential_id\":\"{}\",\"public_key\":\"{}\",\"sign_count\":0,\"backup_eligible\":false,\"backup_state\":false,\"created_unix_s\":1}}",
            a.credential_id_b64(),
            b64url(&a.public_key())
        )
    };
    let h = b64url(&HANDLE);
    let ok = format!(
        "{{\"version\":1,\"user_handle\":\"{h}\",\"passkeys\":[{}]}}",
        rec(&all[1])
    );
    assert!(parse_store(ok.as_bytes()).is_ok(), "baseline");
    let mut off_curve = all[1].public_key();
    off_curve[64] ^= 1;
    let mut cases = vec![
        format!("{{\"version\":1,\"user_handle\":\"{h}\",\"passkeys\":[],\"extra\":1}}"),
        format!("{{\"version\":2,\"user_handle\":\"{h}\",\"passkeys\":[]}}"),
        format!("{{\"version\":0,\"user_handle\":\"{h}\",\"passkeys\":[]}}"),
        format!("{{\"user_handle\":\"{h}\",\"passkeys\":[]}}"),
        "{\"version\":1,\"passkeys\":[]}".to_string(),
        format!("{{\"version\":1,\"user_handle\":\"{h}\"}}"),
        format!(
            "{{\"version\":1,\"user_handle\":\"{}\",\"passkeys\":[]}}",
            b64url(&HANDLE[..15])
        ),
        format!("{{\"version\":1,\"user_handle\":\"{h}=\",\"passkeys\":[]}}"),
        format!(
            "{{\"version\":1,\"user_handle\":\"{h}\",\"passkeys\":[{}]}}",
            rec(&all[1]).replace("\"created_unix_s\":1", "\"created_unix_s\":1,\"x\":1")
        ),
        format!(
            "{{\"version\":1,\"user_handle\":\"{h}\",\"passkeys\":[{},{}]}}",
            rec(&all[1]),
            rec(&all[1])
        ),
        format!(
            "{{\"version\":1,\"user_handle\":\"{h}\",\"passkeys\":[{}]}}",
            rec(&all[1]).replace(&b64url(&all[1].public_key()), &b64url(&off_curve))
        ),
        format!(
            "{{\"version\":1,\"user_handle\":\"{h}\",\"passkeys\":[{}]}}",
            rec(&all[1]).replace(
                &b64url(&all[1].public_key()),
                &b64url(&all[1].public_key()[..64])
            )
        ),
        format!(
            "{{\"version\":1,\"user_handle\":\"{h}\",\"passkeys\":[{}]}}",
            rec(&all[1]).replace(&all[1].credential_id_b64(), "")
        ),
        format!(
            "{{\"version\":1,\"user_handle\":\"{h}\",\"passkeys\":[{}]}}",
            rec(&all[1]).replace(
                &all[1].credential_id_b64(),
                &b64url(&vec![1u8; MAX_CREDENTIAL_ID_BYTES + 1])
            )
        ),
        "not json".to_string(),
        String::new(),
        "[]".to_string(),
    ];
    let five: Vec<String> = all.iter().map(rec).collect();
    cases.push(format!(
        "{{\"version\":1,\"user_handle\":\"{h}\",\"passkeys\":[{}]}}",
        five.join(",")
    ));
    for case in &cases {
        assert_eq!(
            malformed(case),
            Some(StoreError::Malformed),
            "{}",
            &case[..case.len().min(120)]
        );
    }

    // Size bound: padded to exactly the bound is accepted, one byte more is TooLarge.
    let mut at_bound = ok.clone().into_bytes();
    at_bound.resize(MAX_CREDENTIAL_STORE_BYTES, b' ');
    assert!(parse_store(&at_bound).is_ok());
    at_bound.push(b' ');
    assert_eq!(parse_store(&at_bound).err(), Some(StoreError::TooLarge));

    // Fixed English error texts.
    for (err, text) in [
        (StoreError::Io, "credential store unreadable"),
        (
            StoreError::Insecure,
            "credential store permissions are insecure",
        ),
        (StoreError::TooLarge, "credential store too large"),
        (StoreError::Malformed, "credential store malformed"),
        (StoreError::Busy, "credential store busy"),
        (StoreError::Full, "credential store full"),
        (StoreError::Duplicate, "passkey already registered"),
        (StoreError::NoSuchPasskey, "no such passkey"),
        (
            StoreError::UserHandleConflict,
            "registration conflicts with the stored user",
        ),
    ] {
        assert_eq!(err.to_string(), text);
    }
}

/// Runs `f` on another thread and fails if it does not return within 5 s (a FIFO or a
/// blocking open must never stall the service).
fn bounded<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the store never blocks")
}

/// Test 28 (RMC35): file rules: missing is zero passkeys; a regular `0600` file owned by the
/// uid is read; group/world bits, a foreign owner, a symlink (never followed), a directory,
/// a FIFO (never blocks), an empty or oversize file are refused.
#[test]
fn test_rmc_credential_store_file_rules() {
    let dir = tempfile::tempdir().unwrap();
    let path = store_path(dir.path());
    let uid = own_uid();
    let all = keys();

    let mut store = CredentialStore::new(path.clone(), uid);
    assert!(matches!(store.load(), Ok(None)), "missing = zero passkeys");
    assert_eq!(store.live_credential_hashes(), Some(Vec::new()));

    write_store(&path, &all[..2], 0o600);
    let mut store = CredentialStore::new(path.clone(), uid);
    match store.load() {
        Ok(Some(file)) => assert!(*file == file_of(&all[..2])),
        other => panic!("valid store: {:?}", other.err()),
    }
    assert_eq!(
        store.live_credential_hashes(),
        Some(vec![
            sha256(&all[0].credential_id),
            sha256(&all[1].credential_id)
        ])
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(
        CredentialStore::new(path.clone(), uid).load().is_ok(),
        "0400 is fine"
    );

    for mode in [0o644, 0o640, 0o604, 0o660, 0o606] {
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(
            CredentialStore::new(path.clone(), uid).load().err(),
            Some(StoreError::Insecure),
            "{mode:o}"
        );
        assert_eq!(
            CredentialStore::new(path.clone(), uid).live_credential_hashes(),
            None
        );
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        CredentialStore::new(path.clone(), uid.wrapping_add(1))
            .load()
            .err(),
        Some(StoreError::Insecure),
        "a file not owned by the service uid"
    );

    // Symlink: never followed.
    let target = dir.path().join("real.json");
    write_store(&target, &all[..1], 0o600);
    let link = dir.path().join("link.json");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let result = CredentialStore::new(link.clone(), uid)
        .load()
        .map(|f| f.is_some());
    assert!(
        matches!(result, Err(StoreError::Insecure | StoreError::Io)),
        "{result:?}"
    );

    // Directory, FIFO, empty, oversize.
    let as_dir = dir.path().join("dir.json");
    fs::create_dir(&as_dir).unwrap();
    assert!(CredentialStore::new(as_dir, uid).load().is_err());
    let fifo = dir.path().join("fifo.json");
    nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    let fifo_result = bounded(move || CredentialStore::new(fifo, uid).load().map(|f| f.is_some()));
    assert!(fifo_result.is_err(), "a FIFO is refused without blocking");
    let empty = dir.path().join("empty.json");
    write_file_mode(&empty, b"", 0o600);
    assert_eq!(
        CredentialStore::new(empty, uid).load().err(),
        Some(StoreError::Malformed),
        "an empty file is never zero passkeys"
    );
    let oversize = dir.path().join("big.json");
    write_file_mode(
        &oversize,
        &vec![b' '; MAX_CREDENTIAL_STORE_BYTES + 1],
        0o600,
    );
    assert_eq!(
        CredentialStore::new(oversize, uid).load().err(),
        Some(StoreError::TooLarge)
    );
}

fn temp_files(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp"))
        .collect()
}

/// Test 29 (RMC35): read-modify-write under `flock`: a new file is `0600`; `f` sees the
/// fresh file (cache ignored); a failure inside `f` leaves the old bytes and no temp file; a
/// concurrent lock holder makes it `Busy` after `STORE_LOCK_TIMEOUT_MS`; an unsafe parent
/// directory is refused; a changed file stamp makes `load` re-read.
#[test]
fn test_rmc_credential_store_atomic_update() {
    let dir = tempfile::tempdir().unwrap();
    let path = store_path(dir.path());
    let uid = own_uid();
    let all = keys();
    let random = scripted_random();
    let mut store = CredentialStore::new(path.clone(), uid);

    // Creation.
    let created = store.update(&random, |old| {
        assert!(old.is_none(), "no file yet");
        Ok((file_of(&all[..1]), 7u32))
    });
    assert_eq!(created, Ok(7));
    let meta = fs::metadata(&path).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    assert!(parse_store(&fs::read(&path).unwrap()) == Ok(file_of(&all[..1])));
    assert!(temp_files(dir.path()).is_empty());

    // `f` receives the current file, written meanwhile by someone else.
    write_store(&path, &all[..2], 0o600);
    let seen = store.update(&random, |old| {
        let old = old.expect("current file");
        let n = old.passkeys.len();
        Ok((old, n))
    });
    assert_eq!(seen, Ok(2), "the cache is ignored by update");

    // Failure inside `f`: old bytes kept, no temp file.
    let before = fs::read(&path).unwrap();
    let failed: Result<(), StoreError> = store.update(&random, |_| Err(StoreError::Full));
    assert_eq!(failed, Err(StoreError::Full));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(temp_files(dir.path()).is_empty());

    // A concurrent holder of the lock.
    let lock_path = PathBuf::from(format!("{}.lock", path.display()));
    let lock_file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    let held = nix::fcntl::Flock::lock(lock_file, nix::fcntl::FlockArg::LockExclusiveNonblock)
        .map_err(|(_, e)| e)
        .unwrap();
    let start = Instant::now();
    let busy: Result<(), StoreError> = store.update(&random, |old| Ok((old.unwrap(), ())));
    let waited = start.elapsed();
    assert_eq!(busy, Err(StoreError::Busy));
    assert!(
        waited >= Duration::from_millis(STORE_LOCK_TIMEOUT_MS - 25)
            && waited < Duration::from_secs(5),
        "bounded lock wait: {waited:?}"
    );
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "nothing written while busy"
    );
    drop(held);
    assert_eq!(store.update(&random, |old| Ok((old.unwrap(), ()))), Ok(()));

    // Stamp change ⇒ reload.
    let mut reader = CredentialStore::new(path.clone(), uid);
    assert_eq!(
        reader.load().ok().flatten().map(|f| f.passkeys.len()),
        Some(2)
    );
    write_store(&path, &all[..3], 0o600);
    assert_eq!(
        reader.load().ok().flatten().map(|f| f.passkeys.len()),
        Some(3),
        "a new file stamp is re-read"
    );

    // Unsafe or missing parent directory.
    let open_dir = tempfile::tempdir().unwrap();
    fs::set_permissions(open_dir.path(), fs::Permissions::from_mode(0o770)).unwrap();
    let mut exposed = CredentialStore::new(store_path(open_dir.path()), uid);
    let result = exposed.update(&random, |_| Ok((file_of(&all[..1]), ())));
    assert!(
        matches!(result, Err(StoreError::Insecure)),
        "a group-writable directory: {result:?}"
    );
    assert!(!store_path(open_dir.path()).exists());
    let mut orphan = CredentialStore::new(dir.path().join("missing/remote-passkeys.json"), uid);
    assert!(orphan
        .update(&random, |_| Ok((file_of(&all[..1]), ())))
        .is_err());
    assert!(!dir.path().join("missing").exists(), "never created");
}

/// A-T2 (C12c): `<path>.lock` as a symlink is refused, never followed: the target keeps its
/// bytes and mode, and the store file is not written.
#[test]
fn test_rmc_credential_store_lock_file_is_never_followed() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let path = store_path(dir.path());
    let uid = own_uid();
    let target = elsewhere.path().join("victim");
    write_file_mode(&target, b"victim bytes", 0o640);
    let lock_path = PathBuf::from(format!("{}.lock", path.display()));
    std::os::unix::fs::symlink(&target, &lock_path).unwrap();

    let mut store = CredentialStore::new(path.clone(), uid);
    let result = store.update(&scripted_random(), |_| Ok((file_of(&keys()[..1]), ())));
    assert!(
        matches!(result, Err(StoreError::Insecure | StoreError::Io)),
        "{result:?}"
    );
    assert_eq!(fs::read(&target).unwrap(), b"victim bytes");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(!path.exists(), "the store file is not written");
    assert!(fs::symlink_metadata(&lock_path)
        .unwrap()
        .file_type()
        .is_symlink());
}

// ---------------------------------------------------------------------------------------
// Direct biometric prompt (GitHub #354 sub-issue 1, ADR 2026-10-12 "Named Credentials on
// Authenticated Passkey Ceremonies of `soos-remote`", spec
// `AI/architect_spec_remote_direct_biometric.md` §3.1, §8.3; matrix RDB7)
// ---------------------------------------------------------------------------------------

/// The six WebAuthn L3 transport names, canonical order (spec §3.1).
const TRANSPORT_NAMES: [&str; 6] = ["internal", "hybrid", "usb", "nfc", "ble", "smart-card"];

/// The set holding exactly the members of `ALL` selected by the low six bits of `mask`.
fn transport_set_of(mask: u8) -> soos_remote::credentials::TransportSet {
    use soos_remote::credentials::{AuthenticatorTransport, TransportSet};
    let mut set = TransportSet::EMPTY;
    for (i, t) in AuthenticatorTransport::ALL.iter().enumerate() {
        if mask & (1 << i) != 0 {
            set = set.with(*t);
        }
    }
    set
}

/// RDB7 (spec §3.1, §4): exact, case-sensitive names; canonical output order; the default
/// allow-list transports; every set is bounded (≤ 6 names, no duplicate, canonical order).
#[test]
fn test_rdb_transport_set_canonical_and_bounded() {
    use soos_remote::credentials::{AuthenticatorTransport, TransportSet};

    assert_eq!(AuthenticatorTransport::ALL.len(), 6);
    let names: Vec<&str> = AuthenticatorTransport::ALL
        .iter()
        .map(|t| t.as_str())
        .collect();
    assert_eq!(names, TRANSPORT_NAMES.to_vec(), "canonical order");
    for t in AuthenticatorTransport::ALL {
        assert_eq!(AuthenticatorTransport::from_name(t.as_str()), Some(t));
    }
    for bad in [
        "",
        "Internal",
        "INTERNAL",
        " internal",
        "internal ",
        "hybrid\0",
        "smartcard",
        "smart_card",
        "Smart-Card",
        "cable",
        "lightning",
        "future-x",
    ] {
        assert_eq!(AuthenticatorTransport::from_name(bad), None, "{bad:?}");
    }

    assert!(TransportSet::EMPTY.is_empty());
    assert!(TransportSet::EMPTY.names().is_empty());
    assert_eq!(TransportSet::default(), TransportSet::EMPTY);
    assert_eq!(
        TransportSet::DEFAULT_ALLOW.names(),
        vec!["internal", "hybrid"]
    );
    assert!(TransportSet::DEFAULT_ALLOW.contains(AuthenticatorTransport::Internal));
    assert!(TransportSet::DEFAULT_ALLOW.contains(AuthenticatorTransport::Hybrid));
    assert!(!TransportSet::DEFAULT_ALLOW.contains(AuthenticatorTransport::Usb));
    assert!(!TransportSet::DEFAULT_ALLOW.is_empty());

    // Insertion order never matters; insertion is idempotent.
    let set = TransportSet::EMPTY
        .with(AuthenticatorTransport::SmartCard)
        .with(AuthenticatorTransport::Usb)
        .with(AuthenticatorTransport::Internal)
        .with(AuthenticatorTransport::Usb);
    assert_eq!(set.names(), vec!["internal", "usb", "smart-card"]);
    assert_eq!(
        set,
        TransportSet::EMPTY
            .with(AuthenticatorTransport::Internal)
            .with(AuthenticatorTransport::Usb)
            .with(AuthenticatorTransport::SmartCard)
    );

    // Every one of the 64 sets: ≤ 6 names, exactly its members, canonical order.
    for mask in 0u8..64 {
        let set = transport_set_of(mask);
        let names = set.names();
        assert!(names.len() <= 6);
        assert_eq!(names.len(), mask.count_ones() as usize, "{mask:06b}");
        let expected: Vec<&str> = TRANSPORT_NAMES
            .iter()
            .enumerate()
            .filter(|(i, _)| mask & (1 << i) != 0)
            .map(|(_, n)| *n)
            .collect();
        assert_eq!(names, expected, "{mask:06b}");
        assert_eq!(set.is_empty(), mask == 0);
        for (i, t) in AuthenticatorTransport::ALL.iter().enumerate() {
            assert_eq!(set.contains(*t), mask & (1 << i) != 0);
        }
    }
}

/// RDB7 (spec §3.1, D-5, A-6): a legacy store parses with empty transports; a record with
/// transports round-trips in canonical order; an empty set is omitted and encodes byte for
/// byte as the legacy format; every malformed `transports` value is refused; the largest
/// store with all six transports stays within `MAX_CREDENTIAL_STORE_BYTES`.
#[test]
fn test_rdb_store_transports_roundtrip_and_strict_parse() {
    use soos_remote::credentials::{AuthenticatorTransport, TransportSet};

    let all = keys();
    // Legacy file (no `transports` key): every record has EMPTY.
    let passkeys: Vec<StoredPasskey> = all[..2].iter().map(StoredPasskey::of).collect();
    let legacy = store_json(&HANDLE, &passkeys);
    let parsed = parse_store(legacy.as_bytes()).expect("a legacy store stays valid");
    assert_eq!(parsed.passkeys.len(), 2);
    for record in &parsed.passkeys {
        assert_eq!(record.transports, TransportSet::EMPTY);
    }
    // An empty set is omitted: the encoding is byte-identical to the legacy format.
    let plain = file_of(&all[..2]);
    let encoded = encode_store(&plain);
    assert_eq!(
        String::from_utf8(encoded.clone()).unwrap(),
        legacy,
        "no transports key when the set is empty"
    );
    let json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert!(json["passkeys"][0].get("transports").is_none());

    // A record with transports round-trips, written in canonical order.
    let mut with = file_of(&all[..2]);
    with.passkeys[1].transports = TransportSet::EMPTY
        .with(AuthenticatorTransport::Usb)
        .with(AuthenticatorTransport::Internal);
    let encoded = encode_store(&with);
    let json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(
        json["passkeys"][1]["transports"],
        serde_json::json!(["internal", "usb"])
    );
    assert!(json["passkeys"][0].get("transports").is_none());
    assert!(parse_store(&encoded) == Ok(with.clone()), "round trip");
    // A hand-written record with transports in any valid order parses to the same set.
    let text = legacy.replacen(
        "\"created_unix_s\":1700000000}]",
        "\"created_unix_s\":1700000000,\"transports\":[\"usb\",\"internal\"]}]",
        1,
    );
    assert_ne!(text, legacy, "fixture edit applied");
    let parsed = parse_store(text.as_bytes()).expect("valid transports");
    assert_eq!(parsed.passkeys[1].transports, with.passkeys[1].transports);
    assert_eq!(parsed.passkeys[0].transports, TransportSet::EMPTY);

    // Strict read: unknown, wrong case, duplicate, 7 entries, non-array, non-string.
    let malformed: Vec<&str> = vec![
        "[\"future-x\"]",
        "[\"Internal\"]",
        "[\"usb\",\"usb\"]",
        "[\"internal\",\"hybrid\",\"usb\",\"nfc\",\"ble\",\"smart-card\",\"internal\"]",
        "\"internal\"",
        "[1]",
        "[[\"internal\"]]",
        "{\"internal\":true}",
        "7",
    ];
    for value in malformed {
        let text = legacy.replacen(
            "\"created_unix_s\":1700000000}]",
            &format!("\"created_unix_s\":1700000000,\"transports\":{value}}}]"),
            1,
        );
        assert_ne!(text, legacy);
        assert_eq!(
            parse_store(text.as_bytes()).err(),
            Some(StoreError::Malformed),
            "transports {value}"
        );
    }

    // The largest legal store, every record with all six transports, fits the bound.
    let every = transport_set_of(0b11_1111);
    assert_eq!(every.names().len(), 6);
    let big: Vec<Authenticator> = (0..MAX_PASSKEYS)
        .map(|i| {
            let mut id = vec![0xee; MAX_CREDENTIAL_ID_BYTES];
            id[0] = i as u8;
            Authenticator::device(0x60 + i as u8, &id)
        })
        .collect();
    let mut big_file = file_of(&big);
    for record in &mut big_file.passkeys {
        record.transports = every;
    }
    let big_encoded = encode_store(&big_file);
    assert!(
        big_encoded.len() <= MAX_CREDENTIAL_STORE_BYTES,
        "{} bytes",
        big_encoded.len()
    );
    assert!(parse_store(&big_encoded) == Ok(big_file));
}

/// RDB7 (spec §3.1 `allow_credentials`, D-1, D-3): one `{id, transports}` per record in store
/// order, the id in base64url, the stored transports in canonical order, and
/// `["internal", "hybrid"]` for a record without transports; an empty store is `[]`.
#[test]
fn test_rdb_allow_credentials_shape() {
    use soos_remote::credentials::{allow_credentials, AuthenticatorTransport, TransportSet};

    let all = keys();
    let mut file = file_of(&all[..4]);
    file.passkeys[1].transports = TransportSet::EMPTY
        .with(AuthenticatorTransport::Nfc)
        .with(AuthenticatorTransport::Usb);
    file.passkeys[2].transports = transport_set_of(0b11_1111);
    file.passkeys[3].transports = TransportSet::EMPTY.with(AuthenticatorTransport::Hybrid);
    assert_eq!(
        allow_credentials(&file),
        serde_json::json!([
            {"id": all[0].credential_id_b64(), "transports": ["internal", "hybrid"]},
            {"id": all[1].credential_id_b64(), "transports": ["usb", "nfc"]},
            {"id": all[2].credential_id_b64(), "transports": TRANSPORT_NAMES},
            {"id": all[3].credential_id_b64(), "transports": ["hybrid"]},
        ])
    );
    // Every entry has exactly the two keys.
    for entry in allow_credentials(&file).as_array().unwrap() {
        let mut keys: Vec<&String> = entry.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, vec!["id", "transports"]);
    }
    assert_eq!(allow_credentials(&file_of(&[])), serde_json::json!([]));
    // A 1023-byte id is encoded whole (base64url, no padding).
    let long = Authenticator::device(0x66, &vec![0xab; MAX_CREDENTIAL_ID_BYTES]);
    let value = allow_credentials(&file_of(std::slice::from_ref(&long)));
    assert_eq!(value[0]["id"], long.credential_id_b64());
    assert_eq!(
        b64url_decode(value[0]["id"].as_str().unwrap()).len(),
        MAX_CREDENTIAL_ID_BYTES
    );
}
