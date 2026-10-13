//! README asset contract (GitHub #381, PR 1 of #356; ADR 2026-10-13 "README Assets Are
//! Generated From Synthetic Data, Never Hand-Edited"; architect spec
//! `AI/architect_spec_readme_capture.md` §4 and §8; matrix RAC10–RAC16):
//!
//! - every image the README references resolves under `Docs/assets/` (or is an allowed badge)
//!   and carries alt text;
//! - every file under `Docs/assets/` is listed in `Docs/assets/MANIFEST.toml` with a known
//!   generator, the `synthetic` source and a matching SHA-256 (no hand-added image, so no owner
//!   photo can slip in);
//! - the asset budget (per file, total, file count) and formats (PNG, WebP, SVG);
//! - no EXIF/XMP/text/ICC metadata and no animation;
//! - the capture tooling stays dev-only and hardware-free.
//!
//! The crate has no external dependency: the SHA-256, the manifest grammar and the image chunk
//! walkers are implemented here.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Directory holding every README image.
const ASSETS_DIR: &str = "Docs/assets";
/// Manifest listing every asset (path relative to [`ASSETS_DIR`]).
const MANIFEST_PATH: &str = "Docs/assets/MANIFEST.toml";
/// Manifest file name (not an asset).
const MANIFEST_NAME: &str = "MANIFEST.toml";
/// Per-file budget, inclusive (OD-4, assumed — owner to confirm).
const MAX_ASSET_BYTES: u64 = 1_048_576;
/// Budget of every asset together, inclusive (OD-4).
const MAX_TOTAL_ASSET_BYTES: u64 = 8_388_608;
/// Most asset files (OD-4).
const MAX_ASSET_FILES: usize = 32;
/// Accepted formats (still images only, OD-2).
const ALLOWED_EXTENSIONS: [&str; 3] = ["png", "webp", "svg"];
/// Provenance allow-list: the only tools that may produce an asset.
const ALLOWED_GENERATORS: [&str; 2] =
    ["soos-gui readme_snapshots", "scripts/capture_readme_web.sh"];
/// The only accepted `source` value.
const ALLOWED_SOURCE: &str = "synthetic";
/// External images the README may show (badges only).
const SHIELDS_PREFIX: &str = "https://img.shields.io/";
const WORKFLOW_BADGE_PREFIX: &str = "https://github.com/soos-security/soos/actions/workflows/";
const WORKFLOW_BADGE_SUFFIX: &str = "/badge.svg";

/// PNG chunks an asset may carry (critical chunks plus colour/size information).
const PNG_ALLOWED_CHUNKS: [&[u8; 4]; 9] = [
    b"IHDR", b"PLTE", b"tRNS", b"gAMA", b"cHRM", b"sRGB", b"pHYs", b"IDAT", b"IEND",
];
/// WebP chunks an asset may carry (still images).
const WEBP_ALLOWED_CHUNKS: [&[u8; 4]; 4] = [b"VP8 ", b"VP8L", b"VP8X", b"ALPH"];
/// VP8X flag bits: ICC (0x20), alpha (0x10), EXIF (0x08), XMP (0x04), animation (0x02).
const VP8X_FORBIDDEN_FLAGS: u8 = 0x20 | 0x08 | 0x04 | 0x02;
/// Markup an SVG asset may not contain.
const SVG_FORBIDDEN: [&str; 7] = [
    "<metadata",
    "<image",
    "sodipodi:",
    "inkscape:",
    "<script",
    "href=\"http",
    "href=\"data:",
];

/// The capture scripts (spec §2).
const CAPTURE_SCRIPT: &str = "scripts/capture_readme_assets.sh";
const WEB_CAPTURE_SCRIPT: &str = "scripts/capture_readme_web.sh";
const FIXTURE_SERVER: &str = "tests/fixtures/examples/readme_web_fixture_server.rs";
const GUI_MANIFEST: &str = "crates/gui/Cargo.toml";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let path = workspace_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("Cannot read {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------------------
// SHA-256 (FIPS 180-4), no external crate
// ---------------------------------------------------------------------------------------

const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// Lowercase hex SHA-256 of `data`.
fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    for block in message.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(value);
        }
    }
    h.iter().map(|word| format!("{word:08x}")).collect()
}

// ---------------------------------------------------------------------------------------
// README image references
// ---------------------------------------------------------------------------------------

/// One image reference of the README.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageRef {
    /// Alt text (`None` for a `<source>` element, whose `<img>` carries the alt).
    alt: Option<String>,
    /// Referenced URL or relative path.
    url: String,
}

