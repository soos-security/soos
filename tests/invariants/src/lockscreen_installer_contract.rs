//! Installer opt-in of the lock-screen adapters (GitHub #432 = PR 6 of #352; ADR 2026-10-18
//! "Installer Opt-In of the Lock-Screen Adapters and the Shipped Widget Build"; spec
//! `AI/architect_spec_lockscreen_installer.md`; rows LKI1-LKI10, LKI11 and LKI12 being the
//! owner's hardware checks).
//!
//! - `scripts/install.sh` stages `soos-lockstatus` and `soos-lock-widget` (optional artifacts),
//!   the GNOME Shell system extension and the hyprlock snippet, journaled and rolled back.
//! - `scripts/uninstall.sh` removes exactly those files and never follows a symbolic link.
//! - `scripts/setup.sh` gains `--lock-widget on|off` (default on, the kill switch
//!   `<sysconfdir>/soos/lockwidget.disable`), detects the lockers read-only, runs
//!   `gnome-extensions enable soos-lockscreen@soos` only as the session owner from a local GNOME
//!   seat and only prints the swaylock-plugin and hyprlock lines.
//!
//! The setup contracts run the real `setup.sh` against a fake root in a fake checkout with stub
//! commands and a restricted `PATH`, like `setup_contract.rs` (no root needed).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const UUID: &str = "soos-lockscreen@soos";
const EXT_FILES: [&str; 3] = ["metadata.json", "extension.js", "stylesheet.css"];
const EXT_STAGE_DIR: &str = "usr/share/gnome-shell/extensions/soos-lockscreen@soos";
const HYPRLOCK_STAGE: &str = "usr/share/soos/lockscreen/hyprlock-soos.conf";
const ADAPTERS: [&str; 2] = ["soos-lockstatus", "soos-lock-widget"];
const REQUIRED: [&str; 5] = [
    "soos-daemon",
    "soos-admin",
    "soos-enroll",
    "soos-gui",
    "libpam_soos.so",
];

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

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soos_lki_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path)
        .unwrap_or_else(|e| panic!("metadata {}: {e}", path.display()))
        .permissions()
        .mode()
        & 0o7777
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

/// Writes stub release artifacts; the two lock-screen adapters only when `adapters`.
fn artifacts(dir: &Path, adapters: bool) {
    fs::create_dir_all(dir).expect("artifact dir");
    let mut names: Vec<&str> = REQUIRED.to_vec();
    if adapters {
        names.extend(ADAPTERS);
    }
    for name in names {
        let path = dir.join(name);
        fs::write(&path, format!("fixture:{name}\n")).expect("artifact");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

fn run_install(stage: &Path, art: &Path, extra: &[&OsStr]) -> Output {
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
        ])
        .args(extra)
        .output()
        .expect("run install.sh")
}

fn run_uninstall(stage: &Path) -> Output {
    Command::new("bash")
        .arg(workspace_root().join("scripts/uninstall.sh"))
        .args([
            OsStr::new("--destdir"),
            stage.as_os_str(),
            OsStr::new("--keep-data"),
            OsStr::new("--skip-systemd"),
        ])
        .output()
        .expect("run uninstall.sh")
}

/// Every entry below `dir` (relative path, type, mode, content), sorted.
fn tree(dir: &Path) -> BTreeMap<String, String> {
    fn walk(base: &Path, dir: &Path, map: &mut BTreeMap<String, String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).expect("metadata");
            let key = path
                .strip_prefix(base)
                .expect("relative")
                .display()
                .to_string();
            let mode = meta.permissions().mode() & 0o7777;
            if meta.file_type().is_symlink() {
                map.insert(
                    key,
                    format!("link {:?}", fs::read_link(&path).expect("link")),
                );
            } else if meta.is_dir() {
                map.insert(key, format!("dir {mode:o}"));
                walk(base, &path, map);
            } else {
                map.insert(
                    key,
                    format!("file {mode:o} {:?}", fs::read(&path).expect("read")),
                );
            }
        }
    }
    let mut map = BTreeMap::new();
    walk(dir, dir, &mut map);
    map
}

// ---------------------------------------------------------------------------
// install.sh and uninstall.sh
// ---------------------------------------------------------------------------

