//! One-command setup `scripts/setup.sh` core (GitHub #372, PR 1 of #362; ADR 2026-10-13
//! "One-Command Setup `scripts/setup.sh` and Default-On Installer Profile"; spec
//! `AI/architect_spec_one_command_install.md`; rows SET1-SET18).
//!
//! Static contracts read the script; behavioral contracts run it without root against a fake
//! root (`--prefix`, `--sysconfdir`, `--localstatedir`, `--runstatedir`, `--pam-dir` below a
//! scratch directory) in a fake checkout whose helper scripts are stubs, with a restricted
//! `PATH`: stub commands first, then symlinks to a fixed set of harmless tools. No real `sudo`,
//! package manager, `systemctl`, `loginctl`, `cargo` or `tailscale` is reachable.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const SETUP: &str = "scripts/setup.sh";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read_repo(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// The script without comment-only lines (a `#` after code is kept: it never hides a command
/// in this script, and keeping it makes the checks stricter).
fn setup_code() -> String {
    read_repo(SETUP)
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Static contracts
// ---------------------------------------------------------------------------

/// SET1: present, executable, strict bash, no `eval`, os-release parsed and never sourced.
#[test]
fn test_set_s1_script_is_strict_bash_without_eval() {
    let path = workspace_root().join(SETUP);
    let mode = fs::metadata(&path)
        .expect("scripts/setup.sh must exist")
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111, "scripts/setup.sh must be executable");
    let text = read_repo(SETUP);
    assert!(
        text.starts_with("#!/usr/bin/env bash\n"),
        "setup.sh must be a bash script"
    );
    assert!(
        text.lines().any(|l| l.trim() == "set -euo pipefail"),
        "setup.sh must run with set -euo pipefail"
    );
    let code = setup_code();
    for word in code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        assert_ne!(word, "eval", "setup.sh must never use eval");
    }
    for line in code.lines() {
        let t = line.trim_start();
        assert!(
            !(t.starts_with("source ") || t.starts_with(". ")) || !t.contains("os-release"),
            "os-release must be parsed, never sourced: {line}"
        );
        assert!(
            !t.contains("source ") || !t.contains("release"),
            "os-release must never be sourced: {line}"
        );
    }
    assert!(
        code.contains("while IFS= read -r"),
        "os-release must be parsed line by line"
    );
}

/// SET2: the package lists come only from `scripts/check_build_deps.sh --print-packages`;
/// no package name of those lists is repeated in setup.sh.
#[test]
fn test_set_s2_package_lists_come_only_from_check_build_deps() {
    let code = setup_code();
    assert!(
        code.contains("check_build_deps.sh") && code.contains("--print-packages"),
        "setup.sh must read the package lists from check_build_deps.sh --print-packages"
    );
    for group in ["build", "models", "gui"] {
        assert!(
            code.contains(group),
            "setup.sh must request the {group} group"
        );
    }
    let deps = read_repo("scripts/check_build_deps.sh");
    let mut tokens = Vec::new();
    for line in deps.lines() {
        let line = line.trim();
        let Some((name, rest)) = line.split_once("=(") else {
            continue;
        };
        if !(name.ends_with("_BUILD") || name.ends_with("_GUI") || name.ends_with("_MODELS")) {
            continue;
        }
        for token in rest.trim_end_matches(')').split_whitespace() {
            tokens.push(token.to_string());
        }
    }
    assert!(
        tokens.len() > 20,
        "parsed too few package names from check_build_deps.sh"
    );
    // Shell words (a path such as `packaging/pam/arch` is one word, not a package name).
    let words: Vec<&str> = code
        .split(|c: char| c.is_whitespace() || "\"'()=;|&".contains(c))
        .collect();
    for token in &tokens {
        assert!(
            !words.contains(&token.as_str()),
            "setup.sh names the package '{token}' of check_build_deps.sh"
        );
    }
    for token in tokens
        .iter()
        .filter(|t| t.contains('-') || t.chars().any(|c| c.is_ascii_digit()))
    {
        assert!(
            !code.contains(token.as_str()),
            "setup.sh duplicates the package name '{token}' of check_build_deps.sh"
        );
    }
}

/// SET3: this PR never touches PAM, Tailscale, the remote companion, the daemon `[preview]`
/// section, the network lock, never downloads anything itself and never loosens a mode.
#[test]
fn test_set_s3_no_pam_tailscale_remote_or_network_lock_action() {
    let code = setup_code();
    for line in code.lines() {
        if let Some(pos) = line.replace("command -v tailscale ", "").find("tailscale ") {
            let line = line.replace("command -v tailscale ", "");
            assert!(
                line[pos..].starts_with("tailscale version"),
                "only the read-only `tailscale version` is allowed: {line}"
            );
        }
        for banned in ["$EUID", "${EUID", "$UID", "${UID"] {
            assert!(
                !line.contains(banned),
                "use `id -u`, never {banned}: {line}"
            );
        }
    }
    for forbidden in [
        "pam-auth-update",
        "authselect ",
        "gdm enable",
        "/pam.d",
        "network-lock install",
        "install_remote.sh",
        "[preview]",
        "remote.toml",
        "curl ",
        "wget ",
        "chmod 666",
        "chmod 777",
        "chmod a+w",
        "chmod o+w",
        "pkexec ",
        "-Sy",
    ] {
        assert!(
            !code.contains(forbidden),
            "setup.sh must not contain '{forbidden}' in this PR"
        );
    }
    assert!(
        code.contains("install_rustup.sh"),
        "the toolchain must come only from the verified scripts/install_rustup.sh"
    );
    assert!(
        code.contains("--release --locked --workspace"),
        "setup.sh must build with cargo build --release --locked --workspace"
    );
}

/// SET4: bounded state file and documented constants.
#[test]
fn test_set_s4_bounded_constants() {
    let text = read_repo(SETUP);
    for constant in [
        "readonly MAX_STATE_BYTES=4096",
        "readonly SETUP_STATE_FORMAT=1",
        "readonly READY_TIMEOUT_S=30",
        "readonly CAMERA_TIMEOUT_S=15",
        "readonly LOGINCTL_TIMEOUT_S=5",
        "readonly MAX_USER_LEN=32",
        "readonly EXIT_REFUSED=4",
    ] {
        assert!(
            text.lines().any(|l| l.trim() == constant),
            "setup.sh must declare `{constant}`"
        );
    }
}

/// SET17: documentation, ADR, matrix and walkthrough.
#[test]
fn test_set_s5_docs_adr_matrix_and_walkthrough() {
    let readme = read_repo("README.md");
    assert!(
        readme.contains("./scripts/setup.sh"),
        "README must lead its install section with ./scripts/setup.sh"
    );
    let setup_pos = readme.find("./scripts/setup.sh").expect("setup in README");
    let manual_pos = readme
        .find("sudo ./scripts/install.sh --build")
        .expect("manual procedure kept in README");
    assert!(
        setup_pos < manual_pos,
        "the one command must come before the manual procedure"
    );
    let pkg = read_repo("Docs/PACKAGING_AND_PROVISIONING.md");
    for needle in [
        "scripts/setup.sh",
        "--dry-run",
        "setup.json",
        "local seat",
        "soos-setup",
    ] {
        assert!(
            pkg.contains(needle),
            "Docs/PACKAGING_AND_PROVISIONING.md must mention {needle}"
        );
    }
    let adr = read_repo("AI/DECISIONS.md");
    assert!(
        adr.contains("One-Command Setup `scripts/setup.sh` and Default-On Installer Profile"),
        "AI/DECISIONS.md must carry the setup ADR"
    );
    let matrix = read_repo("AI/VERIFICATION_MATRIX.md");
    assert!(
        matrix.contains("| SET1 |") && matrix.contains("| SET18 |"),
        "the matrix must carry rows SET1-SET18"
    );
    assert!(
        workspace_root()
            .join("AI/walkthroughs/200_one_command_install.md")
            .is_file(),
        "walkthrough 200 must exist"
    );
}

