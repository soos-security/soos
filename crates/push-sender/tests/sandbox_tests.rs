//! Contract tests of the push sender's start-up sandbox self-check (soos-remote security
//! hardening S-7, ADR "[2026-10-12] soos-remote Security Hardening"; spec
//! `AI/architect_spec_remote_security_hardening.md` §8, §11.2; rows RSH11, RSH12).
//!
//! Contract:
//! - The home anchor is the running binary's path minus the exact trailing components
//!   `.local/bin/soos-push-sender` (component-wise), absolute, not `/`, without `..`; it is
//!   never read from the environment.
//! - `evaluate_sandbox` is pure. In this order: the anchor resolves (`HomeUnresolved`); some
//!   mount at exactly `/run` is a read-only tmpfs (`RunVisible`); some read-only tmpfs whose
//!   mount point (not `/`) equals or component-wise prefixes the home hides it, and none of
//!   its direct child mounts covers or lies inside the home except the exact binary bind
//!   (`HomeVisible`); `/var/log` is not listable (`LogsReadable`).
//! - Mount points are octal-unescaped; options are matched as exact comma-separated items of
//!   the per-mount options; fstype is exactly `tmpfs`; malformed lines are skipped.
//! - Outside its sandbox the sender exits 78 before binding its socket.
//!
//! Fixtures follow `/proc/self/mountinfo` (proc(5)): mount ID, parent ID, major:minor, root,
//! mount point, per-mount options, optional fields, `-`, fstype, source, super options.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use soos_push_protocol::EXIT_CONFIG;
use soos_push_sender::{
    evaluate_sandbox, home_from_exe, probe_sandbox, SandboxError, MAX_MOUNTINFO_BYTES,
    SANDBOX_HIDDEN_LOG_DIR, SANDBOX_MOUNTINFO_PATH, SENDER_EXE_HOME_RELATIVE,
};

const EXE: &str = "/home/u/.local/bin/soos-push-sender";

/// Mount ID of the `ProtectHome=tmpfs` mount over `/home` in [`unit_table`].
const HOME_TMPFS_ID: u32 = 405;

/// The host's own mounts, still listed below the sandbox's (observed on the owner host).
const HOST_LINES: &str = "\
24 1 259:2 / / rw,relatime shared:1 - btrfs /dev/nvme0n1p2 rw,ssd,space_cache=v2
25 24 0:24 / /run rw,nosuid,nodev,relatime shared:12 - tmpfs tmpfs rw,size=3264220k,mode=755
26 24 259:2 /@home /home rw,relatime shared:30 - btrfs /dev/nvme0n1p2 rw,ssd
27 24 0:6 / /dev rw,nosuid,relatime shared:2 - devtmpfs devtmpfs rw,size=4096k,mode=755
28 24 0:22 / /proc rw,nosuid,nodev,noexec,relatime shared:13 - proc proc rw
29 25 0:51 / /run/user/1000 rw,nosuid,nodev,relatime shared:500 - tmpfs tmpfs rw,size=1632108k,mode=700,uid=1000,gid=1000
30 24 259:2 /@log /var/log rw,relatime shared:31 - btrfs /dev/nvme0n1p2 rw,ssd
";

/// A host without any sandbox (a manual run).
fn host_table() -> String {
    HOST_LINES.to_string()
}

