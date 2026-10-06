//! Contract tests of the pure push runtime pieces of `soos-remote` (`push.rs`, `alerts.rs`
//! `is_live`; ADR 2026-10-06 "Web Push Notifications for Failed-Password Alerts Through a
//! Separate Sender Unit", architect spec `AI/architect_spec_remote_web_push.md` §3, §5,
//! tests 18–22 and 24; matrix RMC60, RMC65, RMC67, RMC69).
//!
//! Pure functions and the `0600` store in a `TempDir`; scripted randomness; no clock, no
//! network. Test 23 (subscribe body parsing) is in `push_server_tests.rs`: the spec defines
//! that parsing only through the route answers (§6.2 steps 4–7), not as a public function.

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

#[path = "common/push.rs"]
mod push;

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;

use passkey::*;
use push::*;
use soos_push_protocol::PushEndpoint;
use soos_remote::alerts::{is_live, Attempt, AttemptKind};
use soos_remote::auth::{RandomError, RandomSource};
use soos_remote::config::PushPreviews;
use soos_remote::journal::{AccountClass, SourceClass};
use soos_remote::push::{
    alert_payload, subscription_line, test_payload, PushScheduler, PushStore, PushStoreError,
    PushSubscription, PushSummary, UpsertResult,
};
use soos_remote::webpush::VapidKey;
use soos_remote::{
    MAX_PUSH_PLAINTEXT_BYTES, MAX_PUSH_STORE_BYTES, MAX_PUSH_SUBSCRIPTIONS, PUSH_COALESCE_MS,
    PUSH_MAX_ATTEMPT_AGE_MS, PUSH_MAX_PER_HOUR, PUSH_MIN_INTERVAL_MS, PUSH_STORE_FILE_NAME,
    PUSH_TEST_TOPIC, PUSH_TOPIC, PUSH_TTL_S,
};

const RP_ID: &str = "pc.tail1234.ts.net";
const HOUR_MS: u64 = 3_600_000;

fn counter_random() -> RandomSource {
    let counter = Arc::new(AtomicU64::new(1));
    Arc::new(move |buf: &mut [u8]| -> Result<(), RandomError> {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        let seed = sha256(&n.to_be_bytes());
        for (i, b) in buf.iter_mut().enumerate() {
            *b = seed[i % 32] ^ (i / 32) as u8;
        }
        Ok(())
    })
}

fn failing_random() -> RandomSource {
    Arc::new(|_: &mut [u8]| -> Result<(), RandomError> { Err(RandomError::Failed) })
}

fn attempt(class: SourceClass, account: AccountClass, kind: AttemptKind, at_us: u64) -> Attempt {
    Attempt {
        class,
        account,
        kind,
        at_us,
    }
}

fn wrong(at_us: u64) -> Attempt {
    attempt(
        SourceClass::LockScreen,
        AccountClass::Owner,
        AttemptKind::WrongPassword,
        at_us,
    )
}

/// Spec §3.1 constants used by this suite.
#[test]
fn test_rwp_push_constants_match_the_spec() {
    assert_eq!(MAX_PUSH_SUBSCRIPTIONS, 4);
    assert_eq!(MAX_PUSH_STORE_BYTES, 16_384);
    assert_eq!(PUSH_STORE_FILE_NAME, "remote-push.json");
    assert_eq!(PUSH_COALESCE_MS, 3000);
    assert_eq!(PUSH_MIN_INTERVAL_MS, 30_000);
    assert_eq!(PUSH_MAX_PER_HOUR, 20);
    assert_eq!(PUSH_MAX_ATTEMPT_AGE_MS, 300_000);
    assert_eq!(PUSH_TTL_S, 43_200);
    assert_eq!(PUSH_TOPIC, "soosalerts");
    assert_eq!(PUSH_TEST_TOPIC, "soostest");
    assert_eq!(MAX_PUSH_PLAINTEXT_BYTES, 1024);
}

// ---------------------------------------------------------------------------------------
// Test 18 — scheduler
// ---------------------------------------------------------------------------------------

