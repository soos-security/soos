//! Android asset contract of `soos-remote` (GitHub #349, ADR 2026-10-09 "Android Support for
//! the `soos-remote` Phone Companion and Web Push", architect spec
//! `AI/architect_spec_remote_android.md` §2.3, §6 and §11.2; matrix RAN1, RAN4, RAN5).
//!
//! The four new icons (`icon-192.png`, `icon-512.png`, `icon-maskable-512.png`,
//! `badge-96.png`) are routed `GET`/`HEAD` only as `image/png`, Funnel-public, under the
//! unchanged CSP; their pixels are checked with the test-only `png` decoder (AM-7): the
//! maskable mark stays inside the 40 % safe-zone circle on brand blue, the badge is a white
//! silhouette on transparency, the `any` icons are full-bleed brand renders.
//!
//! The asset ids are reached through `route()` (never named), so this file compiles before
//! the four `AssetId` variants exist; the variant names are pinned through `Debug`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::manual_range_contains,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

#[path = "common/passkey.rs"]
mod passkey;

#[path = "common/harness.rs"]
mod harness;

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use harness::*;
use soos_remote::assets::{asset, AssetId};
use soos_remote::http::Method;
use soos_remote::routes::{allow_header, is_funnel_public, route, Route};

const IP_A: &str = "203.0.113.10";

const BLUE: [u8; 3] = [0x00, 0x47, 0xBB];
const PALE: [u8; 3] = [0xED, 0xF1, 0xFF];

/// `(path, AssetId variant name, file name)` of spec §2.3.
const NEW_ASSETS: [(&str, &str, &str); 4] = [
    ("/icon-192.png", "Icon192", "icon-192.png"),
    ("/icon-512.png", "Icon512", "icon-512.png"),
    (
        "/icon-maskable-512.png",
        "IconMaskable512",
        "icon-maskable-512.png",
    ),
    ("/badge-96.png", "Badge96", "badge-96.png"),
];

fn asset_path(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join(file)
}

fn on_disk(file: &str) -> Vec<u8> {
    let path = asset_path(file);
    fs::read(&path).unwrap_or_else(|e| panic!("{} must exist (spec §6): {e}", path.display()))
}

/// The asset id `route(GET, path)` resolves to.
fn routed_id(path: &str) -> AssetId {
    match route(Method::Get, path) {
        Route::Asset(id) => id,
        other => panic!("GET {path} must route to an asset, got {other:?}"),
    }
}

/// A decoded 8-bit image: `(width, height, channels, pixels)`.
struct Image {
    width: u32,
    height: u32,
    channels: usize,
    data: Vec<u8>,
}

impl Image {
    fn pixel(&self, x: u32, y: u32) -> &[u8] {
        let at = (y as usize * self.width as usize + x as usize) * self.channels;
        &self.data[at..at + self.channels]
    }
}

/// Decodes `file` without any transformation; asserts 8-bit `color`.
fn decode(file: &str, color: png::ColorType) -> Image {
    let bytes = on_disk(file);
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder
        .read_info()
        .unwrap_or_else(|e| panic!("{file}: {e}"));
    let mut data = vec![0u8; reader.output_buffer_size().expect("bounded output")];
    let info = reader
        .next_frame(&mut data)
        .unwrap_or_else(|e| panic!("{file}: {e}"));
    assert_eq!(info.color_type, color, "{file} color type");
    assert_eq!(info.bit_depth, png::BitDepth::Eight, "{file} bit depth");
    data.truncate(info.buffer_size());
    let channels = match color {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(
        data.len(),
        info.width as usize * info.height as usize * channels,
        "{file}: tightly packed rows"
    );
    Image {
        width: info.width,
        height: info.height,
        channels,
        data,
    }
}

fn close(a: &[u8], b: [u8; 3], tolerance: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tolerance)
}

/// RAN5 (spec §2.3): the four paths route `GET`/`HEAD` to their own asset (variant names
/// pinned), `POST`/other are `405` with `Allow: GET, HEAD`, near-miss paths are `404`, the
/// asset is `image/png` with the bytes on disk, and it is Funnel-public.
#[test]
fn test_ran_assets_are_routed_as_png() {
    let mut seen = Vec::new();
    for (path, variant, file) in NEW_ASSETS {
        let id = routed_id(path);
        assert_eq!(format!("{id:?}"), variant, "GET {path}");
        assert_eq!(route(Method::Head, path), Route::Asset(id), "HEAD {path}");
        assert_eq!(
            route(Method::Get, &format!("{path}?v=2")),
            Route::Asset(id),
            "query string ignored"
        );
        for method in [Method::Post, Method::Other] {
            assert_eq!(route(method, path), Route::MethodNotAllowed, "{path}");
        }
        assert_eq!(allow_header(path), "GET, HEAD", "{path}");
        let upper_ext = path.replace(".png", ".PNG");
        let mut capitalized = path.to_string();
        capitalized.replace_range(1..2, &path[1..2].to_uppercase());
        for near in [
            format!("{path}/"),
            capitalized,
            upper_ext,
            format!("/assets{path}"),
        ] {
            assert_eq!(route(Method::Get, &near), Route::NotFound, "GET {near}");
        }
        let a = asset(id);
        assert_eq!(a.content_type, "image/png", "{path}");
        assert_eq!(a.body, on_disk(file).as_slice(), "{path} embeds {file}");
        assert!(
            is_funnel_public(Route::Asset(id)),
            "{path} is Funnel-public"
        );
        assert!(!seen.contains(&id), "{path} has its own asset id");
        seen.push(id);
    }
    for existing in [
        AssetId::Index,
        AssetId::AppJs,
        AssetId::StyleCss,
        AssetId::Manifest,
        AssetId::IconSvg,
        AssetId::AppleTouchIcon,
        AssetId::ServiceWorker,
    ] {
        assert!(!seen.contains(&existing), "{existing:?} is not reused");
    }
}

