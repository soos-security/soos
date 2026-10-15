//! Additive contract from the Phase 3 audit (GitHub #379, `AI/auditor_constraints_onboarding_tour.md`
//! constraints 2, 3, 6 and advisory A4; matrix OBT19–OBT22).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{wait_until, SETTLE_TIMEOUT};
use soos_gui::feature_control::{
    FeatureAction, FeatureExecutor, FeatureOutcome, FeatureRunner, ProgramRunner, SystemFlag,
    SystemProgramRunner,
};
use soos_gui::onboarding::owned_by_euid;

fn euid() -> u32 {
    nix::unistd::geteuid().as_raw()
}

#[test]
fn test_obt_state_file_is_written_only_in_the_users_own_tree() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("cfg").join("soos").join("gui-tour.json");
    // The deepest existing ancestor is the temporary directory, owned by this user.
    assert!(owned_by_euid(&missing, euid()));
    // Any other effective uid (a root session with the user's HOME, or the reverse) is refused.
    assert!(!owned_by_euid(&missing, euid().wrapping_add(1)));
    let file = dir.path().join("gui-tour.json");
    std::fs::write(&file, b"{}").unwrap();
    assert!(owned_by_euid(&file, euid()));
    assert!(!owned_by_euid(&file, euid().wrapping_add(1)));
}

#[test]
fn test_obt_mark_seen_checks_ownership_before_creating_anything() {
    let src = include_str!("../src/onboarding.rs");
    let mark = src.find("pub fn mark_seen").expect("mark_seen exists");
    let body = &src[mark..];
    let check = body
        .find("owned_by_euid(")
        .expect("ownership check in mark_seen");
    let create = body
        .find("DirBuilder::new()")
        .expect("directory creation in mark_seen");
    assert!(
        check < create,
        "the ownership check must precede any creation"
    );
    let app = include_str!("../src/app.rs");
    assert!(
        app.contains("owned_by_euid("),
        "the app never uses a tour store in another user's tree"
    );
}

struct PanickingExecutor;

impl FeatureExecutor for PanickingExecutor {
    fn execute(&self, _action: FeatureAction) -> FeatureOutcome {
        panic!("executor failure");
    }
}

#[test]
fn test_obt_dead_worker_reports_a_failed_change() {
    let mut runner = FeatureRunner::new(Arc::new(PanickingExecutor), Arc::new(|| {}));
    let action = FeatureAction::SetSystemFlag {
        flag: SystemFlag::Presence,
        present: true,
    };
    runner.submit(action).unwrap();
    let mut outcomes = Vec::new();
    assert!(wait_until(SETTLE_TIMEOUT, || {
        outcomes.extend(runner.poll());
        !runner.is_busy()
    }));
    assert!(
        matches!(
            outcomes.as_slice(),
            [FeatureOutcome::Applied { action: a, result: Err(_) }] if *a == action
        ),
        "{outcomes:?}"
    );
    assert!(!runner.privileged_in_flight());
}

#[test]
fn test_obt_every_task_submission_goes_through_submit_task() {
    let app = include_str!("../src/app.rs");
    assert_eq!(
        app.matches("self.tasks.submit(").count(),
        1,
        "TaskRunner::submit must be called only from submit_task"
    );
    let wrapper = app.find("fn submit_task(").expect("submit_task exists");
    let call = app.find("self.tasks.submit(").unwrap();
    let next_fn = app[wrapper + 1..]
        .find("\n    fn ")
        .map(|i| i + wrapper + 1);
    assert!(call > wrapper && next_fn.is_none_or(|end| call < end));
    let body = &app[wrapper..call];
    assert!(
        body.contains("privileged_in_flight()"),
        "submit_task refuses while a feature Polkit action runs"
    );
}

#[test]
fn test_obt_program_runner_kills_and_reaps_after_the_deadline() {
    let sleep = "/usr/bin/sleep";
    if !std::path::Path::new(sleep).exists() {
        return;
    }
    let start = Instant::now();
    let result = SystemProgramRunner.run(
        sleep,
        &["5".to_string()],
        Some(Duration::from_millis(100)),
        64,
    );
    assert!(result.is_err(), "{result:?}");
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "the child must be killed at the deadline"
    );
    let ok = SystemProgramRunner
        .run("/usr/bin/true", &[], Some(Duration::from_secs(5)), 64)
        .unwrap();
    assert!(ok.success);
}