// ---------------------------------------------------------------------------
// Behavioral harness
// ---------------------------------------------------------------------------

const SAFE_TOOLS: &[&str] = &[
    "bash", "env", "cat", "cp", "mv", "rm", "mkdir", "chmod", "sed", "grep", "head", "tail", "tr",
    "cut", "sort", "wc", "stat", "touch", "find", "mktemp", "dirname", "basename", "cmp",
    "install", "timeout", "ln", "readlink", "uname", "ls", "rmdir", "sleep", "tee", "date",
    "realpath", "printf", "true", "false", "od",
];

/// Common prologue of every stub: logs `uid=<0|1000> <name> <args>`.
const STUB_PROLOGUE: &str = r#"
__uid=1000
[[ "${STUB_AS_ROOT:-0}" == 1 ]] && __uid=0
printf 'uid=%s %s\n' "${__uid}" "$(basename "$0") $*" >> "${STUB_LOG}"
"#;

/// Emulates `env_reset`: only PATH, LC_ALL, the STUB_* variables and the sudo variables
/// reach the command (no HOME of the user, no TMPDIR, no SSH_*, no XDG_SESSION_ID).
const STUB_SUDO: &str = r#"
[[ "${1:-}" == "--" ]] && shift
keep=(PATH="${PATH}" LC_ALL=C HOME="${STUB_ROOT_HOME}" STUB_AS_ROOT=1 SUDO_USER="${STUB_USER:-alice}" SUDO_UID=1000)
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
    "-u") if (( root )); then echo 0; else echo 1000; fi ;;
    "-un") if (( root )); then echo root; else echo alice; fi ;;
    "-u alice") echo 1000 ;;
    "-u root") echo 0 ;;
    "-nG alice")
        if [[ -e "${STUB_STATE}/in_group" ]]; then echo "alice wheel soos"; else echo "alice wheel"; fi ;;
    *) exit 1 ;;
esac
"#;

const STUB_GETENT: &str = r#"
case "$*" in
    "group soos") [[ -e "${STUB_STATE}/group_soos" ]] && echo "soos:x:964:alice" || exit 2 ;;
    "passwd alice"|"passwd 1000") echo "alice:x:1000:1000:Alice:${STUB_HOME}:/bin/bash" ;;
    "passwd root"|"passwd 0") echo "root:x:0:0:root:/root:/bin/bash" ;;
    *) exit 2 ;;
esac
"#;

/// Answers per session id: the user's desk session (`STUB_DISPLAY_SESSION`, default `c2`) is
/// described by `STUB_DESK_REMOTE` / `STUB_DESK_SEAT` (default local, `seat0`); any other id
/// (the shell's own session, `self` or `XDG_SESSION_ID`) by `STUB_REMOTE` / `STUB_SEAT`.
/// `STUB_SELF_FAIL=1` makes the own session unknown to logind (the call fails).
const STUB_LOGINCTL: &str = r#"
[[ "${STUB_LOGINCTL_FAIL:-0}" == 1 ]] && exit 1
desk="${STUB_DISPLAY_SESSION:-c2}"
case "${1:-}" in
    show-user) echo "Display=${desk}"; exit 0 ;;
    show-session)
        if [[ "${2:-}" == "${desk}" ]]; then
            echo "Remote=${STUB_DESK_REMOTE:-no}"
            echo "Seat=${STUB_DESK_SEAT-seat0}"
            exit 0
        fi
        [[ "${STUB_SELF_FAIL:-0}" == 1 ]] && exit 1
        echo "Remote=${STUB_REMOTE:-no}"
        echo "Seat=${STUB_SEAT-seat0}" ;;
    *) exit 1 ;;
esac
"#;

const STUB_SYSTEMCTL: &str = r#"
case "${1:-}" in
    is-enabled) [[ -e "${STUB_STATE}/unit_enabled" ]] ;;
    is-active) [[ -e "${STUB_STATE}/unit_active" ]] ;;
    stop) rm -f "${STUB_STATE}/unit_active" ;;
    start) [[ "${STUB_START_RC:-0}" == 0 ]] || exit "${STUB_START_RC}"; touch "${STUB_STATE}/unit_active" ;;
    *) exit 0 ;;
esac
"#;

const STUB_PACMAN: &str = r#"
case "${1:-}" in
    -Q)
        shift
        rc=0
        for p in "$@"; do
            if [[ -e "${STUB_STATE}/pkg_${p}" ]]; then echo "${p} 1.0-1"; else echo "error: package '${p}' was not found" >&2; rc=1; fi
        done
        exit "${rc}" ;;
    -S)
        [[ "${STUB_PKG_RC:-0}" == 0 ]] || exit "${STUB_PKG_RC}"
        for p in "$@"; do [[ "$p" == -* ]] || touch "${STUB_STATE}/pkg_${p}"; done ;;
    *) exit 1 ;;
esac
"#;

const STUB_DPKG_QUERY: &str = r#"
p="${!#}"
if [[ -e "${STUB_STATE}/pkg_${p}" ]]; then printf 'installed'; else echo "dpkg-query: no packages found matching ${p}" >&2; exit 1; fi
"#;

const STUB_APT_GET: &str = r#"
[[ "${STUB_PKG_RC:-0}" == 0 ]] || exit "${STUB_PKG_RC}"
for p in "$@"; do [[ "$p" == -* || "$p" == install ]] || touch "${STUB_STATE}/pkg_${p}"; done
"#;

const STUB_RPM: &str = r#"
p="${!#}"
[[ -e "${STUB_STATE}/pkg_${p}" ]]
"#;

const STUB_DNF: &str = r#"
[[ "${STUB_PKG_RC:-0}" == 0 ]] || exit "${STUB_PKG_RC}"
for p in "$@"; do [[ "$p" == -* || "$p" == install ]] || touch "${STUB_STATE}/pkg_${p}"; done
"#;

const STUB_CARGO: &str = r#"
printf 'cargo-env SOOS_BINDIR=%s CARGO_TARGET_DIR=%s PWD=%s HOME=%s USER=%s CARGO_HOME=%s RUSTUP_HOME=%s RUSTFLAGS=%s\n' \
    "${SOOS_BINDIR:-}" "${CARGO_TARGET_DIR:-}" "$(pwd)" "${HOME:-}" "${USER:-}" "${CARGO_HOME:-}" \
    "${RUSTUP_HOME:-}" "${RUSTFLAGS:-}" >> "${STUB_LOG}"
[[ "${STUB_CARGO_RC:-0}" == 0 ]] || exit "${STUB_CARGO_RC}"
out="${CARGO_TARGET_DIR:?}/release"
mkdir -p "${out}"
printf 'daemon\n' > "${out}/soos-daemon"
cp "${STUB_FAKES}/soos-admin" "${out}/soos-admin"
cp "${STUB_FAKES}/soos-enroll" "${out}/soos-enroll"
printf 'gui\n' > "${out}/soos-gui"
printf 'pam\n' > "${out}/libpam_soos.so"
chmod 0755 "${out}/soos-daemon" "${out}/soos-admin" "${out}/soos-enroll" "${out}/soos-gui"
"#;

