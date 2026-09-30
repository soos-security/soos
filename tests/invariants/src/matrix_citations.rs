//! Verification-matrix citation invariants (GitHub #187, review TCI-04).
//!
//! `AI/VERIFICATION_MATRIX.md` is the contractual acceptance record. A row marked
//! `✅ Verified` or `☑ Validated` (and every checked `- [x]` global invariant) may only cite
//! evidence that exists in the repository:
//!
//! - a test reference `name`, `module::name`, `crate::module::name` or
//!   `soos-crate::module::name` (`name` starting with `test_`) must resolve to a real test
//!   function (`#[test]`, `#[tokio::test]`, `proptest!`, or a shell/Python test function) in a
//!   file selected by every qualifier (file stem, parent directory, inline `mod`, or package);
//! - a prefix glob `test_prefix_*` must match at least one such test function;
//! - a module glob `module::*` must name a file or module that contains at least one test;
//! - a repository path (`crates/...`, `tests/...`, `scripts/...`, ...) or a bare source file
//!   name (`*.rs`, `*.sh`, `*.py`) must exist; a `.rs` file cited in the evidence columns must
//!   contain at least one test (a production source file is never test evidence).
//!
//! Text inside an italic annotation `*( ... )*` is history (e.g. "renamed from", "never
//! existed") and is not a citation. Brace lists (`Dockerfile.{ubuntu,fedora}`), escaped table
//! pipes (`\|`) and "and siblings" wording are handled by the parser, which is self-tested.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const MATRIX: &str = "AI/VERIFICATION_MATRIX.md";

/// Status prefixes that claim the criterion is proven by evidence.
const CLAIMED_STATUSES: [&str; 2] = ["✅ Verified", "☑ Validated"];

/// Grammar placeholders used by rows that describe the citation format itself (PTF5).
const PLACEHOLDER_NAMES: [&str; 1] = ["test_name"];

/// Directories never scanned (build output, VCS metadata, local agent state).
const SKIPPED_DIRS: [&str; 4] = [".git", "target", ".claude", "node_modules"];

/// Lower bound on the number of checked citations, so the scan can never pass vacuously
/// because the parser silently stopped recognizing the matrix format.
const MIN_CHECKED_CITATIONS: usize = 900;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// One repository source file that may define tests.
struct SourceFile {
    rel: String,
    stem: String,
    parent: String,
    ext: String,
    text: String,
    /// Test functions defined in the file (computed once).
    tests: Vec<String>,
    /// Inline or declared modules (`mod name {` / `mod name;`).
    mods: BTreeSet<String>,
}

/// Snapshot of the repository used to resolve citations.
struct Repo {
    /// Every file and directory, repository-relative with `/` separators.
    entries: BTreeSet<String>,
    /// Top-level entry names (a cited path must start with one of them).
    top_level: BTreeSet<String>,
    sources: Vec<SourceFile>,
    /// Package name (`soos-daemon`, also `soos_daemon`) -> crate directory prefix.
    packages: BTreeMap<String, String>,
}

impl Repo {
    fn load(root: &Path) -> Self {
        let mut entries = BTreeSet::new();
        let mut sources = Vec::new();
        walk(root, root, &mut entries, &mut sources);
        let top_level = entries
            .iter()
            .map(|e| e.split('/').next().unwrap_or(e).to_string())
            .collect();
        let mut packages = BTreeMap::new();
        for manifest in entries.iter().filter(|e| e.ends_with("Cargo.toml")) {
            let Some(dir) = manifest.strip_suffix("Cargo.toml") else {
                continue;
            };
            if dir.is_empty() {
                continue;
            }
            let text = fs::read_to_string(root.join(manifest)).unwrap_or_default();
            if let Some(name) = text
                .lines()
                .find_map(|l| l.trim().strip_prefix("name = \""))
                .and_then(|rest| rest.split('"').next())
            {
                packages.insert(name.to_string(), dir.to_string());
                packages.insert(name.replace('-', "_"), dir.to_string());
            }
        }
        Self {
            entries,
            top_level,
            sources,
            packages,
        }
    }
}

