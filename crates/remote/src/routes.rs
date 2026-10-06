//! Route table and the lock / unlock CSRF checks (spec §2.5, D4; ADR 2026-10-06).
//!
//! Paths are matched exactly (no normalisation, no case folding): anything that is not in
//! the table is `404`, a known path with a wrong method is `405`.

use crate::assets::AssetId;
use crate::http::{Method, RequestHead};
use crate::{ACTION_HEADER, ACTION_LOCK, ACTION_UNLOCK};

/// Resolved route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// A static asset.
    Asset(AssetId),
    /// `GET /api/status`.
    Status,
    /// `GET /api/events`.
    Events,
    /// `POST /api/lock`.
    Lock,
    /// `POST /api/unlock` (ADR 2026-10-06).
    Unlock,
    /// `404`.
    NotFound,
    /// `405` with an `Allow` header.
    MethodNotAllowed,
}

/// Path of the lock `POST` route.
pub const LOCK_PATH: &str = "/api/lock";
/// Path of the unlock `POST` route.
pub const UNLOCK_PATH: &str = "/api/unlock";

/// The `Allow` header value of a `405` on `path`.
#[must_use]
pub fn allow_header(path: &str) -> &'static str {
    let path = path.split('?').next().unwrap_or(path);
    if path == LOCK_PATH || path == UNLOCK_PATH {
        "POST"
    } else {
        "GET, HEAD"
    }
}

/// Pure. GET/HEAD for assets, `/api/status` and `/api/events`; POST only for `/api/lock`
/// and `/api/unlock`.
/// `/` → `Asset(Index)`. Query strings are ignored.
#[must_use]
pub fn route(method: Method, path: &str) -> Route {
    let path = path.split('?').next().unwrap_or(path);
    let read_route = match path {
        "/" | "/index.html" => Some(Route::Asset(AssetId::Index)),
        "/app.js" => Some(Route::Asset(AssetId::AppJs)),
        "/style.css" => Some(Route::Asset(AssetId::StyleCss)),
        "/manifest.webmanifest" => Some(Route::Asset(AssetId::Manifest)),
        "/icon.svg" => Some(Route::Asset(AssetId::IconSvg)),
        "/apple-touch-icon.png" => Some(Route::Asset(AssetId::AppleTouchIcon)),
        "/api/status" => Some(Route::Status),
        "/api/events" => Some(Route::Events),
        _ => None,
    };
    if let Some(found) = read_route {
        return match method {
            Method::Get | Method::Head => found,
            Method::Post | Method::Other => Route::MethodNotAllowed,
        };
    }
    let post_route = match path {
        LOCK_PATH => Some(Route::Lock),
        UNLOCK_PATH => Some(Route::Unlock),
        _ => None,
    };
    if let Some(found) = post_route {
        return match method {
            Method::Post => found,
            Method::Get | Method::Head | Method::Other => Route::MethodNotAllowed,
        };
    }
    Route::NotFound
}

/// CSRF refusal; every variant → `403`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CsrfError {
    /// `X-Soos-Action` absent, repeated or not the route's action.
    #[error("action header missing")]
    MissingActionHeader,
    /// `Sec-Fetch-Site` present and not `same-origin`.
    #[error("cross-site request")]
    CrossSite,
    /// `Origin` present and not `https://<normalized host>`.
    #[error("origin mismatch")]
    OriginMismatch,
}

/// Exactly one header `name` (names are already lowercased by the parser): `Ok(None)` when
/// absent, `Ok(Some(value))` when present once, `Err(())` when repeated.
fn optional_single<'a>(head: &'a RequestHead, name: &str) -> Result<Option<&'a [u8]>, ()> {
    let mut found = None;
    for (header, value) in &head.headers {
        if header == name {
            if found.is_some() {
                return Err(());
            }
            found = Some(value.as_slice());
        }
    }
    Ok(found)
}

/// Pure; spec D4. In order: exactly one `X-Soos-Action` whose bytes are exactly `lock`;
/// `Sec-Fetch-Site`, if present, exactly one with bytes `same-origin`; `Origin`, if present,
/// exactly one, ASCII-lowercased, equal to `https://<normalized_host>` or
/// `https://<normalized_host>:443`. `normalized_host` is the value returned by `check_host`.
///
/// # Errors
///
/// [`CsrfError`].
pub fn check_lock_csrf(head: &RequestHead, normalized_host: &str) -> Result<(), CsrfError> {
    check_action_csrf(head, normalized_host, ACTION_LOCK)
}

/// Pure; the rules of [`check_lock_csrf`] with the action value `unlock` (ADR 2026-10-06).
///
/// # Errors
///
/// [`CsrfError`].
pub fn check_unlock_csrf(head: &RequestHead, normalized_host: &str) -> Result<(), CsrfError> {
    check_action_csrf(head, normalized_host, ACTION_UNLOCK)
}

/// The shared CSRF rules; `action` is the exact `X-Soos-Action` value of the route.
fn check_action_csrf(
    head: &RequestHead,
    normalized_host: &str,
    action: &str,
) -> Result<(), CsrfError> {
    match optional_single(head, ACTION_HEADER) {
        Ok(Some(value)) if value == action.as_bytes() => {}
        _ => return Err(CsrfError::MissingActionHeader),
    }
    match optional_single(head, "sec-fetch-site") {
        Ok(None) => {}
        Ok(Some(value)) if value == b"same-origin" => {}
        _ => return Err(CsrfError::CrossSite),
    }
    match optional_single(head, "origin") {
        Ok(None) => Ok(()),
        Ok(Some(value)) => {
            let text = std::str::from_utf8(value).map_err(|_| CsrfError::OriginMismatch)?;
            let lowered = text.to_ascii_lowercase();
            let expected = format!("https://{normalized_host}");
            let expected_with_port = format!("https://{normalized_host}:443");
            if lowered == expected || lowered == expected_with_port {
                Ok(())
            } else {
                Err(CsrfError::OriginMismatch)
            }
        }
        Err(()) => Err(CsrfError::OriginMismatch),
    }
}
