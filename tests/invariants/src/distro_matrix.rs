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

// ---------------------------------------------------------------------------
// TCI-06 (#189) and ONB-10b (#274): the Docker PAM matrix asserts real outcomes, the
// timeout case asserts the deadline, every mock failure mode is exercised, and the
// Fedora / Arch deployment paths run in CI.
// ---------------------------------------------------------------------------

/// Shared shell helpers sourced by `tests/docker/test_suite.sh`.
const PAM_CASE_LIB: &str = "tests/docker/pam_case_lib.sh";

/// Returns the body of the case `id` (e.g. `"T6"`) in `test_suite.sh`: from its
/// `# T6:` header comment up to the next case header.
fn suite_case<'a>(suite: &'a str, id: &str) -> &'a str {
    let header = format!("\n# {id}:");
    let start = suite
        .find(&header)
        .unwrap_or_else(|| panic!("test_suite.sh must contain a '# {id}:' case header"))
        + 1;
    let body_start = start + header.len() - 1;
    let rest = &suite[body_start..];
    let mut end = rest.len();
    let mut offset = 0;
    while let Some(i) = rest[offset..].find("\n# T") {
        let pos = offset + i;
        if rest[pos + 4..].starts_with(|c: char| c.is_ascii_digit()) {
            end = pos;
            break;
        }
        offset = pos + 4;
    }
    &suite[start..body_start + end]
}

/// Lines of `script` that are not comments.
fn code_lines(script: &str) -> Vec<&str> {
    script
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect()
}

/// Runs `snippet` in bash after sourcing the PAM case library; returns (status, stdout).
fn run_case_lib(snippet: &str) -> (bool, String) {
    let root = workspace_root();
    let lib = root.join(PAM_CASE_LIB);
    let out = Command::new("bash")
        .arg("-c")
        .arg(format!("set -euo pipefail; source \"$1\"; {snippet}"))
        .arg("bash")
        .arg(&lib)
        .output()
        .expect("run bash");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    )
}

/// #189 — the timeout budget of a case is read from the PAM stack under test (first
/// `pam_soos.so` auth line that is not the password-failed hook), with the module's own
/// default (1000 ms) and clamp (10..=5000 ms), never hard-coded.
#[test]
fn test_pam_case_lib_reads_timeout_from_the_stack_under_test() {
    let dir = scratch_dir("stack-timeout");
    let cases = [
        (
            "auth [success=done default=ignore] pam_soos.so timeout_ms=250\nauth required pam_unix.so\n",
            "250",
        ),
        (
            "auth optional pam_soos.so event=password-failed timeout_ms=20\n\
             auth  [success=done default=ignore]  pam_soos.so timeout_ms=2500\n",
            "2500",
        ),
        (
            "auth [success=done default=ignore] pam_soos.so\nauth required pam_unix.so\n",
            "1000",
        ),
        (
            "auth [success=done default=ignore] pam_soos.so timeout_ms=99999\n",
            "5000",
        ),
        (
            "auth [success=done default=ignore] pam_soos.so timeout_ms=1\n",
            "10",
        ),
        (
            "# auth sufficient pam_soos.so timeout_ms=7\n\
             auth [success=done default=ignore] pam_soos.so service=sudo timeout_ms=300\n",
            "300",
        ),
    ];
    for (n, (stack, expected)) in cases.iter().enumerate() {
        let file = dir.join(format!("stack-{n}"));
        fs::write(&file, stack).expect("write stack");
        let (ok, out) = run_case_lib(&format!("pam_stack_timeout_ms '{}'", file.display()));
        assert!(ok, "pam_stack_timeout_ms failed on stack:\n{stack}");
        assert_eq!(
            &out, expected,
            "pam_stack_timeout_ms returned the wrong budget for:\n{stack}"
        );
    }
    let file = dir.join("no-soos");
    fs::write(&file, "auth required pam_unix.so\n").expect("write stack");
    let (ok, _) = run_case_lib(&format!("pam_stack_timeout_ms '{}'", file.display()));
    assert!(
        !ok,
        "pam_stack_timeout_ms must fail on a stack without a pam_soos.so auth line"
    );
    fs::remove_dir_all(&dir).ok();
}

