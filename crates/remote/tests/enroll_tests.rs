//! Contract tests of the one-time enrollment code of `soos-remote` (ADR 2026-10-06
//! "Tailscale Funnel Access and In-House Passkey Authentication for `soos-remote`",
//! architect spec §4.5, tests 30–31, matrix RMC34).

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
use std::sync::Arc;

use passkey::{code_file_line, code_hash, own_uid, sha256, write_file_mode};
use soos_remote::auth::{system_random, RandomError, RandomSource};
use soos_remote::enroll::{
    encode_code_file, normalized_code_hash, parse_code_file, read_code_file, remove_code_file,
    write_code_file, CodeFile, EnrollCode, EnrollError,
};
use soos_remote::{
    ENROLL_CODE_ALPHABET, ENROLL_CODE_FILE_NAME, ENROLL_CODE_LEN, MAX_ENROLL_CODE_FILE_BYTES,
};

/// Bytes `0xe0 + i`: only the low five bits select a symbol, so the code is "0123456789".
fn scripted_random() -> RandomSource {
    Arc::new(|buf: &mut [u8]| -> Result<(), RandomError> {
        for (i, b) in buf.iter_mut().enumerate() {
            *b = 0xe0u8.wrapping_add(i as u8);
        }
        Ok(())
    })
}

fn failing_random() -> RandomSource {
    Arc::new(|_: &mut [u8]| -> Result<(), RandomError> { Err(RandomError::Failed) })
}

/// Test 30 (RMC34): 10 Crockford base32 symbols (50 bits) shown `XXXXX-XXXXX`, uniform
/// mapping `byte & 31`, hashed normalized; normalisation uppercases and drops `-` and
/// spaces; `I L O U`, other lengths and inputs over 32 bytes are refused; RNG failure is an
/// error.
#[test]
fn test_rmc_enroll_code_generation_and_normalisation() {
    let code = EnrollCode::generate(&scripted_random()).expect("scripted code");
    assert_eq!(code.display(), "01234-56789");
    assert_eq!(code.hash(), code_hash("0123456789"));
    assert!(matches!(
        EnrollCode::generate(&failing_random()),
        Err(RandomError::Failed)
    ));

    let a = EnrollCode::generate(&system_random()).expect("system RNG");
    let b = EnrollCode::generate(&system_random()).expect("system RNG");
    for code in [&a, &b] {
        let shown = code.display();
        assert_eq!(shown.len(), ENROLL_CODE_LEN + 1);
        assert_eq!(shown.as_bytes()[5], b'-');
        let symbols: String = shown.chars().filter(|c| *c != '-').collect();
        assert_eq!(symbols.len(), ENROLL_CODE_LEN);
        assert!(
            symbols.bytes().all(|s| ENROLL_CODE_ALPHABET.contains(&s)),
            "{shown}"
        );
        assert_eq!(normalized_code_hash(&shown), Some(code.hash()));
    }
    assert_ne!(a.display(), b.display(), "50 random bits");

    let expected = Some(code_hash("ABCDEFGHJK"));
    for input in [
        "ABCDE-FGHJK",
        "ABCDEFGHJK",
        "abcde-fghjk",
        "aBcDe FgHjK",
        " abcde-fghjk ",
        "A-B-C-D-E-F-G-H-J-K",
        "--ABCDEFGHJK--",
    ] {
        assert_eq!(normalized_code_hash(input), expected, "{input:?}");
    }
    let over_32 = format!("ABCDE{}FGHJK", "-".repeat(23));
    assert_eq!(over_32.len(), 33);
    for input in [
        "ABCDE-FGHJI",
        "ABCDE-FGHJL",
        "ABCDE-FGHJO",
        "ABCDE-FGHJU",
        "ABCDE-FGHJ",
        "ABCDE-FGHJKM",
        "",
        "-----",
        "ABCDE_FGHJK",
        "ABCDE\tFGHJK",
        "ABCDÉ-FGHJK",
        over_32.as_str(),
    ] {
        assert_eq!(normalized_code_hash(input), None, "{input:?}");
    }
    assert_eq!(code_hash("ABCDEFGHJK"), sha256(b"ABCDEFGHJK"));
}

