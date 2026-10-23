//! Contract tests of the system instance mode of `soos-push-sender` (ADR "[2026-10-23] Push
//! Sender as a Sandboxed System Service"; spec `AI/architect_spec_push_sender_system_service.md`
//! §4; matrix PSS4–PSS8).
//!
//! Contract:
//! - `parse_run_mode`: no argument is the user mode (unchanged); exactly
//!   `--system-instance <uid>` is the system mode, `<uid>` 1–10 ASCII digits without sign or
//!   leading zero, parsed as `u32`, never `0` nor `u32::MAX`; anything else is `Usage`.
//! - `check_instance_identity`: root (real or effective) and an instance uid equal to the
//!   sender's own (real or effective) uid are refused (`Identity`).
//! - `evaluate_system_sandbox` (pure, in this order): `/run` is a read-only tmpfs
//!   (`RunVisible`); `/home` and `/root` are each a read-only tmpfs with no child mount
//!   (`HomeVisible`); `/var/log` is not listable (`LogsReadable`).
//! - `listener_from_fd` accepts only a listening `AF_UNIX` `SOCK_STREAM` socket.
//! - The binary in system mode exits 78 on refused arguments, a refused identity or outside
//!   its sandbox, with one constant log line, before it serves anything.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Contractual test suite utilizes direct assertions, unwrap, and indexing"
)]

use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use soos_push_protocol::{EXIT_CONFIG, PUSH_SYSTEM_INSTANCE_ARG};
use soos_push_sender::{
    check_instance_identity, evaluate_system_sandbox, listener_from_fd, parse_run_mode,
    probe_system_sandbox, InstanceError, RunMode, SandboxError,
};

fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

/// The host's own mounts (no sandbox).
const HOST_LINES: &str = "\
24 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw
25 24 0:24 / /run rw,nosuid,nodev,relatime shared:12 - tmpfs tmpfs rw,size=3264220k,mode=755
26 24 259:3 / /home rw,relatime shared:30 - ext4 /dev/nvme0n1p3 rw
27 24 0:22 / /proc rw,nosuid,nodev,noexec,relatime shared:13 - proc proc rw
28 25 0:51 / /run/user/1000 rw,nosuid,nodev,relatime shared:500 - tmpfs tmpfs rw,mode=700,uid=1000,gid=1000
29 24 259:2 /var/log /var/log rw,relatime shared:31 - ext4 /dev/nvme0n1p2 rw
";

/// The mount table of `packaging/soos-push-sender@.service` (ubuntu:24.04, systemd 255): the
/// host lines, then the sandbox (`/run` ro tmpfs with the two resolver files bound back,
/// `ProtectHome=tmpfs` over `/home`, `/root` and `/run/user`, inaccessible logs).
fn unit_table() -> String {
    let mut table = HOST_LINES.to_string();
    table.push_str(
        "\
500 24 259:2 / / ro,relatime master:1 - ext4 /dev/nvme0n1p2 rw
501 500 0:70 / /dev ro,nosuid,noexec,relatime - tmpfs tmpfs ro,mode=755
502 500 0:22 / /proc rw,nosuid,nodev,noexec,relatime master:13 - proc proc rw,hidepid=invisible,subset=pid
503 500 0:71 / /run ro,nosuid,nodev,relatime - tmpfs tmpfs ro,mode=755
504 503 0:24 /systemd/resolve/stub-resolv.conf /run/systemd/resolve/stub-resolv.conf ro,nosuid,nodev,relatime master:12 - tmpfs tmpfs rw,mode=755
505 503 0:72 / /run/user ro,nosuid,nodev,relatime - tmpfs tmpfs ro,mode=755
506 500 0:73 / /home ro,nosuid,nodev,relatime - tmpfs tmpfs ro,mode=755
507 500 0:74 / /root ro,nosuid,nodev,relatime - tmpfs tmpfs ro,mode=755
508 500 0:75 / /tmp rw,nosuid,nodev - tmpfs tmpfs rw
509 500 0:76 / /var/tmp rw,nosuid,nodev - tmpfs tmpfs rw
510 500 0:77 / /var/log ro,nosuid,nodev,noexec,relatime - tmpfs tmpfs ro,mode=000
511 500 0:77 / /var/lib/systemd/coredump ro,nosuid,nodev,noexec,relatime - tmpfs tmpfs ro,mode=000
",
    );
    table
}

