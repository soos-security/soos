# Architect Spec — `soos-push-sender` as a Sandboxed System Service

- **Date**: 2026-10-23
- **Branch**: `fix/push-sender-system-service`, created from `origin/main` at `e7826de`.
  GitHub-only hardware-report fix: no backlog id, not registered in `scripts/sync_issue.py`.
- **Problem (owner hardware report, 2026-10-23)**: on Ubuntu 24.04
  (`kernel.apparmor_restrict_unprivileged_userns=1`, the default since 23.10) the user unit
  `packaging/soos-push-sender.service` exits `226/NAMESPACE`: the unprivileged user manager
  needs an unprivileged user namespace for every mount-namespace directive (`ProtectSystem=`,
  `ProtectHome=`, `TemporaryFileSystem=`, `InaccessiblePaths=`, `PrivateTmp=`,
  `PrivateDevices=`, `PrivateIPC=`, …) and AppArmor denies it. `scripts/setup.sh` fails in the
  companion phase; the owner had to pass `--push off --offline-alerts off`.
- **Owner decision (2026-10-23)**: run `soos-push-sender` as a sandboxed **system** service
  (privileged manager, no user namespace needed), e.g. `DynamicUser=yes`, isolation at least as
  strong as today. Rejected: relaxing the sandbox; disabling push on Ubuntu only.
- **Matrix**: new prefix **`PSS`** (push sender system), rows PSS1–PSS14 (§11).
- **ADR**: amendment "[2026-10-23] Push Sender as a Sandboxed System Service" (§12).
- **Walkthrough**: `AI/walkthroughs/255_push_sender_system_service.md` (253 is the last on
  main; 254 is reserved by another PR).

---

## 1. Scope and blast radius

| Area | Change |
|---|---|
| `packaging/soos-push-sender@.socket` (new) | System template socket, instance = numeric uid |
| `packaging/soos-push-sender@.service` (new) | System template service, `DynamicUser=yes`, socket-activated |
| `packaging/soos-push-sender.service` | **Unchanged** (legacy root-free user unit, still installed by `install_remote.sh`, never enabled by setup) |
| `crates/push-protocol/src/lib.rs` | New constants `PUSH_SYSTEM_SOCKET_ROOT`, `PUSH_SYSTEM_INSTANCE_ARG`; new fn `system_push_socket_path(uid)` |
| `crates/push-sender/src/lib.rs` | Additive: `RunMode`, `parse_run_mode`, `InstanceError`, `check_instance_identity`, `evaluate_system_sandbox`, `probe_system_sandbox`, `listener_from_fd` |
| `crates/push-sender/src/main.rs` | Additive dispatch after `check_not_root`, before the user-mode `probe_sandbox()`; new `run_system_instance` |
| `crates/remote/src/push.rs` | `UnixPushTransport` gains a candidate list (`PushSocketCandidate { path, peer_uid }`), `UnixPushTransport::with_candidates`; `push_socket_candidates` pure fn |
| `crates/remote/src/main.rs` | Builds the candidates (system socket first, legacy runtime socket second) when `push_socket_path` was not set explicitly |
| `crates/remote/src/config.rs` | **Unchanged** (the configured / derived path keeps its current resolution) |
| `crates/gui/src/onboarding.rs` | Wording "user unit" → "system service" |
| `scripts/install.sh` | Optional artifact `soos-push-sender` → `<prefix>/libexec/soos/soos-push-sender` 0755 and both units 0644 into the unit directory (only when the binary is built); never enabled |
| `scripts/uninstall.sh` | Disables every enabled instance, removes both units and the binary |
| `scripts/setup.sh` | System phase: `--push-sender on` enables `soos-push-sender@<uid>.socket`; companion phase: verifies the socket, disables the legacy user unit, no longer enables the user unit |
| `scripts/install_remote.sh` | Printed steps only: names the system socket first, the user unit as the root-free alternative |
| `packaging/rpm/soos.spec` | `%files` gains the binary and both units |
| Docs | `Docs/REMOTE_COMPANION.md` §2d, `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md` (troubleshooting), `AI/ARCHITECTURE.md` §13 row |
| Tests | `crates/push-sender/tests/system_instance_tests.rs`, `crates/push-protocol/tests/system_socket_tests.rs`, `crates/remote/tests/push_system_socket_tests.rs`, `tests/invariants/src/push_sender_system_contract.rs`, new SET tests in `setup_user_phase_contract.rs`, Docker harness `tests/docker/push_sender_system_unit_test.sh` |

