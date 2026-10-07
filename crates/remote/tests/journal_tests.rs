//! Contract tests of the journal reading layer of the failed-password alerts (ADR
//! 2026-10-06 "Failed-Password Alerts in `soos-remote` From the System Journal", architect
//! spec `AI/architect_spec_remote_auth_alerts.md` §2, tests 1–12 and 51; matrix RMC46–RMC48,
//! RMC52).
//!
//! Pure: exact `journalctl` argv, cursor bounds, bounded JSON-line parsing (string and
//! byte-array shapes, duplicate keys, bounds), trust classification on journald-set fields,
//! the three signal grammars, account mapping to three classes, `_EXE` normalisation. No
//! journal, no process, no network.

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

#[path = "common/journal.rs"]
mod journal;

use proptest::prelude::*;
use serde_json::{json, Value};

use journal::*;
use soos_remote::journal::{
    classify_entry, exe_for_comparison, follow_args, parse_entry, probe_args, AccountClass,
    EntryError, FollowStart, HelperSide, JournalCursor, JournalEntry, OwnerLogin, Signal,
    SourceClass, TrustContext, EXE_DELETED_SUFFIX, JOURNALCTL_PATH,
};
use soos_remote::{MAX_CURSOR_LEN, MAX_FIELD_BYTES, MAX_JOURNAL_LINE_BYTES, MAX_MESSAGE_BYTES};

const T: u64 = 1_699_990_000_000_000;

fn ctx(lock_screen_programs: &[&str]) -> TrustContext {
    TrustContext {
        owner_uid: OWNER_UID,
        owner_login: OwnerLogin::parse(OWNER).expect("valid owner login"),
        lock_screen_programs: lock_screen_programs.iter().map(|s| s.to_string()).collect(),
    }
}

fn parsed(line: &JLine) -> JournalEntry {
    match parse_entry(&line.bytes()) {
        Ok(entry) => entry,
        Err(e) => panic!(
            "line must parse, got {e:?}: {}",
            String::from_utf8_lossy(&line.bytes())
        ),
    }
}

fn parse_err(line: &JLine) -> EntryError {
    match parse_entry(&line.bytes()) {
        Ok(_) => panic!(
            "line must be refused: {}",
            String::from_utf8_lossy(&line.bytes())
        ),
        Err(e) => e,
    }
}

fn classify(line: &JLine, ctx: &TrustContext) -> Option<Signal> {
    classify_entry(&parsed(line), ctx)
}

fn check(side: HelperSide, account: AccountClass, at_us: u64) -> Option<Signal> {
    Some(Signal::Check {
        side,
        account,
        at_us,
    })
}

fn failure(
    class: SourceClass,
    side: HelperSide,
    account: AccountClass,
    at_us: u64,
    trusted: bool,
) -> Option<Signal> {
    Some(Signal::Failure {
        class,
        side,
        account,
        at_us,
        trusted,
    })
}

// ---------------------------------------------------------------------------------------
// Test 1 — argv
// ---------------------------------------------------------------------------------------

/// Test 1 (RMC52, A-1, §2.1): the probe and follower argv, element by element; the match
/// comes last; no shell-like or journal-widening option anywhere.
#[test]
fn test_rmc_alerts_probe_and_follow_args_are_exact() {
    assert_eq!(JOURNALCTL_PATH, "/usr/bin/journalctl");
    assert_eq!(
        probe_args(),
        vec![
            "--no-pager",
            "--quiet",
            "--lines=1",
            "--output=json",
            "--output-fields=_UID",
            "_UID=0",
        ]
    );
    let fields =
        "--output-fields=MESSAGE,SYSLOG_IDENTIFIER,SYSLOG_FACILITY,_UID,_COMM,_EXE,_TRANSPORT";
    assert_eq!(
        follow_args(&FollowStart::Since { unix_s: 1 }),
        vec![
            "--follow",
            "--no-pager",
            "--quiet",
            "--lines=all",
            "--output=json",
            fields,
            "--since=@1",
            "SYSLOG_FACILITY=10",
        ]
    );
    assert_eq!(
        follow_args(&FollowStart::Since {
            unix_s: 1_699_913_600
        })[6],
        "--since=@1699913600"
    );
    let cursor = JournalCursor::parse("s=abc;i=1f;b=0123;m=42;t=5;x=9").unwrap();
    assert_eq!(
        follow_args(&FollowStart::AfterCursor(cursor)),
        vec![
            "--follow",
            "--no-pager",
            "--quiet",
            "--lines=all",
            "--output=json",
            fields,
            "--after-cursor=s=abc;i=1f;b=0123;m=42;t=5;x=9",
            "SYSLOG_FACILITY=10",
        ]
    );
    let all: Vec<Vec<String>> = vec![
        probe_args(),
        follow_args(&FollowStart::Since { unix_s: 0 }),
        follow_args(&FollowStart::AfterCursor(
            JournalCursor::parse("s=1").unwrap(),
        )),
    ];
    for args in &all {
        for arg in args {
            for forbidden in [
                "--user",
                "-g",
                "--grep",
                "--all",
                "-a",
                "--merge",
                "--directory",
                "--file",
                "--root",
                "--cursor-file",
                "-c",
                "sh",
            ] {
                assert!(
                    arg != forbidden && !arg.starts_with(&format!("{forbidden}=")),
                    "forbidden argument {arg:?}"
                );
            }
        }
    }
    // The match is always the last element.
    for args in &all[1..] {
        assert_eq!(args.last().map(String::as_str), Some("SYSLOG_FACILITY=10"));
    }
}

// ---------------------------------------------------------------------------------------
// Test 2 — cursor
// ---------------------------------------------------------------------------------------

