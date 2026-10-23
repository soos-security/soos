//! One-command setup, PR A of the owner decisions of 2026-10-23 (GitHub #362; spec
//! `AI/architect_spec_one_command_setup.md` §5, §6, §19; ADR 2026-10-23 "One Command Without
//! Manual Steps", part 1; rows SDP1-SDP26): the preflight dependency matrix (`companion` and
//! `android` package groups, the scoped `REQUIRED_TOOLS` table and its static coverage check, so
//! that no command is discovered missing in the middle of a run), the backup of a `remote.toml`
//! that setup did not write (`remote.toml.bak-<UTC>`, then setup continues), the managed grammar
//! that keeps a trailing `[android]` table, and the follow-ups of the review of PR #473 (the
//! push sender `--prefix` drop-in).
//!
//! The behavioral contracts run the real `scripts/setup.sh` without root against a fake root and
//! a fake HOME (harness cloned from `setup_user_phase_contract.rs`, which stays untouched apart
//! from the acknowledged wiring M3): stub commands first on a restricted `PATH`, then symlinks to
//! a fixed set of harmless tools. No real `sudo`, package manager, `systemctl`, `tailscale`,
//! `jq`, `dig` or `curl` is reachable, and nothing touches the network.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::setup_scanner_contract::{
    command_word_index, lex, names_fixed_program, script_assignments, unquote, variable_refs,
    Origin, VarRef,
};

const SETUP: &str = "scripts/setup.sh";
const DEPS: &str = "scripts/check_build_deps.sh";
const DOCKER_TEST: &str = "tests/docker/setup_packages_test.sh";
const LOGIN: &str = "owner.login@example.com";
const RP_ID: &str = "pc.tail1234.ts.net";
const USER_LOGIN: &str = "me@example.com";
const REMOTE_MARKER: &str =
    "# Managed by scripts/setup.sh (soos-setup): re-running setup rewrites this file.";
const FP_A: &str = "AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99";
const FP_B: &str = "01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF";
/// A value of a key setup does not manage: it must never be printed.
const SENTINEL: &str = "mailto:sentinel-7f3a@example.org";
/// The fixed UTC stamp of the `date` stub (`STUB_DATE_FIXED=1`).
const FIXED_STAMP: &str = "20261023T120000Z";

/// The seven boolean keys, in the order setup writes them.
const BOOL_KEYS: [&str; 7] = [
    "allow_funnel",
    "allow_unlock",
    "password_alerts",
    "push_notifications",
    "offline_alerts",
    "camera_view",
    "camera_view_funnel",
];