fn walk(root: &Path, dir: &Path, entries: &mut BTreeSet<String>, sources: &mut Vec<SourceFile>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if path.is_dir() {
            if SKIPPED_DIRS.contains(&name.as_str()) {
                continue;
            }
            entries.insert(rel);
            walk(root, &path, entries, sources);
            continue;
        }
        entries.insert(rel.clone());
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();
        if ["rs", "sh", "py"].contains(&ext.as_str()) {
            let text = fs::read_to_string(&path).unwrap_or_default();
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let parent = path
                .parent()
                .and_then(Path::file_name)
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let mut file = SourceFile {
                rel,
                stem,
                parent,
                ext,
                text,
                tests: Vec::new(),
                mods: BTreeSet::new(),
            };
            file.tests = test_functions(&file);
            file.mods = declared_modules(&file.text);
            sources.push(file);
        }
    }
}

/// A claimed row of the matrix.
#[derive(Debug)]
struct ClaimedRow {
    line_no: usize,
    id: String,
    /// Criterion text (never test evidence by itself).
    criterion: String,
    /// Test method and status columns.
    evidence: String,
}

/// Splits a Markdown table line on unescaped `|`.
fn split_cells(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                current.push('|');
                chars.next();
            }
            '|' => cells.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    cells.push(current);
    cells
        .into_iter()
        .map(|c| c.trim().to_string())
        .collect::<Vec<_>>()
}

/// Returns every row whose status claims evidence, plus checked global invariants.
fn claimed_rows(matrix: &str) -> Vec<ClaimedRow> {
    let mut rows = Vec::new();
    for (idx, line) in matrix.lines().enumerate() {
        let line_no = idx + 1;
        if let Some(rest) = line.strip_prefix("- [x]") {
            rows.push(ClaimedRow {
                line_no,
                id: format!("global-invariant@L{line_no}"),
                criterion: String::new(),
                evidence: rest.to_string(),
            });
            continue;
        }
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<String> = split_cells(line)
            .into_iter()
            .filter(|c| !c.is_empty())
            .collect();
        if cells.len() < 3 {
            continue;
        }
        let status = cells.last().map(String::as_str).unwrap_or_default();
        if !CLAIMED_STATUSES.iter().any(|s| status.starts_with(s)) {
            continue;
        }
        rows.push(ClaimedRow {
            line_no,
            id: cells[0].clone(),
            criterion: cells[1].clone(),
            evidence: cells[2..].join(" | "),
        });
    }
    rows
}

/// Removes italic history annotations `*( ... )*`.
fn strip_annotations(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("*(") {
        out.push_str(&rest[..start]);
        match rest[start..].find(")*") {
            Some(end) => rest = &rest[start + end + 2..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Contents of the inline code spans of `text`.
fn code_spans(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).collect()
}

/// Expands one level of `{a,b,c}` brace lists (recursively for several lists).
fn expand_braces(token: &str) -> Vec<String> {
    let (Some(open), Some(close)) = (token.find('{'), token.find('}')) else {
        return vec![token.to_string()];
    };
    if close < open {
        return vec![token.to_string()];
    }
    let (head, tail) = (&token[..open], &token[close + 1..]);
    token[open + 1..close]
        .split(',')
        .flat_map(|alt| expand_braces(&format!("{head}{}{tail}", alt.trim())))
        .collect()
}

/// Splits a code span into candidate citation tokens.
fn tokens(span: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in span.split_whitespace() {
        // Split on commas that are not inside a brace list.
        let mut depth = 0_i32;
        let mut current = String::new();
        let mut parts = Vec::new();
        for c in word.chars() {
            match c {
                '{' => {
                    depth += 1;
                    current.push(c);
                }
                '}' => {
                    depth -= 1;
                    current.push(c);
                }
                ',' if depth <= 0 => parts.push(std::mem::take(&mut current)),
                _ => current.push(c),
            }
        }
        parts.push(current);
        for part in parts {
            let trimmed = part.trim_matches(|c: char| "()[];,\"'".contains(c));
            let trimmed = trimmed.trim_end_matches(['.', ':']);
            if trimmed.is_empty() {
                continue;
            }
            out.extend(expand_braces(trimmed));
        }
    }
    out
}

/// A resolvable citation.
#[derive(Debug, PartialEq, Eq)]
enum Citation {
    /// `qualifiers::name` where `name` is `test_*`, `test_prefix_*` or `*`.
    Test {
        qualifiers: Vec<String>,
        name: String,
    },
    /// Repository-relative path (may contain `*` / `**` globs).
    Path(String),
    /// Bare source file name such as `debug_vision_tests.rs`.
    FileName(String),
}

fn is_ident_glob(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '*')
}

fn classify(token: &str, top_level: &BTreeSet<String>) -> Option<Citation> {
    if token.contains('/') {
        let path = token.split(['#', ':']).next().unwrap_or(token);
        let first = path.split('/').next().unwrap_or_default();
        if top_level.contains(first) {
            return Some(Citation::Path(path.trim_end_matches('/').to_string()));
        }
        return None;
    }
    if !token.contains("::") {
        if let Some((stem, ext)) = token.rsplit_once('.') {
            let simple = stem
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c));
            if simple && !stem.is_empty() && ["rs", "sh", "py"].contains(&ext) {
                return Some(Citation::FileName(token.to_string()));
            }
        }
    }
    let segments: Vec<&str> = token.split("::").collect();
    let name = *segments.last()?;
    let is_test = name.starts_with("test_") && is_ident_glob(name);
    let is_module_glob = name == "*" && segments.len() > 1;
    if !(is_test || is_module_glob) || PLACEHOLDER_NAMES.contains(&name) {
        return None;
    }
    let qualifiers: Vec<String> = segments[..segments.len() - 1]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if qualifiers.iter().any(|q| {
        q.is_empty()
            || !q
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
    }) {
        return None;
    }
    Some(Citation::Test {
        qualifiers,
        name: name.to_string(),
    })
}