/// The sandbox of `packaging/soos-push-sender.service` for the home `/home/u` (captured
/// layout of spec §1.1), optionally with the resolver drop-in of `Docs/REMOTE_COMPANION.md`
/// §2d (`BindReadOnlyPaths=-/run/systemd/resolve`).
fn unit_table(resolver: bool) -> String {
    let mut table = host_table();
    table.push_str(
        "\
400 24 259:2 / / ro,relatime master:1 - btrfs /dev/nvme0n1p2 rw,ssd
401 400 0:70 / /dev ro,nosuid,noexec,relatime - tmpfs tmpfs ro,mode=755
402 400 0:22 / /proc rw,nosuid,nodev,noexec,relatime master:13 - proc proc rw
403 400 0:71 / /run ro,nosuid,nodev,relatime - tmpfs tmpfs ro,mode=755
404 403 0:51 /soos-push /run/user/1000/soos-push rw,nosuid,nodev,relatime master:500 - tmpfs tmpfs rw,size=1632108k,mode=700,uid=1000,gid=1000
405 400 0:72 / /home ro,nosuid,nodev,relatime - tmpfs tmpfs ro,mode=755
406 405 259:2 /@home/u/.local/bin/soos-push-sender /home/u/.local/bin/soos-push-sender ro,relatime master:30 - btrfs /dev/nvme0n1p2 rw,ssd
407 400 0:73 / /root ro,nosuid,nodev,relatime - tmpfs tmpfs ro,mode=755
408 400 0:74 / /tmp rw,nosuid,nodev - tmpfs tmpfs rw
409 400 0:75 / /var/tmp rw,nosuid,nodev - tmpfs tmpfs rw
410 400 0:76 / /var/log ro,nosuid,nodev,noexec,relatime - tmpfs tmpfs ro,mode=000
411 400 0:76 / /var/lib/systemd/coredump ro,nosuid,nodev,noexec,relatime - tmpfs tmpfs ro,mode=000
",
    );
    if resolver {
        table.push_str(
            "412 403 0:24 /systemd/resolve /run/systemd/resolve ro,nosuid,nodev,relatime master:12 - tmpfs tmpfs rw,size=3264220k,mode=755\n",
        );
    }
    table
}

/// [`unit_table`] without the line whose mount point is exactly `mount_point` (sandbox part
/// only, mount IDs 400 and above).
fn without(table: &str, mount_point: &str) -> String {
    table
        .lines()
        .filter(|line| {
            let fields: Vec<&str> = line.split(' ').collect();
            let id: u32 = fields[0].parse().unwrap_or(0);
            !(id >= 400 && fields.get(4) == Some(&mount_point))
        })
        .map(|l| format!("{l}\n"))
        .collect()
}

/// One mount line with the given IDs, mount point, per-mount options and fstype.
fn line(id: &str, parent: &str, mount_point: &str, options: &str, fstype: &str) -> String {
    format!("{id} {parent} 0:90 / {mount_point} {options} - {fstype} {fstype} rw\n")
}

