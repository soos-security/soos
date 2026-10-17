//! `soos-lockstatus`: prints the coarse soos face-scan state of the current user for a screen
//! locker (`Docs/LOCKSCREEN_WIDGET.md`).
//!
//! `--once` is run by hyprlock `cmd[update:N]` labels, which render stdout verbatim: it always
//! exits 0, never writes to stderr and finishes within `ONCE_EXIT_BUDGET_MS`; every daemon or transport
//! failure prints the Idle rendering, and a usage error with `--once` prints nothing. `--watch` prints one JSON line per change until stdout closes.

#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::io::Write;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use soos_lockstatus::args::Args;
use soos_lockstatus::client::monotonic_now_ms;
use soos_lockstatus::format::once_output;
use soos_lockstatus::motion::{resolve_motion_prefs, MotionPrefs, REDUCE_MOTION_ENV};
use soos_lockstatus::poll::{output_closed, watch, Poller};
use soos_lockstatus::shared::{boottime_now_ms, shared_fetch};
use soos_lockstatus::view::LockView;
use soos_lockstatus::EXCHANGE_BUDGET_MS;

fn main() -> ExitCode {
    // No panic message may reach stderr (hyprlock and the desktop adapters read our streams).
    std::panic::set_hook(Box::new(|_| {}));
    let raw: Vec<OsString> = std::env::args_os().collect();
    let once_requested = raw.iter().skip(1).any(|arg| arg == "--once");
    let args = match Args::try_parse_from(&raw) {
        Ok(args) => args,
        Err(err) => return usage_error(&err, once_requested),
    };
    if args.once {
        let format = args.format;
        let text = std::panic::catch_unwind(move || run_once(&args)).unwrap_or_else(|_| {
            once_output(format, &LockView::UNAVAILABLE, 0, MotionPrefs::default())
        });
        let mut stdout = std::io::stdout().lock();
        // A closed stdout is not worth reporting: the locker went away.
        let _ignored = stdout
            .write_all(text.as_bytes())
            .and_then(|()| stdout.flush());
        return ExitCode::SUCCESS;
    }
    if std::panic::catch_unwind(move || run_watch(&args)).is_ok() {
        ExitCode::SUCCESS
    } else {
        let _ignored = writeln!(std::io::stderr(), "soos-lockstatus: internal error");
        ExitCode::FAILURE
    }
}

/// Help and version go to stdout as usual; any other parse error is reported on stderr, except
/// with `--once`, which prints nothing and exits 0 (a configuration mistake never reaches
/// hyprlock's stderr).
fn usage_error(err: &clap::Error, once_requested: bool) -> ExitCode {
    use clap::error::ErrorKind;
    if matches!(
        err.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
    ) {
        let _ignored = err.print();
        return ExitCode::SUCCESS;
    }
    if once_requested {
        return ExitCode::SUCCESS;
    }
    let _ignored = err.print();
    ExitCode::from(2)
}

fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
}

fn run_once(args: &Args) -> String {
    let outcome = shared_fetch(
        runtime_dir().as_deref(),
        &args.socket,
        soos_lockstatus::current_uid(),
        std::time::Duration::from_millis(EXCHANGE_BUDGET_MS),
        &boottime_now_ms,
    );
    let prefs = resolve_motion_prefs(
        args.reduce_motion,
        std::env::var_os(REDUCE_MOTION_ENV).as_deref(),
    );
    once_output(
        args.format,
        &outcome.view,
        monotonic_now_ms().unwrap_or(0),
        prefs,
    )
}

fn run_watch(args: &Args) {
    let mut poller = Poller::new(
        args.socket.clone(),
        runtime_dir(),
        soos_lockstatus::current_uid(),
        args.interval_ms,
    );
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    // Ends when the reader closes stdout: detected before every poll (a stable state writes
    // nothing) or by the first failed write. Both are a normal end.
    let _closed = watch(&mut poller, &mut out, &|| !output_closed(stdout.as_fd()));
}
