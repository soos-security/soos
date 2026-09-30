//! Anti-spoofing (PAD) contract invariants.
//!
//! - GitHub #171 (PAD-05): the MiniFASNetV2 class-order contract must be stated one way only.
//!   Every living document that states a live class index or a class ordering must agree with
//!   the code constant `DEFAULT_MINIFASNET_LIVE_CLASS_INDEX`, and the PAD verification-matrix
//!   rows must cite tests that exist.
//! - GitHub #170 (PAD-04): operator-supplied thresholds must never bypass validation.

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

/// Marker that exempts a historical line from the class-contract scan. It must be written
/// on the line itself, e.g. `*(Superseded by ADR 2026-09-29 ...)*`.
const SUPERSEDED_MARKER: &str = "*(Superseded";

/// Living documents (not walkthroughs, not dated review reports) that may state the contract.
const CONTRACT_FILES: [&str; 6] = [
    "AGENTS.md",
    "AI/ARCHITECTURE.md",
    "AI/DECISIONS.md",
    "AI/VERIFICATION_MATRIX.md",
    "AI/BACKLOG.md",
    "models/README.md",
];

/// Living document directories scanned recursively for `*.md`.
const CONTRACT_DIRS: [&str; 2] = ["Docs", ".agents/skills"];

/// Verification-matrix rows whose test evidence backs the PAD security criteria.
const PAD_MATRIX_ROWS: [&str; 9] = [
    "PAD1", "NGM9", "NGM10", "ASG1", "ASG2", "PLC1", "PLC2", "PLC3", "PTF1",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("Unable to locate workspace root directory")
        .to_path_buf()
}