/// Test 18 (RMC67, W-13, §5.2, F-10): first send `PUSH_COALESCE_MS` after the first note,
/// one summary per burst, then ≥ 30 s spacing and ≤ 20 sends per rolling hour; `newest_*`
/// follow the greatest journal time; `merge` adds saturating counts, keeps the time span
/// and takes `newest_*` from the newer operand.
#[test]
fn test_rwp_scheduler_coalesces_and_rate_limits() {
    // Coalescing.
    let mut s = PushScheduler::new();
    assert_eq!(s.due_at(), None);
    assert!(s.take(1_000_000).is_none(), "nothing pending");
    let t = 10_000;
    for i in 0..5u64 {
        s.note(wrong(1_700_000_000_000_000 + i * 500_000), t + i * 500);
    }
    assert_eq!(s.due_at(), Some(t + PUSH_COALESCE_MS));
    assert!(s.take(t + PUSH_COALESCE_MS - 1).is_none());
    let summary = s.take(t + PUSH_COALESCE_MS).expect("due");
    assert_eq!(summary.wrong_password, 5);
    assert_eq!(summary.locked_out, 0);
    assert_eq!(summary.first_unix_ms, 1_700_000_000_000);
    assert_eq!(summary.last_unix_ms, 1_700_000_002_000);
    assert_eq!(summary.newest_source, SourceClass::LockScreen);
    assert_eq!(summary.newest_account, AccountClass::Owner);
    assert_eq!(summary.newest_kind, AttemptKind::WrongPassword);
    assert_eq!(s.due_at(), None, "pending cleared");

    // Spacing after a send.
    let sent = t + PUSH_COALESCE_MS;
    s.note(wrong(1_700_000_010_000_000), sent + 1);
    assert_eq!(s.due_at(), Some(sent + PUSH_MIN_INTERVAL_MS));
    assert!(s.take(sent + PUSH_MIN_INTERVAL_MS - 1).is_none());
    assert_eq!(
        s.take(sent + PUSH_MIN_INTERVAL_MS).unwrap().wrong_password,
        1
    );
    // A note long after the last send waits only for the coalescing delay.
    let late = sent + PUSH_MIN_INTERVAL_MS + 100_000;
    s.note(wrong(1_700_000_200_000_000), late);
    assert_eq!(s.due_at(), Some(late + PUSH_COALESCE_MS));

    // At most PUSH_MAX_PER_HOUR sends per rolling hour.
    let mut s = PushScheduler::new();
    s.note(wrong(1), 0);
    let mut sends = vec![s.due_at().unwrap()];
    assert!(s.take(sends[0]).is_some());
    for _ in 1..PUSH_MAX_PER_HOUR {
        let last = *sends.last().unwrap();
        s.note(wrong(2), last + 1);
        let due = s.due_at().unwrap();
        assert_eq!(due, last + PUSH_MIN_INTERVAL_MS);
        assert!(s.take(due).is_some());
        sends.push(due);
    }
    assert_eq!(sends.len(), PUSH_MAX_PER_HOUR as usize);
    let last = *sends.last().unwrap();
    s.note(wrong(3), last + 1);
    assert_eq!(
        s.due_at(),
        Some(sends[0] + HOUR_MS),
        "the 21st send waits until the oldest leaves the window"
    );
    assert!(s.take(sends[0] + HOUR_MS - 1).is_none());
    assert!(s.take(sends[0] + HOUR_MS).is_some());
    // The window moved by one send: the next is again limited by the spacing.
    s.note(wrong(4), sends[0] + HOUR_MS + 1);
    let due = s.due_at().unwrap();
    assert_eq!(
        due,
        (sends[0] + HOUR_MS + PUSH_MIN_INTERVAL_MS).max(sends[1] + HOUR_MS)
    );

    // Counts by kind; newest_* follow the greatest at_us (ties: the later note).
    let mut s = PushScheduler::new();
    s.note(
        attempt(
            SourceClass::Sudo,
            AccountClass::Root,
            AttemptKind::WrongPassword,
            5_000_000,
        ),
        0,
    );
    s.note(
        attempt(
            SourceClass::Login,
            AccountClass::Other,
            AttemptKind::LockedOut,
            1_000_000,
        ),
        1,
    );
    s.note(
        attempt(
            SourceClass::Other,
            AccountClass::Owner,
            AttemptKind::LockedOut,
            3_000_000,
        ),
        2,
    );
    let summary = s.take(10_000).unwrap();
    assert_eq!((summary.wrong_password, summary.locked_out), (1, 2));
    assert_eq!(summary.newest_source, SourceClass::Sudo);
    assert_eq!(summary.newest_account, AccountClass::Root);
    assert_eq!(summary.newest_kind, AttemptKind::WrongPassword);
    assert_eq!(summary.first_unix_ms, 1000);
    assert_eq!(summary.last_unix_ms, 5000);
    let mut s = PushScheduler::new();
    s.note(wrong(7_000_000), 0);
    s.note(
        attempt(
            SourceClass::Sudo,
            AccountClass::Root,
            AttemptKind::LockedOut,
            7_000_000,
        ),
        1,
    );
    let summary = s.take(10_000).unwrap();
    assert_eq!(summary.newest_source, SourceClass::Sudo, "tie: later note");
    assert_eq!(summary.newest_kind, AttemptKind::LockedOut);

    // merge.
    let older = PushSummary {
        wrong_password: 3,
        locked_out: 1,
        newest_source: SourceClass::LockScreen,
        newest_account: AccountClass::Owner,
        newest_kind: AttemptKind::WrongPassword,
        first_unix_ms: 1000,
        last_unix_ms: 9000,
    };
    let newer = PushSummary {
        wrong_password: 2,
        locked_out: 0,
        newest_source: SourceClass::Sudo,
        newest_account: AccountClass::Root,
        newest_kind: AttemptKind::WrongPassword,
        first_unix_ms: 5000,
        last_unix_ms: 12_000,
    };
    let merged = PushSummary::merge(&older, &newer);
    assert_eq!(
        merged,
        PushSummary {
            wrong_password: 5,
            locked_out: 1,
            newest_source: SourceClass::Sudo,
            newest_account: AccountClass::Root,
            newest_kind: AttemptKind::WrongPassword,
            first_unix_ms: 1000,
            last_unix_ms: 12_000,
        }
    );
    // newest_* from the operand with the greater last_unix_ms, whatever the argument order.
    let late_older = PushSummary {
        last_unix_ms: 20_000,
        ..older
    };
    let merged = PushSummary::merge(&late_older, &newer);
    assert_eq!(merged.newest_source, SourceClass::LockScreen);
    assert_eq!(merged.newest_account, AccountClass::Owner);
    assert_eq!(merged.last_unix_ms, 20_000);
    assert_eq!(merged.first_unix_ms, 1000);
    // Ties → newer.
    let tie = PushSummary {
        last_unix_ms: 12_000,
        ..older
    };
    assert_eq!(
        PushSummary::merge(&tie, &newer).newest_source,
        SourceClass::Sudo
    );
    // Saturation.
    let full = PushSummary {
        wrong_password: u32::MAX,
        locked_out: u32::MAX - 1,
        ..older
    };
    let merged = PushSummary::merge(&full, &newer);
    assert_eq!(merged.wrong_password, u32::MAX);
    assert_eq!(merged.locked_out, u32::MAX - 1);
    let merged = PushSummary::merge(&full, &full);
    assert_eq!(
        (merged.wrong_password, merged.locked_out),
        (u32::MAX, u32::MAX)
    );
}