/// Test 2 (RMC52, §2.2): accepted charset `[A-Za-z0-9=;_-]`, 1..=256 bytes; Debug redacted.
#[test]
fn test_rmc_alerts_journal_cursor_bounds() {
    assert_eq!(MAX_CURSOR_LEN, 256);
    let observed =
        "s=0b1e0c7f6e1a4d3c8a9b2c1d0e9f8a7b;i=1a2b3c;b=4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a;\
                    m=1f2e3d4c;t=63f0a1b2c3d4e;x=9a8b7c6d5e4f3a2b";
    let cursor = JournalCursor::parse(observed).unwrap();
    assert_eq!(cursor.as_str(), observed);
    assert!(JournalCursor::parse("AZaz09=;_-").is_some());
    assert!(JournalCursor::parse(&"a".repeat(256)).is_some());
    assert!(JournalCursor::parse(&"a".repeat(257)).is_none());
    assert!(JournalCursor::parse("").is_none());
    for bad in [
        "s=1 i=2", "s=1/2", "s=1\n", "s=1,2", "s=é", "s=1\"", "s=1\\",
    ] {
        assert!(JournalCursor::parse(bad).is_none(), "{bad:?}");
    }
    let debug = format!("{cursor:?}");
    assert!(debug.contains("redacted"), "{debug}");
    assert!(!debug.contains("0b1e0c7f"), "{debug}");
}

// ---------------------------------------------------------------------------------------
// Tests 3–5 — entry parsing
// ---------------------------------------------------------------------------------------

/// Test 3 (RMC46, §2.3): the observed shapes — string fields and byte-array fields of
/// printable UTF-8 — give the same entry; a missing or malformed cursor is not kept but the
/// entry is.
#[test]
fn test_rmc_alerts_parse_entry_accepts_string_and_byte_array_fields() {
    let line = pam_unix_line("sudo", OWNER_UID, Some(SUDO_EXE), OWNER, T);
    let entry = parsed(&line);
    assert_eq!(entry.realtime_us, T);
    assert_eq!(
        entry.cursor.as_ref().map(JournalCursor::as_str),
        Some(cursor_for(T).as_str())
    );
    assert_eq!(
        entry.message.as_str(),
        pam_unix_message("sudo", OWNER_UID, OWNER)
    );
    assert_eq!(
        entry.identifier.as_deref().map(String::as_str),
        Some("sudo")
    );
    assert_eq!(entry.facility, 10);
    assert_eq!(entry.uid, OWNER_UID);
    assert_eq!(entry.comm.as_deref().map(String::as_str), Some("sudo"));
    assert_eq!(entry.exe.as_deref().map(String::as_str), Some(SUDO_EXE));
    assert_eq!(entry.transport.as_str(), "syslog");

    // The byte-array form of every read field.
    let message = "password check failed for user (sooshost)";
    let bytes_line = chkpwd_line(0, OWNER, T)
        .with_bytes("MESSAGE", message.as_bytes())
        .with_bytes("SYSLOG_IDENTIFIER", b"unix_chkpwd")
        .with_bytes("_COMM", b"unix_chkpwd")
        .with_bytes("_UID", b"0")
        .with_bytes("_TRANSPORT", b"syslog")
        .with_bytes("SYSLOG_FACILITY", b"10")
        .with_bytes("__REALTIME_TIMESTAMP", T.to_string().as_bytes());
    let entry = parsed(&bytes_line);
    assert_eq!(entry.message.as_str(), message);
    assert_eq!(entry.uid, 0);
    assert_eq!(entry.realtime_us, T);
    assert_eq!(entry.transport.as_str(), "syslog");
    assert_eq!(
        entry.identifier.as_deref().map(String::as_str),
        Some("unix_chkpwd")
    );

    // Optional fields absent.
    let entry = parsed(
        &chkpwd_line(0, OWNER, T)
            .without("__CURSOR")
            .without("SYSLOG_IDENTIFIER")
            .without("_COMM"),
    );
    assert!(entry.cursor.is_none());
    assert!(entry.identifier.is_none());
    assert!(entry.comm.is_none());
    assert!(entry.exe.is_none());

    // A malformed or over-long cursor is dropped, the entry kept.
    for bad in ["s=1 i=2".to_string(), "a".repeat(257)] {
        let entry = parsed(&chkpwd_line(0, OWNER, T).with_str("__CURSOR", &bad));
        assert!(entry.cursor.is_none(), "{bad:?}");
    }

    // Unknown keys (strings, numbers, nested values, nulls) are ignored, duplicated or not.
    let entry = parsed(
        &chkpwd_line(0, OWNER, T)
            .with("_SYSTEMD_UNIT", json!("user@1000.service"))
            .with("__SEQNUM", json!(12))
            .with("X_NESTED", json!({"a": [1, {"b": null}]}))
            .with("X_NULL", Value::Null)
            .duplicated("X_DUP", json!("a"))
            .duplicated("X_DUP", json!("b")),
    );
    assert_eq!(entry.uid, 0);

    // Boundary values that are accepted.
    let entry =
        parsed(&chkpwd_line(0, OWNER, T).with_str("__REALTIME_TIMESTAMP", "18446744073709551615"));
    assert_eq!(entry.realtime_us, u64::MAX);
    let entry = parsed(&chkpwd_line(0, OWNER, T).with_str("_UID", "4294967295"));
    assert_eq!(entry.uid, u32::MAX);
    let max_message = "m".repeat(MAX_MESSAGE_BYTES);
    let entry = parsed(&chkpwd_line(0, OWNER, T).with_str("MESSAGE", &max_message));
    assert_eq!(entry.message.len(), MAX_MESSAGE_BYTES);
    let entry = parsed(&chkpwd_line(0, OWNER, T).with_str("_COMM", &"c".repeat(MAX_FIELD_BYTES)));
    assert_eq!(
        entry.comm.as_deref().map(String::len),
        Some(MAX_FIELD_BYTES)
    );
}