fn collect_with_extension(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_with_extension(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

/// Reads the live class index from its single definition in `crates/inference-ort/src/pad.rs`.
fn code_live_class_index(root: &Path) -> usize {
    let source = fs::read_to_string(root.join("crates/inference-ort/src/pad.rs"))
        .expect("read crates/inference-ort/src/pad.rs");
    let prefix = "pub const DEFAULT_MINIFASNET_LIVE_CLASS_INDEX: usize = ";
    let definitions: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with(prefix))
        .collect();
    assert_eq!(
        definitions.len(),
        1,
        "DEFAULT_MINIFASNET_LIVE_CLASS_INDEX must be defined exactly once"
    );
    definitions[0][prefix.len()..]
        .trim_end_matches(';')
        .trim()
        .parse()
        .expect("DEFAULT_MINIFASNET_LIVE_CLASS_INDEX must be an integer literal")
}

/// First ASCII digit after `rest`, skipping only decoration (spaces, backticks, bold markers,
/// `=`, `(`, `:`). Returns `None` when anything else comes first.
fn leading_digit(rest: &str) -> Option<usize> {
    for c in rest.chars() {
        match c {
            ' ' | '`' | '*' | '=' | '(' | ':' => {}
            d if d.is_ascii_digit() => return d.to_digit(10).map(|v| v as usize),
            _ => return None,
        }
    }
    None
}

/// Extracts every live-class-index claim stated on one (lower-cased) line.
fn claims_in_line(line: &str) -> Vec<usize> {
    let mut claims = Vec::new();

    // "live class index 1", "live class index **1**", "default_minifasnet_live_class_index = 1",
    // "`DEFAULT_MINIFASNET_LIVE_CLASS_INDEX` (1".
    for needle in ["live class index", "default_minifasnet_live_class_index"] {
        for (pos, _) in line.match_indices(needle) {
            if let Some(d) = leading_digit(&line[pos + needle.len()..]) {
                claims.push(d);
            }
        }
    }

    // "class 1 = live", "index 0 = live", "class 1 = genuine".
    for needle in ["= live", "= genuine"] {
        for (pos, _) in line.match_indices(needle) {
            let before = line[..pos].trim_end();
            let Some(digit) = before.chars().last().and_then(|c| c.to_digit(10)) else {
                continue;
            };
            let head = before[..before.len() - 1].trim_end();
            if head.ends_with("class") || head.ends_with("index") {
                claims.push(digit as usize);
            }
        }
    }

    // Three-class orderings such as "[PrintPhoto, Live, ScreenReplay]".
    let known = [
        "live",
        "genuine",
        "print",
        "printphoto",
        "replay",
        "screenreplay",
    ];
    let mut search = line;
    while let Some(open) = search.find('[') {
        let after = &search[open + 1..];
        let Some(close) = after.find(']') else {
            break;
        };
        let items: Vec<String> = after[..close]
            .split(',')
            .map(|s| s.trim().trim_matches('`').replace([' ', '_'], ""))
            .collect();
        if items.len() == 3 && items.iter().all(|i| known.contains(&i.as_str())) {
            if let Some(idx) = items.iter().position(|i| i == "live" || i == "genuine") {
                claims.push(idx);
            }
        }
        search = &after[close + 1..];
    }

    claims
}

/// Invariant (GitHub #171, PAD-05): every living document states the MiniFASNetV2 class
/// contract exactly like the code (`DEFAULT_MINIFASNET_LIVE_CLASS_INDEX`). Historical lines
/// must carry the explicit `*(Superseded ...)*` marker to be exempted.
#[test]
fn test_minifasnet_class_contract_prose_matches_code_constant() {
    let root = workspace_root();
    let live_index = code_live_class_index(&root);

    let mut files: Vec<PathBuf> = CONTRACT_FILES.iter().map(|f| root.join(f)).collect();
    for dir in CONTRACT_DIRS {
        collect_with_extension(&root.join(dir), "md", &mut files);
    }

    let mut violations = Vec::new();
    let mut agreeing_claims = 0usize;
    let mut files_with_claims = std::collections::BTreeSet::new();
    for file in &files {
        let content = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("read contract document {}: {e}", file.display()));
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(file)
            .display()
            .to_string();
        for (line_no, line) in content.lines().enumerate() {
            if line.contains(SUPERSEDED_MARKER) {
                continue;
            }
            for claim in claims_in_line(&line.to_lowercase()) {
                if claim == live_index {
                    agreeing_claims += 1;
                    files_with_claims.insert(rel.clone());
                } else {
                    violations.push(format!(
                        "{rel}:{}: states live class index {claim}: {}",
                        line_no + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "MiniFASNetV2 class contract drift: DEFAULT_MINIFASNET_LIVE_CLASS_INDEX = {live_index} \
         ([PrintPhoto, Live, ScreenReplay]) but these living documents disagree (fix the prose, \
         or mark a historical line with `{SUPERSEDED_MARKER} ...)*`):\n{}",
        violations.join("\n")
    );
    for required in [
        "AI/ARCHITECTURE.md",
        "AI/DECISIONS.md",
        "AI/VERIFICATION_MATRIX.md",
        "Docs/INFERENCE_ORT_CRATE.md",
        "models/README.md",
    ] {
        assert!(
            files_with_claims.contains(required),
            "{required} must state the MiniFASNetV2 live class index ({live_index}) explicitly"
        );
    }
    assert!(agreeing_claims >= 5, "expected at least 5 agreeing claims");
}

/// Self-test of the claim extractor so that the scan above cannot pass vacuously.
#[test]
fn test_class_contract_claim_extractor_detects_every_phrasing() {
    let cases: [(&str, &[usize]); 8] = [
        (
            "uses `[live, print, replay]` ordering (class 0 = live)",
            &[0, 0],
        ),
        ("live class index **1** [72, 79]", &[1]),
        ("`default_minifasnet_live_class_index = 2`", &[2]),
        (
            "defaults to `default_minifasnet_live_class_index` (1, x)",
            &[1],
        ),
        (
            "class 0 = print spoof, class 1 = live, class 2 = replay",
            &[1],
        ),
        ("ordering: index 0 = live (configurable)", &[0]),
        ("`[printphoto, live, screenreplay]`", &[1]),
        ("live class index fixed by the constant; [a, b, c]", &[]),
    ];
    for (line, expected) in cases {
        assert_eq!(claims_in_line(line), expected.to_vec(), "line: {line}");
    }
}

/// Collects `module::test_name` references cited in one verification-matrix row.
fn cited_tests(row: &str) -> Vec<(String, String)> {
    let mut refs = Vec::new();
    for chunk in row.split('`').skip(1).step_by(2) {
        let Some((module, name)) = chunk.rsplit_once("::") else {
            continue;
        };
        if !name.starts_with("test_") || name.ends_with('*') || name.contains(' ') {
            continue;
        }
        let module = module.rsplit("::").next().unwrap_or(module);
        refs.push((module.to_string(), name.to_string()));
    }
    refs
}

/// Invariant (GitHub #171, PAD-05): the PAD rows of `AI/VERIFICATION_MATRIX.md` present only
/// tests that exist in the workspace (ASG1 used to cite two non-existent tests as evidence).
#[test]
fn test_pad_matrix_rows_cite_existing_tests() {
    let root = workspace_root();
    let matrix = fs::read_to_string(root.join("AI/VERIFICATION_MATRIX.md"))
        .expect("read AI/VERIFICATION_MATRIX.md");

    let mut rs_files = Vec::new();
    collect_with_extension(&root.join("crates"), "rs", &mut rs_files);
    collect_with_extension(&root.join("tests"), "rs", &mut rs_files);
    let sources: Vec<String> = rs_files
        .iter()
        .map(|p| fs::read_to_string(p).expect("read rust source"))
        .collect();

    let mut missing = Vec::new();
    let mut checked = 0usize;
    for row_id in PAD_MATRIX_ROWS {
        let prefix = format!("| {row_id} |");
        let rows: Vec<&str> = matrix.lines().filter(|l| l.starts_with(&prefix)).collect();
        assert!(
            !rows.is_empty(),
            "AI/VERIFICATION_MATRIX.md must contain row {row_id}"
        );
        for row in rows {
            for (module, name) in cited_tests(row) {
                checked += 1;
                let signature = format!("fn {name}(");
                if !sources.iter().any(|src| src.contains(&signature)) {
                    missing.push(format!("{row_id}: {module}::{name}"));
                }
            }
        }
    }

    assert!(checked > 0, "PAD matrix rows must cite at least one test");
    assert!(
        missing.is_empty(),
        "PAD verification-matrix rows cite tests that do not exist:\n{}",
        missing.join("\n")
    );
}

/// Invariant (GitHub #170, PAD-04): `ThresholdConfig::new_raw` performs no validation and
/// must never be used by production code outside `soos-policy`; operator configuration goes
/// through `ThresholdConfigBuilder::build_with_security_floor`.
#[test]
fn test_thresholds_never_built_unvalidated_outside_policy() {
    let root = workspace_root();
    let crates_dir = root.join("crates");
    let mut files = Vec::new();
    for entry in fs::read_dir(&crates_dir).expect("read crates/") {
        let crate_dir = entry.expect("crate dir entry").path();
        if crate_dir.file_name().is_some_and(|n| n == "policy") {
            continue;
        }
        collect_with_extension(&crate_dir.join("src"), "rs", &mut files);
    }
    assert!(!files.is_empty(), "production sources must be scanned");

    let mut violations = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file).expect("read production source");
        for (line_no, line) in source.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                continue;
            }
            if trimmed.contains("new_raw(") {
                let rel = file.strip_prefix(&root).unwrap_or(file).display();
                violations.push(format!("{rel}:{}: {trimmed}", line_no + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "Thresholds must be validated (build_with_security_floor), never built with new_raw:\n{}",
        violations.join("\n")
    );

    let config_src = fs::read_to_string(crates_dir.join("daemon/src/config.rs"))
        .expect("read crates/daemon/src/config.rs");
    assert!(
        config_src.contains("build_with_security_floor()"),
        "soos-daemon must validate [pipeline.thresholds] with build_with_security_floor()"
    );
}
