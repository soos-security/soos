//! README landing-page contract (GitHub #408, PR 2 of #356; ADR 2026-10-17 "README Restructure
//! as a Product Landing Page"; architect spec `AI/architect_spec_readme_redesign.md` §3; matrix
//! RDM1–RDM8, RDM10–RDM13):
//!
//! - every relative Markdown or HTML link of `README.md` (images included) resolves to an
//!   existing file or directory; `#anchors`, queries, external URLs and code are ignored;
//! - at most 6 badges, each pointing at a real workflow or at the pinned toolchain, and no
//!   release badge while the repository has no tag and no release file;
//! - no overstated wording ("enterprise-grade", "military-grade", "unbreakable", "certified");
//! - the landing-page sections appear in order, with a Mermaid diagram under "How it works";
//! - the security section states the password fallback, the kill switch and the PAD caveat;
//! - the comparison cites upstream Howdy and Microsoft documentation;
//! - while `scripts/setup.sh` turns Funnel, remote unlock or the camera view on, the README never
//!   calls them opt-in and its security section names the setup, passkey and opt-out paths.
//!
//! The image provenance rules (alt text, `Docs/assets/`, allowed badge hosts, manifest) stay in
//! `readme_assets_contract` (RAC10–RAC16). std only.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// Most badges the hero may show (#356 acceptance).
const MAX_BADGES: usize = 6;
/// Words the README must never use, matched as whole words, case-insensitive.
const BANNED_WORDS: [&str; 4] = [
    "enterprise-grade",
    "military-grade",
    "unbreakable",
    "certified",
];
/// Level-2 heading keywords, in the required order.
const REQUIRED_SECTIONS: [&str; 10] = [
    "Why",
    "Features",
    "Screenshots",
    "Quick Start",
    "How it works",
    "Security",
    "Comparison",
    "FAQ",
    "Roadmap",
    "License",
];
/// Needles the security section must contain.
const SECURITY_NEEDLES: [&str; 5] = [
    "PAM_IGNORE",
    "/etc/soos/disabled",
    "#212",
    "AI/ARCHITECTURE.md",
    "Docs/REMOTE_COMPANION.md",
];
/// Upstream sources the comparison section must link.
const COMPARISON_SOURCES: [&str; 2] = [
    "https://github.com/boltgolt/howdy",
    "https://learn.microsoft.com/",
];
/// Badge hosts (same allow-list as `readme_assets_contract`).
const SHIELDS_PREFIX: &str = "https://img.shields.io/";
const WORKFLOW_BADGE_PREFIX: &str = "https://github.com/soos-security/soos/actions/workflows/";
const WORKFLOW_BADGE_SUFFIX: &str = "/badge.svg";
/// A file whose presence counts as release evidence.
const RELEASE_FILE: &str = "CHANGELOG.md";

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
// Markdown helpers
// ---------------------------------------------------------------------------------------

/// The Markdown without fenced code blocks (``` and ~~~) and their fence lines.
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

/// One line without its inline code spans (`` `...` ``).
fn strip_inline_code(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    for c in line.chars() {
        if c == '`' {
            in_code = !in_code;
            continue;
        }
        if !in_code {
            out.push(c);
        }
    }
    out
}

/// The first whitespace-separated token of a link destination, `<>` trimmed.
fn destination(raw: &str) -> String {
    raw.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c| c == '<' || c == '>')
        .to_string()
}

