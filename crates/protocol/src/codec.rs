//! Codec binaire borné pour le protocole IPC v1.
//!
//! Utilise `postcard` (serde binaire compact) pour la sérialisation.
//! Chaque message est préfixé par sa taille en `u32` big-endian pour
//! le framing sur `SOCK_STREAM`. Sur `SOCK_SEQPACKET`, le préfixe
//! est redondant mais conservé pour la cohérence.
//!
//! # Sécurité
//!
//! - La taille maximale est vérifiée AVANT toute désérialisation.
//! - Un message trop grand est rejeté sans allocation.
//! - Le codec ne fait aucune I/O : il opère sur des slices `&[u8]`.

use crate::types::MAX_MESSAGE_SIZE;
use serde::{de::DeserializeOwned, Serialize};

// ---------------------------------------------------------------------------
// Erreurs de codec
// ---------------------------------------------------------------------------

/// Erreurs de sérialisation/désérialisation du protocole.
#[derive(Debug)]
pub enum CodecError {
    /// Le message sérialisé dépasse la taille maximale autorisée.
    MessageTooLarge { size: usize, max: usize },
    /// Erreur de sérialisation postcard.
    Serialize(postcard::Error),
    /// Erreur de désérialisation postcard.
    Deserialize(postcard::Error),
    /// Le buffer fourni est trop petit pour contenir le préfixe de taille.
    BufferTooSmall,
    /// La taille déclarée dans le préfixe dépasse la taille maximale.
    DeclaredSizeTooLarge { declared: usize, max: usize },
}

impl core::fmt::Display for CodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MessageTooLarge { size, max } => {
                write!(f, "message too large: {size} bytes (max {max})")
            }
            Self::Serialize(e) => write!(f, "serialization error: {e}"),
            Self::Deserialize(e) => write!(f, "deserialization error: {e}"),
            Self::BufferTooSmall => write!(f, "buffer too small for size prefix"),
            Self::DeclaredSizeTooLarge { declared, max } => {
                write!(
                    f,
                    "declared message size too large: {declared} bytes (max {max})"
                )
            }
        }
    }
}

impl std::error::Error for CodecError {}

// ---------------------------------------------------------------------------
// Encodage
// ---------------------------------------------------------------------------

/// Sérialise un message en bytes avec préfixe de taille `u32` big-endian.
///
/// Le format du buffer résultant est :
/// ```text
/// [taille: u32 BE][payload: postcard bytes]
/// ```
///
/// # Errors
///
/// Retourne `CodecError::MessageTooLarge` si le payload sérialisé
/// dépasse [`MAX_MESSAGE_SIZE`] octets.
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, CodecError> {
    let payload = postcard::to_allocvec(msg).map_err(CodecError::Serialize)?;

    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(CodecError::MessageTooLarge {
            size: payload.len(),
            max: MAX_MESSAGE_SIZE,
        });
    }

    // Préfixe u32 big-endian + payload
    let size_prefix = u32::try_from(payload.len())
        .map_err(|_| CodecError::MessageTooLarge {
            size: payload.len(),
            max: MAX_MESSAGE_SIZE,
        })?
        .to_be_bytes();

    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&size_prefix);
    buf.extend_from_slice(&payload);
    Ok(buf)
}

// ---------------------------------------------------------------------------
// Décodage
// ---------------------------------------------------------------------------