/// LKI1: with the adapters built, install.sh stages both programs 0755 in `<prefix>/bin` (the
/// `/usr/bin` the GNOME extension trusts), the three extension files 0644 byte-equal to
/// `packaging/gnome-shell/` in a 0755 directory, and the hyprlock snippet 0644.
#[test]
fn test_lki1_install_stages_the_lock_screen_adapters() {
    let dir = scratch("stage");
    let (stage, art) = (dir.join("stage"), dir.join("art"));
    artifacts(&art, true);
    let out = run_install(&stage, &art, &[OsStr::new("--skip-models")]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    for name in ADAPTERS {
        let path = stage.join("usr/bin").join(name);
        assert_eq!(mode_of(&path), 0o755, "{name}");
        assert_eq!(
            fs::read(&path).unwrap(),
            fs::read(art.join(name)).unwrap(),
            "{name}"
        );
    }
    let ext = stage.join(EXT_STAGE_DIR);
    assert_eq!(mode_of(&ext), 0o755);
    let mut staged: Vec<String> = fs::read_dir(&ext)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    staged.sort();
    let mut expected: Vec<String> = EXT_FILES.iter().map(|f| (*f).to_string()).collect();
    expected.sort();
    assert_eq!(staged, expected, "exactly the three extension files");
    for file in EXT_FILES {
        assert_eq!(mode_of(&ext.join(file)), 0o644, "{file}");
        assert_eq!(
            fs::read(ext.join(file)).unwrap(),
            fs::read(
                workspace_root()
                    .join("packaging/gnome-shell")
                    .join(UUID)
                    .join(file)
            )
            .unwrap(),
            "{file} copied verbatim"
        );
    }
    let snippet = stage.join(HYPRLOCK_STAGE);
    assert_eq!(mode_of(&snippet), 0o644);
    assert_eq!(
        fs::read_to_string(&snippet).unwrap(),
        read_repo("packaging/lockscreen/hyprlock-soos.conf")
    );
    assert!(
        !stage.join("etc/soos/lockwidget.disable").exists(),
        "the installer never turns the widget off: default on"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// LKI2: the adapters are optional artifacts: an artifact set without them still installs
/// (exit 0, no INCOMPLETE banner), names the skipped adapters, and still ships the extension
/// and the snippet; the five required artifacts stay mandatory.
#[test]
fn test_lki2_adapters_are_optional_artifacts() {
    let dir = scratch("optional");
    let (stage, art) = (dir.join("stage"), dir.join("art"));
    artifacts(&art, false);
    let out = run_install(&stage, &art, &[OsStr::new("--skip-models")]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    assert!(
        t.contains("soos-lockstatus") && t.contains("soos-lock-widget") && t.contains("skipped"),
        "{t}"
    );
    assert!(!t.contains("INCOMPLETE"), "{t}");
    for name in ADAPTERS {
        assert!(!stage.join("usr/bin").join(name).exists(), "{name}");
    }
    assert!(stage.join(EXT_STAGE_DIR).join("metadata.json").is_file());
    assert!(stage.join(HYPRLOCK_STAGE).is_file());

    let stage2 = dir.join("stage2");
    fs::remove_file(art.join("soos-gui")).unwrap();
    let out = run_install(&stage2, &art, &[OsStr::new("--skip-models")]);
    assert_ne!(code(&out), 0, "a required artifact stays mandatory");
    assert!(tree(&stage2).is_empty(), "preflight failure writes nothing");
    let _ = fs::remove_dir_all(&dir);
}

/// LKI3: a late failure (model verification) rolls back the adapters too: the stage is left
/// exactly as it was, and an existing older extension file gets its previous content back.
#[test]
fn test_lki3_late_failure_rolls_back_the_adapters() {
    let dir = scratch("rollback");
    let (stage, art) = (dir.join("stage"), dir.join("art"));
    artifacts(&art, true);
    let previous = stage.join(EXT_STAGE_DIR).join("extension.js");
    fs::create_dir_all(previous.parent().unwrap()).unwrap();
    fs::write(&previous, "// previous version\n").unwrap();
    fs::set_permissions(&previous, fs::Permissions::from_mode(0o644)).unwrap();
    let before = tree(&stage);

    let model = dir.join("model.onnx");
    fs::write(&model, b"not-the-attested-model").unwrap();
    let manifest = dir.join("manifest.toml");
    fs::write(
        &manifest,
        format!(
            "[manifest]\nversion = \"2.0.0\"\n\n[models.bad_model]\nid = \"bad_model\"\n\
             filename = \"bad_model.onnx\"\nsha256 = \"{}\"\nlicense = \"MIT\"\n\
             source_url = \"file://{}\"\n",
            "0".repeat(64),
            model.display()
        ),
    )
    .unwrap();
    let out = run_install(
        &stage,
        &art,
        &[OsStr::new("--manifest"), manifest.as_os_str()],
    );
    assert_ne!(code(&out), 0, "{}", text(&out));
    assert!(text(&out).contains("rolled back"), "{}", text(&out));
    assert_eq!(tree(&stage), before, "every adapter change rolled back");
    let _ = fs::remove_dir_all(&dir);
}

/// LKI4: uninstall.sh removes both programs, exactly the three extension files and its
/// directory, the snippet and the emptied `share/soos`; it keeps a file it did not install
/// and never follows a symbolic link in place of the extension directory.
#[test]
fn test_lki4_uninstall_removes_the_adapters_without_following_links() {
    let dir = scratch("uninstall");
    let (stage, art) = (dir.join("stage"), dir.join("art"));
    artifacts(&art, true);
    let out = run_install(&stage, &art, &[OsStr::new("--skip-models")]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let out = run_uninstall(&stage);
    assert_eq!(code(&out), 0, "{}", text(&out));
    for name in ADAPTERS {
        assert!(!stage.join("usr/bin").join(name).exists(), "{name}");
    }
    assert!(!stage.join(EXT_STAGE_DIR).exists(), "extension removed");
    assert!(!stage.join(HYPRLOCK_STAGE).exists(), "snippet removed");
    assert!(
        !stage.join("usr/share/soos").exists(),
        "empty share/soos removed"
    );

    // A file soos did not install keeps the directory.
    let out = run_install(&stage, &art, &[OsStr::new("--skip-models")]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let foreign = stage.join(EXT_STAGE_DIR).join("local.css");
    fs::write(&foreign, "/* mine */\n").unwrap();
    let out = run_uninstall(&stage);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(foreign.is_file(), "a foreign file is never removed");
    for file in EXT_FILES {
        assert!(!stage.join(EXT_STAGE_DIR).join(file).exists(), "{file}");
    }

    // A symbolic link in place of the directory is not followed.
    let stage = dir.join("linked");
    let elsewhere = dir.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    for file in EXT_FILES {
        fs::write(elsewhere.join(file), "keep\n").unwrap();
    }
    let ext = stage.join(EXT_STAGE_DIR);
    fs::create_dir_all(ext.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &ext).unwrap();
    let out = run_uninstall(&stage);
    assert_eq!(code(&out), 0, "{}", text(&out));
    for file in EXT_FILES {
        assert!(elsewhere.join(file).is_file(), "link target untouched");
    }
    assert!(fs::symlink_metadata(&ext).unwrap().file_type().is_symlink());

    // install.sh refuses to write through such a link (and rolls back).
    artifacts(&art, true);
    let out = run_install(&stage, &art, &[OsStr::new("--skip-models")]);
    assert_ne!(code(&out), 0, "{}", text(&out));
    for file in EXT_FILES {
        assert_eq!(fs::read_to_string(elsewhere.join(file)).unwrap(), "keep\n");
    }
    let _ = fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// setup.sh harness (fake root, fake checkout, stub commands)
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
keep=(PATH="${PATH}" LC_ALL=C HOME="${STUB_ROOT_HOME}" STUB_AS_ROOT=1 SUDO_USER=alice SUDO_UID="${STUB_UID}")
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
        if [[ -e "${STUB_STATE}/in_group" ]]; then echo "alice wheel soos"; else echo "alice wheel"; fi ;;
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
    show-user) echo "Display=c2" ;;
    show-session) echo "Remote=${STUB_REMOTE:-no}"; echo "Seat=seat0" ;;
    *) exit 1 ;;
esac
"#;

const STUB_SYSTEMCTL: &str = r#"
case "${1:-}" in
    is-enabled) [[ -e "${STUB_STATE}/unit_enabled" ]] ;;
    is-active) [[ -e "${STUB_STATE}/unit_active" ]] ;;
    stop) rm -f "${STUB_STATE}/unit_active" ;;
    start) touch "${STUB_STATE}/unit_active" ;;
    *) exit 0 ;;