/// Every citation of one claimed row, with a flag telling whether it sits in the evidence
/// columns.
fn citations(row: &ClaimedRow, top_level: &BTreeSet<String>) -> Vec<(Citation, bool)> {
    let mut out = Vec::new();
    for (text, is_evidence) in [(&row.criterion, false), (&row.evidence, true)] {
        let clean = strip_annotations(text);
        for span in code_spans(&clean) {
            for token in tokens(span) {
                if let Some(citation) = classify(&token, top_level) {
                    out.push((citation, is_evidence));
                }
            }
        }
    }
    out
}

/// Names of the test functions defined in one source file.
fn test_functions(file: &SourceFile) -> Vec<String> {
    let lines: Vec<&str> = file.text.lines().collect();
    let mut names = Vec::new();
    match file.ext.as_str() {
        "rs" => {
            let in_proptest = file.text.contains("proptest!");
            for (idx, line) in lines.iter().enumerate() {
                let trimmed = line.trim_start();
                let Some(pos) = trimmed.find("fn ") else {
                    continue;
                };
                let before = &trimmed[..pos];
                let prefix_ok = before
                    .split_whitespace()
                    .all(|w| ["pub", "pub(crate)", "async", "const", "unsafe"].contains(&w));
                if !prefix_ok {
                    continue;
                }
                let name: String = trimmed[pos + 3..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if name.is_empty() {
                    continue;
                }
                if in_proptest || has_test_attribute(&lines, idx) {
                    names.push(name);
                }
            }
        }
        "sh" => {
            for line in &lines {
                let t = line.trim_start();
                let keyword = t.starts_with("function ");
                let t = t.strip_prefix("function ").unwrap_or(t);
                let name: String = t
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() && (keyword || t[name.len()..].trim_start().starts_with("()")) {
                    names.push(name);
                }
            }
        }
        "py" => {
            for line in &lines {
                if let Some(rest) = line.trim_start().strip_prefix("def ") {
                    names.push(
                        rest.chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                            .collect(),
                    );
                }
            }
        }
        _ => {}
    }
    names
}

/// True when the attribute block directly above line `idx` contains a test attribute.
fn has_test_attribute(lines: &[&str], idx: usize) -> bool {
    let mut i = idx;
    while i > 0 {
        i -= 1;
        let t = lines[i].trim();
        if t.is_empty() || t.starts_with("//") {
            continue;
        }
        if t.starts_with("#[") || t.starts_with(")]") || t.starts_with("reason") || t.ends_with(',')
        {
            if t.starts_with("#[") && t.contains("test") && !t.starts_with("#[cfg(test)]") {
                return true;
            }
            continue;
        }
        return false;
    }
    false
}

/// Module names declared in a Rust source (`mod name {` or `mod name;`).
fn declared_modules(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|l| {
            let t = l.trim_start();
            let t = t.strip_prefix("pub ").unwrap_or(t);
            let t = t.strip_prefix("pub(crate) ").unwrap_or(t);
            let rest = t.strip_prefix("mod ")?;
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            let after = rest[name.len()..].trim_start();
            (!name.is_empty() && (after.starts_with('{') || after.starts_with(';'))).then_some(name)
        })
        .collect()
}

fn is_test_file(file: &SourceFile) -> bool {
    !file.tests.is_empty()
}

