//! Real-systemd acceptance harness for `soos-daemon.service` (GitHub #211, ONB-15; matrix
//! rows IRP8, SUA1–SUA6).
//!
//! `tests/docker/systemd_unit_acceptance_test.sh` boots ubuntu:24.04 with systemd as PID 1 in
//! Docker and drives the shipped unit with `systemctl`. The harness itself needs Docker and runs
//! in the `systemd-unit` CI job; these static checks pin that it exists, rebuilds what it tests,
//! keys on the unit's real values and on log lines the daemon really emits, asserts
//! "condition failed", the start limit and the `READY=1` ordering, and gates `CI Success`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, indexing and bounded arithmetic"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const HARNESS: &str = "tests/docker/systemd_unit_acceptance_test.sh";
const DOCKERFILE: &str = "tests/docker/Dockerfile.systemd";
const UNIT: &str = "packaging/soos-daemon.service";
const DAEMON_MAIN: &str = "crates/daemon/src/main.rs";
const CI: &str = ".github/workflows/ci.yml";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// Non-comment lines of a shell script.
fn code_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect()
}

/// Value of the first uncommented `key=` line of the unit.
fn unit_value(key: &str) -> String {
    read(UNIT)
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| l.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("{UNIT} must set {key}="))
        .to_string()
}

/// Value of a `readonly NAME=value` constant of the harness (quotes stripped).
fn harness_constant(name: &str) -> String {
    read(HARNESS)
        .lines()
        .find_map(|l| l.trim().strip_prefix(&format!("readonly {name}=")))
        .unwrap_or_else(|| panic!("{HARNESS} must define readonly {name}"))
        .trim_matches('"')
        .to_string()
}

/// Body of the shell function `name` (from its definition to the closing `}` in column 0).
fn shell_function<'a>(text: &'a str, name: &str) -> &'a str {
    let start = text
        .find(&format!("\n{name}() {{"))
        .unwrap_or_else(|| panic!("{HARNESS} must define {name}()"));
    let body = &text[start + 1..];
    let end = body.find("\n}\n").expect("function end");
    &body[..end]
}

fn position(haystack: &str, needle: &str, what: &str) -> usize {
    haystack
        .find(needle)
        .unwrap_or_else(|| panic!("{what}: missing `{needle}`"))
}

fn ci_job<'a>(ci: &'a str, name: &str) -> &'a str {
    let start = ci
        .find(&format!("\n  {name}:"))
        .unwrap_or_else(|| panic!("ci.yml must define the {name} job"));
    let job = &ci[start + 1..];
    let end = job.find("\n  # ----").unwrap_or(job.len());
    &job[..end]
}

/// SUA1 — the harness exists, is strict bash, isolates the host, and the image boots systemd
/// as PID 1 without any host camera node.
#[test]
fn test_systemd_acceptance_harness_exists_and_isolates_the_host() {
    let path = workspace_root().join(HARNESS);
    let mode = fs::metadata(&path)
        .unwrap_or_else(|e| panic!("{HARNESS} must exist: {e}"))
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0, "{HARNESS} must be executable");
    let text = read(HARNESS);
    assert!(
        text.starts_with("#!/usr/bin/env bash\n"),
        "{HARNESS} must be a bash script"
    );
    assert!(
        text.contains("\nset -euo pipefail\n"),
        "{HARNESS} must use set -euo pipefail"
    );
    let syntax = Command::new("bash")
        .arg("-n")
        .arg(&path)
        .output()
        .expect("run bash -n");
    assert!(
        syntax.status.success(),
        "bash -n {HARNESS}: {}",
        String::from_utf8_lossy(&syntax.stderr)
    );

    let code = code_lines(&text).join("\n");
    for needle in [
        "--privileged --cgroupns=private",
        "--tmpfs /run --tmpfs /run/lock",
        "-v \"${WORKSPACE_ROOT}:/workspace:ro\"",
        "-v \"${HOST_MODELS_DIR}:${IN_CONTAINER_HOST_MODELS}:ro\"",
        "tests/docker/Dockerfile.systemd",
    ] {
        assert!(code.contains(needle), "{HARNESS} must contain `{needle}`");
    }
    assert!(
        !code.contains("sudo"),
        "{HARNESS} must never use sudo: the host is never modified"
    );
    // The mock camera is the only camera the daemon uses in the container.
    assert!(
        code.contains("'use_mock_camera = true'"),
        "{HARNESS} must configure the mock camera"
    );

    let dockerfile = read(DOCKERFILE);
    assert!(
        dockerfile.contains("\nFROM ubuntu:24.04\n"),
        "{DOCKERFILE} must use ubuntu:24.04 (same glibc as the release build image)"
    );
    assert!(
        dockerfile.contains("rm -rf /dev/video* /dev/media* /dev/v4l && exec /sbin/init"),
        "{DOCKERFILE} must remove the host V4L2 nodes before exec'ing systemd as PID 1"
    );
    assert!(
        !dockerfile.contains("\nCOPY ") && !dockerfile.contains("\nADD "),
        "{DOCKERFILE} must not copy sources (empty build context)"
    );
}