fn exe(path: &str) -> PathBuf {
    PathBuf::from(path)
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

#[test]
fn test_rsh_sandbox_constants() {
    assert_eq!(SANDBOX_MOUNTINFO_PATH, "/proc/self/mountinfo");
    assert_eq!(MAX_MOUNTINFO_BYTES, 1_048_576);
    assert_eq!(SANDBOX_HIDDEN_LOG_DIR, "/var/log");
    assert_eq!(SENDER_EXE_HOME_RELATIVE, ".local/bin/soos-push-sender");
    assert_eq!(EXIT_CONFIG, 78);
}

// ---------------------------------------------------------------------------
// Accepted and refused tables
// ---------------------------------------------------------------------------

#[test]
fn test_rsh_sandbox_accepts_the_unit_mount_table() {
    for resolver in [false, true] {
        assert_eq!(
            evaluate_sandbox(&unit_table(resolver), &exe(EXE), false),
            Ok(()),
            "resolver drop-in: {resolver}"
        );
    }
    // Order-independent: the sandbox lines first, then the host's.
    let table = unit_table(true);
    let mut reversed: Vec<&str> = table.lines().collect();
    reversed.reverse();
    let reversed: String = reversed.iter().map(|l| format!("{l}\n")).collect();
    assert_eq!(evaluate_sandbox(&reversed, &exe(EXE), false), Ok(()));
}

#[test]
fn test_rsh_sandbox_refuses_host_mount_table() {
    assert_eq!(
        evaluate_sandbox(&host_table(), &exe(EXE), true),
        Err(SandboxError::RunVisible)
    );
    assert_eq!(
        evaluate_sandbox(&host_table(), &exe(EXE), false),
        Err(SandboxError::RunVisible)
    );
    assert_eq!(
        evaluate_sandbox("", &exe(EXE), false),
        Err(SandboxError::RunVisible),
        "an empty mount table proves nothing"
    );
}

#[test]
fn test_rsh_sandbox_requires_ro_tmpfs_run() {
    let base = without(&unit_table(false), "/run");
    for (name, extra) in [
        ("no /run mount", String::new()),
        (
            "rw tmpfs /run",
            line("403", "400", "/run", "rw,nosuid,nodev", "tmpfs"),
        ),
        (
            "ro ext4 /run",
            line("403", "400", "/run", "ro,nosuid,nodev", "ext4"),
        ),
        (
            "ro tmpfs at /run/user only",
            line("403", "400", "/run/user", "ro,nosuid", "tmpfs"),
        ),
        (
            "ro tmpfs at /run2",
            line("403", "400", "/run2", "ro,nosuid", "tmpfs"),
        ),
    ] {
        let table = format!("{base}{extra}");
        assert_eq!(
            evaluate_sandbox(&table, &exe(EXE), false),
            Err(SandboxError::RunVisible),
            "{name}"
        );
    }
    let table = format!("{base}{}", line("403", "400", "/run", "ro", "tmpfs"));
    assert_eq!(evaluate_sandbox(&table, &exe(EXE), false), Ok(()));
}

#[test]
fn test_rsh_sandbox_requires_tmpfs_over_home() {
    let bind_path = "/home/u/.local/bin/soos-push-sender";
    let base = without(&without(&unit_table(false), "/home"), bind_path);
    for (name, extra) in [
        ("no tmpfs over /home", String::new()),
        (
            "rw tmpfs /home",
            line("405", "400", "/home", "rw,nosuid", "tmpfs"),
        ),
        (
            "ro btrfs /home",
            line("405", "400", "/home", "ro,nosuid", "btrfs"),
        ),
        (
            "ro tmpfs /home2 never covers /home/u",
            line("405", "400", "/home2", "ro,nosuid", "tmpfs"),
        ),
        (
            "ro tmpfs /hom is not a component prefix",
            line("405", "400", "/hom", "ro", "tmpfs"),
        ),
        (
            "ro tmpfs over another home",
            line("405", "400", "/home/other", "ro", "tmpfs"),
        ),
        (
            "ro tmpfs over a subdirectory of the home",
            line("405", "400", "/home/u/.local", "ro", "tmpfs"),
        ),
        ("ro tmpfs over /", line("405", "400", "/", "ro", "tmpfs")),
    ] {
        let table = format!("{base}{extra}");
        assert_eq!(
            evaluate_sandbox(&table, &exe(EXE), false),
            Err(SandboxError::HomeVisible),
            "{name}"
        );
    }
    // A read-only tmpfs over /home, or exactly over the home directory, hides it.
    for mount_point in ["/home", "/home/u"] {
        let table = format!(
            "{base}{}{}",
            line("405", "400", mount_point, "ro,nosuid", "tmpfs"),
            line("406", "405", bind_path, "ro", "btrfs")
        );
        assert_eq!(
            evaluate_sandbox(&table, &exe(EXE), false),
            Ok(()),
            "{mount_point}"
        );
    }
}

#[test]
fn test_rsh_sandbox_refuses_listable_logs() {
    assert_eq!(
        evaluate_sandbox(&unit_table(false), &exe(EXE), true),
        Err(SandboxError::LogsReadable)
    );
    // Error order: home anchor, /run, home, logs.
    assert_eq!(
        evaluate_sandbox(&host_table(), &exe("/usr/bin/soos-push-sender"), true),
        Err(SandboxError::HomeUnresolved)
    );
    assert_eq!(
        evaluate_sandbox(&without(&unit_table(false), "/home"), &exe(EXE), true),
        Err(SandboxError::HomeVisible)
    );
    assert_eq!(
        evaluate_sandbox(
            &without(&without(&unit_table(false), "/home"), "/run"),
            &exe(EXE),
            true
        ),
        Err(SandboxError::RunVisible)
    );
}

#[test]
fn test_rsh_sandbox_unescapes_mount_points() {
    // A home directory named "u v": the kernel writes the space as \040.
    let spaced = "/home/u v/.local/bin/soos-push-sender";
    let base = without(
        &without(&unit_table(false), "/home"),
        "/home/u/.local/bin/soos-push-sender",
    );
    let home_tmpfs = line("405", "400", "/home", "ro,nosuid", "tmpfs");
    let bind = line(
        "406",
        "405",
        "/home/u\\040v/.local/bin/soos-push-sender",
        "ro",
        "btrfs",
    );
    let table = format!("{base}{home_tmpfs}{bind}");
    assert_eq!(evaluate_sandbox(&table, &exe(spaced), false), Ok(()));
    // The home itself bound back on top (`BindPaths=%h`), escaped: refused only when the
    // mount point is unescaped before the comparison.
    let reexposed = format!(
        "{table}{}",
        line("413", "405", "/home/u\\040v", "rw", "btrfs")
    );
    assert_eq!(
        evaluate_sandbox(&reexposed, &exe(spaced), false),
        Err(SandboxError::HomeVisible)
    );
    // Tab (\011), newline (\012) and backslash (\134) are unescaped as well.
    for (dir, escaped) in [
        ("a\tb", "a\\011b"),
        ("a\nb", "a\\012b"),
        ("a\\b", "a\\134b"),
    ] {
        let home_exe = format!("/home/{dir}/.local/bin/soos-push-sender");
        let table = format!(
            "{base}{home_tmpfs}{}{}",
            line(
                "406",
                "405",
                &format!("/home/{escaped}/.local/bin/soos-push-sender"),
                "ro",
                "btrfs"
            ),
            line("413", "405", &format!("/home/{escaped}"), "rw", "btrfs")
        );
        assert_eq!(
            evaluate_sandbox(&table, &exe(&home_exe), false),
            Err(SandboxError::HomeVisible),
            "{escaped}"
        );
    }
    // An escaped /run mount point is not /run.
    let run_escaped = format!(
        "{}{}",
        without(&unit_table(false), "/run"),
        line("403", "400", "/run\\040", "ro", "tmpfs")
    );
    assert_eq!(
        evaluate_sandbox(&run_escaped, &exe(EXE), false),
        Err(SandboxError::RunVisible)
    );
}

#[test]
fn test_rsh_sandbox_option_match_is_exact() {
    let base = without(&unit_table(false), "/run");
    for options in [
        "rw,nosuid,nodev",
        "nosuid,rootcontext=system_u:object_r:var_run_t:s0",
        "rw,rootcontext=ro",
        "rw,romode",
        "rw,nosuid,errors=remount-ro",
        "xro",
        "ro2",
    ] {
        let table = format!("{base}{}", line("403", "400", "/run", options, "tmpfs"));
        assert_eq!(
            evaluate_sandbox(&table, &exe(EXE), false),
            Err(SandboxError::RunVisible),
            "{options}"
        );
    }
    // `ro` only in the super options (after `-`) is not a read-only mount.
    let table = format!("{base}403 400 0:71 / /run rw,nosuid,nodev - tmpfs tmpfs ro,mode=755\n");
    assert_eq!(
        evaluate_sandbox(&table, &exe(EXE), false),
        Err(SandboxError::RunVisible)
    );
    // The fstype is exactly `tmpfs`.
    for fstype in ["devtmpfs", "tmpfs2", "ramfs"] {
        let table = format!("{base}{}", line("403", "400", "/run", "ro", fstype));
        assert_eq!(
            evaluate_sandbox(&table, &exe(EXE), false),
            Err(SandboxError::RunVisible),
            "{fstype}"
        );
    }
    // `ro` anywhere in the list, with optional fields present.
    for options in ["nosuid,ro", "ro", "nodev,ro,relatime"] {
        let table =
            format!("{base}403 400 0:71 / /run {options} shared:9 master:2 - tmpfs tmpfs rw\n");
        assert_eq!(
            evaluate_sandbox(&table, &exe(EXE), false),
            Ok(()),
            "{options}"
        );
    }
}

// ---------------------------------------------------------------------------
// Home anchor (PE-SR1)
// ---------------------------------------------------------------------------

#[test]
fn test_rsh_sandbox_home_anchor_unresolved_refused() {
    assert_eq!(home_from_exe(Path::new(EXE)), Ok(PathBuf::from("/home/u")));
    assert_eq!(
        home_from_exe(Path::new("/var/home/u/.local/bin/soos-push-sender")),
        Ok(PathBuf::from("/var/home/u"))
    );
    for refused in [
        "home/u/.local/bin/soos-push-sender",
        "/.local/bin/soos-push-sender",
        ".local/bin/soos-push-sender",
        "/usr/bin/soos-push-sender",
        "/home/u/.local/bin/soos-push-sender (deleted)",
        "/home/u/x.local/bin/soos-push-sender",
        "/home/u/.local/bin/soos-remote",
        "/home/u/.local/sbin/soos-push-sender",
        "/home/../u/.local/bin/soos-push-sender",
        "/home/u/../.local/bin/soos-push-sender",
        "/",
        "",
    ] {
        assert_eq!(
            home_from_exe(Path::new(refused)),
            Err(SandboxError::HomeUnresolved),
            "{refused:?}"
        );
    }
    // The anchor is checked before any other rule.
    for table in [host_table(), unit_table(false), String::new()] {
        assert_eq!(
            evaluate_sandbox(&table, &exe("/usr/bin/soos-push-sender"), true),
            Err(SandboxError::HomeUnresolved)
        );
        assert_eq!(
            evaluate_sandbox(
                &table,
                &exe("/home/u/.local/bin/soos-push-sender (deleted)"),
                false
            ),
            Err(SandboxError::HomeUnresolved)
        );
    }
}

#[test]
fn test_rsh_sandbox_home_outside_home_dir() {
    let base = without(
        &without(&unit_table(false), "/home"),
        "/home/u/.local/bin/soos-push-sender",
    );
    // Image-based layout: /home -> /var/home, the tmpfs at the resolved /var/home.
    let var_home = format!(
        "{base}{}{}",
        line("405", "400", "/var/home", "ro,nosuid", "tmpfs"),
        line(
            "406",
            "405",
            "/var/home/u/.local/bin/soos-push-sender",
            "ro",
            "btrfs"
        )
    );
    let var_exe = exe("/var/home/u/.local/bin/soos-push-sender");
    assert_eq!(evaluate_sandbox(&var_home, &var_exe, false), Ok(()));
    // The tmpfs only at /home does not hide /var/home/u.
    let only_home = format!(
        "{base}{}{}",
        line("405", "400", "/home", "ro,nosuid", "tmpfs"),
        line(
            "406",
            "400",
            "/var/home/u/.local/bin/soos-push-sender",
            "ro",
            "btrfs"
        )
    );
    assert_eq!(
        evaluate_sandbox(&only_home, &var_exe, false),
        Err(SandboxError::HomeVisible)
    );
    // A home outside every ProtectHome= path stays visible: refused.
    let srv = format!(
        "{base}{}",
        line("405", "400", "/home", "ro,nosuid", "tmpfs")
    );
    assert_eq!(
        evaluate_sandbox(&srv, &exe("/srv/u/.local/bin/soos-push-sender"), false),
        Err(SandboxError::HomeVisible)
    );
    // root's home under its own read-only tmpfs.
    let root_home = format!(
        "{}{}",
        without(&base, "/root"),
        line("407", "400", "/root", "ro,nosuid", "tmpfs")
    );
    let root_home = format!(
        "{root_home}{}",
        line(
            "406",
            "407",
            "/root/.local/bin/soos-push-sender",
            "ro",
            "btrfs"
        )
    );
    assert_eq!(
        evaluate_sandbox(&root_home, &exe("/root/.local/bin/soos-push-sender"), false),
        Ok(())
    );
}

// ---------------------------------------------------------------------------
// Re-exposure on top of the hidden home (PE-SR9)
// ---------------------------------------------------------------------------

#[test]
fn test_rsh_sandbox_refuses_mount_over_hidden_home() {
    let parent = HOME_TMPFS_ID.to_string();
    for (name, mount_point) in [
        ("BindPaths=%h", "/home/u"),
        ("a bind over /home itself", "/home"),
        ("BindReadOnlyPaths=%h/.config", "/home/u/.config"),
        (
            "a bind deep inside the home",
            "/home/u/.local/share/keyrings",
        ),
    ] {
        let table = format!(
            "{}{}",
            unit_table(true),
            line("413", &parent, mount_point, "ro,relatime", "btrfs")
        );
        assert_eq!(
            evaluate_sandbox(&table, &exe(EXE), false),
            Err(SandboxError::HomeVisible),
            "{name}"
        );
    }
    // The same mount point below a host mount (parent is not the home tmpfs): unchanged.
    let host_child = format!(
        "{}{}",
        unit_table(true),
        line("413", "26", "/home/u", "rw,relatime", "btrfs")
    );
    assert_eq!(evaluate_sandbox(&host_child, &exe(EXE), false), Ok(()));
    // A child of the tmpfs outside the home (another user's directory): unchanged.
    let other = format!(
        "{}{}",
        unit_table(true),
        line("413", &parent, "/home/other", "ro", "btrfs")
    );
    assert_eq!(evaluate_sandbox(&other, &exe(EXE), false), Ok(()));
    // A child of the tmpfs whose path only shares a text prefix with the home.
    let sibling = format!(
        "{}{}",
        unit_table(true),
        line("413", &parent, "/home/u2", "ro", "btrfs")
    );
    assert_eq!(evaluate_sandbox(&sibling, &exe(EXE), false), Ok(()));
}

#[test]
fn test_rsh_sandbox_binary_bind_is_the_only_allowed_child() {
    assert_eq!(
        evaluate_sandbox(&unit_table(false), &exe(EXE), false),
        Ok(())
    );
    let parent = HOME_TMPFS_ID.to_string();
    for mount_point in [
        "/home/u/.local/bin",
        "/home/u/.local/bin/soos-push-sender2",
        "/home/u/.local",
        "/home/u/.local/bin/soos-push-sender/x",
    ] {
        let table = format!(
            "{}{}",
            unit_table(false),
            line("413", &parent, mount_point, "ro", "btrfs")
        );
        assert_eq!(
            evaluate_sandbox(&table, &exe(EXE), false),
            Err(SandboxError::HomeVisible),
            "{mount_point}"
        );
    }
    // Lines whose ID fields do not parse are malformed and skipped.
    for malformed in [
        format!("x {parent} 0:90 / /home/u rw - btrfs /dev/x rw\n"),
        "413 y 0:90 / /home/u rw - btrfs /dev/x rw\n".to_string(),
        "-1 405 0:90 / /home/u rw - btrfs /dev/x rw\n".to_string(),
        "413 405\n".to_string(),
        "garbage\n".to_string(),
    ] {
        let table = format!("{}{malformed}", unit_table(false));
        assert_eq!(
            evaluate_sandbox(&table, &exe(EXE), false),
            Ok(()),
            "{malformed:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// I/O wrapper and binary (RSH12)
// ---------------------------------------------------------------------------

/// The test process runs from `target/`, never from `~/.local/bin/soos-push-sender`, and
/// under no sandbox: the probe refuses.
#[test]
fn test_rsh_probe_sandbox_refuses_an_unsandboxed_process() {
    assert!(probe_sandbox().is_err());
}

/// The built binary, run by hand outside its unit, exits 78 before binding its socket.
#[test]
fn test_rsh_sender_binary_exits_78_outside_its_sandbox() {
    if nix::unistd::getuid().is_root() {
        return;
    }
    let runtime = tempfile::tempdir().expect("tempdir");
    let mut child = Command::new(env!("CARGO_BIN_EXE_soos-push-sender"))
        .env_clear()
        .env("XDG_RUNTIME_DIR", runtime.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the sender");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break Some(status);
        }
        if started.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let status = status.expect("the sender must exit on its own outside its sandbox");
    assert_eq!(status.code(), Some(EXIT_CONFIG));
    assert!(
        !runtime.path().join("soos-push").exists(),
        "no socket directory is created before the sandbox check"
    );
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut stderr).unwrap();
    assert!(
        stderr.contains("push sender sandbox not detected; refusing to start"),
        "{stderr}"
    );
}
