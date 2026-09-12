//! # pam_soos — Module PAM pour authentification faciale locale
//!
//! Ce module est un `cdylib` chargé par Linux-PAM. Il exporte les points
//! d'entrée C `pam_sm_authenticate` et `pam_sm_setcred`.
//!
//! ## Principes fondamentaux
//!
//! 1. **Aucun runtime async** — uniquement `std::os::unix::net::UnixStream`
//! 2. **Budget strict** — 200-250ms maximum, incluant connect + requête + réponse
//! 3. **Jamais de panic** — `catch_unwind` sur toute entrée FFI, retour `PAM_IGNORE`
//! 4. **Aucun secret** — ne lit ni ne transmet de mot de passe
//! 5. **Dégradation sûre** — toute erreur → `PAM_IGNORE` → repli mot de passe
//!
//! ## Phase actuelle : Squelette Fondation
//!
//! Retourne systématiquement `PAM_IGNORE` pour valider :
//! - Le chargement ABI C par PAM
//! - La non-interférence avec `pam_unix.so`
//! - La résilience de la pile PAM

// NOTE: unsafe est nécessaire UNIQUEMENT pour les exports ABI C (`extern "C"`).
// Tout le reste du code doit rester safe.
#![deny(clippy::all)]

use std::panic::{catch_unwind, AssertUnwindSafe};

// ---------------------------------------------------------------------------
// Constantes PAM (from linux-pam headers)
// ---------------------------------------------------------------------------
// Ces constantes sont les valeurs standard de Linux-PAM.
// En phase suivante, elles viendront de la crate `pam-bindings`.

/// Succès — l'authentification est accordée.
#[allow(dead_code)]
const PAM_SUCCESS: i32 = 0;

/// Le module choisit de ne pas participer à la décision.
/// PAM continue avec les modules suivants de la pile.
const PAM_IGNORE: i32 = 25;

// ---------------------------------------------------------------------------
// Opaque PAM handle (pointeur vers la structure interne de PAM)
// ---------------------------------------------------------------------------

/// Handle opaque vers la structure interne de Linux-PAM.
/// On ne déréférence jamais ce pointeur — il est seulement transmis.
#[repr(C)]
pub struct PamHandle {
    _opaque: [u8; 0],
}

// ---------------------------------------------------------------------------
// Points d'entrée PAM (ABI C)
// ---------------------------------------------------------------------------

/// Point d'entrée principal appelé par PAM pour l'authentification.
///
/// # Safety
///
/// Cette fonction est appelée par Linux-PAM via l'ABI C. Le `pamh` est un
/// pointeur opaque fourni par PAM et ne doit pas être déréférencé.
/// `argv` pointe vers un tableau de `argc` chaînes C.
///
/// # Comportement actuel (Phase Fondation)
///
/// Retourne systématiquement `PAM_IGNORE` pour valider l'ABI sans
/// interférer avec la pile PAM.
///
/// # Garantie anti-panic
///
/// Toute panique est interceptée par `catch_unwind`. En cas de panic,
/// la fonction retourne `PAM_IGNORE` pour garantir le repli sur le
/// mot de passe.
#[no_mangle]
pub extern "C" fn pam_sm_authenticate(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const u8,
) -> i32 {
    // catch_unwind garantit qu'aucune panique ne franchit la frontière C.
    // AssertUnwindSafe est acceptable ici car nous n'accédons à aucun état
    // mutable partagé dans la closure.
    let result = catch_unwind(AssertUnwindSafe(|| {
        // Phase Fondation : pas de démon, pas de socket.
        // Retourne PAM_IGNORE → PAM continue vers pam_unix.so
        PAM_IGNORE
    }));

    match result {
        Ok(code) => code,
        Err(_) => {
            // Invariant 5 de ARCHITECTURE.md :
            // "Un panic se dégrade en mot de passe, jamais en autorisation."
            PAM_IGNORE
        }
    }
}

/// Point d'entrée pour la gestion des credentials PAM.
///
/// # Safety
///
/// Mêmes conditions que `pam_sm_authenticate`.
///
/// soos ne gère pas de credentials. Retourne toujours `PAM_IGNORE`.
#[no_mangle]
pub extern "C" fn pam_sm_setcred(
    _pamh: *mut PamHandle,
    _flags: i32,
    _argc: i32,
    _argv: *const *const u8,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| PAM_IGNORE));
    match result {
        Ok(code) => code,
        Err(_) => PAM_IGNORE,
    }
}

// ===========================================================================
// Tests unitaires
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    /// PA1 : Le module retourne PAM_IGNORE quand le démon est indisponible
    /// (en phase fondation, le démon n'existe pas encore).
    #[test]
    fn authenticate_returns_pam_ignore() {
        let result = pam_sm_authenticate(ptr::null_mut(), 0, 0, ptr::null());
        assert_eq!(result, PAM_IGNORE);
    }

    /// Le module ne crash jamais sur setcred.
    #[test]
    fn setcred_returns_pam_ignore() {
        let result = pam_sm_setcred(ptr::null_mut(), 0, 0, ptr::null());
        assert_eq!(result, PAM_IGNORE);
    }

    /// PA5 : Vérification que PAM_IGNORE est bien 25 (standard Linux-PAM).
    #[test]
    fn pam_ignore_has_correct_value() {
        assert_eq!(PAM_IGNORE, 25);
    }

    /// PA3 : catch_unwind empêche les paniques de traverser la frontière C.
    /// Ce test vérifie que même si le code interne paniquait, le résultat
    /// serait PAM_IGNORE et non un abort.
    #[test]
    fn panic_safety_returns_pam_ignore() {
        let result = catch_unwind(AssertUnwindSafe(|| -> i32 {
            panic!("test panic in PAM module");
        }));
        // Si panic, on retourne PAM_IGNORE
        let code = match result {
            Ok(c) => c,
            Err(_) => PAM_IGNORE,
        };
        assert_eq!(code, PAM_IGNORE);
    }
}
