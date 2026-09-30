//! Invariants for the multi-distribution test infrastructure (GitHub #162, #163, #168).
//!
//! - ONB-04 (#162): every sandbox Dockerfile must write real multi-line PAM files; the
//!   stacks are rebuilt here with `bash` (the `/bin/sh` of fedora:40 and archlinux:latest,
//!   whose builtin `echo` does not interpret `\n`) and parsed line by line.
//! - ONB-05 (#163): `tests/distro/run_distro_validation.sh` only calls scripts that exist,
//!   its `--dry-run` and its no-Docker path never run anything privileged, and the distro
//!   scripts refuse to mutate a host without explicit consent.
//! - ONB-10 (#168): deployment tests keep the `0750 root:soos` socket directory and the
//!   `0660` socket invariant, and enroll through `soos-enroll --mock` instead of fabricating
//!   a template file.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Sandbox images whose PAM stacks are exercised by `tests/docker/test_suite.sh`.
const SANDBOX_DOCKERFILES: [&str; 4] = [
    "Dockerfile",
    "tests/docker/Dockerfile.ubuntu",
    "tests/docker/Dockerfile.fedora",
    "tests/docker/Dockerfile.arch",
];

/// Distribution deployment scripts driven by `run_distro_validation.sh`.
const DISTRO_SCRIPTS: [&str; 3] = [
    "tests/distro/debian_ubuntu_test.sh",
    "tests/distro/fedora_rhel_test.sh",
    "tests/distro/arch_linux_test.sh",
];

/// Explicit consent flag required before a distro script mutates the host it runs on.
const HOST_CONSENT_FLAG: &str = "--allow-host-changes";

/// Commands that a dry run (or a refused live run) must never execute. Each is shadowed by
/// a recording shim placed first in `PATH`.
const PRIVILEGED_COMMANDS: [&str; 30] = [
    "docker",
    "dpkg",
    "apt-get",
    "rpm",
    "rpmbuild",
    "dnf",
    "pacman",
    "makepkg",
    "cargo",
    "gcc",
    "useradd",
    "usermod",
    "groupadd",
    "chpasswd",
    "pam-auth-update",
    "authselect",
    "systemctl",
    "soos-admin",
    "soos-enroll",
    "install",
    "chmod",
    "chown",
    "rm",
    "mkdir",
    "cp",
    "mv",
    "python3",
    "pkill",
    "sudo",
    "tee",
];

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("Unable to locate workspace root directory")
        .to_path_buf()
}

/// Creates a unique scratch directory under the system temporary directory.
fn scratch_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "soos-inv-{}-{}-{}-{}",
        tag,
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Splits a Dockerfile into logical instructions the way the Docker parser does: a
/// trailing backslash joins the next physical line, and comment lines are dropped, also
/// inside a continuation.
fn dockerfile_instructions(content: &str) -> Vec<String> {
    let mut instructions = Vec::new();
    let mut current = String::new();
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || (current.is_empty() && trimmed.is_empty()) {
            continue;
        }
        if let Some(stripped) = line.strip_suffix('\\') {
            current.push_str(stripped);
        } else {
            current.push_str(line);
            instructions.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        instructions.push(current);
    }
    instructions
}

/// Returns the shell command of every `RUN` instruction that writes into `/etc/pam.d/`.
fn pam_writing_run_commands(dockerfile: &str) -> Vec<String> {
    dockerfile_instructions(dockerfile)
        .into_iter()
        .filter_map(|instr| {
            let trimmed = instr.trim_start();
            trimmed
                .strip_prefix("RUN ")
                .map(|cmd| cmd.trim().to_string())
        })
        .filter(|cmd| cmd.contains("/etc/pam.d/"))
        .collect()
}

