//! # soos-protocol
//!
//! Types IPC bornés et codec v1 pour la communication entre le module PAM
//! (`pam_soos.so`) et le démon soos (`soos-daemon`).
//!
//! ## Principes de conception
//!
//! - **Aucune I/O** : ce crate ne dépend ni de fichiers, ni de sockets, ni de réseau.
//! - **Borné** : la taille maximale d'un message sérialisé est de [`MAX_MESSAGE_SIZE`] octets.
//! - **Versionné** : chaque message contient un champ `version` pour la compatibilité.
//! - **Sûr** : `#![forbid(unsafe_code)]` est actif.
//!
//! ## Protocole v1
//!
//! ```text
//! Request v1:  version | kind=AUTH | request_id[32] | uid_hint:u32 |
//!              service_len:u8 | service[≤64] | deadline_monotonic_ns:u64
//! Response v1: version | request_id[32] | verdict:u8 | reason_class:u8 |
//!              issued_monotonic_ns:u64 | expires_monotonic_ns:u64
//! ```

#![forbid(unsafe_code)]
#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod codec;
pub mod types;

pub use codec::{decode, encode};
pub use types::{
    EventKind, ReasonClass, Request, RequestKind, Response, Verdict, CURRENT_VERSION,
    MAX_MESSAGE_SIZE, MAX_SERVICE_LEN, REQUEST_ID_LEN,
};