No change to the PAM module, the daemon, the IPC wire format, the push wire format
(`soos-push-protocol` frames) or the policy crate.

---

## 2. Units

### 2.1 `packaging/soos-push-sender@.socket`

```ini
[Unit]
Description=soos push sender socket for uid %i
Documentation=https://github.com/Mysticaly622/soos/blob/main/Docs/REMOTE_COMPANION.md

[Socket]
ListenStream=/run/soos-push/%i/push.sock
SocketUser=%i
SocketMode=0600
DirectoryMode=0755
RemoveOnStop=yes
Accept=no

[Install]
WantedBy=sockets.target
```

- The socket is created by PID 1 (root), owned by the desktop user `%i`, mode `0600`: only that
  uid (and root) can `connect()`. Not group- or world-writable. Its directories
  `/run/soos-push` and `/run/soos-push/<uid>` are root-owned `0755` (systemd creates them),
  so no other unprivileged user can create or replace anything in them.
- One instance per desktop uid; instances never share a socket (multi-user hosts).

### 2.2 `packaging/soos-push-sender@.service`

```ini
[Unit]
Description=soos push sender for uid %i (outbound Web Push for the remote companion)
Documentation=https://github.com/Mysticaly622/soos/blob/main/Docs/REMOTE_COMPANION.md
Requires=soos-push-sender@%i.socket
After=soos-push-sender@%i.socket

[Service]
Type=exec
ExecStart=/usr/libexec/soos/soos-push-sender --system-instance %i
StandardInput=socket
StandardOutput=journal
StandardError=journal
DynamicUser=yes
Restart=on-failure
RestartSec=5
RestartPreventExitStatus=78
NoNewPrivileges=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
LockPersonality=yes
MemoryDenyWriteExecute=yes
RestrictRealtime=yes
RestrictSUIDSGID=yes
SystemCallArchitectures=native
ProtectSystem=strict
ProtectHome=tmpfs
TemporaryFileSystem=/run:ro
InaccessiblePaths=-/var/log -/var/lib/systemd/coredump
BindReadOnlyPaths=-/run/systemd/resolve/stub-resolv.conf -/run/systemd/resolve/resolv.conf
PrivateTmp=yes
PrivateDevices=yes
PrivateIPC=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
ProtectProc=invisible
ProcSubset=pid
RestrictNamespaces=yes
RemoveIPC=yes
KeyringMode=private
CapabilityBoundingSet=
AmbientCapabilities=
SystemCallFilter=@system-service
SystemCallFilter=~@privileged
SystemCallErrorNumber=EPERM
UMask=0077
```

No `[Install]` section: the service is only started by its socket.

### 2.3 Isolation comparison with the user unit (same or stronger)

| Property | User unit (today) | System unit (new) |
|---|---|---|
| Runtime uid | the desktop user | a dynamic uid (61184–65519), never 0, never the user → cannot signal, ptrace or read the user's processes or files even outside the mount sandbox |
| Capabilities | none | none (`CapabilityBoundingSet=` empty, `AmbientCapabilities=` empty) |
| Every mount directive of the user unit | yes (but needs an unprivileged user namespace) | yes, applied by PID 1 without any user namespace |
| `/run` | ro tmpfs, own runtime dir bound back | ro tmpfs; **no** directory bound back (the socket arrives as fd 0); only the two systemd-resolved `resolv.conf` files read-only (`-` = optional) |
| Home | `ProtectHome=tmpfs` + the binary bound back | `ProtectHome=tmpfs` (`/home`, `/root`, `/run/user`), nothing bound back (binary in `/usr/libexec`) |
| Added | — | `ProtectControlGroups=yes`, `ProtectProc=invisible`, `ProcSubset=pid`, `RemoveIPC=yes`, `KeyringMode=private`, `SystemCallFilter=~@privileged` (none available to the user manager) |
| Network | `AF_UNIX AF_INET AF_INET6`, push host allowlist + public-address filter in code | unchanged |

`PrivateUsers=yes` is **not** used: inside a private user namespace the client uid is unmapped
and `SO_PEERCRED` would report the overflow uid 65534, defeating the peer check (§4.3).

The two `BindReadOnlyPaths=` files replace the opt-in directory drop-in of §2d for the common
Ubuntu layout (`/etc/resolv.conf` → `/run/systemd/resolve/stub-resolv.conf`): narrower than the
documented drop-in (two read-only files instead of the directory with its varlink sockets),
required for push to work at all on Ubuntu 24.04. The rest of `/run` stays hidden.

---

## 3. Push protocol constants (single source)