/// Minimal glob matcher on `/`-separated paths (`*`, `?`, `**`).
fn glob_match(pattern: &str, path: &str) -> bool {
    fn segment(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(b'*'), _) => segment(&p[1..], s) || (!s.is_empty() && segment(p, &s[1..])),
            (Some(b'?'), Some(_)) => segment(&p[1..], &s[1..]),
            (Some(a), Some(b)) if a == b => segment(&p[1..], &s[1..]),
            _ => false,
        }
    }
    fn parts(p: &[&str], s: &[&str]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(&"**"), _) => parts(&p[1..], s) || (!s.is_empty() && parts(p, &s[1..])),
            (Some(a), Some(b)) => segment(a.as_bytes(), b.as_bytes()) && parts(&p[1..], &s[1..]),
            _ => false,
        }
    }
    let p: Vec<&str> = pattern.split('/').collect();
    let s: Vec<&str> = path.split('/').collect();
    parts(&p, &s)
}

/// Resolves one citation; returns the failure reason when it does not resolve.
fn resolve(repo: &Repo, citation: &Citation, is_evidence: bool) -> Result<(), String> {
    match citation {
        Citation::Path(path) => {
            let matched: Vec<&String> = if path.contains(['*', '?']) {
                repo.entries
                    .iter()
                    .filter(|e| glob_match(path, e))
                    .collect()
            } else {
                repo.entries.iter().filter(|e| *e == path).collect()
            };
            if matched.is_empty() {
                return Err("path does not exist".into());
            }
            check_rs_evidence(repo, &matched, is_evidence)
        }
        Citation::FileName(name) => {
            let matched: Vec<&String> = repo
                .entries
                .iter()
                .filter(|e| e.rsplit('/').next() == Some(name.as_str()))
                .collect();
            if matched.is_empty() {
                return Err("no file with this name exists".into());
            }
            check_rs_evidence(repo, &matched, is_evidence)
        }
        Citation::Test { qualifiers, name } => {
            let mut candidates: Vec<&SourceFile> = repo.sources.iter().collect();
            for q in qualifiers {
                if ["tests", "crate", "super", "self"].contains(&q.as_str()) {
                    continue;
                }
                if let Some(dir) = repo.packages.get(q) {
                    candidates.retain(|f| f.rel.starts_with(dir.as_str()));
                } else {
                    candidates.retain(|f| f.stem == *q || f.parent == *q || f.mods.contains(q));
                }
                if candidates.is_empty() {
                    return Err(format!("qualifier `{q}` names no file, module or package"));
                }
            }
            if name == "*" {
                return if candidates.iter().any(|f| is_test_file(f)) {
                    Ok(())
                } else {
                    Err("module glob matches no file containing a test".into())
                };
            }
            let found = candidates.iter().any(|f| {
                f.tests.iter().any(|t| match name.strip_suffix('*') {
                    Some(prefix) => t.starts_with(prefix),
                    None => t == name,
                })
            });
            if found {
                Ok(())
            } else {
                Err("no test function with this name exists in the qualified files".into())
            }
        }
    }
}

fn check_rs_evidence(repo: &Repo, matched: &[&String], is_evidence: bool) -> Result<(), String> {
    let rs: Vec<&&String> = matched.iter().filter(|m| m.ends_with(".rs")).collect();
    if !is_evidence || rs.is_empty() || rs.len() < matched.len() {
        return Ok(());
    }
    // Test support code (fixtures, helpers) lives under a `tests/` directory and may be cited
    // alongside the tests that use it; a production source without tests is never evidence.
    let acceptable = repo
        .sources
        .iter()
        .filter(|f| rs.iter().any(|m| ***m == f.rel))
        .any(|f| is_test_file(f) || f.rel.starts_with("tests/") || f.rel.contains("/tests/"));
    if acceptable {
        Ok(())
    } else {
        Err("production source cited as test evidence but it contains no test".into())
    }
}