/// #189 — the deadline assertion fails once the elapsed time exceeds the budget plus the
/// documented tolerance, and the injected daemon delay always exceeds that bound.
#[test]
fn test_pam_case_lib_deadline_assertion_can_fail() {
    let (ok, _) = run_case_lib("assert_elapsed_within_deadline T 900 250");
    assert!(ok, "900 ms is within 250 ms + tolerance");
    let (ok, _) = run_case_lib("assert_elapsed_within_deadline T 2250 250");
    assert!(
        !ok,
        "2250 ms (a module that waited for the delayed daemon) must fail the deadline"
    );
    for timeout in [10u64, 250, 1000, 2500, 5000] {
        let (ok, bound) = run_case_lib(&format!("deadline_bound_ms {timeout}"));
        assert!(ok);
        let (ok, delay) = run_case_lib(&format!("timeout_mock_delay_ms {timeout}"));
        assert!(ok);
        let bound: u64 = bound.parse().expect("bound is a number");
        let delay: u64 = delay.parse().expect("delay is a number");
        assert!(
            bound > timeout && delay > bound,
            "the mock delay ({delay} ms) must exceed the deadline bound ({bound} ms) for \
             timeout_ms={timeout}, so a module that waits for the daemon is detected"
        );
        let (ok, _) = run_case_lib(&format!(
            "assert_elapsed_within_deadline T {bound} {timeout}"
        ));
        assert!(ok, "the bound itself is accepted");
        let (ok, _) = run_case_lib(&format!(
            "assert_elapsed_within_deadline T {} {timeout}",
            bound + 1
        ));
        assert!(!ok, "one millisecond over the bound is rejected");
    }
}

/// #189 — T2 derives the mock delay and the elapsed bound from the stack's `timeout_ms`
/// and asserts the bound; a late `Allow` is never honored (T2b).
#[test]
fn test_pam_matrix_t2_asserts_the_deadline() {
    let root = workspace_root();
    let suite = fs::read_to_string(root.join("tests/docker/test_suite.sh")).expect("read suite");
    assert!(
        code_lines(&suite)
            .iter()
            .any(|l| l.trim_start().starts_with("source ") && l.contains(PAM_CASE_LIB)),
        "test_suite.sh must source {PAM_CASE_LIB}"
    );
    let t2 = suite_case(&suite, "T2");
    let code = code_lines(t2).join("\n");
    assert!(
        code.contains("pam_stack_timeout_ms /etc/pam.d/test-soos"),
        "T2 must read timeout_ms from /etc/pam.d/test-soos:\n{t2}"
    );
    assert!(
        code.contains("timeout_mock_delay_ms"),
        "T2 must derive the mock delay from the stack timeout:\n{t2}"
    );
    assert!(
        code.contains("assert_elapsed_within_deadline"),
        "T2 must assert the elapsed time against the deadline:\n{t2}"
    );
    assert!(
        !code.contains("--delay 0.5"),
        "T2 must not hard-code the mock delay:\n{t2}"
    );
    let t2b = suite_case(&suite, "T2b");
    let code = code_lines(t2b).join("\n");
    assert!(
        code.contains("assert_no_facial_authorization test-soos"),
        "T2b must assert that a late Allow never authenticates without a password:\n{t2b}"
    );
    assert!(
        code.contains("assert_elapsed_within_deadline"),
        "T2b must assert the deadline too:\n{t2b}"
    );
}

/// #189 — T6 (distribution stack) and T8 (absent module) assert real outcomes: no `warn`
/// escape hatch, every failed expectation exits non-zero.
#[test]
fn test_pam_matrix_t6_and_t8_can_fail() {
    let root = workspace_root();
    let suite = fs::read_to_string(root.join("tests/docker/test_suite.sh")).expect("read suite");
    for id in ["T6", "T8"] {
        let case = suite_case(&suite, id);
        let code = code_lines(case);
        assert!(
            !code.iter().any(|l| l.trim_start().starts_with("warn ")),
            "{id} must not downgrade a failed expectation to a warning:\n{case}"
        );
        assert!(
            code.iter().filter(|l| l.contains("exit 1")).count() >= 3,
            "{id} must fail on facial, valid-password and wrong-password expectations:\n{case}"
        );
        assert!(
            code.iter().any(|l| l.contains("wrong_password")),
            "{id} must assert that a wrong password is rejected:\n{case}"
        );
    }
    let t6 = code_lines(suite_case(&suite, "T6")).join("\n");
    assert!(
        t6.contains("pam_test_runner \"${DISTRO_SERVICE}\" testuser; then"),
        "T6 must assert facial authentication (no password) on the distribution stack:\n{t6}"
    );
    let t8 = code_lines(suite_case(&suite, "T8")).join("\n");
    assert!(
        t8.contains("assert_no_facial_authorization test-soos"),
        "T8 must assert that the stack never authenticates without the module and a password:\n{t8}"
    );
}

