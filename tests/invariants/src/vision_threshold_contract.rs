//! Vision threshold and documentation contract invariants (verification matrix rows VTD6-VTD8).
//!
//! - GitHub #251 (VIS-09) and #215 (PAD-10): the face-detector and PAD thresholds of
//!   `soos-daemon`, `soos-enroll` and `soos-gui` come from `VisionPipelineConfig` (named
//!   constants of `soos-vision`), never from per-binary numeric literals, and the GUI never
//!   hardcodes its own liveness display threshold.
//! - GitHub #250 (VIS-08): `Docs/VISION_CRATE.md` and `Docs/INFERENCE_ORT_CRATE.md` list every
//!   source module of their crate and state the code match threshold, and the ADR describing
//!   SCRFD output parsing no longer claims ordinal grouping without a supersession marker.

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

/// Production crates that build an ORT face detector or PAD detector.
const BINARY_CRATES: [&str; 3] = [
    "crates/daemon/src",
    "crates/gui/src",
    "crates/enrollment-cli/src",
];

/// Detector / PAD construction calls whose arguments must carry no numeric literal.
const CONSTRUCTORS: [&str; 3] = [
    "OrtScrfdDetector::new(",
    "OrtPadDetector::new(",
    "build_pad_detector(",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("Unable to locate workspace root directory")
        .to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Source text before the first `#[cfg(test)]` (unit-test modules may use literals).
fn production_part(source: &str) -> &str {
    source.find("#[cfg(test)]").map_or(source, |i| &source[..i])
}

/// Arguments of the call starting right after `open` (the index after the opening paren).
fn call_arguments(source: &str, open: usize) -> &str {
    let mut depth = 1usize;
    for (i, c) in source[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open..open + i];
                }
            }
            _ => {}
        }
    }
    &source[open..]
}

/// True when `text` contains a floating-point literal such as `0.45` or `.80`.
fn contains_float_literal(text: &str) -> bool {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .any(|token| {
            let starts_numeric = token
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c == '.');
            starts_numeric
                && token
                    .as_bytes()
                    .windows(2)
                    .any(|w| w[0] == b'.' && w[1].is_ascii_digit())
        })
}

/// Invariant (GitHub #251, #215): no production detector/PAD construction site of the three
/// binaries passes a threshold literal; every value comes from `VisionPipelineConfig`.
#[test]
fn test_binaries_build_detectors_from_pipeline_config() {
    let root = workspace_root();
    let mut violations = Vec::new();
    let mut sites = 0usize;
    for dir in BINARY_CRATES {
        let mut files = Vec::new();
        rust_files(&root.join(dir), &mut files);
        for file in files {
            let source = fs::read_to_string(&file).expect("read source");
            let production = production_part(&source);
            for ctor in CONSTRUCTORS {
                for (pos, _) in production.match_indices(ctor) {
                    // Skip the definition `fn build_pad_detector(` itself.
                    if production[..pos].ends_with("fn ") {
                        continue;
                    }
                    let args = call_arguments(production, pos + ctor.len());
                    sites += 1;
                    if contains_float_literal(args) {
                        violations.push(format!(
                            "{}: `{ctor}{})` passes a threshold literal",
                            file.strip_prefix(&root).unwrap_or(&file).display(),
                            args.trim()
                        ));
                    }
                }
            }
        }
    }
    assert!(
        sites >= 6,
        "expected detector and PAD construction sites in daemon, gui and enrollment-cli, found {sites}"
    );
    assert!(
        violations.is_empty(),
        "detector/PAD thresholds must come from VisionPipelineConfig (GitHub #251, #215):\n{}",
        violations.join("\n")
    );
}

