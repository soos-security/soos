//! Static contracts of the Playwright cross-engine bench of the `soos-remote` web app
//! (GitHub #464, PR 2 of #355; owner decision of 2026-10-21 on #355; amendment of 2026-10-21 to
//! ADR 2026-10-13 "Browser Support Policy for the `soos-remote` Web App"; matrix RPW1–RPW9):
//!
//! - RPW-S1 the toolchain is pinned: exact `@playwright/test`, lockfile with integrity hashes
//!   and no install script, `.npmrc` without lifecycle scripts, the official image by digest
//!   at the same release, `npm ci --ignore-scripts`, a test run without network;
//! - RPW-S2 the CI job is a separate, advisory workflow: read-only token, SHA-pinned actions, no
//!   secret, three engines, step-level `continue-on-error` on the bench step only, outside
//!   `CI Success` and outside `ADVISORY_CHECKS` (still two entries);
//! - RPW-S3 the specs cover the RXB behaviours on Chromium, Firefox and WebKit, against the
//!   loopback fixture server only, with no trace, screenshot or video, no focused or skipped
//!   test, and a page-error guard on every test;
//! - RPW-S4 the bench is dev-only: never packaged, never a workspace member, ignored outputs;
//! - RPW-S5 ADR amendment, `Docs/REMOTE_COMPANION.md`, CI doc, matrix rows and walkthrough 248.
//!
//! The browser behaviour itself is proven by the bench (`tests/browser/specs`); these checks
//! keep its supply chain, isolation and advisory status from drifting.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;

use crate::remote_companion_contract::{exists, read, workspace_root};

const BENCH: &str = "tests/browser";
const PACKAGE_JSON: &str = "tests/browser/package.json";
const PACKAGE_LOCK: &str = "tests/browser/package-lock.json";
const NPMRC: &str = "tests/browser/.npmrc";
const CONFIG: &str = "tests/browser/playwright.config.mjs";
const FIXTURES: &str = "tests/browser/fixtures.mjs";
const SCRIPT: &str = "scripts/remote_browser_tests.sh";
const WORKFLOW: &str = ".github/workflows/browser.yml";
const CI: &str = ".github/workflows/ci.yml";
const GATE: &str = "scripts/pr_checks_gate.sh";
const DEPENDABOT: &str = ".github/dependabot.yml";
const GITIGNORE: &str = ".gitignore";
const DECISIONS: &str = "AI/DECISIONS.md";
const MATRIX: &str = "AI/VERIFICATION_MATRIX.md";
const REMOTE_DOC: &str = "Docs/REMOTE_COMPANION.md";
const CI_DOC: &str = "Docs/CI_CD_AND_SECURITY.md";
const WALKTHROUGH: &str = "AI/walkthroughs/248_remote_playwright.md";

const ADR_TITLE: &str = "Browser Support Policy for the `soos-remote` Web App";
const ENGINES: [&str; 3] = ["chromium", "firefox", "webkit"];
/// The three packages of the lockfile (no other dependency is allowed).
const LOCKED_PACKAGES: [&str; 3] = [
    "node_modules/@playwright/test",
    "node_modules/playwright",
    "node_modules/playwright-core",
];
const IMAGE_PREFIX: &str = "readonly PLAYWRIGHT_IMAGE=\"mcr.microsoft.com/playwright:v";