/// Invariant (GitHub #187, TCI-04): every citation of every `✅ Verified` / `☑ Validated`
/// row (and every checked global invariant) of `AI/VERIFICATION_MATRIX.md` resolves to a
/// real test function or an existing repository file.
#[test]
fn test_matrix_claimed_rows_cite_only_existing_evidence() {
    let root = workspace_root();
    let matrix = fs::read_to_string(root.join(MATRIX)).expect("read AI/VERIFICATION_MATRIX.md");
    let repo = Repo::load(&root);
    let rows = claimed_rows(&matrix);
    assert!(rows.len() >= 300, "only {} claimed rows parsed", rows.len());

    let mut checked = 0_usize;
    let mut failures = Vec::new();
    for row in &rows {
        for (citation, is_evidence) in citations(row, &repo.top_level) {
            checked += 1;
            if let Err(reason) = resolve(&repo, &citation, is_evidence) {
                failures.push(format!(
                    "{MATRIX}:{} {}: {citation:?}: {reason}",
                    row.line_no, row.id
                ));
            }
        }
    }
    assert!(
        checked >= MIN_CHECKED_CITATIONS,
        "only {checked} citations checked; the matrix parser no longer recognizes the format"
    );
    assert!(
        failures.is_empty(),
        "{} of {checked} verification-matrix citations do not resolve (point the row to the real \
         test, or downgrade it to `⬜ Pending` with a reason; never invent evidence):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Invariant (TCI-04): a claimed row whose criterion opens with `**Superseded` contradicts
/// itself; a superseded row must carry a non-claimed status.
#[test]
fn test_matrix_superseded_rows_do_not_claim_verification() {
    let matrix = fs::read_to_string(workspace_root().join(MATRIX)).expect("read matrix");
    let offenders: Vec<String> = claimed_rows(&matrix)
        .into_iter()
        .filter(|r| r.criterion.starts_with("**Superseded"))
        .map(|r| format!("L{} {}", r.line_no, r.id))
        .collect();
    assert!(
        offenders.is_empty(),
        "superseded rows still claim verification: {offenders:?}"
    );
}

/// Invariant (TCI-04): claimed rows state the daemon decision budget exactly as the code
/// constant `DECISION_BUDGET_MS` (D7 used to say 150 ms while the code says 900 ms).
#[test]
fn test_matrix_decision_budget_matches_code_constant() {
    let root = workspace_root();
    let pipeline =
        fs::read_to_string(root.join("crates/daemon/src/pipeline.rs")).expect("pipeline.rs");
    let budget: u64 = pipeline
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("pub const DECISION_BUDGET_MS: u64 = ")
        })
        .and_then(|v| v.trim_end_matches(';').replace('_', "").parse().ok())
        .expect("DECISION_BUDGET_MS constant");
    let matrix = fs::read_to_string(root.join(MATRIX)).expect("read matrix");
    let mut stated = 0_usize;
    let mut drift = Vec::new();
    for row in claimed_rows(&matrix) {
        let text = strip_annotations(&format!("{} {}", row.criterion, row.evidence));
        for value in decision_budget_claims(&text) {
            stated += 1;
            if value != budget {
                drift.push(format!("L{} {}: {value} ms", row.line_no, row.id));
            }
        }
    }
    assert!(stated >= 2, "expected the decision budget to be stated");
    assert!(
        drift.is_empty(),
        "DECISION_BUDGET_MS = {budget} but claimed rows state: {drift:?}"
    );
}

/// Millisecond values written directly before or after "decision budget".
fn decision_budget_claims(text: &str) -> Vec<u64> {
    let lower = text.to_lowercase();
    let mut out = Vec::new();
    for (pos, _) in lower.match_indices("decision budget") {
        let after = lower[pos + "decision budget".len()..].trim_start();
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() && after[digits.len()..].trim_start().starts_with("ms") {
            out.extend(digits.parse::<u64>().ok());
            continue;
        }
        let before = lower[..pos].trim_end();
        if let Some(num) = before.strip_suffix("ms") {
            let num = num.trim_end();
            let digits: String = num
                .chars()
                .rev()
                .take_while(char::is_ascii_digit)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            out.extend(digits.parse::<u64>().ok());
        }
    }
    out
}

/// Invariant (TCI-04): `OrtLandmarkDetector` was removed (NGM6); a claimed row may only
/// mention it to record that removal.
#[test]
fn test_matrix_claimed_rows_do_not_list_removed_landmark_detector() {
    let matrix = fs::read_to_string(workspace_root().join(MATRIX)).expect("read matrix");
    let offenders: Vec<String> = claimed_rows(&matrix)
        .into_iter()
        .filter(|r| {
            let text = strip_annotations(&format!("{} {}", r.criterion, r.evidence));
            text.contains("OrtLandmarkDetector") && !text.contains("removed")
        })
        .map(|r| format!("L{} {}", r.line_no, r.id))
        .collect();
    assert!(
        offenders.is_empty(),
        "rows list the removed OrtLandmarkDetector: {offenders:?}"
    );
}