/// #189 — every behavior the mock daemon can simulate is exercised by the PAM matrix.
#[test]
fn test_pam_matrix_exercises_every_mock_daemon_mode() {
    let root = workspace_root();
    let mock = fs::read_to_string(root.join("tests/docker/mock_daemon.py")).expect("read mock");
    let choices_start = mock
        .find("choices=[")
        .expect("mock_daemon.py must declare --mode choices");
    let choices = &mock[choices_start + "choices=[".len()..];
    let choices = &choices[..choices.find(']').expect("closing bracket")];
    let modes: Vec<&str> = choices
        .split(',')
        .map(|m| m.trim().trim_matches('"'))
        .filter(|m| !m.is_empty())
        .collect();
    for required in [
        "allow",
        "deny",
        "timeout",
        "crash-immediate",
        "crash-partial",
        "crash-truncated",
        "malformed",
        "wrong-request-id",
        "bad-version",
        "oversized",
        "empty",
    ] {
        assert!(
            modes.contains(&required),
            "mock_daemon.py must support --mode {required} (modes: {modes:?})"
        );
    }
    let suite = fs::read_to_string(root.join("tests/docker/test_suite.sh")).expect("read suite");
    let code = code_lines(&suite).join("\n");
    let tokens: Vec<&str> = code
        .split(|c: char| c.is_whitespace() || c == ';' || c == '"')
        .collect();
    for mode in &modes {
        assert!(
            code.contains(&format!("--mode {mode}"))
                || (code.contains("--mode \"${") && tokens.contains(mode)),
            "tests/docker/test_suite.sh never exercises mock mode '{mode}'"
        );
    }
}

/// Frames a minimal protocol-v1 `Request` whose request_id is `id` (0xAB bytes, so the
/// mock never classifies it as an `Event`).
fn framed_test_request(id: &[u8; 32]) -> Vec<u8> {
    let mut body = vec![1u8, 0u8];
    body.extend_from_slice(id);
    body.push(0); // uid_hint = 0 (varint)
    body.push(4);
    body.extend_from_slice(b"test");
    body.push(1); // deadline_monotonic_ns (varint)
    let mut frame = u32::try_from(body.len())
        .expect("small body")
        .to_be_bytes()
        .to_vec();
    frame.extend(body);
    frame
}

/// Sends one request to a mock daemon in `mode` and returns every byte it answered.
fn mock_exchange(mode: &str) -> Vec<u8> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let root = workspace_root();
    let dir = scratch_dir(&format!("mock-{mode}"));
    let sock = dir.join("daemon.sock");
    let mut child = Command::new("python3")
        .arg(root.join("tests/docker/mock_daemon.py"))
        .args(["--mode", mode, "--one-shot", "--socket"])
        .arg(&sock)
        .args(["--socket-group", &current_group_name()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn mock_daemon.py");
    let appeared = wait_for_socket(&sock, Duration::from_secs(10));
    let answer = if appeared {
        let mut stream = UnixStream::connect(&sock).expect("connect mock");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("read timeout");
        stream
            .write_all(&framed_test_request(&[0xAB; 32]))
            .expect("write request");
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).expect("read answer");
        Some(answer)
    } else {
        None
    };
    child.kill().ok();
    child.wait().ok();
    fs::remove_dir_all(&dir).ok();
    answer.unwrap_or_else(|| panic!("mock_daemon.py --mode {mode} never created its socket"))
}

/// CLOCK_MONOTONIC as seen by another process (the clock the mock daemon stamps with).
fn monotonic_ns_via_python() -> u64 {
    let out = Command::new("python3")
        .args([
            "-c",
            "import time; print(time.clock_gettime_ns(time.CLOCK_MONOTONIC))",
        ])
        .output()
        .expect("run python3");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("monotonic nanoseconds")
}

