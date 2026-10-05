//! GitHub #333 — GDM PAM file hardening, enable/status agreement and deterministic fixes of
//! two flaky tests (spec `AI/architect_spec_gdm_hardening_flaky_tests.md`, rows GHF2, GHF6,
//! GHF7, GHF8, GHF9).
//!
//! - GHF2: `gdm enable` checks and reads the GDM file through one descriptor (static checks).
//! - GHF6: the timeout line of `scripts/wait_daemon_ready.sh` prints the `%q` form of the
//!   `--admin` path; no message prints a raw `${ADMIN_BIN}`.
//! - GHF7: the daemon logs the `READY=1` send time in the readiness message and the systemd
//!   harness orders it against `ActiveEnterTimestampMonotonic` (owner approval OA-2).
//! - GHF8: the 0 ms clamp test retries, at most 20 times, only attempts without a request
//!   (owner approval OA-3); its assertion is unchanged.
//! - GHF9: ADR, deployment guide, architecture, daemon doc, project facts, matrix and
//!   walkthrough 180 describe the changes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contract tests utilize direct assertions, panics, indexing and bounded arithmetic"
)]

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::installer_contract::scratch_dir;
use crate::packaging_ownership_contract::combined;

const GDM_RS: &str = "crates/admin-cli/src/gdm.rs";
const WAIT_READY: &str = "scripts/wait_daemon_ready.sh";
const HARNESS: &str = "tests/docker/systemd_unit_acceptance_test.sh";
const DAEMON_MAIN: &str = "crates/daemon/src/main.rs";
const SD_NOTIFY: &str = "crates/daemon/src/sd_notify.rs";
const DEADLINE_TESTS: &str = "crates/admin-cli/tests/cli_deadline_json_tests.rs";

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

/// Source of the Rust function `name` (from `fn name(` to the first line that is exactly `}`).
fn rust_fn<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("fn {name} not found"));
    let rest = &src[start..];
    let end = rest.find("\n}\n").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

/// Body of the shell function `name` (from `name() {` to the first line that is exactly `}`).
fn shell_fn<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src
        .find(&format!("{name}() {{"))
        .unwrap_or_else(|| panic!("{name}() not found"));
    let rest = &src[start..];
    let end = rest.find("\n}\n").map_or(rest.len(), |i| i + 3);
    &rest[..end]
}

/// The text of a Markdown section: from the heading starting with `start` to the next
/// heading starting with `next` (or the end of the document).
fn section<'a>(doc: &'a str, start: &str, next: &str) -> &'a str {
    let from = doc
        .find(start)
        .unwrap_or_else(|| panic!("section '{start}' not found"));
    let rest = &doc[from..];
    let to = rest[start.len()..]
        .find(next)
        .map_or(rest.len(), |i| i + start.len());
    &rest[..to]
}

// ---------------------------------------------------------------------------
// GHF2 — one descriptor for the type check, the size check and the read
// ---------------------------------------------------------------------------

/// GHF2: `ensure_gdm_pam_line_with` no longer checks the path and then reads it again by
/// name; every `O_NOFOLLOW` open of `gdm.rs` also passes `O_NOCTTY` (spec §4.2).
#[test]
fn test_ghf2_enable_reads_the_gdm_file_through_one_descriptor() {
    let src = read_repo(GDM_RS);
    assert!(
        !src.contains("fn regular_file_metadata("),
        "{GDM_RS}: regular_file_metadata (path-based lstat before the read) must be gone"
    );
    let body = rust_fn(&src, "ensure_gdm_pam_line_with");
    for forbidden in [
        "read_bounded_utf8(",
        "symlink_metadata(pam_file",
        "fs::metadata(pam_file",
    ] {
        assert!(
            !body.contains(forbidden),
            "ensure_gdm_pam_line_with must not use `{forbidden}`: the type, size, mode, owner \
             and bytes come from one O_NOFOLLOW descriptor"
        );
    }
    let opens: Vec<&str> = src
        .match_indices("custom_flags(")
        .map(|(i, _)| {
            let line_end = src[i..].find(')').map_or(src.len(), |e| i + e);
            &src[i..line_end]
        })
        .filter(|flags| flags.contains("O_NOFOLLOW"))
        .collect();
    assert!(
        opens.len() >= 2,
        "{GDM_RS}: the PAM file snapshot reader and the backup reader open with O_NOFOLLOW"
    );
    for flags in opens {
        assert!(
            flags.contains("O_NOCTTY") && flags.contains("O_NONBLOCK"),
            "{GDM_RS}: `{flags}` must also pass O_NOCTTY and O_NONBLOCK (spec §4.2, finding 9)"
        );
    }
}

