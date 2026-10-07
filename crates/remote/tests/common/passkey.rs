//! Deterministic WebAuthn fixtures for the passkey contract tests (ADR 2026-10-06 "Tailscale
//! Funnel Access and In-House Passkey Authentication for `soos-remote`", architect spec §10).
//!
//! Everything here is built independently of the crate under test: P-256 keys come from
//! fixed seeds (`p256::ecdsa::SigningKey`, RFC 6979 deterministic signatures), CBOR from
//! `ciborium`, hashes from `sha2`, base64url from a local encoder. The store and code files
//! are written in the on-disk formats of spec §4.4 / §4.5 by hand, never through the crate.

#![allow(
    dead_code,
    reason = "Shared test fixtures library used conditionally across test modules"
)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use ciborium::value::Value;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use sha2::{Digest, Sha256};

/// WebAuthn flag bits (L3 §6.1), independent of the crate's `flags` module.
pub const UP: u8 = 0x01;
pub const UV: u8 = 0x04;
pub const BE: u8 = 0x08;
pub const BS: u8 = 0x10;
pub const AT: u8 = 0x40;
pub const ED: u8 = 0x80;

/// The verbatim WebAuthn L3 §16.2 vector ("ES256 Credential with No Attestation",
/// `rp_id = "example.org"`), hex encoded.
pub mod l3_16_2 {
    pub const RP_ID: &str = "example.org";
    pub const REG_CLIENT_DATA: &str = "7b2274797065223a22776562617574686e2e637265617465222c226368616c6c656e6765223a22414d4d507434557878475453746e63647134313759447742466938767049612d7077386f4f755657345441222c226f726967696e223a2268747470733a2f2f6578616d706c652e6f7267222c2263726f73734f726967696e223a66616c73652c22657874726144617461223a22636c69656e74446174614a534f4e206d617920626520657874656e6465642077697468206164646974696f6e616c206669656c647320696e20746865206675747572652c207375636820617320746869733a20426b5165446a646354427258426941774a544c453551227d";
    pub const ATTESTATION_OBJECT: &str = "a363666d74646e6f6e656761747453746d74a068617574684461746158a4bfabc37432958b063360d3ad6461c9c4735ae7f8edd46592a5e0f01452b2e4b559000000008446ccb9ab1db374750b2367ff6f3a1f0020f91f391db4c9b2fde0ea70189cba3fb63f579ba6122b33ad94ff3ec330084be4a5010203262001215820afefa16f97ca9b2d23eb86ccb64098d20db90856062eb249c33a9b672f26df61225820930a56b87a2fca66334b03458abf879717c12cc68ed73290af2e2664796b9220";
    pub const CREDENTIAL_ID: &str =
        "f91f391db4c9b2fde0ea70189cba3fb63f579ba6122b33ad94ff3ec330084be4";
    pub const PUBLIC_KEY_X: &str =
        "afefa16f97ca9b2d23eb86ccb64098d20db90856062eb249c33a9b672f26df61";
    pub const PUBLIC_KEY_Y: &str =
        "930a56b87a2fca66334b03458abf879717c12cc68ed73290af2e2664796b9220";
    pub const AUTHENTICATOR_DATA: &str =
        "bfabc37432958b063360d3ad6461c9c4735ae7f8edd46592a5e0f01452b2e4b51900000000";
    pub const GET_CLIENT_DATA: &str = "7b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a224f63446e55685158756c5455506f334a5558543049393770767a7a59425039745a63685879617630314167222c226f726967696e223a2268747470733a2f2f6578616d706c652e6f7267222c2263726f73734f726967696e223a66616c73657d";
    pub const SIGNATURE: &str = "3046022100f50a4e2e4409249c4a853ba361282f09841df4dd4547a13a87780218deffcd380221008480ac0f0b93538174f575bf11a1dd5d78c6e486013f937295ea13653e331e87";
}