/// The scopes of `REQUIRED_TOOLS` (spec R1-4, R2-5).
const SCOPES: [&str; 10] = [
    "base",
    "family:debian",
    "family:fedora",
    "family:arch",
    "escalate",
    "root-start",
    "toolchain",
    "optional",
    "companion",
    "android",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read_repo(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// The script without comment-only lines.
fn code_of(rel: &str) -> String {
    read_repo(rel)
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn print_packages(distro: &str, group: &str) -> Output {
    Command::new("bash")
        .arg(workspace_root().join(DEPS))
        .args(["--distro", distro, "--print-packages", group])
        .output()
        .expect("run check_build_deps.sh")
}

fn packages(distro: &str, group: &str) -> Vec<String> {
    let out = print_packages(distro, group);
    assert!(
        out.status.success(),
        "{distro}/{group}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// `tool -> scope` of the `declare -rA REQUIRED_TOOLS=(...)` table of setup.sh.
fn required_tools() -> BTreeMap<String, String> {
    let text = read_repo(SETUP);
    let start = text
        .find("declare -rA REQUIRED_TOOLS=(")
        .expect("setup.sh must declare `declare -rA REQUIRED_TOOLS=(`");
    let body = &text[start + "declare -rA REQUIRED_TOOLS=(".len()..];
    let body = &body[..body
        .find("\n)")
        .expect("REQUIRED_TOOLS closes with `)` on its own line")];
    let mut map = BTreeMap::new();
    for item in body.split_whitespace() {
        if item.starts_with('#') {
            panic!("no comment inside REQUIRED_TOOLS: {item}");
        }
        let (key, scope) = item
            .split_once("]=")
            .unwrap_or_else(|| panic!("`[tool]=scope` expected: {item}"));
        let key = key
            .strip_prefix('[')
            .unwrap_or_else(|| panic!("`[tool]=scope` expected: {item}"));
        assert!(
            map.insert(key.to_string(), scope.to_string()).is_none(),
            "duplicate REQUIRED_TOOLS key {key}"
        );
    }
    map
}

// ---------------------------------------------------------------------------
// SDP-S: static contracts
// ---------------------------------------------------------------------------

/// SDP1 (SDP-S1): `check_build_deps.sh --print-packages companion|android` prints the §5.1 lists
/// for every family (Fedora's JDK is decided in PR C, see the ADR), the existing groups are
/// unchanged, and an unknown group still exits 2.
#[test]
fn test_sdp1_companion_and_android_groups() {
    for (distro, group, expected) in [
        (
            "debian",
            "companion",
            "jq bind9-dnsutils curl ca-certificates",
        ),
        ("fedora", "companion", "jq bind-utils curl ca-certificates"),
        ("arch", "companion", "jq bind curl ca-certificates"),
        (
            "debian",
            "android",
            "openjdk-17-jdk-headless unzip android-sdk-platform-tools-common",
        ),
        ("fedora", "android", "unzip android-tools"),
        ("arch", "android", "jdk17-openjdk unzip android-udev"),
        (
            "debian",
            "build",
            "build-essential pkg-config libpam0g-dev libclang-dev clang libssl-dev",
        ),
        (
            "fedora",
            "build",
            "gcc gcc-c++ make pkgconf-pkg-config pam-devel clang-devel openssl-devel",
        ),
        ("arch", "build", "base-devel clang openssl pkgconf pam"),
        ("debian", "models", "curl ca-certificates coreutils"),
        (
            "arch",
            "gui",
            "libxkbcommon wayland libglvnd libx11 libxcursor libxi libxrandr",
        ),
    ] {
        assert_eq!(
            packages(distro, group).join(" "),
            expected,
            "{distro}/{group}"
        );
    }
    let out = print_packages("arch", "tailscale-nope");
    assert_eq!(out.status.code(), Some(2), "unknown group exits 2");
    let deps = read_repo(DEPS);
    for name in [
        "DEBIAN_COMPANION=(",
        "FEDORA_COMPANION=(",
        "ARCH_COMPANION=(",
        "DEBIAN_ANDROID=(",
        "FEDORA_ANDROID=(",
        "ARCH_ANDROID=(",
    ] {
        assert!(
            deps.contains(name),
            "check_build_deps.sh must declare {name}"
        );
    }
    assert!(
        deps.contains("<build|gui|models|companion|android>"),
        "the usage names the new groups"
    );
}

/// Every package token of the `_COMPANION` and `_ANDROID` arrays.
fn new_group_tokens() -> Vec<String> {
    let deps = read_repo(DEPS);
    let mut tokens = Vec::new();
    for line in deps.lines() {
        let line = line.trim();
        let Some((name, rest)) = line.split_once("=(") else {
            continue;
        };
        if !(name.ends_with("_COMPANION") || name.ends_with("_ANDROID")) {
            continue;
        }
        for token in rest.trim_end_matches(')').split_whitespace() {
            tokens.push(token.to_string());
        }
    }
    tokens
}

/// SDP2 (SDP-S2, R1-5): no package token of the `_COMPANION`/`_ANDROID` arrays appears in the
/// setup scripts, except as a `REQUIRED_TOOLS` key (`[curl]=`) or as a command word that is
/// itself a `REQUIRED_TOOLS` key.
#[test]
fn test_sdp2_new_package_names_only_in_check_build_deps() {
    let tokens = new_group_tokens();
    assert!(tokens.len() >= 18, "parsed too few tokens: {tokens:?}");
    let tools = required_tools();
    for rel in [
        SETUP,
        "scripts/setup_exposure.sh",
        "scripts/setup_android.sh",
    ] {
        if !workspace_root().join(rel).exists() {
            continue;
        }
        let code = code_of(rel);
        let words: Vec<&str> = code
            .split(|c: char| c.is_whitespace() || "\"'()=;|&".contains(c))
            .collect();
        for token in &tokens {
            let allowed_as_tool = tools.contains_key(token.as_str());
            for word in &words {
                if word == token && !allowed_as_tool {
                    panic!("{rel} names the package '{token}' of check_build_deps.sh");
                }
            }
            if allowed_as_tool {
                // Only as a table key or a command word: never inside a package list.
                for line in code.lines() {
                    let bare = line.split_whitespace().any(|w| w == token);
                    if bare {
                        let as_command = line.trim_start().starts_with(&format!("{token} "))
                            || line.contains(&format!("command -v {token}"));
                        assert!(
                            as_command,
                            "{rel}: '{token}' is a package name; it may appear only as a REQUIRED_TOOLS key or a command word: {line}"
                        );
                    }
                }
            }
        }
    }
}

/// Shell builtins and keywords: never an external command.
const BUILTINS: &[&str] = &[
    ":",
    "[",
    "[[",
    "]]",
    "test",
    "echo",
    "printf",
    "read",
    "local",
    "declare",
    "readonly",
    "typeset",
    "export",
    "unset",
    "set",
    "shift",
    "return",
    "exit",
    "trap",
    "cd",
    "pwd",
    "true",
    "false",
    "wait",
    "break",
    "continue",
    "compgen",
    "mapfile",
    "readarray",
    "umask",
    "case",
    "esac",
    "for",
    "in",
    "do",
    "done",
    "if",
    "fi",
    "then",
    "else",
    "elif",
    "while",
    "until",
    "{",
    "}",
    "(",
    ")",
    "!",
    "let",
    "type",
    "hash",
    "shopt",
    ";;",
    "function",
];

/// The command a variable command word runs, from each of its script-assigned values (`(sudo --)`
/// is `sudo`; an installed soos binary or a repository script by absolute path is exempt).
fn resolve_variable_word(
    word: &str,
    assigned: &std::collections::HashMap<String, Vec<Origin>>,
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for r in variable_refs(word) {
        match r {
            VarRef::Special('@') => {}
            VarRef::Special(c) => return Err(format!("${c} as a command word")),
            VarRef::Indirect(n) => return Err(format!("${{!{n}}} as a command word")),
            VarRef::Named(name) => {
                let Some(origins) = assigned.get(&name) else {
                    return Err(format!("{name} is never assigned"));
                };
                for origin in origins {
                    let Origin::Value(value) = origin else {
                        return Err(format!("{name} is read from input"));
                    };
                    if value.is_empty() {
                        continue;
                    }
                    if names_fixed_program(value) {
                        out.push(format!("path:{}", unquote(value)));
                        continue;
                    }
                    let plain = unquote(value);
                    let plain = plain.trim_start_matches('(').trim_end_matches(')');
                    if let Some(first) = plain.split_whitespace().next() {
                        out.push(first.to_string());
                    }
                }
            }
        }
    }
    Ok(out)
}

/// SDP-S3 scanner: every external command word of `script` that is neither a builtin, a
/// function of the script, an installed soos binary (`soos-*`) or repository script (`*.sh`)
/// by path, nor a key of `keys`.
fn uncovered_commands(script: &str, keys: &BTreeSet<String>) -> Vec<String> {
    let cmds = lex(script);
    let assigned = script_assignments(&cmds);
    let functions: BTreeSet<String> = cmds
        .iter()
        .filter_map(|c| c.words.first())
        .filter_map(|w| w.strip_suffix("()"))
        .map(str::to_string)
        .collect();
    let check = |name: &str, line: usize, text: &str| -> Option<String> {
        let base = name.rsplit('/').next().unwrap_or(name);
        if name.contains('/') && (base.starts_with("soos-") || base.ends_with(".sh")) {
            return None;
        }
        if name.contains('/') {
            return Some(format!("line {line}: absolute path {name}: {text}"));
        }
        if BUILTINS.contains(&name) || functions.contains(name) || keys.contains(name) {
            return None;
        }
        Some(format!(
            "line {line}: `{name}` has no REQUIRED_TOOLS entry: {text}"
        ))
    };
    // A `case` pattern list is read as a "command" ending with an unbalanced `)`; its
    // alternatives before each `|` are single-word "commands" on the same line. They run
    // nothing.
    let unbalanced_close =
        |w: &String| w.ends_with(')') && w.matches('(').count() < w.matches(')').count();
    let mut skip = vec![false; cmds.len()];
    for i in 0..cmds.len() {
        let is_pattern =
            cmds[i].words.len() == 1 && cmds[i].words.last().is_some_and(unbalanced_close);
        if !is_pattern {
            continue;
        }
        skip[i] = true;
        let mut j = i;
        while j > 0
            && cmds[j - 1].line == cmds[i].line
            && cmds[j - 1].words.len() == 1
            && !skip[j - 1]
        {
            j -= 1;
            skip[j] = true;
        }
    }
    let mut found = Vec::new();
    for (i, cmd) in cmds.iter().enumerate() {
        if skip[i] {
            continue;
        }
        let Some(k) = command_word_index(&cmd.words) else {
            continue;
        };
        let raw = &cmd.words[k];
        let text = cmd.words.join(" ");
        if raw.contains('$') {
            if names_fixed_program(raw) {
                found.extend(check(&unquote(raw), cmd.line, &text));
                continue;
            }
            match resolve_variable_word(raw, &assigned) {
                Ok(names) => {
                    for name in names {
                        let name = name.strip_prefix("path:").unwrap_or(&name);
                        found.extend(check(name, cmd.line, &text));
                    }
                }
                Err(e) => found.push(format!("line {}: {e}: {text}", cmd.line)),
            }
            continue;
        }
        let plain = unquote(raw);
        if plain.starts_with('(') || plain.ends_with("()") {
            continue;
        }
        // Only redirections after a subshell (`( ... ) 2>/dev/null`): nothing runs.
        let redirection = plain
            .trim_start_matches(|c: char| c.is_ascii_digit() || c == '&')
            .starts_with(['>', '<']);
        if redirection {
            continue;
        }
        // The one exemption: the bounded read-only `tailscale version` (guarded by
        // `command -v`), which SCN3 forbids naming in any variable, the table included.
        if plain == "tailscale"
            && cmd.words.get(k + 1).map(|w| unquote(w)).as_deref() == Some("version")
        {
            continue;
        }
        found.extend(check(&plain, cmd.line, &text));
    }
    found
}

/// SDP3 (SDP-S3): the tool coverage invariant. Every external command word of setup.sh (SCN
/// lexer: continuation lines, substitutions, variable command words resolved through their
/// script-assigned values) has a `REQUIRED_TOOLS` entry, so a new external command without an
/// entry fails CI. The check is first run on synthetic input (it must reject and accept).
#[test]
fn test_sdp3_every_external_command_has_a_required_tools_entry() {
    let keys: BTreeSet<String> = ["sudo", "grep", "systemctl"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    for (snippet, bad) in [
        ("foo --x\n", true),
        ("x=\"$(frobnicate -q)\"\n", true),
        ("run_bounded 5 systemctl \\\n  frob\nfoo2 a\n", true),
        ("T=(wget -q)\n\"${T[@]}\" x\n", true),
        ("/usr/bin/evil x\n", true),
        ("tailscale serve --bg x\n", true),
        ("out=\"$(run_bounded 5 tailscale version)\"\n", false),
        ("grep -q x f\n", false),
        ("f() { :; }\nf a\n", false),
        ("E=(sudo --)\n\"${E[@]}\" ls\n", false),
        ("A=\"${P}/bin/soos-admin\"\n\"${A}\" add-user x\n", false),
        ("bash_x=1\nrun_bounded 5 systemctl is-active x\n", false),
        ("if grep -q x f; then echo y; fi\n", false),
    ] {
        let found = uncovered_commands(snippet, &keys);
        assert_eq!(
            !found.is_empty(),
            bad,
            "SDP-S3 verdict for {snippet:?} must be {}: {found:?}",
            if bad { "rejected" } else { "accepted" }
        );
    }
    let tools = required_tools();
    let keys: BTreeSet<String> = tools.keys().cloned().collect();
    for rel in [
        SETUP,
        "scripts/setup_exposure.sh",
        "scripts/setup_android.sh",
    ] {
        if !workspace_root().join(rel).exists() {
            continue;
        }
        let found = uncovered_commands(&read_repo(rel), &keys);
        assert!(
            found.is_empty(),
            "{rel} runs commands without a REQUIRED_TOOLS entry:\n{}",
            found.join("\n")
        );
    }
}

/// SDP4: the `REQUIRED_TOOLS` table: scopes from the fixed set, the tools of every scope the
/// spec names, the group tools mapped to their group, and the preflight code that checks the
/// active scopes.
#[test]
fn test_sdp4_required_tools_table_is_scoped() {
    let tools = required_tools();
    for (tool, scope) in &tools {
        assert!(
            SCOPES.contains(&scope.as_str()),
            "{tool}: unknown scope {scope}"
        );
        assert!(
            tool.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)),
            "{tool}: a command name"
        );
    }
    for (tool, scope) in [
        ("jq", "companion"),
        ("dig", "companion"),
        ("curl", "companion"),
        ("java", "android"),
        ("unzip", "android"),
        ("pacman", "family:arch"),
        ("apt-get", "family:debian"),
        ("dpkg-query", "family:debian"),
        ("dnf", "family:fedora"),
        ("rpm", "family:fedora"),
        ("sudo", "escalate"),
        ("doas", "escalate"),
        ("runuser", "root-start"),
        ("cargo", "toolchain"),
        ("gnome-extensions", "optional"),
        ("systemctl", "base"),
        ("bash", "base"),
        ("date", "base"),
    ] {
        assert_eq!(
            tools.get(tool).map(String::as_str),
            Some(scope),
            "REQUIRED_TOOLS[{tool}]"
        );
    }
    // SCN3 forbids the word `tailscale` in any variable of setup.sh, the table included: the
    // bounded `tailscale version` is the one exempted command (SDP-S3, spec §20 F11).
    assert!(!tools.contains_key("tailscale"), "SCN3: no tailscale key");
    let code = code_of(SETUP);
    for needle in [
        "check_scope_tools base",
        "check_scope_tools \"family:${FAMILY}\"",
        "check_group_tools",
        "[FAIL] preflight: %s (package group %s) not found after installing %s",
        "[FAIL] preflight: cargo (toolchain) not found after scripts/install_rustup.sh",
    ] {
        assert!(code.contains(needle), "setup.sh must contain `{needle}`");
    }
}

/// SDP5: the constants of PR A, tied to their sources of truth.
#[test]
fn test_sdp5_constants() {
    let text = read_repo(SETUP);
    for constant in [
        "readonly REMOTE_BACKUP_PREFIX=\"remote.toml.bak-\"",
        "readonly MAX_BACKUP_SUFFIX=9",
        "readonly ANDROID_PACKAGE=\"org.soos.remote\"",
        "readonly MAX_ANDROID_CERT_FINGERPRINTS=2",
    ] {
        assert!(
            text.lines().any(|l| l.trim() == constant),
            "setup.sh must declare `{constant}`"
        );
    }
    let gradle = read_repo("android/app/build.gradle.kts");
    assert!(
        gradle.contains("applicationId = \"org.soos.remote\""),
        "ANDROID_PACKAGE equals the applicationId"
    );
    let remote = read_repo("crates/remote/src/lib.rs");
    assert!(
        remote.contains("pub const MAX_ANDROID_CERT_FINGERPRINTS: usize = 2;"),
        "MAX_ANDROID_CERT_FINGERPRINTS equals soos-remote's bound"
    );
    // The backup is created exclusively (noclobber, O_EXCL) under umask 077, never through a
    // link, and compared byte for byte before the managed file replaces it.
    let code = code_of(SETUP);
    let start = code
        .find("backup_user_config() {")
        .expect("setup.sh must define backup_user_config");
    let body = &code[start..];
    let body = &body[..body.find("\n}\n").expect("end of backup_user_config")];
    for needle in [
        "set -C",
        "umask 077",
        "-L",
        "cmp -s --",
        "MAX_BACKUP_SUFFIX",
        "chmod 0600",
    ] {
        assert!(
            body.contains(needle),
            "backup_user_config must use `{needle}`"
        );
    }
    assert!(
        !body.contains("cp ") && !body.contains("rm "),
        "the backup is never copied through cp (links) nor removed"
    );
}

// ---------------------------------------------------------------------------
// Behavioral harness (cloned from setup_user_phase_contract.rs)
// ---------------------------------------------------------------------------

const SAFE_TOOLS: &[&str] = &[
    "bash", "env", "cat", "cp", "mv", "rm", "mkdir", "chmod", "sed", "grep", "head", "tail", "tr",
    "cut", "sort", "wc", "stat", "touch", "find", "mktemp", "dirname", "basename", "cmp",
    "install", "timeout", "ln", "readlink", "uname", "ls", "rmdir", "sleep", "tee", "date",
    "realpath", "printf", "true", "false", "od", "cksum",
];

const STUB_PROLOGUE: &str = r#"
__uid=1000
[[ "${STUB_AS_ROOT:-0}" == 1 ]] && __uid=0
printf 'uid=%s %s\n' "${__uid}" "$(basename "$0") $*" >> "${STUB_LOG}"
"#;

const STUB_SUDO: &str = r#"
[[ "${1:-}" == "--" ]] && shift
keep=(PATH="${PATH}" LC_ALL=C HOME="${STUB_ROOT_HOME}" STUB_AS_ROOT=1 SUDO_USER="${STUB_USER:-alice}" SUDO_UID="${STUB_UID}")
while IFS= read -r name; do
    [[ "${name}" == STUB_* && "${name}" != STUB_AS_ROOT ]] && keep+=("${name}=${!name}")
done < <(compgen -e)
exec env -i "${keep[@]}" "$@"
"#;

const STUB_RUNUSER: &str = r#"
[[ "${1:-}" == "-u" ]] || exit 64
shift 2
[[ "${1:-}" == "--" ]] && shift
export STUB_AS_ROOT=0
exec "$@"
"#;

const STUB_ID: &str = r#"
args=()
for a in "$@"; do [[ "$a" == "--" ]] || args+=("$a"); done
root=0
[[ "${STUB_AS_ROOT:-0}" == 1 ]] && root=1
case "${args[*]}" in
    "-u") if (( root )); then echo 0; else echo "${STUB_UID}"; fi ;;
    "-un") if (( root )); then echo root; else echo alice; fi ;;
    "-u alice") echo "${STUB_UID}" ;;
    "-u root") echo 0 ;;
    "-nG alice")
        g="alice ${STUB_EXTRA_GROUPS:-}"
        [[ -e "${STUB_STATE}/in_group" ]] && g="${g} soos"
        echo "${g}" ;;
    *) exit 1 ;;