/// Test 4 (RMC46, §2.3, F-11): every shape other than a string or a printable byte array
/// is refused for a read field; a repeated read key is refused even when both values are
/// valid (a last-wins parser would accept it); trailing bytes, missing required fields,
/// another facility, over-long lines and out-of-range numbers are refused.
#[test]
fn test_rmc_alerts_parse_entry_refuses_bad_shapes() {
    let base = || chkpwd_line(0, OWNER, T);

    // Value shapes of a read field.
    for (what, value) in [
        ("null (field over 4096 bytes)", Value::Null),
        ("array of strings (duplicated field)", json!(["a", "b"])),
        ("nested object", json!({"a": "b"})),
        ("number", json!(42)),
        ("boolean", json!(true)),
        ("byte above 255", json!([112, 256])),
        ("negative byte", json!([112, -1])),
        ("float byte", json!([112.5])),
        ("mixed array", json!([112, "a"])),
    ] {
        assert_eq!(
            parse_err(&base().with("MESSAGE", value.clone())),
            EntryError::Shape,
            "MESSAGE as {what}"
        );
        assert_eq!(
            parse_err(&base().with("_COMM", value)),
            EntryError::Shape,
            "_COMM as {what}"
        );
    }
    // Invalid UTF-8 in a byte array.
    assert_eq!(
        parse_err(&base().with_bytes("MESSAGE", &[0x70, 0xff, 0xfe])),
        EntryError::Shape
    );
    // Control characters (ANSI colour codes included) and DEL in MESSAGE.
    for message in [
        "\u{1b}[31mpassword check failed for user (sooshost)\u{1b}[0m",
        "password check failed for user (sooshost)\n",
        "password check failed\tfor user (sooshost)",
        "password check failed for user (soos\u{7f}host)",
        "password check failed for user (\u{0}sooshost)",
    ] {
        assert_eq!(
            parse_err(&base().with_str("MESSAGE", message)),
            EntryError::Shape,
            "{message:?}"
        );
    }
    assert_eq!(
        parse_err(&base().with_bytes("MESSAGE", b"\x1b[2mINFO\x1b[0m password")),
        EntryError::Shape
    );
    // Empty MESSAGE.
    assert_eq!(
        parse_err(&base().with_str("MESSAGE", "")),
        EntryError::Shape
    );

    // A repeated read key, both values valid: every one of the nine read keys.
    for (key, value) in [
        ("__REALTIME_TIMESTAMP", json!(T.to_string())),
        ("__CURSOR", json!(cursor_for(T))),
        (
            "MESSAGE",
            json!("password check failed for user (sooshost)"),
        ),
        ("SYSLOG_IDENTIFIER", json!("unix_chkpwd")),
        ("SYSLOG_FACILITY", json!("10")),
        ("_UID", json!("0")),
        ("_COMM", json!("unix_chkpwd")),
        ("_EXE", json!("/usr/bin/unix_chkpwd")),
        ("_TRANSPORT", json!("syslog")),
    ] {
        let once = base().with(key, value.clone());
        assert!(parse_entry(&once.bytes()).is_ok(), "{key} once must parse");
        assert_eq!(
            parse_err(&once.duplicated(key, value)),
            EntryError::Shape,
            "{key} twice"
        );
    }
    // A repeated read key whose second value differs (last-wins would pick the second).
    assert_eq!(
        parse_err(&base().duplicated("_UID", json!("1000"))),
        EntryError::Shape
    );

    // Not a JSON object, or trailing bytes after it.
    for raw in [
        &b""[..],
        b"[]",
        b"42",
        b"\"text\"",
        b"null",
        b"{",
        b"{\"MESSAGE\":",
        b"not json",
    ] {
        assert_eq!(
            parse_entry(raw).err(),
            Some(EntryError::NotJson),
            "{:?}",
            String::from_utf8_lossy(raw)
        );
    }
    let mut trailing = base().bytes();
    trailing.extend_from_slice(b" {}");
    assert_eq!(parse_entry(&trailing).err(), Some(EntryError::NotJson));
    let mut trailing = base().bytes();
    trailing.extend_from_slice(b"x");
    assert_eq!(parse_entry(&trailing).err(), Some(EntryError::NotJson));

    // One byte over a field bound (string and byte-array forms).
    assert_eq!(
        parse_err(&base().with_str("MESSAGE", &"m".repeat(MAX_MESSAGE_BYTES + 1))),
        EntryError::Shape
    );
    assert_eq!(
        parse_err(&base().with_bytes("MESSAGE", "m".repeat(MAX_MESSAGE_BYTES + 1).as_bytes())),
        EntryError::Shape
    );
    for key in ["_COMM", "_EXE", "SYSLOG_IDENTIFIER", "_TRANSPORT"] {
        assert_eq!(
            parse_err(&base().with_str(key, &"f".repeat(MAX_FIELD_BYTES + 1))),
            EntryError::Shape,
            "{key}"
        );
    }

    // Missing required fields.
    for key in [
        "__REALTIME_TIMESTAMP",
        "MESSAGE",
        "_UID",
        "_TRANSPORT",
        "SYSLOG_FACILITY",
    ] {
        assert_eq!(
            parse_err(&base().without(key)),
            EntryError::MissingField,
            "{key}"
        );
    }

    // Another facility.
    for facility in ["4", "0", "11", "3"] {
        assert_eq!(
            parse_err(&base().with_str("SYSLOG_FACILITY", facility)),
            EntryError::Facility,
            "{facility}"
        );
    }

    // Numbers out of shape or range.
    for uid in ["abc", "-1", "4294967296", "", " 0", "0x0", "1e3"] {
        assert_eq!(
            parse_err(&base().with_str("_UID", uid)),
            EntryError::Shape,
            "_UID {uid:?}"
        );
    }
    for ts in [
        "123456789012345678901",
        "18446744073709551616",
        "99999999999999999999",
        "",
        "-1",
        "12ab",
    ] {
        assert_eq!(
            parse_err(&base().with_str("__REALTIME_TIMESTAMP", ts)),
            EntryError::Shape,
            "__REALTIME_TIMESTAMP {ts:?}"
        );
    }

    // An over-long line is refused before any JSON parsing (even garbage).
    assert_eq!(
        parse_entry(&[b'x'; MAX_JOURNAL_LINE_BYTES + 1]).err(),
        Some(EntryError::TooLong)
    );
    let mut long = base();
    let filler_len = MAX_JOURNAL_LINE_BYTES + 1 - long.bytes().len() - "\"X_FILL\":\"\",".len();
    long = long.with_str("X_FILL", &"z".repeat(filler_len));
    assert_eq!(long.bytes().len(), MAX_JOURNAL_LINE_BYTES + 1);
    assert_eq!(parse_entry(&long.bytes()).err(), Some(EntryError::TooLong));
    // Exactly at the bound is parsed.
    let mut exact = base();
    let filler_len = MAX_JOURNAL_LINE_BYTES - exact.bytes().len() - "\"X_FILL\":\"\",".len();
    exact = exact.with_str("X_FILL", &"z".repeat(filler_len));
    assert_eq!(exact.bytes().len(), MAX_JOURNAL_LINE_BYTES);
    assert!(parse_entry(&exact.bytes()).is_ok());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Test 5 (RMC46): `parse_entry` never panics on arbitrary bytes up to 30 000 bytes.
    #[test]
    fn test_rmc_alerts_parse_entry_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..30_000)) {
        let _ = parse_entry(&bytes);
    }

    /// Test 5 (RMC46): nor on JSON-shaped input mixing the read keys with arbitrary values.
    #[test]
    fn test_rmc_alerts_parse_entry_never_panics_on_json_shapes(
        keys in proptest::collection::vec(
            prop_oneof![
                Just("__REALTIME_TIMESTAMP"), Just("__CURSOR"), Just("MESSAGE"),
                Just("SYSLOG_IDENTIFIER"), Just("SYSLOG_FACILITY"), Just("_UID"),
                Just("_COMM"), Just("_EXE"), Just("_TRANSPORT"), Just("OTHER"),
            ],
            0..14,
        ),
        values in proptest::collection::vec(
            prop_oneof![
                "\\PC{0,40}".prop_map(Value::String),
                proptest::collection::vec(any::<u16>(), 0..20)
                    .prop_map(|v| Value::Array(v.into_iter().map(Value::from).collect())),
                any::<i64>().prop_map(Value::from),
                Just(Value::Null),
                Just(json!({"a": [1, 2]})),
            ],
            14,
        ),
    ) {
        let mut line = JLine { fields: Vec::new() };
        for (key, value) in keys.iter().zip(values) {
            line = line.duplicated(key, value);
        }
        let _ = parse_entry(&line.bytes());
    }
}