// ---------------------------------------------------------------------------------------
// Test 19 — liveness
// ---------------------------------------------------------------------------------------

/// Test 19 (RMC67, §5.3): only attempts at or after the start and at most 300 s old when
/// recorded are pushed; a journal time ahead of now is live.
#[test]
fn test_rwp_is_live_rule() {
    let started = 1_700_000_000_000_000;
    let age = PUSH_MAX_ATTEMPT_AGE_MS * 1000;
    assert!(!is_live(started - 1, started, started));
    assert!(!is_live(0, started, started));
    assert!(is_live(started, started, started));
    assert!(
        is_live(started, started, started + age),
        "exactly 300 s old"
    );
    assert!(!is_live(started, started, started + age + 1));
    assert!(is_live(started + 5, started, started), "ahead of now");
    assert!(is_live(started + 10, started, started + 10 + age));
    assert!(!is_live(started + 10, started, started + 11 + age));
    assert!(is_live(u64::MAX, 0, 0));
    assert!(is_live(0, 0, age));
}

// ---------------------------------------------------------------------------------------
// Test 20 — payload
// ---------------------------------------------------------------------------------------

fn summary(
    wrong_password: u32,
    locked_out: u32,
    source: SourceClass,
    account: AccountClass,
    kind: AttemptKind,
) -> PushSummary {
    PushSummary {
        wrong_password,
        locked_out,
        newest_source: source,
        newest_account: account,
        newest_kind: kind,
        first_unix_ms: 1_759_700_000_000,
        last_unix_ms: 1_759_700_004_321,
    }
}

fn payload_text(summary: &PushSummary, previews: PushPreviews) -> String {
    String::from_utf8(alert_payload(summary, previews, RP_ID).unwrap()).unwrap()
}

fn alerts_json(title: &str, body: &str, w: u32, m: u32, source: &str, account: &str) -> String {
    format!(
        "{{\"web_push\":8030,\"notification\":{{\"title\":\"{title}\",\"body\":\"{body}\",\"navigate\":\"https://pc.tail1234.ts.net/\",\"lang\":\"en\"}},\"soos\":{{\"v\":1,\"kind\":\"alerts\",\"wrong_password\":{w},\"locked_out\":{m},\"source\":{source},\"account\":{account},\"last_unix_ms\":1759700004321}}}}"
    )
}

