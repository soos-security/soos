//! Embedded web assets (spec D11).
//!
//! Every file under `crates/remote/assets/` is embedded at compile time and served from the
//! same origin under a strict CSP; nothing is loaded from the network or the file system
//! at run time.

/// Identifier of an embedded asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetId {
    /// `/` and `/index.html`.
    Index,
    /// `/app.js`.
    AppJs,
    /// `/style.css`.
    StyleCss,
    /// `/manifest.webmanifest`.
    Manifest,
    /// `/icon.svg`.
    IconSvg,
    /// `/apple-touch-icon.png`.
    AppleTouchIcon,
}

/// One embedded asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    /// `Content-Type` value.
    pub content_type: &'static str,
    /// File bytes.
    pub body: &'static [u8],
}

/// The embedded asset for `id`.
#[must_use]
pub fn asset(id: AssetId) -> Asset {
    match id {
        AssetId::Index => Asset {
            content_type: "text/html; charset=utf-8",
            body: include_bytes!("../assets/index.html"),
        },
        AssetId::AppJs => Asset {
            content_type: "text/javascript; charset=utf-8",
            body: include_bytes!("../assets/app.js"),
        },
        AssetId::StyleCss => Asset {
            content_type: "text/css; charset=utf-8",
            body: include_bytes!("../assets/style.css"),
        },
        AssetId::Manifest => Asset {
            content_type: "application/manifest+json",
            body: include_bytes!("../assets/manifest.webmanifest"),
        },
        AssetId::IconSvg => Asset {
            content_type: "image/svg+xml",
            body: include_bytes!("../assets/icon.svg"),
        },
        AssetId::AppleTouchIcon => Asset {
            content_type: "image/png",
            body: include_bytes!("../assets/apple-touch-icon.png"),
        },
    }
}