// ---------------------------------------------------------------------------
// GHF6 — %q form of the --admin path in the timeout line
// ---------------------------------------------------------------------------

/// GHF6: an `--admin` path with a space and a `$` is printed in its `printf '%q'` form in
/// the timeout line (and in the `Last ... error:` line), never raw.
#[test]
fn test_ghf6_readiness_timeout_line_quotes_the_admin_path() {
    let work = scratch_dir("ghf6_quote");
    let manifest = work.join("manifest.toml");
    fs::write(&manifest, "[manifest]\nversion = \"2.0.0\"\n").expect("manifest");
    let socket = work.join("daemon.sock");
    let _listener = UnixListener::bind(&socket).expect("bind socket");
    let admin = work.join("soos admin$HOME");
    fs::write(&admin, "#!/bin/sh\necho 'not ready' >&2\nexit 1\n").expect("write admin");
    fs::set_permissions(&admin, fs::Permissions::from_mode(0o755)).expect("chmod admin");

    let quoted = Command::new("bash")
        .arg("-c")
        .arg("printf '%q' \"$1\"")
        .arg("bash")
        .arg(&admin)
        .output()
        .expect("printf %q");
    let quoted = String::from_utf8(quoted.stdout).expect("utf-8");
    let raw = admin.display().to_string();
    assert_ne!(quoted, raw, "the fixture path must need quoting");

    let out = Command::new("bash")
        .arg(workspace_root().join(WAIT_READY))
        .args([
            OsStr::new("--socket"),
            socket.as_os_str(),
            OsStr::new("--manifest"),
            manifest.as_os_str(),
            OsStr::new("--admin"),
            admin.as_os_str(),
            OsStr::new("--timeout"),
            OsStr::new("1"),
        ])
        .output()
        .expect("run wait_daemon_ready.sh");
    let _ = fs::remove_dir_all(&work);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "output:\n{}", combined(&out));
    assert!(
        stderr.contains(&format!(
            "[ERROR] soos-daemon did not report healthy within 1 s ('{quoted} status' kept failing)."
        )),
        "the timeout line must print the %q form of the --admin path; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains(&format!("        Last '{quoted} status' error:")),
        "the last-error line keeps the %q form; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains(&format!("'{raw} status'")),
        "the raw --admin path must never be printed; stderr:\n{stderr}"
    );
}