/// [`unit_table`] without the sandbox line (id ≥ 500) whose mount point is `mount_point`.
fn without(table: &str, mount_point: &str) -> String {
    table
        .lines()
        .filter(|line| {
            let fields: Vec<&str> = line.split(' ').collect();
            let id: u32 = fields[0].parse().unwrap_or(0);
            !(id >= 500 && fields.get(4) == Some(&mount_point))
        })
        .map(|l| format!("{l}\n"))
        .collect()
}

// ---------------------------------------------------------------------------
// PSS4 — arguments
// ---------------------------------------------------------------------------

#[test]
fn test_pss4_parse_run_mode() {
    assert_eq!(PUSH_SYSTEM_INSTANCE_ARG, "--system-instance");
    assert_eq!(parse_run_mode(args(&[])), Ok(RunMode::User));
    for (uid, expected) in [
        ("1000", 1000),
        ("1", 1),
        ("65534", 65_534),
        ("4294967294", 4_294_967_294),
    ] {
        assert_eq!(
            parse_run_mode(args(&["--system-instance", uid])),
            Ok(RunMode::System {
                allowed_uid: expected
            }),
            "{uid}"
        );
    }
    for bad in [
        vec!["--system-instance"],
        vec!["--system-instance", ""],
        vec!["--system-instance", "0"],
        vec!["--system-instance", "00"],
        vec!["--system-instance", "01000"],
        vec!["--system-instance", "+1000"],
        vec!["--system-instance", "-1"],
        vec!["--system-instance", " 1000"],
        vec!["--system-instance", "1000 "],
        vec!["--system-instance", "1e3"],
        vec!["--system-instance", "root"],
        vec!["--system-instance", "4294967295"],
        vec!["--system-instance", "4294967296"],
        vec!["--system-instance", "12345678901"],
        vec!["--system-instance", "１０００"],
        vec!["--system-instance", "1000", "1001"],
        vec!["--system-instance", "1000", "--system-instance", "1000"],
        vec!["--system-instance=1000"],
        vec!["1000"],
        vec!["--help"],
        vec!["--user"],
    ] {
        assert_eq!(
            parse_run_mode(args(&bad)),
            Err(InstanceError::Usage),
            "{bad:?}"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let non_utf8 = vec![
            OsString::from("--system-instance"),
            OsString::from_vec(vec![0x31, 0xff, 0x30]),
        ];
        assert_eq!(parse_run_mode(non_utf8), Err(InstanceError::Usage));
    }
}

// ---------------------------------------------------------------------------
// PSS5 — identity
// ---------------------------------------------------------------------------

#[test]
fn test_pss5_instance_identity() {
    assert_eq!(check_instance_identity(61_234, 61_234, 1000), Ok(()));
    assert_eq!(
        check_instance_identity(0, 0, 1000),
        Err(InstanceError::Identity)
    );
    assert_eq!(
        check_instance_identity(61_234, 0, 1000),
        Err(InstanceError::Identity)
    );
    assert_eq!(
        check_instance_identity(0, 61_234, 1000),
        Err(InstanceError::Identity)
    );
    // The system instance never shares the desktop user's uid (DynamicUser=yes).
    assert_eq!(
        check_instance_identity(1000, 1000, 1000),
        Err(InstanceError::Identity)
    );
    assert_eq!(
        check_instance_identity(61_234, 1000, 1000),
        Err(InstanceError::Identity)
    );
    assert_eq!(
        check_instance_identity(1000, 61_234, 1000),
        Err(InstanceError::Identity)
    );
    assert_eq!(
        InstanceError::Usage.to_string(),
        "push sender arguments refused"
    );
    assert_eq!(
        InstanceError::Identity.to_string(),
        "push sender instance identity refused"
    );
}

// ---------------------------------------------------------------------------
// PSS6 — system sandbox
// ---------------------------------------------------------------------------

#[test]
fn test_pss6_system_sandbox_accepts_the_unit_table() {
    assert_eq!(evaluate_system_sandbox(&unit_table(), false), Ok(()));
    // Line order does not matter.
    let reversed: String = unit_table()
        .lines()
        .rev()
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(evaluate_system_sandbox(&reversed, false), Ok(()));
    // Malformed lines are skipped.
    let noisy = format!("garbage\n\n1 2\n{}", unit_table());
    assert_eq!(evaluate_system_sandbox(&noisy, false), Ok(()));
}

#[test]
fn test_pss6_system_sandbox_refuses_the_host_table() {
    assert_eq!(
        evaluate_system_sandbox(HOST_LINES, false),
        Err(SandboxError::RunVisible)
    );
    assert_eq!(
        evaluate_system_sandbox("", false),
        Err(SandboxError::RunVisible)
    );
}

#[test]
fn test_pss6_system_sandbox_requires_ro_tmpfs_run() {
    let table = without(&unit_table(), "/run");
    assert_eq!(
        evaluate_system_sandbox(&table, false),
        Err(SandboxError::RunVisible)
    );
    // A read-write tmpfs at /run is not the sandbox.
    let rw = unit_table().replace(
        "503 500 0:71 / /run ro,nosuid",
        "503 500 0:71 / /run rw,nosuid",
    );
    assert_eq!(
        evaluate_system_sandbox(&rw, false),
        Err(SandboxError::RunVisible)
    );
    // A read-only mount at /run that is not a tmpfs is not the sandbox.
    let not_tmpfs = unit_table().replace(
        "/run ro,nosuid,nodev,relatime - tmpfs tmpfs ro,mode=755",
        "/run ro,nosuid,nodev,relatime - ext4 /dev/sda1 ro",
    );
    assert_eq!(
        evaluate_system_sandbox(&not_tmpfs, false),
        Err(SandboxError::RunVisible)
    );
    // "ro" must be an exact per-mount option, not a substring ("rootcontext=").
    let fake_ro = unit_table().replace(
        "503 500 0:71 / /run ro,nosuid",
        "503 500 0:71 / /run rw,rootcontext=x,nosuid",
    );
    assert_eq!(
        evaluate_system_sandbox(&fake_ro, false),
        Err(SandboxError::RunVisible)
    );
}

#[test]
fn test_pss6_system_sandbox_requires_hidden_home_and_root() {
    for hidden in ["/home", "/root"] {
        let table = without(&unit_table(), hidden);
        assert_eq!(
            evaluate_system_sandbox(&table, false),
            Err(SandboxError::HomeVisible),
            "{hidden} missing"
        );
        let rw = unit_table().replace(
            &format!("{hidden} ro,nosuid,nodev,relatime - tmpfs"),
            &format!("{hidden} rw,nosuid,nodev,relatime - tmpfs"),
        );
        assert_eq!(
            evaluate_system_sandbox(&rw, false),
            Err(SandboxError::HomeVisible),
            "{hidden} read-write"
        );
    }
    // Something mounted back inside the hidden /home (ProtectHome=read-only plus a bind,
    // BindPaths=/home/u, ...) is refused, whatever its type.
    for (parent, mount_point) in [("506", "/home/u"), ("507", "/root/.ssh"), ("506", "/home")] {
        let table = format!(
            "{}600 {parent} 259:3 /u {mount_point} rw,relatime - ext4 /dev/nvme0n1p3 rw\n",
            unit_table()
        );
        assert_eq!(
            evaluate_system_sandbox(&table, false),
            Err(SandboxError::HomeVisible),
            "{mount_point} re-exposed"
        );
    }
    // A child of another mount (the host's own /home) does not matter.
    let unrelated = format!(
        "{}601 26 259:3 /u /home/u rw,relatime - ext4 /dev/nvme0n1p3 rw\n",
        unit_table()
    );
    assert_eq!(evaluate_system_sandbox(&unrelated, false), Ok(()));
}

#[test]
fn test_pss6_system_sandbox_refuses_listable_logs() {
    assert_eq!(
        evaluate_system_sandbox(&unit_table(), true),
        Err(SandboxError::LogsReadable)
    );
    // Order of the errors: /run first, then the homes, then the logs.
    assert_eq!(
        evaluate_system_sandbox(&without(&unit_table(), "/run"), true),
        Err(SandboxError::RunVisible)
    );
    assert_eq!(
        evaluate_system_sandbox(&without(&unit_table(), "/home"), true),
        Err(SandboxError::HomeVisible)
    );
}

#[test]
fn test_pss6_probe_system_sandbox_refuses_an_unsandboxed_process() {
    assert!(probe_system_sandbox().is_err());
}

// ---------------------------------------------------------------------------
// PSS7 — inherited listening socket
// ---------------------------------------------------------------------------

#[test]
fn test_pss7_listener_from_fd() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("push.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let fd = OwnedFd::from(listener);
    let listener = listener_from_fd(fd).expect("a listening AF_UNIX stream socket is accepted");
    let mut client = UnixStream::connect(&path).unwrap();
    let (mut served, _) = listener.accept().unwrap();
    client.write_all(b"ping").unwrap();
    let mut buf = [0u8; 4];
    served.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"ping");

    // A bound but not listening socket is refused.
    let (a, _b) = UnixStream::pair().unwrap();
    assert!(
        listener_from_fd(OwnedFd::from(a)).is_err(),
        "connected stream"
    );
    // A listening socket of another family is refused.
    let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    assert!(
        listener_from_fd(OwnedFd::from(tcp)).is_err(),
        "AF_INET listener"
    );
    // A datagram socket is refused.
    let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    assert!(
        listener_from_fd(OwnedFd::from(udp)).is_err(),
        "datagram socket"
    );
    let datagram = std::os::unix::net::UnixDatagram::unbound().unwrap();
    assert!(
        listener_from_fd(OwnedFd::from(datagram)).is_err(),
        "AF_UNIX datagram"
    );
    // A regular file and /dev/null are refused.
    let file = std::fs::File::create(dir.path().join("plain")).unwrap();
    assert!(
        listener_from_fd(OwnedFd::from(file)).is_err(),
        "regular file"
    );
    let null = std::fs::File::open("/dev/null").unwrap();
    assert!(listener_from_fd(OwnedFd::from(null)).is_err(), "/dev/null");
}