```rust
/// Root of the system sender sockets: `<root>/<uid>/push.sock` (packaging/soos-push-sender@.socket).
pub const PUSH_SYSTEM_SOCKET_ROOT: &str = "/run/soos-push";
/// Argument selecting the system instance mode of `soos-push-sender`.
pub const PUSH_SYSTEM_INSTANCE_ARG: &str = "--system-instance";
/// `<PUSH_SYSTEM_SOCKET_ROOT>/<uid>/<PUSH_SOCKET_FILE_NAME>`.
pub fn system_push_socket_path(uid: u32) -> std::path::PathBuf;
```

The unit files repeat `/run/soos-push/%i/push.sock` and `--system-instance %i` literally
(systemd cannot import a constant); an invariant (PSS2) ties them to the constants.

---

## 4. `soos-push-sender` system instance mode

### 4.1 Arguments

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunMode { User, System { allowed_uid: u32 } }

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum InstanceError {
    #[error("push sender arguments refused")] Usage,
    #[error("push sender instance identity refused")] Identity,
}

pub fn parse_run_mode<I: IntoIterator<Item = std::ffi::OsString>>(args: I) -> Result<RunMode, InstanceError>;
pub fn check_instance_identity(uid: u32, euid: u32, allowed_uid: u32) -> Result<(), InstanceError>;
```

- No argument → `User` (the legacy user-unit path, unchanged).
- Exactly `--system-instance <uid>` → `System`. `<uid>`: 1–10 ASCII digits, no sign, no leading
  zero (except none: `0` itself is refused), parsed as `u32`; `0` and `u32::MAX` refused; any
  other argument count, flag or value → `Usage`. Non-UTF-8 → `Usage`.
- `check_instance_identity`: `Identity` when `uid` or `euid` is 0, or when `uid == allowed_uid`
  or `euid == allowed_uid` (the system instance must never share the user's uid: that is the
  `DynamicUser=` guarantee).

### 4.2 Sandbox self-check (additive, S-7 kept)

```rust
pub fn evaluate_system_sandbox(mountinfo: &str, logs_listable: bool) -> Result<(), SandboxError>;
pub fn probe_system_sandbox() -> Result<(), SandboxError>;
```

Rules, in order of their errors (reusing `SandboxError`, no new variant):
1. some mount at exactly `/run` is a read-only tmpfs (`RunVisible`);
2. for each of `/home` and `/root`: some mount at exactly that path is a read-only tmpfs, and no
   direct child mount of such a tmpfs exists (nothing bound back) (`HomeVisible`);
3. `/var/log` is not listable (`LogsReadable`).

`probe_system_sandbox` reads `/proc/self/mountinfo` with the same bound (`MAX_MOUNTINFO_BYTES`)
and checks `/var/log` like `probe_sandbox`; it reads no environment variable and no binary path.
`probe_sandbox`, `evaluate_sandbox`, `home_from_exe` and `SENDER_EXE_HOME_RELATIVE` are unchanged.

### 4.3 Listening socket and peer authentication

```rust
pub fn listener_from_fd(fd: std::os::fd::OwnedFd) -> Result<std::os::unix::net::UnixListener, SocketError>;
```

Accepts only an `AF_UNIX` `SOCK_STREAM` socket in the listening state (`SO_ACCEPTCONN`); any
other descriptor → `SocketError::Io`. `main` obtains the descriptor with
`std::io::stdin().as_fd().try_clone_to_owned()` (`StandardInput=socket`): no `unsafe`, no
`LISTEN_FDS` environment read (the only environment read stays `XDG_RUNTIME_DIR`, RSH12).

Each connection is served by the unchanged `serve_connection(stream, allowed_uid, deliverer)`:
the `SO_PEERCRED` uid must equal the instance uid, otherwise the connection is closed without a
byte read. Two layers: the socket inode (`0600`, owner `%i`) refuses other users at `connect()`,
the peer check refuses anything else that reaches it.

### 4.4 `main` order

```
install_logging();                       // RMC-S42: first
check_not_root(uid, euid)                // unchanged
parse_run_mode(args_os().skip(1))        // Usage → exit 78
  System { allowed_uid } → run_system_instance(uid, euid, allowed_uid)
       check_instance_identity → 78
       probe_system_sandbox()   → 78 "push sender sandbox not detected; refusing to start"
       listener_from_fd(stdin)  → 78 "push sender socket setup failed"
       serve loop (serve_connection(stream, allowed_uid, …))
  User → unchanged: probe_sandbox(), XDG_RUNTIME_DIR, bind_socket(…), serve loop