/// `(spec file, test titles it must define)`: the RXB behaviours of the owner decision.
const SPECS: [(&str, &[&str]); 5] = [
    (
        "tests/browser/specs/signin.spec.mjs",
        &["sign-in screen renders with an enabled passkey button"],
    ),
    (
        "tests/browser/specs/cards.spec.mjs",
        &[
            "status card shows the locked state and the idle session",
            "battery line comes from the event stream",
            "alerts card lists the failed passwords",
            "offline history card lists both periods, newest first",
            "status, battery, alerts and offline cards",
        ],
    ),
    (
        "tests/browser/specs/tour.spec.mjs",
        &[
            "the tour shows once, then is remembered",
            "with a blocked storage the tour shows again after a reload",
        ],
    ),
    (
        "tests/browser/specs/motion.spec.mjs",
        &[
            "no element animates or transitions",
            "the state numeral animates and the buttons transition",
        ],
    ),
    (
        "tests/browser/specs/degradation.spec.mjs",
        &[
            "sign-in is disabled with the fixed sentence",
            "unlock is disabled with the fixed sentence, status still works",
            "lock keeps working on an unlocked PC",
            "with WebAuthn no passkey sentence is shown",
            "the status is polled every STATUS_POLL_MS instead",
            "the push card names the missing feature and hides its buttons",
            "without a service worker the card asks for a normal window",
            "with push the card offers Enable notifications",
        ],
    ),
];

/// The value of a top-level `"key": "value"` line of a two-space-indented JSON file.
fn json_string_field(text: &str, indent: usize, key: &str) -> Option<String> {
    let prefix = format!("{}\"{key}\": \"", " ".repeat(indent));
    text.lines()
        .find_map(|l| l.strip_prefix(prefix.as_str()))
        .map(|rest| rest.trim_end_matches(',').trim_end_matches('"').to_string())
}

/// The body lines of the object opened by `"name": {` at `indent` spaces, up to its closing
/// brace at the same indent.
fn json_object<'a>(text: &'a str, indent: usize, name: &str) -> Vec<&'a str> {
    let open = format!("{}\"{name}\": {{", " ".repeat(indent));
    let close = format!("{}}}", " ".repeat(indent));
    let mut lines = text.lines().skip_while(|l| *l != open);
    assert!(lines.next().is_some(), "object `{name}` not found");
    lines
        .take_while(|l| l.trim_end_matches(',') != close)
        .collect()
}

/// Keys of the direct members of an object body at `indent` spaces.
fn member_keys(body: &[&str], indent: usize) -> Vec<String> {
    let prefix = format!("{}\"", " ".repeat(indent));
    body.iter()
        .filter_map(|l| l.strip_prefix(prefix.as_str()))
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_string)
        .collect()
}

