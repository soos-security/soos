//! Protocol codec and IPC documentation contract (GitHub #224, #225, #226).
//!
//! - #224: the daemon decodes payloads through the shared strict decoder of `soos-protocol`
//!   (`codec::decode_payload`), never through `postcard` directly, so the PAM client and the
//!   daemon apply one strictness.
//! - #225: `encode_with_limit` serializes straight into the final frame buffer; no
//!   intermediate `postcard` allocation holding the request nonce is freed un-zeroized.
//! - #226: `Docs/IPC_PROTOCOL.md`, the protocol crate frame diagram and the PAM crate docs
//!   name identifiers, wire widths, enum indices and deadlines that exist in the code.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    Path::new(&manifest_dir)
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Returns the source text before the first `#[cfg(test)]` (production code only).
fn production_part(src: &str) -> &str {
    src.split("#[cfg(test)]").next().unwrap_or(src)
}

/// Parses `pub enum <name> { Variant = N, ... }` from `types.rs` into `(variant, index)`.
fn enum_variants(src: &str, name: &str) -> Vec<(String, u32)> {
    let header = format!("pub enum {name} {{");
    let start = src
        .find(&header)
        .unwrap_or_else(|| panic!("enum {name} not found"));
    let body = &src[start + header.len()..];
    let end = body.find('}').expect("enum body end");
    body[..end]
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//") && l.contains('='))
        .map(|l| {
            let (variant, index) = l.split_once('=').expect("explicit discriminant");
            let index = index.trim().trim_end_matches(',').trim();
            (
                variant.trim().to_string(),
                index.parse().expect("numeric discriminant"),
            )
        })
        .collect()
}

/// Parses the variant names of `pub enum <name>` in `src` (explicit discriminant or not).
fn enum_variant_names(src: &str, name: &str) -> Vec<String> {
    let header = format!("pub enum {name} {{");
    let start = src
        .find(&header)
        .unwrap_or_else(|| panic!("enum {name} not found"));
    let body = &src[start + header.len()..];
    let mut depth = 0usize;
    let mut names = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("//") || t.starts_with("#[") || t.is_empty() {
            continue;
        }
        if depth == 0 && t.starts_with('}') {
            break;
        }
        if depth == 0 {
            let ident: String = t
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !ident.is_empty() {
                names.push(ident);
            }
        }
        depth += t.matches('{').count();
        depth = depth.saturating_sub(t.matches('}').count());
    }
    names
}

#[test]
fn test_codec_encode_serializes_without_intermediate_buffer() {
    let codec = read("crates/protocol/src/codec.rs");
    let prod = production_part(&codec);
    for forbidden in ["to_allocvec", "to_stdvec", "to_extend"] {
        assert!(
            !prod.contains(forbidden),
            "crates/protocol/src/codec.rs must not serialize through `{forbidden}`: the \
             intermediate buffer holding the request nonce would be freed un-zeroized (#225)"
        );
    }
    assert!(
        prod.contains("postcard::ser_flavors::Size") && prod.contains("postcard::to_slice"),
        "encode_with_limit must size the frame first and serialize into it in place (#225)"
    );
}

#[test]
fn test_codec_decode_is_strict_about_trailing_bytes() {
    let codec = read("crates/protocol/src/codec.rs");
    let prod = production_part(&codec);
    assert!(
        !prod.contains("postcard::from_bytes"),
        "the codec must not use the lenient `postcard::from_bytes` (#224)"
    );
    assert!(
        prod.contains("postcard::take_from_bytes") && prod.contains("TrailingBytes"),
        "the codec must reject unconsumed payload bytes with CodecError::TrailingBytes (#224)"
    );
}

#[test]
fn test_no_crate_decodes_wire_payloads_with_postcard_directly() {
    let crates_dir = workspace_root().join("crates");
    let mut violations = Vec::new();
    for entry in fs::read_dir(&crates_dir).expect("crates dir") {
        let dir = entry.expect("entry").path();
        if dir.file_name().and_then(|n| n.to_str()) == Some("protocol") {
            continue;
        }
        let src = dir.join("src");
        if !src.is_dir() {
            continue;
        }
        let mut stack = vec![src];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).expect("read src") {
                let p = e.expect("entry").path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                    let text = fs::read_to_string(&p).expect("read rs");
                    if production_part(&text).contains("postcard::") {
                        violations.push(
                            p.strip_prefix(workspace_root())
                                .unwrap_or(&p)
                                .display()
                                .to_string(),
                        );
                    }
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "wire payloads must be decoded through soos_protocol::codec (strict), not postcard \
         directly (#224): {violations:?}"
    );
    assert!(
        read("crates/daemon/src/dispatcher.rs").contains("decode_payload"),
        "the daemon dispatcher must use soos_protocol::codec::decode_payload (#224)"
    );
}