/// Value of the HTML attribute `name` inside one tag (double or single quotes).
fn html_attr_values(line: &str, name: &str) -> Vec<String> {
    let lower = line.to_ascii_lowercase();
    let mut values = Vec::new();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(name) {
        let start = from + pos;
        from = start + name.len();
        let before_ok = start == 0 || lower.as_bytes()[start - 1].is_ascii_whitespace();
        let rest = line[start + name.len()..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value = rest[1..].trim_start();
        let Some(quote) = value.chars().next() else {
            continue;
        };
        if quote == '"' || quote == '\'' {
            if let Some(end) = value[1..].find(quote) {
                values.push(value[1..=end].to_string());
            }
        }
    }
    values
}

/// Every link destination of `markdown`: Markdown links and images (`](target)`, nested badges
/// included), reference definitions (`[label]: target`) and HTML `href` / `src` / `srcset`
/// values. Fenced and inline code are ignored.
fn link_targets(markdown: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for raw_line in strip_fences(markdown).lines() {
        let line = strip_inline_code(raw_line);
        // Inline links and images.
        let mut from = 0;
        while let Some(pos) = line[from..].find("](") {
            let start = from + pos + 2;
            let Some(end) = line[start..].find(')') else {
                break;
            };
            targets.push(destination(&line[start..start + end]));
            from = start + end;
        }
        // Reference definitions.
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') && !trimmed.starts_with("[!") {
            if let Some(close) = trimmed.find("]:") {
                targets.push(destination(&trimmed[close + 2..]));
            }
        }
        // HTML attributes.
        for attr in ["href", "src"] {
            targets.extend(html_attr_values(&line, attr));
        }
        for srcset in html_attr_values(&line, "srcset") {
            targets.extend(srcset.split(',').map(destination));
        }
    }
    targets.retain(|t| !t.is_empty());
    targets
}

/// Whether `target` is an external URL (`scheme:` or protocol-relative `//`).
fn is_external(target: &str) -> bool {
    if target.starts_with("//") {
        return true;
    }
    target.split_once(':').is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
            && scheme
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
    })
}

/// Checks one link destination of a file at the root of `root`.
fn check_relative_target(target: &str, root: &Path) -> Result<(), String> {
    if target.starts_with('#') || is_external(target) {
        return Ok(());
    }
    let path = target.split(['?', '#']).next().unwrap_or("");
    if path.is_empty() {
        return Err(format!("link `{target}` has an empty path"));
    }
    if path.starts_with('/') {
        return Err(format!("link `{target}` must be relative to the README"));
    }
    let mut depth: i32 = 0;
    for component in Path::new(path).components() {
        match component {
            Component::ParentDir => depth -= 1,
            Component::Normal(_) => depth += 1,
            _ => {}
        }
        if depth < 0 {
            return Err(format!("link `{target}` leaves the repository"));
        }
    }
    if root.join(path).exists() {
        Ok(())
    } else {
        Err(format!(
            "link `{target}` does not resolve to an existing file"
        ))
    }
}

/// Every image URL of `markdown` (Markdown `![alt](url)` and HTML `<img src>`), outside code.
fn image_urls(markdown: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for raw_line in strip_fences(markdown).lines() {
        let line = strip_inline_code(raw_line);
        let mut from = 0;
        while let Some(pos) = line[from..].find("![") {
            let start = from + pos + 2;
            let Some(close) = line[start..].find("](") else {
                break;
            };
            let url_start = start + close + 2;
            let Some(end) = line[url_start..].find(')') else {
                break;
            };
            urls.push(destination(&line[url_start..url_start + end]));
            from = url_start + end;
        }
        let lower = line.to_ascii_lowercase();
        let mut from = 0;
        while let Some(pos) = lower[from..].find("<img") {
            let start = from + pos;
            let end = lower[start..]
                .find('>')
                .map_or(line.len(), |e| start + e + 1);
            urls.extend(html_attr_values(&line[start..end], "src"));
            from = end;
        }
    }
    urls
}

/// Whether `url` is a badge (shields.io or this repository's workflow `badge.svg`).
fn is_badge(url: &str) -> bool {
    url.starts_with(SHIELDS_PREFIX) || workflow_of_badge(url).is_some()
}

/// The workflow file name of a workflow badge URL.
fn workflow_of_badge(url: &str) -> Option<String> {
    let rest = url.strip_prefix(WORKFLOW_BADGE_PREFIX)?;
    let path = rest.split('?').next().unwrap_or("");
    let name = path.strip_suffix(WORKFLOW_BADGE_SUFFIX)?;
    (!name.is_empty() && !name.contains('/') && !name.contains("..")).then(|| name.to_string())
}