/// One decoded mock `Response` frame (codec v1, hand-decoded: the invariants crate has no
/// dependency on `soos-protocol`).
struct MockResponse {
    version: u8,
    request_id: [u8; 32],
    /// First byte of the verdict varint (single-byte for every mode).
    verdict: u8,
    issued: u64,
    expires: u64,
}

fn read_varint(buf: &[u8], idx: &mut usize) -> u64 {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = buf[*idx];
        *idx += 1;
        value |= u64::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
        shift += 7;
        assert!(shift < 64, "varint too long");
    }
}

/// Decodes a complete mock `Response` frame and asserts it is exactly one frame: the
/// declared length matches the bytes sent and the body is consumed exactly (no trailing
/// byte after the two varint stamps).
fn decode_mock_response(mode: &str, frame: &[u8]) -> MockResponse {
    assert!(frame.len() > 4, "{mode} sends a length prefix and a body");
    let declared = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    assert_eq!(frame.len(), 4 + declared, "{mode} is one complete frame");
    let mut request_id = [0u8; 32];
    request_id.copy_from_slice(&frame[5..37]);
    let verdict = frame[37];
    assert!(
        verdict < 0x80,
        "{mode} verdict discriminant is a single byte"
    );
    let mut idx = 38;
    let reason = read_varint(frame, &mut idx);
    assert!(reason < 0x80, "{mode} reason class is a single byte");
    let issued = read_varint(frame, &mut idx);
    let expires = read_varint(frame, &mut idx);
    assert_eq!(idx, frame.len(), "{mode} body is consumed exactly");
    MockResponse {
        version: frame[4],
        request_id,
        verdict,
        issued,
        expires,
    }
}

/// Sends one request in `mode` and asserts the response carries valid daemon-like stamps
/// read during the exchange (`issued` from CLOCK_MONOTONIC, `expires = issued + 2 s`), so
/// the stamps are never a second defect.
fn mock_response_with_fresh_stamps(mode: &str) -> MockResponse {
    let before = monotonic_ns_via_python();
    let frame = mock_exchange(mode);
    let after = monotonic_ns_via_python();
    let resp = decode_mock_response(mode, &frame);
    assert!(
        (before..=after).contains(&resp.issued),
        "{mode}: issued {} must be read from CLOCK_MONOTONIC between {before} and {after}",
        resp.issued
    );
    assert_eq!(
        resp.expires,
        resp.issued + 2_000_000_000,
        "{mode}: expires = issued + 2 s"
    );
    resp
}

