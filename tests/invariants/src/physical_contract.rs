//! Physical validation suite contract invariants (GitHub #186, review finding TCI-03).
//!
//! The scripts under `tests/physical/` drive the real `soos-enroll` binary. They broke silently
//! when `--skip-root-check` was removed from the CLI, because the earlier invariant only grepped
//! for subcommand words. These checks derive the accepted flags from the clap definition in
//! `crates/enrollment-cli/src/args.rs` (running `soos-enroll --help` is not possible here: the
//! binary checks for root before it parses its arguments) and reject every flag a script passes
//! that the CLI does not accept.

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

/// Clap definition of `soos-enroll`.
const ARGS_RS: &str = "crates/enrollment-cli/src/args.rs";

/// Directory of the physical validation scripts.
const PHYSICAL_DIR: &str = "tests/physical";

/// Scripts that must invoke `soos-enroll` (keeps the scan from passing vacuously).
const ENROLL_SCRIPTS: [&str; 3] = [
    "tests/physical/enrollment_test.sh",
    "tests/physical/multi_user_test.sh",
    "tests/physical/adversarial_test.sh",
];

/// The only way a script may execute the CLI.
const ENROLL_BIN_VAR: &str = "\"${SOOS_ENROLL_BIN}\"";

/// Tokens that end the argument list of one invocation.
const INVOCATION_TERMINATORS: [&str; 9] = [
    "2>/dev/null",
    "2>&1",
    "||",
    "&&",
    "|",
    ";",
    ")",
    "then",
    ">",
];

fn workspace_root() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set");
    Path::new(&manifest_dir)
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// Flags accepted by `soos-enroll`, derived from its clap definition.
#[derive(Debug, Default)]
struct CliSpec {
    /// Flags accepted anywhere (`global = true`, plus `-h` / `--help`).
    global: BTreeSet<String>,
    /// Flags accepted only before the subcommand (`-V` / `--version`, non-global `Cli` flags).
    top_level: BTreeSet<String>,
    /// Subcommand name -> flags of its `Args` struct (plus `-h` / `--help`).
    subcommands: BTreeMap<String, BTreeSet<String>>,
}

fn kebab_field(field: &str) -> String {
    field.replace('_', "-")
}