/// Validates one generated PAM service file and returns its module lines.
fn assert_valid_pam_stack(label: &str, content: &str) -> Vec<String> {
    assert!(
        !content.contains("\\n"),
        "INVARIANT VIOLATION (#162): {} contains a literal '\\n' sequence: the file was \
         written as a single line and PAM ignores every module in it:\n{}",
        label,
        content
    );
    let modules: Vec<String> = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    assert!(
        modules.len() >= 2,
        "INVARIANT VIOLATION (#162): {} must contain at least 2 PAM module lines, found {}:\n{}",
        label,
        modules.len(),
        content
    );
    for line in &modules {
        let mut fields = line.split_whitespace();
        let kind = fields.next().unwrap_or_default();
        let kind = kind.strip_prefix('-').unwrap_or(kind);
        assert!(
            ["auth", "account", "password", "session"].contains(&kind),
            "INVARIANT VIOLATION (#162): {} has a line with an invalid PAM type '{}': {}",
            label,
            kind,
            line
        );
        let rest: Vec<&str> = fields.collect();
        let joined = rest.join(" ");
        let (control_ok, module) = if joined.starts_with('[') {
            match joined.find(']') {
                Some(end) => (true, joined[end + 1..].split_whitespace().next()),
                None => (false, None),
            }
        } else {
            (!rest.is_empty(), rest.get(1).copied())
        };
        assert!(
            control_ok && module.is_some_and(|m| m.ends_with(".so") || kind_is_include(&rest)),
            "INVARIANT VIOLATION (#162): {} has a malformed PAM line (expected '<type> <control> <module>'): {}",
            label,
            line
        );
    }
    modules
}

fn kind_is_include(rest: &[&str]) -> bool {
    matches!(rest.first(), Some(&"include") | Some(&"substack"))
}

/// #162 — every sandbox Dockerfile builds multi-line PAM stacks that load pam_soos then
/// pam_unix, even when `/bin/sh` is bash (fedora:40, archlinux:latest).
#[test]
fn test_sandbox_dockerfiles_produce_multiline_pam_stacks() {
    let root = workspace_root();
    for rel in SANDBOX_DOCKERFILES {
        let dockerfile = fs::read_to_string(root.join(rel))
            .unwrap_or_else(|e| panic!("cannot read {}: {}", rel, e));
        let commands = pam_writing_run_commands(&dockerfile);
        assert!(
            !commands.is_empty(),
            "{} must provision at least one /etc/pam.d/ test service",
            rel
        );

        let out_dir = scratch_dir("pamd");
        let out_prefix = format!("{}/", out_dir.display());
        for cmd in &commands {
            let relocated = cmd.replace("/etc/pam.d/", &out_prefix);
            let status = Command::new("bash")
                .arg("-c")
                .arg(&relocated)
                .current_dir(&out_dir)
                .stdin(Stdio::null())
                .status()
                .expect("run bash");
            assert!(
                status.success(),
                "{}: PAM provisioning RUN instruction failed under bash: {}",
                rel,
                cmd
            );
        }

        let mut generated: Vec<PathBuf> = fs::read_dir(&out_dir)
            .expect("read generated pam.d")
            .map(|e| e.expect("dir entry").path())
            .collect();
        generated.sort();
        assert!(
            generated.iter().any(|p| p.ends_with("test-soos")),
            "{} must provision /etc/pam.d/test-soos (used by T1-T5, T7, T8)",
            rel
        );
        for file in &generated {
            let content = fs::read_to_string(file).expect("read generated pam file");
            let label = format!(
                "{} -> /etc/pam.d/{}",
                rel,
                file.file_name().unwrap().to_string_lossy()
            );
            let modules = assert_valid_pam_stack(&label, &content);
            let soos = modules
                .iter()
                .position(|l| l.starts_with("auth") && l.contains("pam_soos.so"));
            let unix = modules
                .iter()
                .position(|l| l.starts_with("auth") && l.contains("pam_unix.so"));
            match (soos, unix) {
                (Some(s), Some(u)) => assert!(
                    s < u,
                    "{}: the pam_soos.so auth line must precede pam_unix.so",
                    label
                ),
                _ => panic!(
                    "{} must contain separate 'auth ... pam_soos.so' and 'auth ... pam_unix.so' lines:\n{}",
                    label, content
                ),
            }
        }
        fs::remove_dir_all(&out_dir).ok();
    }
}

/// #162 — static lint from the issue: no RUN instruction may rely on `echo` expanding `\n`.
#[test]
fn test_sandbox_dockerfiles_never_rely_on_echo_escape_expansion() {
    let root = workspace_root();
    for rel in SANDBOX_DOCKERFILES {
        let dockerfile = fs::read_to_string(root.join(rel)).expect("read dockerfile");
        for instr in dockerfile_instructions(&dockerfile) {
            let trimmed = instr.trim_start();
            if !trimmed.starts_with("RUN ") {
                continue;
            }
            assert!(
                !(trimmed.contains("echo") && trimmed.contains("\\n")),
                "INVARIANT VIOLATION (#162): {} uses `echo \"...\\n...\"`; /bin/sh is bash on \
                 fedora/arch and writes a literal '\\n'. Use printf '%s\\n' instead: {}",
                rel,
                trimmed
            );
            assert!(
                !trimmed.contains("echo -e"),
                "INVARIANT VIOLATION (#162): {} uses non-portable `echo -e`: {}",
                rel,
                trimmed
            );
        }
    }
}