/// Test 20 (RMC69, W-11, W-12, §5.4, O-2): exact Declarative Web Push bytes for every text
/// case, built from enums and counts only; generic previews hide source and account; the
/// test payload; `u32::MAX` counts stay within 1 KiB.
#[test]
fn test_rwp_alert_payload_exact() {
    let title = "Failed password on your PC";
    let lock_owner = |w, m| {
        summary(
            w,
            m,
            SourceClass::LockScreen,
            AccountClass::Owner,
            AttemptKind::WrongPassword,
        )
    };
    assert_eq!(
        payload_text(&lock_owner(1, 0), PushPreviews::Detailed),
        alerts_json(
            title,
            "1 wrong password \u{2014} lock screen, your account",
            1,
            0,
            "\"lock_screen\"",
            "\"owner\""
        )
    );
    assert_eq!(
        payload_text(&lock_owner(3, 0), PushPreviews::Detailed),
        alerts_json(
            title,
            "3 wrong passwords \u{2014} lock screen, your account",
            3,
            0,
            "\"lock_screen\"",
            "\"owner\""
        )
    );
    let sudo_root = summary(
        2,
        1,
        SourceClass::Sudo,
        AccountClass::Root,
        AttemptKind::LockedOut,
    );
    assert_eq!(
        payload_text(&sudo_root, PushPreviews::Detailed),
        alerts_json(
            title,
            "2 wrong passwords \u{2014} sudo, root and 1 attempt while locked out",
            2,
            1,
            "\"sudo\"",
            "\"root\""
        )
    );
    let login_other = summary(
        1,
        2,
        SourceClass::Login,
        AccountClass::Other,
        AttemptKind::LockedOut,
    );
    assert_eq!(
        payload_text(&login_other, PushPreviews::Detailed),
        alerts_json(
            title,
            "1 wrong password \u{2014} login, another account and 2 attempts while locked out",
            1,
            2,
            "\"login\"",
            "\"other\""
        )
    );
    let locked_only = summary(
        0,
        1,
        SourceClass::Other,
        AccountClass::Owner,
        AttemptKind::LockedOut,
    );
    assert_eq!(
        payload_text(&locked_only, PushPreviews::Detailed),
        alerts_json(
            title,
            "1 attempt while locked out \u{2014} other, your account",
            0,
            1,
            "\"other\"",
            "\"owner\""
        )
    );
    let locked_two = summary(
        0,
        2,
        SourceClass::LockScreen,
        AccountClass::Root,
        AttemptKind::LockedOut,
    );
    assert_eq!(
        payload_text(&locked_two, PushPreviews::Detailed),
        alerts_json(
            title,
            "2 attempts while locked out \u{2014} lock screen, root",
            0,
            2,
            "\"lock_screen\"",
            "\"root\""
        )
    );
    // Generic previews: fixed text, no source, no account, counts kept.
    assert_eq!(
        payload_text(&sudo_root, PushPreviews::Generic),
        alerts_json(
            "Security alert on your PC",
            "Open soos for details",
            2,
            1,
            "null",
            "null"
        )
    );
    // The test notification.
    assert_eq!(
        String::from_utf8(test_payload(RP_ID).unwrap()).unwrap(),
        "{\"web_push\":8030,\"notification\":{\"title\":\"soos test notification\",\"body\":\"Notifications from your PC work\",\"navigate\":\"https://pc.tail1234.ts.net/\",\"lang\":\"en\"},\"soos\":{\"v\":1,\"kind\":\"test\",\"wrong_password\":0,\"locked_out\":0,\"source\":null,\"account\":null,\"last_unix_ms\":null}}"
    );
    // navigate follows rp_id.
    let other = String::from_utf8(
        alert_payload(
            &lock_owner(1, 0),
            PushPreviews::Detailed,
            "box.tail9.ts.net",
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        other.contains("\"navigate\":\"https://box.tail9.ts.net/\""),
        "{other}"
    );
    // Saturated counts stay in bounds and parse as Declarative Web Push.
    for previews in [PushPreviews::Detailed, PushPreviews::Generic] {
        let max = summary(
            u32::MAX,
            u32::MAX,
            SourceClass::LockScreen,
            AccountClass::Other,
            AttemptKind::LockedOut,
        );
        let bytes = alert_payload(&max, previews, RP_ID).unwrap();
        assert!(bytes.len() <= MAX_PUSH_PLAINTEXT_BYTES, "{}", bytes.len());
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["web_push"], 8030);
        assert!(!json["notification"]["title"].as_str().unwrap().is_empty());
        assert_eq!(json["soos"]["wrong_password"], u64::from(u32::MAX));
    }
    assert_eq!(PushPreviews::default(), PushPreviews::Detailed);
}

// ---------------------------------------------------------------------------------------
// Store fixtures
// ---------------------------------------------------------------------------------------

struct Sub {
    endpoint: String,
    ua: UaFixture,
    created: u64,
}