/// The README without fenced code blocks (examples there are not rendered images).
fn strip_fences(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut in_fence = false;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Value of the HTML attribute `name` inside one tag (double or single quotes).
fn html_attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(name) {
        let start = from + pos;
        let before_ok = start == 0 || lower.as_bytes()[start - 1].is_ascii_whitespace();
        let rest = &tag[start + name.len()..];
        let rest_trim = rest.trim_start();
        if before_ok && rest_trim.starts_with('=') {
            let value = rest_trim[1..].trim_start();
            let quote = value.chars().next()?;
            if quote == '"' || quote == '\'' {
                let inner = &value[1..];
                let end = inner.find(quote)?;
                return Some(inner[..end].to_string());
            }
            let end = value
                .find(|c: char| c.is_ascii_whitespace() || c == '>')
                .unwrap_or(value.len());
            return Some(value[..end].to_string());
        }
        from = start + name.len();
    }
    None
}

/// Every image reference: Markdown `![alt](url)`, `<img>` and `<source srcset>`.
fn image_refs(markdown: &str) -> Vec<ImageRef> {
    let text = strip_fences(markdown);
    let mut refs = Vec::new();

    // Markdown images.
    let mut from = 0;
    while let Some(pos) = text[from..].find("![") {
        let start = from + pos + 2;
        let Some(close) = text[start..].find("](") else {
            break;
        };
        let alt = text[start..start + close].to_string();
        let url_start = start + close + 2;
        let Some(end) = text[url_start..].find(')') else {
            break;
        };
        let target = text[url_start..url_start + end].trim();
        let url = target
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(|c| c == '<' || c == '>')
            .to_string();
        refs.push(ImageRef {
            alt: Some(alt),
            url,
        });
        from = url_start + end;
    }

    // HTML tags.
    let lower = text.to_ascii_lowercase();
    for (tag_name, is_img) in [("<img", true), ("<source", false)] {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(tag_name) {
            let start = from + pos;
            let end = lower[start..]
                .find('>')
                .map_or(text.len(), |e| start + e + 1);
            let tag = &text[start..end];
            if is_img {
                refs.push(ImageRef {
                    alt: Some(html_attr(tag, "alt").unwrap_or_default()),
                    url: html_attr(tag, "src").unwrap_or_default(),
                });
            } else if let Some(srcset) = html_attr(tag, "srcset") {
                for candidate in srcset.split(',') {
                    let url = candidate
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_string();
                    refs.push(ImageRef { alt: None, url });
                }
            }
            from = end;
        }
    }
    refs
}

/// Whether `url` is an allowed external badge.
fn is_allowed_badge(url: &str) -> bool {
    if url.starts_with(SHIELDS_PREFIX) {
        return true;
    }
    url.strip_prefix(WORKFLOW_BADGE_PREFIX).is_some_and(|rest| {
        let path = rest.split('?').next().unwrap_or("");
        path.ends_with(WORKFLOW_BADGE_SUFFIX)
            && !path.contains("..")
            && path.matches('/').count() == 1
    })
}

/// Checks one README image reference against the repository at `root`.
fn check_image_ref(image: &ImageRef, root: &Path) -> Result<(), String> {
    if let Some(alt) = &image.alt {
        if alt.trim().is_empty() {
            return Err(format!("image `{}` has no alt text", image.url));
        }
    }
    let url = image.url.as_str();
    if url.is_empty() {
        return Err("image without a source".to_string());
    }
    if url.contains("://") || url.starts_with("//") {
        return if url.starts_with("https://") && is_allowed_badge(url) {
            Ok(())
        } else {
            Err(format!("external image `{url}` is not an allowed badge"))
        };
    }
    let path = url.split(['?', '#']).next().unwrap_or("");
    let path = path.strip_prefix("./").unwrap_or(path);
    if path.starts_with('/') || path.split('/').any(|part| part == "..") {
        return Err(format!("image `{url}` must be a plain relative path"));
    }
    if !path.starts_with(&format!("{ASSETS_DIR}/")) {
        return Err(format!("image `{url}` must live under {ASSETS_DIR}/"));
    }
    let full = root.join(path);
    match fs::symlink_metadata(&full) {
        Ok(meta) if meta.file_type().is_file() => Ok(()),
        Ok(_) => Err(format!("image `{url}` is not a regular file")),
        Err(_) => Err(format!("image `{url}` does not exist")),
    }
}

// ---------------------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------------------

/// One `[[asset]]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestEntry {
    path: String,
    generator: String,
    sha256: String,
    source: String,
}