/// #162 — the in-container suite refuses to run T1-T10 on a malformed PAM stack.
#[test]
fn test_pam_matrix_suite_validates_stack_files_before_running() {
    let root = workspace_root();
    let suite = fs::read_to_string(root.join("tests/docker/test_suite.sh")).expect("read suite");
    let def = suite
        .find("assert_pam_stack_file()")
        .expect("test_suite.sh must define assert_pam_stack_file()");
    let first_case = suite
        .find("T1: Nominal Facial Auth — Daemon Running")
        .expect("T1 banner");
    let call = suite[def + "assert_pam_stack_file()".len()..]
        .find("assert_pam_stack_file /etc/pam.d/test-soos")
        .map(|i| i + def + "assert_pam_stack_file()".len())
        .expect("test_suite.sh must validate /etc/pam.d/test-soos");
    assert!(
        call < first_case,
        "test_suite.sh must validate the PAM stack files before T1"
    );
}

/// Extracts every token ending in `.sh` from a shell script (quotes stripped).
fn referenced_scripts(script: &str) -> Vec<String> {
    script
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .flat_map(|l| {
            l.split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ';')
                .filter(|t| t.ends_with(".sh"))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// #163 — the runner only invokes distro scripts that exist, with no interpolated names.
#[test]
fn test_distro_runner_references_only_existing_scripts() {
    let root = workspace_root();
    let runner = fs::read_to_string(root.join("tests/distro/run_distro_validation.sh"))
        .expect("read runner");
    let refs = referenced_scripts(&runner);
    let mut resolved = Vec::new();
    for token in &refs {
        let rel = if let Some(rest) = token.strip_prefix("${SCRIPT_DIR}/") {
            format!("tests/distro/{}", rest)
        } else if let Some(rest) = token.strip_prefix("/workspace/") {
            rest.to_string()
        } else if let Some(idx) = token.find("tests/distro/") {
            token[idx..].to_string()
        } else if !token.contains('/') && token.ends_with("_test.sh") {
            // Bare names of an explicit distro -> script mapping.
            format!("tests/distro/{}", token)
        } else {
            continue;
        };
        assert!(
            !rel.contains('$'),
            "INVARIANT VIOLATION (#163): run_distro_validation.sh builds a script path from a \
             variable ('{}'); map each distro to an explicit script name",
            token
        );
        assert!(
            root.join(&rel).is_file(),
            "INVARIANT VIOLATION (#163): run_distro_validation.sh references '{}' which does not exist",
            token
        );
        resolved.push(rel);
    }
    for script in DISTRO_SCRIPTS {
        assert!(
            resolved.iter().any(|r| r == script),
            "run_distro_validation.sh must reference {} explicitly",
            script
        );
    }
}

/// Builds a directory of recording shims shadowing every privileged command.
fn shim_dir() -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch_dir("shims");
    let log = dir.join("invocations.log");
    for name in PRIVILEGED_COMMANDS {
        let shim = dir.join(name);
        fs::write(
            &shim,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"{} $*\" >> '{}'\nexit 97\n",
                name,
                log.display()
            ),
        )
        .expect("write shim");
        fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("chmod shim");
    }
    (dir, log)
}

