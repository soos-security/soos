//! Static and installer contracts of the push sender as a sandboxed **system** service (owner
//! hardware report of 2026-10-23: `226/NAMESPACE` on Ubuntu 24.04 with
//! `kernel.apparmor_restrict_unprivileged_userns=1`; ADR "[2026-10-23] Push Sender as a
//! Sandboxed System Service"; spec `AI/architect_spec_push_sender_system_service.md`; rows
//! PSS1, PSS2, PSS9, PSS11, PSS13):
//!
//! - the two template units are exactly the specified line sets and name the protocol constants;
//! - `main.rs` dispatches the system instance after the root check and before the user-mode
//!   sandbox check, and the system path checks identity, sandbox and socket before serving;
//! - `install.sh` stages the binary and both units (never enables them), `uninstall.sh` removes
//!   them, the RPM `%files` lists them;
//! - documentation, ADR and walkthrough.
//!
//! The legacy user unit `packaging/soos-push-sender.service` and its contracts (RMC-S35, RSH9,
//! RSH11) are unchanged.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "Architectural invariant test runner utilizes direct assertions, panics, and indexing"
)]

use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::remote_companion_contract::{
    exists, production_part, read, strip_comments, workspace_root,
};

const SOCKET_UNIT: &str = "packaging/soos-push-sender@.socket";
const SERVICE_UNIT: &str = "packaging/soos-push-sender@.service";
const LEGACY_UNIT: &str = "packaging/soos-push-sender.service";
const PROTOCOL_LIB: &str = "crates/push-protocol/src/lib.rs";
const SENDER_MAIN: &str = "crates/push-sender/src/main.rs";
const SENDER_LIB: &str = "crates/push-sender/src/lib.rs";
const ADR_TITLE: &str = "[2026-10-23] Push Sender as a Sandboxed System Service";
const WALKTHROUGH: &str = "AI/walkthroughs/255_push_sender_system_service.md";

/// The exact non-comment line set of `packaging/soos-push-sender@.socket` (spec §2.1).
const SOCKET_LINES: [&str; 12] = [
    "[Unit]",
    "Description=soos push sender socket for uid %i",
    "Documentation=https://github.com/Mysticaly622/soos/blob/main/Docs/REMOTE_COMPANION.md",
    "[Socket]",
    "ListenStream=/run/soos-push/%i/push.sock",
    "SocketUser=%i",
    "SocketMode=0600",
    "DirectoryMode=0755",
    "RemoveOnStop=yes",
    "Accept=no",
    "[Install]",
    "WantedBy=sockets.target",
];

/// The exact non-comment line set of `packaging/soos-push-sender@.service` (spec §2.2).
const SERVICE_LINES: [&str; 47] = [
    "[Unit]",
    "Description=soos push sender for uid %i (outbound Web Push for the remote companion)",
    "Documentation=https://github.com/Mysticaly622/soos/blob/main/Docs/REMOTE_COMPANION.md",
    "Requires=soos-push-sender@%i.socket",
    "After=soos-push-sender@%i.socket",
    "[Service]",
    "Type=exec",
    "ExecStart=/usr/libexec/soos/soos-push-sender --system-instance %i",
    "StandardInput=socket",
    "StandardOutput=journal",
    "StandardError=journal",
    "DynamicUser=yes",
    "Restart=on-failure",
    "RestartSec=5",
    "RestartPreventExitStatus=78",
    "NoNewPrivileges=yes",
    "RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6",
    "LockPersonality=yes",
    "MemoryDenyWriteExecute=yes",
    "RestrictRealtime=yes",
    "RestrictSUIDSGID=yes",
    "SystemCallArchitectures=native",
    "ProtectSystem=strict",
    "ProtectHome=yes",
    "TemporaryFileSystem=/run:ro",
    "InaccessiblePaths=-/var/log -/var/lib/systemd/coredump",
    "BindReadOnlyPaths=-/run/systemd/resolve/stub-resolv.conf -/run/systemd/resolve/resolv.conf",
    "PrivateTmp=yes",
    "PrivateDevices=yes",
    "PrivateIPC=yes",
    "ProtectKernelTunables=yes",
    "ProtectKernelModules=yes",
    "ProtectKernelLogs=yes",
    "ProtectControlGroups=yes",
    "ProtectClock=yes",
    "ProtectHostname=yes",
    "ProtectProc=invisible",
    "ProcSubset=pid",
    "RestrictNamespaces=yes",
    "RemoveIPC=yes",
    "KeyringMode=private",
    "CapabilityBoundingSet=",
    "AmbientCapabilities=",
    "SystemCallFilter=@system-service",
    "SystemCallFilter=~@privileged",
    "SystemCallErrorNumber=EPERM",
    "UMask=0077",
];