fn is_exact_semver(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// The exact `@playwright/test` version of `package.json`.
fn playwright_version() -> String {
    let package = read(PACKAGE_JSON);
    let dev = json_object(&package, 2, "devDependencies");
    assert_eq!(
        member_keys(&dev, 4),
        vec!["@playwright/test".to_string()],
        "the only dependency"
    );
    let version = json_string_field(&dev.join("\n"), 4, "@playwright/test").expect("version");
    assert!(
        is_exact_semver(&version),
        "exact version, no range: {version}"
    );
    version
}

/// Lines of `text` that are not shell comments.
fn shell_code(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The steps of the bench workflow job, each as its own text block.
fn workflow_steps(workflow: &str) -> Vec<String> {
    let steps = &workflow[workflow.find("\n    steps:\n").expect("steps")..];
    steps
        .split("\n      - ")
        .skip(1)
        .map(|s| format!("- {s}"))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// RPW-S1
// ---------------------------------------------------------------------------------------------

/// RPW-S1 (RPW6): exact versions, integrity-hashed lockfile without install scripts, no lifecycle
/// script, the official image by digest at the same release, `npm ci --ignore-scripts` and a
/// test run without network.
#[test]
fn test_rpw_s1_toolchain_is_pinned() {
    let version = playwright_version();
    let package = read(PACKAGE_JSON);
    assert!(
        package.contains("\n  \"private\": true,"),
        "never published"
    );
    assert!(
        !package.contains("\"dependencies\""),
        "dev dependencies only"
    );
    let scripts = json_object(&package, 2, "scripts");
    assert_eq!(member_keys(&scripts, 4), vec!["test".to_string()]);
    let engines = json_object(&package, 2, "engines");
    let node = json_string_field(&engines.join("\n"), 4, "node").expect("engines.node");
    assert!(is_exact_semver(&node), "exact Node version: {node}");

    let lock = read(PACKAGE_LOCK);
    assert!(lock.contains("\n  \"lockfileVersion\": 3,"), "lockfile v3");
    assert!(!lock.contains("hasInstallScript"), "no install script");
    let packages = json_object(&lock, 2, "packages");
    let mut keys = member_keys(&packages, 4);
    keys.sort();
    let mut expected: Vec<String> = std::iter::once(String::new())
        .chain(LOCKED_PACKAGES.iter().map(|p| (*p).to_string()))
        .collect();
    expected.sort();
    assert_eq!(keys, expected, "exactly the three Playwright packages");
    for name in LOCKED_PACKAGES {
        let entry = json_object(&lock, 4, name).join("\n");
        assert_eq!(
            json_string_field(&entry, 6, "version").as_deref(),
            Some(version.as_str()),
            "{name} at the pinned release"
        );
        let resolved = json_string_field(&entry, 6, "resolved").expect("resolved");
        assert!(
            resolved.starts_with("https://registry.npmjs.org/"),
            "{name}: {resolved}"
        );
        let integrity = json_string_field(&entry, 6, "integrity").expect("integrity");
        assert!(
            integrity.starts_with("sha512-") && integrity.len() > 80,
            "{name}: {integrity}"
        );
        assert!(entry.contains("\"dev\": true"), "{name} is dev-only");
    }

    let npmrc = read(NPMRC);
    for line in [
        "ignore-scripts=true",
        "save-exact=true",
        "engine-strict=true",
    ] {
        assert!(npmrc.lines().any(|l| l == line), ".npmrc: {line}");
    }

    let script = read(SCRIPT);
    assert!(script.starts_with("#!/usr/bin/env bash\n"));
    assert!(script.contains("set -euo pipefail"));
    let image_line = script
        .lines()
        .find(|l| l.starts_with(IMAGE_PREFIX))
        .expect("pinned image line");
    let rest = &image_line[IMAGE_PREFIX.len()..];
    let (tag, digest) = rest
        .trim_end_matches('"')
        .split_once("@sha256:")
        .expect("image pinned by digest");
    assert_eq!(
        tag,
        format!("{version}-noble"),
        "image and package at one release"
    );
    assert!(
        digest.len() == 64
            && digest
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "sha256 digest: {digest}"
    );
    let code = shell_code(&script);
    assert_eq!(
        code.matches("mcr.microsoft.com/").count(),
        1,
        "one image reference"
    );
    assert_eq!(code.matches("\"$PLAYWRIGHT_IMAGE\"").count(), 2);
    assert!(code.contains("npm ci --ignore-scripts"));
    assert!(code.contains("PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1"));
    for flag in [
        "--network none",
        "--cap-drop ALL",
        "--security-opt no-new-privileges",
        ":ro",
        "--user \"$(id -u):$(id -g)\"",
        "npx --no-install playwright test",
    ] {
        assert!(code.contains(flag), "{SCRIPT}: {flag}");
    }
    for forbidden in [
        "npm install",
        "npm i ",
        "playwright install",
        "--network host",
        "--privileged",
        "--ipc=host",
        "secret",
        "TOKEN",
        "curl",
        "wget",
        "docker push",
        "docker login",
    ] {
        assert!(
            !code.contains(forbidden),
            "{SCRIPT} must not use {forbidden}"
        );
    }
    // The test run (the second container) is the one without network.
    let npm = code.find("npm ci --ignore-scripts").unwrap();
    let offline = code.find("--network none").unwrap();
    let test_run = code.find("npx --no-install playwright test").unwrap();
    assert!(
        npm < offline && offline < test_run,
        "install, then offline test run"
    );
}

// ---------------------------------------------------------------------------------------------
// RPW-S2
// ---------------------------------------------------------------------------------------------

/// RPW-S2 (RPW7): a separate, advisory workflow with a read-only token, SHA-pinned actions and no
/// secret; outside `CI Success` and outside `ADVISORY_CHECKS`.
#[test]
fn test_rpw_s2_workflow_is_separate_and_advisory() {
    let workflow = read(WORKFLOW);
    assert!(workflow.contains("\npermissions:\n  contents: read\n"));
    assert_eq!(
        workflow.matches("permissions:").count(),
        1,
        "no job-level permission"
    );
    for forbidden in [
        "secrets.",
        "pull_request_target",
        "GITHUB_TOKEN",
        "upload-artifact",
        "self-hosted",
        "write-all",
    ] {
        assert!(
            !workflow.contains(forbidden),
            "{WORKFLOW} must not use {forbidden}"
        );
    }
    let mut uses = 0;
    for line in workflow.lines().filter(|l| l.contains("uses:")) {
        uses += 1;
        let reference = line.split("uses:").nth(1).unwrap().trim();
        let sha = reference
            .split('@')
            .nth(1)
            .and_then(|r| r.split_whitespace().next())
            .unwrap_or_default();
        assert!(
            sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
            "action pinned to a full commit SHA: {line}"
        );
    }
    assert!(uses >= 1);
    assert!(workflow.contains("persist-credentials: false"));
    assert!(workflow.contains("engine: [chromium, firefox, webkit]"));
    assert!(workflow.contains("fail-fast: false"));
    assert!(workflow.contains("timeout-minutes:"));
    assert!(
        workflow.contains("name: \"Remote web app (${{ matrix.engine }}, Playwright, advisory)\"")
    );

    let steps = workflow_steps(&workflow);
    let with_continue: Vec<&String> = steps
        .iter()
        .filter(|s| s.contains("continue-on-error"))
        .collect();
    assert_eq!(with_continue.len(), 1, "only the bench step is advisory");
    let bench = with_continue[0];
    assert!(bench.contains("continue-on-error: true"));
    assert!(bench.contains("id: bench"));
    assert!(bench.contains("run: ./scripts/remote_browser_tests.sh -- --project=\"$ENGINE\""));
    let report = steps
        .iter()
        .find(|s| s.contains("name: Advisory result"))
        .expect("result step");
    for needle in [
        "if: always()",
        "${{ steps.bench.outcome }}",
        "$GITHUB_STEP_SUMMARY",
        "::warning",
    ] {
        assert!(report.contains(needle), "the result is reported: {needle}");
    }
    let header = &workflow[..workflow.find("\nname: ").unwrap()];
    for needle in ["CI Success", "ADVISORY_CHECKS", "continue-on-error"] {
        assert!(header.contains(needle), "the header explains {needle}");
    }

    let ci = read(CI);
    for forbidden in [
        "remote_browser_tests",
        "Playwright",
        "tests/browser",
        "remote-browsers",
    ] {
        assert!(
            !ci.contains(forbidden),
            "ci.yml stays unaware of {forbidden}"
        );
    }
    let gate_line = read(GATE)
        .lines()
        .find(|l| l.starts_with("readonly ADVISORY_CHECKS=("))
        .expect("advisory list")
        .to_string();
    assert!(!gate_line.contains("Playwright") && !gate_line.contains("Remote web app"));
    assert_eq!(gate_line.matches("\" \"").count(), 1, "still two entries");

    let dependabot = read(DEPENDABOT);
    let npm = &dependabot[dependabot
        .find("package-ecosystem: npm")
        .expect("npm ecosystem")..];
    assert!(npm.contains("directory: /tests/browser"));
    assert!(npm.contains("default-days: 7"), "npm cooldown");
}

// ---------------------------------------------------------------------------------------------
// RPW-S3
// ---------------------------------------------------------------------------------------------

/// RPW-S3 (RPW1–RPW5, RPW8): the specs cover the RXB behaviours on the three engines, against the
/// loopback fixture server only, with no trace, screenshot or video and a page-error guard.
#[test]
fn test_rpw_s3_specs_cover_the_rxb_behaviours() {
    let config = read(CONFIG);
    for engine in ENGINES {
        assert!(
            config.contains(&format!("{{ name: \"{engine}\", use:")),
            "project {engine}"
        );
    }
    for off in ["trace: \"off\"", "screenshot: \"off\"", "video: \"off\""] {
        assert!(config.contains(off), "{CONFIG}: {off}");
    }
    for needle in [
        "forbidOnly: true",
        "retries: 0",
        "timezoneId: \"UTC\"",
        "locale: \"en-US\"",
    ] {
        assert!(config.contains(needle), "{CONFIG}: {needle}");
    }

    let fixtures = read(FIXTURES);
    for needle in [
        "process.env.SOOS_FIXTURE_SERVER",
        "\"--clock\", \"fixed\"",
        "http://127.0.0.1:",
        "page.on(\"pageerror\"",
        "{ auto: true }",
        "expect(errors, \"uncaught page errors\").toEqual([])",
        "page.clock.setFixedTime(FIXED_CAPTURE_UNIX_MS)",
        "FIXED_CAPTURE_UNIX_MS = 1790847660000;",
        "child.kill(\"SIGKILL\")",
    ] {
        assert!(fixtures.contains(needle), "{FIXTURES}: {needle}");
    }
    assert!(
        read("tests/fixtures/mod.rs").contains("FIXED_CAPTURE_UNIX_MS: u64 = 1_790_847_660_000;"),
        "the bench and the fixture share one capture instant"
    );

    let dir = workspace_root().join(BENCH).join("specs");
    let mut found: Vec<String> = fs::read_dir(&dir)
        .expect("specs dir")
        .filter_map(Result::ok)
        .map(|e| format!("{BENCH}/specs/{}", e.file_name().to_string_lossy()))
        .collect();
    found.sort();
    let mut listed: Vec<String> = SPECS.iter().map(|(f, _)| (*f).to_string()).collect();
    listed.sort();
    assert_eq!(found, listed, "the spec files are exactly the contract");

    let mut sources: Vec<(String, String)> = SPECS
        .iter()
        .map(|(f, _)| ((*f).to_string(), read(f)))
        .collect();
    sources.push((CONFIG.to_string(), config));
    sources.push((FIXTURES.to_string(), fixtures));
    for (file, text) in &sources {
        let without_loopback = text.replace("http://127.0.0.1:", "");
        for forbidden in [
            "http://",
            "https://",
            "ws://",
            "wss://",
            "test.only",
            "describe.only",
            "test.skip",
            "test.fixme",
            "test.fail",
            "console.log",
            "localhost",
            "toHaveScreenshot",
            "screenshot(",
        ] {
            assert!(
                !without_loopback.contains(forbidden),
                "{file} must not contain {forbidden}"
            );
        }
        if file.ends_with(".spec.mjs") {
            assert!(
                text.contains("from \"../fixtures.mjs\""),
                "{file} uses the guarded fixtures"
            );
            assert!(
                !text.contains("from \"@playwright/test\""),
                "{file} never bypasses the page-error guard"
            );
        }
    }
    for (file, titles) in SPECS {
        let text = read(file);
        for title in titles {
            assert!(
                text.contains(&format!("test(\"{title}\"")),
                "{file} defines `{title}`"
            );
        }
    }
    // Missing features are removed before any page script runs, never stubbed into success.
    let degradation = read("tests/browser/specs/degradation.spec.mjs");
    for feature in [
        "\"webauthn\"",
        "\"eventSource\"",
        "\"push\"",
        "\"serviceWorker\"",
    ] {
        assert!(
            degradation.contains(feature),
            "degradation covers {feature}"
        );
    }
    for text in [
        "PASSKEY_UNSUPPORTED_TEXT",
        "PUSH_UNSUPPORTED_TEXT",
        "PUSH_NO_WORKER_TEXT",
    ] {
        assert!(
            degradation.contains(&format!("appText(\"{text}\")")),
            "the fixed sentence {text} is read from app.js"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// RPW-S4
// ---------------------------------------------------------------------------------------------

/// RPW-S4 (RPW8): the bench is dev-only (never packaged, never a workspace member) and its
/// install tree and outputs are ignored.
#[test]
fn test_rpw_s4_bench_is_dev_only() {
    let manifest = read("Cargo.toml");
    assert!(
        !manifest.contains("tests/browser"),
        "not a workspace member"
    );
    let root = workspace_root();
    let mut stack = vec![root.join("packaging")];
    let mut scanned = 0;
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(text) = fs::read_to_string(&path) {
                scanned += 1;
                for forbidden in ["tests/browser", "playwright", "remote_browser_tests"] {
                    assert!(
                        !text.contains(forbidden),
                        "{} must not ship {forbidden}",
                        path.display()
                    );
                }
            }
        }
    }
    assert!(scanned > 0, "the packaging tree was scanned");
    for rel in [
        "Dockerfile",
        "scripts/install.sh",
        "scripts/setup.sh",
        "scripts/release_dry_run.sh",
    ] {
        if exists(rel) {
            assert!(
                !read(rel).contains("tests/browser"),
                "{rel} must not use the bench"
            );
        }
    }
    let ignore = read(GITIGNORE);
    for line in [
        "tests/browser/node_modules/",
        "tests/browser/test-results/",
        "tests/browser/playwright-report/",
    ] {
        assert!(ignore.lines().any(|l| l == line), ".gitignore: {line}");
    }
}

// ---------------------------------------------------------------------------------------------
// RPW-S5
// ---------------------------------------------------------------------------------------------

/// RPW-S5 (RPW9): ADR amendment, companion and CI documentation, matrix rows and walkthrough.
#[test]
fn test_rpw_s5_docs_adr_matrix_walkthrough() {
    let decisions = read(DECISIONS);
    let adr = decisions
        .lines()
        .find(|l| l.contains(ADR_TITLE) && l.starts_with("* **[2026-10-13]"))
        .expect("the browser support ADR");
    for needle in [
        "Amendment 2026-10-21",
        "#464",
        "Playwright",
        "advisory",
        "--network none",
        "continue-on-error",
    ] {
        assert!(adr.contains(needle), "the ADR amendment names {needle}");
    }
    let doc = read(REMOTE_DOC);
    let i = doc.find("\n## 2i. Browser support").expect("## 2i.");
    let three = doc.find("\n## 3. ").expect("## 3.");
    let section = &doc[i..three];
    for needle in [
        "scripts/remote_browser_tests.sh",
        "Chromium",
        "Firefox",
        "WebKit",
        "advisory",
        "RPW1",
    ] {
        assert!(section.contains(needle), "§2i mentions {needle}");
    }
    let ci_doc = read(CI_DOC);
    for needle in [
        "browser.yml",
        "scripts/remote_browser_tests.sh",
        "continue-on-error",
        "ADVISORY_CHECKS",
        "--network none",
        "sha256:",
        "package-lock.json",
    ] {
        assert!(ci_doc.contains(needle), "{CI_DOC} mentions {needle}");
    }
    let matrix = read(MATRIX);
    for row in 1..=9 {
        assert!(
            matrix.contains(&format!("| RPW{row} |")),
            "matrix row RPW{row}"
        );
    }
    assert!(exists(WALKTHROUGH), "{WALKTHROUGH} exists");
    let walkthrough = read(WALKTHROUGH);
    for needle in ["#464", "#355", "deliberate break"] {
        assert!(
            walkthrough.contains(needle),
            "the walkthrough names {needle}"
        );
    }
}