esac
"#;

const STUB_PACMAN: &str = r#"
[[ "${1:-}" == -Q ]] && exit 0
exit 1
"#;

const STUB_CARGO: &str = r#"
out="${CARGO_TARGET_DIR:?}/release"
mkdir -p "${out}"
printf 'daemon\n' > "${out}/soos-daemon"
cp "${STUB_FAKES}/soos-admin" "${out}/soos-admin"
printf 'enroll\n' > "${out}/soos-enroll"
printf 'gui\n' > "${out}/soos-gui"
printf 'pam\n' > "${out}/libpam_soos.so"
printf 'lockstatus\n' > "${out}/soos-lockstatus"
printf 'widget\n' > "${out}/soos-lock-widget"
chmod 0755 "${out}"/soos-*
"#;

/// `gnome-extensions`: `list --enabled` prints the enabled set; `enable` fails with
/// `STUB_GEXT_ENABLE_RC` (the shell has not loaded the extension yet), else records it.
const STUB_GNOME_EXTENSIONS: &str = r#"
case "${1:-}" in
    list) [[ -e "${STUB_STATE}/gext_enabled" ]] && echo "soos-lockscreen@soos"; exit 0 ;;
    enable)
        [[ "${STUB_GEXT_ENABLE_RC:-0}" == 0 ]] || exit "${STUB_GEXT_ENABLE_RC}"
        [[ "${2:-}" == "soos-lockscreen@soos" ]] || exit 2
        touch "${STUB_STATE}/gext_enabled" ;;
    *) exit 1 ;;
esac
"#;

const FAKE_SOOS_ADMIN: &str = r#"
case "${1:-}" in
    add-user) touch "${STUB_STATE}/in_group" ;;
    *) exit 0 ;;
esac
"#;

/// The face install is not under test: the stub copies what the real install.sh copies.
const STUB_INSTALL_SH: &str = r#"
artifacts="" prefix=/usr sysconfdir=/etc localstatedir=/var pamdir=/usr/lib/security
while [[ $# -gt 0 ]]; do
    case "$1" in
        --artifact-dir) artifacts="$2"; shift 2 ;;
        --prefix) prefix="$2"; shift 2 ;;
        --sysconfdir) sysconfdir="$2"; shift 2 ;;
        --localstatedir) localstatedir="$2"; shift 2 ;;
        --pam-dir) pamdir="$2"; shift 2 ;;
        *) shift ;;
    esac
done
repo="$(cd "$(dirname "$0")/.." && pwd)"
ext="${prefix}/share/gnome-shell/extensions/soos-lockscreen@soos"
mkdir -p "${prefix}/bin" "${prefix}/libexec/soos" "${pamdir}" "${sysconfdir}/systemd/system" \
    "${localstatedir}/lib/soos/models" "${localstatedir}/lib/soos/biometrics" "${prefix}/share/soos/pam" \
    "${ext}" "${prefix}/share/soos/lockscreen"
cp "${repo}/scripts/provision_master_key.sh" "${prefix}/libexec/soos/provision-master-key"
[[ -e "${localstatedir}/lib/soos/master.key" ]] || printf 'k' > "${localstatedir}/lib/soos/master.key"
cp "${repo}/packaging/pam/arch/system-auth.snippet" "${prefix}/share/soos/pam/system-auth.snippet"
touch "${STUB_STATE}/group_soos"
cp "${artifacts}/soos-daemon" "${prefix}/libexec/soos/soos-daemon"
for b in soos-admin soos-enroll soos-gui soos-lockstatus soos-lock-widget; do cp "${artifacts}/${b}" "${prefix}/bin/${b}"; done
cp "${artifacts}/libpam_soos.so" "${pamdir}/pam_soos.so"
cp "${repo}/packaging/soos-daemon.service" "${sysconfdir}/systemd/system/soos-daemon.service"
cp "${repo}/models/manifest.toml" "${localstatedir}/lib/soos/models/manifest.toml"
cp "${repo}/packaging/gnome-shell/soos-lockscreen@soos/"* "${ext}/"
cp "${repo}/packaging/lockscreen/hyprlock-soos.conf" "${prefix}/share/soos/lockscreen/"
touch "${STUB_STATE}/unit_enabled" "${STUB_STATE}/unit_active"
"#;

const STUB_EXIT_0: &str = "\nexit 0\n";

const STUB_MODELS_SH: &str = r#"
[[ " $* " == *" --check-only "* ]] || exit 64
exit 0
"#;

struct Harness {
    dir: PathBuf,
    repo: PathBuf,
    root: PathBuf,
    home: PathBuf,
    state: PathBuf,
    log: PathBuf,
    env: Vec<(String, String)>,
}

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

fn real_uid() -> String {
    let out = Command::new("id").arg("-u").output().expect("id -u");
    let uid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(
        !uid.is_empty() && uid != "0",
        "runs as a normal user: {uid}"
    );
    uid
}

