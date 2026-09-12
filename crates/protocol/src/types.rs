//! Types du protocole IPC v1 entre le module PAM et le démon soos.
//!
//! Tous les types sont bornés en taille et vérifiés à la construction.
//! Le `request_id` est un identifiant aléatoire de 256 bits (32 octets)
//! qui lie une requête à sa réponse et empêche le rejeu logique.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Constantes du protocole
// ---------------------------------------------------------------------------

/// Version courante du protocole.
pub const CURRENT_VERSION: u8 = 1;

/// Taille maximale d'un message sérialisé en octets.
/// Au-delà de cette taille, le message est rejeté AVANT désérialisation.
pub const MAX_MESSAGE_SIZE: usize = 4096;

/// Longueur du `request_id` en octets (256 bits).
pub const REQUEST_ID_LEN: usize = 32;

/// Longueur maximale du nom de service PAM en octets.
pub const MAX_SERVICE_LEN: usize = 64;

// ---------------------------------------------------------------------------
// Types de requête
// ---------------------------------------------------------------------------

/// Identifiant unique d'une requête d'authentification.
/// Généré par `getrandom`, non réutilisé, lié à la réponse.
pub type RequestId = [u8; REQUEST_ID_LEN];

/// Type de requête envoyée par le module PAM au démon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum RequestKind {
    /// Demande d'authentification faciale.
    Auth = 0,
}

/// Type d'événement notifié par le module PAM au démon (best-effort).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum EventKind {
    /// L'authentification par mot de passe a échoué.
    /// Déclenche potentiellement la capture de preuve (opt-in).
    PasswordFailed = 0,
}

/// Requête d'authentification envoyée par le module PAM.
///
/// Le `uid_hint` n'est qu'une assertion de cohérence : l'UID faisant
/// autorité est celui de `SO_PEERCRED` lu par le démon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Version du protocole.
    pub version: u8,
    /// Type de requête.
    pub kind: RequestKind,
    /// Identifiant unique de la requête (256 bits, `getrandom`).
    pub request_id: RequestId,
    /// UID déclaré par le client PAM (vérifié contre `SO_PEERCRED`).
    pub uid_hint: u32,
    /// Nom du service PAM (ex: "gdm", "sudo", "login"). Borné à 64 octets.
    pub service: String,
    /// Deadline monotone en nanosecondes. Au-delà, le démon doit répondre
    /// `Unavailable` même si le traitement n'est pas terminé.
    pub deadline_monotonic_ns: u64,
}

/// Événement notifié au démon après un échec d'authentification classique.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Version du protocole.
    pub version: u8,
    /// Type d'événement.
    pub kind: EventKind,
    /// Identifiant de la requête d'authentification faciale associée (si applicable).
    pub request_id: Option<RequestId>,
    /// Nom du service PAM.
    pub service: String,
    /// Timestamp monotone en nanosecondes.
    pub timestamp_monotonic_ns: u64,
}

// ---------------------------------------------------------------------------
// Types de réponse
// ---------------------------------------------------------------------------

/// Verdict rendu par le démon après analyse faciale.
///
/// `Deny` et `Unavailable` sont volontairement indiscernables pour le module PAM
/// (les deux mènent à `PAM_IGNORE`). Le démon peut les distinguer pour
/// l'observabilité interne.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Verdict {
    /// Visage unique, PAD acceptable, score ≥ seuil, contexte valide.
    Allow = 0,
    /// Pas de visage, plusieurs visages, score faible, PAD négatif.
    Deny = 1,
    /// Caméra/modèle/socket non prêt, délai dépassé, erreur interne.
    Unavailable = 2,
    /// Requête malformée, UID incohérent, rate-limit atteint.
    ProtocolError = 3,
}

/// Classe de raison du verdict (pour observabilité interne du démon).
/// Ne doit JAMAIS être exposée au module PAM ni aux logs accessibles utilisateur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ReasonClass {
    /// Authentification faciale réussie.
    FaceMatch = 0,
    /// Aucun visage détecté dans la frame.
    NoFace = 1,
    /// Plusieurs visages détectés.
    MultipleFaces = 2,
    /// Score de similarité sous le seuil.
    ScoreBelowThreshold = 3,
    /// PAD (Presentation Attack Detection) négatif.
    PadFailed = 4,
    /// Caméra indisponible ou non prête.
    CameraUnavailable = 5,
    /// Modèle ONNX non chargé ou invalide.
    ModelUnavailable = 6,
    /// Frame trop ancienne (> 150ms).
    StaleFrame = 7,
    /// Deadline dépassée.
    Timeout = 8,
    /// Rate limit atteint pour cet UID.
    RateLimited = 9,
    /// UID incohérent entre requête et `SO_PEERCRED`.
    UidMismatch = 10,
    /// Requête malformée ou version non supportée.
    MalformedRequest = 11,
    /// Erreur interne non classifiée.
    InternalError = 12,
}

