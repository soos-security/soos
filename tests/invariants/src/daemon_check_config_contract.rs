//! `scripts/setup.sh` validates the `daemon.toml` it creates with the installed
//! `soos-daemon --check-config` before restarting the daemon (GitHub #441, part of #362; ADR
//! 2026-10-18 "`soos-daemon --check-config` and Setup Validation of the Created
//! `daemon.toml`"; spec `AI/architect_spec_daemon_check_config.md`; rows DCC8-DCC10).
//!
//! The creation path runs only for `/etc/soos/daemon.toml`, which the fake-`--sysconfdir`
//! harness of `setup_user_phase_contract.rs` never reaches. Here the real `phase_preview` and the
//! helpers it calls are extracted from `setup.sh` verbatim and run without root against a
//! temporary directory, with a stub `soos-daemon` and a `systemctl` that only records its
//! arguments. The real-root path is `tests/docker/setup_root_preview_test.sh` (cases F1, F2).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, and indexing"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const SETUP: &str = "scripts/setup.sh";
const DOCKER_TEST: &str = "tests/docker/setup_root_preview_test.sh";
const FIXTURE: &str = "tests/fixtures/setup/daemon.toml";

/// The exact bounded call `phase_preview` makes before the restart.
const CHECK_CALL: &str =
    "run_bounded \"${QUERY_TIMEOUT_S}\" \"${DAEMON_BIN}\" --check-config --config \"${DAEMON_TOML}\"";

/// The `setup.sh` functions `phase_preview` needs, extracted verbatim.
const FUNCTIONS: [&str; 7] = [
    "run_bounded",
    "render_daemon_preview",
    "daemon_toml_allows_view",
    "daemon_preview_state",
    "daemon_preview_hint",
    "ensure_soos_conf_dir",
    "phase_preview",
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

fn bash_path() -> PathBuf {
    ["/usr/bin/bash", "/bin/bash"]
        .iter()
        .map(PathBuf::from)
        .find(|p| fs::metadata(p).is_ok_and(|m| m.is_file()))
        .expect("bash available")
}

/// The definition of shell function `name` in `script`: from `name() {` to the first `}` line.
fn function_of(script: &str, name: &str) -> String {
    let header = format!("{name}() {{");
    let mut lines = script.lines().skip_while(|l| *l != header);
    let mut out = Vec::new();
    for line in lines.by_ref() {
        out.push(line);
        if line == "}" {
            return out.join("\n");
        }
    }
    panic!("{SETUP}: function {name} not found or not closed");
}

/// The `readonly NAME=...` line of `setup.sh`.
fn readonly_line(script: &str, name: &str) -> String {
    script
        .lines()
        .find(|l| l.starts_with(&format!("readonly {name}=")))
        .unwrap_or_else(|| panic!("{SETUP}: readonly {name} not found"))
        .to_string()
}

struct PreviewRun {
    out: Output,
    log: Vec<String>,
    daemon_toml: PathBuf,
    etc_soos: PathBuf,
}

impl PreviewRun {
    fn text(&self) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&self.out.stdout),
            String::from_utf8_lossy(&self.out.stderr)
        )
    }

    fn last_stdout_line(&self) -> String {
        String::from_utf8_lossy(&self.out.stdout)
            .lines()
            .last()
            .unwrap_or("")
            .to_string()
    }
}