esac
"#;

const STUB_GETENT: &str = r#"
case "$*" in
    "group soos") [[ -e "${STUB_STATE}/group_soos" ]] && echo "soos:x:964:alice" || exit 2 ;;
    "passwd alice"|"passwd ${STUB_UID}") echo "alice:x:${STUB_UID}:${STUB_UID}:Alice:${STUB_HOME}:/bin/bash" ;;
    "passwd root"|"passwd 0") echo "root:x:0:0:root:/root:/bin/bash" ;;
    *) exit 2 ;;
esac
"#;

const STUB_LOGINCTL: &str = r#"
case "${1:-}" in
    show-user) echo "Display=c2"; exit 0 ;;
    show-session) echo "Remote=no"; echo "Seat=seat0" ;;
    *) exit 1 ;;
esac
"#;

/// System units: one state pair; `--user` units: their own state files.
const STUB_SYSTEMCTL: &str = r#"
if [[ "${1:-}" == "--user" ]]; then
    shift
    verb="${1:-}"
    shift || true
    units=()
    now=0
    for a in "$@"; do
        case "$a" in
            --now) now=1 ;;
            --quiet|-q) ;;
            *) units+=("$a") ;;
        esac
    done
    rc=0
    for u in "${units[@]}"; do
        case "${verb}" in
            is-enabled) [[ -e "${STUB_STATE}/user_${u}_enabled" ]] || rc=1 ;;
            is-active) [[ -e "${STUB_STATE}/user_${u}_active" ]] || rc=3 ;;
            enable)
                touch "${STUB_STATE}/user_${u}_enabled"
                if (( now )); then
                    [[ "${STUB_USER_START_RC:-0}" == 0 ]] || exit "${STUB_USER_START_RC}"
                    touch "${STUB_STATE}/user_${u}_active"
                fi ;;
            disable)
                rm -f "${STUB_STATE}/user_${u}_enabled"
                (( now )) && rm -f "${STUB_STATE}/user_${u}_active" ;;
            start|restart)
                [[ "${STUB_USER_START_RC:-0}" == 0 ]] || exit "${STUB_USER_START_RC}"
                touch "${STUB_STATE}/user_${u}_active" ;;
            try-restart) ;;
            stop) rm -f "${STUB_STATE}/user_${u}_active" ;;
        esac
    done
    exit "${rc}"
fi
case "${1:-}" in
    is-enabled) [[ -e "${STUB_STATE}/unit_enabled" ]] ;;
    is-active) [[ -e "${STUB_STATE}/unit_active" ]] ;;
    stop) rm -f "${STUB_STATE}/unit_active" ;;
    start|restart) touch "${STUB_STATE}/unit_active" ;;
    *) exit 0 ;;
esac
"#;

/// `-S` marks the packages installed unless `STUB_PACMAN_NOOP=1` (a package that installs
/// nothing, SDP-B2).
const STUB_PACMAN: &str = r#"
case "${1:-}" in
    -Q)
        shift
        rc=0
        for p in "$@"; do
            if [[ -e "${STUB_STATE}/pkg_${p}" ]]; then echo "${p} 1.0-1"; else rc=1; fi
        done
        exit "${rc}" ;;
    -S)
        [[ "${STUB_PACMAN_NOOP:-0}" == 1 ]] && exit 0
        for p in "$@"; do [[ "$p" == -* ]] || touch "${STUB_STATE}/pkg_${p}"; done ;;
    *) exit 1 ;;
esac
"#;

const STUB_CARGO: &str = r#"
out="${CARGO_TARGET_DIR:?}/release"
mkdir -p "${out}"
printf 'daemon\n' > "${out}/soos-daemon"
cp "${STUB_FAKES}/soos-admin" "${out}/soos-admin"
cp "${STUB_FAKES}/soos-enroll" "${out}/soos-enroll"
cp "${STUB_FAKES}/soos-remote" "${out}/soos-remote"
cp "${STUB_FAKES}/soos-push-sender" "${out}/soos-push-sender"
printf 'gui\n' > "${out}/soos-gui"
printf 'pam\n' > "${out}/libpam_soos.so"
chmod 0755 "${out}/soos-daemon" "${out}/soos-admin" "${out}/soos-enroll" "${out}/soos-gui" \
    "${out}/soos-remote" "${out}/soos-push-sender"
"#;

const STUB_LOG_ONLY: &str = "\nexit 0\n";

const STUB_TAILSCALE: &str = r#"
[[ "${1:-}" == version ]] && printf '%s\n  tailscale commit: stub\n' "${STUB_TS_VERSION:-1.102.4}"
exit 0
"#;

/// `date -u +%Y%m%dT%H%M%SZ` prints a fixed stamp with `STUB_DATE_FIXED=1`; anything else runs
/// the real `date` of the tools directory.
const STUB_DATE: &str = r#"
if [[ "${STUB_DATE_FIXED:-0}" == 1 && "$*" == "-u +%Y%m%dT%H%M%SZ" ]]; then
    echo "20261023T120000Z"
    exit 0
fi
exec "${STUB_TOOLS}/date" "$@"
"#;

const FAKE_SOOS_ADMIN: &str = r#"
case "${1:-}" in
    add-user) touch "${STUB_STATE}/in_group" ;;
    camera) echo "selected: /dev/video0 (stub)" ;;
    *) exit 0 ;;
esac
"#;

const FAKE_SOOS_ENROLL: &str = r#"
if [[ -e "${STUB_STATE}/unit_active" ]]; then exit 1; fi
dir=""
while [[ $# -gt 0 ]]; do
    case "$1" in --biometrics-dir) dir="$2"; shift 2 ;; *) shift ;; esac
done
[[ -n "${dir}" ]] || exit 64
mkdir -p "${dir}"
touch "${dir}/${STUB_UID}.cbor.enc"
"#;

/// Fake `soos-remote`: validates the prerequisites the real service enforces (exit 78),
/// including `[android]` needing `allow_funnel = true`; `STUB_REMOTE_RC` forces a status.
const FAKE_SOOS_REMOTE: &str = r#"
cfg=""
[[ "${1:-}" == "--config" ]] && cfg="${2:-}"
[[ "${STUB_REMOTE_RC:-0}" == 0 ]] || exit "${STUB_REMOTE_RC}"
[[ -f "${cfg}" ]] || exit 78
mode="$(stat -c '%a' "${cfg}")"
[[ "${mode}" == 600 ]] || exit 78
has() { grep -qx "$1" "${cfg}"; }
grep -q '^allowed_logins = \["[^"]' "${cfg}" || exit 78
grep -q '^rp_id = "' "${cfg}" || exit 78
if has 'offline_alerts = true'; then has 'push_notifications = true' || exit 78; fi
if has 'push_notifications = true'; then has 'password_alerts = true' || exit 78; fi
if has 'camera_view_funnel = true'; then
    has 'camera_view = true' || exit 78
    has 'allow_funnel = true' || exit 78