const STUB_GPASSWD: &str = r#"
rm -f "${STUB_STATE}/in_group"
"#;

const STUB_LOG_ONLY: &str = "\nexit 0\n";

const STUB_TAILSCALE: &str = r#"
[[ "${1:-}" == version ]] && printf '1.102.4\n  tailscale commit: stub\n'
exit 0
"#;

const FAKE_SOOS_ADMIN: &str = r#"
case "${1:-}" in
    add-user)
        [[ "${STUB_ADDUSER_RC:-0}" == 0 ]] || exit "${STUB_ADDUSER_RC}"
        touch "${STUB_STATE}/in_group" ;;
    camera) echo "selected: /dev/video0 (stub)"; exit "${STUB_CAMERA_RC:-0}" ;;
    *) exit 0 ;;
esac
"#;

/// Like the real CLI, enrollment fails with "Camera is busy" while the daemon runs.
const FAKE_SOOS_ENROLL: &str = r#"
if [[ -e "${STUB_STATE}/unit_active" ]]; then
    echo "[ERROR] Camera is busy: another process holds the camera device" >&2
    exit 1
fi
[[ "${STUB_ENROLL_RC:-0}" == 0 ]] || exit "${STUB_ENROLL_RC}"
dir=""
while [[ $# -gt 0 ]]; do
    case "$1" in --biometrics-dir) dir="$2"; shift 2 ;; *) shift ;; esac
done
[[ -n "${dir}" ]] || exit 64
mkdir -p "${dir}"
touch "${dir}/1000.cbor.enc"
"#;

const STUB_INSTALL_SH: &str = r#"
[[ "${STUB_INSTALL_RC:-0}" == 0 || "${STUB_INSTALL_RC}" == 70 ]] || exit "${STUB_INSTALL_RC}"
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
touch "${STUB_STATE}/unit_enabled" "${STUB_STATE}/unit_active"
exit "${STUB_INSTALL_RC:-0}"
"#;

const STUB_RUSTUP_SH: &str = r#"
mkdir -p "${HOME}/.cargo/bin"
cp "${STUB_FAKES}/cargo" "${HOME}/.cargo/bin/cargo"
chmod 0755 "${HOME}/.cargo/bin/cargo"
"#;

const STUB_READY_SH: &str = r#"
exit "${STUB_READY_RC:-0}"
"#;

const STUB_MODELS_SH: &str = r#"
[[ " $* " == *" --check-only "* ]] || exit 64
exit "${STUB_MODELS_RC:-0}"
"#;

const ARCH_OS_RELEASE: &str =
    "NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\nBUILD_ID=rolling\n";

/// Arch packages of the three groups (copied from check_build_deps.sh by the harness at run
/// time, never hard-coded here except the two the tests leave missing).
const MISSING_ARCH_PACKAGES: [&str; 2] = ["clang", "libxi"];

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

/// A regular file with an execute bit (a PATH directory may hold a plain `env` file).
fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn bash_path() -> PathBuf {
    let path_var = std::env::var_os("PATH").expect("PATH");
    std::env::split_paths(&path_var)
        .map(|d| d.join("bash"))
        .find(|p| is_executable(p))
        .expect("bash on PATH")
}

fn write_exec(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, body).expect("write stub");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
}

impl Harness {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("soos_setup_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch");
        let repo = dir.join("repo");
        let stubs = dir.join("stubs");
        let tools = dir.join("tools");
        let fakes = dir.join("fakes");
        let root = dir.join("root");
        let home = dir.join("home/alice");
        let state = dir.join("stub_state");
        let log = dir.join("stub.log");
        for d in [&repo, &stubs, &tools, &fakes, &root, &home, &state] {
            fs::create_dir_all(d).expect("mkdir");
        }
        fs::create_dir_all(dir.join("tmp")).expect("tmp");
        fs::write(&log, "").expect("log");

        // Fake checkout: the real setup.sh and check_build_deps.sh, stubs for the rest.
        let real = workspace_root();
        for rel in [
            "scripts/setup.sh",
            "scripts/check_build_deps.sh",
            "scripts/provision_master_key.sh",
            "packaging/soos-daemon.service",
            "packaging/pam/arch/system-auth.snippet",
            "models/manifest.toml",
        ] {
            let dst = repo.join(rel);
            fs::create_dir_all(dst.parent().expect("parent")).expect("mkdir");
            if real.join(rel).exists() {
                fs::copy(real.join(rel), &dst).expect("copy repo file");
            }
        }
        let bash = bash_path();
        let shebang = format!("#!{}\n", bash.display());
        // Harness paths are baked into every stub, so a stub still logs when the script runs
        // it under `env -i` (the root build allow-list).
        let baked = format!(
            ": \"${{STUB_LOG:={}}}\" \"${{STUB_STATE:={}}}\" \"${{STUB_HOME:={}}}\" \"${{STUB_FAKES:={}}}\"\nexport STUB_LOG STUB_STATE STUB_HOME STUB_FAKES\n",
            log.display(),
            state.display(),
            home.display(),
            fakes.display()
        );
        let stub = |body: &str| format!("{shebang}{baked}{STUB_PROLOGUE}{body}");
        write_exec(&repo.join("scripts/install.sh"), &stub(STUB_INSTALL_SH));
        write_exec(
            &repo.join("scripts/install_rustup.sh"),
            &stub(STUB_RUSTUP_SH),
        );
        write_exec(
            &repo.join("scripts/wait_daemon_ready.sh"),
            &stub(STUB_READY_SH),
        );
        write_exec(
            &repo.join("scripts/download_models.sh"),
            &stub(STUB_MODELS_SH),
        );

        // Stub commands.
        for (name, body) in [
            ("sudo", STUB_SUDO),
            ("runuser", STUB_RUNUSER),
            ("loginctl", STUB_LOGINCTL),
            ("systemctl", STUB_SYSTEMCTL),
            ("pacman", STUB_PACMAN),
            ("dpkg-query", STUB_DPKG_QUERY),
            ("apt-get", STUB_APT_GET),
            ("rpm", STUB_RPM),
            ("dnf", STUB_DNF),
            ("cargo", STUB_CARGO),
            ("gpasswd", STUB_GPASSWD),
            ("deluser", STUB_GPASSWD),
            ("tailscale", STUB_TAILSCALE),
            ("pam-auth-update", STUB_LOG_ONLY),
            ("authselect", STUB_LOG_ONLY),
            ("usermod", STUB_LOG_ONLY),
            ("curl", STUB_LOG_ONLY),
        ] {
            write_exec(&stubs.join(name), &stub(body));
        }
        // id and getent are queried often: no log line.
        write_exec(&stubs.join("id"), &format!("{shebang}{STUB_ID}"));
        write_exec(&stubs.join("getent"), &format!("{shebang}{STUB_GETENT}"));
        write_exec(&fakes.join("soos-admin"), &stub(FAKE_SOOS_ADMIN));
        write_exec(&fakes.join("soos-enroll"), &stub(FAKE_SOOS_ENROLL));
        write_exec(&fakes.join("cargo"), &stub(STUB_CARGO));

        // Harmless real tools only.
        let path_var = std::env::var_os("PATH").expect("PATH");
        for tool in SAFE_TOOLS {
            if let Some(found) = std::env::split_paths(&path_var)
                .map(|d| d.join(tool))
                .find(|p| is_executable(p))
            {
                std::os::unix::fs::symlink(&found, tools.join(tool)).expect("symlink tool");
            }
        }

