//! Contract (GitHub #398, matrix OBT24): after a `remote.toml` edit, the desktop executor
//! confirms that `soos-remote.service` is still running after `systemctl --user try-restart`.
//!
//! The unit is `Type=exec` (`packaging/soos-remote.service`): `try-restart` returns once the new
//! process is executed, before `soos-remote` parses its configuration. A refused configuration
//! exits 78 and `RestartPreventExitStatus=78` leaves the unit `failed`; a runtime exit puts it in
//! `auto-restart`. The check is bounded, runs on the feature worker thread and never puts raw
//! `systemctl` output in a message.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use soos_gui::feature_control::{
    companion_restart_command, companion_state_command, parse_companion_state, CompanionState,
    FeatureAction, FeatureExecutor, FeatureOutcome, FeaturePaths, ProgramOutput, ProgramRunner,
    RemoteSwitch, RestartCheck, SystemFeatureExecutor, MAX_RESTART_CHECK_INTERVAL,
    MAX_RESTART_CHECK_POLLS, MAX_UNIT_SHOW_BYTES, REMOTE_UNIT, USER_SYSTEMCTL_TIMEOUT,
};
use soos_gui::privileged::SYSTEMCTL_PROGRAM;

/// `(program, args, deadline, max_stdout)`.
type Call = (String, Vec<String>, Option<Duration>, usize);

/// Answers each call from a queue; an empty queue answers success with no output.
#[derive(Default)]
struct ScriptedRunner {
    calls: Mutex<Vec<Call>>,
    answers: Mutex<VecDeque<Result<ProgramOutput, String>>>,
}

impl ScriptedRunner {
    fn with(answers: Vec<Result<ProgramOutput, String>>) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            answers: Mutex::new(answers.into()),
        })
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

impl ProgramRunner for ScriptedRunner {
    fn run(
        &self,
        program: &str,
        args: &[String],
        deadline: Option<Duration>,
        max_stdout: usize,
    ) -> Result<ProgramOutput, String> {
        self.calls
            .lock()
            .unwrap()
            .push((program.to_string(), args.to_vec(), deadline, max_stdout));
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(ok("")))
    }
}

fn ok(stdout: &str) -> ProgramOutput {
    ProgramOutput {
        success: true,
        code: Some(0),
        stdout: stdout.as_bytes().to_vec(),
    }
}

fn exit(code: i32) -> ProgramOutput {
    ProgramOutput {
        success: false,
        code: Some(code),
        stdout: Vec::new(),
    }
}

fn state(active: &str, sub: &str) -> Result<ProgramOutput, String> {
    Ok(ok(&format!("ActiveState={active}\nSubState={sub}\n")))
}

fn running() -> Result<ProgramOutput, String> {
    state("active", "running")
}

/// The restart itself succeeds.
fn restarted() -> Result<ProgramOutput, String> {
    Ok(ok(""))
}

const FAST_TIMEOUT: Duration = Duration::from_millis(400);

