//! Loopback-only fixture server for the README captures of the phone companion web app
//! (GitHub #381; spec `AI/architect_spec_readme_capture.md` §3.5 and §10). Never part of
//! `soos-remote`, never packaged; driven by `scripts/capture_readme_web.sh`.
//!
//! ```text
//! readme_web_fixture_server --scene <unlocked|locked-alerts> --assets <dir> [--serve-ms N]
//! readme_web_fixture_server --strip-png <in.png> <out.png>
//! ```

use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use soos_test_fixtures::readme_web::{serve, strip_png_metadata, WebScene};

/// Default and bounds of `--serve-ms`.
const DEFAULT_SERVE_MS: u64 = 60_000;
const MIN_SERVE_MS: u64 = 1_000;
const MAX_SERVE_MS: u64 = 300_000;

fn strip(input: &str, output: &str) -> Result<(), String> {
    let bytes = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
    let stripped = strip_png_metadata(&bytes).map_err(|e| format!("{input}: {e}"))?;
    std::fs::write(output, stripped).map_err(|e| format!("{output}: {e}"))
}

fn run(args: &[String]) -> Result<(), String> {
    if let [flag, input, output] = args {
        if flag == "--strip-png" {
            return strip(input, output);
        }
    }
    let mut scene = None;
    let mut assets = None;
    let mut serve_ms = DEFAULT_SERVE_MS;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let value = iter.next().ok_or_else(|| format!("{arg} needs a value"))?;
        match arg.as_str() {
            "--scene" => scene = WebScene::parse(value),
            "--assets" => assets = Some(PathBuf::from(value)),
            "--serve-ms" => {
                serve_ms = value
                    .parse::<u64>()
                    .map_err(|_| "--serve-ms needs a number".to_string())?
                    .clamp(MIN_SERVE_MS, MAX_SERVE_MS);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let scene = scene.ok_or("--scene unlocked|locked-alerts is required")?;
    let assets = assets.ok_or("--assets <dir> is required")?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let mut stdout = std::io::stdout();
    writeln!(stdout, "PORT={port}").map_err(|e| e.to_string())?;
    stdout.flush().map_err(|e| e.to_string())?;
    serve(listener, scene, &assets, Duration::from_millis(serve_ms)).map_err(|e| e.to_string())
}

#[allow(
    clippy::print_stderr,
    reason = "Command-line fixture tool reports its usage error on stderr"
)]
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("readme_web_fixture_server: {e}");
            ExitCode::from(2)
        }
    }
}
