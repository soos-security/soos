//! Biometric store selection for the GUI (review findings CAM-08 / STO-12, GitHub #156).
//!
//! The GUI used to fall back silently to a key and a template store under the system
//! temporary directory whenever the root-owned system store was unreadable, so users could
//! believe they had enrolled for PAM while their template sat in a scratch directory. The
//! selection is now explicit:
//!
//! - [`GuiStore::System`]: the system store is directly accessible (root session).
//! - [`GuiStore::Polkit`]: the system store is not accessible; the GUI keeps **no** local
//!   store and lists, imports and deletes templates through `pkexec soos-enroll`.
//! - [`GuiStore::Developer`]: opt-in `--dev-store <DIR>`; templates stay in `DIR`, are never
//!   used by PAM, and a banner says so on every frame.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use soos_biometric_store::{BiometricStore, BiometricStoreError, MasterKey};

/// Master key file name inside a `--dev-store` directory.
pub const DEV_STORE_KEY_FILE: &str = "master.key";

/// Template directory name inside a `--dev-store` directory.
pub const DEV_STORE_BIOMETRICS_DIR: &str = "biometrics";

/// Store backing the GUI session.
#[derive(Debug)]
pub enum GuiStore {
    /// The root-owned system store, opened directly.
    System(Arc<BiometricStore>),
    /// Explicit developer store (`--dev-store <DIR>`), never used by PAM.
    Developer {
        /// Opened developer store.
        store: Arc<BiometricStore>,
        /// Developer store directory given on the command line.
        dir: PathBuf,
    },
    /// No local store: every template operation goes through `pkexec soos-enroll`.
    Polkit,
}

/// Error returned by [`resolve_gui_store`].
#[derive(Debug, thiserror::Error)]
pub enum GuiStoreError {
    /// `--dev-store` must be an absolute path.
    #[error("--dev-store must be an absolute path: {0}")]
    RelativeDevStore(PathBuf),
    /// The developer directory could not be created.
    #[error("cannot create the developer store directory: {0}")]
    Io(#[from] std::io::Error),
    /// The developer key or store could not be opened.
    #[error("cannot open the developer store: {0}")]
    Store(#[from] BiometricStoreError),
}

impl GuiStore {
    /// Store readable in-process, if any (`None` in [`GuiStore::Polkit`] mode).
    pub fn local(&self) -> Option<&Arc<BiometricStore>> {
        match self {
            Self::System(store) | Self::Developer { store, .. } => Some(store),
            Self::Polkit => None,
        }
    }

    /// Whether template operations must go through `pkexec soos-enroll`.
    pub fn uses_polkit(&self) -> bool {
        matches!(self, Self::Polkit)
    }

    /// Persistent warning banner for the developer mode (`None` otherwise).
    pub fn banner(&self) -> Option<String> {
        match self {
            Self::Developer { dir, .. } => Some(format!(
                "DEVELOPER STORE: templates are saved in {} and are NOT used by PAM. \
                 Restart without --dev-store to enroll into the system store.",
                dir.display()
            )),
            Self::System(_) | Self::Polkit => None,
        }
    }
}

/// Selects the GUI store without any implicit fallback location.
///
/// With `dev_store`, opens (creating if needed, mode `0700`) `<dir>/master.key` and
/// `<dir>/biometrics`. Otherwise opens the system key and store; when either is not
/// accessible (unprivileged session), returns [`GuiStore::Polkit`] and creates nothing.
///
/// # Errors
///
/// [`GuiStoreError`] only for an unusable `--dev-store` directory.
pub fn resolve_gui_store(
    key_file: &Path,
    biometrics_dir: &Path,
    dev_store: Option<&Path>,
) -> Result<GuiStore, GuiStoreError> {
    if let Some(dir) = dev_store {
        if !dir.is_absolute() {
            return Err(GuiStoreError::RelativeDevStore(dir.to_path_buf()));
        }
        create_private_dir(dir)?;
        let key = MasterKey::load_or_create(dir.join(DEV_STORE_KEY_FILE))?;
        let store = BiometricStore::new(dir.join(DEV_STORE_BIOMETRICS_DIR), key)?;
        return Ok(GuiStore::Developer {
            store: Arc::new(store),
            dir: dir.to_path_buf(),
        });
    }

    let opened = MasterKey::load_or_create(key_file)
        .and_then(|key| BiometricStore::new(biometrics_dir, key));
    match opened {
        Ok(store) => Ok(GuiStore::System(Arc::new(store))),
        Err(e) => {
            tracing::info!(
                "System biometric store not accessible from this session ({e}); \
                 template operations will use Polkit (pkexec soos-enroll)"
            );
            Ok(GuiStore::Polkit)
        }
    }
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}