/// Runs the extracted `phase_preview` for an absent `daemon.toml` under a temporary
/// `SYSCONFDIR`, with a stub `soos-daemon` exiting `check_rc` for `--check-config`.
fn run_phase_preview(tag: &str, check_rc: i32) -> PreviewRun {
    let script = read_repo(SETUP);
    let dir = std::env::temp_dir().join(format!("soos_dcc_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let etc = dir.join("etc");
    fs::create_dir_all(&etc).unwrap();
    let log = dir.join("calls.log");
    fs::write(&log, "").unwrap();
    let daemon_bin = dir.join("libexec/soos/soos-daemon");
    fs::create_dir_all(daemon_bin.parent().unwrap()).unwrap();
    fs::write(
        &daemon_bin,
        format!(
            "#!{bash}\n\
             state=absent\n\
             [[ \"${{1:-}}\" == --check-config && \"${{2:-}}\" == --config && -f \"${{3:-}}\" ]] && state=present\n\
             printf 'soos-daemon %s %s\\n' \"$*\" \"${{state}}\" >> '{log}'\n\
             exit {check_rc}\n",
            bash = bash_path().display(),
            log = log.display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&daemon_bin, fs::Permissions::from_mode(0o755)).unwrap();
    // `run_bounded` runs `timeout systemctl ...`, which resolves `systemctl` through `PATH`:
    // the stub directory comes first so the host's real systemctl is never reachable.
    let stubs = dir.join("stubs");
    fs::create_dir_all(&stubs).unwrap();
    let systemctl = stubs.join("systemctl");
    fs::write(
        &systemctl,
        format!(
            "#!{bash}\nprintf 'systemctl %s\\n' \"$*\" >> '{log}'\nexit 0\n",
            bash = bash_path().display(),
            log = log.display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&systemctl, fs::Permissions::from_mode(0o755)).unwrap();
    let daemon_toml = etc.join("soos/daemon.toml");

    let mut harness = String::new();
    harness.push_str("set -u\n");
    harness.push_str(&readonly_line(&script, "DAEMON_PREVIEW_MARKER"));
    harness.push('\n');
    harness.push_str(&readonly_line(&script, "MAX_DAEMON_TOML_SCAN_BYTES"));
    harness.push('\n');
    // Setup-only (owner-approved 2026-10-18, #437): the restart bound read from the script.
    harness.push_str(&readonly_line(&script, "UNIT_START_TIMEOUT_S"));
    harness.push('\n');
    harness.push_str(&format!(
        "SYSCONFDIR='{etc}'\nDAEMON_TOML='{toml}'\nDAEMON_READ_CONFIG='{toml}'\n\
         DAEMON_BIN='{bin}'\nUNIT=soos-daemon.service\nQUERY_TIMEOUT_S=10\n\
         CAMERA_DAEMON=on\nTARGET_UID=1000\nJOURNAL=()\nPREVIEW_DAEMON_DOWN=false\n\
         info() {{ printf '[INFO]  %s\\n' \"$*\"; }}\n\
         error() {{ printf '[ERROR] %s\\n' \"$*\" >&2; }}\n\
         [[ \"$(command -v systemctl)\" == '{stub}' ]] || {{ echo 'systemctl stub not first in PATH' >&2; exit 99; }}\n\
         daemon_ready() {{ printf 'daemon_ready\\n' >> '{log}'; }}\n",
        etc = etc.display(),
        toml = daemon_toml.display(),
        bin = daemon_bin.display(),
        log = log.display(),
        stub = systemctl.display(),
    ));
    for name in FUNCTIONS {
        harness.push_str(&function_of(&script, name));
        harness.push('\n');
    }
    harness.push_str(
        "rc=0\nphase_preview || rc=$?\n\
         printf 'rc=%s journal=%s down=%s\\n' \"${rc}\" \"${JOURNAL[*]:-}\" \"${PREVIEW_DAEMON_DOWN}\"\n",
    );
    let harness_path = dir.join("harness.sh");
    fs::write(&harness_path, harness).unwrap();

    let out = Command::new(bash_path())
        .arg(&harness_path)
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", stubs.display()))
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .expect("run harness");
    let log = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    PreviewRun {
        out,
        log,
        daemon_toml,
        etc_soos: etc.join("soos"),
    }
}

/// DCC8: the `[preview]` render of `setup.sh` is byte-equal to the fixture that
/// `soos-daemon --check-config` accepts (`soos-daemon::check_config_tests::test_dcc5_*`).
#[test]
fn test_dcc8_setup_render_matches_the_checked_fixture() {
    let script = read_repo(SETUP);
    let harness = format!(
        "{}\nTARGET_UID=1000\n{}\nrender_daemon_preview\n",
        readonly_line(&script, "DAEMON_PREVIEW_MARKER"),
        function_of(&script, "render_daemon_preview")
    );
    let out = Command::new(bash_path())
        .arg("-c")
        .arg(&harness)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("run render");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        read_repo(FIXTURE),
        "the setup render and {FIXTURE} must be the same bytes"
    );
}

/// DCC8: a file `soos-daemon --check-config` refuses is removed again before any restart:
/// `phase_preview` fails (the caller exits 1 through `fail_and_rollback`), its journal no longer
/// holds `daemon_toml_created` (whose rollback would `try-restart`), and `systemctl` never runs.
#[test]
fn test_dcc8_refused_file_is_removed_without_restart() {
    let run = run_phase_preview("refused", 78);
    let text = run.text();
    assert!(
        run.log
            .iter()
            .any(|l| l.starts_with("soos-daemon --check-config --config ")
                && l.contains("daemon.toml present")),
        "the installed daemon checks the created file: {:?}\n{text}",
        run.log
    );
    assert!(
        text.contains("[FAIL] camera view (daemon): soos-daemon --check-config refused ")
            && text.contains("(removed again; soos-daemon.service not restarted)"),
        "{text}"
    );
    assert!(
        !run.daemon_toml.exists(),
        "the refused file must be removed"
    );
    assert!(
        !run.log.iter().any(|l| l.starts_with("systemctl")),
        "no restart after a refusal: {:?}",
        run.log
    );
    assert!(
        !run.log.iter().any(|l| l == "daemon_ready"),
        "{:?}",
        run.log
    );
    assert_eq!(
        run.last_stdout_line(),
        "rc=1 journal=soos_conf_dir_created down=false",
        "{text}"
    );
    let leftovers: Vec<_> = fs::read_dir(&run.etc_soos)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name())
        .collect();
    assert!(
        leftovers.is_empty(),
        "no file left in {}: {leftovers:?}",
        run.etc_soos.display()
    );
}

/// DCC8: a file the check accepts is checked before the daemon is restarted on it.
#[test]
fn test_dcc8_accepted_file_is_checked_before_the_restart() {
    let run = run_phase_preview("accepted", 0);
    let text = run.text();
    assert!(run.daemon_toml.is_file(), "{text}");
    assert_eq!(
        fs::read_to_string(&run.daemon_toml).unwrap(),
        read_repo(FIXTURE),
        "{text}"
    );
    let check = run
        .log
        .iter()
        .position(|l| {
            l.starts_with("soos-daemon --check-config --config ") && l.ends_with(" present")
        })
        .unwrap_or_else(|| panic!("no check of the created file: {:?}", run.log));
    let restart = run
        .log
        .iter()
        .position(|l| l == "systemctl restart soos-daemon.service")
        .unwrap_or_else(|| panic!("no restart: {:?}", run.log));
    assert!(check < restart, "check before restart: {:?}", run.log);
    assert_eq!(
        run.log
            .iter()
            .filter(|l| l.starts_with("soos-daemon "))
            .count(),
        1,
        "exactly one check: {:?}",
        run.log
    );
    assert!(
        text.contains("[DONE] camera view (daemon): ") && text.contains("restarted"),
        "{text}"
    );
    assert_eq!(
        run.last_stdout_line(),
        "rc=0 journal=soos_conf_dir_created daemon_toml_created down=false",
        "{text}"
    );
}

/// DCC9: static rules — the installed daemon path, the bounded check right after the file is
/// created and before the restart, an existing file still never edited, and the traceability.
#[test]
fn test_dcc9_static_rules_and_traceability() {
    let script = read_repo(SETUP);
    assert!(
        script
            .lines()
            .any(|l| l == "DAEMON_BIN=\"${PREFIX}/libexec/soos/soos-daemon\""),
        "{SETUP} must name the installed daemon"
    );
    let phase = function_of(&script, "phase_preview");
    let created = phase
        .find("JOURNAL+=(daemon_toml_created)")
        .expect("journal entry");
    let check = phase.find(CHECK_CALL).expect("bounded check call");
    let restart = phase
        .find("systemctl restart \"${UNIT}\"")
        .expect("restart");
    assert!(
        created < check && check < restart,
        "order: create, check, restart"
    );
    // One call (the `[FAIL]` message also names the option, so the call itself is counted).
    let calls = |text: &str| text.matches("--check-config --config").count();
    assert_eq!(calls(&phase), 1, "one check");
    assert_eq!(calls(&script), 1, "the check runs only in phase_preview");
    // AS-U2 unchanged: an existing file is reported, never written.
    assert!(phase.contains("exists and setup never edits it"));

    let daemon_doc = read_repo("Docs/DAEMON.md");
    for needle in ["--check-config", "EX_NOINPUT", "EX_CONFIG", "66", "78"] {
        assert!(
            daemon_doc.contains(needle),
            "Docs/DAEMON.md must mention {needle}"
        );
    }
    let pkg = read_repo("Docs/PACKAGING_AND_PROVISIONING.md");
    assert!(
        pkg.contains("soos-daemon --check-config --config"),
        "Docs/PACKAGING_AND_PROVISIONING.md must describe the check"
    );
    let adr = read_repo("AI/DECISIONS.md");
    assert!(
        adr.contains(
            "`soos-daemon --check-config` and Setup Validation of the Created `daemon.toml`"
        ),
        "AI/DECISIONS.md must carry the ADR"
    );
    let matrix = read_repo("AI/VERIFICATION_MATRIX.md");
    for row in 1..=11 {
        assert!(
            matrix.contains(&format!("| DCC{row} |")),
            "the matrix must carry row DCC{row}"
        );
    }
    assert!(
        workspace_root()
            .join("AI/walkthroughs/240_daemon_check_config.md")
            .is_file(),
        "walkthrough 240 must exist"
    );
}

/// DCC10: the real-root Docker harness answers `--check-config` without starting a daemon and
/// runs the refusal (F1) and check-before-restart (F2) cases in its default case list.
#[test]
fn test_dcc10_docker_root_cases_wired() {
    let docker = read_repo(DOCKER_TEST);
    for needle in [
        "readonly ALL_CASES=(A1 A2 B C1 C2 D E1 E2 F1 F2)",
        "readonly CHECK_LOG=",
        "\"--check-config\" in sys.argv",
        "check-refuses",
        "case_f1()",
        "case_f2()",
        "soos-daemon --check-config refused ${DAEMON_TOML} (removed again; ${UNIT} not restarted)",
        "expect_rc 1",
    ] {
        assert!(
            docker.contains(needle),
            "{DOCKER_TEST} must contain `{needle}`"
        );
    }
    // The stand-in answers the check before it reads its mode for a start or binds the socket.
    let check = docker
        .find("\"--check-config\" in sys.argv")
        .expect("check branch");
    let bind = docker.find("server.bind(SOCKET_PATH)").expect("bind");
    assert!(check < bind, "the check branch precedes the socket bind");
}