        // Fake root: systemd present, GDM display manager, Arch os-release.
        fs::create_dir_all(root.join("run/systemd/system")).expect("systemd dir");
        fs::create_dir_all(root.join("etc/systemd/system")).expect("unit dir");
        std::os::unix::fs::symlink(
            "/usr/lib/systemd/system/gdm.service",
            root.join("etc/systemd/system/display-manager.service"),
        )
        .expect("display-manager link");
        fs::create_dir_all(dir.join("roothome")).expect("root home");
        fs::write(root.join("etc/os-release"), ARCH_OS_RELEASE).expect("os-release");

        let path = format!("{}:{}", stubs.display(), tools.display());
        let env = vec![
            ("PATH".to_string(), path),
            ("HOME".to_string(), home.display().to_string()),
            ("LC_ALL".to_string(), "C".to_string()),
            ("TMPDIR".to_string(), dir.join("tmp").display().to_string()),
            ("STUB_LOG".to_string(), log.display().to_string()),
            ("STUB_STATE".to_string(), state.display().to_string()),
            ("STUB_HOME".to_string(), home.display().to_string()),
            ("STUB_FAKES".to_string(), fakes.display().to_string()),
            (
                "STUB_ROOT_HOME".to_string(),
                dir.join("roothome").display().to_string(),
            ),
            (
                "STUB_BIOMETRICS".to_string(),
                root.join("var/lib/soos/biometrics").display().to_string(),
            ),
        ];
        let h = Self {
            dir,
            repo,
            stubs,
            root,
            home,
            state,
            log,
            env,
        };
        h.mark_arch_packages_installed_except(&MISSING_ARCH_PACKAGES);
        h
    }

    fn set_env(&mut self, key: &str, value: &str) {
        self.env.retain(|(k, _)| k != key);
        self.env.push((key.to_string(), value.to_string()));
    }

    fn os_release(&self, content: &str) {
        fs::write(self.root.join("etc/os-release"), content).expect("os-release");
    }

    fn mark_arch_packages_installed_except(&self, missing: &[&str]) {
        for group in ["build", "models", "gui"] {
            for p in print_packages("arch", group) {
                if !missing.contains(&p.as_str()) {
                    fs::write(self.state.join(format!("pkg_{p}")), "").expect("pkg");
                }
            }
        }
    }

    fn fake_root_args(&self) -> Vec<String> {
        vec![
            "--prefix".into(),
            self.root.join("usr").display().to_string(),
            "--sysconfdir".into(),
            self.root.join("etc").display().to_string(),
            "--localstatedir".into(),
            self.root.join("var").display().to_string(),
            "--runstatedir".into(),
            self.root.join("run").display().to_string(),
            "--pam-dir".into(),
            self.root.join("usr/lib/security").display().to_string(),
        ]
    }

    fn run(&self, args: &[&str]) -> Output {
        fs::write(&self.log, "").expect("reset log");
        let mut cmd = Command::new(bash_path());
        cmd.arg(self.repo.join(SETUP))
            .args(self.fake_root_args())
            .args(args)
            .current_dir(&self.dir)
            .env_clear()
            .stdin(Stdio::null());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd.output().expect("run setup.sh")
    }

    fn log_lines(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .expect("read log")
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn state_file(&self) -> PathBuf {
        self.root.join("var/lib/soos/state/setup.json")
    }

    /// Whole-tree snapshot (type, mode, content) of the fake root, the fake home, the checkout
    /// and the temporary directory.
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

fn print_packages(distro: &str, group: &str) -> Vec<String> {
    let out = Command::new("bash")
        .arg(workspace_root().join("scripts/check_build_deps.sh"))
        .args(["--distro", distro, "--print-packages", group])
        .output()
        .expect("run check_build_deps.sh");
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect()
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

/// Index of the first log line containing every needle, or panics with the log.
fn find_line(lines: &[String], needles: &[&str]) -> usize {
    lines
        .iter()
        .position(|l| needles.iter().all(|n| l.contains(n)))
        .unwrap_or_else(|| panic!("no log line with {needles:?} in:\n{}", lines.join("\n")))
}

fn has_line(lines: &[String], needles: &[&str]) -> bool {
    lines.iter().any(|l| needles.iter().all(|n| l.contains(n)))
}

/// Log lines a run that must change nothing may produce (read-only queries).
fn is_read_only_call(line: &str) -> bool {
    let cmd = line.split_whitespace().nth(1).unwrap_or("");
    let arg = line.split_whitespace().nth(2).unwrap_or("");
    match cmd {
        "loginctl" => arg == "show-session" || arg == "show-user",
        "pacman" => arg == "-Q",
        "dpkg-query" | "rpm" => true,
        "systemctl" => arg == "is-enabled" || arg == "is-active",
        "tailscale" => arg == "version",
        "download_models.sh" => line.contains(" --check-only"),
        _ => false,
    }
}

fn assert_no_forbidden_calls(lines: &[String]) {
    for line in lines {
        let mut words = line.split_whitespace().skip(1);
        if words.next() == Some("tailscale") {
            assert_eq!(
                words.next(),
                Some("version"),
                "only `tailscale version`: {line}"
            );
        }
    }
    for forbidden in ["pam-auth-update", "authselect", "curl", "usermod"] {
        assert!(
            !lines
                .iter()
                .any(|l| l.split_whitespace().nth(1) == Some(forbidden)),
            "setup.sh must never run {forbidden}: {lines:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Behavioral contracts
// ---------------------------------------------------------------------------

/// SET5: `--help` exits 0 and documents every option the parser accepts.
#[test]
fn test_set_help_documents_every_parsed_option() {
    let h = Harness::new("help");
    let out = h.run(&["--help"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let help = text(&out);
    let script = read_repo(SETUP);
    let mut options = Vec::new();
    for line in script.lines() {
        let t = line.trim();
        // Case patterns only: `-y|--yes)`, `--dry-run)`, ...
        if !t.ends_with(')') || !t.starts_with('-') || t.contains(' ') {
            continue;
        }
        for opt in t.trim_end_matches(')').split('|') {
            if opt.starts_with("--") {
                options.push(opt.to_string());
            }
        }
    }
    for expected in [
        "--yes",
        "--dry-run",
        "--user",
        "--no-enroll",
        "--presence",
        "--skip-packages",
        "--distro",
        "--prefix",
        "--sysconfdir",
        "--localstatedir",
        "--runstatedir",
        "--pam-dir",
        "--help",
        "--reinstall",
        "--system-phase",
        "--artifact-dir",
        "--local-seat",
    ] {
        assert!(
            options.iter().any(|o| o == expected),
            "parser must accept {expected}"
        );
    }
    for opt in &options {
        assert!(help.contains(opt.as_str()), "--help must document {opt}");
    }
    for code in ["0 ", "1 ", "2 ", "3 ", "4 ", "40 ", "70 ", "130 ", "143 "] {
        assert!(
            help.contains(&format!("  {code}")),
            "--help must list exit code {code}"
        );
    }
    assert!(h.log_lines().is_empty(), "--help must run nothing");
}

/// SET6: invalid arguments exit 2 before anything runs.
#[test]
fn test_set_invalid_arguments_exit_2_without_change() {
    let h = Harness::new("badargs");
    let before = h.snapshot();
    for args in [
        vec!["--bogus"],
        vec!["--user"],
        vec!["--presence", "maybe"],
        vec!["--user", "Bad;Name"],
        vec!["--user", "root"],
        vec!["--user", "a234567890123456789012345678901234"],
        vec!["--distro", "gentoo"],
        vec!["--prefix", "relative/path"],
        vec!["--system-phase", "--user", "alice"],
        vec!["--local-seat", "yes"],
        vec!["--artifact-dir", "/tmp"],
    ] {
        let mut full = args.clone();
        full.push("--yes");
        let out = h.run(&full);
        assert_eq!(code(&out), 2, "{args:?} must exit 2: {}", text(&out));
        let lines = h.log_lines();
        assert!(
            lines.iter().all(|l| is_read_only_call(l)),
            "{args:?} must change nothing: {lines:?}"
        );
    }
    assert_eq!(before, h.snapshot(), "invalid arguments must write nothing");
}

/// SET7: `--dry-run` prints the detection and the full plan and changes nothing, as a user
/// and as root (no escalation, no build, no package install, no temp file).
#[test]
fn test_set_dry_run_changes_nothing_and_prints_plan() {
    let mut h = Harness::new("dryrun");
    let before = h.snapshot();
    let out = h.run(&["--dry-run"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    for needle in [
        "[DETECT] distribution: Arch Linux (family: arch)",
        "[DETECT] package manager: pacman",
        "[DETECT] init: systemd",
        "[DETECT] session: local seat",
        "[DETECT] display manager: gdm",
        "[DETECT] tailscale: 1.102.4",
        "[DETECT] camera:",
        "[DETECT] user: alice",
        "[PLAN] packages: will run",
        "pacman -S --needed --noconfirm clang libxi",
        "[PLAN] toolchain: already done",
        "[PLAN] build: will run",
        "[PLAN] system: will run",
        "[PLAN] access: will run",
        "[PLAN] presence: already done",
        "[PLAN] verify: will run",
        "[PLAN] enrollment: will run",
        "Dry run",
    ] {
        assert!(
            t.contains(needle),
            "dry-run output must contain {needle}:\n{t}"
        );
    }
    let lines = h.log_lines();
    assert!(
        lines.iter().all(|l| is_read_only_call(l)),
        "dry-run must only query: {lines:?}"
    );
    assert_eq!(before, h.snapshot(), "dry-run must write nothing");

    h.set_env("STUB_AS_ROOT", "1");
    h.set_env("SUDO_USER", "alice");
    let out = h.run(&["--dry-run"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(text(&out).contains("[DETECT] user: alice"));
    let lines = h.log_lines();
    assert!(
        lines.iter().all(|l| is_read_only_call(l)),
        "dry-run as root must only query: {lines:?}"
    );
    assert_eq!(before, h.snapshot(), "dry-run as root must write nothing");
}

/// SET8: os-release is parsed (never executed) and maps to the package family.
#[test]
fn test_set_detection_parses_os_release_without_sourcing() {
    let h = Harness::new("osrel");
    let pwned = h.dir.join("pwned");
    h.os_release(&format!(
        "PRETTY_NAME=\"$(touch {p})Evil\"\nID=arch\ntouch {p}\nID_LIKE=`touch {p}`\n",
        p = pwned.display()
    ));
    let out = h.run(&["--dry-run"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(!pwned.exists(), "os-release content must never be executed");
    assert!(text(&out).contains("(family: arch)"));

    for (content, manager) in [
        (
            "PRETTY_NAME=\"Ubuntu 24.04 LTS\"\nID=ubuntu\nID_LIKE=debian\n",
            "apt-get",
        ),
        (
            "PRETTY_NAME=\"Rocky Linux 9\"\nID=\"rocky\"\nID_LIKE=\"rhel centos fedora\"\n",
            "dnf",
        ),
        (
            "PRETTY_NAME=\"EndeavourOS\"\nID=endeavouros\nID_LIKE=arch\n",
            "pacman",
        ),
    ] {
        h.os_release(content);
        let out = h.run(&["--dry-run"]);
        assert_eq!(code(&out), 0, "{}", text(&out));
        assert!(
            text(&out).contains(&format!("[DETECT] package manager: {manager}")),
            "{content} must map to {manager}:\n{}",
            text(&out)
        );
    }
}

/// SET8: an unsupported distribution or a host without systemd is refused before any change.
#[test]
fn test_set_unsupported_host_refused_before_change() {
    let h = Harness::new("unsupported");
    h.os_release("PRETTY_NAME=\"Gentoo\"\nID=gentoo\n");
    let before = h.snapshot();
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert!(text(&out).contains("--distro"), "must hint --distro");
    assert!(h.log_lines().iter().all(|l| is_read_only_call(l)));
    assert_eq!(before, h.snapshot());

    h.os_release(ARCH_OS_RELEASE);
    fs::remove_dir_all(h.root.join("run/systemd")).expect("rm systemd");
    let before = h.snapshot();
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert!(text(&out).contains("systemd"));
    assert!(h.log_lines().iter().all(|l| is_read_only_call(l)));
    assert_eq!(before, h.snapshot());
}

/// SET9: without a TTY, `--yes` is required; as root, an invoking user is required; a normal
/// user needs sudo or doas. All refused with exit 2 before any change.
#[test]
fn test_set_preconditions_refused_before_change() {
    let mut h = Harness::new("precond");
    let before = h.snapshot();
    let out = h.run(&[]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert!(text(&out).contains("--yes"));
    assert!(h.log_lines().iter().all(|l| is_read_only_call(l)));

    h.set_env("STUB_AS_ROOT", "1");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 2, "root without invoking user: {}", text(&out));
    assert!(text(&out).contains("--user"));
    assert!(h.log_lines().iter().all(|l| is_read_only_call(l)));

    h.set_env("STUB_AS_ROOT", "0");
    fs::remove_file(h.stubs.join("sudo")).expect("rm sudo stub");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 2, "no sudo/doas: {}", text(&out));
    assert!(text(&out).contains("sudo"));
    assert!(h.log_lines().iter().all(|l| is_read_only_call(l)));
    assert_eq!(before, h.snapshot());
}

/// SET10: a full run as the session owner installs the face profile in order: missing packages
/// only, build as the user (never root), one escalation, install.sh --start, add-user, verify
/// gate, camera report, local-seat enrollment, state file; nothing touches PAM or Tailscale.
#[test]
fn test_set_full_run_as_user_installs_face_profile() {
    let h = Harness::new("full");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let lines = h.log_lines();
    assert_no_forbidden_calls(&lines);

    let pkg = find_line(&lines, &["uid=0 pacman -S"]);
    let pkg_line = &lines[pkg];
    let installed: Vec<&str> = pkg_line
        .split_whitespace()
        .skip(2)
        .filter(|t| !t.starts_with('-'))
        .collect();
    assert_eq!(installed, MISSING_ARCH_PACKAGES, "only missing packages");
    assert!(pkg_line.contains("--needed") && pkg_line.contains("--noconfirm"));

    let build = find_line(
        &lines,
        &["uid=1000 cargo build --release --locked --workspace"],
    );
    assert!(
        !has_line(&lines, &["uid=0 cargo"]),
        "the build must never run as root"
    );
    let env_line = &lines[find_line(&lines, &["cargo-env"])];
    let usr = h.root.join("usr");
    assert!(
        env_line.contains(&format!("SOOS_BINDIR={}/bin", usr.display())),
        "{env_line}"
    );
    assert!(
        env_line.contains(&format!(
            "CARGO_TARGET_DIR={}",
            h.repo.join("target").display()
        )),
        "{env_line}"
    );

    let escalate = find_line(&lines, &["uid=1000 sudo", "--system-phase"]);
    assert_eq!(
        lines.iter().filter(|l| l.contains("uid=1000 sudo")).count(),
        2,
        "exactly two escalations (packages, system phase): {lines:?}"
    );
    let install = find_line(&lines, &["uid=0 install.sh", "--start"]);
    assert!(lines[install].contains(&format!(
        "--artifact-dir {}",
        h.repo.join("target/release").display()
    )));
    let add_user = find_line(&lines, &["uid=0 soos-admin add-user alice"]);
    let ready = find_line(&lines, &["uid=0 wait_daemon_ready.sh", "--timeout 30"]);
    let camera = find_line(&lines, &["uid=0 soos-admin camera list"]);
    let stop = find_line(&lines, &["uid=0 systemctl stop soos-daemon.service"]);
    let enroll = find_line(&lines, &["uid=0 soos-enroll", "enroll --username alice"]);
    let bio = h.root.join("var/lib/soos/biometrics");
    assert!(
        lines[enroll].contains(&format!("--biometrics-dir {}", bio.display()))
            && lines[enroll].contains("--key-file")
            && lines[enroll].contains("--models-dir"),
        "enrollment paths derive from --localstatedir: {}",
        lines[enroll]
    );
    let start = find_line(&lines, &["uid=0 systemctl start soos-daemon.service"]);
    let ready_again = lines
        .iter()
        .rposition(|l| l.contains("uid=0 wait_daemon_ready.sh"))
        .expect("readiness after enrollment");
    assert!(pkg < build && build < escalate && escalate < install);
    assert!(install < add_user && add_user < ready && ready < camera && camera < stop);
    assert!(stop < enroll && enroll < start && start < ready_again);
    assert!(
        lines[install].contains("--distro arch"),
        "{}",
        lines[install]
    );
    assert!(
        lines[escalate].contains("--local-seat yes"),
        "seat verdict passed as an argument: {}",
        lines[escalate]
    );
    assert!(
        h.state.join("unit_active").exists(),
        "the daemon runs again after enrollment"
    );

    let state = h.state_file();
    let meta = fs::symlink_metadata(&state).expect("state file written");
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    let dir_mode = fs::metadata(state.parent().unwrap())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(dir_mode & 0o777, 0o700);
    let content = fs::read_to_string(&state).unwrap();
    assert_eq!(
        content,
        "{\n  \"format\": 1,\n  \"tool\": \"soos-setup\",\n  \"profile\": \"face\",\n  \"user\": \"alice\",\n  \"group_added_by_setup\": true,\n  \"presence_flag_created_by_setup\": false,\n  \"presence\": \"on\",\n  \"enrolled\": true\n}\n"
    );
    let t = text(&out);
    assert!(t.contains("log out and back in"), "re-login notice:\n{t}");
    assert!(t.contains("/etc/soos/disabled"), "rescue command:\n{t}");
    assert!(
        t.contains("uninstall.sh --keep-data"),
        "uninstall command:\n{t}"
    );
}

/// SET11: a second run with the same flags changes nothing and reports every phase done.
#[test]
fn test_set_second_run_is_idempotent() {
    let h = Harness::new("idem");
    let first = h.run(&["--yes"]);
    assert_eq!(code(&first), 0, "{}", text(&first));
    let before = h.snapshot();
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let lines = h.log_lines();
    for absent in [
        &["pacman -S"][..],
        &["install.sh"],
        &["add-user"],
        &["soos-enroll"],
        &["gpasswd"],
    ] {
        assert!(
            !has_line(&lines, absent),
            "second run must not run {absent:?}: {lines:?}"
        );
    }
    let t = text(&out);
    for phase in [
        "packages",
        "toolchain",
        "system",
        "access",
        "presence",
        "enrollment",
    ] {
        assert!(
            t.contains(&format!("[PLAN] {phase}: already done")),
            "{phase} must be already done:\n{t}"
        );
    }
    assert_eq!(before, h.snapshot(), "second run must change nothing");

    let dry = h.run(&["--dry-run"]);
    assert_eq!(code(&dry), 0);
    assert!(text(&dry).contains("[PLAN] system: already done"));
}

/// SET12: started as root through sudo, setup builds as the invoking user (runuser), never as
/// root, and runs the system phase inline without another escalation.
#[test]
fn test_set_full_run_as_root_builds_as_invoking_user() {
    let mut h = Harness::new("asroot");
    h.set_env("STUB_AS_ROOT", "1");
    h.set_env("SUDO_USER", "alice");
    // Root's toolchain environment must not reach the user build (env -i allow-list).
    h.set_env("CARGO_HOME", "/root/.cargo");
    h.set_env("RUSTUP_HOME", "/root/.rustup");
    h.set_env("RUSTFLAGS", "-Cinjected-by-root");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let lines = h.log_lines();
    find_line(&lines, &["uid=0 runuser -u alice"]);
    find_line(
        &lines,
        &["uid=1000 cargo build --release --locked --workspace"],
    );
    assert!(!has_line(&lines, &["uid=0 cargo"]));
    let env_line = &lines[find_line(&lines, &["cargo-env"])];
    assert!(
        env_line.contains(" CARGO_HOME= RUSTUP_HOME= RUSTFLAGS=")
            && env_line.ends_with("RUSTFLAGS="),
        "root's toolchain variables must not leak into the user build: {env_line}"
    );
    assert!(
        env_line.contains(&format!("HOME={} USER=alice", h.home.display())),
        "the build runs with the build user's HOME and USER: {env_line}"
    );
    assert!(!has_line(&lines, &["sudo"]), "no escalation as root");
    find_line(&lines, &["uid=0 install.sh", "--start"]);
    find_line(&lines, &["uid=0 soos-enroll", "enroll --username alice"]);
}

/// SET13: enrollment only from a local seat; otherwise skipped with the exact command.
#[test]
fn test_set_remote_session_skips_enrollment_with_command() {
    for (key, value) in [
        ("STUB_REMOTE", "yes"),
        ("STUB_SEAT", ""),
        ("STUB_LOGINCTL_FAIL", "1"),
        // Set for the user half only: the sudo stub resets the environment, so only the
        // `--local-seat no` argument can carry the verdict to the root half.
        ("SSH_CONNECTION", "192.0.2.1 50000 192.0.2.2 22"),
    ] {
        let mut h = Harness::new(&format!("remote_{}", key.to_lowercase()));
        h.set_env(key, value);
        let out = h.run(&["--yes"]);
        assert_eq!(code(&out), 0, "{key}={value}: {}", text(&out));
        let t = text(&out);
        assert!(
            t.contains("[DETECT] session: remote or no local seat"),
            "{key}: {t}"
        );
        assert!(
            t.contains("sudo soos-enroll enroll --username alice"),
            "{key}: exact command must be printed:\n{t}"
        );
        assert!(
            !has_line(&h.log_lines(), &["soos-enroll"]),
            "{key}: enrollment must not run"
        );
        let state = fs::read_to_string(h.state_file()).expect("state");
        assert!(state.contains("\"enrolled\": false"));
    }

    let h = Harness::new("noenroll");
    let out = h.run(&["--yes", "--no-enroll"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(text(&out).contains("sudo soos-enroll enroll --username alice"));
    assert!(!has_line(&h.log_lines(), &["soos-enroll"]));
}

/// SET14: a missing toolchain is installed only through the verified install_rustup.sh, as
/// the user, and the build then uses ~/.cargo/bin/cargo.
#[test]
fn test_set_missing_toolchain_uses_verified_rustup() {
    let h = Harness::new("rustup");
    fs::remove_file(h.stubs.join("cargo")).expect("rm cargo stub");
    let dry = h.run(&["--dry-run"]);
    assert!(text(&dry).contains("[PLAN] toolchain: will run"));
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let lines = h.log_lines();
    let rustup = find_line(&lines, &["uid=1000 install_rustup.sh"]);
    let build = find_line(&lines, &["uid=1000 cargo build"]);
    assert!(rustup < build);
    assert!(h.home.join(".cargo/bin/cargo").is_file());
}

/// SET15: failures before the commit point: a build failure exits 40 with nothing installed;
/// an install.sh failure exits 1 and stops before access; exit 70 is kept, nothing rolled back.
#[test]
fn test_set_build_and_install_failures_stop_cleanly() {
    let mut h = Harness::new("buildfail");
    h.set_env("STUB_CARGO_RC", "101");
    let root_before = {
        let mut m = BTreeMap::new();
        snapshot_into(&h.root, &h.root, &mut m);
        m
    };
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 40, "{}", text(&out));
    let lines = h.log_lines();
    assert!(!has_line(&lines, &["--system-phase"]));
    assert!(!has_line(&lines, &["install.sh"]));
    let mut root_after = BTreeMap::new();
    snapshot_into(&h.root, &h.root, &mut root_after);
    assert_eq!(
        root_before, root_after,
        "nothing installed after a build failure"
    );

    let mut h = Harness::new("installfail");
    h.set_env("STUB_INSTALL_RC", "60");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 1, "{}", text(&out));
    assert!(text(&out).contains("install.sh"));
    let lines = h.log_lines();
    for absent in ["add-user", "soos-enroll", "wait_daemon_ready", "gpasswd"] {
        assert!(
            !has_line(&lines, &[absent]),
            "{absent} after install failure"
        );
    }
    assert!(!h.state_file().exists());

    let mut h = Harness::new("notready");
    h.set_env("STUB_INSTALL_RC", "70");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 70, "{}", text(&out));
    let lines = h.log_lines();
    assert!(!has_line(&lines, &["soos-enroll"]));
    assert!(!has_line(&lines, &["gpasswd"]));
    assert!(
        h.root.join("usr/bin/soos-admin").exists(),
        "a committed install is kept"
    );
}

/// SET16: a runtime failure after access rolls back setup's own changes of this run.
#[test]
fn test_set_runtime_failure_rolls_back_setup_changes() {
    let h = Harness::new("rollback");
    // /etc/soos is a regular file: the presence flag cannot be written.
    fs::write(h.root.join("etc/soos"), "not a directory").expect("blocker");
    let out = h.run(&["--yes", "--presence", "off"]);
    assert_eq!(code(&out), 1, "{}", text(&out));
    let lines = h.log_lines();
    let add = find_line(&lines, &["uid=0 soos-admin add-user alice"]);
    let undo = find_line(&lines, &["uid=0 gpasswd -d alice soos"]);
    assert!(add < undo);
    assert!(!has_line(&lines, &["soos-enroll"]));
    assert!(!h.state_file().exists(), "no state after a rollback");
    let t = text(&out);
    assert!(t.contains("rolled back"), "{t}");
    assert!(t.contains("uninstall.sh --keep-data"), "{t}");
}

/// SET16: an enrollment failure keeps the install (exit 3, retry command), a verify gate
/// failure exits 70 without enrollment, a missing camera skips enrollment (exit 0).
#[test]
fn test_set_late_failures_keep_the_install() {
    let mut h = Harness::new("enrollfail");
    h.set_env("STUB_ENROLL_RC", "1");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 3, "{}", text(&out));
    let lines = h.log_lines();
    let stop = find_line(&lines, &["uid=0 systemctl stop soos-daemon.service"]);
    let start = find_line(&lines, &["uid=0 systemctl start soos-daemon.service"]);
    assert!(
        stop < start,
        "the daemon is restarted after a failed enrollment"
    );
    assert!(h.state.join("unit_active").exists());
    assert!(text(&out).contains("sudo soos-enroll enroll --username alice"));
    assert!(!has_line(&h.log_lines(), &["gpasswd"]));
    let state = fs::read_to_string(h.state_file()).expect("state");
    assert!(
        state.contains("\"enrolled\": false") && state.contains("\"group_added_by_setup\": true")
    );

    let mut h = Harness::new("verifyfail");
    h.set_env("STUB_READY_RC", "1");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 70, "{}", text(&out));
    let lines = h.log_lines();
    assert!(!has_line(&lines, &["soos-enroll"]));
    assert!(!has_line(&lines, &["gpasswd"]));
    assert!(h.state_file().exists());

    let mut h = Harness::new("nocamera");
    h.set_env("STUB_CAMERA_RC", "1");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(text(&out).contains("no camera"));
    assert!(!has_line(&h.log_lines(), &["soos-enroll"]));
}

/// SET18: presence auto-unlock stays on by default; `--presence off|on` toggles the flag and is
/// idempotent; an existing flag is never removed without `--presence on`.
#[test]
fn test_set_presence_flag_toggles() {
    let h = Harness::new("presence");
    let flag = h.root.join("etc/soos/presence.disable");
    let out = h.run(&["--yes", "--presence", "off"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let meta = fs::symlink_metadata(&flag).expect("flag created");
    assert!(meta.is_file());
    assert_eq!(meta.permissions().mode() & 0o777, 0o644);
    let state = fs::read_to_string(h.state_file()).unwrap();
    assert!(state.contains("\"presence\": \"off\""));
    assert!(state.contains("\"presence_flag_created_by_setup\": true"));

    let before = h.snapshot();
    let out = h.run(&["--yes", "--presence", "off"]);
    assert_eq!(code(&out), 0);
    assert!(text(&out).contains("[PLAN] presence: already done"));
    assert_eq!(before, h.snapshot());

    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0);
    assert!(
        flag.exists(),
        "an existing flag is kept without --presence on"
    );
    assert!(text(&out).contains("--presence on"));

    let out = h.run(&["--yes", "--presence", "on"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(!flag.exists());
    let state = fs::read_to_string(h.state_file()).unwrap();
    assert!(state.contains("\"presence\": \"on\""));
    assert!(
        state.contains("\"presence_flag_created_by_setup\": true"),
        "sticky record"
    );
}

/// Writes an untrusted `setup.json` (state path, scratch directory).
type StatePrep = fn(&Path, &Path);

/// SET18: an untrusted state file (symlink, oversize, bad grammar, loose mode) is refused
/// before the system install.
#[test]
fn test_set_state_file_untrusted_content_refused() {
    let cases: [(&str, StatePrep); 5] = [
        ("symlink", |state, dir| {
            std::os::unix::fs::symlink(dir.join("elsewhere"), state).unwrap();
        }),
        ("oversize", |state, _| {
            fs::write(state, "x".repeat(4097)).unwrap();
            fs::set_permissions(state, fs::Permissions::from_mode(0o600)).unwrap();
        }),
        ("grammar", |state, _| {
            fs::write(state, "{\n  \"format\": 1,\n  \"user\": \"$(reboot)\"\n}\n").unwrap();
            fs::set_permissions(state, fs::Permissions::from_mode(0o600)).unwrap();
        }),
        ("mode", |state, _| {
            fs::write(state, "{\n  \"format\": 1\n}\n").unwrap();
            fs::set_permissions(state, fs::Permissions::from_mode(0o666)).unwrap();
        }),
        ("otheruser", |state, _| {
            fs::write(state, "{\n  \"format\": 1,\n  \"user\": \"bob\"\n}\n").unwrap();
            fs::set_permissions(state, fs::Permissions::from_mode(0o600)).unwrap();
        }),
    ];
    for (tag, prepare) in cases {
        let h = Harness::new(&format!("state_{tag}"));
        let dir = h.root.join("var/lib/soos/state");
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        prepare(&h.state_file(), &h.dir);
        // Through the user half: the root half refuses before any system change (exit 4).
        let out = h.run(&["--yes"]);
        assert_eq!(code(&out), 4, "{tag}: {}", text(&out));
        assert!(text(&out).contains("setup.json"), "{tag}: {}", text(&out));
        assert!(
            !has_line(&h.log_lines(), &["install.sh"]),
            "{tag}: refused before the system install"
        );
        // Started as root: refused in the preflight, before packages and build (exit 2).
        let mut h = h;
        h.set_env("STUB_AS_ROOT", "1");
        h.set_env("SUDO_USER", "alice");
        let out = h.run(&["--yes"]);
        assert_eq!(code(&out), 2, "{tag} as root: {}", text(&out));
        let lines = h.log_lines();
        assert!(
            lines.iter().all(|l| is_read_only_call(l)),
            "{tag} as root: nothing may change: {lines:?}"
        );
    }
}

/// SET10: a user already in the soos group is not added again and not recorded as added.
#[test]
fn test_set_user_in_group_skips_add_user() {
    let h = Harness::new("ingroup");
    fs::write(h.state.join("in_group"), "").unwrap();
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(!has_line(&h.log_lines(), &["add-user"]));
    let state = fs::read_to_string(h.state_file()).unwrap();
    assert!(state.contains("\"group_added_by_setup\": false"));
}

/// SET10: Debian family: apt-get installs only the missing packages, queried with dpkg-query.
#[test]
fn test_set_debian_uses_apt_for_missing_packages() {
    let h = Harness::new("debian");
    h.os_release("PRETTY_NAME=\"Debian GNU/Linux 12\"\nID=debian\n");
    for group in ["build", "models", "gui"] {
        for p in print_packages("debian", group) {
            if p != "libclang-dev" {
                fs::write(h.state.join(format!("pkg_{p}")), "").unwrap();
            }
        }
    }
    let out = h.run(&["--yes", "--no-enroll"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let lines = h.log_lines();
    let line = &lines[find_line(&lines, &["uid=0 apt-get install"])];
    assert!(line.contains("-y") && line.contains("--no-install-recommends"));
    let pkgs: Vec<&str> = line
        .split_whitespace()
        .skip(3)
        .filter(|t| !t.starts_with('-'))
        .collect();
    assert_eq!(pkgs, ["libclang-dev"]);
}

/// SET10: `--skip-packages` installs nothing and prints the command.
#[test]
fn test_set_skip_packages_prints_command_only() {
    let h = Harness::new("skippkg");
    let out = h.run(&["--yes", "--skip-packages", "--no-enroll"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(!has_line(&h.log_lines(), &["pacman -S"]));
    assert!(text(&out).contains("pacman -S --needed --noconfirm clang libxi"));
}

/// SET13: when `loginctl show-session self` fails in the user half (a terminal under the user
/// service manager), the user's graphical session decides; started as root there is no
/// such fallback.
#[test]
fn test_set_seat_falls_back_to_the_display_session_in_the_user_half() {
    let mut h = Harness::new("seatfallback");
    h.set_env("STUB_SELF_FAIL", "1");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(text(&out).contains("[DETECT] session: local seat"));
    let lines = h.log_lines();
    find_line(&lines, &["uid=1000 loginctl show-user alice"]);
    find_line(&lines, &["uid=0 soos-enroll", "enroll --username alice"]);

    let mut h = Harness::new("seatfallback_root");
    h.set_env("STUB_SELF_FAIL", "1");
    h.set_env("STUB_AS_ROOT", "1");
    h.set_env("SUDO_USER", "alice");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(text(&out).contains("[DETECT] session: remote or no local seat"));
    assert!(!has_line(&h.log_lines(), &["soos-enroll"]));
}

/// SET11: a damaged install is not "already done"; `--reinstall` forces the system phase; a
/// verify failure after a skipped system phase points to `--reinstall`.
#[test]
fn test_set_damaged_install_and_reinstall() {
    let mut h = Harness::new("damaged");
    let first = h.run(&["--yes"]);
    assert_eq!(code(&first), 0, "{}", text(&first));

    let out = h.run(&["--dry-run"]);
    assert!(text(&out).contains("[PLAN] system: already done"));
    h.set_env("STUB_MODELS_RC", "1");
    let out = h.run(&["--dry-run"]);
    assert!(
        text(&out).contains("[PLAN] system: will run"),
        "a failed model check means not installed:\n{}",
        text(&out)
    );
    h.set_env("STUB_MODELS_RC", "0");
    fs::remove_file(h.root.join("var/lib/soos/master.key")).unwrap();
    let out = h.run(&["--dry-run"]);
    assert!(text(&out).contains("[PLAN] system: will run"));
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    find_line(&h.log_lines(), &["uid=0 install.sh", "--start"]);

    let out = h.run(&["--yes", "--reinstall"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    find_line(&h.log_lines(), &["uid=0 install.sh", "--start"]);

    h.set_env("STUB_READY_RC", "1");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 70, "{}", text(&out));
    assert!(!has_line(&h.log_lines(), &["install.sh"]));
    assert!(text(&out).contains("--reinstall"), "{}", text(&out));
}

/// SET13 (candid review of #372, finding 1): a shell session that logind positively reports
/// as remote (or seatless) is never replaced by the user's desk session, e.g. an operator
/// over SSH running `su - alice` while Alice is logged in at the desk.
#[test]
fn test_set_remote_own_session_never_borrows_the_desk_session() {
    for (key, value) in [("STUB_REMOTE", "yes"), ("STUB_SEAT", "")] {
        let mut h = Harness::new(&format!("borrow_{}", key.to_lowercase()));
        h.set_env("XDG_SESSION_ID", "c7");
        h.set_env(key, value);
        let out = h.run(&["--yes"]);
        assert_eq!(code(&out), 0, "{key}: {}", text(&out));
        let t = text(&out);
        assert!(
            t.contains("[DETECT] session: remote or no local seat"),
            "{key}: {t}"
        );
        let lines = h.log_lines();
        assert!(
            !has_line(&lines, &["loginctl show-user"]),
            "{key}: no desk-session fallback after a positive answer: {lines:?}"
        );
        assert!(!has_line(&lines, &["soos-enroll"]), "{key}: no enrollment");
        assert!(
            t.contains("sudo soos-enroll enroll --username alice"),
            "{key}: command printed:\n{t}"
        );
    }
}

/// SET13 (candid review of #372, finding 2): the stop marker of the enrollment hand-off is
/// cleared only after `systemctl start` succeeded, so a signal in between still restarts.
#[test]
fn test_set_daemon_stop_marker_cleared_only_after_restart() {
    let text = read_repo(SETUP);
    let start = text.find("phase_enrollment() {").expect("phase_enrollment");
    let body = &text[start..];
    let body = &body[..body.find("\n}\n").expect("end of phase_enrollment")];
    let restart = body
        .find("systemctl start \"${UNIT}\"")
        .expect("restart in phase_enrollment");
    let cleared: Vec<usize> = body
        .match_indices("DAEMON_STOPPED=false")
        .map(|(i, _)| i)
        .collect();
    assert!(!cleared.is_empty(), "the marker must be cleared");
    for pos in cleared {
        assert!(
            pos > restart,
            "DAEMON_STOPPED=false must come after the successful systemctl start"
        );
    }
}