/// Parses the manifest grammar of spec §4 (no TOML crate).
fn parse_manifest(text: &str) -> Result<Vec<ManifestEntry>, String> {
    let mut entries = Vec::new();
    let mut current: Option<BTreeMap<String, String>> = None;
    let finish = |fields: BTreeMap<String, String>| -> Result<ManifestEntry, String> {
        let keys: Vec<&str> = fields.keys().map(String::as_str).collect();
        if keys != ["generator", "path", "sha256", "source"] {
            return Err(format!(
                "asset entry must have exactly path, generator, sha256, source: {keys:?}"
            ));
        }
        let entry = ManifestEntry {
            path: fields["path"].clone(),
            generator: fields["generator"].clone(),
            sha256: fields["sha256"].clone(),
            source: fields["source"].clone(),
        };
        if entry.sha256.len() != 64
            || !entry
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(format!(
                "{}: sha256 must be 64 lowercase hex digits",
                entry.path
            ));
        }
        if entry.path.is_empty()
            || entry.path.starts_with('/')
            || entry
                .path
                .split('/')
                .any(|p| p == ".." || p == "." || p.is_empty())
        {
            return Err(format!(
                "asset path `{}` must be relative to {ASSETS_DIR}",
                entry.path
            ));
        }
        Ok(entry)
    };
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[asset]]" {
            if let Some(fields) = current.take() {
                entries.push(finish(fields)?);
            }
            current = Some(BTreeMap::new());
            continue;
        }
        let Some(fields) = current.as_mut() else {
            return Err(format!(
                "line {}: key outside an [[asset]] table",
                number + 1
            ));
        };
        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {}: expected key = \"value\"", number + 1));
        };
        let key = key.trim();
        let value = value.trim();
        if !matches!(key, "path" | "generator" | "sha256" | "source") {
            return Err(format!("line {}: unknown key `{key}`", number + 1));
        }
        if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
            return Err(format!("line {}: value must be a basic string", number + 1));
        }
        let inner = &value[1..value.len() - 1];
        if inner.contains('"') || inner.contains('\\') {
            return Err(format!("line {}: escapes are not allowed", number + 1));
        }
        if fields.insert(key.to_string(), inner.to_string()).is_some() {
            return Err(format!("line {}: duplicate key `{key}`", number + 1));
        }
    }
    if let Some(fields) = current.take() {
        entries.push(finish(fields)?);
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in &entries {
        if !seen.insert(entry.path.clone()) {
            return Err(format!("duplicate asset path `{}`", entry.path));
        }
    }
    Ok(entries)
}

/// Every file under `dir` (recursive), as paths relative to `dir`; symlinks are reported as
/// errors (an asset must be a regular file).
fn asset_files(dir: &Path) -> Result<Vec<(String, u64)>, String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, u64)>) -> Result<(), String> {
        let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            let rel = path
                .strip_prefix(base)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            if meta.file_type().is_symlink() {
                return Err(format!("{rel} is a symbolic link"));
            }
            if meta.is_dir() {
                walk(base, &path, out)?;
            } else {
                out.push((rel, meta.len()));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

// ---------------------------------------------------------------------------------------
// Metadata detectors
// ---------------------------------------------------------------------------------------

/// Checks a PNG: signature, chunk walk, only allow-listed chunks (no text, EXIF, time, ICC,
/// animation), ends with IEND.
fn check_png(bytes: &[u8]) -> Result<(), String> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 8 || bytes[..8] != SIGNATURE {
        return Err("not a PNG file".to_string());
    }
    let mut offset = 8usize;
    let mut first = true;
    loop {
        let header = bytes
            .get(offset..offset + 8)
            .ok_or("truncated PNG chunk header")?;
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let kind: [u8; 4] = [header[4], header[5], header[6], header[7]];
        if first && &kind != b"IHDR" {
            return Err("PNG must start with IHDR".to_string());
        }
        first = false;
        if !PNG_ALLOWED_CHUNKS.iter().any(|allowed| **allowed == kind) {
            return Err(format!(
                "PNG chunk `{}` is not allowed (metadata or animation)",
                String::from_utf8_lossy(&kind)
            ));
        }
        let end = offset
            .checked_add(12)
            .and_then(|v| v.checked_add(len))
            .ok_or("PNG chunk length overflow")?;
        if end > bytes.len() {
            return Err("truncated PNG chunk".to_string());
        }
        offset = end;
        if &kind == b"IEND" {
            return if offset == bytes.len() {
                Ok(())
            } else {
                Err("data after the PNG IEND chunk".to_string())
            };
        }
    }
}

/// Checks a still WebP: RIFF header, allow-listed chunks, VP8X flags without ICC/EXIF/XMP/
/// animation.
fn check_webp(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err("not a WebP file".to_string());
    }
    let riff_len = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    if riff_len.checked_add(8) != Some(bytes.len()) {
        return Err("WebP RIFF length does not match the file size".to_string());
    }
    let mut offset = 12usize;
    while offset < bytes.len() {
        let header = bytes
            .get(offset..offset + 8)
            .ok_or("truncated WebP chunk header")?;
        let kind: [u8; 4] = [header[0], header[1], header[2], header[3]];
        let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        if !WEBP_ALLOWED_CHUNKS.iter().any(|allowed| **allowed == kind) {
            return Err(format!(
                "WebP chunk `{}` is not allowed (metadata or animation)",
                String::from_utf8_lossy(&kind)
            ));
        }
        let payload = bytes
            .get(offset + 8..offset + 8 + len)
            .ok_or("truncated WebP chunk")?;
        if &kind == b"VP8X" {
            let flags = *payload.first().ok_or("empty VP8X chunk")?;
            if flags & VP8X_FORBIDDEN_FLAGS != 0 {
                return Err("WebP VP8X declares ICC, EXIF, XMP or animation".to_string());
            }
        }
        offset = offset + 8 + len + (len % 2);
    }
    Ok(())
}