/// Désérialise un message depuis un buffer avec préfixe de taille `u32` big-endian.
///
/// # Sécurité
///
/// - Vérifie que la taille déclarée ne dépasse pas [`MAX_MESSAGE_SIZE`] AVANT
///   toute allocation ou désérialisation.
/// - Un buffer malformé ne peut pas provoquer d'allocation non bornée.
///
/// # Errors
///
/// - `BufferTooSmall` si le buffer fait moins de 4 octets
/// - `DeclaredSizeTooLarge` si le préfixe annonce plus de [`MAX_MESSAGE_SIZE`]
/// - `Deserialize` si le payload est invalide
pub fn decode<T: DeserializeOwned>(buf: &[u8]) -> Result<T, CodecError> {
    if buf.len() < 4 {
        return Err(CodecError::BufferTooSmall);
    }

    let size_bytes: [u8; 4] = buf[..4]
        .try_into()
        .map_err(|_| CodecError::BufferTooSmall)?;
    let declared_size = u32::from_be_bytes(size_bytes) as usize;

    if declared_size > MAX_MESSAGE_SIZE {
        return Err(CodecError::DeclaredSizeTooLarge {
            declared: declared_size,
            max: MAX_MESSAGE_SIZE,
        });
    }

    let payload_end = 4 + declared_size;
    if buf.len() < payload_end {
        return Err(CodecError::BufferTooSmall);
    }

    postcard::from_bytes(&buf[4..payload_end]).map_err(CodecError::Deserialize)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    /// Helper : crée une requête de test valide.
    fn test_request() -> Request {
        Request {
            version: CURRENT_VERSION,
            kind: RequestKind::Auth,
            request_id: [0xAB; REQUEST_ID_LEN],
            uid_hint: 1000,
            service: "sudo".to_string(),
            deadline_monotonic_ns: 250_000_000,
        }
    }

    /// Helper : crée une réponse de test valide.
    fn test_response(verdict: Verdict) -> Response {
        Response {
            version: CURRENT_VERSION,
            request_id: [0xAB; REQUEST_ID_LEN],
            verdict,
            reason_class: ReasonClass::FaceMatch,
            issued_monotonic_ns: 100_000_000,
            expires_monotonic_ns: 102_000_000_000,
        }
    }

    // --- P1 : Round-trip sérialisation/désérialisation ---

    #[test]
    fn request_roundtrip() {
        let req = test_request();
        let encoded = encode(&req).expect("encode should succeed");
        let decoded: Request = decode(&encoded).expect("decode should succeed");

        assert_eq!(decoded.version, req.version);
        assert_eq!(decoded.kind, req.kind);
        assert_eq!(decoded.request_id, req.request_id);
        assert_eq!(decoded.uid_hint, req.uid_hint);
        assert_eq!(decoded.service, req.service);
        assert_eq!(decoded.deadline_monotonic_ns, req.deadline_monotonic_ns);
    }

    #[test]
    fn response_roundtrip() {
        let resp = test_response(Verdict::Allow);
        let encoded = encode(&resp).expect("encode should succeed");
        let decoded: Response = decode(&encoded).expect("decode should succeed");

        assert_eq!(decoded.version, resp.version);
        assert_eq!(decoded.request_id, resp.request_id);
        assert_eq!(decoded.verdict, resp.verdict);
        assert_eq!(decoded.reason_class, resp.reason_class);
        assert_eq!(decoded.issued_monotonic_ns, resp.issued_monotonic_ns);
        assert_eq!(decoded.expires_monotonic_ns, resp.expires_monotonic_ns);
    }

    #[test]
    fn event_roundtrip() {
        let evt = Event {
            version: CURRENT_VERSION,
            kind: EventKind::PasswordFailed,
            request_id: Some([0xCD; REQUEST_ID_LEN]),
            service: "gdm".to_string(),
            timestamp_monotonic_ns: 500_000_000,
        };
        let encoded = encode(&evt).expect("encode should succeed");
        let decoded: Event = decode(&encoded).expect("decode should succeed");

        assert_eq!(decoded.version, evt.version);
        assert_eq!(decoded.kind, evt.kind);
        assert_eq!(decoded.request_id, evt.request_id);
        assert_eq!(decoded.service, evt.service);
    }

    // --- P2 : Rejet des messages trop grands ---

    #[test]
    fn rejects_oversized_service_name() {
        let req = Request {
            version: CURRENT_VERSION,
            kind: RequestKind::Auth,
            request_id: [0; REQUEST_ID_LEN],
            uid_hint: 1000,
            service: "a".repeat(MAX_SERVICE_LEN + 1),
            deadline_monotonic_ns: 0,
        };
        // La validation doit rejeter un service trop long
        assert!(req.validate().is_err());
    }

    #[test]
    fn rejects_declared_size_too_large() {
        // Forge un buffer avec un préfixe de taille énorme
        let mut buf = vec![0u8; 8];
        #[allow(clippy::cast_possible_truncation)]
        let fake_size: u32 = (MAX_MESSAGE_SIZE as u32) + 1;
        buf[..4].copy_from_slice(&fake_size.to_be_bytes());

        let result: Result<Request, _> = decode(&buf);
        assert!(result.is_err());
    }

    // --- P3 : request_id est bien 32 octets ---

    #[test]
    fn request_id_is_256_bits() {
        assert_eq!(REQUEST_ID_LEN, 32);
        let req = test_request();
        assert_eq!(req.request_id.len(), 32);
    }

    // --- P4 : Protocole versionné ---

    #[test]
    fn rejects_unsupported_version() {
        let req = Request {
            version: 99,
            kind: RequestKind::Auth,
            request_id: [0; REQUEST_ID_LEN],
            uid_hint: 1000,
            service: "test".to_string(),
            deadline_monotonic_ns: 0,
        };
        assert!(req.validate().is_err());
        if let Err(ValidationError::UnsupportedVersion { version }) = req.validate() {
            assert_eq!(version, 99);
        } else {
            panic!("expected UnsupportedVersion error");
        }
    }

    // --- Tests de verdict ---

    #[test]
    fn verdict_allow_does_not_ignore() {
        assert!(!Verdict::Allow.should_ignore());
    }

    #[test]
    fn verdict_deny_should_ignore() {
        assert!(Verdict::Deny.should_ignore());
    }

    #[test]
    fn verdict_unavailable_should_ignore() {
        assert!(Verdict::Unavailable.should_ignore());
    }

    #[test]
    fn verdict_protocol_error_should_ignore() {
        assert!(Verdict::ProtocolError.should_ignore());
    }

    // --- Tests de Response ---

    #[test]
    fn response_is_allow_only_with_current_version() {
        let resp = test_response(Verdict::Allow);
        assert!(resp.is_allow());

        let mut bad_version = test_response(Verdict::Allow);
        bad_version.version = 99;
        assert!(!bad_version.is_allow());
    }

    #[test]
    fn response_matches_request_id() {
        let resp = test_response(Verdict::Allow);
        let matching_id = [0xAB; REQUEST_ID_LEN];
        let wrong_id = [0x00; REQUEST_ID_LEN];

        assert!(resp.matches_request(&matching_id));
        assert!(!resp.matches_request(&wrong_id));
    }

    // --- Test de buffer malformé ---

    #[test]
    fn decode_empty_buffer_fails() {
        let result: Result<Request, _> = decode(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn decode_short_buffer_fails() {
        let result: Result<Request, _> = decode(&[0, 0, 0]);
        assert!(result.is_err());
    }

    #[test]
    fn decode_truncated_payload_fails() {
        let req = test_request();
        let encoded = encode(&req).expect("encode should succeed");
        // Tronquer le payload
        let truncated = &encoded[..encoded.len() - 5];
        let result: Result<Request, _> = decode(truncated);
        assert!(result.is_err());
    }
}