// ---------------------------------------------------------------------------------------
// Tests 6–11 — classification
// ---------------------------------------------------------------------------------------

/// Test 6 (RMC47, §2.4): `unix_chkpwd` checks — side from `_UID`, other UIDs ignored, the
/// identifier / `_COMM` / `_EXE` gates, and the grammar edges.
#[test]
fn test_rmc_alerts_classify_unix_chkpwd() {
    let c = ctx(&[]);
    assert_eq!(
        classify(&chkpwd_line(0, OWNER, T), &c),
        check(HelperSide::Root, AccountClass::Owner, T)
    );
    assert_eq!(
        classify(&chkpwd_line(OWNER_UID, OWNER, T), &c),
        check(HelperSide::Owner, AccountClass::Owner, T)
    );
    assert_eq!(classify(&chkpwd_line(OTHER_UID, OWNER, T), &c), None);
    assert_eq!(classify(&chkpwd_line(65534, OWNER, T), &c), None);

    // Identifier gate.
    assert_eq!(
        classify(&chkpwd_line(0, OWNER, T).without("SYSLOG_IDENTIFIER"), &c),
        None
    );
    for identifier in ["unix_chkpwdx", "Unix_chkpwd", "pam", "sudo"] {
        assert_eq!(
            classify(
                &chkpwd_line(0, OWNER, T).with_str("SYSLOG_IDENTIFIER", identifier),
                &c
            ),
            None,
            "{identifier}"
        );
    }
    // `_COMM` gate: absent or `unix_chkpwd`.
    assert_eq!(
        classify(&chkpwd_line(0, OWNER, T).without("_COMM"), &c),
        check(HelperSide::Root, AccountClass::Owner, T)
    );
    for comm in ["bash", "unix_chkpw", "python3"] {
        assert_eq!(
            classify(
                &chkpwd_line(OWNER_UID, OWNER, T).with_str("_COMM", comm),
                &c
            ),
            None,
            "{comm}"
        );
    }
    // `_EXE` gate: absent or one of the two helper paths.
    for exe in ["/usr/bin/unix_chkpwd", "/usr/sbin/unix_chkpwd"] {
        assert_eq!(
            classify(&chkpwd_line(0, OWNER, T).with_str("_EXE", exe), &c),
            check(HelperSide::Root, AccountClass::Owner, T),
            "{exe}"
        );
    }
    for exe in [
        "/tmp/unix_chkpwd",
        "/home/sooshost/bin/unix_chkpwd",
        "/usr/bin/unix_chkpwd2",
        "/usr/local/bin/unix_chkpwd",
    ] {
        assert_eq!(
            classify(&chkpwd_line(OWNER_UID, OWNER, T).with_str("_EXE", exe), &c),
            None,
            "{exe}"
        );
    }
    // Common gate: transport and facility.
    for transport in ["journal", "stdout", "kernel"] {
        assert_eq!(
            classify(
                &chkpwd_line(0, OWNER, T).with_str("_TRANSPORT", transport),
                &c
            ),
            None,
            "{transport}"
        );
    }

    // Grammar edges.
    let with_message = |m: &str| chkpwd_line(0, OWNER, T).with_str("MESSAGE", m);
    for message in [
        "password check failed for user (sooshost",
        "password check failed for user ()",
        "password check failed for user sooshost",
        "password check failed for user (sooshost) ",
        "password check failed for user (sooshost) extra",
        " password check failed for user (sooshost)",
        "Password check failed for user (sooshost)",
        "password check failed for users (sooshost)",
    ] {
        assert_eq!(classify(&with_message(message), &c), None, "{message:?}");
    }
    let name_256 = "n".repeat(256);
    assert_eq!(
        classify(
            &with_message(&format!("password check failed for user ({name_256})")),
            &c
        ),
        check(HelperSide::Root, AccountClass::Other, T)
    );
    let name_257 = "n".repeat(257);
    assert_eq!(
        classify(
            &with_message(&format!("password check failed for user ({name_257})")),
            &c
        ),
        None
    );
    assert_eq!(
        classify(&with_message("password check failed for user (root)"), &c),
        check(HelperSide::Root, AccountClass::Root, T)
    );
}