fi
if has '[android]'; then has 'allow_funnel = true' || exit 78; fi
echo "No passkey registered."
"#;

const STUB_INSTALL_SH: &str = r##"
printf 'install-sh-args %s\n' "$*" >> "${STUB_LOG}"
artifacts="" prefix=/usr sysconfdir=/etc localstatedir=/var pamdir=/usr/lib/security
while [[ $# -gt 0 ]]; do
    case "$1" in
        --artifact-dir) artifacts="$2"; shift 2 ;;
        --prefix) prefix="$2"; shift 2 ;;
        --sysconfdir) sysconfdir="$2"; shift 2 ;;
        --localstatedir) localstatedir="$2"; shift 2 ;;
        --runstatedir) shift 2 ;;
        --pam-dir) pamdir="$2"; shift 2 ;;
        *) shift ;;
    esac
done
repo="$(cd "$(dirname "$0")/.." && pwd)"
mkdir -p "${prefix}/bin" "${prefix}/libexec/soos" "${pamdir}" "${sysconfdir}/systemd/system" \
    "${localstatedir}/lib/soos/models" "${localstatedir}/lib/soos/biometrics" "${prefix}/share/soos/pam"
cp "${repo}/scripts/provision_master_key.sh" "${prefix}/libexec/soos/provision-master-key"
[[ -e "${localstatedir}/lib/soos/master.key" ]] || printf 'k' > "${localstatedir}/lib/soos/master.key"
cp "${repo}/packaging/pam/arch/system-auth.snippet" "${prefix}/share/soos/pam/system-auth.snippet"
touch "${STUB_STATE}/group_soos"
cp "${artifacts}/soos-daemon" "${prefix}/libexec/soos/soos-daemon"
for b in soos-admin soos-enroll soos-gui; do cp "${artifacts}/${b}" "${prefix}/bin/${b}"; done
cp "${artifacts}/libpam_soos.so" "${pamdir}/pam_soos.so"
cp "${repo}/packaging/soos-daemon.service" "${sysconfdir}/systemd/system/soos-daemon.service"
cp "${repo}/models/manifest.toml" "${localstatedir}/lib/soos/models/manifest.toml"
if [[ -f "${artifacts}/soos-push-sender" ]]; then
    cp "${artifacts}/soos-push-sender" "${prefix}/libexec/soos/soos-push-sender"
    for u in soos-push-sender@.socket soos-push-sender@.service; do
        cp "${repo}/packaging/${u}" "${sysconfdir}/systemd/system/${u}"
    done
    if [[ "${prefix}" != /usr ]]; then
        d="${sysconfdir}/systemd/system/soos-push-sender@.service.d"
        mkdir -p "${d}"
        printf '%s\n' "# Written by scripts/install.sh for --prefix ${prefix} (ADR 2026-10-23 \"Push Sender as a Sandboxed System Service\")." \
            "[Service]" "ExecStart=" "ExecStart=${prefix}/libexec/soos/soos-push-sender --system-instance %i" > "${d}/prefix.conf"
    fi
fi
touch "${STUB_STATE}/unit_enabled" "${STUB_STATE}/unit_active"
"##;

const STUB_INSTALL_REMOTE: &str = r#"
[[ "${STUB_AS_ROOT:-0}" == 1 ]] && exit 1
repo="$(cd "$(dirname "$0")/.." && pwd)"
conf="${XDG_CONFIG_HOME:-$HOME/.config}"
install -Dm755 "${CARGO_TARGET_DIR:?}/release/soos-remote" "${HOME}/.local/bin/soos-remote"
install -Dm755 "${CARGO_TARGET_DIR}/release/soos-push-sender" "${HOME}/.local/bin/soos-push-sender"
install -Dm644 "${repo}/packaging/soos-remote.service" "${conf}/systemd/user/soos-remote.service"
install -Dm644 "${repo}/packaging/soos-push-sender.service" "${conf}/systemd/user/soos-push-sender.service"
if [[ ! -e "${conf}/soos/remote.toml" ]]; then
    umask 077
    mkdir -p "${conf}/soos"
    printf '# template\nallowed_logins = []\n' > "${conf}/soos/remote.toml"
fi
"#;

/// `STUB_RUSTUP_NOOP=1`: installs nothing (SDP-B2); otherwise a cargo in ~/.cargo/bin.
const STUB_RUSTUP_SH: &str = r#"
[[ "${STUB_RUSTUP_NOOP:-0}" == 1 ]] && exit 0
mkdir -p "${HOME}/.cargo/bin"
cp "${STUB_FAKES}/cargo" "${HOME}/.cargo/bin/cargo"
chmod 0755 "${HOME}/.cargo/bin/cargo"
"#;

const STUB_READY_SH: &str = "\nexit 0\n";

const STUB_MODELS_SH: &str = r#"
[[ " $* " == *" --check-only "* ]] || exit 64
exit 0
"#;

const ARCH_OS_RELEASE: &str =
    "NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\nBUILD_ID=rolling\n";

struct Harness {
    dir: PathBuf,
    repo: PathBuf,
    stubs: PathBuf,
    root: PathBuf,
    home: PathBuf,
    state: PathBuf,
    log: PathBuf,
    env: Vec<(String, String)>,
}

fn real_uid() -> String {
    let out = Command::new("id").arg("-u").output().expect("run id -u");
    let uid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!uid.is_empty() && uid.bytes().all(|b| b.is_ascii_digit()));
    assert_ne!(uid, "0", "the setup contract runs as a normal user");
    uid
}

fn bash_path() -> PathBuf {
    ["/usr/bin/bash", "/bin/bash"]
        .iter()
        .map(PathBuf::from)
        .find(|p| is_executable(p))
        .expect("bash available")
}

fn write_exec(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, body).expect("write stub");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
}

fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .permissions()
        .mode()
        & 0o7777
}

impl Harness {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("soos_setup_deps_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        let stubs = dir.join("stubs");
        let tools = dir.join("tools");
        let fakes = dir.join("fakes");
        let root = dir.join("root");
        let home = dir.join("home/alice");
        let runtime = dir.join("runtime");
        let state = dir.join("stub_state");
        let log = dir.join("stub.log");
        for d in [
            &repo, &stubs, &tools, &fakes, &root, &home, &runtime, &state,
        ] {
            fs::create_dir_all(d).expect("mkdir");
        }
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).expect("runtime");
        fs::create_dir_all(dir.join("tmp")).expect("tmp");
        fs::write(&log, "").expect("log");

        let real = workspace_root();
        for rel in [
            SETUP,
            DEPS,
            "scripts/provision_master_key.sh",
            "packaging/soos-daemon.service",
            "packaging/soos-remote.service",
            "packaging/soos-push-sender.service",
            "packaging/soos-push-sender@.socket",
            "packaging/soos-push-sender@.service",
            "packaging/pam/arch/system-auth.snippet",
            "models/manifest.toml",
        ] {
            let dst = repo.join(rel);
            fs::create_dir_all(dst.parent().expect("parent")).expect("mkdir");
            fs::copy(real.join(rel), &dst).expect("copy repo file");
        }
        let bash = bash_path();
        let shebang = format!("#!{}\n", bash.display());
        let uid = real_uid();
        let baked = format!(
            ": \"${{STUB_LOG:={}}}\" \"${{STUB_STATE:={}}}\" \"${{STUB_HOME:={}}}\" \"${{STUB_FAKES:={}}}\" \"${{STUB_TOOLS:={}}}\" \"${{STUB_UID:={uid}}}\"\nexport STUB_LOG STUB_STATE STUB_HOME STUB_FAKES STUB_TOOLS STUB_UID\n",
            log.display(),
            state.display(),
            home.display(),
            fakes.display(),
            tools.display()
        );
        let stub = |body: &str| format!("{shebang}{baked}{STUB_PROLOGUE}{body}");
        for (rel, body) in [
            ("scripts/install.sh", STUB_INSTALL_SH),
            ("scripts/install_rustup.sh", STUB_RUSTUP_SH),
            ("scripts/wait_daemon_ready.sh", STUB_READY_SH),
            ("scripts/download_models.sh", STUB_MODELS_SH),
            ("scripts/install_remote.sh", STUB_INSTALL_REMOTE),
        ] {
            write_exec(&repo.join(rel), &stub(body));
        }
        for (name, body) in [
            ("sudo", STUB_SUDO),
            ("runuser", STUB_RUNUSER),
            ("loginctl", STUB_LOGINCTL),
            ("systemctl", STUB_SYSTEMCTL),
            ("pacman", STUB_PACMAN),
            ("cargo", STUB_CARGO),
            ("gpasswd", STUB_LOG_ONLY),
            ("tailscale", STUB_TAILSCALE),
            ("curl", STUB_LOG_ONLY),
            ("jq", STUB_LOG_ONLY),
            ("dig", STUB_LOG_ONLY),
        ] {
            write_exec(&stubs.join(name), &stub(body));
        }
        write_exec(&fakes.join("cargo"), &stub(STUB_CARGO));
        write_exec(&stubs.join("date"), &format!("{shebang}{baked}{STUB_DATE}"));
        write_exec(&stubs.join("id"), &format!("{shebang}{baked}{STUB_ID}"));
        write_exec(
            &stubs.join("getent"),
            &format!("{shebang}{baked}{STUB_GETENT}"),
        );
        write_exec(&fakes.join("soos-admin"), &stub(FAKE_SOOS_ADMIN));
        write_exec(&fakes.join("soos-enroll"), &stub(FAKE_SOOS_ENROLL));
        write_exec(&fakes.join("soos-remote"), &stub(FAKE_SOOS_REMOTE));
        write_exec(&fakes.join("soos-push-sender"), &stub("\nexit 0\n"));