fn sub(endpoint: String, seed: u8, created: u64) -> Sub {
    Sub {
        endpoint,
        ua: UaFixture::new(seed),
        created,
    }
}

const PRIVATE_B64: &str = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw";

/// The exact on-disk JSON of spec §5.1.
fn store_text(private_b64: &str, subs: &[Sub]) -> String {
    let entries: Vec<String> = subs
        .iter()
        .map(|s| {
            format!(
                "{{\"endpoint\":\"{}\",\"p256dh\":\"{}\",\"auth\":\"{}\",\"created_unix_s\":{}}}",
                s.endpoint,
                s.ua.p256dh_b64(),
                s.ua.auth_b64(),
                s.created
            )
        })
        .collect();
    format!(
        "{{\"version\":1,\"vapid_private_key\":\"{private_b64}\",\"subscriptions\":[{}]}}",
        entries.join(",")
    )
}

fn store_path(dir: &Path) -> PathBuf {
    dir.join(PUSH_STORE_FILE_NAME)
}

/// Bytes, inode and mtime: a file the code must never rewrite.
fn snapshot(path: &Path) -> (Vec<u8>, u64, std::time::SystemTime, u32) {
    let meta = fs::symlink_metadata(path).unwrap();
    let bytes = if meta.file_type().is_symlink() {
        fs::read_link(path)
            .unwrap()
            .to_string_lossy()
            .into_owned()
            .into_bytes()
    } else {
        fs::read(path).unwrap()
    };
    (bytes, meta.ino(), meta.modified().unwrap(), meta.mode())
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

fn new_sub(endpoint: &str, seed: u8, created: u64) -> PushSubscription {
    PushSubscription {
        endpoint: PushEndpoint::parse(endpoint).unwrap(),
        keys: UaFixture::new(seed).ua_keys(),
        created_unix_s: created,
    }
}

fn load_err(store: &PushStore) -> PushStoreError {
    match store.load() {
        Ok(_) => panic!("load must fail"),
        Err(e) => e,
    }
}

fn load_or_create_err(store: &PushStore, random: &RandomSource) -> PushStoreError {
    match store.load_or_create(random) {
        Ok(_) => panic!("load_or_create must fail"),
        Err(e) => e,
    }
}

// ---------------------------------------------------------------------------------------
// Test 21 — store fail-closed
// ---------------------------------------------------------------------------------------

/// Test 21 (RMC65, W-8, §3.3, §5.1, F-7): an absent store is created `0600` with a fresh key
/// and no subscription; every invalid or insecure file is refused and never rewritten; a
/// store deleted while running is never recreated by `upsert`/`remove_*`; only `reset`
/// replaces it (any owned regular file, never a symlink or a foreign file).
#[test]
fn test_rwp_store_round_trip_and_fail_closed() {
    let uid = own_uid();
    let random = counter_random();

    // Creation.
    let dir = tempfile::tempdir().unwrap();
    let path = store_path(dir.path());
    let store = PushStore::new(path.clone(), uid);
    assert!(store.load().unwrap().is_none(), "absent → Ok(None)");
    assert!(!path.exists());
    let created = store.load_or_create(&random).unwrap();
    assert_eq!(created.subscriptions.len(), 0);
    assert_eq!(mode_of(&path), 0o600);
    let json: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["subscriptions"], serde_json::json!([]));
    let private = json["vapid_private_key"].as_str().unwrap();
    assert_eq!(private.len(), 43);
    let key = VapidKey::from_bytes(&b64url_decode(private).try_into().unwrap()).unwrap();
    assert_eq!(key.public_key_b64(), created.key.public_key_b64());
    let keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys.len(), 3, "{json}");
    // Reload equal; load_or_create never writes over a present file.
    let before = snapshot(&path);
    let reloaded = store.load().unwrap().unwrap();
    assert_eq!(reloaded.key.public_key_b64(), created.key.public_key_b64());
    let again = store.load_or_create(&random).unwrap();
    assert_eq!(again.key.public_key_b64(), created.key.public_key_b64());
    assert_eq!(snapshot(&path), before);

    // A valid store written by hand loads.
    let valid = store_text(PRIVATE_B64, &[sub(apple_endpoint(1), 1, 1_759_700_000)]);
    let dir = tempfile::tempdir().unwrap();
    let path = store_path(dir.path());
    write_file_mode(&path, valid.as_bytes(), 0o600);
    let loaded = PushStore::new(path.clone(), uid).load().unwrap().unwrap();
    assert_eq!(loaded.subscriptions.len(), 1);
    assert_eq!(loaded.subscriptions[0].endpoint.as_str(), apple_endpoint(1));
    assert_eq!(loaded.subscriptions[0].created_unix_s, 1_759_700_000);

    // Refused files, never rewritten (load and load_or_create alike).
    let five: Vec<Sub> = (1..=5)
        .map(|i| sub(apple_endpoint(i), i as u8, 1))
        .collect();
    let dup = [sub(apple_endpoint(1), 1, 1), sub(apple_endpoint(1), 2, 2)];
    let bad_endpoint = [sub("https://evil.example/x".to_string(), 1, 1)];
    let unknown_key = valid.replacen("{\"version\":1,", "{\"version\":1,\"extra\":true,", 1);
    let version2 = valid.replacen("{\"version\":1,", "{\"version\":2,", 1);
    let bad_keys = valid.replace(&UaFixture::new(1).auth_b64(), "AAAA");
    let zero_key = store_text(&b64url(&[0u8; 32]), &[]);
    let short_key = store_text("AAAA", &[]);
    let mut big = valid.clone().into_bytes();
    big.resize(MAX_PUSH_STORE_BYTES + 1, b' ');
    let malformed: Vec<(&str, Vec<u8>)> = vec![
        ("invalid JSON", b"{\"version\":1,".to_vec()),
        ("unknown key", unknown_key.into_bytes()),
        ("version 2", version2.into_bytes()),
        (
            "5 subscriptions",
            store_text(PRIVATE_B64, &five).into_bytes(),
        ),
        (
            "duplicate endpoint",
            store_text(PRIVATE_B64, &dup).into_bytes(),
        ),
        (
            "invalid endpoint",
            store_text(PRIVATE_B64, &bad_endpoint).into_bytes(),
        ),
        ("invalid keys", bad_keys.into_bytes()),
        ("zero private key", zero_key.into_bytes()),
        ("short private key", short_key.into_bytes()),
    ];
    for (what, bytes) in malformed {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(dir.path());
        write_file_mode(&path, &bytes, 0o600);
        let before = snapshot(&path);
        let store = PushStore::new(path.clone(), uid);
        assert_eq!(load_err(&store), PushStoreError::Malformed, "{what}");
        assert_eq!(
            load_or_create_err(&store, &random),
            PushStoreError::Malformed,
            "{what}"
        );
        assert_eq!(snapshot(&path), before, "{what}: untouched");
    }
    // Oversized (refused before parsing, whatever the class).
    {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(dir.path());
        write_file_mode(&path, &big, 0o600);
        let before = snapshot(&path);
        let store = PushStore::new(path.clone(), uid);
        assert!(matches!(
            load_err(&store),
            PushStoreError::Refused | PushStoreError::Malformed
        ));
        assert!(store.load_or_create(&random).is_err());
        assert_eq!(snapshot(&path), before);
    }
    // Insecure: mode 0644, a foreign owner, a symlink to a valid store, a directory.
    {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(dir.path());
        write_file_mode(&path, valid.as_bytes(), 0o644);
        let before = snapshot(&path);
        let store = PushStore::new(path.clone(), uid);
        assert_eq!(load_err(&store), PushStoreError::Refused, "mode 0644");
        assert_eq!(load_or_create_err(&store, &random), PushStoreError::Refused);
        assert_eq!(snapshot(&path), before);

        write_file_mode(&path, valid.as_bytes(), 0o600);
        let before = snapshot(&path);
        let foreign = PushStore::new(path.clone(), uid.wrapping_add(1));
        assert_eq!(load_err(&foreign), PushStoreError::Refused, "foreign owner");
        assert_eq!(
            load_or_create_err(&foreign, &random),
            PushStoreError::Refused
        );
        assert_eq!(snapshot(&path), before);

        let target = dir.path().join("real.json");
        write_file_mode(&target, valid.as_bytes(), 0o600);
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        let before = snapshot(&path);
        let target_before = snapshot(&target);
        assert_eq!(load_err(&store), PushStoreError::Refused, "symlink");
        assert_eq!(load_or_create_err(&store, &random), PushStoreError::Refused);
        assert_eq!(snapshot(&path), before);
        assert_eq!(snapshot(&target), target_before);

        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert_eq!(load_err(&store), PushStoreError::Refused, "directory");
    }
    // RNG failure on an absent file: no file.
    {
        let dir = tempfile::tempdir().unwrap();
        let path = store_path(dir.path());
        let store = PushStore::new(path.clone(), uid);
        assert_eq!(
            load_or_create_err(&store, &failing_random()),
            PushStoreError::Random
        );
        assert!(fs::symlink_metadata(&path).is_err(), "nothing created");
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            0,
            "no temporary file left"
        );
    }

    // F-7: deleted while running → never recreated except by reset.
    let dir = tempfile::tempdir().unwrap();
    let path = store_path(dir.path());
    let store = PushStore::new(path.clone(), uid);
    let first = store.load_or_create(&random).unwrap();
    store
        .upsert(new_sub(&apple_endpoint(1), 1, 100), &random)
        .unwrap();
    fs::remove_file(&path).unwrap();
    assert!(store.load().unwrap().is_none());
    assert_eq!(
        store
            .upsert(new_sub(&apple_endpoint(2), 2, 200), &random)
            .map(|_| ()),
        Err(PushStoreError::Missing)
    );
    assert_eq!(
        store.remove_endpoint(&PushEndpoint::parse(&apple_endpoint(1)).unwrap(), &random),
        Err(PushStoreError::Missing)
    );
    assert_eq!(store.remove_index(1, &random), Err(PushStoreError::Missing));
    assert!(fs::symlink_metadata(&path).is_err(), "no file was created");
    let reset = store.reset(&random).unwrap();
    assert_eq!(reset.subscriptions.len(), 0);
    assert_ne!(
        reset.key.public_key_b64(),
        first.key.public_key_b64(),
        "a different key"
    );
    assert_eq!(mode_of(&path), 0o600);
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded.key.public_key_b64(), reset.key.public_key_b64());
    // reset over an existing valid store also replaces the key and drops subscriptions.
    store
        .upsert(new_sub(&apple_endpoint(3), 3, 300), &random)
        .unwrap();
    let reset2 = store.reset(&random).unwrap();
    assert_ne!(reset2.key.public_key_b64(), reset.key.public_key_b64());
    assert_eq!(store.load().unwrap().unwrap().subscriptions.len(), 0);

    // reset over an invalid file owned by the uid (0644, bad JSON) → replaced.
    let dir = tempfile::tempdir().unwrap();
    let path = store_path(dir.path());
    write_file_mode(&path, b"{not json", 0o644);
    let store = PushStore::new(path.clone(), uid);
    let replaced = store.reset(&random).unwrap();
    assert_eq!(mode_of(&path), 0o600);
    assert_eq!(
        store.load().unwrap().unwrap().key.public_key_b64(),
        replaced.key.public_key_b64()
    );
    // reset with a failing random source: old bytes unchanged.
    let before = snapshot(&path);
    assert_eq!(
        store.reset(&failing_random()).map(|_| ()),
        Err(PushStoreError::Random)
    );
    assert_eq!(snapshot(&path), before);
    // reset with a symlink or a foreign owner → Refused, untouched.
    let target = dir.path().join("target.json");
    write_file_mode(&target, b"keep", 0o600);
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let before = snapshot(&path);
    assert_eq!(
        store.reset(&random).map(|_| ()),
        Err(PushStoreError::Refused)
    );
    assert_eq!(snapshot(&path), before);
    assert_eq!(fs::read(&target).unwrap(), b"keep");
    fs::remove_file(&path).unwrap();
    write_file_mode(&path, b"{}", 0o600);
    let before = snapshot(&path);
    let foreign = PushStore::new(path.clone(), uid.wrapping_add(1));
    assert_eq!(
        foreign.reset(&random).map(|_| ()),
        Err(PushStoreError::Refused)
    );
    assert_eq!(snapshot(&path), before);
}