/// Test 31 (RMC34, S-6): the code file line format, a `0600` atomic write in the socket
/// directory that replaces any previous code and never follows a symlink, the read rules
/// (owner, mode, size, regular file) and an idempotent removal.
#[test]
fn test_rmc_enroll_code_file_rules() {
    let hash = code_hash("ABCDEFGHJK");
    let file = CodeFile {
        code_hash: hash,
        expires_unix_s: 1_700_000_300,
    };
    let line = code_file_line(&hash, 1_700_000_300);
    assert_eq!(encode_code_file(&file), line);
    assert!(parse_code_file(line.as_bytes()) == Ok(file));
    let upper = line.replace(
        &passkey::to_hex(&hash),
        &passkey::to_hex(&hash).to_uppercase(),
    );
    for bad in [
        upper,
        line.replace("v1", "v2"),
        line.replace(' ', "  "),
        line.replace("1700000300", "-1"),
        line.replace("1700000300", "17000003x0"),
        line.replace("1700000300", ""),
        line.replace("\n", " extra\n"),
        format!(
            "soos-remote-enroll-v1 {} 1\n",
            &passkey::to_hex(&hash)[..63]
        ),
        format!("soos-remote-enroll-v1 {}0 1\n", passkey::to_hex(&hash)),
        String::new(),
        "x".repeat(MAX_ENROLL_CODE_FILE_BYTES + 1),
    ] {
        assert!(
            parse_code_file(bad.as_bytes()) == Err(EnrollError::Malformed),
            "{bad:?}"
        );
    }

    let dir = tempfile::tempdir().unwrap();
    let uid = own_uid();
    let path = dir.path().join(ENROLL_CODE_FILE_NAME);
    assert!(
        read_code_file(dir.path(), uid) == Ok(None),
        "missing = no code"
    );
    write_code_file(dir.path(), &file, &scripted_random()).expect("write");
    assert_eq!(fs::read_to_string(&path).unwrap(), line);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(read_code_file(dir.path(), uid) == Ok(Some(file)));

    // Replaces a previous code.
    let second = CodeFile {
        code_hash: code_hash("0123456789"),
        expires_unix_s: 1_700_000_600,
    };
    write_code_file(dir.path(), &second, &scripted_random()).expect("rewrite");
    assert!(read_code_file(dir.path(), uid) == Ok(Some(second)));
    let leftovers: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != ENROLL_CODE_FILE_NAME)
        .collect();
    assert!(leftovers.is_empty(), "no temp file left: {leftovers:?}");

    // Read rules.
    for mode in [0o644, 0o640, 0o604] {
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        assert!(
            read_code_file(dir.path(), uid) == Err(EnrollError::Insecure),
            "{mode:o}"
        );
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        read_code_file(dir.path(), uid.wrapping_add(1)) == Err(EnrollError::Insecure),
        "foreign owner"
    );
    write_file_mode(&path, &vec![b'x'; MAX_ENROLL_CODE_FILE_BYTES + 1], 0o600);
    let oversize = read_code_file(dir.path(), uid);
    assert!(
        oversize == Err(EnrollError::Malformed) || oversize == Ok(None),
        "an oversize file is never a code"
    );
    write_file_mode(&path, b"garbage\n", 0o600);
    assert!(read_code_file(dir.path(), uid) == Err(EnrollError::Malformed));

    // Symlinks are never followed, neither by read, write nor remove.
    fs::remove_file(&path).unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let target = elsewhere.path().join("target");
    write_file_mode(&target, line.as_bytes(), 0o600);
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(
        read_code_file(dir.path(), uid).is_err(),
        "a symlinked code file is refused"
    );
    remove_code_file(dir.path());
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        line,
        "target untouched"
    );
    if path.symlink_metadata().is_ok() {
        fs::remove_file(&path).unwrap();
    }
    std::os::unix::fs::symlink(&target, &path).unwrap();
    write_code_file(dir.path(), &second, &scripted_random()).expect("write over a symlink");
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        line,
        "the write replaced the link, never wrote through it"
    );
    assert!(!fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(read_code_file(dir.path(), uid) == Ok(Some(second)));

    // Removal is idempotent.
    remove_code_file(dir.path());
    assert!(!path.exists());
    remove_code_file(dir.path());
    assert!(read_code_file(dir.path(), uid) == Ok(None));

    for (err, text) in [
        (EnrollError::Io, "enrollment code file unreadable"),
        (
            EnrollError::Insecure,
            "enrollment code file permissions are insecure",
        ),
        (EnrollError::Malformed, "enrollment code file malformed"),
    ] {
        assert_eq!(err.to_string(), text);
    }
}