/// Every badge URL of `markdown`.
fn badges(markdown: &str) -> Vec<String> {
    image_urls(markdown)
        .into_iter()
        .filter(|u| is_badge(u))
        .collect()
}

/// Whether a badge URL shows a release or tag version.
fn is_release_badge(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("release") || lower.contains("/v/tag") || lower.contains("/tag/")
}

/// Whether `text` contains `word` as a whole word, ASCII case-insensitive; boundaries are
/// non-alphanumeric characters or the text ends.
fn contains_whole_word(text: &str, word: &str) -> bool {
    let hay = text.to_ascii_lowercase();
    let needle = word.to_ascii_lowercase();
    let bytes = hay.as_bytes();
    let mut from = 0;
    while let Some(pos) = hay[from..].find(&needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

/// Level-2 headings (`## `) outside fenced code, as (byte offset in `markdown`, text).
fn level2_headings(markdown: &str) -> Vec<(usize, String)> {
    let mut headings = Vec::new();
    let mut in_fence = false;
    let mut offset = 0;
    for line in markdown.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        } else if !in_fence {
            if let Some(text) = line.strip_prefix("## ") {
                headings.push((offset, text.trim().to_string()));
            }
        }
        offset += line.len();
    }
    headings
}

/// Indices into `level2_headings` of each required keyword, in order, or the first missing one.
fn ordered_sections(headings: &[(usize, String)], keywords: &[&str]) -> Result<Vec<usize>, String> {
    let mut found = Vec::new();
    let mut next = 0;
    for keyword in keywords {
        let lower = keyword.to_ascii_lowercase();
        let Some(rel) = headings[next..]
            .iter()
            .position(|(_, text)| text.to_ascii_lowercase().contains(&lower))
        else {
            return Err(format!(
                "no `## ` heading containing `{keyword}` after the previous required section; headings: {:?}",
                headings.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>()
            ));
        };
        found.push(next + rel);
        next += rel + 1;
    }
    Ok(found)
}

/// The text of the first level-2 section whose heading contains `keyword` (heading included,
/// up to the next level-2 heading).
fn section<'a>(markdown: &'a str, keyword: &str) -> Option<&'a str> {
    let headings = level2_headings(markdown);
    let lower = keyword.to_ascii_lowercase();
    let index = headings
        .iter()
        .position(|(_, text)| text.to_ascii_lowercase().contains(&lower))?;
    let start = headings[index].0;
    let end = headings.get(index + 1).map_or(markdown.len(), |(o, _)| *o);
    Some(&markdown[start..end])
}

/// Whether the repository has release evidence: a git tag or a release file. A failing or
/// missing `git` counts as "no tag" (stricter).
fn has_release_evidence(root: &Path) -> bool {
    if root.join(RELEASE_FILE).exists() {
        return true;
    }
    let mut cmd = Command::new("git");
    cmd.current_dir(root).args(["tag", "--list"]);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(&key);
        }
    }
    cmd.output().is_ok_and(|out| {
        out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty()
    })
}

/// The `channel` of `rust-toolchain.toml`.
fn pinned_rust(root: &Path) -> String {
    let text = fs::read_to_string(root.join("rust-toolchain.toml")).expect("rust-toolchain.toml");
    text.lines()
        .find_map(|l| {
            let (key, value) = l.split_once('=')?;
            (key.trim() == "channel").then(|| value.trim().trim_matches('"').to_string())
        })
        .expect("channel in rust-toolchain.toml")
}