```

All failures before serving exit 78 (`EXIT_CONFIG`, `RestartPreventExitStatus=78`): a refused
start is a configuration error, never retried in a loop.

---

## 5. `soos-remote` client

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushSocketCandidate { pub path: PathBuf, pub peer_uid: u32 }

/// `configured` is `config.push.socket_path`; `derived_default` is
/// `$XDG_RUNTIME_DIR/soos-push/push.sock` when computable. Equal → the default:
/// [system socket (peer 0), configured (peer owner)]; otherwise [configured (peer owner)].
pub fn push_socket_candidates(configured: &Path, derived_default: Option<&Path>, owner_uid: u32) -> Vec<PushSocketCandidate>;

impl UnixPushTransport {
    pub fn new(socket_path: PathBuf, owner_uid: u32) -> Self;                       // unchanged: one candidate, peer = owner
    pub fn with_candidates(candidates: Vec<PushSocketCandidate>, owner_uid: u32) -> Self;
}
```

Per exchange: the **first candidate whose path exists** (`symlink_metadata` not `NotFound`) is
used, and only it (no fall-through after a present-but-refused socket: fail closed, no
downgrade). For it: a socket (not a symlink), inode owned by `owner_uid`, peer uid equal to the
candidate's `peer_uid` (0 for the system socket, because PID 1 called `listen()`; the owner for
the legacy user unit). Every refusal is `TransportError::Unavailable` as today.

`config.rs` is unchanged: `push_socket_path` absent still resolves to the legacy runtime path
(and still needs `XDG_RUNTIME_DIR`); `main.rs` turns it into the candidate list. An explicit
`push_socket_path` is the only candidate (peer = owner) — documented.

---

## 6. Installation (root phase)

`scripts/install.sh`:
- `OPTIONAL_PUSH_SENDER=soos-push-sender`: when the artifact exists, install it 0755 to
  `${TARGET_LIBEXEC_DIR}/soos-push-sender` and both units 0644 to `${TARGET_SYSTEMD_DIR}`
  (journaled, rolled back like every file); otherwise an info line. Never enables or starts a
  unit (per-user instances are enabled by setup or the owner).
- `--prefix` other than `/usr`: warning that the units run `/usr/libexec/soos/soos-push-sender`.

`scripts/uninstall.sh`: for every `soos-push-sender@*.socket` link under
`${TARGET_SYSTEMD_DIR}/sockets.target.wants` (and the same under `/etc/systemd/system` for the
package unit dir case) `systemctl disable --now` the socket and `stop` the service instance,
then remove both unit files and the binary; `daemon-reload` as today. `--dry-run` writes
nothing (existing early exit).

`packaging/rpm/soos.spec` `%files`: the binary and both units.

---

## 7. `scripts/setup.sh`

### 7.1 User half (read-only decisions)

- `SENDER_SOCKET_UNIT="soos-push-sender@${TARGET_UID}.socket"`, `LEGACY_SENDER_UNIT=soos-push-sender.service`.
- Push sender requested when the companion runs with a managed configuration (not `kept`) and
  `push_notifications = true`, unless **kept off**: push was already on in that file, the
  socket is not enabled (`systemctl is-enabled`, system manager, read-only) **and** the legacy
  user unit is not enabled either (an enabled legacy unit is migrated, not "kept off").
- The system phase receives `--push-sender on` only then.

### 7.2 System half (`phase_push_sender`, after `phase_lock_widget`, before `phase_preview`)

- `--push-sender` absent/off → nothing.
- Read-path rule: the units run `/usr/libexec/soos/soos-push-sender` and listen on
  `/run/soos-push/<uid>/push.sock`, the path `soos-remote` reads. The step is reported done only
  when `${PREFIX}` is `/usr` (the binary path the unit runs) and `systemctl enable --now` of the
  socket succeeds (systemd loaded the unit from a directory it reads); with another prefix:
  `[SKIP] push sender: not applied` plus the manual command.
- Already enabled and active → `[DONE] push sender: already done`.
- Otherwise `systemctl enable --now soos-push-sender@<uid>.socket` (bounded by
  `QUERY_TIMEOUT_S`), journaled for the system rollback (disable --now on rollback when this run
  enabled it). Failure → `fail_and_rollback "push sender"`.
- `--dry-run`: plan line only.

### 7.3 Companion phase

- The sender user unit is never enabled or started any more.
- When push is requested and the system step ran: `systemctl is-active soos-push-sender@<uid>.socket`
  must succeed, else `companion_failed`.