/// Checks an SVG: no editor metadata, no embedded raster, no script, no remote reference.
fn check_svg(text: &str) -> Result<(), String> {
    if !text.contains("<svg") {
        return Err("not an SVG file".to_string());
    }
    let lower = text.to_ascii_lowercase();
    for needle in SVG_FORBIDDEN {
        if lower.contains(needle) {
            return Err(format!("SVG contains `{needle}`"));
        }
    }
    Ok(())
}

/// Checks one asset by extension.
fn check_asset_bytes(rel: &str, bytes: &[u8]) -> Result<(), String> {
    let extension = rel.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match extension {
        "png" => check_png(bytes),
        "webp" => check_webp(bytes),
        "svg" => check_svg(&String::from_utf8_lossy(bytes)),
        other => Err(format!("format `{other}` is not allowed")),
    }
}

/// The `[section]` → lines map of a Cargo manifest (table headers only, no TOML crate).
fn manifest_sections(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut sections: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            current = line.trim_matches(['[', ']']).to_string();
            sections.entry(current.clone()).or_default();
        } else if !line.is_empty() && !line.starts_with('#') {
            sections
                .entry(current.clone())
                .or_default()
                .push(line.to_string());
        }
    }
    sections
}

/// Every regular file under `rel` (recursive), as `(repo-relative path, content)`; unreadable
/// or binary files are skipped.
fn text_files(rel: &str) -> Vec<(String, String)> {
    let root = workspace_root();
    let mut out = Vec::new();
    let mut stack = vec![root.join(rel)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(text) = fs::read_to_string(&path) {
                let rel = path
                    .strip_prefix(&root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                out.push((rel, text));
            }
        }
    }
    out
}