fn kebab_variant(variant: &str) -> String {
    let mut out = String::new();
    for (i, c) in variant.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn quoted_value(item: &str) -> String {
    let value = item.split_once('=').expect("key = value").1.trim();
    value.trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// Parses the `#[arg(...)]` attributes, the `Args` structs and the `Commands` enum.
fn parse_cli_spec(source: &str) -> CliSpec {
    let lines: Vec<&str> = source.lines().collect();
    let mut struct_flags: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut global: BTreeSet<String> = BTreeSet::new();
    let mut current_struct: Option<String> = None;

    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("pub struct ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            current_struct = Some(name);
            continue;
        }
        if !line.starts_with("#[arg(") {
            continue;
        }
        assert!(
            line.ends_with(")]"),
            "{ARGS_RS}:{}: multi-line #[arg(...)] attributes are not supported by this invariant; \
             keep each attribute on one line or extend the parser",
            i + 1
        );
        let owner = current_struct
            .clone()
            .unwrap_or_else(|| panic!("{ARGS_RS}:{}: #[arg] outside a struct", i + 1));
        let field = lines[i + 1..]
            .iter()
            .map(|l| l.trim())
            .find(|l| l.starts_with("pub ") && l.contains(':'))
            .and_then(|l| l.strip_prefix("pub "))
            .and_then(|l| l.split(':').next())
            .unwrap_or_else(|| panic!("{ARGS_RS}:{}: #[arg] without a field", i + 1))
            .trim()
            .to_string();

        let content = &line["#[arg(".len()..line.len() - ")]".len()];
        let mut names = Vec::new();
        let mut is_global = false;
        for item in content.split(',').map(str::trim) {
            if item == "long" {
                names.push(format!("--{}", kebab_field(&field)));
            } else if item.starts_with("long ") || item.starts_with("long=") {
                names.push(format!("--{}", quoted_value(item)));
            } else if item == "short" {
                let first = field.chars().next().expect("non-empty field name");
                names.push(format!("-{first}"));
            } else if item.starts_with("short ") || item.starts_with("short=") {
                names.push(format!("-{}", quoted_value(item)));
            } else if item.replace(' ', "") == "global=true" {
                is_global = true;
            }
        }
        let set = struct_flags.entry(owner).or_default();
        for name in names {
            if is_global {
                global.insert(name.clone());
            }
            set.insert(name);
        }
    }

    let mut spec = CliSpec {
        global,
        ..CliSpec::default()
    };
    for builtin in ["-h", "--help"] {
        spec.global.insert(builtin.to_string());
    }
    spec.top_level
        .extend(["-V".to_string(), "--version".to_string()]);
    if let Some(cli_flags) = struct_flags.get("Cli") {
        spec.top_level.extend(cli_flags.iter().cloned());
    }

    let enum_start = lines
        .iter()
        .position(|l| l.trim() == "pub enum Commands {")
        .expect("args.rs must define `pub enum Commands {`");
    for raw in &lines[enum_start + 1..] {
        let line = raw.trim();
        if line.starts_with('}') {
            break;
        }
        if line.starts_with("//") || line.starts_with('#') || line.is_empty() {
            continue;
        }
        let (variant, rest) = line
            .split_once('(')
            .unwrap_or_else(|| panic!("unexpected Commands variant line: {line}"));
        let args_struct = rest.trim_end_matches(',').trim_end_matches(')');
        let flags = struct_flags.get(args_struct).cloned().unwrap_or_default();
        spec.subcommands
            .insert(kebab_variant(variant.trim()), flags);
    }
    spec
}

/// Joins `\` continuations and removes comment lines and heredoc bodies.
fn logical_lines(script: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = String::new();
    let mut heredoc_end: Option<String> = None;
    for raw in script.lines() {
        if let Some(end) = &heredoc_end {
            if raw.trim() == end {
                heredoc_end = None;
            }
            continue;
        }
        if raw.trim_start().starts_with('#') && pending.is_empty() {
            continue;
        }
        if let Some(pos) = raw.find("<< ") {
            let marker = raw[pos + 3..]
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_matches(|c| c == '\'' || c == '"')
                .to_string();
            if !marker.is_empty() {
                heredoc_end = Some(marker);
            }
        }
        if let Some(stripped) = raw.trim_end().strip_suffix('\\') {
            pending.push_str(stripped);
            pending.push(' ');
            continue;
        }
        pending.push_str(raw);
        out.push(std::mem::take(&mut pending));
    }
    if !pending.is_empty() {
        out.push(pending);
    }
    out
}

/// Bash array assignments `NAME=( ... )` / `NAME+=( ... )` (possibly multi-line) -> tokens.
fn array_assignments(lines: &[String]) -> BTreeMap<String, Vec<String>> {
    let mut arrays: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut open: Option<String> = None;
    for line in lines {
        let trimmed = line.trim();
        if let Some(name) = &open {
            let body = trimmed.trim_end_matches(')');
            arrays
                .entry(name.clone())
                .or_default()
                .extend(body.split_whitespace().map(unquote));
            if trimmed.ends_with(')') {
                open = None;
            }
            continue;
        }
        let Some(pos) = trimmed.find("=(") else {
            continue;
        };
        let name = trimmed[..pos].trim_end_matches('+');
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
            continue;
        }
        let body = &trimmed[pos + 2..];
        let closed = body.trim_end().ends_with(')');
        arrays
            .entry(name.to_string())
            .or_default()
            .extend(body.trim_end_matches(')').split_whitespace().map(unquote));
        if !closed {
            open = Some(name.to_string());
        }
    }
    arrays
}

fn unquote(token: &str) -> String {
    token.trim_matches(|c| c == '"' || c == '\'').to_string()
}

/// Array name referenced as `"${NAME[@]}"`, if any.
fn array_reference(token: &str) -> Option<String> {
    let inner = unquote(token);
    inner
        .strip_prefix("${")
        .and_then(|t| t.strip_suffix("[@]}"))
        .map(str::to_string)
}

fn flag_name(token: &str) -> Option<String> {
    let t = unquote(token);
    let bytes = t.as_bytes();
    let is_flag = (t.starts_with("--") && bytes.get(2).is_some_and(u8::is_ascii_alphabetic))
        || (t.starts_with('-')
            && !t.starts_with("--")
            && bytes.get(1).is_some_and(u8::is_ascii_alphabetic));
    is_flag.then(|| t.split('=').next().unwrap_or_default().to_string())
}

/// Words after which the next word is executed as a command.
const COMMAND_PREFIXES: [&str; 10] = ["if", "elif", "!", "then", "do", "&&", "||", ";", "$(", "("];