/// The non-empty, non-comment, trimmed lines of a unit file.
fn unit_lines(unit: &str) -> Vec<&str> {
    unit.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with(';'))
        .collect()
}

fn pos(hay: &str, needle: &str, what: &str) -> usize {
    hay.find(needle)
        .unwrap_or_else(|| panic!("{what}: `{needle}` not found"))
}

/// Byte offset of the `}` closing the balanced block opened at `open` (a `{`).
fn block_end(code: &str, open: usize) -> usize {
    let mut depth = 0usize;
    for (i, c) in code[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return open + i;
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced block at byte {open}");
}

/// The body of the first item whose text contains `signature`.
fn body_of(code: &str, signature: &str) -> String {
    let at = pos(code, signature, "body_of");
    let open = at + code[at..].find('{').expect("a body");
    let end = block_end(code, open);
    code[open + 1..end].to_string()
}

fn code_of(rel: &str) -> String {
    assert!(exists(rel), "{rel} must exist");
    strip_comments(production_part(&read(rel)))
}

// ---------------------------------------------------------------------------------------
// PSS1 — the two units
// ---------------------------------------------------------------------------------------

/// PSS1: both template units are exactly the specified line sets, in order; the service has
/// no `[Install]` section (socket-activated only) and none of the widening directives.
#[test]
fn test_pss1_system_units_are_exact() {
    let socket = read(SOCKET_UNIT);
    assert_eq!(unit_lines(&socket), SOCKET_LINES.to_vec(), "{SOCKET_UNIT}");

    let service = read(SERVICE_UNIT);
    let lines = unit_lines(&service);
    assert_eq!(lines, SERVICE_LINES.to_vec(), "{SERVICE_UNIT}");
    for forbidden in [
        "[Install]",
        "User=",
        "Group=",
        "SupplementaryGroups=",
        "PrivateUsers=",
        "PrivateNetwork=",
        "ProtectHome=read-only",
        "ProtectHome=no",
        "BindPaths=",
        "ReadWritePaths=",
        "ReadOnlyPaths=",
        "Environment=",
        "EnvironmentFile=",
        "PassEnvironment=",
        "RuntimeDirectory=",
        "StateDirectory=",
        "CacheDirectory=",
        "LogsDirectory=",
        "AmbientCapabilities=CAP",
        "CapabilityBoundingSet=CAP",
    ] {
        assert!(
            !lines.iter().any(|l| l.starts_with(forbidden)),
            "{SERVICE_UNIT} must not contain {forbidden}"
        );
    }
    // The comment header explains why this is a system unit.
    let header: String = service
        .lines()
        .filter(|l| l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    for needle in ["226/NAMESPACE", "DynamicUser", "SO_PEERCRED", "/var/log"] {
        assert!(
            header.contains(needle),
            "{SERVICE_UNIT} header comment must mention {needle}"
        );
    }
    // The legacy user unit is kept as the root-free alternative (its exact contract is RMC-S35).
    assert!(exists(LEGACY_UNIT), "{LEGACY_UNIT} is kept");
}

// ---------------------------------------------------------------------------------------
// PSS2 — single source of the socket path and argument
// ---------------------------------------------------------------------------------------

/// PSS2: the unit literals equal the protocol constants and the install path of install.sh.
#[test]
fn test_pss2_units_match_protocol_constants() {
    let protocol = code_of(PROTOCOL_LIB);
    for constant in [
        "pub const PUSH_SYSTEM_SOCKET_ROOT: &str = \"/run/soos-push\";",
        "pub const PUSH_SYSTEM_INSTANCE_ARG: &str = \"--system-instance\";",
        "pub const PUSH_SOCKET_FILE_NAME: &str = \"push.sock\";",
    ] {
        assert!(
            protocol.contains(constant),
            "{PROTOCOL_LIB} must define {constant}"
        );
    }
    let socket = read(SOCKET_UNIT);
    assert!(unit_lines(&socket).contains(&"ListenStream=/run/soos-push/%i/push.sock"));
    let service = read(SERVICE_UNIT);
    assert!(unit_lines(&service)
        .contains(&"ExecStart=/usr/libexec/soos/soos-push-sender --system-instance %i"));
    let install = read("scripts/install.sh");
    assert!(
        install.contains("${TARGET_LIBEXEC_DIR}/soos-push-sender"),
        "install.sh installs the sender where the unit runs it (<prefix>/libexec/soos)"
    );
}

// ---------------------------------------------------------------------------------------
// PSS9 — main dispatch
// ---------------------------------------------------------------------------------------

/// PSS9: `parse_run_mode` runs after `check_not_root` and before the user-mode
/// `probe_sandbox()`; the system path checks identity, then its sandbox, then the inherited
/// socket, before it serves; it takes the socket from stdin (no `LISTEN_FDS`, no raw fd);
/// every refusal exits 78 with one constant line.
#[test]
fn test_pss9_main_dispatch_order() {
    let main = code_of(SENDER_MAIN);
    let body = body_of(&main, "fn main(");
    let root = pos(&body, "check_not_root(", "main");
    let mode = pos(&body, "parse_run_mode(", "main");
    let probe = pos(&body, "probe_sandbox()", "main");
    assert!(
        root < mode && mode < probe,
        "parse_run_mode runs after check_not_root and before probe_sandbox()"
    );
    assert!(body.contains("tracing::error!(\"push sender arguments refused\")"));

    let system = body_of(&main, "fn run_system_instance(");
    let identity = pos(&system, "check_instance_identity(", "run_system_instance");
    let sandbox = pos(&system, "probe_system_sandbox()", "run_system_instance");
    let fd = pos(&system, "listener_from_fd(", "run_system_instance");
    let serve = pos(&system, "serve_connection(", "run_system_instance");
    assert!(
        identity < sandbox && sandbox < fd && fd < serve,
        "identity, sandbox, socket, then serve"
    );
    for line in [
        "tracing::error!(\"push sender instance identity refused\")",
        "tracing::error!(\"push sender sandbox not detected; refusing to start\")",
        "tracing::error!(\"push sender socket setup failed\")",
    ] {
        assert!(system.contains(line), "run_system_instance logs {line}");
    }
    assert!(
        system.matches("exit_code(EXIT_CONFIG)").count() >= 3,
        "every refusal of the system path exits 78"
    );
    assert!(system.contains("stdin()"), "the socket comes from stdin");
    for forbidden in [
        "LISTEN_FDS",
        "LISTEN_PID",
        "from_raw_fd",
        "FromRawFd",
        "bind_socket(",
        "XDG_RUNTIME_DIR",
    ] {
        assert!(
            !system.contains(forbidden),
            "run_system_instance must not use {forbidden}"
        );
    }
    let lib = code_of(SENDER_LIB);
    for signature in [
        "pub fn parse_run_mode",
        "pub fn check_instance_identity(",
        "pub fn evaluate_system_sandbox(",
        "pub fn probe_system_sandbox(",
        "pub fn listener_from_fd(",
        "pub enum RunMode",
        "pub enum InstanceError",
    ] {
        assert!(lib.contains(signature), "{SENDER_LIB} defines {signature}");
    }
    assert!(!lib.contains("from_raw_fd"), "no raw fd conversion");
}

// ---------------------------------------------------------------------------------------
// PSS11 — install.sh, uninstall.sh, packages
// ---------------------------------------------------------------------------------------

const REQUIRED: [&str; 5] = [
    "soos-daemon",
    "soos-admin",
    "soos-enroll",
    "soos-gui",
    "libpam_soos.so",
];

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soos_pss_{tag}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn artifacts(dir: &Path, with_sender: bool) {
    fs::create_dir_all(dir).unwrap();
    let mut names: Vec<&str> = REQUIRED.to_vec();
    if with_sender {
        names.push("soos-push-sender");
    }
    for name in names {
        let path = dir.join(name);
        fs::write(&path, format!("fixture:{name}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn run_install(stage: &Path, art: &Path, extra: &[&str]) -> Output {
    Command::new("bash")
        .arg(workspace_root().join("scripts/install.sh"))
        .args([
            OsStr::new("--destdir"),
            stage.as_os_str(),
            OsStr::new("--artifact-dir"),
            art.as_os_str(),
            OsStr::new("--pam-dir"),
            OsStr::new("/usr/lib/security"),
            OsStr::new("--skip-systemd"),
            OsStr::new("--skip-models"),
        ])
        .args(extra)
        .output()
        .expect("run install.sh")
}

fn run_uninstall(stage: &Path) -> Output {
    Command::new("bash")
        .arg(workspace_root().join("scripts/uninstall.sh"))
        .args([
            OsStr::new("--destdir"),
            stage.as_os_str(),
            OsStr::new("--keep-data"),
            OsStr::new("--skip-systemd"),
        ])
        .output()
        .expect("run uninstall.sh")
}

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

/// PSS11: with the sender built, install.sh stages the binary 0755 in `<prefix>/libexec/soos`
/// and both units 0644 byte-equal to `packaging/`; without it, neither the binary nor the units;
/// uninstall.sh removes all three.
#[test]
fn test_pss11_install_stages_the_system_sender_and_uninstall_removes_it() {
    let dir = scratch("install");
    let (stage, art) = (dir.join("stage"), dir.join("art"));
    artifacts(&art, true);
    let out = run_install(&stage, &art, &[]);
    assert!(
        out.status.success(),
        "install.sh failed:\n{}",
        combined(&out)
    );
    let binary = stage.join("usr/libexec/soos/soos-push-sender");
    assert_eq!(
        fs::read_to_string(&binary).unwrap(),
        "fixture:soos-push-sender\n"
    );
    assert_eq!(mode_of(&binary), 0o755);
    for unit in ["soos-push-sender@.socket", "soos-push-sender@.service"] {
        let installed = stage.join("etc/systemd/system").join(unit);
        assert_eq!(
            fs::read(&installed).unwrap(),
            fs::read(workspace_root().join("packaging").join(unit)).unwrap(),
            "{unit} byte-equal to packaging/"
        );
        assert_eq!(mode_of(&installed), 0o644, "{unit}");
    }
    assert!(
        !stage
            .join("etc/systemd/system/sockets.target.wants")
            .exists(),
        "install.sh never enables an instance"
    );

    let out = run_uninstall(&stage);
    assert!(
        out.status.success(),
        "uninstall.sh failed:\n{}",
        combined(&out)
    );
    assert!(!binary.exists(), "the sender binary is removed");
    for unit in ["soos-push-sender@.socket", "soos-push-sender@.service"] {
        assert!(
            !stage.join("etc/systemd/system").join(unit).exists(),
            "{unit} is removed"
        );
    }

    // Without the sender artifact: neither the binary nor its units are installed.
    let dir2 = scratch("install_nosender");
    let (stage2, art2) = (dir2.join("stage"), dir2.join("art"));
    artifacts(&art2, false);
    let out = run_install(&stage2, &art2, &[]);
    assert!(
        out.status.success(),
        "install.sh failed:\n{}",
        combined(&out)
    );
    assert!(!stage2.join("usr/libexec/soos/soos-push-sender").exists());
    assert!(!stage2
        .join("etc/systemd/system/soos-push-sender@.socket")
        .exists());
    assert!(!stage2
        .join("etc/systemd/system/soos-push-sender@.service")
        .exists());
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&dir2);
}

/// PSS11: the scripts never enable an instance on their own; uninstall.sh disables every
/// enabled instance (live install) before removing the units; the RPM `%files` lists the
/// three files; the dry run of uninstall.sh exits before any removal.
#[test]
fn test_pss11_scripts_and_packages_statically() {
    let install = strip_shell_comments(&read("scripts/install.sh"));
    for line in install.lines() {
        assert!(
            !(line.contains("systemctl") && line.contains("soos-push-sender")),
            "install.sh never runs systemctl on the push sender (it installs the templates only): {line}"
        );
        assert!(
            !line.contains("soos-push-sender@$") && !line.contains("soos-push-sender@${"),
            "install.sh never names an instance: {line}"
        );
    }
    let uninstall = strip_shell_comments(&read("scripts/uninstall.sh"));
    let disable = pos(
        &uninstall,
        "sockets.target.wants/soos-push-sender@",
        "uninstall.sh",
    );
    let remove = pos(
        &uninstall,
        "rm -f \"${TARGET_SYSTEMD_DIR}/soos-push-sender@.socket\"",
        "uninstall.sh",
    );
    assert!(
        disable < remove,
        "instances are disabled before the units are removed"
    );
    assert!(uninstall.contains("rm -f \"${TARGET_LIBEXEC_DIR}/soos-push-sender\""));
    let libexec_rm = pos(
        &uninstall,
        "rm -f \"${TARGET_LIBEXEC_DIR}/soos-push-sender\"",
        "uninstall.sh",
    );
    let rmdir = pos(
        &uninstall,
        "rmdir \"${TARGET_LIBEXEC_DIR}\"",
        "uninstall.sh",
    );
    assert!(
        libexec_rm < rmdir,
        "the binary goes before the libexec directory"
    );
    let dry = pos(&uninstall, "Dry run complete", "uninstall.sh");
    assert!(dry < disable, "the dry run exits before any change");

    let spec = read("packaging/rpm/soos.spec");
    let files = &spec[pos(&spec, "%files", "soos.spec")..];
    for needle in [
        "/usr/libexec/soos/soos-push-sender",
        "%{_unitdir}/soos-push-sender@.socket",
        "%{_unitdir}/soos-push-sender@.service",
    ] {
        assert!(files.contains(needle), "soos.spec %files: {needle}");
    }
}

/// The script without its `#` comment lines.
fn strip_shell_comments(script: &str) -> String {
    script
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text of `body` from `start` up to (not including) the first later line that starts with
/// one of `ends` (or the end).
fn section_from<'a>(body: &'a str, start: &str, ends: &[&str]) -> &'a str {
    let at = pos(body, start, start);
    let rest = &body[at + start.len()..];
    let end = rest
        .lines()
        .scan(0usize, |offset, line| {
            let here = *offset;
            *offset += line.len() + 1;
            Some((here, line))
        })
        .skip(1)
        .find(|(_, line)| ends.iter().any(|e| line.starts_with(e)))
        .map_or(rest.len(), |(here, _)| here);
    &rest[..end]
}

/// PSS18 (candid review of 2026-10-23): removing a package disables and stops every enabled
/// instance before the template units go: RPM `%preun` on erase (`$1` is 0), the Arch
/// `pre_remove()` of `soos.install` (referenced by `install=`), the Debian `prerm` on `remove`;
/// only names of the form `soos-push-sender@<digits>.socket` reach `systemctl`.
#[test]
fn test_pss18_packages_disable_instances_on_removal() {
    let spec = read("packaging/rpm/soos.spec");
    let preun = section_from(&spec, "%preun", &["%post", "%files", "%changelog"]);
    let arch = read("packaging/arch/soos.install");
    let pre_remove = section_from(&arch, "pre_remove() {", &["}"]);
    assert!(
        read("packaging/arch/PKGBUILD")
            .lines()
            .any(|l| l.trim() == "install=soos.install"),
        "PKGBUILD references soos.install"
    );
    let prerm = read("packaging/debian/prerm");
    let remove = section_from(&prerm, "remove|deconfigure)", &["    upgrade|"]);
    for (name, body) in [
        ("soos.spec %preun", preun),
        ("soos.install pre_remove", pre_remove),
        ("debian prerm remove", remove),
    ] {
        for needle in [
            "/etc/systemd/system/sockets.target.wants/soos-push-sender@",
            "soos-push-sender@[0-9]",
            "systemctl disable --now",
            "systemctl stop",
        ] {
            assert!(
                body.contains(needle),
                "{name} must contain {needle}:\n{body}"
            );
        }
    }
    let erase = pos(preun, "\"$1\" -eq 0", "soos.spec %preun");
    let disable = pos(preun, "soos-push-sender@", "soos.spec %preun");
    assert!(
        erase < disable,
        "the RPM instances are disabled on erase only, never on upgrade:\n{preun}"
    );
}

// ---------------------------------------------------------------------------------------
// PSS19 — prefix drop-in (lead decision of 2026-10-23, candid review round 1 fix 1)
// ---------------------------------------------------------------------------------------

const DROP_IN_DIR: &str = "etc/systemd/system/soos-push-sender@.service.d";
const DROP_IN_MARKER: &str = "# Written by scripts/install.sh for --prefix ";

fn run_uninstall_prefix(stage: &Path, prefix: &str) -> Output {
    Command::new("bash")
        .arg(workspace_root().join("scripts/uninstall.sh"))
        .args([
            OsStr::new("--destdir"),
            stage.as_os_str(),
            OsStr::new("--prefix"),
            OsStr::new(prefix),
            OsStr::new("--keep-data"),
            OsStr::new("--skip-systemd"),
        ])
        .output()
        .expect("run uninstall.sh")
}

/// PSS19: with another `--prefix`, install.sh stages the sender under `<prefix>/libexec/soos`
/// and a 0644 drop-in `soos-push-sender@.service.d/prefix.conf` that resets `ExecStart=` to
/// that binary (so the shipped unit, which names `/usr/libexec`, runs what was installed);
/// with `/usr` no drop-in is written and a stale one is removed; uninstall.sh removes it and
/// its directory.
#[test]
fn test_pss19_install_writes_the_prefix_drop_in() {
    let dir = scratch("dropin");
    let (stage, art) = (dir.join("stage"), dir.join("art"));
    artifacts(&art, true);
    let out = run_install(&stage, &art, &["--prefix", "/usr/local"]);
    assert!(
        out.status.success(),
        "install.sh failed:\n{}",
        combined(&out)
    );
    assert_eq!(
        fs::read_to_string(stage.join("usr/local/libexec/soos/soos-push-sender")).unwrap(),
        "fixture:soos-push-sender\n"
    );
    let drop_in = stage.join(DROP_IN_DIR).join("prefix.conf");
    assert_eq!(mode_of(&drop_in), 0o644);
    let text = fs::read_to_string(&drop_in).unwrap();
    assert!(
        text.starts_with(&format!("{DROP_IN_MARKER}/usr/local")),
        "{text}"
    );
    assert_eq!(
        unit_lines(&text),
        vec![
            "[Service]",
            "ExecStart=",
            "ExecStart=/usr/local/libexec/soos/soos-push-sender --system-instance %i",
        ],
        "the drop-in only replaces ExecStart"
    );
    assert!(
        !fs::read_dir(stage.join(DROP_IN_DIR))
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with('.')),
        "no temporary file is left in the drop-in directory"
    );

    // A re-run with /usr removes the stale drop-in.
    let out = run_install(&stage, &art, &[]);
    assert!(
        out.status.success(),
        "install.sh failed:\n{}",
        combined(&out)
    );
    assert!(!drop_in.exists(), "a stale prefix drop-in is removed");

    // /usr from the start: no drop-in.
    let dir2 = scratch("dropin_usr");
    let (stage2, art2) = (dir2.join("stage"), dir2.join("art"));
    artifacts(&art2, true);
    let out = run_install(&stage2, &art2, &[]);
    assert!(
        out.status.success(),
        "install.sh failed:\n{}",
        combined(&out)
    );
    assert!(!stage2.join(DROP_IN_DIR).exists());

    // uninstall.sh removes the drop-in and its directory.
    let dir3 = scratch("dropin_uninstall");
    let (stage3, art3) = (dir3.join("stage"), dir3.join("art"));
    artifacts(&art3, true);
    let out = run_install(&stage3, &art3, &["--prefix", "/opt/soos"]);
    assert!(
        out.status.success(),
        "install.sh failed:\n{}",
        combined(&out)
    );
    assert!(stage3.join(DROP_IN_DIR).join("prefix.conf").is_file());
    let out = run_uninstall_prefix(&stage3, "/opt/soos");
    assert!(
        out.status.success(),
        "uninstall.sh failed:\n{}",
        combined(&out)
    );
    assert!(
        !stage3.join(DROP_IN_DIR).exists(),
        "the drop-in directory is removed"
    );
    assert!(!stage3
        .join("opt/soos/libexec/soos/soos-push-sender")
        .exists());
    for d in [dir, dir2, dir3] {
        let _ = fs::remove_dir_all(d);
    }
}

/// PSS19: a prefix that would break the unit syntax or that the sandbox hides is refused before
/// any change, and the drop-in directory is never written through a symbolic link.
#[test]
fn test_pss19_install_refuses_unsafe_prefixes_and_links() {
    for (n, prefix) in [
        "/opt/so os",
        "/opt/so%os",
        "/opt/so\nos",
        "/opt/so\"os",
        "/opt/so\\os",
        "opt/soos",
        "/opt/../usr",
        "/opt/./soos",
        "/opt//soos",
        "/opt/soos/",
        "/home/alice/opt",
        "/root/opt",
        "/run/soos",
        "/tmp/soos",
        "/var/tmp/soos",
        "/var/log/soos",
    ]
    .iter()
    .enumerate()
    {
        let dir = scratch(&format!("dropin_bad_{n}"));
        let (stage, art) = (dir.join("stage"), dir.join("art"));
        artifacts(&art, true);
        let out = run_install(&stage, &art, &["--prefix", prefix]);
        assert!(!out.status.success(), "{prefix:?} must be refused");
        assert!(
            combined(&out).contains("push sender"),
            "{prefix:?}: the refusal names the push sender:\n{}",
            combined(&out)
        );
        assert!(
            !stage.join(DROP_IN_DIR).exists(),
            "{prefix:?}: nothing written"
        );
        let _ = fs::remove_dir_all(dir);
    }

    // A symbolic link in place of the drop-in directory is refused and never followed.
    let dir = scratch("dropin_link");
    let (stage, art) = (dir.join("stage"), dir.join("art"));
    artifacts(&art, true);
    let elsewhere = dir.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    fs::create_dir_all(stage.join("etc/systemd/system")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, stage.join(DROP_IN_DIR)).unwrap();
    let out = run_install(&stage, &art, &["--prefix", "/usr/local"]);
    assert!(
        !out.status.success(),
        "a linked drop-in directory is refused"
    );
    assert_eq!(
        fs::read_dir(&elsewhere).unwrap().count(),
        0,
        "link never followed"
    );
    let _ = fs::remove_dir_all(dir);

    // Statically: the drop-in goes through the journaled, atomic tracked_install.
    let install = strip_shell_comments(&read("scripts/install.sh"));
    assert!(
        install
            .lines()
            .any(|l| l.contains("tracked_install 0644") && l.contains("prefix.conf")),
        "the drop-in is written by tracked_install"
    );
}

/// PSS19: package removal also deletes a prefix drop-in that install.sh wrote (recognised by its
/// marker line), so it cannot override the packaged unit later.
#[test]
fn test_pss19_packages_remove_the_prefix_drop_in() {
    let spec = read("packaging/rpm/soos.spec");
    let preun = section_from(&spec, "%preun", &["%post", "%files", "%changelog"]);
    let arch = read("packaging/arch/soos.install");
    let pre_remove = section_from(&arch, "pre_remove() {", &["}"]);
    let prerm = read("packaging/debian/prerm");
    let remove = section_from(&prerm, "remove|deconfigure)", &["    upgrade|"]);
    for (name, body) in [
        ("soos.spec %preun", preun),
        ("soos.install pre_remove", pre_remove),
        ("debian prerm remove", remove),
    ] {
        for needle in [
            "/etc/systemd/system/soos-push-sender@.service.d/prefix.conf",
            "Written by scripts/install.sh for --prefix",
        ] {
            assert!(
                body.contains(needle),
                "{name} must contain {needle}:\n{body}"
            );
        }
    }
}

// ---------------------------------------------------------------------------------------
// PSS13 — documentation, ADR, walkthrough
// ---------------------------------------------------------------------------------------

/// The `## <number> ` section of a Markdown document (up to the next `## `).
fn section<'a>(doc: &'a str, number: &str) -> &'a str {
    let heading = format!("\n## {number} ");
    let at = pos(doc, &heading, number);
    let rest = &doc[at + 1..];
    let end = rest[3..].find("\n## ").map_or(rest.len(), |e| e + 3);
    &rest[..end]
}