// ---------------------------------------------------------------------------------------
// Test 22 — subscription operations
// ---------------------------------------------------------------------------------------

/// Test 22 (RMC65, §5.1): `Added`, `Replaced` (new keys, creation time kept), `TooMany` at
/// the 5th distinct endpoint (file unchanged), removal by endpoint and by 1-based index.
#[test]
fn test_rwp_store_subscription_operations() {
    let uid = own_uid();
    let random = counter_random();
    let dir = tempfile::tempdir().unwrap();
    let path = store_path(dir.path());
    let store = PushStore::new(path.clone(), uid);
    store.load_or_create(&random).unwrap();

    assert_eq!(
        store
            .upsert(new_sub(&apple_endpoint(1), 1, 100), &random)
            .unwrap(),
        UpsertResult::Added
    );
    assert_eq!(
        store
            .upsert(new_sub(&apple_endpoint(1), 2, 999), &random)
            .unwrap(),
        UpsertResult::Replaced
    );
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded.subscriptions.len(), 1);
    assert_eq!(loaded.subscriptions[0].created_unix_s, 100, "creation kept");
    assert!(
        loaded.subscriptions[0].keys.p256dh == UaFixture::new(2).ua_keys().p256dh,
        "keys replaced"
    );
    assert_eq!(*loaded.subscriptions[0].keys.auth, UaFixture::new(2).auth);
    assert_eq!(mode_of(&path), 0o600);

    for (i, endpoint) in [mozilla_endpoint(2), fcm_endpoint(3), apple_endpoint(4)]
        .iter()
        .enumerate()
    {
        assert_eq!(
            store
                .upsert(new_sub(endpoint, 10 + i as u8, 200 + i as u64), &random)
                .unwrap(),
            UpsertResult::Added
        );
    }
    assert_eq!(
        store.load().unwrap().unwrap().subscriptions.len(),
        MAX_PUSH_SUBSCRIPTIONS
    );
    let before = fs::read(&path).unwrap();
    assert_eq!(
        store
            .upsert(new_sub(&apple_endpoint(5), 20, 500), &random)
            .map(|_| ()),
        Err(PushStoreError::TooMany)
    );
    assert_eq!(fs::read(&path).unwrap(), before, "store unchanged");
    // An existing endpoint is still replaceable when full.
    assert_eq!(
        store
            .upsert(new_sub(&fcm_endpoint(3), 21, 1), &random)
            .unwrap(),
        UpsertResult::Replaced
    );

    // Removal by endpoint.
    let mozilla = PushEndpoint::parse(&mozilla_endpoint(2)).unwrap();
    assert_eq!(store.remove_endpoint(&mozilla, &random), Ok(true));
    assert_eq!(store.remove_endpoint(&mozilla, &random), Ok(false));
    let order: Vec<String> = store
        .load()
        .unwrap()
        .unwrap()
        .subscriptions
        .iter()
        .map(|s| s.endpoint.as_str().to_string())
        .collect();
    assert_eq!(
        order,
        vec![apple_endpoint(1), fcm_endpoint(3), apple_endpoint(4)],
        "store order kept"
    );
    // Removal by 1-based index.
    assert_eq!(store.remove_index(0, &random), Ok(false));
    assert_eq!(store.remove_index(4, &random), Ok(false));
    assert_eq!(store.remove_index(2, &random), Ok(true));
    let order: Vec<String> = store
        .load()
        .unwrap()
        .unwrap()
        .subscriptions
        .iter()
        .map(|s| s.endpoint.as_str().to_string())
        .collect();
    assert_eq!(order, vec![apple_endpoint(1), apple_endpoint(4)]);
    assert_eq!(store.remove_index(1, &random), Ok(true));
    assert_eq!(store.remove_index(1, &random), Ok(true));
    assert_eq!(store.remove_index(1, &random), Ok(false));
    assert_eq!(mode_of(&path), 0o600);
    // Error texts are fixed.
    for e in [
        PushStoreError::Refused,
        PushStoreError::Malformed,
        PushStoreError::Io,
        PushStoreError::Locked,
        PushStoreError::Random,
        PushStoreError::TooMany,
        PushStoreError::Missing,
    ] {
        assert!(!e.to_string().is_empty());
    }
}

