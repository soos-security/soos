//! One-command setup `scripts/setup.sh`, user phase: the phone companion profile (GitHub #400,
//! PR 2 of #362; ADR 2026-10-15 "Setup User Phase: Phone Companion Profile With Default-On
//! Features"; spec `AI/architect_spec_setup_user_phase.md`; rows SET20-SET39).
//!
//! Like `setup_contract.rs` (PR 1, not edited here), the behavioral contracts run the real
//! script without root against a fake root and a fake HOME, in a fake checkout whose helper
//! scripts are stubs, with a restricted `PATH`: stub commands first, then symlinks to a fixed
//! set of harmless tools. No real `sudo`, package manager, `systemctl`, `loginctl`, `cargo`,
//! `soos-remote` or `tailscale` is reachable. This file has its own harness so the PR 1
//! contract file stays byte-identical.

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
const INSTALL_REMOTE: &str = "scripts/install_remote.sh";
const LOGIN: &str = "owner.login@example.com";
const RP_ID: &str = "pc.tail1234.ts.net";
const REMOTE_MARKER: &str =
    "# Managed by scripts/setup.sh (soos-setup): re-running setup rewrites this file.";

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

/// True when byte offset `pos` of `line` is inside a quoted string (odd number of `"` or `'`
/// before it), the rule of the migrated SET3 contract.
fn quoted_at(line: &str, pos: usize) -> bool {
    let before = &line[..pos];
    before.matches('"').count() % 2 == 1 || before.matches('\'').count() % 2 == 1
}

// ---------------------------------------------------------------------------
// Static contracts
// ---------------------------------------------------------------------------

/// SET20: the user-phase constants, the commands setup may only print (never run), and the
/// prompts that never appear with `--yes`.
#[test]
fn test_set_u1_static_rules_of_the_user_phase() {
    let text = read_repo(SETUP);
    for constant in [
        "readonly EXIT_COMPANION=5",
        "readonly MIN_TAILSCALE_VERSION=\"1.102.4\"",
        "readonly MAX_REMOTE_CONFIG_BYTES=16384",
        "readonly MAX_DAEMON_TOML_SCAN_BYTES=65536",
        "readonly MAX_LOGIN_LEN=254",
        "readonly MAX_HOST_LEN=253",
        "readonly COMPANION_SETTLE_S=2",
    ] {
        assert!(
            text.lines().any(|l| l.trim() == constant),
            "setup.sh must declare `{constant}`"
        );
    }
    assert!(
        text.contains(REMOTE_MARKER),
        "setup.sh must write the managed remote.toml marker"
    );
    let code = code_of(SETUP);
    // Printed only, never executed: every occurrence sits inside a quoted message.
    for needle in [
        "usermod",
        "enable-linger",
        "enroll-code",
        "tailscale serve",
        "tailscale funnel",
    ] {
        for line in code.lines() {
            let mut from = 0;
            while let Some(found) = line[from..].find(needle) {
                let pos = from + found;
                assert!(
                    quoted_at(line, pos),
                    "`{needle}` may only be printed inside a quoted message: {line}"
                );
                from = pos + needle.len();
            }
        }
    }
    for forbidden in [
        "tailscale status",
        "tailscale up",
        "tailscale set",
        "--http",
    ] {
        assert!(
            !code.contains(forbidden),
            "setup.sh must never contain `{forbidden}`"
        );
    }
    // Prompts only in an interactive run (never with --yes or --dry-run).
    let prompts: Vec<&str> = code.lines().filter(|l| l.contains("read -r -p")).collect();
    assert!(
        prompts.len() >= 3,
        "plan confirmation, login and node name prompts"
    );
    assert!(
        code.contains("INTERACTIVE=true"),
        "the companion prompts are guarded by an INTERACTIVE decision"
    );
}

/// SET35 (static half): `install_remote.sh` copies the binaries from the cargo target directory
/// cargo really used (`CARGO_TARGET_DIR`, relative values against the checkout).
#[test]
fn test_set_u2_install_remote_honours_cargo_target_dir_statically() {
    let code = code_of(INSTALL_REMOTE);
    assert!(
        code.contains("CARGO_TARGET_DIR"),
        "install_remote.sh must resolve CARGO_TARGET_DIR"
    );
    assert!(
        !code.contains("\"${REPO_ROOT}/target/release/soos-remote\""),
        "install_remote.sh must not hard-code <repo>/target/release"
    );
}

// ---------------------------------------------------------------------------
// Behavioral harness
// ---------------------------------------------------------------------------

const SAFE_TOOLS: &[&str] = &[
    "bash", "env", "cat", "cp", "mv", "rm", "mkdir", "chmod", "sed", "grep", "head", "tail", "tr",
    "cut", "sort", "wc", "stat", "touch", "find", "mktemp", "dirname", "basename", "cmp",
    "install", "timeout", "ln", "readlink", "uname", "ls", "rmdir", "sleep", "tee", "date",
    "realpath", "printf", "true", "false", "od", "cksum",
];

/// Logs `uid=<0|1000> <name> <args>` (role label, see `setup_contract.rs`).
const STUB_PROLOGUE: &str = r#"
__uid=1000
[[ "${STUB_AS_ROOT:-0}" == 1 ]] && __uid=0
printf 'uid=%s %s\n' "${__uid}" "$(basename "$0") $*" >> "${STUB_LOG}"
"#;

/// `env_reset`: only PATH, LC_ALL, root's HOME, the STUB_* and sudo variables reach the command.
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

/// System units keep the PR 1 behaviour; `--user` units have their own state files
/// (`user_<unit>_enabled`, `user_<unit>_active`). `STUB_USER_START_RC` fails every user start.
/// `STUB_USER_DIE_UNIT` names a user unit that stops right after this run starts or restarts it
/// (`is-active` then answers 3); `STUB_SYSTEM_OFF_UNIT` names a system unit the owner disabled
/// and stopped (`is-active` 3, `is-enabled` 1), for the push sender socket (ADR 2026-10-23).
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
        if [[ "${u}" == "${STUB_USER_FAIL_UNIT:-}" ]]; then
            case "${verb}" in enable|start|restart) exit 1 ;; esac
        fi
        if [[ "${u}" == "${STUB_USER_DIE_UNIT:-}" ]]; then
            case "${verb}" in
                start|restart) touch "${STUB_STATE}/user_${u}_died" ;;
                enable) (( now )) && touch "${STUB_STATE}/user_${u}_died" ;;
            esac
        fi
        case "${verb}" in
            is-enabled) [[ -e "${STUB_STATE}/user_${u}_enabled" ]] || rc=1 ;;
            is-active)
                if [[ -e "${STUB_STATE}/user_${u}_died" ]]; then
                    rc=3
                else
                    [[ -e "${STUB_STATE}/user_${u}_active" ]] || rc=3
                fi ;;
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
if [[ -n "${STUB_SYSTEM_OFF_UNIT:-}" && " $* " == *" ${STUB_SYSTEM_OFF_UNIT} "* ]]; then
    case "${1:-}" in
        is-active) exit 3 ;;
        is-enabled) exit 1 ;;
    esac
fi
case "${1:-}" in
    is-enabled) [[ -e "${STUB_STATE}/unit_enabled" ]] ;;
    is-active) [[ -e "${STUB_STATE}/unit_active" ]] ;;
    stop) rm -f "${STUB_STATE}/unit_active" ;;
    start|restart) touch "${STUB_STATE}/unit_active" ;;
    *) exit 0 ;;
esac
"#;

const STUB_PACMAN: &str = r#"
case "${1:-}" in
    -Q)
        shift
        rc=0
        for p in "$@"; do
            if [[ -e "${STUB_STATE}/pkg_${p}" ]]; then echo "${p} 1.0-1"; else rc=1; fi
        done
        exit "${rc}" ;;
    -S) for p in "$@"; do [[ "$p" == -* ]] || touch "${STUB_STATE}/pkg_${p}"; done ;;
    *) exit 1 ;;
esac
"#;

/// The build writes the face binaries and the two companion binaries (fake programs copied from
/// the fakes directory) into `$CARGO_TARGET_DIR/release`.
const STUB_CARGO: &str = r#"
printf 'cargo-env CARGO_TARGET_DIR=%s\n' "${CARGO_TARGET_DIR:-}" >> "${STUB_LOG}"
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

const STUB_GPASSWD: &str = r#"
rm -f "${STUB_STATE}/in_group"
"#;

const STUB_LOG_ONLY: &str = "\nexit 0\n";

const STUB_TAILSCALE: &str = r#"
[[ "${1:-}" == version ]] && printf '%s\n  tailscale commit: stub\n' "${STUB_TS_VERSION:-1.102.4}"
exit 0
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

/// Fake `soos-remote`: `--config <path> passkeys list` validates the prerequisites the real
/// service enforces (exit 78) and prints no identity; `STUB_REMOTE_RC` forces a status.
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
echo "No passkey registered."
"#;

const FAKE_PUSH_SENDER: &str = "\nexit 0\n";

const STUB_INSTALL_SH: &str = r#"
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
"#;

/// Stub of `scripts/install_remote.sh` with the real contract: refuses root, installs the two
/// binaries from `$CARGO_TARGET_DIR/release` and the two units, writes the template only when
/// the configuration is absent, never enables a unit. `STUB_INSTALL_REMOTE_RC` fails it.
const STUB_INSTALL_REMOTE: &str = r#"
[[ "${STUB_AS_ROOT:-0}" == 1 ]] && exit 1
[[ "${STUB_INSTALL_REMOTE_RC:-0}" == 0 ]] || exit "${STUB_INSTALL_REMOTE_RC}"
printf 'install-remote-env CARGO_TARGET_DIR=%s HOME=%s\n' "${CARGO_TARGET_DIR:-}" "${HOME:-}" >> "${STUB_LOG}"
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

const STUB_RUSTUP_SH: &str = "\nexit 0\n";