/// Réponse du démon au module PAM.
///
/// Mono-usage : liée au `request_id`, à l'UID, au service et à une
/// échéance courte. Jamais mise en cache côté PAM.
///
/// Implémente `Zeroize` manuellement car les enums `Verdict` et `ReasonClass`
/// ne supportent pas le derive automatique (pas de `DefaultIsZeroes`).
/// À la destruction, les champs sensibles sont effacés et les enums
/// sont ramenés à des valeurs sûres (`Deny` / `InternalError`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// Version du protocole.
    pub version: u8,
    /// Identifiant de la requête à laquelle cette réponse correspond.
    pub request_id: RequestId,
    /// Verdict rendu par le démon.
    pub verdict: Verdict,
    /// Classe de raison (observabilité interne).
    pub reason_class: ReasonClass,
    /// Timestamp monotone d'émission (nanosecondes).
    pub issued_monotonic_ns: u64,
    /// Timestamp monotone d'expiration (nanosecondes).
    /// Typiquement 1-2 secondes après `issued_monotonic_ns`.
    pub expires_monotonic_ns: u64,
}

impl zeroize::Zeroize for Response {
    fn zeroize(&mut self) {
        self.version.zeroize();
        self.request_id.zeroize();
        // Les enums sans champs ne peuvent pas être "zeroized" au sens binaire.
        // On les ramène à des valeurs sûres (non-Allow) pour garantir qu'une
        // réponse effacée ne puisse jamais être interprétée comme une autorisation.
        self.verdict = Verdict::Deny;
        self.reason_class = ReasonClass::InternalError;
        self.issued_monotonic_ns.zeroize();
        self.expires_monotonic_ns.zeroize();
    }
}

impl Drop for Response {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.zeroize();
    }
}

// ---------------------------------------------------------------------------
// Validations
// ---------------------------------------------------------------------------

/// Erreurs de validation des messages du protocole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Le nom du service dépasse la taille maximale autorisée.
    ServiceTooLong { len: usize, max: usize },
    /// La version du protocole n'est pas supportée.
    UnsupportedVersion { version: u8 },
}

impl core::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ServiceTooLong { len, max } => {
                write!(f, "service name too long: {len} bytes (max {max})")
            }
            Self::UnsupportedVersion { version } => {
                write!(f, "unsupported protocol version: {version}")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

impl Request {
    /// Valide la requête selon les contraintes du protocole.
    ///
    /// # Errors
    ///
    /// Retourne `ValidationError` si :
    /// - Le nom du service dépasse [`MAX_SERVICE_LEN`] octets
    /// - La version du protocole n'est pas [`CURRENT_VERSION`]
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.version != CURRENT_VERSION {
            return Err(ValidationError::UnsupportedVersion {
                version: self.version,
            });
        }
        if self.service.len() > MAX_SERVICE_LEN {
            return Err(ValidationError::ServiceTooLong {
                len: self.service.len(),
                max: MAX_SERVICE_LEN,
            });
        }
        Ok(())
    }
}

impl Response {
    /// Vérifie si cette réponse autorise l'authentification.
    ///
    /// Seul un verdict `Allow` avec la version courante est considéré
    /// comme une autorisation valide.
    #[must_use]
    pub fn is_allow(&self) -> bool {
        self.version == CURRENT_VERSION && self.verdict == Verdict::Allow
    }

    /// Vérifie si la réponse correspond à la requête donnée.
    #[must_use]
    pub fn matches_request(&self, request_id: &RequestId) -> bool {
        self.request_id == *request_id
    }
}

impl Verdict {
    /// Retourne `true` si le verdict doit mener à `PAM_IGNORE`.
    ///
    /// Tous les verdicts sauf `Allow` mènent à `PAM_IGNORE`.
    #[must_use]
    pub fn should_ignore(self) -> bool {
        self != Self::Allow
    }
}