/// SUA2 — the artifacts under test are always rebuilt and installed the way an operator does.
#[test]
fn test_systemd_acceptance_harness_rebuilds_and_installs_like_an_operator() {
    let text = read(HARNESS);
    let code = code_lines(&text);
    assert!(
        code.iter()
            .any(|l| l.trim() == "cargo build --locked --release -p soos-daemon -p soos-admin-cli"),
        "{HARNESS} must always run the locked release build of the daemon and soos-admin"
    );
    assert!(
        !text.contains("skip-build"),
        "{HARNESS} must not offer a way to reuse a stale release artifact"
    );
    assert!(
        code.iter().any(|l| l.contains(
            "bash /workspace/scripts/install.sh --artifact-dir /target/release --allow-missing"
        )),
        "{HARNESS} must install the fresh artifacts with scripts/install.sh"
    );
    assert!(
        code.iter()
            .any(|l| l.contains("cmp -s /workspace/packaging/soos-daemon.service")),
        "{HARNESS} must prove the installed unit is the shipped one"
    );
    assert!(
        code.iter().any(|l| l.contains("systemd-analyze verify")),
        "{HARNESS} must run systemd-analyze verify on the installed unit"
    );
    assert!(
        !code.iter().any(|l| l.contains("soos-daemon.service.d")),
        "{HARNESS} must test the shipped unit without a drop-in override"
    );
}

/// SUA3 — without the models manifest the unit is skipped ("condition failed"), never run and
/// never restarted, both on `systemctl start` and at boot.
#[test]
fn test_systemd_acceptance_harness_asserts_condition_failed() {
    let text = read(HARNESS);
    let manifest = format!("{}/manifest.toml", harness_constant("MODELS_DIR"));
    assert_eq!(
        unit_value("ConditionPathExists"),
        manifest,
        "the harness must remove exactly the path the unit's condition checks"
    );
    assert_eq!(harness_constant("MANIFEST"), "${MODELS_DIR}/manifest.toml");

    for name in [
        "part1_condition_failed_without_models",
        "part1_condition_failed_at_boot",
    ] {
        let body = shell_function(&text, name);
        for needle in [
            "expect_prop ConditionResult \"no\"",
            "expect_prop NRestarts \"0\"",
            "expect_prop ExecMainPID \"0\"",
            "skipped because of an unmet condition check (ConditionPathExists=${MANIFEST})",
            // No daemon execution: zero manager "Starting" lines.
            "-eq 0 ]]",
        ] {
            assert!(body.contains(needle), "{name}() must contain `{needle}`");
        }
    }
    assert!(
        shell_function(&text, "count_starts_since").contains("'^Starting soos-daemon.service'"),
        "count_starts_since() must count the manager's per-execution Starting lines"
    );
    assert!(
        text.contains("\"${DOCKER}\" restart"),
        "{HARNESS} must reboot the container to check the enabled unit at boot"
    );
}