/// `STUB_READY_FAIL_ONCE=1` fails only the first readiness check of a run.
const STUB_READY_SH: &str = r#"
if [[ "${STUB_READY_FAIL_ONCE:-0}" == 1 && ! -e "${STUB_STATE}/ready_failed_once" ]]; then
    touch "${STUB_STATE}/ready_failed_once"
    exit 1
fi
exit "${STUB_READY_RC:-0}"
"#;

const STUB_MODELS_SH: &str = r#"
[[ " $* " == *" --check-only "* ]] || exit 64
exit 0
"#;

const ARCH_OS_RELEASE: &str =
    "NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\nBUILD_ID=rolling\n";

struct Harness {
    dir: PathBuf,
    repo: PathBuf,
    root: PathBuf,
    home: PathBuf,
    runtime: PathBuf,
    state: PathBuf,
    log: PathBuf,
    env: Vec<(String, String)>,
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn real_uid() -> String {
    let out = Command::new("id").arg("-u").output().expect("run id -u");
    let uid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(
        !uid.is_empty() && uid.bytes().all(|b| b.is_ascii_digit()),
        "numeric uid expected: {uid:?}"
    );
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

/// Packages of the three groups, from the real `check_build_deps.sh`.
fn print_packages(group: &str) -> Vec<String> {
    let out = Command::new("bash")
        .arg(workspace_root().join("scripts/check_build_deps.sh"))
        .args(["--distro", "arch", "--print-packages", group])
        .output()
        .expect("run check_build_deps.sh");
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

impl Harness {
    fn new(tag: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("soos_setup_user_{tag}_{}", std::process::id()));
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
            "scripts/setup.sh",
            "scripts/check_build_deps.sh",
            "scripts/provision_master_key.sh",
            "packaging/soos-daemon.service",
            "packaging/soos-remote.service",
            "packaging/soos-push-sender.service",
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
            ": \"${{STUB_LOG:={}}}\" \"${{STUB_STATE:={}}}\" \"${{STUB_HOME:={}}}\" \"${{STUB_FAKES:={}}}\" \"${{STUB_UID:={uid}}}\"\nexport STUB_LOG STUB_STATE STUB_HOME STUB_FAKES STUB_UID\n",
            log.display(),
            state.display(),
            home.display(),
            fakes.display()
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
            ("gpasswd", STUB_GPASSWD),
            ("tailscale", STUB_TAILSCALE),
            ("usermod", STUB_LOG_ONLY),
            ("pam-auth-update", STUB_LOG_ONLY),
            ("authselect", STUB_LOG_ONLY),
            ("curl", STUB_LOG_ONLY),
        ] {
            write_exec(&stubs.join(name), &stub(body));
        }
        write_exec(&stubs.join("id"), &format!("{shebang}{baked}{STUB_ID}"));
        write_exec(
            &stubs.join("getent"),
            &format!("{shebang}{baked}{STUB_GETENT}"),
        );
        write_exec(&fakes.join("soos-admin"), &stub(FAKE_SOOS_ADMIN));
        write_exec(&fakes.join("soos-enroll"), &stub(FAKE_SOOS_ENROLL));
        write_exec(&fakes.join("soos-remote"), &stub(FAKE_SOOS_REMOTE));
        write_exec(&fakes.join("soos-push-sender"), &stub(FAKE_PUSH_SENDER));

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
        for group in ["build", "models", "gui"] {
            for p in print_packages(group) {
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
            ("STUB_UID".to_string(), uid),
            (
                "STUB_ROOT_HOME".to_string(),
                dir.join("roothome").display().to_string(),
            ),
        ];
        Self {
            dir,
            repo,
            root,
            home,
            runtime,
            state,
            log,
            env,
        }
    }

    fn set_env(&mut self, key: &str, value: &str) {
        self.env.retain(|(k, _)| k != key);
        self.env.push((key.to_string(), value.to_string()));
    }

    fn unset_env(&mut self, key: &str) {
        self.env.retain(|(k, _)| k != key);
    }

    fn run(&self, args: &[&str]) -> Output {
        fs::write(&self.log, "").expect("reset log");
        let root = |rel: &str| self.root.join(rel).display().to_string();
        let mut cmd = Command::new(bash_path());
        cmd.arg(self.repo.join(SETUP))
            .args([
                "--prefix".to_string(),
                root("usr"),
                "--sysconfdir".to_string(),
                root("etc"),
                "--localstatedir".to_string(),
                root("var"),
                "--runstatedir".to_string(),
                root("run"),
                "--pam-dir".to_string(),
                root("usr/lib/security"),
            ])
            .args(args)
            .current_dir(&self.dir)
            .env_clear()
            .stdin(Stdio::null());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        cmd.output().expect("run setup.sh")
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

    fn remote_toml(&self) -> PathBuf {
        self.home.join(".config/soos/remote.toml")
    }

    fn daemon_toml(&self) -> PathBuf {
        self.root.join("etc/soos/daemon.toml")
    }

    fn user_unit(&self, unit: &str, what: &str) -> bool {
        self.state.join(format!("user_{unit}_{what}")).exists()
    }

    /// Whole-tree snapshot (type, mode, content) of the fake root, HOME, runtime directory,
    /// checkout and temporary directory.
    fn snapshot(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for base in [
            &self.root,
            &self.home,
            &self.runtime,
            &self.repo,
            &self.dir.join("tmp"),
        ] {
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

/// Read-only log lines (a run that must change nothing may produce only these).
fn is_read_only_call(line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    let cmd = words.get(1).copied().unwrap_or("");
    let arg = words.get(2).copied().unwrap_or("");
    let arg2 = words.get(3).copied().unwrap_or("");
    match cmd {
        "loginctl" => arg == "show-session" || arg == "show-user",
        "pacman" => arg == "-Q",
        "systemctl" => {
            arg == "is-enabled"
                || arg == "is-active"
                || (arg == "--user" && (arg2 == "is-enabled" || arg2 == "is-active"))
        }
        "tailscale" => arg == "version",
        "download_models.sh" => line.contains(" --check-only"),
        _ => false,
    }
}

/// Only `tailscale version` ever runs; no PAM tool, `curl`, `usermod` or linger change.
fn assert_no_forbidden_calls(lines: &[String]) {
    for line in lines {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.get(1) == Some(&"tailscale") {
            assert_eq!(
                words.get(2),
                Some(&"version"),
                "only `tailscale version`: {line}"
            );
        }
        for forbidden in ["pam-auth-update", "authselect", "curl", "usermod"] {
            assert_ne!(
                words.get(1),
                Some(&forbidden),
                "setup.sh must never run {forbidden}: {line}"
            );
        }
        assert!(
            !line.contains("enable-linger"),
            "setup.sh must never enable linger: {line}"
        );
    }
}

/// The managed `remote.toml` setup renders for these boolean values (in `BOOL_KEYS` order).
fn managed_remote(values: [bool; 7]) -> String {
    let mut s = String::new();
    s.push_str(REMOTE_MARKER);
    s.push('\n');
    s.push_str(
        "# A feature turned off later (for example in the soos-gui feature tour) stays off until\n",
    );
    s.push_str("# setup runs with its --<feature> on flag. Reference: Docs/REMOTE_COMPANION.md section 5.\n");
    s.push_str(&format!("allowed_logins = [\"{LOGIN}\"]\n"));
    s.push_str(&format!("rp_id = \"{RP_ID}\"\n"));
    for (key, value) in BOOL_KEYS.iter().zip(values) {
        s.push_str(&format!("{key} = {value}\n"));
    }
    s
}

/// Rewrites `key = true` to `key = false` like the soos-gui feature tour does.
fn gui_turn_off(path: &Path, key: &str) {
    let text = fs::read_to_string(path).expect("read remote.toml");
    let from = format!("{key} = true\n");
    assert!(text.contains(&from), "{key} must be on before the GUI edit");
    fs::write(path, text.replace(&from, &format!("{key} = false\n"))).expect("write");
}

// ---------------------------------------------------------------------------
// Behavioral contracts
// ---------------------------------------------------------------------------

/// SET21: `--help` documents every companion option and exit code 5.
#[test]
fn test_set_u3_help_documents_the_companion_options() {
    let h = Harness::new("help");
    let out = h.run(&["--help"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let help = text(&out);
    for opt in [
        "--no-remote",
        "--tailscale-login",
        "--rp-id",
        "--funnel",
        "--remote-unlock",
        "--password-alerts",
        "--push",
        "--offline-alerts",
        "--camera-view",
        "--camera-view-funnel",
    ] {
        assert!(help.contains(opt), "--help must document {opt}");
    }
    assert!(help.contains("  5 "), "--help must list exit code 5");
    assert!(h.log_lines().is_empty(), "--help must run nothing");
}

/// SET22: inconsistent or invalid companion options exit 2 before any change.
#[test]
fn test_set_u4_inconsistent_options_exit_2_without_change() {
    let mut h = Harness::new("badopts");
    let before = h.snapshot();
    for args in [
        vec!["--push", "on", "--password-alerts", "off"],
        vec!["--offline-alerts", "on", "--push", "off"],
        vec!["--offline-alerts", "on", "--password-alerts", "off"],
        vec!["--camera-view-funnel", "on", "--camera-view", "off"],
        vec!["--camera-view-funnel", "on", "--funnel", "off"],
        vec!["--no-remote", "--funnel", "on"],
        vec!["--no-remote", "--tailscale-login", LOGIN],
        vec!["--funnel", "maybe"],
        vec!["--push"],
        vec!["--tailscale-login", "bad login"],
        vec!["--tailscale-login", "a\"b@example.com"],
        vec!["--rp-id", "pc.ts.net"],
        vec!["--rp-id", "pc.example.com"],
        vec!["--rp-id", "pc..tail.ts.net"],
        vec!["--rp-id", "pc.tail1234.ts.net:443"],
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
    // A feature flag without the login and node name cannot be applied with --yes.
    for args in [
        vec!["--yes", "--funnel", "on"],
        vec!["--yes", "--tailscale-login", LOGIN],
        vec!["--yes", "--rp-id", RP_ID],
    ] {
        let out = h.run(&args);
        assert_eq!(code(&out), 2, "{args:?} must exit 2: {}", text(&out));
        assert!(text(&out).contains("--rp-id") || text(&out).contains("--tailscale-login"));
    }
    // Started as root, the companion cannot be set up: its options are refused.
    h.set_env("STUB_AS_ROOT", "1");
    h.set_env("SUDO_USER", "alice");
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert!(text(&out).contains("without sudo"), "{}", text(&out));
    assert_eq!(before, h.snapshot(), "refused runs must write nothing");
}

/// SET23: `--dry-run` with the login and node name prints the companion plan, never the login,
/// and changes nothing.
#[test]
fn test_set_u5_dry_run_prints_the_companion_plan_without_change() {
    let h = Harness::new("dry");
    let before = h.snapshot();
    let out = h.run(&["--dry-run", "--tailscale-login", LOGIN, "--rp-id", RP_ID]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    for needle in [
        "[PLAN] companion: will run",
        "[PLAN] companion config: will run",
        "Funnel on",
        "remote unlock on",
        "camera view on",
        "push on",
        "offline alerts on",
        "[PLAN] camera view (daemon): manual (not applied)",
        "soos-daemon reads /etc/soos/daemon.toml",
        "[PLAN] companion units: will run",
        "[PLAN] tailscale: printed only",
        "Dry run",
    ] {
        assert!(t.contains(needle), "dry-run must print {needle}:\n{t}");
    }
    assert!(
        !t.contains(LOGIN),
        "the Tailscale login is never printed:\n{t}"
    );
    let lines = h.log_lines();
    assert!(
        lines.iter().all(|l| is_read_only_call(l)),
        "dry-run must only query: {lines:?}"
    );
    assert_eq!(before, h.snapshot(), "dry-run must write nothing");

    // Without the login and node name the plan says how to provide them.
    let out = h.run(&["--dry-run"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    assert!(
        t.contains("[PLAN] companion: skipped") && t.contains("--tailscale-login"),
        "{t}"
    );
    assert_eq!(before, h.snapshot());
}

/// SET24: a full run with the login and node name writes the default-on configuration, creates
/// the daemon `[preview]` section, installs and starts the companion as the user, and only
/// prints the Tailscale, passkey, journal and linger steps.
#[test]
fn test_set_u6_full_run_sets_up_the_default_on_companion() {
    let h = Harness::new("full");
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let lines = h.log_lines();
    assert_no_forbidden_calls(&lines);
    assert_eq!(
        lines.iter().filter(|l| l.contains("uid=1000 sudo")).count(),
        1,
        "one escalation (the packages are installed): {lines:?}"
    );
    let escalate = find_line(&lines, &["uid=1000 sudo", "--system-phase"]);
    assert!(
        !lines[escalate].contains("--camera-view"),
        "the daemon side is not requested outside /etc: {}",
        lines[escalate]
    );
    assert!(
        !lines[escalate].contains(LOGIN) && !lines[escalate].contains("--rp-id"),
        "the root half never receives the companion identity: {}",
        lines[escalate]
    );

    // Daemon side: soos-daemon reads only /etc/soos/daemon.toml, so with the fake
    // --sysconfdir nothing is written, nothing restarted, and the exact change is printed.
    let uid = real_uid();
    assert!(
        !h.daemon_toml().exists(),
        "never written where the daemon does not read"
    );
    assert!(!has_line(&lines, &["systemctl restart soos-daemon"]));
    let t = text(&out);
    assert!(
        t.contains("/etc/soos/daemon.toml") && t.contains(&format!("allowed_uids = [{uid}]")),
        "the administrator change for the file the daemon reads is printed:\n{t}"
    );

    // User side: configuration, install, validation, units — all as the user.
    let remote = h.remote_toml();
    assert_eq!(
        fs::read_to_string(&remote).expect("remote.toml"),
        managed_remote([true; 7])
    );
    assert_eq!(mode_of(&remote), 0o600);
    assert_eq!(mode_of(remote.parent().unwrap()), 0o700);
    let install = find_line(&lines, &["uid=1000 install_remote.sh"]);
    let env_line = &lines[find_line(&lines, &["install-remote-env"])];
    assert!(
        env_line.contains(&format!(
            "CARGO_TARGET_DIR={}",
            h.repo.join("target").display()
        )),
        "{env_line}"
    );
    let validate = find_line(&lines, &["uid=1000 soos-remote --config", "passkeys list"]);
    let enable = find_line(
        &lines,
        &["uid=1000 systemctl --user enable --now soos-remote.service"],
    );
    let sender = find_line(
        &lines,
        &[&format!(
            "uid=0 systemctl enable --now soos-push-sender@{}.socket",
            real_uid()
        )],
    );
    let system = find_line(&lines, &["uid=0 install.sh"]);
    assert!(system < install && install < validate && validate < enable);
    assert!(
        system < sender && sender < install,
        "the root half enables the sender socket between install.sh and install_remote.sh"
    );
    assert!(!has_line(&lines, &["uid=0 install_remote.sh"]));
    assert!(!has_line(&lines, &["uid=0 soos-remote"]));
    assert!(h.user_unit("soos-remote.service", "active"));
    assert!(!h.user_unit("soos-push-sender.service", "enabled"));

    let t = text(&out);
    assert!(
        !t.contains(LOGIN),
        "the Tailscale login is never printed:\n{t}"
    );
    for needle in [
        "tailscale funnel --bg unix:",
        "remote.sock",
        &format!("https://{RP_ID}"),
        "soos-remote enroll-code",
        "usermod -aG systemd-journal alice",
        "loginctl enable-linger alice",
        "[DONE] companion",
    ] {
        assert!(t.contains(needle), "output must contain {needle}:\n{t}");
    }
    assert!(h.root.join("var/lib/soos/state/setup.json").exists());
}

/// SET25: a second run (with or without the identity flags) changes nothing and reports every
/// companion step as already done; the journal group is reported when present.
#[test]
fn test_set_u7_second_run_is_idempotent() {
    let mut h = Harness::new("idem");
    let first = h.run_companion(&[]);
    assert_eq!(code(&first), 0, "{}", text(&first));
    h.set_env("STUB_EXTRA_GROUPS", "systemd-journal");
    let before = h.snapshot();
    for args in [
        vec!["--yes", "--tailscale-login", LOGIN, "--rp-id", RP_ID],
        vec!["--yes"],
    ] {
        let out = h.run(&args);
        assert_eq!(code(&out), 0, "{args:?}: {}", text(&out));
        let lines = h.log_lines();
        for absent in [
            &["systemctl --user enable"][..],
            &["systemctl --user start"],
            &["systemctl --user restart"],
            &["systemctl --user try-restart"],
            &["systemctl restart soos-daemon"],
        ] {
            assert!(
                !has_line(&lines, absent),
                "{args:?}: second run must not run {absent:?}: {lines:?}"
            );
        }
        let t = text(&out);
        assert!(
            t.contains("[DONE] companion: already up to date"),
            "{args:?}: unchanged installed files are reported:\n{t}"
        );
        assert!(
            t.contains("[PLAN] camera view (daemon): manual (not applied)"),
            "{args:?}: the daemon side outside /etc is never reported done:\n{t}"
        );
        for phase in ["companion config", "companion units"] {
            assert!(
                t.contains(&format!("[PLAN] {phase}: already done")),
                "{args:?}: {phase} must be already done:\n{t}"
            );
        }
        assert!(
            !t.contains("usermod -aG"),
            "a member of systemd-journal gets no group hint:\n{t}"
        );
        assert_eq!(
            before,
            h.snapshot(),
            "{args:?}: second run must change nothing"
        );
    }
}

/// SET26: an explicit `false` written later (the GUI tour) is kept, with its dependents;
/// a matching `--<feature> on` turns it back on; an `on` whose prerequisite is kept off is
/// refused before any change.
#[test]
fn test_set_u8_explicit_false_is_kept_unless_its_flag_says_on() {
    let h = Harness::new("keepoff");
    let first = h.run_companion(&[]);
    assert_eq!(code(&first), 0, "{}", text(&first));
    gui_turn_off(&h.remote_toml(), "allow_funnel");
    gui_turn_off(&h.remote_toml(), "camera_view_funnel");
    gui_turn_off(&h.remote_toml(), "push_notifications");
    gui_turn_off(&h.remote_toml(), "offline_alerts");

    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(
        fs::read_to_string(h.remote_toml()).unwrap(),
        managed_remote([false, true, true, false, false, true, false]),
        "keys turned off later stay off"
    );
    assert!(text(&out).contains("kept off"), "{}", text(&out));

    let before = h.snapshot();
    let out = h.run(&["--yes", "--offline-alerts", "on"]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert!(text(&out).contains("--push on"), "{}", text(&out));
    assert_eq!(before, h.snapshot(), "a refused run writes nothing");
    let out = h.run(&["--yes", "--camera-view-funnel", "on"]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert!(text(&out).contains("--funnel on"), "{}", text(&out));
    assert_eq!(before, h.snapshot());

    let out = h.run(&["--yes", "--funnel", "on", "--push", "on"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(
        fs::read_to_string(h.remote_toml()).unwrap(),
        managed_remote([true, true, true, true, false, true, false]),
        "only the keys named by an on flag come back; their dependents stay as they were"
    );
    let lines = h.log_lines();
    find_line(
        &lines,
        &["uid=1000 systemctl --user restart soos-remote.service"],
    );
    let backup = h.home.join(".config/soos/remote.toml.soos-setup.bak");
    assert_eq!(
        mode_of(&backup),
        0o600,
        "the replaced file is kept as a 0600 backup"
    );
}

/// SET27: switching push or password alerts off also switches its dependents off; offline
/// alerts are never written without push; the sender unit is not enabled without push.
#[test]
fn test_set_u9_prerequisites_cascade_and_offline_needs_push() {
    let h = Harness::new("cascade");
    let out = h.run_companion(&["--push", "off"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(
        fs::read_to_string(h.remote_toml()).unwrap(),
        managed_remote([true, true, true, false, false, true, true])
    );
    assert!(!h.user_unit("soos-push-sender.service", "enabled"));

    let h = Harness::new("cascade2");
    let out = h.run_companion(&["--password-alerts", "off", "--funnel", "off"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(
        fs::read_to_string(h.remote_toml()).unwrap(),
        managed_remote([false, true, false, false, false, true, false])
    );
    let t = text(&out);
    assert!(
        t.contains("tailscale serve --bg unix:") && !t.contains("tailscale funnel --bg"),
        "without Funnel the tailnet command is printed:\n{t}"
    );

    let h = Harness::new("cascade3");
    let out = h.run_companion(&["--camera-view", "off"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(
        fs::read_to_string(h.remote_toml()).unwrap(),
        managed_remote([true, true, true, true, true, false, false])
    );
    assert!(
        !h.daemon_toml().exists(),
        "no daemon side without the camera view"
    );
    assert!(!has_line(&h.log_lines(), &["--camera-view on"]));
}

/// SET28: without the login and node name a `--yes` run skips the companion with the exact
/// steps and touches nothing of it; `--no-remote` skips it silently.
#[test]
fn test_set_u10_without_identity_the_companion_is_skipped() {
    let h = Harness::new("noid");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    assert!(
        t.contains("--tailscale-login") && t.contains("--rp-id"),
        "the re-run command is printed:\n{t}"
    );
    assert!(
        t.contains("Phone companion: skipped ("),
        "the skip reason is repeated after the run:\n{t}"
    );
    let lines = h.log_lines();
    assert!(!has_line(&lines, &["install_remote.sh"]));
    assert!(!has_line(&lines, &["systemctl --user"]));
    assert!(!has_line(&lines, &["--camera-view"]));
    assert!(!h.home.join(".config").exists());
    assert!(!h.daemon_toml().exists());

    let h = Harness::new("noremote");
    let out = h.run_companion(&["--no-remote"]);
    assert_eq!(
        code(&out),
        2,
        "--no-remote with companion options: {}",
        text(&out)
    );
    let out = h.run(&["--yes", "--no-remote"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(text(&out).contains("[PLAN] companion: skipped (--no-remote)"));
    assert!(!h.home.join(".config").exists());
}

/// SET29: started as root, the companion is skipped with the command to run as the user; the
/// root start never writes under the user's HOME.
#[test]
fn test_set_u11_root_start_skips_the_companion() {
    let mut h = Harness::new("root");
    h.set_env("STUB_AS_ROOT", "1");
    h.set_env("SUDO_USER", "alice");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    assert!(
        t.contains("[PLAN] companion: skipped") && t.contains("without sudo"),
        "{t}"
    );
    assert!(!h.home.join(".config").exists());
    assert!(!h.home.join(".local").exists());
    assert!(!h.daemon_toml().exists());
    assert!(!has_line(&h.log_lines(), &["install_remote.sh"]));
}

/// SET30: the untouched `install_remote.sh` template is replaced (backup kept); a user-managed
/// file is never edited and the companion options are then refused.
#[test]
fn test_set_u12_template_replaced_user_file_untouched() {
    let h = Harness::new("template");
    let dir = h.home.join(".config/soos");
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let template = "# soos-remote configuration\n#\n# comment\nallowed_logins = []\n\n# allow_funnel = false\n";
    fs::write(h.remote_toml(), template).unwrap();
    fs::set_permissions(h.remote_toml(), fs::Permissions::from_mode(0o600)).unwrap();
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(
        fs::read_to_string(h.remote_toml()).unwrap(),
        managed_remote([true; 7])
    );
    let backup = dir.join("remote.toml.soos-setup.bak");
    assert_eq!(fs::read_to_string(&backup).unwrap(), template);
    assert_eq!(mode_of(&backup), 0o600);

    let h = Harness::new("userfile");
    let dir = h.home.join(".config/soos");
    fs::create_dir_all(&dir).unwrap();
    let mine = "allowed_logins = [\"me@example.com\"]\nrp_id = \"pc.tail1234.ts.net\"\nallow_funnel = false\n";
    fs::write(h.remote_toml(), mine).unwrap();
    fs::set_permissions(h.remote_toml(), fs::Permissions::from_mode(0o600)).unwrap();
    let before = h.snapshot();
    let out = h.run_companion(&[]);
    assert_eq!(
        code(&out),
        2,
        "options with a user-managed file: {}",
        text(&out)
    );
    assert_eq!(before, h.snapshot());
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(fs::read_to_string(h.remote_toml()).unwrap(), mine);
    let t = text(&out);
    assert!(t.contains("[PLAN] companion config: kept"), "{t}");
    let lines = h.log_lines();
    assert!(!has_line(&lines, &["systemctl --user enable"]));
    assert!(!has_line(&lines, &["--camera-view on"]));
}

/// SET31: a symbolic link (configuration file or directory) or a relative `XDG_CONFIG_HOME`
/// refuses the companion options before any change; a missing `XDG_RUNTIME_DIR` skips it.
#[test]
fn test_set_u13_unsafe_paths_refused_before_change() {
    let mut h = Harness::new("links");
    let dir = h.home.join(".config/soos");
    fs::create_dir_all(&dir).unwrap();
    let target = h.dir.join("elsewhere.toml");
    fs::write(&target, "x = 1\n").unwrap();
    std::os::unix::fs::symlink(&target, h.remote_toml()).unwrap();
    let before = h.snapshot();
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert!(text(&out).contains("symbolic link"), "{}", text(&out));
    assert_eq!(before, h.snapshot());
    assert_eq!(fs::read_to_string(&target).unwrap(), "x = 1\n");
    fs::remove_file(h.remote_toml()).unwrap();

    h.set_env("XDG_CONFIG_HOME", "relative/config");
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    h.unset_env("XDG_CONFIG_HOME");

    h.unset_env("XDG_RUNTIME_DIR");
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(text(&out).contains("XDG_RUNTIME_DIR"), "{}", text(&out));
    assert!(!h.remote_toml().exists());
}

/// SET32: an existing `daemon.toml` is never edited: the exact `[preview]` lines are printed,
/// or it is reported already done when it allows the view.
#[test]
fn test_set_u14_existing_daemon_toml_is_never_edited() {
    let h = Harness::new("daemontoml");
    fs::create_dir_all(h.root.join("etc/soos")).unwrap();
    let mine = "[pipeline]\ncamera_device = \"/dev/video2\"\n";
    fs::write(h.daemon_toml(), mine).unwrap();
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(fs::read_to_string(h.daemon_toml()).unwrap(), mine);
    let t = text(&out);
    assert!(
        t.contains("remote_view = true") && t.contains(&format!("allowed_uids = [{}]", real_uid())),
        "the administrator change is printed:\n{t}"
    );
    assert!(!has_line(
        &h.log_lines(),
        &["systemctl restart soos-daemon"]
    ));

    let h = Harness::new("daemontoml2");
    fs::create_dir_all(h.root.join("etc/soos")).unwrap();
    let allows = format!(
        "[pipeline]\ncamera_device = \"/dev/video2\"\n\n[preview]\nenabled = true\nallowed_uids = [4242, {}]\nremote_view = true\n",
        real_uid()
    );
    fs::write(h.daemon_toml(), &allows).unwrap();
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(
        text(&out).contains("[PLAN] camera view (daemon): manual (not applied)"),
        "{}",
        text(&out)
    );
    assert_eq!(fs::read_to_string(h.daemon_toml()).unwrap(), allows);
}

/// SET33: a companion failure exits 5, rolls back the companion changes of the run (created
/// file removed, replaced file restored, units enabled by setup disabled) and keeps the face
/// install.
#[test]
fn test_set_u15_companion_failures_roll_back_and_exit_5() {
    let mut h = Harness::new("cfail");
    h.set_env("STUB_REMOTE_RC", "78");
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 5, "{}", text(&out));
    assert!(
        !h.remote_toml().exists(),
        "a created configuration is removed"
    );
    assert!(!h.user_unit("soos-remote.service", "enabled"));
    assert!(h.root.join("var/lib/soos/state/setup.json").exists());
    assert!(text(&out).contains("[FAIL] companion"), "{}", text(&out));

    h.unset_env("STUB_REMOTE_RC");
    let ok = h.run_companion(&[]);
    assert_eq!(code(&ok), 0, "{}", text(&ok));
    let good = fs::read_to_string(h.remote_toml()).unwrap();
    h.set_env("STUB_REMOTE_RC", "78");
    let out = h.run_companion(&["--remote-unlock", "off"]);
    assert_eq!(code(&out), 5, "{}", text(&out));
    assert_eq!(
        fs::read_to_string(h.remote_toml()).unwrap(),
        good,
        "a replaced configuration is restored"
    );
    assert_eq!(mode_of(&h.remote_toml()), 0o600);

    let mut h = Harness::new("ufail");
    h.set_env("STUB_USER_START_RC", "1");
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 5, "{}", text(&out));
    assert!(!h.user_unit("soos-remote.service", "enabled"));
    assert!(!h.user_unit("soos-push-sender.service", "enabled"));
    assert!(!h.remote_toml().exists());

    let mut h = Harness::new("ifail");
    h.set_env("STUB_INSTALL_REMOTE_RC", "1");
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 5, "{}", text(&out));
    assert!(!h.remote_toml().exists());
    assert!(text(&out).contains("install_remote.sh"), "{}", text(&out));
}

/// SET33 (candid review round 2): a unit this run started (inactive before) is stopped again by
/// the companion rollback; its enablement is left as it was.
#[test]
fn test_set_u22_rollback_stops_a_unit_started_by_the_run() {
    let mut h = Harness::new("ustart");
    let first = h.run_companion(&[]);
    assert_eq!(code(&first), 0, "{}", text(&first));
    fs::remove_file(h.state.join("user_soos-remote.service_active")).unwrap();
    h.set_env("STUB_USER_DIE_UNIT", "soos-remote.service");
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 5, "{}", text(&out));
    let lines = h.log_lines();
    find_line(
        &lines,
        &["uid=1000 systemctl --user start soos-remote.service"],
    );
    find_line(
        &lines,
        &["uid=1000 systemctl --user stop soos-remote.service"],
    );
    assert!(!h.user_unit("soos-remote.service", "active"));
    assert!(h.user_unit("soos-remote.service", "enabled"));

    // Active before the run and restarted by it: the rollback restarts it on the restored file.
    let mut h = Harness::new("urestart");
    let first = h.run_companion(&[]);
    assert_eq!(code(&first), 0, "{}", text(&first));
    let good = fs::read_to_string(h.remote_toml()).unwrap();
    h.set_env("STUB_USER_DIE_UNIT", "soos-remote.service");
    let out = h.run(&["--yes", "--remote-unlock", "off"]);
    assert_eq!(code(&out), 5, "{}", text(&out));
    assert_eq!(fs::read_to_string(h.remote_toml()).unwrap(), good);
    let lines = h.log_lines();
    let restarts: Vec<_> = lines
        .iter()
        .filter(|l| l.contains("uid=1000 systemctl --user restart soos-remote.service"))
        .collect();
    assert_eq!(
        restarts.len(),
        2,
        "restarted by the run, then by the rollback: {lines:?}"
    );
    assert!(h.user_unit("soos-remote.service", "active"));

    // A directory created by the failed run is removed again.
    let mut h = Harness::new("udir");
    h.set_env("STUB_REMOTE_RC", "78");
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 5, "{}", text(&out));
    assert!(!h.home.join(".config/soos").exists());

    // An unreadable remote.toml is refused cleanly before any change.
    let h = Harness::new("unread");
    let dir = h.home.join(".config/soos");
    fs::create_dir_all(&dir).unwrap();
    fs::write(h.remote_toml(), "x = 1\n").unwrap();
    fs::set_permissions(h.remote_toml(), fs::Permissions::from_mode(0o000)).unwrap();
    let readable = fs::read(h.remote_toml()).is_ok();
    let out = h.run_companion(&[]);
    if !readable {
        assert_eq!(code(&out), 2, "{}", text(&out));
        assert!(text(&out).contains("readable"), "{}", text(&out));
    }
    fs::set_permissions(h.remote_toml(), fs::Permissions::from_mode(0o600)).unwrap();
}

/// SET34 (lead review of PR #402): soos-daemon reads only `/etc/soos/daemon.toml`, so the
/// root half never writes the `[preview]` section under another `--sysconfdir`, even when asked
/// with `--camera-view on`; it reports the step as not applied with the exact change, restarts
/// nothing, and the run still succeeds.
#[test]
fn test_set_u16_daemon_side_never_written_outside_etc() {
    let mut h = Harness::new("dside");
    let first = h.run(&["--yes", "--no-remote"]);
    assert_eq!(code(&first), 0, "{}", text(&first));
    h.set_env("STUB_AS_ROOT", "1");
    let artifacts = h.repo.join("target/release").display().to_string();
    let out = h.run(&[
        "--system-phase",
        "--yes",
        "--user",
        "alice",
        "--artifact-dir",
        &artifacts,
        "--local-seat",
        "no",
        "--distro",
        "arch",
        "--camera-view",
        "on",
    ]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    assert!(
        t.contains("[SKIP] camera view (daemon): not applied")
            && t.contains("/etc/soos/daemon.toml")
            && t.contains(&format!("allowed_uids = [{}]", real_uid())),
        "{t}"
    );
    assert!(!h.daemon_toml().exists());
    assert!(!h.root.join("etc/soos").read_dir().is_ok_and(|mut d| d
        .any(|e| e.is_ok_and(|e| e.file_name().to_string_lossy().starts_with(".daemon.toml")))));
    assert!(!has_line(
        &h.log_lines(),
        &["systemctl restart soos-daemon"]
    ));
}

/// SET30 (lead review of PR #402): a `remote.toml.soos-setup.bak` that is not a regular file is
/// refused before any change (a backup could not be written or restored through it).
#[test]
fn test_set_u23_backup_path_must_be_a_regular_file() {
    let h = Harness::new("bakdir");
    let dir = h.home.join(".config/soos");
    fs::create_dir_all(dir.join("remote.toml.soos-setup.bak")).unwrap();
    let before = h.snapshot();
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert!(text(&out).contains("soos-setup.bak"), "{}", text(&out));
    assert_eq!(before, h.snapshot());
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(
        text(&out).contains("[PLAN] companion: skipped (unsafe path)"),
        "{}",
        text(&out)
    );
    assert!(!h.remote_toml().exists());
}

/// SET37 (plan evaluation F2): a companion unit the owner disabled later stays disabled on a
/// re-run; the command to enable it is printed.
#[test]
fn test_set_u19_disabled_unit_stays_disabled() {
    let h = Harness::new("keepunit");
    let first = h.run_companion(&[]);
    assert_eq!(code(&first), 0, "{}", text(&first));
    for what in ["enabled", "active"] {
        fs::remove_file(h.state.join(format!("user_soos-remote.service_{what}"))).unwrap();
    }
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(!h.user_unit("soos-remote.service", "enabled"));
    assert!(!h.user_unit("soos-remote.service", "active"));
    let t = text(&out);
    assert!(t.contains("[PLAN] companion units: kept off"), "{t}");
    assert!(
        t.contains("systemctl --user enable --now soos-remote.service"),
        "{t}"
    );
    assert!(!has_line(&h.log_lines(), &["systemctl --user enable"]));
}

/// SET37 (plan evaluation round 2, R2-F1): switching push on later enables and starts the
/// sender like a first write.
#[test]
fn test_set_u21_push_switched_on_later_enables_the_sender() {
    let mut h = Harness::new("pushon");
    let socket = format!("soos-push-sender@{}.socket", real_uid());
    let enable_line = format!("uid=0 systemctl enable --now {socket}");
    let first = h.run_companion(&["--push", "off"]);
    assert_eq!(code(&first), 0, "{}", text(&first));
    assert!(!has_line(&h.log_lines(), &[&enable_line]));
    let out = h.run(&["--yes", "--push", "on"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    find_line(&h.log_lines(), &[&enable_line]);
    assert!(!h.user_unit("soos-push-sender.service", "enabled"));
    assert!(
        !text(&out).contains(&format!("{socket} is not enabled")),
        "{}",
        text(&out)
    );
    // Push already on and the sender disabled later: it stays disabled.
    h.set_env("STUB_SYSTEM_OFF_UNIT", &socket);
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(
        !has_line(&h.log_lines(), &[&enable_line]),
        "{:?}",
        h.log_lines()
    );
    assert!(
        text(&out).contains(&format!("{socket} is not enabled")),
        "{}",
        text(&out)
    );
}

/// SET38 (plan evaluation F-m10, F-m11): a group- or world-writable configuration directory is
/// refused; the managed render is byte-equal to the fixture that `soos-remote`'s own parser
/// accepts (`crates/remote/tests/setup_render_tests.rs`).
#[test]
fn test_set_u20_writable_config_dir_refused_and_render_matches_fixture() {
    let h = Harness::new("wdir");
    let dir = h.home.join(".config/soos");
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o775)).unwrap();
    let before = h.snapshot();
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert!(
        text(&out).contains("group- or world-writable"),
        "{}",
        text(&out)
    );
    assert_eq!(before, h.snapshot());

    assert_eq!(
        managed_remote([true; 7]),
        read_repo("tests/fixtures/setup/remote.toml"),
        "the contract render and the parser fixture are the same bytes"
    );
}

/// SET35: the real `install_remote.sh` installs the binaries cargo built into
/// `CARGO_TARGET_DIR` (absolute or relative to the checkout).
#[test]
fn test_set_u17_install_remote_uses_cargo_target_dir() {
    for (tag, target_arg) in [("abs", None), ("rel", Some("custom-target"))] {
        let dir =
            std::env::temp_dir().join(format!("soos_install_remote_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        let home = dir.join("home");
        let stubs = dir.join("stubs");
        let log = dir.join("log");
        for d in [&repo, &home, &stubs] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(&log, "").unwrap();
        for rel in [
            INSTALL_REMOTE,
            "packaging/soos-remote.service",
            "packaging/soos-push-sender.service",
        ] {
            let dst = repo.join(rel);
            fs::create_dir_all(dst.parent().unwrap()).unwrap();
            fs::copy(workspace_root().join(rel), &dst).unwrap();
        }
        let target = match target_arg {
            None => dir.join("abs-target"),
            Some(rel) => repo.join(rel),
        };
        let shebang = format!("#!{}\n", bash_path().display());
        write_exec(
            &stubs.join("cargo"),
            &format!(
                "{shebang}out=\"${{CARGO_TARGET_DIR:-$PWD/target}}\"\n[[ \"$out\" == /* ]] || out=\"$PWD/$out\"\nmkdir -p \"$out/release\"\nprintf 'remote\\n' > \"$out/release/soos-remote\"\nprintf 'sender\\n' > \"$out/release/soos-push-sender\"\n"
            ),
        );
        write_exec(
            &stubs.join("systemctl"),
            &format!("{shebang}printf '%s\\n' \"$*\" >> {}\n", log.display()),
        );
        let path = format!(
            "{}:{}",
            stubs.display(),
            std::env::var("PATH").expect("PATH")
        );
        let value = match target_arg {
            None => target.display().to_string(),
            Some(rel) => rel.to_string(),
        };
        let out = Command::new(bash_path())
            .arg(repo.join(INSTALL_REMOTE))
            .current_dir(&dir)
            .env_clear()
            .env("PATH", path)
            .env("HOME", &home)
            .env("XDG_RUNTIME_DIR", dir.join("run"))
            .env("CARGO_TARGET_DIR", value)
            .stdin(Stdio::null())
            .output()
            .expect("run install_remote.sh");
        assert!(out.status.success(), "{tag}: {}", text(&out));
        assert_eq!(
            fs::read_to_string(home.join(".local/bin/soos-remote")).unwrap(),
            "remote\n",
            "{tag}: binary copied from the cargo target directory"
        );
        assert_eq!(
            fs::read_to_string(home.join(".local/bin/soos-push-sender")).unwrap(),
            "sender\n"
        );
        assert!(target.join("release/soos-remote").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}

/// SET36: documentation, ADR, matrix rows and walkthrough 213.
#[test]
fn test_set_u18_docs_adr_matrix_and_walkthrough() {
    let pkg = read_repo("Docs/PACKAGING_AND_PROVISIONING.md");
    for needle in [
        "--tailscale-login",
        "--rp-id",
        "--no-remote",
        "camera_view_funnel",
    ] {
        assert!(
            pkg.contains(needle),
            "Docs/PACKAGING_AND_PROVISIONING.md must mention {needle}"
        );
    }
    let remote = read_repo("Docs/REMOTE_COMPANION.md");
    assert!(
        remote.contains("scripts/setup.sh"),
        "Docs/REMOTE_COMPANION.md must point to scripts/setup.sh"
    );
    let adr = read_repo("AI/DECISIONS.md");
    assert!(
        adr.contains("Setup User Phase: Phone Companion Profile With Default-On Features"),
        "AI/DECISIONS.md must carry the user-phase ADR"
    );
    let matrix = read_repo("AI/VERIFICATION_MATRIX.md");
    assert!(
        matrix.contains("| SET20 |")
            && matrix.contains("| SET38 |")
            && matrix.contains("| SET39 |"),
        "the matrix must carry rows SET20-SET39"
    );
    assert!(
        workspace_root()
            .join("AI/walkthroughs/213_setup_user_phase.md")
            .is_file(),
        "walkthrough 213 must exist"
    );
}

/// SET34 (root path, owner decision of 2026-10-17): the `/etc/soos/daemon.toml` creation path,
/// which the fake-`--sysconfdir` harness above cannot reach, runs as real root in Docker:
/// `tests/docker/setup_root_preview_test.sh` drives the unmodified `setup.sh` through the real
/// `sudo` under a real systemd, and its CI job is required (part of `CI Success`).
#[test]
fn test_set_u24_root_preview_docker_job_wired_into_ci() {
    const DOCKER_TEST: &str = "tests/docker/setup_root_preview_test.sh";
    assert!(
        is_executable(&workspace_root().join(DOCKER_TEST)),
        "{DOCKER_TEST} must be executable"
    );
    let script = read_repo(DOCKER_TEST);
    for needle in [
        // Pinned systemd image of the systemd-unit job, read-only workspace.
        "tests/docker/Dockerfile.systemd",
        "\"${repo}:/workspace:ro\"",
        "--privileged --cgroupns=private",
        "host_sysctl_snapshot",
        "assert_host_state_units_masked",
        // The setup.sh under test is the repository's, byte for byte, escalating with sudo.
        "cmp -s /workspace/scripts/setup.sh \"${CHECKOUT}/scripts/setup.sh\"",
        "NOPASSWD",
        "runuser -u \"${TEST_USER}\"",
        "/workspace/packaging/soos-daemon.service",
        "/workspace/scripts/wait_daemon_ready.sh",
        "readonly DAEMON_TOML=\"/etc/soos/daemon.toml\"",
        // The cases of the owner decision.
        "case_a1",
        "case_a2",
        "case_b",
        "case_c1",
        "case_c2",
        "case_d",
        "case_e1",
        "case_e2",
        "644 root:root 1",
        "expect_rc 1",
        "expect_rc 70",
        "companion_not_run",
        "is a symbolic link; setup never follows it",
        "exists and setup never edits it",
    ] {
        assert!(
            script.contains(needle),
            "{DOCKER_TEST} must contain `{needle}`"
        );
    }
    assert!(
        !code_of(DOCKER_TEST).contains("--sysconfdir"),
        "the root path runs with the default /etc, never a --sysconfdir"
    );
    assert!(
        !script.contains(":/workspace\"") && !script.contains(":/workspace "),
        "the workspace is mounted read-only"
    );
    // No test hook in setup.sh: the path soos-daemon reads is a literal constant.
    let setup = read_repo(SETUP);
    assert!(
        setup
            .lines()
            .any(|l| l.trim() == "readonly DAEMON_READ_CONFIG=\"/etc/soos/daemon.toml\""),
        "setup.sh must keep /etc/soos/daemon.toml as a literal constant"
    );
    let ci = read_repo(".github/workflows/ci.yml");
    let start = ci
        .find("\n  setup-root-preview:")
        .expect("CI job setup-root-preview");
    let job = &ci[start + 1..];
    let job = &job[..job.find("\n  # ----").unwrap_or(job.len())];
    for needle in [
        "needs: lint",
        "timeout-minutes:",
        "persist-credentials: false",
        "uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1",
        "run: ./tests/docker/setup_root_preview_test.sh",
    ] {
        assert!(job.contains(needle), "setup-root-preview: `{needle}`");
    }
    assert!(
        !job.contains("permissions:"),
        "no job-level permissions (workflow default contents: read)"
    );
    let needs = ci
        .lines()
        .find(|l| l.trim_start().starts_with("needs: [lint, clippy"))
        .expect("ci-success needs");
    assert!(needs.contains("setup-root-preview"), "required: {needs}");
    assert!(
        !read_repo("scripts/pr_checks_gate.sh").contains("setup.sh root camera view"),
        "the job is required, never advisory"
    );
}

// ---------------------------------------------------------------------------
// Read paths: the rescue kill switch and the normalised `--sysconfdir` (GitHub #455, part of
// #362; follow-up of #402, #452 and #454; ADR 2026-10-18 "Setup Read Paths: the Rescue Kill
// Switch and One Lexical Normalisation of the Path Options"; rows SRP1-SRP10)
// ---------------------------------------------------------------------------

/// The one global kill switch `pam_soos.so` and soos-daemon read.
const KILL_SWITCH_READ_PATH: &str = "/etc/soos/disabled";

/// Value of `pub const <name>: &str = "<value>";` in a Rust source.
fn srp_str_const(source: &str, name: &str) -> String {
    let needle = format!("pub const {name}: &str = \"");
    let start = source
        .find(&needle)
        .unwrap_or_else(|| panic!("{name} not declared"))
        + needle.len();
    let end = source[start..].find('"').expect("closing quote");
    source[start..start + end].to_string()
}

/// The lexical form `setup.sh` gives an absolute path: repeated and trailing slashes collapsed,
/// `.` components dropped (the harness paths never hold `..`).
fn srp_lexical(path: &Path) -> String {
    let raw = path.display().to_string();
    assert!(raw.starts_with('/'), "absolute harness path: {raw}");
    let parts: Vec<&str> = raw
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    assert!(!parts.contains(&".."), "harness path without ..: {raw}");
    format!("/{}", parts.join("/"))
}

/// The read-path constants of the harness copy of `setup.sh` pointed at the fake root (as LWF3
/// does for the lock-widget flag), so that `<root>/etc` plays the role of `/etc`.
fn srp_point_read_paths_at_fake_root(h: &Harness) -> String {
    let etc = srp_lexical(&h.root.join("etc"));
    let path = h.repo.join(SETUP);
    let mut script = fs::read_to_string(&path).unwrap();
    for (name, rel) in [
        ("DAEMON_READ_CONFIG", "daemon.toml"),
        ("LOCK_WIDGET_READ_FLAG", "lockwidget.disable"),
    ] {
        let original = format!("readonly {name}=\"/etc/soos/{rel}\"");
        assert!(script.contains(&original), "{name} line present");
        script = script.replace(&original, &format!("readonly {name}=\"{etc}/soos/{rel}\""));
    }
    // The presence read path of GitHub #453 (PR #454, merged before this section).
    let presence = "readonly PRESENCE_READ_FLAG=\"/etc/soos/presence.disable\"";
    script = script.replace(
        presence,
        &format!("readonly PRESENCE_READ_FLAG=\"{etc}/soos/presence.disable\""),
    );
    fs::write(&path, script).unwrap();
    etc
}

fn srp_line<'a>(t: &'a str, prefix: &str) -> &'a str {
    let found: Vec<&str> = t.lines().filter(|l| l.starts_with(prefix)).collect();
    assert_eq!(found.len(), 1, "one `{prefix}` line:\n{t}");
    found[0]
}

/// SRP1: the rescue command is pinned on every side: `setup.sh` names
/// `KILL_SWITCH_READ_FLAG="/etc/soos/disabled"`, equal to the global flag `pam_soos.so` reads
/// (`DEFAULT_FLAG_DIR` joined with `disabled`, `flag_dir` never set from a module argument) and
/// to the one soos-daemon reads (`DEFAULT_KILL_SWITCH_DIR` joined with `GLOBAL_DISABLE_FLAG`,
/// watched by `main.rs` with no path option); the only `Rescue:` line of `setup.sh` prints it.
#[test]
fn test_srp1_rescue_kill_switch_matches_pam_and_daemon() {
    let script = read_repo(SETUP);
    assert!(
        script
            .lines()
            .any(|l| l == format!("readonly KILL_SWITCH_READ_FLAG=\"{KILL_SWITCH_READ_PATH}\"")),
        "setup.sh pins the kill-switch read path"
    );
    let rescue: Vec<&str> = script.lines().filter(|l| l.contains("Rescue:")).collect();
    assert_eq!(rescue.len(), 1, "one rescue line: {rescue:?}");
    assert!(
        rescue[0].contains("Rescue: sudo touch ${KILL_SWITCH_READ_FLAG} "),
        "the rescue command names the read path: {}",
        rescue[0]
    );
    assert!(
        !rescue[0].contains("${KILL_SWITCH}") && !rescue[0].contains("${SYSCONFDIR}"),
        "the rescue command never names <sysconfdir>: {}",
        rescue[0]
    );

    let pam = read_repo("crates/pam/src/config.rs");
    assert_eq!(
        format!("{}/disabled", srp_str_const(&pam, "DEFAULT_FLAG_DIR")),
        KILL_SWITCH_READ_PATH,
        "setup.sh and pam_soos.so agree"
    );
    assert!(
        pam.contains("self.flag_dir.join(\"disabled\").exists()"),
        "pam_soos.so checks <flag_dir>/disabled"
    );
    let mut assignments = Vec::new();
    for entry in fs::read_dir(workspace_root().join("crates/pam/src"))
        .unwrap()
        .flatten()
    {
        let body = fs::read_to_string(entry.path()).unwrap_or_default();
        for line in body.lines() {
            let t = line.trim();
            if t.contains("flag_dir =") || t.starts_with("flag_dir:") {
                assignments.push(t.to_string());
            }
        }
    }
    assert_eq!(
        assignments,
        vec!["flag_dir: PathBuf::from(DEFAULT_FLAG_DIR),".to_string()],
        "pam_soos.so sets flag_dir only from DEFAULT_FLAG_DIR"
    );

    let presence = read_repo("crates/daemon/src/presence/mod.rs");
    assert_eq!(
        format!(
            "{}/{}",
            srp_str_const(&presence, "DEFAULT_KILL_SWITCH_DIR"),
            srp_str_const(&presence, "GLOBAL_DISABLE_FLAG")
        ),
        KILL_SWITCH_READ_PATH,
        "setup.sh and soos-daemon agree"
    );
    assert!(
        read_repo("crates/daemon/src/main.rs")
            .contains("PresenceSwitch::new(PathBuf::from(DEFAULT_KILL_SWITCH_DIR))"),
        "soos-daemon watches the constant directory, never a configurable one"
    );
}

/// SRP2: with a `--sysconfdir` other than `/etc` (the harness's fake root), the summary's rescue
/// command names `/etc/soos/disabled`, never `<sysconfdir>/soos/disabled`, and setup still
/// never creates a kill switch.
#[test]
fn test_srp2_summary_rescue_names_the_read_kill_switch() {
    let h = Harness::new("srp_rescue");
    let out = h.run(&["--yes", "--no-remote"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    let rescue: Vec<&str> = t.lines().filter(|l| l.contains("Rescue: ")).collect();
    assert_eq!(rescue.len(), 1, "one rescue line:\n{t}");
    assert!(
        rescue[0].contains(&format!("Rescue: sudo touch {KILL_SWITCH_READ_PATH} ")),
        "{}",
        rescue[0]
    );
    let elsewhere = h.root.join("etc/soos/disabled").display().to_string();
    assert!(
        !t.contains(&format!("sudo touch {elsewhere}")),
        "no rescue command names <sysconfdir>:\n{t}"
    );
    assert!(!h.root.join("etc/soos/disabled").exists());
}

/// SRP3: `--sysconfdir` spelt `<etc>/`, `<etc>//` or `<etc-parent>//etc/./` is the read
/// directory for every read-path comparison: with the read paths pointed at the fake root, the
/// dry-run plan creates `daemon.toml` and `lockwidget.disable` there and never says
/// "not applied", and every printed path is the normalised one.
#[test]
fn test_srp3_slash_spellings_of_the_read_directory_are_applied() {
    let h = Harness::new("srp_spellings");
    let etc = srp_point_read_paths_at_fake_root(&h);
    let parent = srp_lexical(&h.root);
    let before = h.snapshot();
    for spelling in [
        format!("{etc}/"),
        format!("{etc}//"),
        format!("{parent}//etc/./"),
    ] {
        let out = h.run(&[
            "--dry-run",
            "--tailscale-login",
            LOGIN,
            "--rp-id",
            RP_ID,
            "--lock-widget",
            "off",
            "--sysconfdir",
            &spelling,
        ]);
        assert_eq!(code(&out), 0, "{spelling}: {}", text(&out));
        let t = text(&out);
        let daemon = srp_line(&t, "[PLAN] camera view (daemon): ");
        assert!(
            daemon.contains(&format!("will run - create {etc}/soos/daemon.toml ")),
            "{spelling}: {daemon}"
        );
        let widget = srp_line(&t, "[PLAN] lock widget: ");
        assert!(
            widget.contains(&format!("{etc}/soos/lockwidget.disable")),
            "{spelling}: {widget}"
        );
        assert!(
            !t.contains("not applied"),
            "{spelling}: the read directory is never reported elsewhere:\n{t}"
        );
        for raw in [
            format!("{etc}//"),
            format!("{etc}/./"),
            format!("{parent}//"),
        ] {
            assert!(
                !t.contains(&raw),
                "{spelling}: no unnormalised path `{raw}`:\n{t}"
            );
        }
    }
    assert_eq!(before, h.snapshot(), "dry runs write nothing");
}

/// SRP4: a real run with `--sysconfdir <etc>//` and the read paths pointed at the fake root
/// reports the lock-widget flag `[DONE]` at the normalised path, never "not applied".
#[test]
fn test_srp4_double_slash_sysconfdir_run_reports_the_flag_done() {
    let h = Harness::new("srp_run");
    let etc = srp_point_read_paths_at_fake_root(&h);
    let spelling = format!("{etc}//");
    let out = h.run(&[
        "--yes",
        "--no-remote",
        "--lock-widget",
        "off",
        "--sysconfdir",
        &spelling,
    ]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    assert!(
        t.contains(&format!(
            "[DONE] lock widget: turned off ({etc}/soos/lockwidget.disable)"
        )),
        "{t}"
    );
    assert!(!t.contains("not applied: the GNOME extension reads"), "{t}");
    assert!(h.root.join("etc/soos/lockwidget.disable").is_file());
}

/// SRP5: the comparison is lexical and never follows a symbolic link: a `--sysconfdir` that is a
/// link to the read directory is still reported "not applied"; a `..` component is refused
/// with exit 2 before any change, for `--sysconfdir` and the other path options.
#[test]
fn test_srp5_links_are_not_followed_and_dot_dot_is_refused() {
    let h = Harness::new("srp_link");
    let etc = srp_point_read_paths_at_fake_root(&h);
    let link = h.dir.join("etc-link");
    std::os::unix::fs::symlink(h.root.join("etc"), &link).unwrap();
    let link = srp_lexical(&link);
    let out = h.run(&[
        "--dry-run",
        "--tailscale-login",
        LOGIN,
        "--rp-id",
        RP_ID,
        "--sysconfdir",
        &link,
    ]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    assert!(
        srp_line(&t, "[PLAN] camera view (daemon): ").contains("manual (not applied)"),
        "a link to the read directory is not the read directory:\n{t}"
    );

    let before = h.snapshot();
    for args in [
        vec!["--sysconfdir".to_string(), format!("{etc}/../etc")],
        vec!["--sysconfdir".to_string(), format!("{etc}/..")],
        vec!["--sysconfdir".to_string(), "/../etc".to_string()],
        vec![
            "--prefix".to_string(),
            format!("{}/../usr", srp_lexical(&h.root.join("usr"))),
        ],
    ] {
        let mut full: Vec<&str> = args.iter().map(String::as_str).collect();
        full.push("--yes");
        let out = h.run(&full);
        assert_eq!(code(&out), 2, "{args:?} must exit 2: {}", text(&out));
        assert!(text(&out).contains(".."), "{args:?}: {}", text(&out));
        let lines = h.log_lines();
        assert!(
            lines.iter().all(|l| is_read_only_call(l)),
            "{args:?} must change nothing: {lines:?}"
        );
    }
    assert_eq!(before, h.snapshot(), "refused runs write nothing");
}

/// SRP6: the documentation and the traceability agree with the code: the setup section of
/// `Docs/PACKAGING_AND_PROVISIONING.md` names the rescue read path and the normalisation, the
/// ADR records the fix, the matrix carries SRP1-SRP6 and walkthrough 244 exists.
#[test]
fn test_srp6_documentation_and_traceability() {
    let pkg = read_repo("Docs/PACKAGING_AND_PROVISIONING.md");
    let flat: String = pkg.split_whitespace().collect::<Vec<_>>().join(" ");
    let para = flat
        .split("**Read paths (GitHub #455).**")
        .nth(1)
        .expect("read-paths paragraph");
    let para = &para[..para.find(" | ").unwrap_or(para.len())];
    for needle in [
        KILL_SWITCH_READ_PATH,
        "Rescue:",
        "`..`",
        "symbolic link",
        "`/etc/`",
        "`/etc//`",
    ] {
        assert!(para.contains(needle), "read-paths paragraph: {needle}");
    }
    assert!(
        read_repo("AI/DECISIONS.md").contains("GitHub #455"),
        "the ADR records the fix"
    );
    let matrix = read_repo("AI/VERIFICATION_MATRIX.md");
    for row in 1..=6 {
        assert!(
            matrix.contains(&format!("| SRP{row} |")),
            "matrix row SRP{row}"
        );
    }
    assert!(workspace_root()
        .join("AI/walkthroughs/244_setup_read_paths.md")
        .is_file());
}

/// Runs a real `--yes --no-remote` setup with `CARGO_TARGET_DIR=<value>` and checks that the
/// root half received, and `install.sh` installed from, `<expected>` (a path without `..`),
/// the directory the fake cargo really wrote.
fn srp_run_with_target_dir(h: &mut Harness, value: &str, expected: &Path) {
    h.set_env("CARGO_TARGET_DIR", value);
    let out = h.run(&["--yes", "--no-remote"]);
    let t = text(&out);
    assert_eq!(code(&out), 0, "CARGO_TARGET_DIR={value}: {t}");
    let expected = expected.display().to_string();
    assert!(!expected.contains("/../"), "{expected}");
    assert!(
        Path::new(&expected).join("soos-daemon").is_file(),
        "the fake cargo wrote {expected}"
    );
    assert!(
        t.contains(&format!("[DONE] build: {expected}\n")),
        "CARGO_TARGET_DIR={value}: the build reports {expected}:\n{t}"
    );
    let lines = h.log_lines();
    let escalate = &lines[find_line(&lines, &["uid=1000 sudo", "--system-phase"])];
    assert!(
        escalate.contains(&format!(" --artifact-dir {expected} ")),
        "CARGO_TARGET_DIR={value}: the root half gets {expected}: {escalate}"
    );
    assert!(
        !escalate.contains("/../"),
        "CARGO_TARGET_DIR={value}: no `..` reaches the root half: {escalate}"
    );
    let install = &lines[find_line(&lines, &["uid=0 install.sh", "--artifact-dir"])];
    assert!(
        install.contains(&format!(" --artifact-dir {expected} ")),
        "CARGO_TARGET_DIR={value}: install.sh installs from {expected}: {install}"
    );
    assert!(
        !t.contains("must not contain a '..' component"),
        "CARGO_TARGET_DIR={value}: {t}"
    );
}

/// SRP7: a relative `CARGO_TARGET_DIR` with a `..` component (`../shared-target`, documented as
/// resolved against the checkout) still installs: after the build the user half passes the root
/// half a `..`-free `--artifact-dir`, the directory cargo wrote.
#[test]
fn test_srp7_relative_target_dir_with_dot_dot_reaches_the_system_phase() {
    let mut h = Harness::new("srp_rel_target");
    let expected = fs::canonicalize(&h.dir)
        .unwrap()
        .join("shared-target/release");
    srp_run_with_target_dir(&mut h, "../shared-target", &expected);
}

/// SRP8: an absolute `CARGO_TARGET_DIR` with a `..` component still installs, and the `..` is
/// taken as the kernel takes it for cargo (through a symbolic link, never by text): the root half
/// gets the directory cargo really wrote, never the lexical one.
#[test]
fn test_srp8_absolute_target_dir_with_dot_dot_reaches_the_system_phase() {
    let mut h = Harness::new("srp_abs_target");
    let base = fs::canonicalize(&h.dir).unwrap();
    let value = format!("{}/repo/../plain-target", base.display());
    srp_run_with_target_dir(&mut h, &value, &base.join("plain-target/release"));

    let mut h = Harness::new("srp_link_target");
    let base = fs::canonicalize(&h.dir).unwrap();
    fs::create_dir_all(base.join("deep/sub")).unwrap();
    std::os::unix::fs::symlink(base.join("deep/sub"), base.join("link")).unwrap();
    let value = format!("{}/link/../linked-target", base.display());
    srp_run_with_target_dir(&mut h, &value, &base.join("deep/linked-target/release"));
    assert!(
        !base.join("linked-target").exists(),
        "the lexical resolution of the `..` is never used"
    );
}

/// SRP9: the root half invoked directly still refuses an `--artifact-dir` with a `..`
/// component, with exit 2 and before any change.
#[test]
fn test_srp9_system_phase_refuses_a_dot_dot_artifact_dir() {
    let mut h = Harness::new("srp_root_dot_dot");
    h.set_env("STUB_AS_ROOT", "1");
    let raw = format!("{}/target/../target/release", srp_lexical(&h.repo));
    let before = h.snapshot();
    let out = h.run(&[
        "--system-phase",
        "--yes",
        "--user",
        "alice",
        "--artifact-dir",
        &raw,
        "--local-seat",
        "no",
        "--distro",
        "arch",
    ]);
    let t = text(&out);
    assert_eq!(code(&out), 2, "{t}");
    assert!(
        t.contains("--artifact-dir must not contain a '..' component"),
        "{t}"
    );
    let lines = h.log_lines();
    assert!(
        lines.iter().all(|l| is_read_only_call(l)),
        "a refused root half changes nothing: {lines:?}"
    );
    assert_eq!(before, h.snapshot(), "a refused root half writes nothing");
}

/// SRP10: the documentation and the traceability record the resolution of a `CARGO_TARGET_DIR`
/// with `..` and the refusal kept for a direct `--artifact-dir`.
#[test]
fn test_srp10_artifact_dir_resolution_documented() {
    let pkg = read_repo("Docs/PACKAGING_AND_PROVISIONING.md");
    let flat: String = pkg.split_whitespace().collect::<Vec<_>>().join(" ");
    let para = flat
        .split("**Read paths (GitHub #455).**")
        .nth(1)
        .expect("read-paths paragraph");
    let para = &para[..para.find(" | ").unwrap_or(para.len())];
    for needle in ["`CARGO_TARGET_DIR`", "`pwd -P`", "`--artifact-dir`"] {
        assert!(para.contains(needle), "read-paths paragraph: {needle}");
    }
    let matrix = read_repo("AI/VERIFICATION_MATRIX.md");
    for row in 7..=10 {
        assert!(
            matrix.contains(&format!("| SRP{row} |")),
            "matrix row SRP{row}"
        );
    }
    let walkthrough = read_repo("AI/walkthroughs/244_setup_read_paths.md");
    assert!(walkthrough.contains("CARGO_TARGET_DIR"), "walkthrough 244");
}

// ---------------------------------------------------------------------------
// PSS12 — the push sender as a system service (ADR 2026-10-23 "Push Sender as a Sandboxed
// System Service"; spec AI/architect_spec_push_sender_system_service.md §7). New functions
// only: the SET contracts above are unchanged.
// ---------------------------------------------------------------------------

/// PSS12: with push on, the root half enables `soos-push-sender@<uid>.socket` (system manager)
/// after install.sh; the companion verifies that socket as the user and never enables or starts
/// the user unit `soos-push-sender.service`.
#[test]
fn test_pss12_setup_enables_the_system_sender_not_the_user_unit() {
    let h = Harness::new("pss_sys");
    let uid = real_uid();
    let socket = format!("soos-push-sender@{uid}.socket");
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let lines = h.log_lines();
    let install = find_line(&lines, &["uid=0 install.sh"]);
    let enable = find_line(&lines, &["uid=0 systemctl enable --now", &socket]);
    let remote_install = find_line(&lines, &[&format!("uid={uid} install_remote.sh")]);
    assert!(
        install < enable && enable < remote_install,
        "the root half enables the socket after install.sh and before the companion"
    );
    find_line(
        &lines,
        &[&format!("uid={uid} systemctl is-active"), &socket],
    );
    assert!(
        !has_line(&lines, &["systemctl --user enable", "soos-push-sender"]),
        "the user unit is never enabled: {lines:?}"
    );
    assert!(
        !has_line(&lines, &["systemctl --user start", "soos-push-sender"]),
        "the user unit is never started: {lines:?}"
    );
    assert!(!h.user_unit("soos-push-sender.service", "enabled"));
    assert!(!h.user_unit("soos-push-sender.service", "active"));
    assert!(h.user_unit("soos-remote.service", "active"));
    let t = text(&out);
    assert!(
        t.contains(&format!("[DONE] push sender: {socket} enabled")),
        "{t}"
    );
}

/// PSS12: push off: no instance is enabled.
#[test]
fn test_pss12_push_off_enables_no_system_sender() {
    let h = Harness::new("pss_off");
    let out = h.run_companion(&["--push", "off"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let lines = h.log_lines();
    assert!(
        !has_line(&lines, &["systemctl enable", "soos-push-sender@"]),
        "{lines:?}"
    );
    assert!(!has_line(&lines, &["--push-sender"]), "{lines:?}");
}

/// PSS12: a legacy user unit enabled by an earlier setup is disabled and stopped (migration);
/// the companion succeeds.
#[test]
fn test_pss12_setup_disables_a_legacy_user_unit() {
    let h = Harness::new("pss_legacy");
    let uid = real_uid();
    for what in ["enabled", "active"] {
        fs::write(
            h.state
                .join(format!("user_soos-push-sender.service_{what}")),
            "",
        )
        .unwrap();
    }
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let lines = h.log_lines();
    find_line(
        &lines,
        &[&format!(
            "uid={uid} systemctl --user disable --now soos-push-sender.service"
        )],
    );
    assert!(!h.user_unit("soos-push-sender.service", "enabled"));
    assert!(!h.user_unit("soos-push-sender.service", "active"));
    assert!(
        text(&out).contains("soos-push-sender.service disabled"),
        "{}",
        text(&out)
    );
}

/// PSS12: a companion failure after the migration re-enables the legacy user unit (the rollback
/// restores the state before the run).
#[test]
fn test_pss12_rollback_restores_the_legacy_user_unit() {
    let mut h = Harness::new("pss_legacy_rb");
    for what in ["enabled", "active"] {
        fs::write(
            h.state
                .join(format!("user_soos-push-sender.service_{what}")),
            "",
        )
        .unwrap();
    }
    h.set_env("STUB_USER_FAIL_UNIT", "soos-remote.service");
    let out = h.run_companion(&[]);
    assert_eq!(code(&out), 5, "{}", text(&out));
    assert!(h.user_unit("soos-push-sender.service", "enabled"));
    assert!(h.user_unit("soos-push-sender.service", "active"));
}

/// PSS12: the dry run plans the system sender and writes nothing.
#[test]
fn test_pss12_dry_run_plans_the_system_sender_without_change() {
    let h = Harness::new("pss_dry");
    let uid = real_uid();
    let before = h.snapshot();
    let out = h.run(&["--dry-run", "--tailscale-login", LOGIN, "--rp-id", RP_ID]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    assert!(
        t.contains(&format!(
            "[PLAN] push sender: will run - enable soos-push-sender@{uid}.socket"
        )),
        "{t}"
    );
    let lines = h.log_lines();
    assert!(
        lines.iter().all(|l| is_read_only_call(l)),
        "dry-run must only query: {lines:?}"
    );
    assert_eq!(before, h.snapshot(), "dry-run must write nothing");
}