/// Test 7 (RMC47, §2.4 trust table, F-2): every row of the `pam_unix` trust table,
/// including `_EXE` absent, a developer test binary, `/usr/bin/sudo`, `/usr/bin/su`, an empty
/// `lock_screen_programs`, and the owner's real locker trusted only when configured.
#[test]
fn test_rmc_alerts_classify_pam_unix_trust_table() {
    let configured = ctx(&[OWNER_LOCKER]);
    let empty = ctx(&[]);
    let defaults = ctx(&[
        "/usr/bin/swaylock",
        "/usr/bin/hyprlock",
        "/usr/bin/gtklock",
        "/usr/bin/waylock",
    ]);

    // Row 1: `_UID == 0` → trusted, side Root, whatever `_EXE`.
    for (service, exe, class) in [
        ("gdm-password", Some(GDM_WORKER_EXE), SourceClass::Login),
        ("polkit-1", Some(POLKIT_HELPER_EXE), SourceClass::Other),
        ("sshd", None, SourceClass::Other),
        ("swaylock", None, SourceClass::LockScreen),
    ] {
        for c in [&configured, &empty] {
            assert_eq!(
                classify(&pam_unix_line(service, 0, exe, OWNER, T), c),
                failure(class, HelperSide::Root, AccountClass::Owner, T, true),
                "{service}"
            );
        }
    }

    // Row 2: owner UID with the setuid `/usr/bin/sudo` or `/usr/bin/su` → trusted, Root.
    assert_eq!(
        classify(
            &pam_unix_line("sudo", OWNER_UID, Some(SUDO_EXE), OWNER, T),
            &empty
        ),
        failure(
            SourceClass::Sudo,
            HelperSide::Root,
            AccountClass::Owner,
            T,
            true
        )
    );
    assert_eq!(
        classify(
            &pam_unix_line("su", OWNER_UID, Some(SU_EXE), "root", T),
            &empty
        ),
        failure(
            SourceClass::Other,
            HelperSide::Root,
            AccountClass::Root,
            T,
            true
        )
    );
    assert_eq!(
        classify(
            &pam_unix_line("su-l", OWNER_UID, Some(SU_EXE), "root", T),
            &empty
        ),
        failure(
            SourceClass::Other,
            HelperSide::Root,
            AccountClass::Root,
            T,
            true
        )
    );

    // Row 3: owner UID, lock-screen service, `_EXE` in `lock_screen_programs` → trusted,
    // side Owner.
    assert_eq!(
        classify(
            &pam_unix_line("swaylock", OWNER_UID, Some(OWNER_LOCKER), OWNER, T),
            &configured
        ),
        failure(
            SourceClass::LockScreen,
            HelperSide::Owner,
            AccountClass::Owner,
            T,
            true
        )
    );
    assert_eq!(
        classify(
            &pam_unix_line("swaylock", OWNER_UID, Some("/usr/bin/swaylock"), OWNER, T),
            &defaults
        ),
        failure(
            SourceClass::LockScreen,
            HelperSide::Owner,
            AccountClass::Owner,
            T,
            true
        )
    );

    // Row 4: owner UID, anything else → untrusted, side Owner.
    let untrusted = |class| failure(class, HelperSide::Owner, AccountClass::Owner, T, false);
    // The owner's real locker is not trusted when it is not configured.
    for c in [&empty, &defaults] {
        assert_eq!(
            classify(
                &pam_unix_line("swaylock", OWNER_UID, Some(OWNER_LOCKER), OWNER, T),
                c
            ),
            untrusted(SourceClass::LockScreen)
        );
    }
    // No `_EXE`.
    assert_eq!(
        classify(
            &pam_unix_line("swaylock", OWNER_UID, None, OWNER, T),
            &configured
        ),
        untrusted(SourceClass::LockScreen)
    );
    // A developer test binary writing a real `pam_unix(swaylock:auth)` line.
    assert_eq!(
        classify(
            &pam_unix_line("swaylock", OWNER_UID, Some(TEST_BINARY), OWNER, T),
            &configured
        ),
        untrusted(SourceClass::LockScreen)
    );
    // `sudo` service without the setuid `_EXE`.
    assert_eq!(
        classify(
            &pam_unix_line("sudo", OWNER_UID, None, OWNER, T),
            &configured
        ),
        untrusted(SourceClass::Sudo)
    );
    assert_eq!(
        classify(
            &pam_unix_line("sudo", OWNER_UID, Some("/home/sooshost/bin/sudo"), OWNER, T),
            &configured
        ),
        untrusted(SourceClass::Sudo)
    );
    // A configured locker logging a non-lock-screen service is not row 3.
    assert_eq!(
        classify(
            &pam_unix_line("polkit-1", OWNER_UID, Some(OWNER_LOCKER), OWNER, T),
            &configured
        ),
        untrusted(SourceClass::Other)
    );
    assert_eq!(
        classify(
            &pam_unix_line("gdm-password", OWNER_UID, Some(OWNER_LOCKER), OWNER, T),
            &configured
        ),
        untrusted(SourceClass::Login)
    );
    // A forged identifier changes nothing: trust comes from `_UID` / `_EXE` only.
    assert_eq!(
        classify(
            &pam_unix_line("sudo", OWNER_UID, None, OWNER, T).with_str("SYSLOG_IDENTIFIER", "sudo"),
            &configured
        ),
        untrusted(SourceClass::Sudo)
    );
    // Prefix and path comparisons are exact.
    assert_eq!(
        classify(
            &pam_unix_line(
                "swaylock",
                OWNER_UID,
                Some("/home/sooshost/.local/bin/swaylock-plugin2"),
                OWNER,
                T
            ),
            &configured
        ),
        untrusted(SourceClass::LockScreen)
    );

    // Any other UID → no signal.
    for uid in [OTHER_UID, 65534, 999] {
        assert_eq!(
            classify(
                &pam_unix_line("sudo", uid, Some(SUDO_EXE), OWNER, T),
                &configured
            ),
            None,
            "uid {uid}"
        );
    }
}