/// RAN5 (spec §2.3): served by the real server over the tailnet and to an anonymous Funnel
/// caller: `200`, `image/png`, the exact pinned CSP and the mandatory headers; `HEAD` gives
/// the same headers and no body; `POST` is `405`.
#[tokio::test(start_paused = true)]
async fn test_ran_assets_are_served_with_the_unchanged_csp() {
    let h = Harness::start_with(Options::funnel()).await;
    let anonymous = Via::funnel(IP_A);
    for (path, _, file) in NEW_ASSETS {
        let body = on_disk(file);
        for via in [Via::Tailnet, anonymous.clone()] {
            let r = h.get_via(&via, path).await;
            assert_eq!(r.status, 200, "GET {path} via {via:?}");
            assert_eq!(r.header("content-type"), Some("image/png"), "{path}");
            assert_eq!(r.header("content-security-policy"), Some(CSP), "{path}");
            assert_eq!(
                r.header("x-content-type-options"),
                Some("nosniff"),
                "{path}"
            );
            assert_eq!(r.header("cache-control"), Some("no-store"), "{path}");
            r.assert_mandatory_headers();
            assert_eq!(r.body, body, "{path} body");
            assert!(
                r.header("set-cookie").is_none(),
                "an asset never sets a cookie"
            );

            let head = h.send(&via, "HEAD", path, &[], None).await;
            assert_eq!(head.status, 200, "HEAD {path}");
            assert_eq!(head.header("content-type"), Some("image/png"));
            assert_eq!(
                head.header("content-length"),
                Some(body.len().to_string().as_str())
            );
            head.assert_mandatory_headers();
            assert!(head.body.is_empty(), "HEAD {path} has no body");
        }
        let r = h.send(&Via::Tailnet, "POST", path, &[], None).await;
        assert_eq!(r.status, 405, "POST {path}");
        assert_eq!(r.header("allow"), Some("GET, HEAD"));
        r.assert_mandatory_headers();
    }
}

/// RAN4 (spec §6, D-2): the maskable icon is 512 x 512 RGB; every pixel whose centre lies
/// farther than 204.8 px (40 %) from the icon centre is exactly brand blue; the two pale
/// spikes are present near the centre.
#[test]
fn test_ran_maskable_icon_keeps_the_mark_in_the_safe_zone() {
    let image = decode("icon-maskable-512.png", png::ColorType::Rgb);
    assert_eq!((image.width, image.height), (512, 512));
    let radius = 0.4 * 512.0;
    let mut outside = 0usize;
    for y in 0..image.height {
        for x in 0..image.width {
            let dx = f64::from(x) + 0.5 - 256.0;
            let dy = f64::from(y) + 0.5 - 256.0;
            if (dx * dx + dy * dy).sqrt() > radius {
                outside += 1;
                assert_eq!(
                    image.pixel(x, y),
                    BLUE,
                    "pixel ({x}, {y}) outside the safe zone must be #0047BB"
                );
            }
        }
    }
    assert!(
        outside > 100_000,
        "the safe-zone ring was scanned ({outside})"
    );
    for (x, y) in [(266, 246), (246, 266)] {
        assert_eq!(image.pixel(x, y), PALE, "spike pixel ({x}, {y}) is #EDF1FF");
    }
}

/// RAN1 (spec §6, D-3): the badge is 96 x 96 RGBA, every visible pixel white, about a tenth
/// opaque, transparent at the corners and off the spikes, opaque on the spikes.
#[test]
fn test_ran_badge_is_white_on_transparent() {
    let image = decode("badge-96.png", png::ColorType::Rgba);
    assert_eq!((image.width, image.height), (96, 96));
    let mut opaque = 0usize;
    for y in 0..96 {
        for x in 0..96 {
            let p = image.pixel(x, y);
            if p[3] > 0 {
                assert_eq!(
                    &p[..3],
                    [255, 255, 255],
                    "visible pixel ({x}, {y}) is white"
                );
            }
            if p[3] >= 128 {
                opaque += 1;
            }
        }
    }
    let fraction = opaque as f64 / (96.0 * 96.0);
    assert!(
        (0.08..=0.14).contains(&fraction),
        "opaque fraction {fraction} within [0.08, 0.14]"
    );
    for (x, y) in [(0, 0), (95, 0), (0, 95), (95, 95), (24, 24), (72, 72)] {
        assert_eq!(image.pixel(x, y)[3], 0, "({x}, {y}) is transparent");
    }
    for (x, y) in [(52, 44), (44, 52)] {
        assert_eq!(image.pixel(x, y)[3], 255, "({x}, {y}) is opaque");
    }
}

/// RAN4 (spec §6): `icon-192.png` and `icon-512.png` are opaque RGB renders of their size,
/// blue at the four corners and pale on the spike of the top-right quadrant.
#[test]
fn test_ran_any_icons_are_full_bleed_brand_renders() {
    for (file, n) in [("icon-192.png", 192u32), ("icon-512.png", 512)] {
        let image = decode(file, png::ColorType::Rgb);
        assert_eq!((image.width, image.height), (n, n), "{file}");
        for (x, y) in [(0, 0), (n - 1, 0), (0, n - 1), (n - 1, n - 1)] {
            assert_eq!(image.pixel(x, y), BLUE, "{file} corner ({x}, {y})");
        }
        let (x, y) = ((0.55 * f64::from(n)) as u32, (0.45 * f64::from(n)) as u32);
        let p = image.pixel(x, y);
        assert!(close(p, PALE, 2), "{file} ({x}, {y}) is #EDF1FF ± 2: {p:?}");
    }
}