fn fast(polls: u32) -> RestartCheck {
    RestartCheck {
        polls,
        interval: Duration::ZERO,
        call_timeout: FAST_TIMEOUT,
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    paths: FeaturePaths,
    config: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let etc = dir.path().join("etc-soos");
    std::fs::create_dir(&etc).unwrap();
    let cfg_dir = dir.path().join("cfg");
    std::fs::create_dir(&cfg_dir).unwrap();
    let config = cfg_dir.join("remote.toml");
    std::fs::write(&config, "allow_unlock = true\n").unwrap();
    let paths = FeaturePaths {
        daemon_config: etc.join("daemon.toml"),
        etc_dir: etc,
        network_lock_rule: dir.path().join("49-soos-network-lock.rules"),
        network_lock_rfkill: dir.path().join("72-soos-network-lock-rfkill.rules"),
        remote_config: Some(config.clone()),
        running_as_root: false,
        uid: 1000,
    };
    Fixture {
        _dir: dir,
        paths,
        config,
    }
}

fn applied(outcome: FeatureOutcome) -> Result<(), String> {
    // Every caller asserts `Ok` or a specific message, so this error never passes a test.
    let FeatureOutcome::Applied { result, .. } = outcome else {
        return Err("expected an applied outcome, got a refresh".to_string());
    };
    result
}

/// Runs `DisableRemote(Unlock)` on a fresh fixture with `check`; returns the result, the calls
/// and the final `remote.toml`.
fn run_disable(
    check: RestartCheck,
    answers: Vec<Result<ProgramOutput, String>>,
) -> (Result<(), String>, Vec<Call>, String) {
    let fx = fixture();
    let runner = ScriptedRunner::with(answers);
    let executor =
        SystemFeatureExecutor::new(fx.paths.clone(), runner.clone()).with_restart_check(check);
    let result = applied(executor.execute(FeatureAction::DisableRemote(RemoteSwitch::Unlock)));
    let text = std::fs::read_to_string(&fx.config).unwrap();
    (result, runner.calls(), text)
}

fn state_call(timeout: Duration) -> Call {
    let (program, args) = companion_state_command();
    (
        program.to_string(),
        args,
        Some(timeout),
        MAX_UNIT_SHOW_BYTES,
    )
}

fn restart_call() -> Call {
    let (program, args) = companion_restart_command();
    (program.to_string(), args, Some(USER_SYSTEMCTL_TIMEOUT), 0)
}

// ---------------------------------------------------------------------------------------------
// Command and parser
// ---------------------------------------------------------------------------------------------

#[test]
fn test_obt_restart_check_state_command_is_a_fixed_user_vector() {
    let (program, args) = companion_state_command();
    assert_eq!(program, SYSTEMCTL_PROGRAM);
    assert_eq!(
        args,
        vec![
            "--user",
            "show",
            "--property=ActiveState",
            "--property=SubState",
            REMOTE_UNIT
        ]
    );
}

#[test]
fn test_obt_restart_check_parse_states() {
    for (text, expected) in [
        (
            "ActiveState=active\nSubState=running\n",
            CompanionState::Running,
        ),
        (
            "SubState=running\nActiveState=active\n",
            CompanionState::Running,
        ),
        (
            "ActiveState=active \nSubState=exited\n",
            CompanionState::Running,
        ),
        (
            "ActiveState=activating\nSubState=start\n",
            CompanionState::Changing,
        ),
        (
            "ActiveState=reloading\nSubState=reload\n",
            CompanionState::Changing,
        ),
        (
            "ActiveState=refreshing\nSubState=running\n",
            CompanionState::Changing,
        ),
        (
            "ActiveState=deactivating\nSubState=stop-sigterm\n",
            CompanionState::Stopping,
        ),
        (
            "ActiveState=activating\nSubState=auto-restart\n",
            CompanionState::AutoRestart,
        ),
        (
            "ActiveState=activating\nSubState=auto-restart-queued\n",
            CompanionState::AutoRestart,
        ),
        (
            "SubState=auto-restart\nActiveState=failed\n",
            CompanionState::AutoRestart,
        ),
        (
            "ActiveState=inactive\nSubState=dead\n",
            CompanionState::Inactive,
        ),
        (
            "ActiveState=failed\nSubState=failed\n",
            CompanionState::Failed,
        ),
        ("ActiveState=failed\n", CompanionState::Failed),
        (
            "ActiveState=maintenance\nSubState=x\n",
            CompanionState::Unknown,
        ),
        ("SubState=running\n", CompanionState::Unknown),
        ("", CompanionState::Unknown),
        ("garbage", CompanionState::Unknown),
        ("XActiveState=active\n", CompanionState::Unknown),
    ] {
        assert_eq!(parse_companion_state(text), expected, "{text:?}");
    }
}

#[test]
fn test_obt_restart_check_default_policy_values() {
    let check = RestartCheck::DEFAULT;
    assert_eq!(check.polls, 3);
    assert_eq!(check.interval, Duration::from_millis(500));
    assert_eq!(check.call_timeout, Duration::from_millis(500));
    assert_eq!(MAX_RESTART_CHECK_POLLS, 10);
    assert_eq!(MAX_RESTART_CHECK_INTERVAL, Duration::from_secs(1));
    // The whole check after the restart stays within about 3 s.
    let after_restart = check.call_timeout + (check.interval + check.call_timeout) * check.polls;
    assert!(
        after_restart <= Duration::from_millis(3500),
        "{after_restart:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------------------------

#[test]
fn test_obt_restart_check_confirms_a_running_unit() {
    let (result, calls, text) = run_disable(
        fast(3),
        vec![running(), restarted(), running(), running(), running()],
    );
    assert_eq!(result, Ok(()));
    assert_eq!(text, "allow_unlock = false\n");
    assert_eq!(
        calls,
        vec![
            state_call(FAST_TIMEOUT),
            restart_call(),
            state_call(FAST_TIMEOUT),
            state_call(FAST_TIMEOUT),
            state_call(FAST_TIMEOUT),
        ]
    );
}

#[test]
fn test_obt_restart_check_skips_polls_when_the_unit_was_not_running() {
    for probe in [
        state("inactive", "dead"),
        state("failed", "failed"),
        state("deactivating", "stop-sigterm"),
    ] {
        let (result, calls, text) = run_disable(fast(3), vec![probe, restarted()]);
        assert_eq!(result, Ok(()));
        assert_eq!(text, "allow_unlock = false\n");
        assert_eq!(calls, vec![state_call(FAST_TIMEOUT), restart_call()]);
    }
}

#[test]
fn test_obt_restart_check_reports_a_unit_that_fails_after_running() {
    let (result, calls, text) = run_disable(
        fast(3),
        vec![
            running(),
            restarted(),
            running(),
            state("failed", "failed"),
            running(),
        ],
    );
    let err = result.unwrap_err();
    assert!(err.starts_with("Saved"), "{err}");
    assert!(err.contains("(failed)"), "{err}");
    assert!(err.contains("journalctl --user -u soos-remote"), "{err}");
    assert_eq!(text, "allow_unlock = false\n");
    // Polling stops at the first stopped state.
    assert_eq!(calls.len(), 4, "{calls:?}");
}

#[test]
fn test_obt_restart_check_reports_auto_restart_and_inactive() {
    for (poll, word) in [
        (state("activating", "auto-restart"), "(restarting)"),
        (state("inactive", "dead"), "(inactive)"),
    ] {
        let (result, calls, _) = run_disable(fast(3), vec![running(), restarted(), poll]);
        let err = result.unwrap_err();
        assert!(err.starts_with("Saved"), "{err}");
        assert!(err.contains(word), "{err}");
        assert_eq!(calls.len(), 3, "{calls:?}");
    }
}

#[test]
fn test_obt_restart_check_reports_an_unconfirmed_unit() {
    // Never `active` at the last poll: changing, unreadable, failed call, or a call error.
    for last in [
        state("activating", "start"),
        state("deactivating", "stop"),
        Ok(ok("nonsense")),
        Ok(exit(1)),
        Err("systemctl did not answer within 400 ms".to_string()),
    ] {
        let (result, calls, _) =
            run_disable(fast(2), vec![running(), restarted(), running(), last]);
        let err = result.unwrap_err();
        assert!(err.starts_with("Saved"), "{err}");
        assert!(err.contains("not confirmed running"), "{err}");
        assert_eq!(calls.len(), 4, "{calls:?}");
    }
    // A late `active` after a changing state is confirmed.
    let (result, _, _) = run_disable(
        fast(2),
        vec![
            running(),
            restarted(),
            state("activating", "start"),
            running(),
        ],
    );
    assert_eq!(result, Ok(()));
}

#[test]
fn test_obt_restart_check_polls_after_an_unknown_probe() {
    // A failed probe is treated as "maybe running": the restart is confirmed by polling.
    let (result, calls, _) = run_disable(fast(1), vec![Ok(exit(1)), restarted(), running()]);
    assert_eq!(result, Ok(()));
    assert_eq!(calls.len(), 3, "{calls:?}");
    let (result, _, _) = run_disable(
        fast(1),
        vec![Err("x".to_string()), restarted(), state("inactive", "dead")],
    );
    assert!(result.unwrap_err().contains("(inactive)"));
}

#[test]
fn test_obt_restart_check_clamps_polls_interval_and_timeout() {
    // 0 polls is one poll.
    let (result, calls, _) = run_disable(fast(0), vec![running(), restarted(), running()]);
    assert_eq!(result, Ok(()));
    assert_eq!(calls.len(), 3, "{calls:?}");
    // 1000 polls is clamped to MAX_RESTART_CHECK_POLLS (the queue answers success with no
    // output afterwards: unknown, so the unit is not confirmed).
    let (result, calls, _) = run_disable(fast(1000), vec![running(), restarted()]);
    assert!(result.is_err());
    assert_eq!(
        calls.len(),
        2 + MAX_RESTART_CHECK_POLLS as usize,
        "{calls:?}"
    );
    // A call timeout above the systemctl deadline is clamped to it.
    let (_, calls, _) = run_disable(
        RestartCheck {
            polls: 1,
            interval: Duration::ZERO,
            call_timeout: Duration::from_secs(3600),
        },
        vec![running(), restarted(), running()],
    );
    assert_eq!(calls[0].2, Some(USER_SYSTEMCTL_TIMEOUT));
    assert_eq!(calls[2].2, Some(USER_SYSTEMCTL_TIMEOUT));
}

#[test]
fn test_obt_restart_check_clamps_the_interval() {
    // An interval far above the bound must not stall the worker: one poll with a 1 h interval
    // finishes within the clamped interval plus slack.
    let fx = fixture();
    let runner = ScriptedRunner::with(vec![running(), restarted(), running()]);
    let executor =
        SystemFeatureExecutor::new(fx.paths.clone(), runner).with_restart_check(RestartCheck {
            polls: 1,
            interval: Duration::from_secs(3600),
            call_timeout: FAST_TIMEOUT,
        });
    let start = std::time::Instant::now();
    let result = applied(executor.execute(FeatureAction::DisableRemote(RemoteSwitch::Unlock)));
    assert_eq!(result, Ok(()));
    assert!(
        start.elapsed() < MAX_RESTART_CHECK_INTERVAL + Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn test_obt_restart_check_restart_failure_runs_no_poll() {
    let (result, calls, text) = run_disable(fast(3), vec![running(), Ok(exit(5))]);
    let err = result.unwrap_err();
    assert!(
        err.starts_with("Saved, but soos-remote did not restart"),
        "{err}"
    );
    assert_eq!(text, "allow_unlock = false\n");
    assert_eq!(calls, vec![state_call(FAST_TIMEOUT), restart_call()]);
}

#[test]
fn test_obt_restart_check_noop_edit_runs_nothing() {
    let fx = fixture();
    std::fs::write(&fx.config, "allow_unlock = false\n").unwrap();
    let runner = ScriptedRunner::with(Vec::new());
    let executor =
        SystemFeatureExecutor::new(fx.paths.clone(), runner.clone()).with_restart_check(fast(3));
    let result = applied(executor.execute(FeatureAction::DisableRemote(RemoteSwitch::Unlock)));
    assert_eq!(result, Ok(()));
    assert!(runner.calls().is_empty());
}

#[test]
fn test_obt_restart_check_errors_carry_no_systemctl_output() {
    const MARKER: &str = "secret-marker-7f3a";
    for poll in [
        Ok(ok(&format!("ActiveState=failed\nSubState={MARKER}\n"))),
        Ok(ok(&format!("ActiveState={MARKER}\n"))),
    ] {
        let (result, _, _) = run_disable(fast(1), vec![running(), restarted(), poll]);
        let err = result.unwrap_err();
        assert!(!err.contains(MARKER), "{err}");
    }
}

#[test]
fn test_obt_restart_check_for_desktop_confirms_the_restart() {
    let fx = fixture();
    let runner = ScriptedRunner::with(vec![running(), restarted(), state("failed", "failed")]);
    let executor = SystemFeatureExecutor::for_desktop(fx.paths.clone(), runner.clone());
    let result = applied(executor.execute(FeatureAction::DisableRemote(RemoteSwitch::Unlock)));
    assert!(result.unwrap_err().contains("(failed)"));
    let calls = runner.calls();
    let default_timeout = RestartCheck::DEFAULT.call_timeout;
    assert_eq!(
        calls,
        vec![
            state_call(default_timeout),
            restart_call(),
            state_call(default_timeout),
        ]
    );
}

fn rust_sources(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn test_obt_restart_check_app_builds_the_desktop_executor() {
    let app = include_str!("../src/app.rs");
    assert_eq!(
        app.matches("SystemFeatureExecutor::for_desktop(").count(),
        1
    );
    // No production source builds the executor without the restart check.
    let mut sources = Vec::new();
    rust_sources(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    assert!(sources.len() > 5, "{sources:?}");
    for path in sources {
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("SystemFeatureExecutor::new("),
            "{} must build the executor with SystemFeatureExecutor::for_desktop",
            path.display()
        );
    }
}

#[test]
fn test_obt_restart_check_sleeps_before_each_poll() {
    let fx = fixture();
    let runner = ScriptedRunner::with(vec![running(), restarted(), running(), running()]);
    let executor =
        SystemFeatureExecutor::new(fx.paths.clone(), runner).with_restart_check(RestartCheck {
            polls: 2,
            interval: Duration::from_millis(50),
            call_timeout: FAST_TIMEOUT,
        });
    let start = std::time::Instant::now();
    let result = applied(executor.execute(FeatureAction::DisableRemote(RemoteSwitch::Unlock)));
    assert_eq!(result, Ok(()));
    assert!(
        start.elapsed() >= Duration::from_millis(100),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn test_obt_restart_check_ignores_the_output_of_a_failed_call() {
    // A state call that exits non-zero is unknown even if it printed `active`.
    let failed_but_active = Ok(ProgramOutput {
        success: false,
        code: Some(1),
        stdout: b"ActiveState=active\nSubState=running\n".to_vec(),
    });
    let (result, _, _) = run_disable(fast(1), vec![running(), restarted(), failed_but_active]);
    assert!(result.unwrap_err().contains("not confirmed running"));
    // Same for the probe: a failed probe that printed `inactive` still leads to polling.
    let failed_but_inactive = Ok(ProgramOutput {
        success: false,
        code: Some(1),
        stdout: b"ActiveState=inactive\nSubState=dead\n".to_vec(),
    });
    let (result, calls, _) =
        run_disable(fast(1), vec![failed_but_inactive, restarted(), running()]);
    assert_eq!(result, Ok(()));
    assert_eq!(calls.len(), 3, "{calls:?}");
}