        let path_var = std::env::var_os("PATH").expect("PATH");
        for tool in SAFE_TOOLS {
            if let Some(found) = std::env::split_paths(&path_var)
                .map(|d| d.join(tool))
                .find(|p| is_executable(p))
            {
                std::os::unix::fs::symlink(&found, tools.join(tool)).expect("symlink tool");
            }
        }

        fs::create_dir_all(root.join("run/systemd/system")).expect("systemd dir");
        fs::create_dir_all(root.join("etc/systemd/system")).expect("unit dir");
        fs::create_dir_all(dir.join("roothome")).expect("root home");
        fs::write(root.join("etc/os-release"), ARCH_OS_RELEASE).expect("os-release");
        for group in ["build", "models", "gui", "companion"] {
            for p in packages("arch", group) {
                fs::write(state.join(format!("pkg_{p}")), "").expect("pkg");
            }
        }

        let path = format!("{}:{}", stubs.display(), tools.display());
        let env = vec![
            ("PATH".to_string(), path),
            ("HOME".to_string(), home.display().to_string()),
            ("LC_ALL".to_string(), "C".to_string()),
            ("TMPDIR".to_string(), dir.join("tmp").display().to_string()),
            ("XDG_RUNTIME_DIR".to_string(), runtime.display().to_string()),
            ("STUB_LOG".to_string(), log.display().to_string()),
            ("STUB_STATE".to_string(), state.display().to_string()),
            ("STUB_HOME".to_string(), home.display().to_string()),
            ("STUB_FAKES".to_string(), fakes.display().to_string()),
            ("STUB_TOOLS".to_string(), tools.display().to_string()),
            ("STUB_UID".to_string(), uid),
            (
                "STUB_ROOT_HOME".to_string(),
                dir.join("roothome").display().to_string(),
            ),
        ];
        Self {
            dir,
            repo,
            stubs,
            root,
            home,
            state,
            log,
            env,
        }
    }

    fn set_env(&mut self, key: &str, value: &str) {
        self.env.retain(|(k, _)| k != key);
        self.env.push((key.to_string(), value.to_string()));
    }

    fn run_with_root_args(&self, root_args: &[String], args: &[&str]) -> Output {
        fs::write(&self.log, "").expect("reset log");
        let mut cmd = Command::new(bash_path());
        cmd.arg(self.repo.join(SETUP))
            .args(root_args)
            .args(args)
            .current_dir(&self.dir)
            .env_clear()
            .stdin(Stdio::null());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd.output().expect("run setup.sh")
    }

    fn root_args(&self, prefix: &str) -> Vec<String> {
        let root = |rel: &str| self.root.join(rel).display().to_string();
        vec![
            "--prefix".to_string(),
            prefix.to_string(),
            "--sysconfdir".to_string(),
            root("etc"),
            "--localstatedir".to_string(),
            root("var"),
            "--runstatedir".to_string(),
            root("run"),
            "--pam-dir".to_string(),
            root("usr/lib/security"),
        ]
    }

    fn run(&self, args: &[&str]) -> Output {
        let prefix = self.root.join("usr").display().to_string();
        self.run_with_root_args(&self.root_args(&prefix), args)
    }

    /// `--yes` plus the Tailscale login and node name.
    fn run_companion(&self, extra: &[&str]) -> Output {
        let mut args = vec!["--yes", "--tailscale-login", LOGIN, "--rp-id", RP_ID];
        args.extend_from_slice(extra);
        self.run(&args)
    }

    fn log_lines(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .expect("read log")
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn config_dir(&self) -> PathBuf {
        self.home.join(".config/soos")
    }

    fn remote_toml(&self) -> PathBuf {
        self.config_dir().join("remote.toml")
    }

    /// Writes a user file (0600 in a 0700 directory).
    fn user_file(&self, content: &str) {
        let dir = self.config_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(self.remote_toml(), content).unwrap();
        fs::set_permissions(self.remote_toml(), fs::Permissions::from_mode(0o600)).unwrap();
    }

    /// The `remote.toml.bak-*` files of the configuration directory.
    fn backups(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = fs::read_dir(self.config_dir())
            .map(|d| {
                d.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with("remote.toml.bak-"))
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    fn user_unit(&self, unit: &str, what: &str) -> bool {
        self.state.join(format!("user_{unit}_{what}")).exists()
    }

    fn snapshot(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for base in [&self.root, &self.home, &self.repo, &self.dir.join("tmp")] {
            snapshot_into(base, base, &mut map);
        }
        map
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn snapshot_into(base: &Path, dir: &Path, map: &mut BTreeMap<String, String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let meta = fs::symlink_metadata(&path).expect("metadata");
        let key = format!("{}:{}", base.display(), path.display());
        let mode = meta.permissions().mode() & 0o7777;
        if meta.file_type().is_symlink() {
            map.insert(
                key,
                format!("link {:?}", fs::read_link(&path).expect("link")),
            );
        } else if meta.is_dir() {
            map.insert(key, format!("dir {mode:o}"));
            snapshot_into(base, &path, map);
        } else {
            let content = fs::read(&path).expect("read");
            map.insert(key, format!("file {mode:o} {content:?}"));
        }
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn code(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

fn find_line(lines: &[String], needles: &[&str]) -> usize {
    lines
        .iter()
        .position(|l| needles.iter().all(|n| l.contains(n)))
        .unwrap_or_else(|| panic!("no log line with {needles:?} in:\n{}", lines.join("\n")))
}

fn has_line(lines: &[String], needles: &[&str]) -> bool {
    lines.iter().any(|l| needles.iter().all(|n| l.contains(n)))
}

/// The managed `remote.toml` for this login and these booleans, with an optional `[android]`
/// block (the fingerprints in order).
fn managed(login: &str, values: [bool; 7], android: &[&str]) -> String {
    let mut s = String::new();
    s.push_str(REMOTE_MARKER);
    s.push('\n');
    s.push_str(
        "# A feature turned off later (for example in the soos-gui feature tour) stays off until\n",
    );
    s.push_str("# setup runs with its --<feature> on flag. Reference: Docs/REMOTE_COMPANION.md section 5.\n");
    s.push_str(&format!("allowed_logins = [\"{login}\"]\n"));
    s.push_str(&format!("rp_id = \"{RP_ID}\"\n"));
    for (key, value) in BOOL_KEYS.iter().zip(values) {
        s.push_str(&format!("{key} = {value}\n"));
    }
    if !android.is_empty() {
        let list: Vec<String> = android.iter().map(|f| format!("\"{f}\"")).collect();
        s.push_str(&format!(
            "\n[android]\npackage = \"org.soos.remote\"\nsha256_cert_fingerprints = [{}]\n",
            list.join(", ")
        ));
    }
    s
}

fn android_block(fps: &[&str]) -> String {
    let list: Vec<String> = fps.iter().map(|f| format!("\"{f}\"")).collect();
    format!(
        "\n[android]\npackage = \"org.soos.remote\"\nsha256_cert_fingerprints = [{}]\n",
        list.join(", ")
    )
}

/// True when `name` is `remote.toml.bak-<YYYYmmddTHHMMSSZ>` (optionally `-<1..9>`).
fn is_backup_name(name: &str, suffix: Option<u8>) -> bool {
    let Some(stamp) = name.strip_prefix("remote.toml.bak-") else {
        return false;
    };
    let (stamp, tail) = stamp.split_at(stamp.len().min(16));
    let b = stamp.as_bytes();
    let stamp_ok = b.len() == 16
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'T'
        && b[9..15].iter().all(u8::is_ascii_digit)
        && b[15] == b'Z';
    stamp_ok
        && match suffix {
            None => tail.is_empty(),
            Some(n) => tail == format!("-{n}"),
        }
}

// ---------------------------------------------------------------------------
// SDP-B: preflight dependency matrix
// ---------------------------------------------------------------------------

/// SDP6 (SDP-B1): a missing package of the `companion` group is installed in the package step,
/// before the build, when the companion runs; with `--no-remote` the group is never queried.
#[test]
fn test_sdp6_companion_group_installed_before_the_build() {
    let h = Harness::new("b1");
    fs::remove_file(h.state.join("pkg_jq")).unwrap();
    let dry = h.run(&["--dry-run", "--tailscale-login", LOGIN, "--rp-id", RP_ID]);
    assert_eq!(code(&dry), 0, "{}", text(&dry));
    assert!(
        text(&dry).contains("[PLAN] packages: will run") && text(&dry).contains("companion"),
        "{}",
        text(&dry)
    );
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let lines = h.log_lines();
    let install = find_line(&lines, &["uid=0 pacman -S --needed --noconfirm", "jq"]);
    let build = find_line(&lines, &["uid=1000 cargo build"]);
    assert!(install < build, "packages before the build: {lines:?}");
    assert!(
        !lines[install].contains(" bind ") && !lines[install].contains(" curl"),
        "only the missing package: {}",
        lines[install]
    );

    let h = Harness::new("b1noremote");
    fs::remove_file(h.state.join("pkg_jq")).unwrap();
    let out = h.run(&["--yes", "--no-remote"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let lines = h.log_lines();
    assert!(
        !has_line(&lines, &["pacman -Q jq"]) && !has_line(&lines, &["pacman -S"]),
        "without the companion the companion group is never queried: {lines:?}"
    );
}

/// SDP7 (SDP-B2): the tools of the requested groups are checked after the package step and
/// before the build. A missing `companion` tool never fails the run (the Tailscale steps stay
/// printed); a toolchain still missing after `install_rustup.sh` fails with exit 1 before any
/// build or escalation of the system phase.
#[test]
fn test_sdp7_tools_checked_after_the_package_step() {
    let mut h = Harness::new("b2");
    fs::remove_file(h.state.join("pkg_jq")).unwrap();
    fs::remove_file(h.stubs.join("jq")).unwrap();
    h.set_env("STUB_PACMAN_NOOP", "1");
    let out = h.run_companion(&[]);
    assert_eq!(
        code(&out),
        0,
        "a companion tool never fails: {}",
        text(&out)
    );
    let t = text(&out);
    let note = t
        .find("preflight: jq (package group companion) not found")
        .unwrap_or_else(|| panic!("the missing tool is named:\n{t}"));
    let build = t.find("Building: cargo build").expect("build line");
    assert!(note < build, "named before the build:\n{t}");

    let mut h = Harness::new("b2cargo");
    fs::remove_file(h.stubs.join("cargo")).unwrap();
    h.set_env("STUB_RUSTUP_NOOP", "1");
    let out = h.run(&["--yes", "--no-remote"]);
    assert_eq!(code(&out), 1, "{}", text(&out));
    assert!(
        text(&out).contains(
            "[FAIL] preflight: cargo (toolchain) not found after scripts/install_rustup.sh"
        ),
        "{}",
        text(&out)
    );
    let lines = h.log_lines();
    assert!(has_line(&lines, &["uid=1000 install_rustup.sh"]));
    assert!(!has_line(&lines, &["--system-phase"]), "{lines:?}");
}

/// SDP8: a required base command missing from `PATH` is refused in preflight (exit 2, nothing
/// changed), before any escalation.
#[test]
fn test_sdp8_missing_base_command_refused_in_preflight() {
    let h = Harness::new("base");
    let tools = h.dir.join("tools");
    fs::remove_file(tools.join("cksum")).unwrap();
    let before = h.snapshot();
    let out = h.run(&["--yes", "--no-remote"]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert!(
        text(&out).contains("cksum"),
        "the missing command is named:\n{}",
        text(&out)
    );
    assert!(!has_line(&h.log_lines(), &["sudo"]));
    assert_eq!(before, h.snapshot());
}

// ---------------------------------------------------------------------------
// SDP-B: an existing remote.toml not written by setup (OD-T3)
// ---------------------------------------------------------------------------

/// SDP9 (SDP-B4): with the identity flags, a user file is backed up to exactly one
/// `remote.toml.bak-<UTC>` (0600, byte-equal) and setup writes the managed file, keeps the
/// explicit `false`, enables the units, names the backup and never prints a value of the file;
/// the names of the keys it no longer sets are listed.
#[test]
fn test_sdp9_user_file_backed_up_and_replaced() {
    let h = Harness::new("b4");
    let mine = format!(
        "# my own file\nallowed_logins = [\"{USER_LOGIN}\"]\nrp_id = \"{RP_ID}\"\nallow_funnel = false\nvapid_subject = \"{SENTINEL}\"\npoll_interval_ms = 500\n"
    );
    h.user_file(&mine);
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let backups = h.backups();
    assert_eq!(backups.len(), 1, "exactly one backup: {backups:?}");
    let name = backups[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(is_backup_name(&name, None), "{name}");
    assert_eq!(fs::read_to_string(&backups[0]).unwrap(), mine);
    assert_eq!(mode_of(&backups[0]), 0o600);
    assert_eq!(
        fs::read_to_string(h.remote_toml()).unwrap(),
        managed(LOGIN, [false, true, true, true, true, true, false], &[])
    );
    assert_eq!(mode_of(&h.remote_toml()), 0o600);
    assert!(
        !h.config_dir().join("remote.toml.soos-setup.bak").exists(),
        "the timestamped backup is the only copy"
    );
    assert!(h.user_unit("soos-remote.service", "enabled"));
    assert!(h.user_unit("soos-remote.service", "active"));
    let t = text(&out);
    assert!(t.contains("[PLAN] companion config: back up"), "{t}");
    assert!(
        t.contains(&format!(
            "[DONE] companion config: your previous remote.toml was saved as {}",
            backups[0].display()
        )) && t.contains("setup now manages remote.toml"),
        "{t}"
    );
    for value in [SENTINEL, USER_LOGIN, LOGIN] {
        assert!(
            !t.contains(value),
            "no value of the file is printed ({value}):\n{t}"
        );
    }
    assert!(
        t.contains("vapid_subject") && t.contains("poll_interval_ms"),
        "the names of the keys no longer set are listed:\n{t}"
    );
    // R2-10: keys absent from the file and turned on by the default profile are named.
    assert!(
        t.contains("turned on by setup's default profile: allow_unlock, password_alerts, push_notifications, offline_alerts, camera_view"),
        "{t}"
    );
}

/// SDP10 (SDP-B5, R2-9): without identity flags the identity comes from the user file itself;
/// without any identity the file is untouched and kept, as before.
#[test]
fn test_sdp10_identity_from_the_user_file_else_kept() {
    let h = Harness::new("b5");
    let mine = format!("allowed_logins = [ \"{USER_LOGIN}\" ]   # me\nrp_id=\"{RP_ID}\"\n");
    h.user_file(&mine);
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(
        fs::read_to_string(h.remote_toml()).unwrap(),
        managed(USER_LOGIN, [true; 7], &[])
    );
    assert_eq!(h.backups().len(), 1);
    assert!(!text(&out).contains(USER_LOGIN), "{}", text(&out));
    assert!(h.user_unit("soos-remote.service", "enabled"));

    let h = Harness::new("b5none");
    let mine =
        "allowed_logins = [\"a@example.com\", \"b@example.com\"]\nrp_id = \"pc.tail1234.ts.net\"\n";
    h.user_file(mine);
    let before = h.snapshot();
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(fs::read_to_string(h.remote_toml()).unwrap(), mine);
    assert!(h.backups().is_empty());
    assert!(
        text(&out).contains("[PLAN] companion config: kept"),
        "{}",
        text(&out)
    );
    assert!(!has_line(&h.log_lines(), &["systemctl --user enable"]));
    let after = h.snapshot();
    let config_key = |m: &BTreeMap<String, String>| {
        m.iter()
            .filter(|(k, _)| k.contains(".config/soos/remote.toml"))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(config_key(&before), config_key(&after));
}

/// SDP11 (SDP-B6): a name that exists is never overwritten or followed: suffixes `-1`..`-9`,
/// then exit 5 with the user file untouched; a symbolic link in place of the name counts as
/// taken and is never written through.
#[test]
fn test_sdp11_backup_name_collisions_and_links() {
    let mut h = Harness::new("b6");
    h.set_env("STUB_DATE_FIXED", "1");
    let mine = format!("allowed_logins = [\"{USER_LOGIN}\"]\nrp_id = \"{RP_ID}\"\n");
    h.user_file(&mine);
    let taken = h
        .config_dir()
        .join(format!("remote.toml.bak-{FIXED_STAMP}"));
    fs::write(&taken, "older backup\n").unwrap();
    let target = h.dir.join("link-target");
    std::os::unix::fs::symlink(
        &target,
        h.config_dir()
            .join(format!("remote.toml.bak-{FIXED_STAMP}-1")),
    )
    .unwrap();
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(fs::read_to_string(&taken).unwrap(), "older backup\n");
    assert!(!target.exists(), "a link is never written through");
    let second = h
        .config_dir()
        .join(format!("remote.toml.bak-{FIXED_STAMP}-2"));
    assert_eq!(fs::read_to_string(&second).unwrap(), mine);
    assert!(is_backup_name(
        &format!("remote.toml.bak-{FIXED_STAMP}-2"),
        Some(2)
    ));

    let mut h = Harness::new("b6full");
    h.set_env("STUB_DATE_FIXED", "1");
    h.user_file(&mine);
    fs::write(
        h.config_dir()
            .join(format!("remote.toml.bak-{FIXED_STAMP}")),
        "x",
    )
    .unwrap();
    for n in 1..=9 {
        fs::write(
            h.config_dir()
                .join(format!("remote.toml.bak-{FIXED_STAMP}-{n}")),
            "x",
        )
        .unwrap();
    }
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 5, "{}", text(&out));
    assert_eq!(fs::read_to_string(h.remote_toml()).unwrap(), mine);
    assert!(!h.user_unit("soos-remote.service", "enabled"));
}

/// SDP12 (SDP-B7, SDP-B8): a symbolic link in place of `remote.toml` is still refused (exit 2,
/// target untouched); a companion failure after the backup restores the user file (0600),
/// keeps the backup and exits 5.
#[test]
fn test_sdp12_link_refused_and_failure_restores_the_user_file() {
    let h = Harness::new("b7");
    fs::create_dir_all(h.config_dir()).unwrap();
    let target = h.dir.join("elsewhere.toml");
    fs::write(&target, "allowed_logins = [\"me@example.com\"]\n").unwrap();
    std::os::unix::fs::symlink(&target, h.remote_toml()).unwrap();
    let before = h.snapshot();
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert_eq!(before, h.snapshot());
    assert!(h.backups().is_empty());

    let mut h = Harness::new("b8");
    h.set_env("STUB_REMOTE_RC", "78");
    let mine =
        format!("allowed_logins = [\"{USER_LOGIN}\"]\nrp_id = \"{RP_ID}\"\nallow_unlock = false\n");
    h.user_file(&mine);
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 5, "{}", text(&out));
    assert_eq!(fs::read_to_string(h.remote_toml()).unwrap(), mine);
    assert_eq!(mode_of(&h.remote_toml()), 0o600);
    let backups = h.backups();
    assert_eq!(backups.len(), 1, "the backup is kept: {backups:?}");
    assert_eq!(fs::read_to_string(&backups[0]).unwrap(), mine);
    assert!(!h.user_unit("soos-remote.service", "enabled"));
}

/// SDP13: a user file larger than the read bound is never backed up partially: with the
/// identity flags the run is refused in preflight (exit 2, nothing changed); without them the
/// file is kept.
#[test]
fn test_sdp13_oversized_user_file_refused_before_change() {
    let h = Harness::new("big");
    let mut big = format!("allowed_logins = [\"{USER_LOGIN}\"]\nrp_id = \"{RP_ID}\"\n");
    while big.len() <= 16384 {
        big.push_str("# padding padding padding padding padding padding padding padding\n");
    }
    h.user_file(&big);
    let before = h.snapshot();
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert!(text(&out).contains("16384"), "{}", text(&out));
    assert_eq!(before, h.snapshot());
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(fs::read_to_string(h.remote_toml()).unwrap(), big);
    assert!(h.backups().is_empty());
}

/// SDP14 (SDP-B11, R2-1): the seven booleans of a user file are read at top level with a
/// tolerant, fail-safe rule: `key=false` and `key = false # c` are `false`, a value the rule does
/// not recognize is `false` (named in the plan, never printed), and the same key below a table
/// header (`[ android ]` included) is ignored.
#[test]
fn test_sdp14_tolerant_fail_safe_booleans_of_a_user_file() {
    for (tag, body, expected, unrecognized) in [
        (
            "nospace",
            "allow_funnel=false\n",
            [false, true, true, true, true, true, false],
            false,
        ),
        (
            "comment",
            "allow_funnel = false # tailnet only\n",
            [false, true, true, true, true, true, false],
            false,
        ),
        (
            "quoted",
            "allow_funnel = \"false\"\n",
            [false, true, true, true, true, true, false],
            true,
        ),
        (
            "yes",
            "camera_view = yes\n",
            [true, true, true, true, true, false, false],
            true,
        ),
        (
            "below",
            "[ android ]\nallow_funnel = false\n",
            [true; 7],
            false,
        ),
        (
            "belowother",
            "[camera]\npush_notifications = false\n",
            [true; 7],
            false,
        ),
    ] {
        let h = Harness::new(&format!("b11{tag}"));
        h.user_file(&format!(
            "allowed_logins = [\"{USER_LOGIN}\"]\nrp_id = \"{RP_ID}\"\n{body}"
        ));
        let out = h.run_companion(&[]);
        assert_eq!(code(&out), 0, "{tag}: {}", text(&out));
        assert_eq!(
            fs::read_to_string(h.remote_toml()).unwrap(),
            managed(LOGIN, expected, &[]),
            "{tag}"
        );
        let t = text(&out);
        assert!(!has_line(&h.log_lines(), &["tailscale funnel"]), "{tag}");
        if unrecognized {
            assert!(t.contains("value not recognized"), "{tag}: {t}");
            assert!(
                !t.contains("= \"false\"") && !t.contains("= yes"),
                "{tag}: the value is never printed: {t}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// SDP-B: the [android] table (§6.3, R1-9, R2-11)
// ---------------------------------------------------------------------------

/// SDP15 (SDP-B10): a managed file with a trailing `[android]` block stays managed: a run
/// without `--android` changes nothing ("already done"); a malformed block (lower-case hex,
/// three entries, a duplicate, a key after it, another package) makes the file a user file,
/// which the identity flags back up.
#[test]
fn test_sdp15_managed_grammar_keeps_a_trailing_android_block() {
    let h = Harness::new("b10");
    let first = h.run_companion(&[]);
    assert_eq!(code(&first), 0, "{}", text(&first));
    for fps in [&[FP_A][..], &[FP_A, FP_B]] {
        let content = managed(LOGIN, [true; 7], fps);
        fs::write(h.remote_toml(), &content).unwrap();
        let before = h.snapshot();
        let out = h.run(&["--yes"]);
        assert_eq!(code(&out), 0, "{}", text(&out));
        assert_eq!(fs::read_to_string(h.remote_toml()).unwrap(), content);
        assert!(
            text(&out).contains("[PLAN] companion config: already done"),
            "{}",
            text(&out)
        );
        assert!(h.backups().is_empty());
        assert_eq!(before, h.snapshot(), "nothing changed");
    }

    let lower = FP_A.to_ascii_lowercase();
    for (tag, tail) in [
        ("lower", android_block(&[lower.as_str()])),
        ("three", android_block(&[FP_A, FP_B, FP_A])),
        ("dup", android_block(&[FP_A, FP_A])),
        (
            "after",
            format!("{}allow_unlock = false\n", android_block(&[FP_A])),
        ),
        (
            "package",
            android_block(&[FP_A]).replace("org.soos.remote", "org.example.other"),
        ),
    ] {
        let h = Harness::new(&format!("b10{tag}"));
        let first = h.run_companion(&[]);
        assert_eq!(code(&first), 0, "{}", text(&first));
        let content = format!("{}{tail}", managed(LOGIN, [true; 7], &[]));
        fs::write(h.remote_toml(), &content).unwrap();
        let out = h.run_companion(&[]);
        assert_eq!(code(&out), 0, "{tag}: {}", text(&out));
        let backups = h.backups();
        assert_eq!(backups.len(), 1, "{tag}: the malformed file is a user file");
        assert_eq!(fs::read_to_string(&backups[0]).unwrap(), content);
        assert_eq!(
            fs::read_to_string(h.remote_toml()).unwrap(),
            managed(LOGIN, [true; 7], &[]),
            "{tag}: a block that fails the strict grammar is not carried over"
        );
    }
}

/// SDP16 (SDP-B12, R1-9): a user file whose trailing `[android]` table passes the strict grammar
/// is carried over byte-identically into the managed file.
#[test]
fn test_sdp16_user_android_block_carried_over() {
    let h = Harness::new("b12");
    h.user_file(&format!(
        "allowed_logins = [\"{USER_LOGIN}\"]\nrp_id = \"{RP_ID}\"\nallow_funnel = true\n{}",
        android_block(&[FP_A, FP_B])
    ));
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let written = fs::read_to_string(h.remote_toml()).unwrap();
    assert_eq!(written, managed(LOGIN, [true; 7], &[FP_A, FP_B]));
    assert!(
        written.ends_with(&android_block(&[FP_A, FP_B])),
        "the block is last"
    );
    // A second run is idempotent.
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(fs::read_to_string(h.remote_toml()).unwrap(), written);
    assert!(text(&out).contains("[PLAN] companion config: already done"));

    // The documented hand-written block (Docs/REMOTE_COMPANION.md, trailing comments) is
    // carried over too; the comments are not.
    let h = Harness::new("b12doc");
    h.user_file(&format!(
        "allowed_logins = [\"{USER_LOGIN}\"]\nrp_id = \"{RP_ID}\"\n\n[android]   # the app\npackage = \"org.soos.remote\"\nsha256_cert_fingerprints = [\"{FP_A}\"]   # 1 or 2 entries, upper-case, 32 pairs\n"
    ));
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(
        fs::read_to_string(h.remote_toml()).unwrap(),
        managed(LOGIN, [true; 7], &[FP_A])
    );
    assert!(!text(&out).contains("not carried over (see the backup)"));
}

/// SDP17 (SDP-B13): a binding (carried over or managed) while Funnel resolves off (`--funnel
/// off`, or the user file's explicit `allow_funnel = false`) is refused in preflight: exit 2,
/// nothing changed, the message says the app needs Funnel.
#[test]
fn test_sdp17_android_binding_needs_funnel() {
    let h = Harness::new("b13");
    h.user_file(&format!(
        "allowed_logins = [\"{USER_LOGIN}\"]\nrp_id = \"{RP_ID}\"\n{}",
        android_block(&[FP_A])
    ));
    let before = h.snapshot();
    let out = h.run_companion(&["--funnel", "off"]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert!(
        text(&out)
            .contains("the Android app needs Funnel: remove the [android] table or keep Funnel on"),
        "{}",
        text(&out)
    );
    assert_eq!(before, h.snapshot());

    let h = Harness::new("b13file");
    h.user_file(&format!(
        "allowed_logins = [\"{USER_LOGIN}\"]\nrp_id = \"{RP_ID}\"\nallow_funnel = false\n{}",
        android_block(&[FP_A])
    ));
    let before = h.snapshot();
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert_eq!(before, h.snapshot());

    let h = Harness::new("b13managed");
    let first = h.run_companion(&[]);
    assert_eq!(code(&first), 0, "{}", text(&first));
    fs::write(h.remote_toml(), managed(LOGIN, [true; 7], &[FP_A])).unwrap();
    let before = h.snapshot();
    let out = h.run_companion(&["--funnel", "off"]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert_eq!(before, h.snapshot());
}

/// SDP18 (R2-11): a user `[android]` table that fails the strict grammar is not carried over and
/// does not trigger the Funnel refusal; the run says so.
#[test]
fn test_sdp18_non_strict_user_android_table_not_carried() {
    let h = Harness::new("r211");
    h.user_file(&format!(
        "allowed_logins = [\"{USER_LOGIN}\"]\nrp_id = \"{RP_ID}\"\nallow_funnel = false\n\n[ android ]\npackage = \"org.soos.remote\"\nsha256_cert_fingerprints = [\"{}\"]\n",
        FP_A.to_ascii_lowercase()
    ));
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(
        fs::read_to_string(h.remote_toml()).unwrap(),
        managed(LOGIN, [false, true, true, true, true, true, false], &[])
    );
    assert!(
        text(&out).contains("your [android] table was not carried over (see the backup)"),
        "{}",
        text(&out)
    );
}

// ---------------------------------------------------------------------------
// SDP-F: follow-ups of the review of PR #473 (push sender --prefix drop-in)
// ---------------------------------------------------------------------------

const DROP_IN_REL: &str = "etc/systemd/system/soos-push-sender@.service.d/prefix.conf";

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soos_setup_deps_f_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn artifacts(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    for name in [
        "soos-daemon",
        "soos-admin",
        "soos-enroll",
        "libpam_soos.so",
        "soos-gui",
        "soos-push-sender",
    ] {
        let path = dir.join(name);
        fs::write(&path, format!("fixture:{name}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn run_install(stage: &Path, art: &Path, extra: &[&str]) -> Output {
    Command::new("bash")
        .arg(workspace_root().join("scripts/install.sh"))
        .args([
            OsStr::new("--destdir"),
            stage.as_os_str(),
            OsStr::new("--artifact-dir"),
            art.as_os_str(),
            OsStr::new("--pam-dir"),
            OsStr::new("/usr/lib/security"),
            OsStr::new("--skip-systemd"),
            OsStr::new("--skip-models"),
        ])
        .args(extra)
        .output()
        .expect("run install.sh")
}

fn run_uninstall(stage: &Path, prefix: &str) -> Output {
    Command::new("bash")
        .arg(workspace_root().join("scripts/uninstall.sh"))
        .args([
            OsStr::new("--destdir"),
            stage.as_os_str(),
            OsStr::new("--prefix"),
            OsStr::new(prefix),
            OsStr::new("--keep-data"),
            OsStr::new("--skip-systemd"),
        ])
        .output()
        .expect("run uninstall.sh")
}

const ADMIN_DROP_IN: &str = "# An administrator's own override.\n[Service]\nEnvironment=X=1\n";

/// SDP19 (follow-up 1): `install.sh --prefix /usr` and `uninstall.sh` remove
/// `soos-push-sender@.service.d/prefix.conf` only when it carries the install.sh marker line,
/// like the package hooks; an administrator's file of that name is kept.
#[test]
fn test_sdp19_drop_in_removed_only_with_the_marker() {
    let dir = scratch("install_keep");
    let (stage, art) = (dir.join("stage"), dir.join("art"));
    artifacts(&art);
    let drop_in = stage.join(DROP_IN_REL);
    fs::create_dir_all(drop_in.parent().unwrap()).unwrap();
    fs::write(&drop_in, ADMIN_DROP_IN).unwrap();
    let out = run_install(&stage, &art, &[]);
    assert!(out.status.success(), "install.sh failed:\n{}", text(&out));
    assert_eq!(
        fs::read_to_string(&drop_in).unwrap(),
        ADMIN_DROP_IN,
        "install.sh --prefix /usr keeps an unmarked prefix.conf"
    );
    let out = run_uninstall(&stage, "/usr");
    assert!(out.status.success(), "uninstall.sh failed:\n{}", text(&out));
    assert_eq!(
        fs::read_to_string(&drop_in).unwrap(),
        ADMIN_DROP_IN,
        "uninstall.sh keeps an unmarked prefix.conf"
    );
    let _ = fs::remove_dir_all(&dir);

    // The marked file of an earlier --prefix install is still removed by both.
    let dir = scratch("install_marked");
    let (stage, art) = (dir.join("stage"), dir.join("art"));
    artifacts(&art);
    let out = run_install(&stage, &art, &["--prefix", "/opt/soos"]);
    assert!(out.status.success(), "install.sh failed:\n{}", text(&out));
    let drop_in = stage.join(DROP_IN_REL);
    assert!(drop_in.is_file());
    let out = run_uninstall(&stage, "/opt/soos");
    assert!(out.status.success(), "uninstall.sh failed:\n{}", text(&out));
    assert!(!drop_in.exists(), "a marked drop-in is removed");
    let _ = fs::remove_dir_all(&dir);
}

/// SDP20 (follow-up 2): for a `--prefix` other than `/usr`, "system: already done" also needs
/// the install.sh drop-in for that prefix: a deleted drop-in makes setup run install.sh again.
#[test]
fn test_sdp20_system_up_to_date_compares_the_prefix_drop_in() {
    let h = Harness::new("f2");
    let prefix = h.root.join("opt/soos").display().to_string();
    let args = h.root_args(&prefix);
    let first = h.run_with_root_args(&args, &["--yes", "--no-remote"]);
    assert_eq!(code(&first), 0, "{}", text(&first));
    let drop_in = h.root.join(DROP_IN_REL);
    assert!(drop_in.is_file(), "the stub install.sh wrote the drop-in");
    let again = h.run_with_root_args(&args, &["--yes", "--no-remote"]);
    assert_eq!(code(&again), 0, "{}", text(&again));
    assert!(
        text(&again).contains("[DONE] system: already done"),
        "{}",
        text(&again)
    );
    fs::remove_file(&drop_in).unwrap();
    let out = h.run_with_root_args(&args, &["--yes", "--no-remote"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(
        has_line(&h.log_lines(), &["uid=0 install.sh"]),
        "a missing drop-in re-runs install.sh: {}",
        text(&out)
    );
}

/// SDP21 (follow-up 3): a trailing slash (and the other text normalisations) of `--prefix` is
/// removed before install.sh, which refuses non-normalised prefixes.
#[test]
fn test_sdp21_prefix_normalised_before_install_sh() {
    let h = Harness::new("f3");
    let prefix = format!("{}//opt/./soos/", h.root.display());
    let args = h.root_args(&prefix);
    let out = h.run_with_root_args(&args, &["--yes", "--no-remote"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let lines = h.log_lines();
    let line = &lines[find_line(&lines, &["install-sh-args"])];
    assert!(
        line.contains(&format!("--prefix {}/opt/soos ", h.root.display())),
        "{line}"
    );
}

// ---------------------------------------------------------------------------
// SDP-D and traceability
// ---------------------------------------------------------------------------

/// SDP22 (SDP-D1): the Docker package matrix exists, checks every group of check_build_deps.sh
/// in the pinned images (ubuntu:24.04, fedora:40, a current Fedora, Arch) and the
/// `REQUIRED_TOOLS` of each installed group, and is a required CI job.
#[test]
fn test_sdp22_docker_package_matrix_wired_into_ci() {
    assert!(
        is_executable(&workspace_root().join(DOCKER_TEST)),
        "{DOCKER_TEST} must be executable"
    );
    let script = read_repo(DOCKER_TEST);
    for needle in [
        "ubuntu:24.04@sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3",
        "fedora:40@sha256:3c86d25fef9d2001712bc3d9b091fc40cf04be4767e48f1aa3b785bf58d300ed",
        "fedora:44@sha256:",
        "archlinux:latest@sha256:b21322c663be387c0ed9cbc7bbbfe18e41633ad4e7b7c77cfad45f128be20040",
        "--print-packages",
        "apt-cache show",
        "repoquery",
        "pacman -Si",
        "REQUIRED_TOOLS",
        "command -v",
        "build gui models companion android",
        ":/workspace:ro",
    ] {
        assert!(
            script.contains(needle),
            "{DOCKER_TEST} must contain `{needle}`"
        );
    }
    let ci = read_repo(".github/workflows/ci.yml");
    let start = ci
        .find("\n  setup-packages:")
        .expect("CI job setup-packages");
    let job = &ci[start + 1..];
    let job = &job[..job.find("\n  # ----").unwrap_or(job.len())];
    for needle in [
        "needs: lint",
        "timeout-minutes:",
        "persist-credentials: false",
        "run: ./tests/docker/setup_packages_test.sh",
    ] {
        assert!(job.contains(needle), "setup-packages: `{needle}`");
    }
    let needs = ci
        .lines()
        .find(|l| l.trim_start().starts_with("needs: [lint, clippy"))
        .expect("ci-success needs");
    assert!(needs.contains("setup-packages"), "required: {needs}");
}

/// SDP23: the rendered `[android]` block is byte-equal to the fixture `soos-remote` parses
/// (`crates/remote/tests/setup_render_tests.rs`).
#[test]
fn test_sdp23_android_render_matches_the_parser_fixture() {
    let fixture = read_repo("tests/fixtures/setup/remote_android.toml");
    let h = Harness::new("fixture");
    h.user_file(&format!(
        "allowed_logins = [\"{USER_LOGIN}\"]\nrp_id = \"{RP_ID}\"\n{}",
        android_block(&[FP_A, FP_B])
    ));
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(fs::read_to_string(h.remote_toml()).unwrap(), fixture);
}

/// SDP24: documentation, ADR, matrix rows and walkthrough 256.
#[test]
fn test_sdp24_docs_adr_matrix_walkthrough() {
    let pkg = read_repo("Docs/PACKAGING_AND_PROVISIONING.md");
    for needle in [
        "remote.toml.bak-",
        "REQUIRED_TOOLS",
        "--print-packages <build|gui|models|companion|android>",
        "setup_packages_test.sh",
        "bind9-dnsutils",
    ] {
        assert!(
            pkg.contains(needle),
            "PACKAGING_AND_PROVISIONING.md: {needle}"
        );
    }
    let remote = read_repo("Docs/REMOTE_COMPANION.md");
    assert!(
        remote.contains("remote.toml.bak-"),
        "REMOTE_COMPANION.md names the backup"
    );
    let adr = read_repo("AI/DECISIONS.md");
    assert!(
        adr.contains("One Command Without Manual Steps")
            && adr.contains("part 1")
            && adr.contains("java-25"),
        "AI/DECISIONS.md must carry part 1 of the ADR with the Fedora JDK record"
    );
    let matrix = read_repo("AI/VERIFICATION_MATRIX.md");
    for row in ["| SDP1 |", "| SDP24 |", "| SDP25 |", "| SDP26 |"] {
        assert!(matrix.contains(row), "matrix row {row}");
    }
    assert!(
        workspace_root()
            .join("AI/walkthroughs/256_setup_preflight_deps.md")
            .is_file(),
        "walkthrough 256 must exist"
    );
}
