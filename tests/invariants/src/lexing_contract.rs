//! Hardened lexing for the PAM source invariants (GitHub #242, review finding TCI-11).
//!
//! The historical helper `extract_production_code` in `lib.rs` treats any line containing
//! `#[cfg(test)]` as the start of a braced block, so `#[cfg(test)] mod tests;` or a
//! `#[cfg(test)] use ...;` line swallows the production code that follows. It also counts
//! braces inside string and char literals. This module provides [`production_code`], a small
//! Rust lexer that:
//!
//! - blanks comments and the contents of string, raw-string and char literals (lifetimes are
//!   kept), preserving every newline so line numbers stay meaningful;
//! - removes exactly the item annotated by `#[cfg(test)]` / `#[cfg(all(test, ...))]`: an item
//!   that ends with `;` (`mod tests;`, `use ...;`, `const ...;`) is removed up to that `;`,
//!   an item with a body (`mod tests { ... }`, `fn`, `impl`) up to its matching `}`.
//!
//! `scripts/candid_review.sh` embeds the same item rule in awk (single-line literals only) to
//! filter the PAM diff; the end-to-end behaviour is covered by `candid_review_contract`.
//!
//! On top of the extractor, this module adds the PAM checks the text greps missed: the full
//! set of panicking constructs in production code (not only `.unwrap()` / `.expect(`), and
//! the absence of any async runtime in the resolved normal dependency graph of `soos-pam`
//! (not only in the text of `crates/pam/Cargo.toml`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// Replaces comments and literal contents by nothing (keeping the delimiters of string and
/// char literals and every newline).
fn sanitize(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        // Line comment.
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // Block comment (nested, as in Rust).
        if c == '/' && next == Some('*') {
            let mut depth = 1usize;
            i += 2;
            while i < chars.len() && depth > 0 {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    if chars[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            continue;
        }
        // Raw string: r"..." / r#"..."# / br"..." (not preceded by an identifier char).
        let prev_ident = i > 0 && is_ident(chars[i - 1]);
        let raw_start = if !prev_ident && c == 'r' {
            Some(i + 1)
        } else if !prev_ident && c == 'b' && next == Some('r') {
            Some(i + 2)
        } else {
            None
        };
        if let Some(mut j) = raw_start {
            let mut hashes = 0usize;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) == Some(&'"') {
                out.push('"');
                j += 1;
                loop {
                    match chars.get(j) {
                        None => break,
                        Some('"') if (1..=hashes).all(|k| chars.get(j + k) == Some(&'#')) => {
                            j += 1 + hashes;
                            break;
                        }
                        Some('\n') => {
                            out.push('\n');
                            j += 1;
                        }
                        Some(_) => j += 1,
                    }
                }
                out.push('"');
                i = j;
                continue;
            }
        }
        // Ordinary (byte) string.
        if c == '"' {
            out.push('"');
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    i += 1;
                } else if chars[i] == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
            out.push('"');
            i += 1;
            continue;
        }
        // Char literal versus lifetime.
        if c == '\'' {
            if next == Some('\\') {
                let mut j = i + 2;
                while j < chars.len() && chars[j] != '\'' && chars[j] != '\n' {
                    j += 1;
                }
                out.push_str("''");
                i = j + 1;
                continue;
            }
            if next.is_some() && chars.get(i + 2) == Some(&'\'') {
                out.push_str("''");
                i += 3;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// True when the attribute text (whitespace removed) gates its item on `cfg(test)`.
fn is_test_cfg_attribute(attr: &str) -> bool {
    let compact: String = attr.chars().filter(|c| !c.is_whitespace()).collect();
    compact == "#[cfg(test)]" || compact.starts_with("#[cfg(all(test,")
}

/// Returns the production code of a Rust source: comments and literal contents blanked, and
/// every item gated by `#[cfg(test)]` removed exactly (see the module documentation).
/// Newlines are preserved, so line `n` of the result is line `n` of the input.
pub(crate) fn production_code(source: &str) -> String {
    let chars: Vec<char> = sanitize(source).chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    let mut line_start = true;

    while i < chars.len() {
        let c = chars[i];
        if line_start && c == '#' && chars.get(i + 1) == Some(&'[') {
            // Read the whole attribute.
            let mut j = i + 1;
            let mut depth = 0usize;
            while j < chars.len() {
                match chars[j] {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            let attr: String = chars[i..(j + 1).min(chars.len())].iter().collect();
            if is_test_cfg_attribute(&attr) {
                // Skip the attribute, any further attributes, and the gated item.
                let mut k = j + 1;
                let (mut square, mut paren, mut brace) = (0usize, 0usize, 0usize);
                let mut in_body = false;
                while k < chars.len() {
                    let ch = chars[k];
                    if ch == '\n' {
                        out.push('\n');
                    }
                    if in_body {
                        match ch {
                            '{' => brace += 1,
                            '}' => {
                                brace -= 1;
                                if brace == 0 {
                                    k += 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    } else {
                        match ch {
                            '[' => square += 1,
                            ']' => square = square.saturating_sub(1),
                            '(' => paren += 1,
                            ')' => paren = paren.saturating_sub(1),
                            '{' if square == 0 && paren == 0 => {
                                brace = 1;
                                in_body = true;
                            }
                            ';' if square == 0 && paren == 0 => {
                                k += 1;
                                break;
                            }
                            _ => {}
                        }
                    }
                    k += 1;
                }
                i = k;
                line_start = false;
                continue;
            }
            out.push_str(&attr);
            i = j + 1;
            line_start = false;
            continue;
        }
        out.push(c);
        if c == '\n' {
            line_start = true;
        } else if !c.is_whitespace() {
            line_start = false;
        }
        i += 1;
    }
    out
}

/// Recursively collects `.rs` files below `dir`.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Panicking or unfinished constructs forbidden in PAM production code.
const PAM_FORBIDDEN_CONSTRUCTS: [&str; 7] = [
    ".unwrap()",
    ".expect(",
    "panic!(",
    "unreachable!(",
    "todo!(",
    "unimplemented!(",
    "panic_any(",
];

/// Clippy lints that must never be silenced in PAM production code.
const PAM_FORBIDDEN_ALLOWS: [&str; 7] = [
    "clippy::unwrap_used",
    "clippy::expect_used",
    "clippy::panic",
    "clippy::unreachable",
    "clippy::todo",
    "clippy::unimplemented",
    "clippy::indexing_slicing",
];

/// The feature-gated fault-injection hook (GitHub #148) exists to panic on purpose; it is
/// compiled only with `--features fault-injection` and never shipped.
const PAM_PANIC_EXEMPT_FILES: [&str; 1] = ["fault_injection.rs"];

/// Every forbidden construct or lint allowance found in PAM production code, as
/// `file:line: code`.
fn pam_panic_violations() -> Vec<String> {
    let pam_src = workspace_root().join("crates").join("pam").join("src");
    assert!(pam_src.is_dir(), "missing {}", pam_src.display());
    let mut files = Vec::new();
    collect_rs_files(&pam_src, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no PAM source file found");

    let mut violations = Vec::new();
    for file in files {
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if PAM_PANIC_EXEMPT_FILES.contains(&name) {
            continue;
        }
        let source = fs::read_to_string(&file).expect("read PAM source");
        for (idx, line) in production_code(&source).lines().enumerate() {
            let hit = PAM_FORBIDDEN_CONSTRUCTS
                .iter()
                .chain(PAM_FORBIDDEN_ALLOWS.iter())
                .any(|needle| line.contains(needle));
            if hit {
                violations.push(format!("{}:{}: {}", file.display(), idx + 1, line.trim()));
            }
        }
    }
    violations
}

/// Async runtimes that must never be linked into `pam_soos.so`.
const ASYNC_RUNTIMES: [&str; 5] = ["tokio", "async-std", "smol", "async-io", "async-executor"];

/// Package names in the resolved normal (non-dev, non-build) dependency graph of `package`,
/// for every target platform and every feature.
fn normal_dependency_packages(package: &str) -> Vec<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| String::from("cargo"));
    let out = Command::new(cargo)
        .current_dir(workspace_root())
        .args([
            "tree",
            "--locked",
            "-p",
            package,
            "-e",
            "normal",
            "--target",
            "all",
            "--all-features",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ])
        .output()
        .expect("run cargo tree");
    assert!(
        out.status.success(),
        "cargo tree failed for {package} (fail-closed): {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(String::from)
        .collect();
    names.sort();
    names.dedup();
    names
}

#[test]
fn test_fil_extractor_keeps_code_after_cfg_test_mod_declaration() {
    let src = "#[cfg(test)]\nmod tests;\n\nfn production() {\n    let v = maybe().unwrap();\n}\n";
    let prod = production_code(src);
    assert!(
        prod.contains("maybe().unwrap()"),
        "production code after `#[cfg(test)] mod tests;` was swallowed:\n{prod}"
    );
    assert!(
        !prod.contains("mod tests"),
        "the gated declaration must be removed:\n{prod}"
    );
}

#[test]
fn test_fil_extractor_keeps_code_after_cfg_test_use() {
    let src = "#[cfg(test)]\nuse std::collections::HashMap;\n#[cfg(test)]\nuse std::{fmt, io};\n\
               pub fn production() -> u8 {\n    other().expect(\"x\")\n}\n";
    let prod = production_code(src);
    assert!(
        prod.contains("other().expect("),
        "production code after a `#[cfg(test)] use` line was swallowed:\n{prod}"
    );
    assert!(
        !prod.contains("HashMap") && !prod.contains("fmt, io"),
        "{prod}"
    );
}

#[test]
fn test_fil_extractor_strips_test_module_with_multiline_attributes() {
    let src = "fn keep() {}\n\n#[cfg(test)]\n#[allow(\n    clippy::unwrap_used,\n    \
               reason = \"tests\"\n)]\nmod tests {\n    #[test]\n    fn t() {\n        \
               Some(1).unwrap();\n    }\n}\n\nfn keep_after() {}\n";
    let prod = production_code(src);
    assert!(
        prod.contains("fn keep()") && prod.contains("fn keep_after()"),
        "{prod}"
    );
    assert!(!prod.contains("unwrap"), "test module body leaked:\n{prod}");
    assert_eq!(
        prod.lines().count(),
        src.lines().count(),
        "line numbers must be preserved"
    );
}

#[test]
fn test_fil_extractor_ignores_braces_in_literals_and_comments() {
    let src = "#[cfg(test)]\nmod tests {\n    const OPEN: &str = \"{{{\";\n    \
               const C: char = '{';\n    // }\n    const R: &str = r#\"}\"#;\n    \
               fn t<'a>(x: &'a str) -> &'a str { x }\n}\n\nfn production() {\n    \
               value.unwrap();\n}\n";
    let prod = production_code(src);
    assert!(
        prod.contains("value.unwrap()"),
        "braces inside literals or comments desynchronized the extractor:\n{prod}"
    );
    assert!(!prod.contains("OPEN") && !prod.contains("fn t<"), "{prod}");
}

#[test]
fn test_fil_extractor_strips_cfg_test_items_but_not_cfg_not_test() {
    let src = "#[cfg(test)]\nfn helper() -> u8 { Some(1).unwrap() }\n#[cfg(all(test, unix))]\n\
               const X: [u8; 2] = [1, 2];\n#[cfg(not(test))]\nfn shipped() { a.unwrap(); }\n";
    let prod = production_code(src);
    assert!(
        !prod.contains("helper") && !prod.contains("const X"),
        "{prod}"
    );
    assert!(prod.contains("fn shipped() { a.unwrap(); }"), "{prod}");
}

#[test]
fn test_fil_pam_production_code_has_no_panicking_constructs() {
    let violations = pam_panic_violations();
    assert!(
        violations.is_empty(),
        "SECURITY INVARIANT VIOLATION: panicking construct or silenced panic lint in PAM \
         production code:\n{}",
        violations.join("\n")
    );
}

#[test]
fn test_fil_pam_dependency_graph_has_no_async_runtime() {
    // Non-vacuity: the same query must see Tokio in the daemon's graph.
    let daemon = normal_dependency_packages("soos-daemon");
    assert!(
        daemon.iter().any(|name| name == "tokio"),
        "cargo tree query is vacuous: tokio not found in soos-daemon's graph: {daemon:?}"
    );
    let pam = normal_dependency_packages("soos-pam");
    assert!(pam.iter().any(|name| name == "soos-protocol"), "{pam:?}");
    let runtimes: Vec<&String> = pam
        .iter()
        .filter(|name| ASYNC_RUNTIMES.contains(&name.as_str()))
        .collect();
    assert!(
        runtimes.is_empty(),
        "ARCHITECTURE VIOLATION: async runtime reachable from soos-pam (directly or \
         transitively): {runtimes:?}"
    );
}
