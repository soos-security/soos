//! One-time enrollment codes: generation, normalisation, hashing, code file (architect spec
//! §4.5, owner decision D-E).
//!
//! `soos-remote enroll-code` draws 10 Crockford base32 symbols (50 bits) from the CSPRNG,
//! prints them once and writes only their SHA-256 with an expiry to
//! `<socket dir>/enroll-code` (`0600`, atomic, never through a symlink). The running service
//! reads that file to accept a registration from the tailnet. The plaintext code is
//! zeroized on drop and never logged. No clock is read here (R3-1): expiry is decided by the
//! caller.

use std::fmt;
use std::path::Path;

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::auth::{RandomError, RandomSource};
use crate::credentials::{read_owned_file, write_atomic, StoreError};
use crate::{
    ENROLL_CODE_ALPHABET, ENROLL_CODE_FILE_NAME, ENROLL_CODE_LEN, MAX_ENROLL_CODE_FILE_BYTES,
};

/// Longest raw input accepted by [`normalized_code_hash`].
const MAX_CODE_INPUT_BYTES: usize = 32;
/// First token of a code file line.
const CODE_FILE_MAGIC: &str = "soos-remote-enroll-v1";

/// A plaintext enrollment code (CLI output only); zeroized on drop, `Debug` redacted.
pub struct EnrollCode(Zeroizing<String>);

impl fmt::Debug for EnrollCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EnrollCode(<redacted>)")
    }
}

impl EnrollCode {
    /// Uniform symbols by rejection-free mapping: each random byte `& 31` indexes the
    /// 32-symbol alphabet.
    ///
    /// # Errors
    ///
    /// [`RandomError`].
    pub fn generate(random: &RandomSource) -> Result<Self, RandomError> {
        let mut bytes = Zeroizing::new([0u8; ENROLL_CODE_LEN]);
        random(bytes.as_mut_slice())?;
        let mut code = Zeroizing::new(String::with_capacity(ENROLL_CODE_LEN));
        for byte in bytes.iter() {
            let symbol = ENROLL_CODE_ALPHABET
                .get(usize::from(byte & 31))
                .copied()
                .ok_or(RandomError::Failed)?;
            code.push(char::from(symbol));
        }
        Ok(Self(code))
    }

    /// `XXXXX-XXXXX`.
    #[must_use]
    pub fn display(&self) -> String {
        let half = ENROLL_CODE_LEN / 2;
        let first = self.0.get(..half).unwrap_or_default();
        let second = self.0.get(half..).unwrap_or_default();
        format!("{first}-{second}")
    }

    /// SHA-256 of the normalized code.
    #[must_use]
    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.0.as_bytes()).into()
    }
}

/// Pure. Uppercases and removes `-` and ASCII spaces; the rest must be exactly
/// `ENROLL_CODE_LEN` symbols of the alphabet; returns SHA-256 of the normalized code. An
/// input over 32 bytes is refused.
#[must_use]
pub fn normalized_code_hash(input: &str) -> Option<[u8; 32]> {
    if input.len() > MAX_CODE_INPUT_BYTES {
        return None;
    }
    let mut normalized = Zeroizing::new(String::with_capacity(ENROLL_CODE_LEN));
    for c in input.chars() {
        if c == '-' || c == ' ' {
            continue;
        }
        let upper = c.to_ascii_uppercase();
        if !upper.is_ascii() || !ENROLL_CODE_ALPHABET.contains(&u8::try_from(upper).ok()?) {
            return None;
        }
        normalized.push(upper);
    }
    if normalized.len() != ENROLL_CODE_LEN {
        return None;
    }
    Some(Sha256::digest(normalized.as_bytes()).into())
}

/// Parsed code file; `Debug` redacted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CodeFile {
    /// SHA-256 of the normalized code.
    pub code_hash: [u8; 32],
    /// Expiry (Unix s).
    pub expires_unix_s: u64,
}

impl fmt::Debug for CodeFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CodeFile(<redacted>)")
    }
}

/// `"soos-remote-enroll-v1 <64 lowercase hex> <expires_unix_s>\n"`.
#[must_use]
pub fn encode_code_file(file: &CodeFile) -> String {
    let hex: String = file.code_hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("{CODE_FILE_MAGIC} {hex} {}\n", file.expires_unix_s)
}