/// Test 8 (RMC47, §2.4 service table): every listed service maps to its class; other valid
/// services are `Other`; an invalid or over-long service name is no signal.
#[test]
fn test_rmc_alerts_classify_service_classes() {
    let c = ctx(&[]);
    let class_of = |service: &str| match classify(&pam_unix_line(service, 0, None, OWNER, T), &c) {
        Some(Signal::Failure { class, .. }) => Some(class),
        other => {
            assert!(other.is_none(), "{service}: {other:?}");
            None
        }
    };
    for service in [
        "swaylock",
        "hyprlock",
        "gtklock",
        "waylock",
        "i3lock",
        "xscreensaver",
        "kde",
    ] {
        assert_eq!(
            class_of(service),
            Some(SourceClass::LockScreen),
            "{service}"
        );
    }
    for service in ["sudo", "sudo-i"] {
        assert_eq!(class_of(service), Some(SourceClass::Sudo), "{service}");
    }
    for service in ["gdm-password", "login", "sddm", "lightdm", "greetd"] {
        assert_eq!(class_of(service), Some(SourceClass::Login), "{service}");
    }
    for service in ["polkit-1", "su", "su-l", "sshd", "system-auth", "a.b_c-d"] {
        assert_eq!(class_of(service), Some(SourceClass::Other), "{service}");
    }
    let longest = "s".repeat(64);
    assert_eq!(class_of(&longest), Some(SourceClass::Other));
    for service in [
        "swaylock)".to_string(),
        "sway lock".to_string(),
        String::new(),
        "s".repeat(65),
        "sudo/x".to_string(),
        "Swaylock(".to_string(),
        "su:do".to_string(),
    ] {
        assert_eq!(class_of(&service), None, "{service:?}");
    }
    // Case matters: an upper-case lookalike is a valid name of class `Other`.
    assert_eq!(class_of("Swaylock"), Some(SourceClass::Other));
}

/// Test 9 (RMC50, §2.4, A-7): only the "temporarily locked out due to N consecutive failed
/// login attempts" grammar of `pam_faillock` is a signal, and only when trusted.
#[test]
fn test_rmc_alerts_classify_faillock_locked_out_only() {
    let c = ctx(&[OWNER_LOCKER]);
    assert_eq!(
        classify(
            &faillock_locked_line("gdm-password", 0, Some(GDM_WORKER_EXE), OWNER, T),
            &c
        ),
        Some(Signal::LockedOut {
            class: SourceClass::Login,
            account: AccountClass::Owner,
            at_us: T
        })
    );
    assert_eq!(
        classify(
            &faillock_locked_line("swaylock", OWNER_UID, Some(OWNER_LOCKER), OWNER, T),
            &c
        ),
        Some(Signal::LockedOut {
            class: SourceClass::LockScreen,
            account: AccountClass::Owner,
            at_us: T
        })
    );
    assert_eq!(
        classify(
            &faillock_locked_line("sudo", OWNER_UID, Some(SUDO_EXE), "root", T),
            &c
        ),
        Some(Signal::LockedOut {
            class: SourceClass::Sudo,
            account: AccountClass::Root,
            at_us: T
        })
    );
    // Untrusted locked-out lines are no signal.
    for exe in [None, Some(TEST_BINARY)] {
        assert_eq!(
            classify(
                &faillock_locked_line("swaylock", OWNER_UID, exe, OWNER, T),
                &c
            ),
            None,
            "{exe:?}"
        );
    }
    assert_eq!(
        classify(
            &faillock_locked_line("swaylock", OWNER_UID, Some(OWNER_LOCKER), OWNER, T),
            &ctx(&[])
        ),
        None
    );
    // Other faillock grammars and digit bounds.
    let faillock = |message: &str| process_line(0, Some(GDM_WORKER_EXE), message, T);
    for message in [
        "pam_faillock(gdm-password:auth): Consecutive login failures for user sooshost account temporarily locked",
        "pam_faillock(gdm-password:auth): User unknown: sooshost",
        "pam_faillock(gdm-password:auth): Error sending audit message",
        "pam_faillock(gdm-password:auth): User sooshost is temporarily locked out due to 123456 consecutive failed login attempts",
        "pam_faillock(gdm-password:auth): User sooshost is temporarily locked out due to  consecutive failed login attempts",
        "pam_faillock(gdm-password:auth): User sooshost is temporarily locked out due to 3x consecutive failed login attempts",
        "pam_faillock(gdm-password:auth): User sooshost is temporarily locked out due to 3 consecutive failed login attempts.",
        "pam_faillock(gdm password:auth): User sooshost is temporarily locked out due to 3 consecutive failed login attempts",
        "xpam_faillock(gdm-password:auth): User sooshost is temporarily locked out due to 3 consecutive failed login attempts",
    ] {
        assert_eq!(classify(&faillock(message), &c), None, "{message:?}");
    }
    assert_eq!(
        classify(
            &faillock(
                "pam_faillock(gdm-password:auth): User sooshost is temporarily locked out due to 99999 consecutive failed login attempts"
            ),
            &c
        ),
        Some(Signal::LockedOut {
            class: SourceClass::Login,
            account: AccountClass::Owner,
            at_us: T
        })
    );
}