/// SUA4 — a start that always fails ends in the start limit after exactly `StartLimitBurst`
/// runs, with the shipped values.
#[test]
fn test_systemd_acceptance_harness_asserts_start_limit_hit_with_shipped_values() {
    assert_eq!(
        harness_constant("SHIPPED_START_LIMIT_BURST"),
        unit_value("StartLimitBurst"),
        "the harness burst must be the shipped StartLimitBurst"
    );
    assert_eq!(
        harness_constant("SHIPPED_START_LIMIT_INTERVAL_S"),
        unit_value("StartLimitIntervalSec"),
        "the harness interval must be the shipped StartLimitIntervalSec"
    );
    let text = read(HARNESS);
    let body = shell_function(&text, "part3_start_limit_hit");
    for needle in [
        "'Start request repeated too quickly'",
        "start-limit-hit",
        "-eq \"${SHIPPED_START_LIMIT_BURST}\"",
        "expect_prop NRestarts \"${SHIPPED_START_LIMIT_BURST}\"",
        "SHIPPED_START_LIMIT_INTERVAL_S * 1000",
        "'refusing to start daemon (fail-closed)'",
    ] {
        assert!(
            body.contains(needle),
            "part3_start_limit_hit() must contain `{needle}`"
        );
    }
    assert!(
        read(DAEMON_MAIN).contains("refusing to start daemon (fail-closed)"),
        "{DAEMON_MAIN} must still log the fail-closed refusal the harness waits for"
    );
}

/// SUA5 — `systemctl start` returns only after `READY=1`: the socket check comes right after
/// the start, the journal order is bind -> READY=1 -> Started, the readiness time stays well
/// under `TimeoutStartSec`, and the stop is a graceful drain.
#[test]
fn test_systemd_acceptance_harness_asserts_ready_ordering_and_clean_stop() {
    let text = read(HARNESS);
    let body = shell_function(&text, "part2_notify_readiness");
    let what = "part2_notify_readiness()";
    let start = position(body, "systemctl start \"${UNIT}\"", what);
    let socket = position(body, "[[ -S \"${SOCKET}\" ]]", what);
    let mode = position(body, "\"660 root:soos\"", what);
    let active = position(body, "expect_prop ActiveState \"active\"", what);
    assert!(
        start < socket && socket < mode && mode < active,
        "{what} must check the socket and its mode right after systemctl start returned"
    );
    for needle in [
        "listening_line < ready_line && ready_line < started_line",
        "ExecMainStartTimestampMonotonic",
        "ActiveEnterTimestampMonotonic",
        "ready_ms < MAX_READY_MS",
        "scripts/wait_daemon_ready.sh",
        "'\"is_healthy\": true'",
    ] {
        assert!(body.contains(needle), "{what} must contain `{needle}`");
    }

    let timeout_s: u64 = unit_value("TimeoutStartSec")
        .parse()
        .expect("TimeoutStartSec");
    let max_ready_ms: u64 = harness_constant("MAX_READY_MS")
        .parse()
        .expect("MAX_READY_MS");
    assert!(
        max_ready_ms * 2 <= timeout_s * 1000,
        "MAX_READY_MS ({max_ready_ms}) must stay well under TimeoutStartSec={timeout_s}"
    );
    assert_eq!(unit_value("Type"), "notify");
    assert_eq!(unit_value("RuntimeDirectoryMode"), "0750");

    let stop = shell_function(&text, "part5_clean_stop");
    for needle in [
        "expect_prop Result \"success\"",
        "expect_prop ExecMainStatus \"0\"",
        "'Received SIGTERM signal; shutting down gracefully'",
        "'soos-daemon terminated cleanly'",
        "stop_ms < MAX_STOP_MS",
    ] {
        assert!(
            stop.contains(needle),
            "part5_clean_stop() must contain `{needle}`"
        );
    }

    // The harness keys on these daemon log lines; they must exist in production.
    let main = read(DAEMON_MAIN);
    for line in [
        "soos-daemon initialized and listening for PAM requests",
        "Reported readiness to systemd",
        "Received SIGTERM signal; shutting down gracefully",
        "soos-daemon terminated cleanly",
    ] {
        assert!(main.contains(line), "{DAEMON_MAIN} must still log `{line}`");
    }
}