// ---------------------------------------------------------------------------------------
// Test 24 — CLI lines
// ---------------------------------------------------------------------------------------

/// Test 24 (RMC69, F-9, O-2): `push list` lines are `<index>  <host>  created <unix_s>` and
/// never carry the endpoint path or a key.
#[test]
fn test_rwp_cli_lines_never_print_secrets() {
    let ua = UaFixture::new(7);
    let endpoint = apple_endpoint(77);
    let s = PushSubscription {
        endpoint: PushEndpoint::parse(&endpoint).unwrap(),
        keys: ua.ua_keys(),
        created_unix_s: 1_759_700_000,
    };
    let line = subscription_line(1, &s);
    assert_eq!(line, "1  web.push.apple.com  created 1759700000");
    for needle in [
        endpoint_token(&endpoint),
        ua.p256dh_b64(),
        ua.auth_b64(),
        "tok77".to_string(),
    ] {
        assert!(!line.contains(&needle), "{line}");
    }
    let s = PushSubscription {
        endpoint: PushEndpoint::parse(&mozilla_endpoint(3)).unwrap(),
        keys: ua.ua_keys(),
        created_unix_s: 0,
    };
    assert_eq!(
        subscription_line(4, &s),
        "4  updates.push.services.mozilla.com  created 0"
    );
}

/// Regression (owner hardware, 2026-10-06): `web.push.apple.com` answered
/// `400 {"reason":"BadWebPushTopic"}` to `Topic: soos-test` and `201` to `Topic: soostest`, so
/// every topic must use ASCII letters and digits only.
#[test]
fn test_rwp_topics_are_ascii_alphanumeric_for_apple() {
    for topic in [PUSH_TOPIC, PUSH_TEST_TOPIC] {
        assert!(!topic.is_empty(), "{topic:?}");
        assert!(
            topic.bytes().all(|b| b.is_ascii_alphanumeric()),
            "{topic:?} must use ASCII letters and digits only (Apple BadWebPushTopic)"
        );
    }
}