// ---------------------------------------------------------------------------
// PSS8 — the binary in system mode
// ---------------------------------------------------------------------------

/// Runs the built sender with `args` and the given stdin; returns (exit code, stderr).
fn run_sender(argv: &[&str], stdin: Stdio) -> (Option<i32>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_soos-push-sender"))
        .args(argv)
        .env_clear()
        .stdin(stdin)
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
    let status = status.expect("the sender must exit on its own");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    (status.code(), stderr)
}

#[test]
fn test_pss8_binary_refuses_bad_arguments() {
    if nix::unistd::getuid().is_root() {
        return;
    }
    for argv in [
        vec!["--system-instance"],
        vec!["--system-instance", "0"],
        vec!["--system-instance", "abc"],
        vec!["--bogus"],
    ] {
        let (code, stderr) = run_sender(&argv, Stdio::null());
        assert_eq!(code, Some(EXIT_CONFIG), "{argv:?}");
        assert!(
            stderr.contains("push sender arguments refused"),
            "{argv:?}: {stderr}"
        );
    }
}

#[test]
fn test_pss8_binary_refuses_its_own_uid_as_instance() {
    let uid = nix::unistd::getuid();
    if uid.is_root() {
        return;
    }
    let own = uid.as_raw().to_string();
    let (code, stderr) = run_sender(&["--system-instance", &own], Stdio::null());
    assert_eq!(code, Some(EXIT_CONFIG));
    assert!(
        stderr.contains("push sender instance identity refused"),
        "{stderr}"
    );
}

#[test]
fn test_pss8_binary_exits_78_outside_its_system_sandbox() {
    let uid = nix::unistd::getuid();
    if uid.is_root() {
        return;
    }
    // Another uid than ours: the identity passes, the sandbox check refuses. A listening
    // socket on stdin is never served (no connection is accepted).
    let other = if uid.as_raw() == 1 { "2" } else { "1" };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("push.sock");
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let probe = listener.try_clone().unwrap();
    let (code, stderr) = run_sender(
        &["--system-instance", other],
        Stdio::from(OwnedFd::from(listener)),
    );
    assert_eq!(code, Some(EXIT_CONFIG));
    assert!(
        stderr.contains("push sender sandbox not detected; refusing to start"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("push sender ready"),
        "never ready outside its sandbox: {stderr}"
    );
    assert!(probe.accept().is_err(), "nothing was accepted");
}