/// Test 10 (RMC47, §2.4, observation): PAM lines that are not wrong passwords, journal or
/// stdout copies of a valid message, and a `sudo` command-log line embedding the failure
/// grammar are no signal (prefix anchoring).
#[test]
fn test_rmc_alerts_ignored_lines() {
    let c = ctx(&[OWNER_LOCKER]);
    for message in [
        "pam_unix(gdm-password:auth): conversation failed",
        "pam_unix(gdm-password:auth): auth could not identify password for [sooshost]",
        "pam_unix(sudo:session): session opened for user root(uid=0) by sooshost(uid=1000)",
        "pam_unix(sudo:session): session closed for user root",
        "pam_unix(sudo:auth): check pass; user unknown",
        "soos-pam: could not resolve the PAM user",
        "soos-pam: authentication panic caught",
        "pam_unix(sudo:account): authentication failure; logname= uid=0 euid=0 tty= ruser= rhost=  user=sooshost",
    ] {
        assert_eq!(
            classify(&process_line(0, Some(GDM_WORKER_EXE), message, T), &c),
            None,
            "{message:?}"
        );
    }
    // A valid message copied to another transport.
    for transport in ["journal", "stdout", "stderr", "audit"] {
        assert_eq!(
            classify(
                &pam_unix_line("gdm-password", 0, Some(GDM_WORKER_EXE), OWNER, T)
                    .with_str("_TRANSPORT", transport),
                &c
            ),
            None,
            "{transport}"
        );
        assert_eq!(
            classify(
                &chkpwd_line(0, OWNER, T).with_str("_TRANSPORT", transport),
                &c
            ),
            None,
            "{transport}"
        );
    }
    // The sudo command log (`_UID=0`, identifier `sudo`) quoting the grammar in COMMAND=.
    let command_log = format!(
        "{OWNER} : TTY=pts/3 ; PWD=/home/{OWNER} ; USER=root ; COMMAND=/usr/bin/echo \
         pam_unix(sudo:auth): authentication failure; logname= uid=0 euid=0 tty= ruser= rhost=  user=root"
    );
    assert_eq!(
        classify(&process_line(0, Some(SUDO_EXE), &command_log, T), &c),
        None
    );
    let quoted_check =
        format!("{OWNER} : COMMAND=/usr/bin/echo password check failed for user (root)");
    assert_eq!(
        classify(
            &chkpwd_line(0, OWNER, T).with_str("MESSAGE", &quoted_check),
            &c
        ),
        None
    );
}

/// Test 11 (RMC48, A-8): the account field maps to `owner` / `root` / `other` only; the
/// raw text (possibly a password typed into a user-name box) never survives in the signal.
#[test]
fn test_rmc_alerts_account_is_mapped_to_three_classes() {
    let c = ctx(&[]);
    let account_of = |line: &JLine| match classify(line, &c) {
        Some(Signal::Failure { account, .. }) | Some(Signal::Check { account, .. }) => account,
        other => panic!("expected a signal, got {other:?}"),
    };
    let gdm = |name: &str| pam_unix_line("gdm-password", 0, Some(GDM_WORKER_EXE), name, T);
    assert_eq!(account_of(&gdm(OWNER)), AccountClass::Owner);
    assert_eq!(account_of(&gdm("root")), AccountClass::Root);
    assert_eq!(account_of(&gdm("Tr0ub4dor&3xyz")), AccountClass::Other);
    assert_eq!(account_of(&gdm("SoosHost")), AccountClass::Other);
    assert_eq!(account_of(&gdm("sooshost ")), AccountClass::Other);
    assert_eq!(account_of(&gdm("Root")), AccountClass::Other);
    assert_eq!(account_of(&gdm("")), AccountClass::Other);
    // The name is the text after the first ` user=` that follows `rhost=`.
    assert_eq!(
        account_of(&process_line(
            0,
            Some(GDM_WORKER_EXE),
            "pam_unix(gdm-password:auth): authentication failure; logname= uid=0 euid=0 tty= ruser=x user=root rhost=  user=sooshost",
            T
        )),
        AccountClass::Owner
    );
    // No ` user=` at all.
    assert_eq!(
        account_of(&process_line(
            0,
            Some(GDM_WORKER_EXE),
            "pam_unix(gdm-password:auth): authentication failure; logname= uid=0 euid=0 tty= ruser= rhost=",
            T
        )),
        AccountClass::Other
    );
    assert_eq!(
        account_of(&chkpwd_line(0, "hunter2-Secret!", T)),
        AccountClass::Other
    );
    assert_eq!(
        account_of(&chkpwd_line(0, "SOOSHOST", T)),
        AccountClass::Other
    );

    // The signal carries no text: its Debug rendering never contains the typed name.
    let signal = classify(&gdm("Tr0ub4dor&3xyz"), &c).unwrap();
    let debug = format!("{signal:?}");
    assert!(!debug.contains("Tr0ub4dor"), "{debug}");
}