/// Runs a repository script with the recording shims first in `PATH`.
fn run_with_shims(script: &str, args: &[&str], extra_env: &[(&str, &str)]) -> (Output, String) {
    let root = workspace_root();
    let (dir, log) = shim_dir();
    let path = format!(
        "{}:{}",
        dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut cmd = Command::new("bash");
    cmd.arg(root.join(script))
        .args(args)
        .current_dir(&root)
        .env("PATH", path)
        .stdin(Stdio::null());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("run script");
    let invoked = fs::read_to_string(&log).unwrap_or_default();
    fs::remove_dir_all(&dir).ok();
    (output, invoked)
}

/// #163 — `--dry-run` really does nothing privileged, for every distribution.
#[test]
fn test_distro_runner_dry_run_executes_nothing_privileged() {
    let (output, invoked) = run_with_shims(
        "tests/distro/run_distro_validation.sh",
        &["--dry-run", "all"],
        &[],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "run_distro_validation.sh --dry-run all must succeed: {}\n{}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        invoked.is_empty(),
        "INVARIANT VIOLATION (#163): --dry-run executed privileged commands:\n{}",
        invoked
    );
    assert_eq!(
        stdout.matches("[DRY RUN]").count(),
        3,
        "--dry-run all must print the plan of the 3 distribution scripts:\n{}",
        stdout
    );
}

/// #163 — without Docker, the runner refuses instead of running the live suite on the host.
#[test]
fn test_distro_runner_without_docker_refuses_live_host_run() {
    for distro in ["ubuntu", "fedora", "arch", "all"] {
        let (output, invoked) = run_with_shims(
            "tests/distro/run_distro_validation.sh",
            &[distro],
            &[("SOOS_DOCKER", "/nonexistent/soos-no-docker")],
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "INVARIANT VIOLATION (#163): runner '{}' without Docker must fail, not fall back to a live run:\n{}",
            distro,
            stdout
        );
        assert!(
            invoked.is_empty(),
            "INVARIANT VIOLATION (#163): runner '{}' without Docker executed:\n{}",
            distro,
            invoked
        );
        assert!(
            !stdout.contains("Deployment Validation (#32"),
            "INVARIANT VIOLATION (#163): runner '{}' without Docker started a distro script:\n{}",
            distro,
            stdout
        );
        assert!(
            stderr.contains("Docker"),
            "runner '{}' must explain that Docker is required: {}",
            distro,
            stderr
        );
    }
}

/// #163 — `--skip-docker` live runs and direct distro script runs require explicit consent.
#[test]
fn test_distro_scripts_refuse_host_mutation_without_consent() {
    let (output, invoked) = run_with_shims(
        "tests/distro/run_distro_validation.sh",
        &["--skip-docker", "ubuntu"],
        &[],
    );
    assert!(
        !output.status.success() && invoked.is_empty(),
        "INVARIANT VIOLATION (#163): --skip-docker without {} must refuse and execute nothing:\n{}",
        HOST_CONSENT_FLAG,
        invoked
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(HOST_CONSENT_FLAG),
        "--skip-docker refusal must name {}",
        HOST_CONSENT_FLAG
    );

    for script in DISTRO_SCRIPTS {
        let (output, invoked) = run_with_shims(script, &[], &[]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(2),
            "INVARIANT VIOLATION (#163): {} without {} must exit 2 before any host change: {}",
            script,
            HOST_CONSENT_FLAG,
            stderr
        );
        assert!(
            invoked.is_empty(),
            "INVARIANT VIOLATION (#163): {} executed commands without consent:\n{}",
            script,
            invoked
        );
        assert!(
            stderr.contains(HOST_CONSENT_FLAG),
            "{} refusal must name {}: {}",
            script,
            HOST_CONSENT_FLAG,
            stderr
        );
    }
}

/// #163 — the Docker path gives consent only inside the disposable container.
#[test]
fn test_distro_runner_grants_consent_only_inside_container() {
    let root = workspace_root();
    let runner = fs::read_to_string(root.join("tests/distro/run_distro_validation.sh"))
        .expect("read runner");
    let run_idx = runner
        .find("run --rm")
        .expect("runner must start disposable containers with `run --rm`");
    let tail = &runner[run_idx..];
    let end = tail.find("\n}").unwrap_or(tail.len());
    assert!(
        tail[..end].contains(HOST_CONSENT_FLAG),
        "the containerized invocation must pass {} to the distro script",
        HOST_CONSENT_FLAG
    );
}

/// #163 — after a native package install, the rollback step uses the package manager.
#[test]
fn test_distro_scripts_record_the_install_mode_actually_used() {
    let root = workspace_root();
    for (script, install_cmd, mode) in [
        (DISTRO_SCRIPTS[0], "dpkg -i", "deb"),
        (DISTRO_SCRIPTS[1], "rpm -i", "rpm"),
        (DISTRO_SCRIPTS[2], "pacman -U", "pkgbuild"),
    ] {
        let content = fs::read_to_string(root.join(script)).expect("read distro script");
        let idx = content
            .find(install_cmd)
            .unwrap_or_else(|| panic!("{} must install via `{}`", script, install_cmd));
        let branch_end = content[idx..]
            .find("else")
            .map(|e| e + idx)
            .expect("install branch has an else");
        let expected = format!("INSTALL_MODE=\"{}\"", mode);
        assert!(
            content[idx..branch_end].contains(&expected),
            "INVARIANT VIOLATION (#163): {} installs via `{}` but does not set {} for rollback",
            script,
            install_cmd,
            expected
        );
    }
}

/// Every test script, helper and sandbox Dockerfile of the deployment test infrastructure.
fn deployment_test_files() -> Vec<PathBuf> {
    let root = workspace_root();
    let mut files = vec![root.join("Dockerfile")];
    for dir in ["tests/docker", "tests/distro", "tests/physical"] {
        let mut entries: Vec<PathBuf> = fs::read_dir(root.join(dir))
            .unwrap_or_else(|e| panic!("read {}: {}", dir, e))
            .map(|e| e.expect("dir entry").path())
            .filter(|p| p.is_file())
            .collect();
        entries.sort();
        files.extend(entries);
    }
    files
}

/// #168 — no deployment test widens the socket directory or socket permissions.
#[test]
fn test_deployment_tests_never_widen_socket_permissions() {
    for file in deployment_test_files() {
        let Ok(content) = fs::read_to_string(&file) else {
            continue;
        };
        for (n, line) in content.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with('#') {
                continue;
            }
            for forbidden in [
                "chmod 777",
                "chmod 0777",
                "chmod 666",
                "chmod 0666",
                "chmod a+w",
                "chmod o+w",
                "chmod -R 777",
                "0o666",
                "0o777",
            ] {
                assert!(
                    !code.contains(forbidden),
                    "INVARIANT VIOLATION (#168): {}:{} uses '{}' (the socket directory is \
                     0750 root:soos and the socket 0660): {}",
                    file.display(),
                    n + 1,
                    forbidden,
                    line
                );
            }
        }
    }
}