/// True when the text before a word leaves that word in command position.
fn is_command_position(before: &str) -> bool {
    let before = before.trim_end();
    before.is_empty() || COMMAND_PREFIXES.iter().any(|p| before.ends_with(p))
}

/// Checks one script and returns (invocation count, violations).
fn check_script(rel: &str, content: &str, spec: &CliSpec) -> (usize, Vec<String>) {
    let lines = logical_lines(content);
    let arrays = array_assignments(&lines);
    let mut violations = Vec::new();
    let mut invocations = 0usize;

    for line in &lines {
        // Literal execution of a soos-enroll path bypasses the checked invocation form.
        let mut words = line.split_whitespace().peekable();
        while let Some(w) = words.peek() {
            if ["if", "elif", "!", "then", "while", "until"].contains(w) {
                words.next();
            } else {
                break;
            }
        }
        if let Some(first) = words.next() {
            let first = unquote(first);
            if !first.contains('=') && first.ends_with("soos-enroll") {
                violations.push(format!(
                    "{rel}: runs `{first}` directly; invoke the CLI only as {ENROLL_BIN_VAR}: `{}`",
                    line.trim()
                ));
            }
        }

        let needle = format!("{ENROLL_BIN_VAR} ");
        let Some(pos) = line
            .match_indices(&needle)
            .map(|(p, _)| p)
            .find(|p| is_command_position(&line[..*p]))
        else {
            continue;
        };
        invocations += 1;
        let mut subcommand: Option<String> = None;
        for token in line[pos + ENROLL_BIN_VAR.len()..].split_whitespace() {
            let bare = unquote(token);
            if INVOCATION_TERMINATORS.contains(&bare.as_str()) || bare.ends_with(')') {
                break;
            }
            let flags: Vec<String> = if let Some(array) = array_reference(token) {
                let tokens = arrays.get(&array).cloned().unwrap_or_else(|| {
                    violations.push(format!("{rel}: array `{array}` is used but never assigned"));
                    Vec::new()
                });
                tokens.iter().filter_map(|t| flag_name(t)).collect()
            } else if let Some(flag) = flag_name(token) {
                vec![flag]
            } else {
                if subcommand.is_none() && spec.subcommands.contains_key(&bare) {
                    subcommand = Some(bare);
                }
                Vec::new()
            };
            for flag in flags {
                let accepted = match &subcommand {
                    None => spec.global.contains(&flag) || spec.top_level.contains(&flag),
                    Some(sub) => {
                        spec.global.contains(&flag)
                            || spec.subcommands.get(sub).is_some_and(|s| s.contains(&flag))
                    }
                };
                if !accepted {
                    let context = subcommand
                        .as_deref()
                        .map_or("before the subcommand".to_string(), |s| {
                            format!("for `{s}`")
                        });
                    violations.push(format!(
                        "{rel}: soos-enroll does not accept `{flag}` {context}: `{}`",
                        line.trim()
                    ));
                }
            }
        }
        if subcommand.is_none() {
            violations.push(format!(
                "{rel}: soos-enroll invocation without a known subcommand: `{}`",
                line.trim()
            ));
        }
    }
    (invocations, violations)
}

fn load_spec() -> CliSpec {
    let source = fs::read_to_string(workspace_root().join(ARGS_RS)).expect("read args.rs");
    parse_cli_spec(&source)
}

/// Every `*.sh` file under `tests/physical/`, sorted.
fn physical_scripts() -> Vec<(String, String)> {
    let root = workspace_root();
    let mut scripts: Vec<(String, String)> = fs::read_dir(root.join(PHYSICAL_DIR))
        .expect("read tests/physical")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "sh"))
        .map(|p| {
            let rel = p
                .strip_prefix(&root)
                .expect("relative path")
                .display()
                .to_string();
            let content = fs::read_to_string(&p).expect("read script");
            (rel, content)
        })
        .collect();
    scripts.sort();
    scripts
}