/// #189 — the malformed-response modes put exactly one defect on the wire, so each Docker
/// case proves one rejection path of the module (Allow verdicts that must not be honored).
/// GitHub #287 (owner-approved migration 2026-10-01): the mock stamps from CLOCK_MONOTONIC
/// by default, so frames are no longer a fixed 41 bytes; each frame is decoded and its
/// stamps must be valid, which keeps "exactly one defect" true now that the PAM client
/// enforces the expiry.
#[test]
fn test_mock_daemon_malformed_modes_put_one_defect_on_the_wire() {
    let id = [0xABu8; 32];

    let allow = mock_response_with_fresh_stamps("allow");
    assert_eq!(allow.version, 1, "allow version");
    assert_eq!(allow.request_id, id, "allow echoes the request_id");
    assert_eq!(allow.verdict, 0, "allow verdict");

    let deny = mock_response_with_fresh_stamps("deny");
    assert_eq!(deny.request_id, id, "deny echoes the request_id");
    assert_eq!(deny.verdict, 1, "deny verdict is Verdict::Deny");

    let wrong = mock_response_with_fresh_stamps("wrong-request-id");
    assert_eq!(wrong.version, 1, "wrong-request-id keeps version 1");
    assert_ne!(
        wrong.request_id, id,
        "wrong-request-id must not echo the id"
    );
    assert_eq!(
        wrong.verdict, 0,
        "wrong-request-id carries an Allow verdict"
    );

    let version = mock_response_with_fresh_stamps("bad-version");
    assert_ne!(
        version.version, 1,
        "bad-version must not be protocol version 1"
    );
    assert_eq!(version.request_id, id, "bad-version echoes the id");
    assert_eq!(version.verdict, 0, "bad-version carries an Allow verdict");

    let malformed = mock_response_with_fresh_stamps("malformed");
    assert_eq!(malformed.version, 1, "malformed keeps version 1");
    assert_eq!(malformed.request_id, id, "malformed echoes the id");
    assert!(
        malformed.verdict > 3 && malformed.verdict < 0x80,
        "malformed must carry a single-byte, undecodable verdict discriminant"
    );

    // GitHub #287: the only defect of `expired` is its closed issued/expires window.
    let before = monotonic_ns_via_python();
    let expired = decode_mock_response("expired", &mock_exchange("expired"));
    assert_eq!(expired.version, 1, "expired keeps version 1");
    assert_eq!(expired.request_id, id, "expired echoes the id");
    assert_eq!(expired.verdict, 0, "expired carries an Allow verdict");
    assert!(
        expired.issued > 0 && expired.issued <= expired.expires,
        "expired keeps a consistent, stamped window"
    );
    assert!(
        expired.expires < before,
        "expired must send a window that closed before the request"
    );

    let truncated = mock_exchange("crash-truncated");
    assert!(
        truncated.len() >= 4,
        "crash-truncated sends a length prefix"
    );
    let declared = u32::from_be_bytes([truncated[0], truncated[1], truncated[2], truncated[3]]);
    assert!(
        (truncated.len() - 4) < usize::try_from(declared).expect("u32 fits"),
        "crash-truncated must send fewer body bytes than it declares"
    );

    let oversized = mock_exchange("oversized");
    assert_eq!(oversized.len(), 4, "oversized sends only a length prefix");
    assert!(
        u32::from_be_bytes([oversized[0], oversized[1], oversized[2], oversized[3]]) > 4096,
        "oversized must declare more than MAX_MESSAGE_SIZE (4096) bytes"
    );

    let empty = mock_exchange("empty");
    assert_eq!(
        empty,
        0u32.to_be_bytes().to_vec(),
        "empty declares a zero-length body"
    );
}

/// #189 — Deny (T13), truncated (T14) and malformed responses (T15) never authenticate
/// without a password and always leave the password fallback working.
#[test]
fn test_pam_matrix_rejection_cases_assert_no_authorization() {
    let root = workspace_root();
    let suite = fs::read_to_string(root.join("tests/docker/test_suite.sh")).expect("read suite");
    for id in ["T13", "T14", "T15"] {
        let case = suite_case(&suite, id);
        let code = code_lines(case).join("\n");
        assert!(
            code.contains("assert_no_facial_authorization"),
            "{id} must assert that the verdict never authenticates without a password:\n{case}"
        );
        assert!(
            code.contains("assert_password_fallback"),
            "{id} must assert the password fallback:\n{case}"
        );
    }
    let t15 = code_lines(suite_case(&suite, "T15")).join("\n");
    for mode in [
        "malformed",
        "wrong-request-id",
        "bad-version",
        "oversized",
        "empty",
    ] {
        assert!(t15.contains(mode), "T15 must exercise mock mode '{mode}'");
    }
    let lib = fs::read_to_string(root.join(PAM_CASE_LIB)).expect("read case lib");
    for helper in [
        "assert_no_facial_authorization()",
        "assert_password_fallback()",
    ] {
        assert!(lib.contains(helper), "{PAM_CASE_LIB} must define {helper}");
    }
}

/// #274 — every release build in the deployment and package tests honours Cargo.lock.
#[test]
fn test_deployment_and_package_tests_build_with_locked() {
    let root = workspace_root();
    let mut scripts = vec![
        "tests/docker/test_packages.sh",
        "tests/docker/test_suite.sh",
    ];
    scripts.extend(DISTRO_SCRIPTS);
    for script in scripts {
        let content = fs::read_to_string(root.join(script)).expect("read script");
        let builds: Vec<&str> = code_lines(&content)
            .into_iter()
            .filter(|l| l.trim_start().starts_with("cargo build"))
            .collect();
        assert!(!builds.is_empty(), "{script} must build the workspace");
        for line in builds {
            assert!(
                line.contains("--locked"),
                "{script} must build with --locked: {line}"
            );
        }
    }
}