/// #168 — scripts that serve a mock daemon on /run/soos create the directory 0750 root:soos
/// and assert the resulting modes of the directory and the socket.
#[test]
fn test_deployment_tests_create_socket_dir_with_invariant_modes() {
    let root = workspace_root();
    let mut scripts = vec!["tests/docker/test_suite.sh"];
    scripts.extend(DISTRO_SCRIPTS);
    for script in scripts {
        let content = fs::read_to_string(root.join(script)).expect("read script");
        assert!(
            content.contains("install -d -m 0750 -o root -g soos /run/soos"),
            "INVARIANT VIOLATION (#168): {} must create /run/soos with \
             `install -d -m 0750 -o root -g soos /run/soos`",
            script
        );
        assert!(
            content.contains("assert_socket_modes"),
            "{} must assert the modes of /run/soos (750 root:soos) and daemon.sock (660 root:soos)",
            script
        );
    }
}

fn wait_for_socket(path: &Path, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn current_group_name() -> String {
    let out = Command::new("id").arg("-gn").output().expect("id -gn");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// #168 — the mock daemon serves a 0660 group-owned socket and rejects wider modes.
#[test]
fn test_mock_daemon_socket_is_group_restricted() {
    use std::os::unix::fs::PermissionsExt;
    let root = workspace_root();
    let mock = root.join("tests/docker/mock_daemon.py");
    let dir = scratch_dir("mock");
    let sock = dir.join("run").join("daemon.sock");
    let group = current_group_name();

    let mut child = Command::new("python3")
        .arg(&mock)
        .args(["--mode", "allow", "--socket"])
        .arg(&sock)
        .args(["--socket-group", &group])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mock_daemon.py");
    let appeared = wait_for_socket(&sock, Duration::from_secs(10));
    let mode = fs::metadata(&sock).map(|m| m.permissions().mode() & 0o7777);
    child.kill().ok();
    let output = child.wait_with_output().expect("wait mock");
    assert!(
        appeared,
        "mock_daemon.py --socket-group {} never created its socket: {}",
        group,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        mode.expect("socket metadata"),
        0o660,
        "INVARIANT VIOLATION (#168): mock_daemon.py must create its socket with mode 0660"
    );

    for wide in ["0666", "0777", "0662"] {
        let sock = dir.join(format!("wide-{}", wide)).join("daemon.sock");
        let output = Command::new("python3")
            .arg(&mock)
            .args(["--mode", "allow", "--socket"])
            .arg(&sock)
            .args(["--socket-group", &group, "--socket-mode", wide])
            .stdin(Stdio::null())
            .output()
            .expect("run mock_daemon.py");
        assert!(
            !output.status.success() && !sock.exists(),
            "INVARIANT VIOLATION (#168): mock_daemon.py must refuse world-accessible socket mode {}",
            wide
        );
    }
    fs::remove_dir_all(&dir).ok();
}

/// #168 — distro tests enroll through the real CLI (mock camera) and check the real store
/// artifact instead of writing random bytes to a file name the store never reads.
#[test]
fn test_distro_tests_enroll_through_soos_enroll_mock() {
    let root = workspace_root();
    for script in DISTRO_SCRIPTS {
        let content = fs::read_to_string(root.join(script)).expect("read distro script");
        let code: String = content
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains("/dev/urandom"),
            "INVARIANT VIOLATION (#168): {} fabricates a template from /dev/urandom",
            script
        );
        assert!(
            !code.contains(".bin\""),
            "INVARIANT VIOLATION (#168): {} references a '.bin' template the store never reads",
            script
        );
        assert!(
            code.contains("soos-enroll --mock enroll --username \"${TEST_USER}\" --yes"),
            "{} must enroll with `soos-enroll --mock enroll --username \"${{TEST_USER}}\" --yes`",
            script
        );
        assert!(
            code.contains(".cbor.enc"),
            "{} must check the store artifact <uid>.cbor.enc",
            script
        );
        assert!(
            code.contains("600 root:root"),
            "{} must assert the template is mode 600 root:root",
            script
        );
    }
}

/// #162 — the Fedora and Arch sandbox images are exercised by CI, not only locally.
#[test]
fn test_ci_runs_fedora_and_arch_pam_matrix() {
    let root = workspace_root();
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci.yml");
    let start = ci
        .find("\n  distro-pam-matrix:")
        .expect("ci.yml must define the distro-pam-matrix job");
    let job = &ci[start + 1..];
    let end = job.find("\n  # ----").unwrap_or(job.len());
    let job = &job[..end];
    assert!(
        job.contains("distro: [fedora, arch]"),
        "distro-pam-matrix must cover fedora and arch"
    );
    assert!(
        job.contains("./tests/docker/run_matrix.sh \"$DISTRO\""),
        "distro-pam-matrix must run tests/docker/run_matrix.sh for each distro"
    );
}

/// Returns the body of the top-level job `name` in `ci.yml` (up to the next banner comment).
fn ci_job<'a>(ci: &'a str, name: &str) -> &'a str {
    let start = ci
        .find(&format!("\n  {name}:"))
        .unwrap_or_else(|| panic!("ci.yml must define the {name} job"));
    let job = &ci[start + 1..];
    let end = job.find("\n  # ----").unwrap_or(job.len());
    &job[..end]
}