/// GHF6: no `echo` / `printf` message of the helper prints a raw `${ADMIN_BIN}`.
#[test]
fn test_ghf6_no_message_prints_a_raw_admin_path() {
    let script = read_repo(WAIT_READY);
    let offenders: Vec<&str> = script
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter(|l| l.contains("echo ") || l.contains("printf "))
        .filter(|l| l.contains("${ADMIN_BIN}"))
        .filter(|l| {
            !l.trim_start()
                .starts_with("q_admin=\"$(printf '%q' \"${ADMIN_BIN}\")\"")
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "{WAIT_READY}: messages must print \"${{q_admin}}\", not a raw ${{ADMIN_BIN}}:\n{}",
        offenders.join("\n")
    );
    assert!(
        script.contains(
            "echo \"[ERROR] soos-daemon did not report healthy within ${TIMEOUT_S} s ('${q_admin} status' kept failing).\" >&2"
        ),
        "{WAIT_READY}: the timeout line must be exactly the spec §4.6 text"
    );
}

// ---------------------------------------------------------------------------
// GHF7 — READY=1 send time, deterministic harness ordering (OA-2)
// ---------------------------------------------------------------------------

/// GHF7: the daemon reports readiness through `notify_ready_stamped` and logs the two exact
/// message shapes; `sd_notify.rs` stamps with the daemon's `CLOCK_MONOTONIC` helper.
#[test]
fn test_ghf7_daemon_logs_the_ready_send_time_in_the_message_text() {
    // Owner approval OA-4 (2026-10-05): `sd_notify::notify_ready()` is itself the stamped
    // variant, so main.rs keeps calling it; only the logged message shapes are pinned here.
    let main = read_repo(DAEMON_MAIN);
    for shape in [
        "\"Reported readiness to systemd (ready_sent_monotonic_us={us})\"",
        "\"Reported readiness to systemd (ready_sent_monotonic_us=unknown)\"",
    ] {
        assert!(main.contains(shape), "{DAEMON_MAIN} must contain `{shape}`");
    }
    let sd = read_repo(SD_NOTIFY);
    let stamped = rust_fn(&sd, "notify_to_stamped");
    assert!(
        stamped.contains("current_monotonic_nanos"),
        "notify_to_stamped must read soos_daemon::pipeline::current_monotonic_nanos"
    );
}

/// GHF7 (OA-2): the harness no longer orders the daemon line against PID 1's `Started`
/// line; it checks the send stamp against `ActiveEnterTimestampMonotonic` once read.
#[test]
fn test_ghf7_harness_orders_ready_send_time_against_active_enter_timestamp() {
    let text = read_repo(HARNESS);
    let body = shell_fn(&text, "part2_notify_readiness");
    assert!(
        !body.contains("ready_line < started_line"),
        "part2_notify_readiness() must not order two journald inputs"
    );
    let strip = body
        .find("sed -e 's/\\x1b\\[[0-9;]*m//g'")
        .expect("the harness strips SGR sequences before parsing");
    let parse = body
        .find("(ready_sent_monotonic_us=\\([0-9][0-9]*\\))")
        .expect("the harness parses the numeric send stamp");
    let missing = body
        .find("[[ -n \"${ready_sent_us}\" ]]")
        .expect("the harness fails on a missing or unknown stamp");
    let active = body
        .find("active_us=\"$(prop ActiveEnterTimestampMonotonic)\"")
        .expect("the harness reads ActiveEnterTimestampMonotonic");
    let order = body
        .find("(( ready_sent_us <= active_us ))")
        .expect("the harness requires READY=1 sent no later than active");
    assert!(
        strip < parse && parse < missing && active < order,
        "part2_notify_readiness(): strip -> parse -> presence check, and the order check after active_us is read"
    );
    assert!(
        body.contains("(( listening_line < ready_line ))"),
        "the daemon stream order bind -> readiness stays checked"
    );
    assert!(
        body.contains("grep -q 'Reported readiness to systemd'")
            && body.contains("'^Started soos-daemon.service'"),
        "the readiness and Started presence checks stay"
    );
}

// ---------------------------------------------------------------------------
// GHF8 — bounded retry of the 0 ms clamp test (OA-3)
// ---------------------------------------------------------------------------

/// GHF8: the retry bound is 20, applies to the tolerated mode only, and the clamp
/// assertion is unchanged.
#[test]
fn test_ghf8_clamp_test_retry_is_bounded_and_assertion_unchanged() {
    let src = read_repo(DEADLINE_TESTS);
    assert!(
        src.contains("const MAX_CLAMP_ATTEMPTS: u32 = 20;"),
        "{DEADLINE_TESTS} must bound the retries at 20"
    );
    assert!(
        src.contains(
            "\"no request was written in {MAX_CLAMP_ATTEMPTS} attempts; the clamp could not be observed\""
        ),
        "{DEADLINE_TESTS} must fail once the bound is reached"
    );
    let helper = rust_fn(&src, "capture_deadline_with");
    assert!(
        helper.contains("Completion::Required => 1"),
        "the Required mode keeps exactly one attempt"
    );
    let test = rust_fn(
        &src,
        "test_simulate_pam_auth_zero_timeout_is_clamped_to_pam_minimum",
    );
    for needle in [
        "capture_deadline_with(0, Completion::TimeoutTolerated)",
        "let budget = MIN_TIMEOUT_MS * 1_000_000;",
        "deadline >= before + budget && deadline <= after + budget",
    ] {
        assert!(test.contains(needle), "the clamp test must keep `{needle}`");
    }
    let attempt = rust_fn(&src, "capture_attempt");
    assert!(
        attempt.contains("std::io::ErrorKind::UnexpectedEof")
            && attempt.contains("completion == Completion::TimeoutTolerated"),
        "only an UnexpectedEof in the tolerated mode counts as an attempt without request"
    );
}

// ---------------------------------------------------------------------------
// GHF9 — documentation and traceability
// ---------------------------------------------------------------------------

/// GHF9: ADR, deployment guide §2.1, architecture, daemon doc, project facts, matrix rows
/// and walkthrough 180.
#[test]
fn test_ghf9_docs_and_adr_describe_the_changes() {
    let decisions = read_repo("AI/DECISIONS.md");
    assert!(
        decisions.contains("GitHub #333") && decisions.contains("MAX_PAM_STACK_READS"),
        "AI/DECISIONS.md must carry the GitHub #333 ADR (MAX_PAM_STACK_READS)"
    );
    let guide = read_repo("Docs/DISTRIBUTION_DEPLOYMENT.md");
    let gdm = section(&guide, "### 2.1 GDM Login Integration", "## 3.");
    for needle in [
        "changed concurrently",
        "O_NOFOLLOW",
        "MAX_PAM_STACK_READS",
        "bad jump",
        "pam_deny.so",
    ] {
        assert!(
            gdm.contains(needle),
            "Docs/DISTRIBUTION_DEPLOYMENT.md section 2.1 must mention `{needle}`"
        );
    }
    let architecture = read_repo("AI/ARCHITECTURE.md");
    assert!(
        architecture.contains("#333"),
        "AI/ARCHITECTURE.md GDM paragraph must cite GitHub #333 (agreement, jump-target check)"
    );
    let daemon = read_repo("Docs/DAEMON.md");
    for shape in [
        "ready_sent_monotonic_us=<",
        "ready_sent_monotonic_us=unknown",
    ] {
        assert!(
            daemon.contains(shape),
            "Docs/DAEMON.md must document the readiness message shape `{shape}`"
        );
    }
    let facts = read_repo(".agents/skills/dev-workflow/references/project-facts.md");
    let row = facts
        .lines()
        .find(|l| l.contains("`MAX_PAM_STACK_READS`"))
        .expect("project-facts.md must list MAX_PAM_STACK_READS");
    assert!(
        row.contains("crates/admin-cli/src/pam_stack.rs") && row.contains("32"),
        "project-facts row: {row}"
    );
    let matrix = read_repo("AI/VERIFICATION_MATRIX.md");
    for n in 1..=9 {
        assert!(
            matrix.contains(&format!("| GHF{n} |")),
            "AI/VERIFICATION_MATRIX.md must carry row GHF{n}"
        );
    }
    let sua4 = matrix
        .lines()
        .find(|l| l.starts_with("| SUA4 |"))
        .expect("matrix row SUA4");
    assert!(
        sua4.contains("ActiveEnterTimestampMonotonic"),
        "SUA4 must describe the READY=1 send time check (OA-2): {sua4}"
    );
    assert!(
        workspace_root()
            .join("AI/walkthroughs/180_gdm_hardening_flaky_tests.md")
            .is_file(),
        "walkthrough 180 must exist"
    );
}