- Migration: when the legacy user unit `soos-push-sender.service` is enabled or active, it is
  disabled and stopped (`systemctl --user disable --now`), journaled (`user_unit_disabled:` →
  the rollback re-enables it). The legacy unit file stays (installed by `install_remote.sh`, the
  root-free alternative); setup never enables it.
- `companion_units_current` checks the system socket instead of the user unit.
- Guide: "Push is off" hint names the system socket; the resolver warning is printed only for a
  `/run/*` target other than the two bound files.

### 7.4 Rollback / dry run / read path

- Dry run: plan lines only, nothing written (existing early exits).
- Every step reporting "done" writes or enables exactly what the reader uses (§7.2).

---

## 8. Latency budget

Not on the authentication path. The client keeps `PUSH_CONNECT_UNIX_TIMEOUT_MS` and
`PUSH_EXCHANGE_TIMEOUT_MS`; the first connection after boot also includes the socket-activated
start of the sender (process start + mount-table probe, milliseconds) inside the exchange bound.

## 9. Error taxonomy

| Condition | Result |
|---|---|
| Bad arguments, uid 0 / `u32::MAX`, sender uid = instance uid, root | exit 78, one constant log line, not restarted |
| System sandbox missing | exit 78 "push sender sandbox not detected; refusing to start" |
| fd 0 not a listening AF_UNIX stream socket | exit 78 "push sender socket setup failed" |
| Peer uid ≠ instance uid | connection closed, nothing read |
| Client: no candidate exists, wrong owner, symlink, wrong peer | `TransportError::Unavailable` (fixed text) |

## 10. Invariants touched

RMC-S34/S36/S41/S42 (sender hygiene: no new env read, no `unsafe`, no process spawn, constant
log strings), RSH11/RSH12 (sandbox self-check, single env read), RMC-S35/RSH1/RSH9 (legacy user
unit unchanged), SET* (setup companion flow: **migrations listed in §13**).

## 11. Matrix rows (new prefix PSS)

| Row | Criterion | Evidence |
|---|---|---|
| PSS1 | `@.service` is exactly the §2.2 line set; `@.socket` exactly §2.1 | invariants `test_pss1_system_units_are_exact` |
| PSS2 | Unit literals equal the protocol constants (`/run/soos-push/%i/push.sock`, `--system-instance %i`, `/usr/libexec/soos/soos-push-sender`) | invariants `test_pss2_units_match_protocol_constants` |
| PSS3 | `system_push_socket_path` and constants | push-protocol `test_pss3_system_socket_path` |
| PSS4 | `parse_run_mode` accepts exactly the specified forms | push-sender `test_pss4_parse_run_mode` |
| PSS5 | `check_instance_identity` refuses root and the shared uid | push-sender `test_pss5_instance_identity` |
| PSS6 | `evaluate_system_sandbox` rules | push-sender `test_pss6_*` |
| PSS7 | `listener_from_fd` accepts only a listening AF_UNIX stream socket | push-sender `test_pss7_listener_from_fd` |
| PSS8 | Binary in system mode exits 78 on bad args / outside its sandbox, before serving | push-sender `test_pss8_*` |
| PSS9 | `main` order and no new environment read | invariants `test_pss9_main_dispatch_order` |
| PSS10 | Client candidates, peer uid per candidate, first existing candidate only | remote `test_pss10_*` |
| PSS11 | install.sh installs the binary and both units, never enables; uninstall removes them; rpm `%files` | invariants `test_pss11_*` |
| PSS12 | setup: system phase enables the socket, companion never enables the user unit and disables a legacy one; dry run writes nothing | invariants `test_pss12_*` (new SET-harness tests) |
| PSS13 | Docs, ADR, walkthrough | invariants `test_pss13_docs_adr_walkthrough` |
| PSS14 | Under a real systemd (ubuntu:24.04, PID 1) the socket listens `0600 <uid>`, a connection starts the sender as a dynamic uid that passes its sandbox check, another user is refused | Docker `tests/docker/push_sender_system_unit_test.sh` |

## 12. ADR draft

"[2026-10-23] Push Sender as a Sandboxed System Service (amends 2026-10-06 Web Push and
2026-10-12 Security Hardening)": see `AI/DECISIONS.md`.

## 13. Existing tests that the setup change contradicts (owner-approved migration required)

The owner decision moves the sender out of the user manager; `setup.sh` must therefore stop
enabling the user unit. These pre-existing assertions pin the old behaviour and are **not**
changed by this branch (test-integrity rule); the lead applies them after owner approval. The
exact list, with red evidence, is in `AI/plan_evaluator_report.md` §6 and the walkthrough §4.