/// #168 (ONB-10) — the packaging and distro deployment path runs in CI on every pull request
/// for Ubuntu (native `.deb` build, `dpkg -i`, enrollment, password fallback, rollback, and the
/// package content / key-isolation harness), and a failure blocks the `CI Success` gate.
#[test]
fn test_ci_runs_ubuntu_package_deployment_on_pull_requests() {
    let root = workspace_root();
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci.yml");
    let job = ci_job(&ci, "package-deploy");

    assert!(
        job.contains("if: github.event_name != 'schedule'"),
        "package-deploy must run on pull requests (only the schedule is excluded):\n{job}"
    );
    assert!(job.contains("needs: lint"), "package-deploy must need lint");
    assert!(
        job.contains("timeout-minutes:"),
        "package-deploy must set a timeout"
    );
    assert!(
        job.contains("uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1"),
        "package-deploy must use the SHA-pinned checkout action"
    );
    assert!(
        job.contains("persist-credentials: false"),
        "package-deploy must not persist git credentials"
    );
    assert!(
        job.contains("./tests/distro/run_distro_validation.sh ubuntu"),
        "package-deploy must run the Ubuntu deployment validation"
    );
    assert!(
        job.contains("tests/docker/test_packages.sh"),
        "package-deploy must run the package content harness"
    );
    assert!(
        !job.contains("${{ github.event"),
        "package-deploy must not interpolate event data into its steps"
    );

    let gate = ci_job(&ci, "ci-success");
    let needs = gate
        .lines()
        .find(|l| l.trim_start().starts_with("needs:"))
        .expect("ci-success needs list");
    assert!(
        needs.contains("package-deploy"),
        "package-deploy must be part of the CI Success aggregate: {needs}"
    );
}