/// The parser must see the real CLI surface, or the script scan would prove nothing.
#[test]
fn test_enroll_cli_spec_parser_matches_clap_definition() {
    let spec = load_spec();
    for flag in [
        "--biometrics-dir",
        "--key-file",
        "--models-dir",
        "--camera-device",
        "--mock",
        "--help",
    ] {
        assert!(
            spec.global.contains(flag),
            "global flag `{flag}` not parsed: {spec:?}"
        );
    }
    let enroll = spec.subcommands.get("enroll").expect("enroll subcommand");
    for flag in [
        "--uid",
        "-i",
        "--username",
        "-u",
        "--frames",
        "-f",
        "--yes",
        "-y",
    ] {
        assert!(
            enroll.contains(flag),
            "enroll flag `{flag}` not parsed: {enroll:?}"
        );
    }
    let list = spec.subcommands.get("list").expect("list subcommand");
    assert!(
        list.contains("--format"),
        "list --format not parsed: {list:?}"
    );
    assert!(
        spec.subcommands.contains_key("debug-vision"),
        "debug-vision subcommand not parsed"
    );
    let every_flag: BTreeSet<&String> = spec
        .global
        .iter()
        .chain(spec.top_level.iter())
        .chain(spec.subcommands.values().flatten())
        .collect();
    assert!(
        !every_flag.iter().any(|f| f.as_str() == "--skip-root-check"),
        "--skip-root-check must not exist in the CLI (EN11)"
    );
}

/// GitHub #186: physical scripts only pass flags that `soos-enroll` accepts, only through
/// `"${SOOS_ENROLL_BIN}"`, and never the removed `--skip-root-check`.
#[test]
fn test_physical_scripts_only_use_accepted_soos_enroll_flags() {
    let spec = load_spec();
    let mut violations = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (rel, content) in physical_scripts() {
        if content.contains("--skip-root-check") {
            violations.push(format!(
                "{rel}: references the removed `--skip-root-check` flag"
            ));
        }
        if !content.contains("soos-enroll") {
            continue;
        }
        let (count, found) = check_script(&rel, &content, &spec);
        violations.extend(found);
        seen.insert(rel, count);
    }
    for rel in ENROLL_SCRIPTS {
        let count = seen.get(rel).copied().unwrap_or_default();
        if count == 0 {
            violations.push(format!(
                "{rel}: must invoke soos-enroll through {ENROLL_BIN_VAR}"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "physical scripts pass arguments the current soos-enroll CLI rejects:\n{}",
        violations.join("\n")
    );
}

/// GitHub #186: the CLI requires EUID 0 and validates every path as absolute. Scripts that
/// drive it must say so up front instead of failing at the first invocation.
#[test]
fn test_physical_scripts_require_root_and_absolute_paths() {
    let mut violations = Vec::new();
    for rel in ENROLL_SCRIPTS {
        let content = fs::read_to_string(workspace_root().join(rel)).expect("read physical script");
        let has_root_gate = content
            .lines()
            .any(|l| (l.contains("${EUID}") || l.contains("$(id -u)")) && l.contains("-ne 0"));
        if !has_root_gate {
            violations.push(format!(
                "{rel}: must refuse to run soos-enroll unless EUID is 0 (e.g. `if [[ \"${{EUID}}\" -ne 0 ]]`)"
            ));
        }
        for line in content.lines() {
            let t = line.trim();
            for var in ["MODELS_DIR", "BIOMETRICS_DIR", "KEY_FILE"] {
                let Some(value) = t.strip_prefix(&format!("{var}=\"")) else {
                    continue;
                };
                let value = value.trim_end_matches('"');
                if !value.is_empty() && !value.starts_with('/') && !value.starts_with('$') {
                    violations.push(format!(
                        "{rel}: default `{var}=\"{value}\"` is relative; soos-enroll rejects non-absolute paths"
                    ));
                }
            }
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

/// GitHub #186: a physical PAD session verifies against the provisioned template store. A fresh
/// temporary store holds no template, so every presentation would be "rejected" as not enrolled
/// and the APCER / BPCER report would be meaningless.
#[test]
fn test_adversarial_physical_session_uses_an_enrolled_store() {
    let content = fs::read_to_string(workspace_root().join("tests/physical/adversarial_test.sh"))
        .expect("read adversarial_test.sh");
    assert!(
        !content.contains("mktemp"),
        "adversarial_test.sh must not create a temporary (empty) template store"
    );
    let start = content
        .find("if [[ \"${USE_MOCK}\" == \"true\" ]]; then")
        .expect("explicit mock branch");
    let physical = &content[start..];
    let physical = &physical[physical.find("\nelse\n").expect("physical branch")..];
    assert!(
        physical.contains("list --format json"),
        "the physical PAD session must check that the target UID is enrolled before measuring"
    );
}