/// Script lines without comments (a `#` comment or a full comment line).
fn script_code(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------

/// RAC14: SHA-256 against the FIPS 180-4 / NIST vectors.
#[test]
fn test_rac_sha256_known_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(
        sha256_hex(&vec![b'a'; 1_000_000]),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

/// RAC10 (synthetic cases): the reference checker refuses what the contract forbids.
#[test]
fn test_rac_readme_image_refs_are_checked() {
    let dir = std::env::temp_dir().join(format!("soos-rac10-{}", std::process::id()));
    let assets = dir.join("Docs/assets/screenshots");
    fs::create_dir_all(&assets).unwrap();
    fs::write(assets.join("gui-live.png"), b"x").unwrap();
    fs::create_dir_all(dir.join("img")).unwrap();
    fs::write(dir.join("img/outside.png"), b"x").unwrap();

    let markdown = "\
# Title\n\
![Live view](Docs/assets/screenshots/gui-live.png)\n\
![](Docs/assets/screenshots/gui-live.png)\n\
![Missing](Docs/assets/screenshots/missing.png)\n\
![Outside](img/outside.png)\n\
![Escape](Docs/assets/../img/outside.png)\n\
![Remote](https://example.com/a.png)\n\
![CI](https://github.com/soos-security/soos/actions/workflows/ci.yml/badge.svg)\n\
![Licence](https://img.shields.io/badge/license-AGPL-blue)\n\
<picture><source media=\"(prefers-color-scheme: dark)\" srcset=\"Docs/assets/screenshots/gui-live.png\">\
<img src=\"./Docs/assets/screenshots/gui-live.png\" alt=\"Logo\"></picture>\n\
<img src=\"Docs/assets/screenshots/gui-live.png\">\n\
```\n![Ignored](nowhere.png)\n```\n";
    let refs = image_refs(markdown);
    let verdicts: Vec<(String, bool)> = refs
        .iter()
        .map(|r| (r.url.clone(), check_image_ref(r, &dir).is_ok()))
        .collect();
    fs::remove_dir_all(&dir).ok();

    assert!(
        !refs.iter().any(|r| r.url == "nowhere.png"),
        "fenced code is not rendered: {refs:?}"
    );
    let expected = [
        ("Docs/assets/screenshots/gui-live.png", true),
        ("Docs/assets/screenshots/gui-live.png", false), // empty alt
        ("Docs/assets/screenshots/missing.png", false),
        ("img/outside.png", false),
        ("Docs/assets/../img/outside.png", false),
        ("https://example.com/a.png", false),
        (
            "https://github.com/soos-security/soos/actions/workflows/ci.yml/badge.svg",
            true,
        ),
        ("https://img.shields.io/badge/license-AGPL-blue", true),
        ("./Docs/assets/screenshots/gui-live.png", true),
        ("Docs/assets/screenshots/gui-live.png", false), // <img> without alt
        ("Docs/assets/screenshots/gui-live.png", true),  // <source srcset>
    ];
    assert_eq!(verdicts.len(), expected.len(), "{verdicts:?}");
    for (url, ok) in expected {
        assert!(
            verdicts.contains(&(url.to_string(), ok)),
            "expected `{url}` → {ok}: {verdicts:?}"
        );
    }
    assert_eq!(
        verdicts.iter().filter(|(_, ok)| *ok).count(),
        5,
        "{verdicts:?}"
    );
    assert!(!is_allowed_badge(
        "https://github.com/soos-security/soos/actions/workflows/../../evil/badge.svg"
    ));
    assert!(!is_allowed_badge(
        "https://github.com/other/repo/actions/workflows/ci.yml/badge.svg"
    ));
}

/// RAC10: every image of the real README resolves under `Docs/assets/` with alt text.
#[test]
fn test_rac_readme_images_resolve_under_docs_assets_with_alt_text() {
    let root = workspace_root();
    let readme = read("README.md");
    let failures: Vec<String> = image_refs(&readme)
        .iter()
        .filter_map(|r| check_image_ref(r, &root).err())
        .collect();
    assert!(
        failures.is_empty(),
        "README.md image references: {failures:?}"
    );
}

/// RAC11 (synthetic cases): the manifest grammar refuses malformed entries.
#[test]
fn test_rac_manifest_parser_rejects_malformed_entries() {
    let good_sha = "a".repeat(64);
    let good = format!(
        "# comment\n\n[[asset]]\npath = \"screenshots/gui-a.png\"\ngenerator = \"soos-gui readme_snapshots\"\n\
         sha256 = \"{good_sha}\"\nsource = \"synthetic\"\n"
    );
    let parsed = parse_manifest(&good).expect("valid manifest");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].path, "screenshots/gui-a.png");

    let bad_cases = [
        good.replace("sha256", "checksum"),
        good.replace(&good_sha, &"A".repeat(64)),
        good.replace(&good_sha, &"a".repeat(63)),
        good.replace("screenshots/gui-a.png", "../README.md"),
        good.replace("screenshots/gui-a.png", "/etc/passwd"),
        good.replace("source = \"synthetic\"\n", ""),
        format!("{good}{good}"),
        good.replace("[[asset]]\n", ""),
        good.replace("\"synthetic\"", "synthetic"),
        format!("{good}extra = \"x\"\n"),
    ];
    for (i, case) in bad_cases.iter().enumerate() {
        assert!(
            parse_manifest(case).is_err(),
            "bad case {i} accepted:\n{case}"
        );
    }
}