#[test]
fn test_ipc_protocol_doc_names_only_existing_identifiers() {
    let doc = read("Docs/IPC_PROTOCOL.md");
    for stale in [
        "PayloadCorrupted",
        "PROTOCOL_VERSION",
        "ZeroizeOnDrop",
        "RequestKind::Authenticate",
    ] {
        assert!(
            !doc.contains(stale),
            "Docs/IPC_PROTOCOL.md names `{stale}`, which does not exist in the code (#226)"
        );
    }
    for required in [
        "CURRENT_VERSION",
        "MAX_PREVIEW_MESSAGE_SIZE",
        "StatusResponse",
        "PreviewResponse",
        "encode_preview",
        "decode_preview",
        "DeclaredSizeTooLarge",
        "MessageTooLarge",
        "TrailingBytes",
        "ValidationError::ServiceTooLong",
        "varint",
    ] {
        assert!(
            doc.contains(required),
            "Docs/IPC_PROTOCOL.md must document `{required}` (#226)"
        );
    }
}

#[test]
fn test_ipc_protocol_doc_enum_references_resolve() {
    let types = read("crates/protocol/src/types.rs");
    let codec = read("crates/protocol/src/codec.rs");
    let known: Vec<(&str, Vec<String>)> = vec![
        ("RequestKind", enum_variant_names(&types, "RequestKind")),
        ("EventKind", enum_variant_names(&types, "EventKind")),
        ("Verdict", enum_variant_names(&types, "Verdict")),
        ("ReasonClass", enum_variant_names(&types, "ReasonClass")),
        (
            "ValidationError",
            enum_variant_names(&types, "ValidationError"),
        ),
        ("CodecError", enum_variant_names(&codec, "CodecError")),
    ];
    let mut violations = Vec::new();
    for rel in ["Docs/IPC_PROTOCOL.md", "Docs/PAM_MODULE.md"] {
        let doc = read(rel);
        for (enum_name, variants) in &known {
            let needle = format!("{enum_name}::");
            let mut rest = doc.as_str();
            while let Some(pos) = rest.find(&needle) {
                let after = &rest[pos + needle.len()..];
                let ident: String = after
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !ident.is_empty() && !variants.contains(&ident) {
                    violations.push(format!("{rel}: `{enum_name}::{ident}`"));
                }
                rest = after;
            }
        }
    }
    assert!(
        violations.is_empty(),
        "documented enum variants must exist in crates/protocol (#226):\n{}",
        violations.join("\n")
    );
}

#[test]
fn test_ipc_protocol_doc_wire_indices_match_code() {
    let types = read("crates/protocol/src/types.rs");
    let doc = read("Docs/IPC_PROTOCOL.md");
    for enum_name in ["RequestKind", "EventKind", "Verdict", "ReasonClass"] {
        for (variant, index) in enum_variants(&types, enum_name) {
            let row = format!("| `{enum_name}::{variant}` | {index} |");
            assert!(
                doc.contains(&row),
                "Docs/IPC_PROTOCOL.md must list the wire index row `{row}` (#226)"
            );
        }
    }
}

#[test]
fn test_protocol_crate_frame_diagram_uses_postcard_widths() {
    let lib = read("crates/protocol/src/lib.rs");
    for fixed in [
        "uid_hint:u32",
        "service_len:u8",
        "verdict:u8",
        "reason_class:u8",
        "_ns:u64",
    ] {
        assert!(
            !lib.contains(fixed),
            "crates/protocol/src/lib.rs draws `{fixed}` as a fixed width; postcard encodes \
             integers and lengths as varints (#226)"
        );
    }
    assert!(
        lib.contains("varint"),
        "the protocol crate frame diagram must state the postcard varint encoding (#226)"
    );
}

#[test]
fn test_pam_crate_docs_state_no_fixed_deadline_figure() {
    for rel in ["crates/pam/src/lib.rs", "crates/pam/src/ipc.rs"] {
        let text = read(rel);
        for wording in ["200–250", "200-250", "250ms total", "250 ms total"] {
            assert!(
                !text.contains(wording),
                "{rel} restates the obsolete fixed PAM deadline `{wording}`; the deadline is \
                 derived from the clamped `timeout_ms` (#226, ADR 2026-09-30)"
            );
        }
    }
}
