//! Static contracts of the active-illumination on/off differential liveness cues (GitHub #431 =
//! PR 2 of #353; spec `AI/architect_spec_ir_active_cues.md`; matrix IRA35):
//!
//! - `crates/vision/src/ir_active.rs` is zero I/O and silent: no file, network, process or
//!   environment access, no logging or printing macro, no `unsafe`;
//! - the face-crop copies are `Zeroizing`;
//! - the result type is a veto-only proof: no `is_live`, no public field on
//!   `IrActiveCuesClear`;
//! - the module is declared by `soos-vision`, which keeps `#![forbid(unsafe_code)]`;
//! - the ADR and the crate documentation record the cues as provisional and uncalibrated.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::PathBuf;

const MODULE: &str = "crates/vision/src/ir_active.rs";
const ADR_TITLE: &str = "Active-Illumination Differential Liveness Cues";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Lines that are not `//` comments (doc comments included), with their 1-based numbers.
fn code_lines(text: &str) -> Vec<(usize, &str)> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim_start().starts_with("//"))
        .map(|(i, l)| (i + 1, l))
        .collect()
}

#[test]
fn test_ira_active_module_is_zero_io_and_silent() {
    let text = read(MODULE);
    let code = code_lines(&text);
    assert!(code.len() > 100, "{MODULE} looks truncated or missing");
    let banned = [
        "std::fs",
        "std::net",
        "std::process",
        "std::env",
        "std::io",
        "File::",
        "tracing",
        "log::",
        "println!",
        "eprintln!",
        "print!",
        "eprint!",
        "dbg!",
        "unsafe",
        "opencv",
    ];
    let mut violations = Vec::new();
    for (n, line) in &code {
        for token in banned {
            if line.contains(token) {
                violations.push(format!("{MODULE}:{n}: `{token}` in `{}`", line.trim()));
            }
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn test_ira_active_crop_copies_are_zeroizing() {
    let text = read(MODULE);
    assert!(
        text.contains("use zeroize::Zeroizing;"),
        "{MODULE} must import Zeroizing"
    );
    let zeroizing = code_lines(&text)
        .iter()
        .filter(|(_, l)| l.contains("Zeroizing<Vec<u8>>"))
        .count();
    assert!(
        zeroizing >= 1,
        "{MODULE} must hold the face-crop copies in Zeroizing<Vec<u8>>"
    );
}

#[test]
fn test_ira_active_result_is_veto_only() {
    let text = read(MODULE);
    for token in ["is_live", "fn grant", "pub live"] {
        assert!(
            !code_lines(&text).iter().any(|(_, l)| l.contains(token)),
            "{MODULE} must not expose `{token}`: a pass is never a liveness decision"
        );
    }
    let start = text
        .find("pub struct IrActiveCuesClear")
        .expect("IrActiveCuesClear is declared");
    let body = &text[start..];
    let end = body.find('}').expect("struct body ends");
    let fields = &body[..end];
    assert!(
        !fields
            .lines()
            .skip(1)
            .any(|l| l.trim_start().starts_with("pub ")),
        "IrActiveCuesClear must have no public field (it can only be built by the evaluation)"
    );
    let lib = read("crates/vision/src/lib.rs");
    assert!(lib.contains("#![forbid(unsafe_code)]"));
    assert!(lib.contains("pub mod ir_active;"));
}

#[test]
fn test_ira_active_adr_and_docs_are_recorded() {
    let decisions = read("AI/DECISIONS.md");
    let adr = decisions
        .lines()
        .find(|l| l.contains(ADR_TITLE))
        .unwrap_or_else(|| panic!("AI/DECISIONS.md must record `{ADR_TITLE}`"));
    for needle in [
        "2026-09-30",
        "ir_active",
        "uncalibrated",
        "provisional",
        "never grants",
    ] {
        assert!(adr.contains(needle), "the ADR must mention `{needle}`");
    }
    let docs = read("Docs/VISION_CRATE.md");
    for needle in [
        "ir_active.rs",
        "evaluate_ir_active_cues",
        "IrActiveConfig",
        "provisional",
        "uncalibrated",
    ] {
        assert!(
            docs.contains(needle),
            "Docs/VISION_CRATE.md must mention `{needle}`"
        );
    }
}