/// RAC11: every file under `Docs/assets` has one manifest entry (and vice versa) with a known
/// generator, the synthetic source and a matching SHA-256.
#[test]
fn test_rac_docs_assets_are_listed_in_the_manifest_with_matching_sha256() {
    let root = workspace_root();
    let assets_dir = root.join(ASSETS_DIR);
    let manifest = fs::read_to_string(root.join(MANIFEST_PATH))
        .unwrap_or_else(|e| panic!("{MANIFEST_PATH} must exist: {e}"));
    let entries = parse_manifest(&manifest).unwrap_or_else(|e| panic!("{MANIFEST_PATH}: {e}"));
    let files = asset_files(&assets_dir).unwrap_or_else(|e| panic!("{ASSETS_DIR}: {e}"));
    let files: Vec<String> = files
        .into_iter()
        .map(|(rel, _)| rel)
        .filter(|rel| rel != MANIFEST_NAME)
        .collect();
    assert!(!files.is_empty(), "{ASSETS_DIR} holds no asset");

    let listed: Vec<String> = entries.iter().map(|e| e.path.clone()).collect();
    let unlisted: Vec<&String> = files.iter().filter(|f| !listed.contains(f)).collect();
    let missing: Vec<&String> = listed.iter().filter(|l| !files.contains(l)).collect();
    assert!(
        unlisted.is_empty(),
        "assets missing from {MANIFEST_PATH}: {unlisted:?}"
    );
    assert!(
        missing.is_empty(),
        "{MANIFEST_PATH} lists absent files: {missing:?}"
    );

    for entry in &entries {
        assert!(
            ALLOWED_GENERATORS.contains(&entry.generator.as_str()),
            "{}: unknown generator `{}`",
            entry.path,
            entry.generator
        );
        assert_eq!(entry.source, ALLOWED_SOURCE, "{}: source", entry.path);
        let file_name = entry.path.rsplit('/').next().unwrap_or("");
        let expected_prefix = if entry.generator == ALLOWED_GENERATORS[0] {
            "gui-"
        } else {
            "web-"
        };
        assert!(
            entry.path.starts_with("screenshots/") && file_name.starts_with(expected_prefix),
            "{}: a `{}` asset is screenshots/{expected_prefix}*",
            entry.path,
            entry.generator
        );
        let bytes = fs::read(assets_dir.join(&entry.path)).expect("asset bytes");
        assert_eq!(
            sha256_hex(&bytes),
            entry.sha256,
            "{}: SHA-256 differs from {MANIFEST_PATH}; regenerate with {CAPTURE_SCRIPT}",
            entry.path
        );
    }
}

/// RAC12: per-file, total and count budget, allowed formats.
#[test]
fn test_rac_docs_assets_stay_within_the_budget() {
    let files = asset_files(&workspace_root().join(ASSETS_DIR))
        .unwrap_or_else(|e| panic!("{ASSETS_DIR}: {e}"));
    let assets: Vec<&(String, u64)> = files
        .iter()
        .filter(|(rel, _)| rel != MANIFEST_NAME)
        .collect();
    assert!(
        assets.len() <= MAX_ASSET_FILES,
        "{} assets exceed the {MAX_ASSET_FILES}-file budget",
        assets.len()
    );
    let mut total = 0u64;
    for (rel, len) in &assets {
        let extension = rel.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        assert!(
            ALLOWED_EXTENSIONS.contains(&extension),
            "{rel}: format `{extension}` is not allowed ({ALLOWED_EXTENSIONS:?})"
        );
        assert!(*len > 0, "{rel} is empty");
        assert!(
            *len <= MAX_ASSET_BYTES,
            "{rel}: {len} bytes exceed the {MAX_ASSET_BYTES}-byte budget"
        );
        total += len;
    }
    assert!(
        total <= MAX_TOTAL_ASSET_BYTES,
        "{ASSETS_DIR} totals {total} bytes, over the {MAX_TOTAL_ASSET_BYTES}-byte budget"
    );
}