pub fn hex(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2), "even hex length");
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex digit"))
        .collect()
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// base64url without padding (RFC 4648 §5), local reference implementation.
pub fn b64url(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let symbols = chunk.len() + 1;
        for i in 0..symbols {
            out.push(B64URL[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// Inverse of [`b64url`]; panics on anything else (test input only).
pub fn b64url_decode(text: &str) -> Vec<u8> {
    let mut bits = 0u32;
    let mut count = 0;
    let mut out = Vec::new();
    for c in text.bytes() {
        let v = B64URL
            .iter()
            .position(|s| *s == c)
            .unwrap_or_else(|| panic!("not base64url: {text:?}")) as u32;
        bits = (bits << 6) | v;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    out
}

/// `clientDataJSON` bytes: `type`, `challenge`, `origin`, `crossOrigin: false`, then `extra`
/// (raw JSON members, each starting with `,`), in this order.
pub fn client_data(kind: &str, challenge_b64: &str, origin: &str, extra: &str) -> Vec<u8> {
    format!(
        "{{\"type\":\"{kind}\",\"challenge\":\"{challenge_b64}\",\"origin\":\"{origin}\",\"crossOrigin\":false{extra}}}"
    )
    .into_bytes()
}

pub fn int(v: i64) -> Value {
    Value::Integer(v.into())
}

pub fn text(v: &str) -> Value {
    Value::Text(v.to_string())
}

pub fn cbor(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::ser::into_writer(value, &mut out).expect("CBOR encoding");
    out
}

/// A software passkey with a deterministic key (seed byte repeated 32 times).
#[derive(Clone)]
pub struct Authenticator {
    pub key: SigningKey,
    pub credential_id: Vec<u8>,
    /// Backup eligible (synced passkey) — also sets BS on every ceremony.
    pub synced: bool,
}

impl Authenticator {
    pub fn new(seed: u8, credential_id: &[u8], synced: bool) -> Self {
        assert!((1..=0x7f).contains(&seed), "valid P-256 scalar seed");
        Self {
            key: SigningKey::from_slice(&[seed; 32]).expect("valid scalar"),
            credential_id: credential_id.to_vec(),
            synced,
        }
    }

    /// The owner's synced (iCloud-like) passkey: BE|BS, counter always 0.
    pub fn owner() -> Self {
        Self::new(0x11, b"owner-passkey-credential-id-0001", true)
    }

    /// A device-bound passkey with a counter.
    pub fn device(seed: u8, id: &[u8]) -> Self {
        Self::new(seed, id, false)
    }

    /// SEC1 uncompressed point (65 bytes).
    pub fn public_key(&self) -> [u8; 65] {
        let point = self.key.verifying_key().to_encoded_point(false);
        point.as_bytes().try_into().expect("65-byte SEC1 point")
    }

    pub fn x(&self) -> Vec<u8> {
        self.public_key()[1..33].to_vec()
    }

    pub fn y(&self) -> Vec<u8> {
        self.public_key()[33..65].to_vec()
    }

    /// Canonical COSE_Key entries {1:2, 3:-7, -1:1, -2:x, -3:y}.
    pub fn cose_entries(&self) -> Vec<(Value, Value)> {
        vec![
            (int(1), int(2)),
            (int(3), int(-7)),
            (int(-1), int(1)),
            (int(-2), Value::Bytes(self.x())),
            (int(-3), Value::Bytes(self.y())),
        ]
    }

    pub fn cose_key(&self) -> Vec<u8> {
        cbor(&Value::Map(self.cose_entries()))
    }

    /// Default flags of a ceremony with user verification.
    pub fn flags(&self) -> u8 {
        if self.synced {
            UP | UV | BE | BS
        } else {
            UP | UV
        }
    }

    pub fn credential_id_b64(&self) -> String {
        b64url(&self.credential_id)
    }

    /// DER ES256 signature over `authenticator_data || SHA-256(client_data)`.
    pub fn sign(&self, authenticator_data: &[u8], client_data: &[u8]) -> Vec<u8> {
        let mut message = authenticator_data.to_vec();
        message.extend_from_slice(&sha256(client_data));
        let signature: Signature = self.key.sign(&message);
        signature.to_der().as_bytes().to_vec()
    }
}

/// The same ECDSA signature with `s` replaced by `n - s` (one of the two is high-S).
pub fn s_complement(der: &[u8]) -> Vec<u8> {
    let signature = Signature::from_der(der).expect("DER signature");
    let (r, s) = signature.split_scalars();
    let negated = -s;
    Signature::from_scalars(r, negated)
        .expect("valid scalars")
        .to_der()
        .as_bytes()
        .to_vec()
}

/// Registration `authenticatorData`: rpIdHash | flags | signCount | AAGUID (zero) |
/// credentialIdLength | credentialId | `cose_key`.
pub fn registration_auth_data(
    rp_id: &str,
    flags: u8,
    sign_count: u32,
    credential_id: &[u8],
    cose_key: &[u8],
) -> Vec<u8> {
    let mut out = sha256(rp_id.as_bytes()).to_vec();
    out.push(flags);
    out.extend_from_slice(&sign_count.to_be_bytes());
    out.extend_from_slice(&[0u8; 16]);
    out.extend_from_slice(&(credential_id.len() as u16).to_be_bytes());
    out.extend_from_slice(credential_id);
    out.extend_from_slice(cose_key);
    out
}

/// `attestationObject` = CBOR map of the given top-level entries.
pub fn attestation_from_entries(entries: Vec<(Value, Value)>) -> Vec<u8> {
    cbor(&Value::Map(entries))
}

/// `{"fmt":"none","attStmt":{},"authData":auth_data}`.
pub fn none_attestation(auth_data: &[u8]) -> Vec<u8> {
    attestation_from_entries(vec![
        (text("fmt"), text("none")),
        (text("attStmt"), Value::Map(Vec::new())),
        (text("authData"), Value::Bytes(auth_data.to_vec())),
    ])
}

/// A valid `none` registration of `auth` for `rp_id` with `flags` (UP|UV|AT expected).
pub fn registration(auth: &Authenticator, rp_id: &str, flags: u8) -> Vec<u8> {
    none_attestation(&registration_auth_data(
        rp_id,
        flags,
        0,
        &auth.credential_id,
        &auth.cose_key(),
    ))
}

/// Assertion `authenticatorData` (37 bytes).
pub fn assertion_auth_data(rp_id: &str, flags: u8, sign_count: u32) -> Vec<u8> {
    let mut out = sha256(rp_id.as_bytes()).to_vec();
    out.push(flags);
    out.extend_from_slice(&sign_count.to_be_bytes());
    out
}

/// JSON body of login verify and unlock (`AssertionBody`, spec §4.6).
pub fn assertion_body(
    id: &[u8],
    client_data_json: &[u8],
    authenticator_data: &[u8],
    signature: &[u8],
    user_handle: Option<&[u8]>,
) -> String {
    let user_handle = match user_handle {
        Some(handle) => format!(",\"user_handle\":\"{}\"", b64url(handle)),
        None => String::new(),
    };
    format!(
        "{{\"id\":\"{}\",\"client_data_json\":\"{}\",\"authenticator_data\":\"{}\",\"signature\":\"{}\"{user_handle}}}",
        b64url(id),
        b64url(client_data_json),
        b64url(authenticator_data),
        b64url(signature)
    )
}

/// JSON body of register verify (`RegisterVerifyBody`, spec §4.6).
pub fn registration_body(id: &[u8], client_data_json: &[u8], attestation_object: &[u8]) -> String {
    format!(
        "{{\"id\":\"{}\",\"client_data_json\":\"{}\",\"attestation_object\":\"{}\"}}",
        b64url(id),
        b64url(client_data_json),
        b64url(attestation_object)
    )
}

/// What a fixture puts into one store record.
#[derive(Clone)]
pub struct StoredPasskey {
    pub auth: Authenticator,
    pub sign_count: u32,
    pub created_unix_s: u64,
}

impl StoredPasskey {
    pub fn of(auth: &Authenticator) -> Self {
        Self {
            auth: auth.clone(),
            sign_count: 0,
            created_unix_s: 1_700_000_000,
        }
    }
}

/// The store file text of spec §4.4 (version 1).
pub fn store_json(user_handle: &[u8], passkeys: &[StoredPasskey]) -> String {
    let records: Vec<String> = passkeys
        .iter()
        .map(|p| {
            format!(
                "{{\"credential_id\":\"{}\",\"public_key\":\"{}\",\"sign_count\":{},\"backup_eligible\":{},\"backup_state\":{},\"created_unix_s\":{}}}",
                b64url(&p.auth.credential_id),
                b64url(&p.auth.public_key()),
                p.sign_count,
                p.auth.synced,
                p.auth.synced,
                p.created_unix_s
            )
        })
        .collect();
    format!(
        "{{\"version\":1,\"user_handle\":\"{}\",\"passkeys\":[{}]}}",
        b64url(user_handle),
        records.join(",")
    )
}

/// Writes `bytes` to `path` with `mode` (created or truncated, mode forced).
pub fn write_file_mode(path: &Path, bytes: &[u8], mode: u32) {
    let _ = fs::remove_file(path);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .expect("create fixture file");
    file.write_all(bytes).expect("write fixture file");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod fixture file");
}

/// The enrollment code file line of spec §4.5.
pub fn code_file_line(code_hash: &[u8; 32], expires_unix_s: u64) -> String {
    format!(
        "soos-remote-enroll-v1 {} {expires_unix_s}\n",
        to_hex(code_hash)
    )
}

/// SHA-256 of a normalized code (uppercase, no dash), the reference for `normalized_code_hash`.
pub fn code_hash(normalized: &str) -> [u8; 32] {
    sha256(normalized.as_bytes())
}

/// The process uid (owner of every fixture file).
pub fn own_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}