/// #274 — a failing `pam-auth-update` is never ignored by the Debian deployment test, and
/// the generated common-auth is asserted to contain the soos password-failed hook.
#[test]
fn test_debian_deployment_never_ignores_pam_auth_update() {
    let root = workspace_root();
    let content = fs::read_to_string(root.join("tests/distro/debian_ubuntu_test.sh"))
        .expect("read debian test");
    let code = code_lines(&content);
    let calls: Vec<&&str> = code
        .iter()
        .filter(|l| l.contains("pam-auth-update --package"))
        .collect();
    assert!(
        !calls.is_empty(),
        "debian_ubuntu_test.sh must run pam-auth-update"
    );
    for line in calls {
        assert!(
            !line.contains("|| true"),
            "pam-auth-update failure must fail the deployment test: {line}"
        );
    }
    assert!(
        code.iter()
            .any(|l| l.contains("pam_soos.so event=password-failed")
                && l.contains("/etc/pam.d/common-auth")),
        "debian_ubuntu_test.sh must assert the generated common-auth contains the hook"
    );
}

/// #274 — the package harness fails closed: missing packaging tooling or an unknown
/// distribution is an error, never a silent success.
#[test]
fn test_package_harness_fails_closed() {
    let root = workspace_root();
    let content =
        fs::read_to_string(root.join("tests/docker/test_packages.sh")).expect("read harness");
    let code = code_lines(&content);
    assert_eq!(
        code.iter().filter(|l| l.trim() == "exit 0").count(),
        1,
        "test_packages.sh may only exit 0 after every check passed"
    );
    assert!(
        !code
            .iter()
            .any(|l| l.contains("build_packages.sh --dry-run")),
        "an unknown distribution must fail, not fall back to a dry run"
    );
    assert!(
        code.iter().any(|l| l.contains("soos-[0-9]*.rpm")),
        "the RPM branch must select the main package, never soos-debuginfo"
    );
}

/// #274 — the Fedora (RPM, authselect) and Arch (pacman) deployment paths and the RPM /
/// Arch branches of the package harness run in CI on push to main and manual dispatch.
#[test]
fn test_ci_runs_fedora_and_arch_deployment() {
    let root = workspace_root();
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci.yml");
    let job = ci_job(&ci, "distro-deploy");
    assert!(
        job.contains("if: github.event_name == 'push' || github.event_name == 'workflow_dispatch'"),
        "distro-deploy must run on push to main and manual dispatch:\n{job}"
    );
    assert!(job.contains("needs: lint"), "distro-deploy must need lint");
    assert!(
        job.contains("timeout-minutes:"),
        "distro-deploy must set a timeout"
    );
    assert!(
        job.contains("distro: [fedora, arch]"),
        "distro-deploy must cover fedora and arch"
    );
    assert!(
        job.contains("fail-fast: false"),
        "one distribution failing must not hide the other"
    );
    assert!(
        job.contains("uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1"),
        "distro-deploy must use the SHA-pinned checkout action"
    );
    assert!(
        job.contains("persist-credentials: false"),
        "distro-deploy must not persist git credentials"
    );
    assert!(
        job.contains("DISTRO: ${{ matrix.distro }}"),
        "the distribution must reach the steps through env only"
    );
    assert!(
        job.contains("./tests/distro/run_distro_validation.sh \"$DISTRO\""),
        "distro-deploy must run the distribution deployment validation"
    );
    assert!(
        job.contains("tests/docker/test_packages.sh")
            && job.contains("\"soos-distro-target-${DISTRO}\":/workspace/target")
            && job.contains("\"soos-distro-val-${DISTRO}\""),
        "distro-deploy must run the package harness in the same image and target volume"
    );
    assert!(
        !job.contains("${{ github.event"),
        "distro-deploy must not interpolate event data into its steps"
    );
    let mut in_run = false;
    for line in job.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("run:") {
            in_run = true;
        } else if trimmed.starts_with("- name:") || trimmed.starts_with("env:") {
            in_run = false;
        }
        assert!(
            !(in_run && line.contains("${{")),
            "distro-deploy must not interpolate expressions into run: {line}"
        );
    }
    let gate = ci_job(&ci, "ci-success");
    let needs = gate
        .lines()
        .find(|l| l.trim_start().starts_with("needs:"))
        .expect("ci-success needs list");
    assert!(
        !needs.contains("distro-deploy"),
        "distro-deploy is skipped on pull requests and must not be a CI Success dependency"
    );
}