/// RAC13 (synthetic cases): the detectors refuse metadata and animation.
#[test]
fn test_rac_png_webp_svg_metadata_detectors() {
    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0, 0, 0, 0]); // CRC is not checked by the walker
        out
    }
    fn png(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        for (kind, data) in chunks {
            out.extend(png_chunk(kind, data));
        }
        out
    }
    let ihdr: &[u8] = &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0];
    let clean = png(&[(b"IHDR", ihdr), (b"IDAT", &[1, 2]), (b"IEND", &[])]);
    assert!(check_png(&clean).is_ok());
    for kind in [
        b"tEXt", b"zTXt", b"iTXt", b"eXIf", b"tIME", b"iCCP", b"acTL",
    ] {
        let dirty = png(&[
            (b"IHDR", ihdr),
            (kind, &[1]),
            (b"IDAT", &[1]),
            (b"IEND", &[]),
        ]);
        assert!(
            check_png(&dirty).is_err(),
            "{} accepted",
            String::from_utf8_lossy(kind)
        );
    }
    let mut trailing = clean.clone();
    trailing.push(0);
    assert!(check_png(&trailing).is_err(), "data after IEND accepted");
    assert!(
        check_png(&clean[..clean.len() - 3]).is_err(),
        "truncated PNG accepted"
    );
    assert!(check_png(b"GIF89a").is_err());

    fn webp(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut body = b"WEBP".to_vec();
        for (kind, data) in chunks {
            body.extend_from_slice(*kind);
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
            body.extend_from_slice(data);
            if data.len() % 2 == 1 {
                body.push(0);
            }
        }
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    }
    assert!(check_webp(&webp(&[(b"VP8L", &[1, 2, 3])])).is_ok());
    assert!(check_webp(&webp(&[
        (b"VP8X", &[0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
        (b"VP8L", &[1])
    ]))
    .is_ok());
    for flags in [0x20u8, 0x08, 0x04, 0x02] {
        let dirty = webp(&[
            (b"VP8X", &[flags, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            (b"VP8L", &[1]),
        ]);
        assert!(
            check_webp(&dirty).is_err(),
            "VP8X flags {flags:#x} accepted"
        );
    }
    for kind in [b"EXIF", b"XMP ", b"ICCP", b"ANIM", b"ANMF"] {
        let dirty = webp(&[(b"VP8L", &[1, 2]), (kind, &[1])]);
        assert!(
            check_webp(&dirty).is_err(),
            "{} accepted",
            String::from_utf8_lossy(kind)
        );
    }

    assert!(
        check_svg("<svg xmlns=\"http://www.w3.org/2000/svg\"><path d=\"M0 0\"/></svg>").is_ok()
    );
    for dirty in [
        "<svg><metadata>x</metadata></svg>",
        "<svg><image href=\"a.png\"/></svg>",
        "<svg sodipodi:docname=\"a\"></svg>",
        "<svg><script>alert(1)</script></svg>",
        "<svg><use href=\"https://example.com/a.svg#x\"/></svg>",
        "<svg><use href=\"data:image/png;base64,AA\"/></svg>",
    ] {
        assert!(check_svg(dirty).is_err(), "{dirty} accepted");
    }
    assert!(check_asset_bytes("a.gif", b"GIF89a").is_err());
    assert!(check_asset_bytes("a.jpg", &[0xFF, 0xD8]).is_err());
}

/// RAC13: the committed assets carry no metadata and no animation.
#[test]
fn test_rac_docs_assets_carry_no_metadata_and_no_animation() {
    let dir = workspace_root().join(ASSETS_DIR);
    let files = asset_files(&dir).unwrap_or_else(|e| panic!("{ASSETS_DIR}: {e}"));
    let mut checked = 0;
    for (rel, _) in files.iter().filter(|(rel, _)| rel != MANIFEST_NAME) {
        let bytes = fs::read(dir.join(rel)).expect("asset bytes");
        if let Err(e) = check_asset_bytes(rel, &bytes) {
            panic!("{ASSETS_DIR}/{rel}: {e}");
        }
        checked += 1;
    }
    assert!(checked > 0, "{ASSETS_DIR} holds no asset");
}

/// RAC15: the capture tooling is dev-only and adds no surface to the shipped programs.
#[test]
fn test_rac_capture_tooling_is_dev_only() {
    let sections = manifest_sections(&read(GUI_MANIFEST));
    let features = sections.get("features").cloned().unwrap_or_default();
    assert!(
        features
            .iter()
            .any(|l| l.replace(' ', "") == "readme-capture=[]"),
        "{GUI_MANIFEST} declares the empty `readme-capture` feature: {features:?}"
    );
    assert!(
        !features
            .iter()
            .any(|l| l.starts_with("default") && l.contains("readme-capture")),
        "`readme-capture` must never be a default feature"
    );
    for (name, lines) in &sections {
        for dep in ["png", "soos-test-fixtures"] {
            let declares = lines.iter().any(|l| {
                l.split(['=', ' ', '.'])
                    .next()
                    .is_some_and(|key| key.trim() == dep)
            });
            if declares {
                assert_eq!(
                    name, "dev-dependencies",
                    "{GUI_MANIFEST}: `{dep}` may only be a dev-dependency (found in [{name}])"
                );
            }
        }
    }
    let dev = sections
        .get("dev-dependencies")
        .cloned()
        .unwrap_or_default();
    assert!(
        dev.iter().any(|l| l.starts_with("png")),
        "png is a gui dev-dependency"
    );

    // Nothing that builds, installs or packages soos enables the feature.
    let mut offenders = Vec::new();
    for dir in [
        "packaging",
        "scripts",
        ".github",
        "tests/docker",
        "tests/distro",
    ] {
        for (rel, text) in text_files(dir) {
            if rel == CAPTURE_SCRIPT || rel == WEB_CAPTURE_SCRIPT {
                continue;
            }
            if text.contains("readme-capture") || text.contains("readme_web_fixture_server") {
                offenders.push(rel);
            }
        }
    }
    for file in ["Dockerfile", "save.sh", "run_tests.sh"] {
        if let Ok(text) = fs::read_to_string(workspace_root().join(file)) {
            if text.contains("readme-capture") {
                offenders.push(file.to_string());
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "build/packaging files enable the capture tooling: {offenders:?}"
    );

    // The fixture server lives outside soos-remote; soos-remote never mentions it.
    assert!(
        workspace_root().join(FIXTURE_SERVER).is_file(),
        "{FIXTURE_SERVER} must exist"
    );
    let remote_mentions: Vec<String> = text_files("crates/remote")
        .into_iter()
        .filter(|(_, text)| {
            text.contains("readme_web")
                || text.contains("readme-capture")
                || text.contains("soos-demo")
        })
        .map(|(rel, _)| rel)
        .collect();
    assert!(
        remote_mentions.is_empty(),
        "crates/remote mentions the capture tooling: {remote_mentions:?}"
    );
    // Loopback only (spec §10, F-8).
    let server = read(FIXTURE_SERVER);
    let fixtures_lib = read("tests/fixtures/mod.rs");
    assert!(
        server.contains("\"127.0.0.1:0\""),
        "{FIXTURE_SERVER} binds an ephemeral loopback port"
    );
    for (rel, text) in [
        (FIXTURE_SERVER, &server),
        ("tests/fixtures/mod.rs", &fixtures_lib),
    ] {
        for wildcard in ["0.0.0.0", "[::]", "::/", "\"::\""] {
            assert!(
                !text.contains(wildcard),
                "{rel} must never bind `{wildcard}`"
            );
        }
    }
    let fixtures = read("tests/fixtures/Cargo.toml");
    let fixture_deps = manifest_sections(&fixtures)
        .get("dependencies")
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        fixture_deps,
        vec!["soos-frame = { workspace = true }".to_string()],
        "the fixture server is std-only: soos-test-fixtures keeps its single dependency"
    );
}

/// RAC16: the capture scripts are strict, hardware-free and offline.
#[test]
fn test_rac_capture_scripts_are_hardware_free() {
    for script in [CAPTURE_SCRIPT, WEB_CAPTURE_SCRIPT] {
        let text = read(script);
        assert!(
            text.starts_with("#!/usr/bin/env bash\n"),
            "{script}: bash shebang"
        );
        assert!(text.contains("set -euo pipefail"), "{script}: strict mode");
        let code = script_code(&text);
        for forbidden in [
            "/dev/video",
            "sudo",
            "pkexec",
            "systemctl",
            "soos-daemon",
            "soos-remote ",
            "curl",
            "wget",
            "npm",
            "npx",
            "docker pull",
            "/var/lib/soos",
            "/etc/soos",
        ] {
            assert!(
                !code.contains(forbidden),
                "{script} must not use `{forbidden}`"
            );
        }
    }
    let assets = script_code(&read(CAPTURE_SCRIPT));
    assert!(
        assets.contains("SOOS_UPDATE_README_ASSETS=1")
            && assets.contains("--features readme-capture")
            && assets.contains("--test readme_snapshots"),
        "{CAPTURE_SCRIPT} regenerates the GUI screenshots through the snapshot test"
    );
    assert!(
        assets.contains("MANIFEST.toml") && assets.contains("sha256sum"),
        "{CAPTURE_SCRIPT} rewrites the manifest with SHA-256 digests"
    );
    let web = script_code(&read(WEB_CAPTURE_SCRIPT));
    assert!(
        web.contains("--example readme_web_fixture_server"),
        "{WEB_CAPTURE_SCRIPT} serves the pages from the fixture server only"
    );
    assert!(
        web.contains("--user-data-dir") && web.contains("mktemp -d"),
        "{WEB_CAPTURE_SCRIPT} uses a throw-away browser profile"
    );
    assert!(
        web.contains("http://127.0.0.1:"),
        "{WEB_CAPTURE_SCRIPT} only opens the loopback fixture server"
    );
}