/// Self-test of the row and citation parser, so the invariant above cannot pass vacuously.
#[test]
fn test_matrix_citation_parser_recognizes_every_citation_form() {
    let top: BTreeSet<String> = ["crates", "tests", "Docs", "Dockerfile"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let matrix = "\
| # | Criterion | Test Method | Status |
|---|---|---|---|
| X1 | uses `O_CREAT \\| O_EXCL` and `crates/pam/src/lib.rs` | Unit (`a_tests::test_one`, `test_two` and siblings, `soos-invariants::pad_contract::test_three`, `resolver_tests::*`, `test_cli_parse_*`) *(renamed from `test_gone`)* | ✅ Verified |
| X2 | row | `tests/docker/Dockerfile.{ubuntu,fedora}`, `debug_vision_tests.rs`, `module::test_name` | ☑ Validated |
| X3 | pending | `test_missing` | ⬜ Pending (spec ✅ Verified) |
- [x] global (`test_global`, `/var/lib/soos`, `state/.tmp.*`)
";
    let rows = claimed_rows(matrix);
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["X1", "X2", "global-invariant@L6"]);
    assert_eq!(
        rows[0].criterion,
        "uses `O_CREAT | O_EXCL` and `crates/pam/src/lib.rs`"
    );

    let t = |q: &[&str], n: &str| Citation::Test {
        qualifiers: q.iter().map(|s| s.to_string()).collect(),
        name: n.to_string(),
    };
    let got: Vec<(Citation, bool)> = rows.iter().flat_map(|r| citations(r, &top)).collect();
    let expected = vec![
        (Citation::Path("crates/pam/src/lib.rs".into()), false),
        (t(&["a_tests"], "test_one"), true),
        (t(&[], "test_two"), true),
        (t(&["soos-invariants", "pad_contract"], "test_three"), true),
        (t(&["resolver_tests"], "*"), true),
        (t(&[], "test_cli_parse_*"), true),
        (
            Citation::Path("tests/docker/Dockerfile.ubuntu".into()),
            true,
        ),
        (
            Citation::Path("tests/docker/Dockerfile.fedora".into()),
            true,
        ),
        (Citation::FileName("debug_vision_tests.rs".into()), true),
        (t(&[], "test_global"), true),
    ];
    assert_eq!(got, expected);

    assert!(glob_match("Docs/**", "Docs/a/b.md"));
    assert!(glob_match("crates/*/tests/*.rs", "crates/pam/tests/x.rs"));
    assert!(!glob_match("crates/*/tests/*.rs", "crates/pam/src/x.rs"));
    assert_eq!(decision_budget_claims("a 900ms decision budget"), [900]);
    assert_eq!(
        decision_budget_claims("decision budget 150 ms, wake"),
        [150]
    );
    assert!(decision_budget_claims("dynamic decision budget bounding").is_empty());
    let mods = declared_modules("mod tests {\npub mod a_b;\n    mod c { }\nuse mod_x;\n");
    assert_eq!(mods.into_iter().collect::<Vec<_>>(), ["a_b", "c", "tests"]);
}

/// Self-test of test-function detection (attributes, async, proptest, helpers, shell).
#[test]
fn test_matrix_citation_parser_detects_test_functions_only() {
    let rs = SourceFile {
        rel: "crates/x/tests/a_tests.rs".into(),
        stem: "a_tests".into(),
        parent: "tests".into(),
        ext: "rs".into(),
        text: "fn test_helper() {}\n\n#[test]\nfn test_plain() {}\n\n#[tokio::test]\n/// doc\nasync fn test_async() {}\n\n#[cfg(test)]\nmod m {\n    #[test]\n    #[should_panic]\n    fn test_nested() {}\n}\n".into(),
        tests: Vec::new(),
        mods: BTreeSet::new(),
    };
    assert_eq!(
        test_functions(&rs),
        ["test_plain", "test_async", "test_nested"]
    );
    let sh = SourceFile {
        rel: "tests/docker/t.sh".into(),
        stem: "t".into(),
        parent: "docker".into(),
        ext: "sh".into(),
        text: "test_one() {\n}\nfunction test_two {\n}\nverify_x () {\n}\n".into(),
        tests: Vec::new(),
        mods: BTreeSet::new(),
    };
    assert_eq!(test_functions(&sh), ["test_one", "test_two", "verify_x"]);
}