/// PSS13: the companion guide §2d, the packaging and deployment guides, the architecture row,
/// the ADR and the walkthrough describe the system service and the Ubuntu failure.
#[test]
fn test_pss13_docs_adr_walkthrough() {
    let doc = read("Docs/REMOTE_COMPANION.md");
    let push = section(&doc, "2d.");
    for needle in [
        "soos-push-sender@",
        "DynamicUser",
        "/run/soos-push/",
        "226/NAMESPACE",
        "apparmor_restrict_unprivileged_userns",
        "SO_PEERCRED",
        "systemctl --user disable --now soos-push-sender",
    ] {
        assert!(
            push.contains(needle),
            "REMOTE_COMPANION §2d must mention {needle}"
        );
    }
    let packaging = read("Docs/PACKAGING_AND_PROVISIONING.md");
    for needle in [
        "soos-push-sender@.socket",
        "soos-push-sender@.service",
        "libexec/soos/soos-push-sender",
    ] {
        assert!(
            packaging.contains(needle),
            "PACKAGING_AND_PROVISIONING must mention {needle}"
        );
    }
    let deployment = read("Docs/DISTRIBUTION_DEPLOYMENT.md");
    for needle in ["226/NAMESPACE", "soos-push-sender@"] {
        assert!(
            deployment.contains(needle),
            "DISTRIBUTION_DEPLOYMENT must mention {needle}"
        );
    }
    let architecture = read("AI/ARCHITECTURE.md");
    let row = architecture
        .lines()
        .find(|l| l.starts_with("| Push sender"))
        .expect("Push sender row");
    assert!(
        row.contains("system service") && row.contains("DynamicUser"),
        "the architecture row names the system service: {row}"
    );
    assert!(
        read("AI/DECISIONS.md").contains(ADR_TITLE),
        "AI/DECISIONS.md must contain {ADR_TITLE}"
    );
    assert!(exists(WALKTHROUGH), "{WALKTHROUGH} must exist");
}