/// Test 12 (RMC48, §2.4): `OwnerLogin` grammar `[a-z_][a-z0-9_-]*` with an optional final
/// `$`, 1..=32 bytes; Debug redacted.
#[test]
fn test_rmc_alerts_owner_login_bounds() {
    for good in [
        "sooshost",
        "_svc",
        "a-b_c9",
        "machine$",
        "a",
        "_",
        &"a".repeat(32),
        &format!("{}$", "a".repeat(31)),
    ] {
        assert!(OwnerLogin::parse(good).is_some(), "{good:?}");
    }
    for bad in [
        "",
        "Alice",
        "sooshosT",
        "9abc",
        "-abc",
        "a b",
        "a$b",
        "$",
        "a$$",
        "a.b",
        "a/b",
        "é",
        &"a".repeat(33),
        &format!("{}$", "a".repeat(32)),
    ] {
        assert!(OwnerLogin::parse(bad).is_none(), "{bad:?}");
    }
    let login = OwnerLogin::parse("sooshost").unwrap();
    assert_eq!(login, OwnerLogin::parse("sooshost").unwrap());
    let debug = format!("{login:?}");
    assert!(debug.contains("redacted"), "{debug}");
    assert!(!debug.contains("sooshost"), "{debug}");
}

// ---------------------------------------------------------------------------------------
// Test 51 — `_EXE` normalisation
// ---------------------------------------------------------------------------------------

/// Test 51 (RMC47, A-5, F-2 b): exactly one trailing ` (deleted)` is stripped before every
/// `_EXE` comparison; nothing else is normalised.
#[test]
fn test_rmc_alerts_exe_deleted_suffix_is_stripped() {
    assert_eq!(EXE_DELETED_SUFFIX, " (deleted)");
    for (raw, expected) in [
        ("/usr/bin/sudo (deleted)", "/usr/bin/sudo"),
        ("/a (deleted) (deleted)", "/a (deleted)"),
        (" (deleted)", ""),
        ("/a (deleted)/b", "/a (deleted)/b"),
        ("/usr/bin/sudo", "/usr/bin/sudo"),
        ("/usr/bin/sudo(deleted)", "/usr/bin/sudo(deleted)"),
        ("/usr/bin/sudo (DELETED)", "/usr/bin/sudo (DELETED)"),
        ("", ""),
    ] {
        assert_eq!(exe_for_comparison(raw), expected, "{raw:?}");
    }
    let c = ctx(&[OWNER_LOCKER]);
    let deleted = |path: &str| format!("{path}{EXE_DELETED_SUFFIX}");
    // A replaced locker still running is trusted.
    assert_eq!(
        classify(
            &pam_unix_line(
                "swaylock",
                OWNER_UID,
                Some(&deleted(OWNER_LOCKER)),
                OWNER,
                T
            ),
            &c
        ),
        failure(
            SourceClass::LockScreen,
            HelperSide::Owner,
            AccountClass::Owner,
            T,
            true
        )
    );
    assert_eq!(
        classify(
            &faillock_locked_line(
                "swaylock",
                OWNER_UID,
                Some(&deleted(OWNER_LOCKER)),
                OWNER,
                T
            ),
            &c
        ),
        Some(Signal::LockedOut {
            class: SourceClass::LockScreen,
            account: AccountClass::Owner,
            at_us: T
        })
    );
    // `sudo` upgraded under a running process.
    assert_eq!(
        classify(
            &pam_unix_line("sudo", OWNER_UID, Some(&deleted(SUDO_EXE)), OWNER, T),
            &c
        ),
        failure(
            SourceClass::Sudo,
            HelperSide::Root,
            AccountClass::Owner,
            T,
            true
        )
    );
    // Two suffixes, or a lookalike path, stay untrusted.
    assert_eq!(
        classify(
            &pam_unix_line(
                "sudo",
                OWNER_UID,
                Some(&deleted(&deleted(SUDO_EXE))),
                OWNER,
                T
            ),
            &c
        ),
        failure(
            SourceClass::Sudo,
            HelperSide::Owner,
            AccountClass::Owner,
            T,
            false
        )
    );
    assert_eq!(
        classify(
            &pam_unix_line(
                "sudo",
                OWNER_UID,
                Some("/usr/bin/sudox (deleted)"),
                OWNER,
                T
            ),
            &c
        ),
        failure(
            SourceClass::Sudo,
            HelperSide::Owner,
            AccountClass::Owner,
            T,
            false
        )
    );
    // The helper replaced while running is still a check.
    assert_eq!(
        classify(
            &chkpwd_line(OWNER_UID, OWNER, T).with_str("_EXE", "/usr/bin/unix_chkpwd (deleted)"),
            &c
        ),
        check(HelperSide::Owner, AccountClass::Owner, T)
    );
    assert_eq!(
        classify(
            &chkpwd_line(OWNER_UID, OWNER, T)
                .with_str("_EXE", "/usr/bin/unix_chkpwd (deleted) (deleted)"),
            &c
        ),
        None
    );
}