/// Invariant (GitHub #278, SFC12): no production source of the three binaries re-types the
/// cosine match threshold as a literal (the GUI used `0.70f32` while the default moved);
/// every value comes from `VisionPipelineConfig::match_threshold` / `DEFAULT_MATCH_THRESHOLD`.
#[test]
fn test_no_match_threshold_literal_outside_the_constants() {
    let root = workspace_root();
    let mut violations = Vec::new();
    for dir in BINARY_CRATES {
        let mut files = Vec::new();
        rust_files(&root.join(dir), &mut files);
        for file in files {
            let source = fs::read_to_string(&file).expect("read source");
            for line in production_part(&source).lines() {
                if line.contains("match_threshold") && contains_float_literal(line) {
                    violations.push(format!(
                        "{}: {}",
                        file.strip_prefix(&root).unwrap_or(&file).display(),
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "match threshold literals outside the constants (GitHub #278):\n{}",
        violations.join("\n")
    );
}

/// Invariant (GitHub #215): the GUI liveness label and box colour use the pipeline decision,
/// never a hardcoded `score >= <literal>` comparison.
#[test]
fn test_gui_never_hardcodes_pad_display_threshold() {
    let root = workspace_root();
    let mut files = Vec::new();
    rust_files(&root.join("crates/gui/src"), &mut files);
    let mut violations = Vec::new();
    for file in files {
        let source = fs::read_to_string(&file).expect("read source");
        for (n, line) in production_part(&source).lines().enumerate() {
            let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            if let Some(pos) = compact.find("score>=") {
                if contains_float_literal(&compact[pos + "score>=".len()..]) {
                    violations.push(format!(
                        "{}:{}: {}",
                        file.strip_prefix(&root).unwrap_or(&file).display(),
                        n + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "the GUI must use VisionPipelineConfig::pad_passes for liveness display:\n{}",
        violations.join("\n")
    );
}

fn assert_doc_lists_modules(root: &Path, doc: &str, src_dir: &str) {
    let content = fs::read_to_string(root.join(doc)).expect("read crate doc");
    let mut missing = Vec::new();
    for entry in fs::read_dir(root.join(src_dir))
        .expect("read src dir")
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".rs") {
            continue;
        }
        let listed = content
            .lines()
            .any(|l| (l.contains("├── ") || l.contains("└── ")) && l.contains(&name));
        if !listed {
            missing.push(name);
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "{doc} module tree omits source modules of {src_dir} (GitHub #250): {missing:?}"
    );
}

/// Invariant (GitHub #250): the crate documents list every source module.
#[test]
fn test_vision_and_inference_docs_list_every_module() {
    let root = workspace_root();
    assert_doc_lists_modules(&root, "Docs/VISION_CRATE.md", "crates/vision/src");
    assert_doc_lists_modules(
        &root,
        "Docs/INFERENCE_ORT_CRATE.md",
        "crates/inference-ort/src",
    );
}

/// Invariant (GitHub #250): documented match threshold and SCRFD parsing agree with the code.
#[test]
fn test_vision_docs_state_code_thresholds_and_parsing() {
    let root = workspace_root();
    let vision = fs::read_to_string(root.join("Docs/VISION_CRATE.md")).expect("read doc");
    let stale: Vec<&str> = vision
        .lines()
        .filter(|l| l.contains("default") && l.contains("0.45") && !l.contains("nms"))
        .collect();
    assert!(
        stale.is_empty(),
        "Docs/VISION_CRATE.md states a stale 0.45 default (code: DEFAULT_MATCH_THRESHOLD 0.70): {stale:?}"
    );
    assert!(
        vision.contains("DEFAULT_MATCH_THRESHOLD"),
        "Docs/VISION_CRATE.md must name the match threshold constant"
    );

    let decisions = fs::read_to_string(root.join("AI/DECISIONS.md")).expect("read DECISIONS");
    let ordinal: Vec<&str> = decisions
        .lines()
        .filter(|l| l.contains("ordinal grouping") && !l.contains("*(Superseded"))
        .collect();
    assert!(
        ordinal.is_empty(),
        "AI/DECISIONS.md still claims ordinal SCRFD output grouping (code matches by shape): {ordinal:?}"
    );
}

#[cfg(test)]
mod helper_tests {
    use super::contains_float_literal;

    #[test]
    fn test_contains_float_literal_detection() {
        assert!(contains_float_literal("det_session, 0.60, 0.40"));
        assert!(contains_float_literal("pad_session, 0.80"));
        assert!(!contains_float_literal(
            "det_session, config.min_face_confidence"
        ));
        assert!(!contains_float_literal("pad_session, cfg.pad_threshold"));
        assert!(!contains_float_literal("self.config.vision.pad_threshold"));
    }
}