impl Harness {
    /// `lockers`: the locker programs present on the fake host (stubs that log if run).
    fn new(tag: &str, lockers: &[&str]) -> Self {
        let dir = scratch(&format!("setup_{tag}"));
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
        fs::create_dir_all(dir.join("roothome")).expect("roothome");
        fs::write(&log, "").expect("log");
        let real = workspace_root();
        let mut copied = vec![
            "scripts/setup.sh".to_string(),
            "scripts/check_build_deps.sh".to_string(),
            "scripts/provision_master_key.sh".to_string(),
            "packaging/soos-daemon.service".to_string(),
            "packaging/pam/arch/system-auth.snippet".to_string(),
            "packaging/lockscreen/hyprlock-soos.conf".to_string(),
            "models/manifest.toml".to_string(),
        ];
        for file in EXT_FILES {
            copied.push(format!("packaging/gnome-shell/{UUID}/{file}"));
        }
        for rel in &copied {
            let dst = repo.join(rel);
            fs::create_dir_all(dst.parent().unwrap()).unwrap();
            fs::copy(real.join(rel), &dst).unwrap_or_else(|e| panic!("copy {rel}: {e}"));
        }
        let shebang = format!("#!{}\n", bash_path().display());
        let uid = real_uid();
        let baked = format!(
            ": \"${{STUB_LOG:={}}}\" \"${{STUB_STATE:={}}}\" \"${{STUB_HOME:={}}}\" \"${{STUB_FAKES:={}}}\" \"${{STUB_UID:={uid}}}\"\nexport STUB_LOG STUB_STATE STUB_HOME STUB_FAKES STUB_UID\n",
            log.display(),
            state.display(),
            home.display(),
            fakes.display()
        );
        let stub = |body: &str| format!("{shebang}{baked}{STUB_PROLOGUE}{body}");
        write_exec(&repo.join("scripts/install.sh"), &stub(STUB_INSTALL_SH));
        write_exec(
            &repo.join("scripts/wait_daemon_ready.sh"),
            &stub(STUB_EXIT_0),
        );
        write_exec(
            &repo.join("scripts/download_models.sh"),
            &stub(STUB_MODELS_SH),
        );
        let mut commands: Vec<(&str, &str)> = vec![
            ("sudo", STUB_SUDO),
            ("runuser", STUB_RUNUSER),
            ("loginctl", STUB_LOGINCTL),
            ("systemctl", STUB_SYSTEMCTL),
            ("pacman", STUB_PACMAN),
            ("cargo", STUB_CARGO),
            ("gpasswd", STUB_EXIT_0),
            ("gnome-extensions", STUB_GNOME_EXTENSIONS),
        ];
        for locker in lockers {
            commands.push((locker, STUB_EXIT_0));
        }
        for (name, body) in commands {
            write_exec(&stubs.join(name), &stub(body));
        }
        write_exec(&stubs.join("id"), &format!("{shebang}{baked}{STUB_ID}"));
        write_exec(
            &stubs.join("getent"),
            &format!("{shebang}{baked}{STUB_GETENT}"),
        );
        write_exec(&fakes.join("soos-admin"), &stub(FAKE_SOOS_ADMIN));
        let path_var = std::env::var_os("PATH").expect("PATH");
        for tool in SAFE_TOOLS {
            if let Some(found) = std::env::split_paths(&path_var)
                .map(|d| d.join(tool))
                .find(|p| is_executable(p))
            {
                std::os::unix::fs::symlink(&found, tools.join(tool)).expect("symlink tool");
            }
        }
        fs::create_dir_all(root.join("run/systemd/system")).unwrap();
        fs::create_dir_all(root.join("etc/systemd/system")).unwrap();
        fs::write(
            root.join("etc/os-release"),
            "NAME=\"Arch Linux\"\nPRETTY_NAME=\"Arch Linux\"\nID=arch\n",
        )
        .unwrap();
        let env = vec![
            (
                "PATH".to_string(),
                format!("{}:{}", stubs.display(), tools.display()),
            ),
            ("HOME".to_string(), home.display().to_string()),
            ("LC_ALL".to_string(), "C".to_string()),
            ("TMPDIR".to_string(), dir.join("tmp").display().to_string()),
            ("STUB_LOG".to_string(), log.display().to_string()),
            ("STUB_STATE".to_string(), state.display().to_string()),
            ("STUB_HOME".to_string(), home.display().to_string()),
            ("STUB_FAKES".to_string(), fakes.display().to_string()),
            ("STUB_UID".to_string(), uid),
            (
                "STUB_ROOT_HOME".to_string(),
                dir.join("roothome").display().to_string(),
            ),
            (
                "XDG_CURRENT_DESKTOP".to_string(),
                "ubuntu:GNOME".to_string(),
            ),
        ];
        Self {
            dir,
            repo,
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

    fn run(&self, args: &[&str]) -> Output {
        fs::write(&self.log, "").expect("reset log");
        let mut cmd = Command::new(bash_path());
        cmd.arg(self.repo.join("scripts/setup.sh"))
            .args([
                "--prefix",
                &self.root.join("usr").display().to_string(),
                "--sysconfdir",
                &self.root.join("etc").display().to_string(),
                "--localstatedir",
                &self.root.join("var").display().to_string(),
                "--runstatedir",
                &self.root.join("run").display().to_string(),
                "--pam-dir",
                &self.root.join("usr/lib/security").display().to_string(),
                "--no-remote",
                "--no-enroll",
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

    fn log_lines(&self) -> Vec<String> {
        fs::read_to_string(&self.log)
            .expect("log")
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn flag(&self) -> PathBuf {
        self.root.join("etc/soos/lockwidget.disable")
    }

    fn state_file(&self) -> String {
        fs::read_to_string(self.root.join("var/lib/soos/state/setup.json")).expect("state")
    }

    fn snapshot(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        for base in [&self.root, &self.home, &self.repo, &self.dir.join("tmp")] {
            for (k, v) in tree(base) {
                map.insert(format!("{}:{k}", base.display()), v);
            }
        }
        map
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn has_line(lines: &[String], needles: &[&str]) -> bool {
    lines.iter().any(|l| needles.iter().all(|n| l.contains(n)))
}

fn position(lines: &[String], needles: &[&str]) -> usize {
    lines
        .iter()
        .position(|l| needles.iter().all(|n| l.contains(n)))
        .unwrap_or_else(|| panic!("no log line with {needles:?}:\n{}", lines.join("\n")))
}

/// A call that never runs a locker (the lockers are only looked up with `command -v`).
fn assert_no_locker_ran(lines: &[String]) {
    for locker in ["swaylock-plugin", "hyprlock", "gnome-shell"] {
        assert!(
            !lines
                .iter()
                .any(|l| l.split_whitespace().nth(1) == Some(locker)),
            "{locker} must never be run: {lines:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// setup.sh contracts
// ---------------------------------------------------------------------------

/// LKI5: `--lock-widget` is parsed and documented; an invalid value exits 2 before anything
/// runs; the root half receives it as an argument.
#[test]
fn test_lki5_lock_widget_option_is_parsed_and_documented() {
    let script = read_repo("scripts/setup.sh");
    assert!(script.lines().any(|l| l.trim() == "--lock-widget)"));
    assert!(script.contains("SYSTEM_ARGS+=(--lock-widget \"${LOCK_WIDGET}\")"));
    let h = Harness::new("help", &[]);
    let out = h.run(&["--help"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let help = text(&out);
    assert!(help.contains("--lock-widget <on|off>"), "{help}");
    assert!(help.contains("lockwidget.disable"), "{help}");
    let before = h.snapshot();
    let out = h.run(&["--yes", "--lock-widget", "maybe"]);
    assert_eq!(code(&out), 2, "{}", text(&out));
    assert!(h.log_lines().is_empty(), "nothing runs");
    assert_eq!(before, h.snapshot());
}

/// LKI6: default on and the kill switch: a default run creates no flag and writes the
/// format-1 state with no lock-widget key; `--lock-widget off` creates the 0644 flag and records
/// it (sticky), a second identical run changes nothing, a plain run keeps it, and
/// `--lock-widget on` removes it.
#[test]
fn test_lki6_lock_widget_flag_default_on_toggle_and_state() {
    let h = Harness::new("flag", &[]);
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(!h.flag().exists(), "default on: no flag");
    assert!(
        !h.state_file().contains("lock_widget"),
        "{}",
        h.state_file()
    );
    assert!(text(&out).contains("[PLAN] lock widget: already done - on (default"));

    let out = h.run(&["--yes", "--lock-widget", "off"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_eq!(mode_of(&h.flag()), 0o644);
    assert!(fs::symlink_metadata(h.flag()).unwrap().is_file());
    let state = h.state_file();
    assert!(
        state.contains("  \"lock_widget_flag_created_by_setup\": true,\n"),
        "{state}"
    );
    assert!(state.contains("  \"lock_widget\": \"off\",\n"), "{state}");
    assert!(state.ends_with("  \"enrolled\": false\n}\n"), "{state}");

    let before = h.snapshot();
    let out = h.run(&["--yes", "--lock-widget", "off"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(text(&out).contains("[PLAN] lock widget: already done - off"));
    assert_eq!(before, h.snapshot(), "second identical run changes nothing");

    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0);
    assert!(h.flag().exists(), "kept without --lock-widget on");
    assert!(text(&out).contains("--lock-widget on"));

    let out = h.run(&["--yes", "--lock-widget", "on"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(!h.flag().exists());
    let state = h.state_file();
    assert!(!state.contains("\"lock_widget\":"), "{state}");
    assert!(
        state.contains("\"lock_widget_flag_created_by_setup\": true"),
        "sticky record: {state}"
    );

    // An invalid lock-widget value in the state file is refused before the system install.
    let path = h.root.join("var/lib/soos/state/setup.json");
    let bad = h.state_file().replace(
        "  \"enrolled\"",
        "  \"lock_widget\": \"$(reboot)\",\n  \"enrolled\"",
    );
    fs::write(&path, bad).unwrap();
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 4, "{}", text(&out));
    assert!(!has_line(&h.log_lines(), &["install.sh"]));
}

/// LKI7: detection and plan are read-only: the dry run names the lockers found, prints the
/// exact swaylock-plugin command and hyprlock append line, and plans the GNOME enable as the
/// user; it writes nothing and runs no locker and no `gnome-extensions`.
#[test]
fn test_lki7_dry_run_detects_lockers_and_prints_exact_lines() {
    let h = Harness::new("dryrun", &["swaylock-plugin", "hyprlock", "gnome-shell"]);
    let before = h.snapshot();
    let out = h.run(&["--dry-run"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    let usr = h.root.join("usr");
    for needle in [
        "[DETECT] lock screen: swaylock-plugin hyprlock gnome-shell".to_string(),
        format!(
            "[PLAN] lock screen (swaylock-plugin): printed only - start the locker with: swaylock-plugin --command '{}/bin/soos-lock-widget'",
            usr.display()
        ),
        format!(
            "[PLAN] lock screen (hyprlock): printed only - append the label: cat {}/share/soos/lockscreen/hyprlock-soos.conf >> ~/.config/hypr/hyprlock.conf",
            usr.display()
        ),
        "[PLAN] lock screen (GNOME): will run - gnome-extensions enable soos-lockscreen@soos as alice".to_string(),
    ] {
        assert!(t.contains(&needle), "missing {needle}:\n{t}");
    }
    let lines = h.log_lines();
    assert_no_locker_ran(&lines);
    assert!(!has_line(&lines, &["gnome-extensions"]), "{lines:?}");
    assert_eq!(before, h.snapshot(), "dry run writes nothing");

    let h = Harness::new("nolocker", &[]);
    let out = h.run(&["--dry-run"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    assert!(
        t.contains("[DETECT] lock screen: none of swaylock-plugin, hyprlock, gnome-shell found")
    );
    assert!(t.contains("[PLAN] lock screen: skipped (no supported locker found)"));
}

/// LKI8: from a local GNOME seat, the user phase runs `gnome-extensions enable
/// soos-lockscreen@soos` as the user, after the root half, once (a second run finds it
/// enabled); a refused enable (shell not reloaded) is a printed step, never a failure.
#[test]
fn test_lki8_gnome_enable_runs_as_the_user_after_the_system_phase() {
    let mut h = Harness::new("gnome", &["gnome-shell"]);
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let lines = h.log_lines();
    let system = position(&lines, &["uid=0 install.sh"]);
    let enable = position(
        &lines,
        &["uid=1000 gnome-extensions enable soos-lockscreen@soos"],
    );
    assert!(system < enable, "{lines:?}");
    assert!(
        !has_line(&lines, &["uid=0 gnome-extensions"]),
        "never as root"
    );
    assert!(
        text(&out).contains("[DONE] lock screen (GNOME): soos-lockscreen@soos enabled for alice")
    );
    assert_no_locker_ran(&lines);

    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert!(text(&out).contains("[DONE] lock screen (GNOME): already done"));
    assert!(!has_line(&h.log_lines(), &["gnome-extensions enable"]));

    fs::remove_file(h.state.join("gext_enabled")).unwrap();
    h.set_env("STUB_GEXT_ENABLE_RC", "2");
    let out = h.run(&["--yes"]);
    assert_eq!(
        code(&out),
        0,
        "a refused enable never fails setup: {}",
        text(&out)
    );
    let t = text(&out);
    assert!(
        t.contains("log out and back in, then run: gnome-extensions enable soos-lockscreen@soos"),
        "{t}"
    );
}

/// LKI9: no enable without a local GNOME seat or with the widget off: a remote session, a
/// non-GNOME desktop and `--lock-widget off` print the command (or the off notice) and never
/// run `gnome-extensions enable`; setup never writes a user locker configuration.
#[test]
fn test_lki9_no_enable_off_seat_or_when_off_and_no_user_config_write() {
    let cases: [(&str, &str, &[&str], &str); 3] = [
        (
            "remote",
            "STUB_REMOTE",
            &[],
            "printed only (remote session or no local seat)",
        ),
        (
            "kde",
            "XDG_CURRENT_DESKTOP",
            &[],
            "printed only (not a GNOME session)",
        ),
        (
            "off",
            "",
            &["--lock-widget", "off"],
            "[PLAN] lock screen: skipped (lock widget off)",
        ),
    ];
    for (tag, key, args, needle) in cases {
        let mut h = Harness::new(tag, &["gnome-shell", "hyprlock", "swaylock-plugin"]);
        match key {
            "STUB_REMOTE" => h.set_env(key, "yes"),
            "XDG_CURRENT_DESKTOP" => h.set_env(key, "KDE"),
            _ => {}
        }
        let hypr = h.home.join(".config/hypr/hyprlock.conf");
        fs::create_dir_all(hypr.parent().unwrap()).unwrap();
        fs::write(&hypr, "# mine\n").unwrap();
        let home_before = tree(&h.home);
        let mut full = vec!["--yes"];
        full.extend_from_slice(args);
        let out = h.run(&full);
        assert_eq!(code(&out), 0, "{tag}: {}", text(&out));
        let t = text(&out);
        assert!(t.contains(needle), "{tag}: missing {needle}:\n{t}");
        let lines = h.log_lines();
        assert!(
            !has_line(&lines, &["gnome-extensions enable"]),
            "{tag}: {lines:?}"
        );
        assert_no_locker_ran(&lines);
        assert_eq!(
            tree(&h.home),
            home_before,
            "{tag}: the home is never written"
        );
        if tag == "off" {
            assert!(
                t.contains("do not read it: remove the --command argument or the label"),
                "{t}"
            );
            assert!(
                !t.contains("swaylock-plugin --command '"),
                "no locker line when off: {t}"
            );
        } else {
            assert!(
                t.contains("gnome-extensions enable soos-lockscreen@soos"),
                "{tag}: {t}"
            );
            assert!(t.contains("swaylock-plugin --command '"), "{tag}: {t}");
        }
    }
    let code_only: String = read_repo("scripts/setup.sh")
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in ["gsettings", "dconf "] {
        assert!(
            !code_only.contains(forbidden),
            "setup.sh must not contain {forbidden}"
        );
    }
}

// ---------------------------------------------------------------------------
// Packaging, build variant and documentation
// ---------------------------------------------------------------------------

/// LKI10: every installer path builds the one shipped widget variant (the workspace build of
/// the ADR) and ships the adapters: PKGBUILD and the RPM spec install and list them, the Debian
/// rules stage through install.sh; documentation, ADR, matrix and walkthrough are present and
/// the "later PR" placeholders are gone.
#[test]
fn test_lki10_packages_variant_and_documentation() {
    let pkgbuild = read_repo("packaging/arch/PKGBUILD");
    let spec = read_repo("packaging/rpm/soos.spec");
    let rules = read_repo("packaging/debian/rules");
    for (name, text) in [
        ("PKGBUILD", &pkgbuild),
        ("soos.spec", &spec),
        ("rules", &rules),
    ] {
        assert!(
            text.contains("cargo build --locked --release --workspace"),
            "{name} builds the workspace variant"
        );
    }
    assert!(read_repo("scripts/install.sh").contains("cargo build --release --locked --workspace"));
    assert!(rules.contains("bash scripts/install.sh"));
    for needle in [
        "target/release/soos-lockstatus",
        "target/release/soos-lock-widget",
        "packaging/gnome-shell/soos-lockscreen@soos",
        "packaging/lockscreen/hyprlock-soos.conf",
    ] {
        assert!(pkgbuild.contains(needle), "PKGBUILD: {needle}");
        assert!(spec.contains(needle), "soos.spec: {needle}");
    }
    for needle in [
        "%{_bindir}/soos-lockstatus",
        "%{_bindir}/soos-lock-widget",
        "%{_datadir}/gnome-shell/extensions/soos-lockscreen@soos",
        "%{_datadir}/soos/lockscreen/hyprlock-soos.conf",
    ] {
        assert!(spec.contains(needle), "soos.spec %files: {needle}");
    }

    let doc = read_repo("Docs/LOCKSCREEN_WIDGET.md");
    for needle in [
        "--lock-widget off",
        "gnome-extensions enable soos-lockscreen@soos",
        "/usr/share/soos/lockscreen/hyprlock-soos.conf",
        "Installer Opt-In of the Lock-Screen Adapters and the Shipped Widget Build",
    ] {
        assert!(doc.contains(needle), "LOCKSCREEN_WIDGET.md: {needle}");
    }
    for stale in ["later PR of #352", "The installer PR of #352 decides"] {
        assert!(
            !doc.contains(stale),
            "LOCKSCREEN_WIDGET.md still says {stale}"
        );
    }
    let pkg = read_repo("Docs/PACKAGING_AND_PROVISIONING.md");
    assert!(pkg.contains("--lock-widget") && pkg.contains("lock_widget_flag_created_by_setup"));
    let decisions = read_repo("AI/DECISIONS.md");
    assert!(decisions
        .contains("Installer Opt-In of the Lock-Screen Adapters and the Shipped Widget Build"));
    let matrix = read_repo("AI/VERIFICATION_MATRIX.md");
    for row in 1..=12 {
        assert!(
            matrix.contains(&format!("| LKI{row} |")),
            "matrix row LKI{row}"
        );
    }
    assert!(workspace_root()
        .join("AI/walkthroughs/233_lockscreen_installer.md")
        .is_file());
}

// ---------------------------------------------------------------------------
// Lock-widget flag read path and review follow-ups (GitHub #451, follow-up of PR #434; rows
// LWF1-LWF6)
// ---------------------------------------------------------------------------

/// The one lock-widget kill switch the GNOME extension reads (its `KILL_SWITCH_PATH`).
const READ_FLAG: &str = "/etc/soos/lockwidget.disable";

fn lines_of(out: &Output) -> Vec<String> {
    text(out).lines().map(str::to_string).collect()
}

/// A non-/etc `--sysconfdir` run never reports the lock-widget step as applied: no
/// `[DONE] lock widget` line, one `[SKIP] lock widget` line saying "not applied" with the
/// path the extension reads and the exact administrator command, and every lock-widget plan
/// line saying "not applied".
fn assert_lock_widget_not_applied(out: &Output, command: &str) {
    let lines = lines_of(out);
    let t = text(out);
    assert!(
        !lines.iter().any(|l| l.starts_with("[DONE] lock widget")),
        "a flag the extension never reads is never reported done:\n{t}"
    );
    let skips: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("[SKIP] lock widget: "))
        .collect();
    assert_eq!(skips.len(), 1, "one lock-widget phase line:\n{t}");
    for needle in ["not applied", READ_FLAG, command] {
        assert!(skips[0].contains(needle), "missing {needle}: {}", skips[0]);
    }
    let plans: Vec<&String> = lines
        .iter()
        .filter(|l| l.starts_with("[PLAN] lock widget: "))
        .collect();
    assert_eq!(plans.len(), 1, "one lock-widget plan line:\n{t}");
    assert!(
        plans[0].contains("not applied") && plans[0].contains(READ_FLAG),
        "{}",
        plans[0]
    );
    assert!(
        !t.contains("no GNOME card"),
        "no line claims the card is off:\n{t}"
    );
}

/// LWF1: the read path is pinned on both sides: `setup.sh` names the extension's
/// `KILL_SWITCH_PATH` as the only file whose flag step may be reported done, and no other
/// shipped component reads `lockwidget.disable` (the swaylock-plugin widget, the bridge and
/// the hyprlock snippet do not).
#[test]
fn test_lwf1_read_path_matches_the_gnome_extension() {
    let script = read_repo("scripts/setup.sh");
    assert!(
        script
            .lines()
            .any(|l| l == format!("readonly LOCK_WIDGET_READ_FLAG=\"{READ_FLAG}\"")),
        "setup.sh pins the read path"
    );
    let ext = read_repo("packaging/gnome-shell/soos-lockscreen@soos/extension.js");
    let declared = ext
        .lines()
        .find_map(|l| l.strip_prefix("const KILL_SWITCH_PATH = '"))
        .and_then(|rest| rest.strip_suffix("';"))
        .expect("KILL_SWITCH_PATH in extension.js");
    assert_eq!(declared, READ_FLAG, "setup.sh and extension.js agree");
    for rel in [
        "crates/lockstatus",
        "crates/lock-widget",
        "packaging/lockscreen",
    ] {
        let mut stack = vec![workspace_root().join(rel)];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if let Ok(body) = fs::read_to_string(&path) {
                    assert!(
                        !body.contains("lockwidget.disable"),
                        "{} reads the flag: setup.sh must learn its read path",
                        path.display()
                    );
                }
            }
        }
    }
    // The applied/not-applied decision is a comparison with the read path, never a guess.
    assert!(script.contains("\"${LOCK_WIDGET_FLAG}\" != \"${LOCK_WIDGET_READ_FLAG}\""));
}

/// LWF2: with a `--sysconfdir` other than `/etc` (the harness's fake root), `--lock-widget off`,
/// a repeated `off`, a plain run, `--lock-widget on` and a dry run never report the step as
/// applied, print the exact command for `/etc/soos/lockwidget.disable`, and the summary repeats
/// it; the user locker files stay untouched.
#[test]
fn test_lwf2_non_etc_sysconfdir_never_reports_the_flag_applied() {
    let h = Harness::new("lwf_elsewhere", &["gnome-shell"]);
    let touch = format!("sudo touch {READ_FLAG}");
    let remove = format!("sudo rm -f {READ_FLAG}");

    let out = h.run(&["--dry-run", "--lock-widget", "off"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let plan = lines_of(&out)
        .into_iter()
        .find(|l| l.starts_with("[PLAN] lock widget: "))
        .expect("plan line");
    assert!(
        plan.contains("not applied") && plan.contains(&touch),
        "{plan}"
    );

    let out = h.run(&["--yes", "--lock-widget", "off"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_lock_widget_not_applied(&out, &touch);
    assert!(
        lines_of(&out)
            .iter()
            .any(|l| l.contains("Lock screen") && l.contains(&touch)),
        "the summary prints the manual step:\n{}",
        text(&out)
    );

    let out = h.run(&["--yes", "--lock-widget", "off"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_lock_widget_not_applied(&out, &touch);

    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_lock_widget_not_applied(&out, &touch);

    let out = h.run(&["--yes", "--lock-widget", "on"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_lock_widget_not_applied(&out, &remove);
    assert!(
        lines_of(&out)
            .iter()
            .any(|l| l.contains("Lock screen") && l.contains(&remove)),
        "the summary prints the manual step:\n{}",
        text(&out)
    );

    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    assert_lock_widget_not_applied(&out, &remove);
}

/// LWF3: when `<sysconfdir>/soos/lockwidget.disable` is the file the extension reads (the read
/// path pointed at the fake root in a copy of the script), the step is reported done and
/// never "not applied".
#[test]
fn test_lwf3_read_path_flag_is_reported_done() {
    let h = Harness::new("lwf_applied", &[]);
    let path = h.repo.join("scripts/setup.sh");
    let original = format!("readonly LOCK_WIDGET_READ_FLAG=\"{READ_FLAG}\"");
    let script = fs::read_to_string(&path).unwrap();
    assert!(script.contains(&original), "read path line present");
    fs::write(
        &path,
        script.replace(
            &original,
            &format!("readonly LOCK_WIDGET_READ_FLAG=\"{}\"", h.flag().display()),
        ),
    )
    .unwrap();

    let out = h.run(&["--yes", "--lock-widget", "off"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    assert!(
        t.contains(&format!(
            "[DONE] lock widget: turned off ({})",
            h.flag().display()
        )),
        "{t}"
    );
    assert!(!t.contains("not applied: the GNOME extension reads"), "{t}");
    assert!(h.flag().is_file());

    let out = h.run(&["--yes", "--lock-widget", "on"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let t = text(&out);
    assert!(t.contains("[DONE] lock widget: turned on ("), "{t}");
    assert!(!t.contains("not applied: the GNOME extension reads"), "{t}");
    assert!(!h.flag().exists());
}

/// LWF4: the GNOME enable success line does not promise the card at the next lock without
/// saying that GNOME Shell may first need a log-out.
#[test]
fn test_lwf4_gnome_success_line_mentions_the_log_out() {
    let h = Harness::new("lwf_gnome", &["gnome-shell"]);
    let out = h.run(&["--yes"]);
    assert_eq!(code(&out), 0, "{}", text(&out));
    let line = lines_of(&out)
        .into_iter()
        .find(|l| l.starts_with("[DONE] lock screen (GNOME): soos-lockscreen@soos enabled for"))
        .unwrap_or_else(|| panic!("success line:\n{}", text(&out)));
    assert!(line.contains("log out and back in"), "{line}");
}

/// LWF5: uninstall.sh never removes a file through a symbolic link in place of
/// `<prefix>/share/soos` (the hyprlock snippet and the Arch PAM snippet below it stay, the link
/// stays a link) and still succeeds.
#[test]
fn test_lwf5_uninstall_refuses_a_linked_share_soos() {
    let dir = scratch("lwf_share_link");
    let stage = dir.join("stage");
    let target = dir.join("target");
    for rel in ["lockscreen/hyprlock-soos.conf", "pam/system-auth.snippet"] {
        let file = target.join(rel);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "keep\n").unwrap();
    }
    let share = stage.join("usr/share/soos");
    fs::create_dir_all(share.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &share).unwrap();
    let out = run_uninstall(&stage);
    assert_eq!(code(&out), 0, "{}", text(&out));
    for rel in ["lockscreen/hyprlock-soos.conf", "pam/system-auth.snippet"] {
        assert_eq!(
            fs::read_to_string(target.join(rel)).unwrap(),
            "keep\n",
            "{rel} untouched behind the link"
        );
    }
    assert!(fs::symlink_metadata(&share)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(text(&out).contains("symbolic link"), "{}", text(&out));
    let _ = fs::remove_dir_all(&dir);
}

/// LWF6: the documentation agrees with the code: the off state prints a notice (not "nothing
/// printed"), the read path and the "not applied" report are documented, the rollback
/// paragraph covers the lock-widget and camera-view failures (the `--check-config` refusal
/// included), no `setup.sh --uninstall` is promised, and the traceability is present.
#[test]
fn test_lwf6_documentation_and_traceability() {
    let widget = read_repo("Docs/LOCKSCREEN_WIDGET.md");
    let flat: String = widget.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        !flat.contains("nothing enabled or printed"),
        "the off notice is printed"
    );
    for needle in ["not applied", READ_FLAG, "off notice"] {
        assert!(flat.contains(needle), "LOCKSCREEN_WIDGET.md: {needle}");
    }
    let pkg = read_repo("Docs/PACKAGING_AND_PROVISIONING.md");
    let flat: String = pkg.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        !flat.contains("a later `--uninstall`"),
        "no setup.sh --uninstall is promised"
    );
    let rollback = flat
        .split("**Rollback.**")
        .nth(1)
        .and_then(|rest| rest.split(" | Exit |").next())
        .expect("rollback paragraph");
    for needle in ["lock widget", "camera view", "--check-config"] {
        assert!(rollback.contains(needle), "rollback paragraph: {needle}");
    }
    assert!(flat.contains("not applied") && flat.contains(READ_FLAG));
    let matrix = read_repo("AI/VERIFICATION_MATRIX.md");
    for row in 1..=6 {
        assert!(
            matrix.contains(&format!("| LWF{row} |")),
            "matrix row LWF{row}"
        );
    }
    assert!(workspace_root()
        .join("AI/walkthroughs/242_lockwidget_flag_path.md")
        .is_file());
}
