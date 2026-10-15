//! `soos-lock-widget`: animated soos face-scan background for swaylock-plugin
//! (`swaylock-plugin --command soos-lock-widget`), see `Docs/LOCKSCREEN_WIDGET.md`.
//!
//! Exit codes: 0 after `SETUP_FAILURE_EXIT_DELAY_MS` when swaylock-plugin closes the surfaces or
//! the connection; 1 after the same delay on a setup error, a lost poller or a
//! caught panic (one stderr line, never the state); 2 after the same delay on a usage error
//! (help and version exit 0 at once). The delays keep swaylock-plugin, which restarts its
//! background program whenever it exits, from restarting a broken widget in a tight loop.

#![forbid(unsafe_code)]

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use soos_lock_widget::args::Args;
use soos_lock_widget::wayland::{run, Exit, WidgetConfig, SETUP_FAILURE_EXIT_DELAY_MS};
use soos_lockstatus::motion::{resolve_motion_prefs, REDUCE_MOTION_ENV};
use soos_lockstatus::shared::runtime_dir_is_private;

fn main() -> ExitCode {
    std::panic::set_hook(Box::new(|_| {}));
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(err) => {
            if matches!(
                err.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                let _ignored = err.print();
                return ExitCode::SUCCESS;
            }
            // A bad `--command` line would otherwise be restarted by swaylock-plugin in a loop.
            std::thread::sleep(Duration::from_millis(SETUP_FAILURE_EXIT_DELAY_MS));
            let _ignored = err.print();
            return ExitCode::from(2);
        }
    };
    let uid = soos_lockstatus::current_uid();
    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    if !runtime_dir_is_private(runtime_dir.as_deref(), uid) {
        report("XDG_RUNTIME_DIR is missing or not private; the scan state is not read");
    }
    let config = WidgetConfig {
        socket: args.socket,
        runtime_dir,
        uid,
        interval_ms: args.interval_ms,
        prefs: resolve_motion_prefs(
            args.reduce_motion,
            std::env::var_os(REDUCE_MOTION_ENV).as_deref(),
        ),
    };
    match std::panic::catch_unwind(move || run(&config)) {
        Ok(Exit::Closed | Exit::Disconnected) => {
            std::thread::sleep(Duration::from_millis(SETUP_FAILURE_EXIT_DELAY_MS));
            ExitCode::SUCCESS
        }
        Ok(Exit::Setup(reason)) => fail(reason),
        Ok(Exit::PollerGone) => fail("the lock state poller stopped"),
        Err(_) => fail("internal error"),
    }
}

fn report(message: &str) {
    let _ignored = writeln!(std::io::stderr(), "soos-lock-widget: {message}");
}

/// Waits `SETUP_FAILURE_EXIT_DELAY_MS` (swaylock-plugin restarts its background program), then
/// reports and exits 1.
fn fail(message: &str) -> ExitCode {
    std::thread::sleep(Duration::from_millis(SETUP_FAILURE_EXIT_DELAY_MS));
    report(message);
    ExitCode::FAILURE
}