/// SUA6 — the harness runs in CI on every pull request, with the attested models, and gates
/// `CI Success`.
#[test]
fn test_ci_runs_systemd_acceptance_and_gates_on_it() {
    let ci = read(CI);
    let job = ci_job(&ci, "systemd-unit");
    for needle in [
        "if: github.event_name != 'schedule'",
        "needs: lint",
        "timeout-minutes:",
        "uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1",
        "persist-credentials: false",
        "run: ./tests/docker/systemd_unit_acceptance_test.sh --models download",
    ] {
        assert!(
            job.contains(needle),
            "systemd-unit job must contain `{needle}`:\n{job}"
        );
    }
    assert!(
        !job.contains("${{ github.event"),
        "systemd-unit must not interpolate event data into its steps"
    );
    let gate = ci_job(&ci, "ci-success");
    let needs = gate
        .lines()
        .find(|l| l.trim_start().starts_with("needs:"))
        .expect("ci-success needs list");
    assert!(
        needs.contains("systemd-unit"),
        "systemd-unit must be part of the CI Success aggregate: {needs}"
    );
}

/// SUA8: the privileged systemd container never writes host kernel or firmware state. The
/// image masks every boot unit that would apply settings to the host through the writable
/// `/proc/sys` and `/sys` of a `--privileged` container (an unmasked `systemd-sysctl.service`
/// once loosened the host `kernel.sysrq` to the image's 176), removes the TPM and rfkill nodes
/// before init, the harness asserts the masks at run time, and it fails when the host
/// `kernel.*` / `vm.*` / `fs.*` sysctls differ after the run.
#[test]
fn test_systemd_acceptance_container_never_writes_host_state() {
    let dockerfile = read(DOCKERFILE);
    let harness = read(HARNESS);
    for unit in [
        "systemd-sysctl.service",
        "systemd-modules-load.service",
        "systemd-binfmt.service",
        "proc-sys-fs-binfmt_misc.automount",
        "proc-sys-fs-binfmt_misc.mount",
        "systemd-rfkill.service",
        "systemd-rfkill.socket",
        "systemd-backlight@.service",
        "systemd-random-seed.service",
        "systemd-pcrphase.service",
        "systemd-pcrmachine.service",
        "systemd-tpm2-setup-early.service",
        "systemd-tpm2-setup.service",
    ] {
        assert!(
            dockerfile.contains(unit),
            "Dockerfile.systemd must mask {unit} (host isolation under --privileged)"
        );
    }
    assert!(
        dockerfile.contains("ln -sf /dev/null \"/etc/systemd/system/${unit}\" || exit 1"),
        "the host-state units must be masked without `|| true`, so a failure breaks the build"
    );
    assert!(
        dockerfile.contains("/dev/tpm*") && dockerfile.contains("/dev/rfkill"),
        "the TPM and rfkill nodes must be removed before systemd starts"
    );
    let install = harness
        .split("    install)\n")
        .nth(1)
        .expect("the install stage exists");
    let wait = position(install, "wait_for_boot", "install stage boot wait");
    let masked = position(
        install,
        "assert_host_state_units_masked",
        "install stage mask check",
    );
    assert!(
        wait < masked,
        "the install stage must assert the masks right after boot"
    );
    for needle in [
        "HOST_SYSCTL_BEFORE=\"$(host_sysctl_snapshot)\"",
        "host_sysctls_unchanged || fail",
        "grep -E '^(kernel|vm|fs)\\.'",
    ] {
        assert!(
            harness.contains(needle),
            "the harness must snapshot and compare the host sysctls ({needle})"
        );
    }
    let snapshot = position(&harness, "HOST_SYSCTL_BEFORE=", "sysctl snapshot");
    let boot = position(
        &harness,
        "\"${DOCKER}\" run \"${run_args[@]}\"",
        "container boot",
    );
    assert!(
        snapshot < boot,
        "the host sysctls must be snapshotted before the privileged container boots"
    );
}