/// A scratch directory under the workspace `target/`.
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = workspace_root()
        .join("target")
        .join("readme_contract")
        .join(format!("{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("Docs/assets")).expect("create scratch dir");
    fs::write(dir.join("LICENSE"), "licence\n").expect("write LICENSE");
    fs::write(dir.join("Docs/GUIDE.md"), "guide\n").expect("write guide");
    fs::write(dir.join("Docs/assets/a.png"), b"png").expect("write asset");
    dir
}

// ---------------------------------------------------------------------------------------
// RDM1 / RDM2 — relative links
// ---------------------------------------------------------------------------------------

/// RDM1: every relative link of the real README resolves.
#[test]
fn test_rdm_readme_relative_links_resolve() {
    let root = workspace_root();
    let readme = read("README.md");
    let targets = link_targets(&readme);
    let failures: Vec<String> = targets
        .iter()
        .filter_map(|t| check_relative_target(t, &root).err())
        .collect();
    assert!(failures.is_empty(), "README.md links: {failures:?}");
}

/// RDM2: the link parser and resolver on synthetic Markdown.
#[test]
fn test_rdm_link_parser_and_resolver() {
    let root = scratch_dir("links");
    let markdown = "\
# Title
[![CI](https://github.com/soos-security/soos/actions/workflows/ci.yml/badge.svg)](LICENSE)
See [the guide](Docs/GUIDE.md#section) and [top](#title) and [web](https://example.org/x).
Inline code is ignored: `[not a link](missing-inline.md)`.
<a href=\"Docs/GUIDE.md?x=1\">guide</a> <img src=\"Docs/assets/a.png\" alt=\"a\">
[ref]: Docs/GUIDE.md \"title\"
```bash
[fenced](missing-fenced.md)
```
[broken](Docs/MISSING.md)
<a href='missing-html.md'>x</a>
";
    let targets = link_targets(markdown);
    for expected in [
        "https://github.com/soos-security/soos/actions/workflows/ci.yml/badge.svg",
        "LICENSE",
        "Docs/GUIDE.md#section",
        "#title",
        "https://example.org/x",
        "Docs/GUIDE.md?x=1",
        "Docs/assets/a.png",
        "Docs/GUIDE.md",
        "Docs/MISSING.md",
        "missing-html.md",
    ] {
        assert!(
            targets.iter().any(|t| t == expected),
            "parser must find `{expected}` in {targets:?}"
        );
    }
    for ignored in ["missing-inline.md", "missing-fenced.md"] {
        assert!(
            !targets.iter().any(|t| t == ignored),
            "code must be ignored, found `{ignored}`"
        );
    }
    let failures: Vec<String> = targets
        .iter()
        .filter_map(|t| check_relative_target(t, &root).err())
        .collect();
    assert_eq!(
        failures.len(),
        2,
        "exactly the two broken links: {failures:?}"
    );
    assert!(failures.iter().any(|f| f.contains("Docs/MISSING.md")));
    assert!(failures.iter().any(|f| f.contains("missing-html.md")));

    assert!(
        check_relative_target("Docs", &root).is_ok(),
        "directories resolve"
    );
    assert!(check_relative_target("mailto:x@example.org", &root).is_ok());
    assert!(check_relative_target("/etc/passwd", &root).is_err());
    assert!(check_relative_target("../outside.md", &root).is_err());
    assert!(check_relative_target("Docs/../../outside.md", &root).is_err());
    assert!(check_relative_target("?only-query", &root).is_err());
    let _ = fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------------------------
// RDM3 / RDM4 — badges and words
// ---------------------------------------------------------------------------------------

/// RDM3: at most 6 badges, each real, and no release badge without release evidence.
#[test]
fn test_rdm_readme_badges_are_bounded_and_real() {
    let root = workspace_root();
    let readme = read("README.md");
    let found = badges(&readme);
    assert!(
        found.len() <= MAX_BADGES,
        "README.md shows {} badges (at most {MAX_BADGES}): {found:?}",
        found.len()
    );
    if !has_release_evidence(&root) {
        let release: Vec<&String> = found.iter().filter(|u| is_release_badge(u)).collect();
        assert!(
            release.is_empty(),
            "no git tag and no {RELEASE_FILE}: README.md must not show a release badge: {release:?}"
        );
    }
    let rust = pinned_rust(&root);
    for url in &found {
        if let Some(workflow) = workflow_of_badge(url) {
            assert!(
                root.join(".github/workflows").join(&workflow).is_file(),
                "badge `{url}` names a missing workflow .github/workflows/{workflow}"
            );
        }
        let lower = url.to_ascii_lowercase();
        if lower.starts_with(&format!("{SHIELDS_PREFIX}badge/rust-")) {
            assert!(
                url.contains(&format!("-{rust}-")),
                "Rust badge `{url}` must show the pinned toolchain {rust}"
            );
        }
    }
}

/// RDM4: the badge and word helpers.
#[test]
fn test_rdm_badge_and_word_helpers() {
    let markdown = "\
[![CI](https://github.com/soos-security/soos/actions/workflows/ci.yml/badge.svg?branch=main)](x)
![Shot](Docs/assets/a.png)
<img src=\"https://img.shields.io/badge/rust-1.98.1-orange\" alt=\"Rust\">
`![code](https://img.shields.io/badge/code-1-red)`
";
    let found = badges(markdown);
    assert_eq!(
        found.len(),
        2,
        "two badges, screenshot and code ignored: {found:?}"
    );
    assert_eq!(
        workflow_of_badge(&found[0]).as_deref(),
        Some("ci.yml"),
        "workflow name with a query"
    );
    assert_eq!(
        workflow_of_badge("https://github.com/soos-security/soos/actions/workflows/a/b/badge.svg"),
        None
    );
    assert_eq!(
        workflow_of_badge("https://github.com/other/repo/actions/workflows/ci.yml/badge.svg"),
        None
    );
    for release in [
        "https://img.shields.io/github/v/release/soos-security/soos",
        "https://img.shields.io/github/v/tag/soos-security/soos",
        "https://img.shields.io/github/release-date/soos-security/soos",
    ] {
        assert!(is_release_badge(release), "{release} is a release badge");
    }
    assert!(!is_release_badge(
        "https://img.shields.io/badge/rust-1.98.1-orange"
    ));

    assert!(contains_whole_word(
        "An Enterprise-Grade module",
        "enterprise-grade"
    ));
    assert!(contains_whole_word("CERTIFIED.", "certified"));
    assert!(contains_whole_word("self-certified", "certified"));
    assert!(!contains_whole_word("no certification yet", "certified"));
    assert!(!contains_whole_word("uncertified", "certified"));
    assert!(!contains_whole_word("unbreakables", "unbreakable"));
    assert!(contains_whole_word("unbreakable", "unbreakable"));
}

/// RDM5: no overstated wording anywhere in the README.
#[test]
fn test_rdm_readme_has_no_banned_words() {
    let readme = read("README.md");
    let found: Vec<&str> = BANNED_WORDS
        .iter()
        .copied()
        .filter(|w| contains_whole_word(&readme, w))
        .collect();
    assert!(
        found.is_empty(),
        "README.md must not use {found:?} (#356: claims must stay verifiable)"
    );
}

// ---------------------------------------------------------------------------------------
// RDM6 – RDM8 — structure
// ---------------------------------------------------------------------------------------

/// RDM6: the required sections appear in order and "How it works" holds a Mermaid diagram.
#[test]
fn test_rdm_readme_sections_in_order_with_mermaid() {
    let readme = read("README.md");
    let headings = level2_headings(&readme);
    if let Err(e) = ordered_sections(&headings, &REQUIRED_SECTIONS) {
        panic!("README.md section order: {e}");
    }
    let how = section(&readme, "How it works").expect("How it works section");
    assert!(
        how.lines().any(|l| l.trim() == "```mermaid"),
        "the How it works section must contain a ```mermaid block"
    );

    // Helper self-check on synthetic headings.
    let synthetic = "## A\n```\n## Fenced\n```\n## B\n## C\n";
    let h = level2_headings(synthetic);
    assert_eq!(
        h.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(),
        ["A", "B", "C"]
    );
    assert!(ordered_sections(&h, &["A", "C"]).is_ok());
    assert!(ordered_sections(&h, &["C", "A"]).is_err());
    assert_eq!(section(synthetic, "B"), Some("## B\n"));
}

/// RDM7: the security section states the fallback, the kill switch, the PAD caveat and links.
#[test]
fn test_rdm_security_section_states_fallback_kill_switch_and_pad_caveat() {
    let readme = read("README.md");
    let headings = level2_headings(&readme);
    let order = ordered_sections(&headings, &REQUIRED_SECTIONS)
        .unwrap_or_else(|e| panic!("README.md section order: {e}"));
    let index = order[5];
    let start = headings[index].0;
    let end = headings.get(index + 1).map_or(readme.len(), |(o, _)| *o);
    let security = &readme[start..end];
    for needle in SECURITY_NEEDLES {
        assert!(
            security.contains(needle),
            "the README security section must mention `{needle}`"
        );
    }
}

/// RDM8: the comparison section cites upstream Howdy and Microsoft documentation.
#[test]
fn test_rdm_comparison_cites_upstream_sources() {
    let readme = read("README.md");
    let comparison =
        section(&readme, "Comparison").expect("README.md must have a Comparison section");
    let targets = link_targets(comparison);
    for source in COMPARISON_SOURCES {
        assert!(
            targets.iter().any(|t| t.starts_with(source)),
            "the comparison must link {source}*: {targets:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------
// RDM10 – RDM11 — companion wording tied to the setup defaults
// ---------------------------------------------------------------------------------------

/// The setup render fixture, byte-equal to what `scripts/setup.sh` writes (asserted by
/// `setup_user_phase_contract` and `crates/remote/tests/setup_render_tests.rs`).
const SETUP_RENDER_FIXTURE: &str = "tests/fixtures/setup/remote.toml";
/// Companion keys whose default the README describes: Funnel, remote unlock and the camera
/// view (ADR 2026-10-15 "Setup User Phase: Phone Companion Profile With Default-On Features").
const COMPANION_EXPOSURE_KEYS: [&str; 4] = [
    "allow_funnel",
    "allow_unlock",
    "camera_view",
    "camera_view_funnel",
];
/// Needles the security section must contain while setup turns any of those keys on.
const COMPANION_DEFAULT_NEEDLES: [&str; 4] =
    ["scripts/setup.sh", "passkey", "soos-gui", "remote.toml"];

/// The keys of `COMPANION_EXPOSURE_KEYS` that the TOML text sets to `true`.
fn keys_set_true(toml: &str) -> Vec<&'static str> {
    COMPANION_EXPOSURE_KEYS
        .iter()
        .copied()
        .filter(|key| {
            toml.lines().any(|line| {
                let mut parts = line.splitn(2, '=');
                let name = parts.next().unwrap_or("").trim();
                let value = parts.next().unwrap_or("").trim();
                name == *key && value == "true"
            })
        })
        .collect()
}

/// Text units of a Markdown document: every table row and every fenced line on its own,
/// list items and paragraphs joined across their wrapped lines.
fn text_units(markdown: &str) -> Vec<String> {
    let mut units = Vec::new();
    let mut current = String::new();
    let mut in_fence = false;
    let flush = |current: &mut String, units: &mut Vec<String>| {
        if !current.trim().is_empty() {
            units.push(current.trim().to_string());
        }
        current.clear();
    };
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            flush(&mut current, &mut units);
            in_fence = !in_fence;
            continue;
        }
        let list_item = trimmed.starts_with("- ")
            || trimmed
                .split_once(". ")
                .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
        if in_fence || trimmed.starts_with('|') {
            flush(&mut current, &mut units);
            units.push(trimmed.to_string());
        } else if trimmed.is_empty() {
            flush(&mut current, &mut units);
        } else {
            if list_item {
                flush(&mut current, &mut units);
            }
            current.push(' ');
            current.push_str(trimmed);
        }
    }
    flush(&mut current, &mut units);
    units
}

/// Whether a unit calls remote unlock or the camera view opt-in or off by default.
fn calls_exposure_opt_in(unit: &str) -> bool {
    let lower = unit.to_ascii_lowercase().replace("auto-unlock", "");
    let opt_in = ["opt-in", "opt in", "off by default"]
        .iter()
        .any(|w| lower.contains(w));
    let exposure = ["unlock", "camera view", "camera preview", "live camera"]
        .iter()
        .any(|w| lower.contains(w));
    opt_in && exposure
}

/// Whether a unit says the camera view goes through Tailscale without naming Funnel.
fn camera_view_omits_funnel(unit: &str) -> bool {
    let lower = unit.to_ascii_lowercase();
    let camera = ["camera view", "camera preview", "live camera"]
        .iter()
        .any(|w| lower.contains(w));
    camera && lower.contains("tailscale") && !lower.contains("funnel")
}

/// RDM10: while `scripts/setup.sh` turns Funnel, remote unlock or the camera view on, the
/// README never calls them opt-in or off by default, the camera view over Tailscale names
/// Funnel, and the security section names setup, the passkey, soos-gui, `remote.toml` and
/// every such key.
#[test]
fn test_rdm_companion_wording_matches_setup_defaults() {
    let on = keys_set_true(&read(SETUP_RENDER_FIXTURE));
    if on.is_empty() {
        return;
    }
    let readme = read("README.md");
    let units = text_units(&readme);
    let opt_in: Vec<&String> = units.iter().filter(|u| calls_exposure_opt_in(u)).collect();
    assert!(
        opt_in.is_empty(),
        "scripts/setup.sh turns {on:?} on, so README.md must not call remote unlock or the camera view opt-in or off by default: {opt_in:#?}"
    );
    if on.contains(&"camera_view_funnel") {
        let no_funnel: Vec<&String> = units
            .iter()
            .filter(|u| camera_view_omits_funnel(u))
            .collect();
        assert!(
            no_funnel.is_empty(),
            "setup writes camera_view_funnel = true, so a camera view over Tailscale must name Funnel: {no_funnel:#?}"
        );
    }
    let security = section(&readme, "Security").expect("README.md must have a Security section");
    for needle in COMPANION_DEFAULT_NEEDLES
        .iter()
        .copied()
        .chain(on.iter().copied())
    {
        assert!(
            security.contains(needle),
            "the README security section must mention `{needle}` (setup turns {on:?} on)"
        );
    }
}

/// RDM11: the RDM10 helpers on synthetic input.
#[test]
fn test_rdm_companion_wording_helpers() {
    assert_eq!(
        keys_set_true("allow_funnel = true\nallow_unlock=false\n# camera_view = true\ncamera_view_funnel = true\n"),
        vec!["allow_funnel", "camera_view_funnel"]
    );
    assert!(keys_set_true("allow_unlock = \"true\"\n").is_empty());

    let units = text_units(
        "Para one\nwrapped.\n\n- item a\n  wrapped\n- item b\n1. step\n| a | b |\n```mermaid\nx --> y\n```\n",
    );
    assert_eq!(
        units,
        vec![
            "Para one wrapped.",
            "- item a wrapped",
            "- item b",
            "1. step",
            "| a | b |",
            "x --> y",
        ]
    );

    assert!(calls_exposure_opt_in("and, if you\nopt in, unlock it"));
    assert!(calls_exposure_opt_in("Remote unlock is off by default."));
    assert!(calls_exposure_opt_in("opt-in live camera view"));
    assert!(calls_exposure_opt_in(
        "remote[\"soos-remote<br/>opt-in unlock\"]"
    ));
    assert!(!calls_exposure_opt_in(
        "Presence auto-unlock, opt-in polkit rule"
    ));
    assert!(!calls_exposure_opt_in("Opt-in, encrypted camera snapshots"));
    assert!(!calls_exposure_opt_in(
        "remote unlock, on by default with setup"
    ));

    assert!(camera_view_omits_funnel(
        "the live camera view streams through your Tailscale network"
    ));
    assert!(!camera_view_omits_funnel(
        "the live camera view streams over Tailscale Serve or Funnel"
    ));
    assert!(!camera_view_omits_funnel("lock status over Tailscale"));
}

// ---------------------------------------------------------------------------------------
// RDM12 – RDM13 — turning companion keys off in `remote.toml`
// ---------------------------------------------------------------------------------------

/// Needles the security section must contain so a reader can turn a key off in `remote.toml`
/// without `soos-remote` refusing the file (`crates/remote/src/config.rs`: `PushRequiresAlerts`,
/// `OfflineRequiresPush`, `CameraFunnelRequiresCameraView`, `CameraFunnelRequiresFunnel`,
/// `AndroidRequiresFunnel`, exit 78).
const COMPANION_DEPENDENCY_NEEDLES: [&str; 6] = [
    "exit 78",
    "password_alerts",
    "push_notifications",
    "offline_alerts",
    "camera_view_funnel",
    "[android]",
];

/// Whether a unit says every companion key can be turned off in `remote.toml` on its own,
/// without naming the keys that depend on it.
fn claims_independent_opt_out(unit: &str) -> bool {
    let lower = unit.to_ascii_lowercase();
    let independent = [
        "every key can be set to",
        "each can be turned off in `remote.toml`",
        "each can be turned off in remote.toml",
        "or by setting its key to `false`",
    ]
    .iter()
    .any(|w| lower.contains(w));
    independent && lower.contains("remote.toml") && !lower.contains("depend")
}

/// RDM12: the setup render fixture really sets the exposure keys (so RDM10 never passes by
/// reading nothing), no README unit claims a key can be turned off in `remote.toml` on its own,
/// and the security section names the dependent keys and the exit 78 refusal.
#[test]
fn test_rdm_companion_opt_out_names_dependent_keys() {
    let on = keys_set_true(&read(SETUP_RENDER_FIXTURE));
    assert_eq!(
        on,
        COMPANION_EXPOSURE_KEYS.to_vec(),
        "{SETUP_RENDER_FIXTURE} must set every exposure key to `true` as `key = true` lines; update RDM10 and the README with the setup defaults"
    );
    let readme = read("README.md");
    let independent: Vec<String> = text_units(&readme)
        .into_iter()
        .filter(|u| claims_independent_opt_out(u))
        .collect();
    assert!(
        independent.is_empty(),
        "README.md must not say a companion key can be turned off in remote.toml on its own (soos-remote exits 78 on a dangling dependant): {independent:#?}"
    );
    let security = section(&readme, "Security").expect("README.md must have a Security section");
    for needle in COMPANION_DEPENDENCY_NEEDLES {
        assert!(
            security.contains(needle),
            "the README security section must mention `{needle}` when it explains remote.toml opt-outs"
        );
    }
}

/// RDM13: the RDM12 helper on synthetic input.
#[test]
fn test_rdm_companion_opt_out_helper() {
    assert!(claims_independent_opt_out(
        "Every key can be set to `false` in `remote.toml`."
    ));
    assert!(claims_independent_opt_out(
        "each can be turned off in `remote.toml`, and in soos-gui"
    ));
    assert!(claims_independent_opt_out(
        "turned off in soos-gui or by setting its key to `false` in `remote.toml`"
    ));
    assert!(!claims_independent_opt_out(
        "every key in `remote.toml` together with the keys that depend on it"
    ));
    assert!(!claims_independent_opt_out(
        "In `remote.toml`, set a key to `false` together with the keys that depend on it"
    ));
    assert!(!claims_independent_opt_out(
        "every key can be set to anything"
    ));
}