/// One lowercase hex digit.
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => byte.checked_sub(b'0'),
        b'a'..=b'f' => byte.checked_sub(b'a').and_then(|v| v.checked_add(10)),
        _ => None,
    }
}

/// Pure; the exact line of [`encode_code_file`], at most `MAX_ENROLL_CODE_FILE_BYTES`.
///
/// # Errors
///
/// [`EnrollError::Malformed`].
pub fn parse_code_file(bytes: &[u8]) -> Result<CodeFile, EnrollError> {
    if bytes.len() > MAX_ENROLL_CODE_FILE_BYTES {
        return Err(EnrollError::Malformed);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| EnrollError::Malformed)?;
    let line = text.strip_suffix('\n').ok_or(EnrollError::Malformed)?;
    let mut parts = line.split(' ');
    let (Some(magic), Some(hex), Some(expiry), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(EnrollError::Malformed);
    };
    if magic != CODE_FILE_MAGIC || hex.len() != 64 {
        return Err(EnrollError::Malformed);
    }
    let mut code_hash = [0u8; 32];
    let (pairs, rest) = hex.as_bytes().as_chunks::<2>();
    if !rest.is_empty() {
        return Err(EnrollError::Malformed);
    }
    for (slot, [high, low]) in code_hash.iter_mut().zip(pairs) {
        let (Some(high), Some(low)) = (hex_value(*high), hex_value(*low)) else {
            return Err(EnrollError::Malformed);
        };
        *slot = high
            .checked_mul(16)
            .and_then(|h| h.checked_add(low))
            .ok_or(EnrollError::Malformed)?;
    }
    if expiry.is_empty() || !expiry.bytes().all(|b| b.is_ascii_digit()) {
        return Err(EnrollError::Malformed);
    }
    let expires_unix_s = expiry.parse::<u64>().map_err(|_| EnrollError::Malformed)?;
    Ok(CodeFile {
        code_hash,
        expires_unix_s,
    })
}

/// Writes `<socket_dir>/ENROLL_CODE_FILE_NAME` atomically (temp file + rename, `0600`,
/// `O_NOFOLLOW`), replacing any previous code or symlink. `socket_dir` must already be the
/// prepared `0700` socket directory.
///
/// # Errors
///
/// [`EnrollError::Io`].
pub fn write_code_file(
    socket_dir: &Path,
    file: &CodeFile,
    random: &RandomSource,
) -> Result<(), EnrollError> {
    let line = encode_code_file(file);
    write_atomic(
        &socket_dir.join(ENROLL_CODE_FILE_NAME),
        socket_dir,
        line.as_bytes(),
        random,
    )
    .map_err(|_| EnrollError::Io)
}

/// Reads the code file with the store's rules (regular file, owner `uid`, `mode & 0o077 ==
/// 0`, at most `MAX_ENROLL_CODE_FILE_BYTES`, `O_NOFOLLOW | O_NONBLOCK`). Missing ⇒ `Ok(None)`.
///
/// # Errors
///
/// [`EnrollError::Io`], [`EnrollError::Insecure`], [`EnrollError::Malformed`].
pub fn read_code_file(socket_dir: &Path, uid: u32) -> Result<Option<CodeFile>, EnrollError> {
    let path = socket_dir.join(ENROLL_CODE_FILE_NAME);
    match read_owned_file(&path, uid, MAX_ENROLL_CODE_FILE_BYTES) {
        Ok(None) => Ok(None),
        Ok(Some((bytes, _))) => parse_code_file(&bytes).map(Some),
        Err(StoreError::Insecure) => Err(EnrollError::Insecure),
        Err(StoreError::TooLarge | StoreError::Malformed) => Err(EnrollError::Malformed),
        Err(_) => Err(EnrollError::Io),
    }
}

/// Removes the code file only when it is a regular file (a symlink is never followed nor
/// removed); idempotent.
pub fn remove_code_file(socket_dir: &Path) {
    let path = socket_dir.join(ENROLL_CODE_FILE_NAME);
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file()) {
        let _ = std::fs::remove_file(&path);
    }
}

/// Code file failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EnrollError {
    /// The file cannot be read or written.
    #[error("enrollment code file unreadable")]
    Io,
    /// Owner, mode, file type or a symlink.
    #[error("enrollment code file permissions are insecure")]
    Insecure,
    /// Not the expected line.
    #[error("enrollment code file malformed")]
    Malformed,
}
