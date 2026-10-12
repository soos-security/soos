# Architect Spec — soos-remote Security Hardening (review of 2026-10-12)

- **Date**: 2026-10-12
- **Round**: 2 (Phase 1, architect-agent; revises round 1 `1702291` after `AI/plan_evaluator_report.md`
  round 1 `f5058b6`, REVISION_REQUIRED; resolution log in §16). Round 2 APPROVED by the plan evaluator
  (`8883c1c`); its non-blocking items PE-SR8 to PE-SR11 are folded in (log in §17)
- **Branch**: `fix/remote-security-hardening`, created from `origin/main` at `f9234ea` (#350 Android merged).
  GitHub-only security-review fix: no backlog id, **not** registered in `scripts/sync_issue.py`, no
  `AI/BACKLOG.md` entry, commits carry no issue reference.
- **Scope**: every finding of the 2026-10-12 security review of `soos-remote`, the Web Push sender, the live
  camera view and the daemon preview channel (S-1 … S-8d). Each finding was re-verified in the code (and S-1 on
  the host) before designing its fix (§1).
- **Matrix**: new component prefix **`RSH`** (remote security hardening), rows **RSH1–RSH18** (§12). RSH18 is the
  owner hardware check.
- **ADR**: new entry **"[2026-10-12] soos-remote Security Hardening"** in `AI/DECISIONS.md` (draft in §14).
- **Walkthrough**: **`AI/walkthroughs/194_remote_security_hardening.md`** (193 is the highest existing number).
- **Test ids**: static invariants `test_rsh_s1_*` … in a new module `tests/invariants/src/remote_security_hardening_contract.rs`
  (registered in `tests/invariants/src/lib.rs`); behaviour tests `test_rsh_*` in the crates' test directories (§11).
- **Agent limits for every later phase**: never push, open a PR, run `./save.sh` or merge; never install, enable,
  start, stop or restart a unit (`soos-daemon`, `soos-remote`, `soos-push-sender`); never run `tailscale`, `sudo`
  or `curl` against a live endpoint. Commits are local only.

---

## 0. Summary of decisions

| Finding | Verified | Decision (one line) |
|---|---|---|
| S-1 MAJOR sender reads the system journal | **Confirmed on the host** (§1.1) | Add `InaccessiblePaths=-/var/log -/var/lib/systemd/coredump` to `packaging/soos-push-sender.service` (verified to block the read in a user unit on systemd 262); correct docs and ADR wording; CM-SR1 on test 45. |
| S-2 MINOR Tailscale minimum version | Confirmed (docs say 1.38.3, trust verified on 1.102.4 only) | Raise the documented minimum to **1.102.4** for Serve and Funnel, single source `TAILSCALE_MIN_VERIFIED_VERSION` in `crates/remote/src/lib.rs`; the installer **prints** the requirement and the check command (it must not run `tailscale`: RMC-S6 forbids it, see §3.3). |
| S-3 MINOR preview starves face auth | Confirmed | Daemon: one **auth-reserved connection per unprivileged UID** above `max_connections_per_uid`, usable only by an `Auth` request whose **whole first frame** arrives within 200 ms; companion: graceful close on its retry path too. |
| S-4 MINOR Android lock-screen privacy | Confirmed (docs gap) | Docs only: `Docs/REMOTE_COMPANION.md` §2h gains a lock-screen privacy item (Android setting + Chrome per-app setting + `push_previews = "generic"`). |
| S-5 MINOR preview served to SSH processes | Confirmed | Daemon: an unprivileged preview peer must also **run inside** a local seat session scope, or under its own user manager while the UID owns no remote session (the root `Auth` user-manager rule); other placements keep today's rule. |
| S-6 SUGGESTION missing response headers | Confirmed | New `HARDENING_HEADERS` constant appended to every response head (HSTS, COOP, CORP, Permissions-Policy); `MANDATORY_HEADERS` and the CSP stay byte-identical. |
| S-7 SUGGESTION sender never checks its sandbox | Confirmed | Sender start-up self-check (pure, injectable): `/run` read-only tmpfs, the home directory (derived from the running binary's own path, **never from the environment**: RMC-S34) under a read-only tmpfs, `/var/log` not listable; otherwise exit 78 before binding. |
| S-8a installer follows a dangling symlink | Confirmed (reproduced in scratch) | Refuse a symlinked `remote.toml`; create the template under `set -C` (noclobber, `O_EXCL`). |
| S-8b one JPEG after a stop | Confirmed | Re-check `runtime.stop_requested(view)` after `mark_streaming` and before the head is written. |
| S-8c stale push subscription slot | Confirmed (`upsert` keys by endpoint only) | The worker sends the old endpoint as optional `replacesEndpoint`; the server replaces it atomically under the store lock. |
| S-8d MJPEG / NV12 / >480-px frames | Confirmed | Daemon converts frames for the **remote companion origin only** to Grey/YUYV/RGB24 within 640x480 (existing bounded decoder and decimation); `soos-remote` stays decoder-free and strict. |

No finding was false. Nothing touches the root PAM path, `pam_soos.so`, the IPC wire format or the policy crate.

---

## 1. Verification of the findings (evidence)

### 1.1 S-1 (reproduced 2026-10-12 on the owner host, systemd 262, user `willi363` in `wheel`)

`/var/log/journal` is `drwxr-sr-x+ root:systemd-journal` (the `+` ACL grants `wheel`/`adm` read). Transient user
units (`systemd-run --user --wait --pipe`, not a soos unit) were run with the sender's directives (`ProtectSystem=strict`,
`ProtectHome=tmpfs`, `TemporaryFileSystem=/run:ro`, `PrivateTmp`, `PrivateDevices`, `PrivateIPC`, `NoNewPrivileges`,
`CapabilityBoundingSet=`, `RestrictNamespaces`, `ProtectKernelLogs`, `SystemCallFilter=@system-service`):

| Variant | `journalctl -n1 _UID=0` lines | `/var/lib/systemd/coredump` | `/var/log` | DNS (`getent hosts fcm.googleapis.com`) |
|---|---|---|---|---|
| packaged directives | **1 (readable)** | listable | listable | ok |
| + `InaccessiblePaths=-/var/log -/var/lib/systemd/coredump` | **0** | blocked | `Permission denied` | ok |

Mount table inside the hardened variant: `/var/log` and `/var/lib/systemd/coredump` are `ro,noexec` tmpfs-backed
inaccessible nodes; `/run` is a `ro` tmpfs; `/home` a `ro` tmpfs. `TemporaryFileSystem=/var:ro` was rejected as an
alternative: it collides with `PrivateTmp=` (`/var/tmp`) and hides nothing more that matters. The claim of the
2026-10-06 Web Push ADR and of `Docs/REMOTE_COMPANION.md` §2d ("cannot read your files … nothing under `$HOME`
or `/run`") is literally true but omits the journal, so the docs are corrected (§2.3).

### 1.2 Other findings

- **S-2**: `Docs/REMOTE_COMPANION.md` L177 "Tailscale 1.38.3 or later"; `crates/remote/src/identity.rs` header
  comment and ADR 2026-10-06 item (2) rely on `tailscaled` deleting client copies of `Tailscale-User-Login` and
  `Tailscale-Funnel-Request`, verified only on 1.102.4. The tailnet (`serve`) path relies on the same stripping.
- **S-3**: `DEFAULT_MAX_CONNECTIONS_PER_UID = 2` (`crates/daemon/src/limits.rs` L23); admission before any read
  (`dispatcher.rs` L424); `camera_ipc.rs` `exchange` closes the old connection with a 200 ms bounded
  `graceful_close`, and its retry path (`reused && Retryable`) simply `drop`s the connection and reconnects at
  once, so two connections of the UID can coexist; with `soos-gui` open, a user-level lock screen `Auth` is refused
  at admission (`PerUidCap`, password fallback). Fail-safe but a usability/DoS defect.
- **S-4**: §2h has no lock-screen guidance; `push_previews` defaults to `Detailed` (`crates/remote/src/config.rs` L542).
- **S-5**: `dispatcher.rs` L1192 checks only `has_local_seat_session(peer_uid)` (any session of the UID); the peer
  PID's own session is never consulted for preview, unlike `authorize_root_peer` for `Auth`.
- **S-6**: `MANDATORY_HEADERS` (`http.rs` L263) has none of HSTS, COOP, CORP, Permissions-Policy.
- **S-7**: `crates/push-sender/src/main.rs` checks only root and `XDG_RUNTIME_DIR`.
- **S-8a**: `[[ ! -e "${CONFIG_PATH}" ]]` is **true** for a dangling symlink (`-e` follows it), then `cat >` and
  `chmod 0600` follow the link. Reproduced in the scratchpad: `set -C; cat > link` refuses a dangling symlink
  ("cannot overwrite existing file"); `install -Dm644` replaces a symlink instead of following it (units and
  binaries are therefore already safe).
- **S-8b**: `server.rs` `camera_stream`: between `FirstFrame::Ready` and `write_bounded(head)`/`write_part` no stop
  check exists; `mark_streaming` keeps the stop flag (`camera_slot.rs` L286), so the check after it is exact.
- **S-8c**: `PushStore::upsert_ref` (`push.rs` L367) matches by endpoint only; a renewal adds a new endpoint and
  the stale one occupies one of `MAX_PUSH_SUBSCRIPTIONS` (4) slots until a 404/410 removes it (only on a delivery).
- **S-8d**: `camera.rs` L376–384 accepts only Grey/YUYV/RGB24; `camera_jpeg.rs::validate` requires
  `16..=640 x 16..=480`; `preview_image_for_frame` forwards MJPEG of any size unchanged and any frame ≤ 640 wide
  at any height. A 640x480 MJPEG webcam ends `camera_format_unsupported`; a 640x512 Grey IR sensor ends after
  `CAMERA_MAX_BAD_FRAMES` (10).

---

## 2. S-1 — Sender sandbox hides the system logs

### 2.1 Change

`packaging/soos-push-sender.service`, `[Service]`, one new line **directly after** `TemporaryFileSystem=/run:ro`:

```ini
InaccessiblePaths=-/var/log -/var/lib/systemd/coredump
```

The `-` prefix ignores a path absent on the host. Supported in user units (the user manager implies
`PrivateUsers=`, as for the existing `ProtectSystem=`/`ProtectHome=`; verified on systemd 262, §1.1). The unit's
header comment gains: "The system logs (/var/log, the journal included, and /var/lib/systemd/coredump) are
inaccessible: the session owner may be in wheel, adm or systemd-journal."

### 2.2 Test amendment CM-SR1 (owner-approved)

`tests/invariants/src/remote_push_contract.rs`, test 45 `test_rmc_s35_sender_unit_is_sandboxed`:
`SENDER_UNIT_LINES: [&str; 37]` becomes `[&str; 38]` with exactly one new element
`"InaccessiblePaths=-/var/log -/var/lib/systemd/coredump",` inserted after `"TemporaryFileSystem=/run:ro",`.
Nothing removed or reordered; the forbidden list and the `soos-remote.service` assertion are unchanged
(`InaccessiblePaths=` does not start with any forbidden prefix).

### 2.3 Docs

- `Docs/REMOTE_COMPANION.md` §2d *How it works*: "Its sandbox hides your home directory (except its own binary),
  `/run` (except its socket directory) **and the system logs (`/var/log`, the journal included, and
  `/var/lib/systemd/coredump`), which your `wheel`/`adm`/`systemd-journal` membership would otherwise let it
  read**". §2d *Limits*: state that the sandbox is verified on hardware (RSH18) and that a host on a systemd
  older than the supported range must be checked with `systemd-analyze --user security soos-push-sender`.
- `AI/ARCHITECTURE.md` §13 push sender row: "`$HOME`, `/run` and the system logs hidden".
- The new ADR (§14) records the correction of the 2026-10-06 Web Push ADR claim.

### 2.4 Residual

A compromised sender still sees world-readable system files outside these paths (`/etc`, `/usr`) and the
`/proc` entries of other processes of the UID (no PID namespace: `ProtectProc=`/`ProcSubset=` are unsupported in
user units and forbidden by test 45; `PrivatePIDs=` works on systemd 262 but is left out of scope to keep the
minimum-systemd range unchanged — follow-up candidate). `ptrace` is blocked by `SystemCallFilter=@system-service`
and Yama scope 1 on the owner host.

---

## 3. S-2 — Verified Tailscale minimum version

### 3.1 Constant (single source of truth)

`crates/remote/src/lib.rs`:

```rust
/// Oldest Tailscale release whose header handling was verified (it deletes client copies of
/// `Tailscale-User-Login` and `Tailscale-Funnel-Request` before setting them): the owner host
/// check and the source review of 2026-10-06. Documentation and installer text only.
pub const TAILSCALE_MIN_VERIFIED_VERSION: &str = "1.102.4";
```

No code path reads it at runtime (the service cannot know the proxy version); it is the anchor the invariant
test compares the docs and installer against.

### 3.2 Docs

- `Docs/REMOTE_COMPANION.md` §2b *Prerequisites for Funnel*: "Tailscale **1.102.4 or later** on this PC (the
  release whose stripping of client-supplied `Tailscale-User-Login` and `Tailscale-Funnel-Request` headers was
  verified; an older `tailscaled` might forward a forged identity header from an internet client, which the
  service would then treat as a tailnet caller)". The literal `1.38.3` disappears from the file.
- §2 *Trust model* and §4 *Installation*: one sentence each: the tailnet path relies on the same stripping, so
  the same minimum applies to `tailscale serve`; check with `tailscale version` (owner action).
- §8 *Residual limitations*: a downgrade of `tailscaled` below the minimum re-opens the forged-header risk; the
  service cannot detect the proxy version.

### 3.3 Installer (no `tailscale` invocation)

The review asked for a best-effort `tailscale version` parse in `scripts/install_remote.sh`. **This conflicts with
the existing contract `test_rmc_s6_installer_refuses_root_and_never_runs_privileged_commands` (RMC-S6)**, which
allows the word `tailscale` only inside `echo`/`printf` lines; exempting a line would relax that assertion, which
the test-integrity rule forbids. Decision: the installer **prints** the requirement in its "Next steps" (step 3,
before the `tailscale serve` line):

```bash
    echo "     Requires Tailscale 1.102.4 or later on this PC (check with: tailscale version);"
    echo "     older releases were not verified to strip forged identity headers."
```

(`echo` lines only; RMC-S6 unchanged and still green.) Refusing an install is not possible without running
`tailscale`; the requirement is enforced by documentation and owner action (residual, §13).

---

## 4. S-3 — Face authentication is never starved by preview connections

### 4.1 Constants (`crates/daemon/src/limits.rs`)

```rust
/// Extra connections per unprivileged UID admitted only for one facial `Auth` request when the
/// UID already holds `max_connections_per_uid` connections (preview clients cannot starve the
/// user-level lock screen). Root peers never need it (no per-UID cap).
pub const AUTH_RESERVED_CONNECTIONS_PER_UID: usize = 1;

/// Time an auth-reserved connection may take to deliver its **whole first frame** (4-byte
/// length prefix and body) before it is closed without an answer. PAM connects, then shows
/// "[soos] Looking for face..." through the application's PAM conversation (`on_connected`,
/// `crates/pam/src/ipc.rs`), then writes its `Auth` request: that conversation call must
/// return within this window (residual §13.3). Kept <= 250 ms (pinned by RSH5): two existing
/// contracts close an idle over-cap connection against a 500 ms deadline.
pub const AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS: u64 = 200;
```

Not configurable (no new `daemon.toml` key, no `PeerLimitsConfig` field; pinned by RSH5, PE-SR7). Bound per UID becomes `max_connections_per_uid + 1`; global and
unprivileged capacities are unchanged and still bound everything (the reserve consumes an unprivileged permit).

### 4.2 Types (`crates/daemon/src/limits.rs`)

```rust
/// How a connection was admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionClass {
    /// Regular per-UID permit (any request).
    Regular,
    /// Auth-reserved permit: the first and only request must be `RequestKind::Auth`.
    AuthReserve,
}

struct LimiterState {
    total: usize,
    unprivileged: usize,
    per_uid: HashMap<u32, usize>,          // regular permits (unchanged meaning)
    reserved_per_uid: HashMap<u32, usize>, // NEW: auth-reserved permits, ≤ AUTH_RESERVED_CONNECTIONS_PER_UID
}

impl PeerConnectionLimiter {
    /// Unchanged contract (regular permits; `PerUidCap` at the cap).
    pub fn try_acquire(&self, peer_uid: u32) -> Result<PeerConnectionPermit, ConnectionRejection>;

    /// Admits one auth-reserved connection of an unprivileged `peer_uid`.
    /// Order: `GlobalCapacity`, then `ReservedForPrivileged` (unprivileged capacity), then
    /// `PerUidCap` when the UID already holds `AUTH_RESERVED_CONNECTIONS_PER_UID` reserved permits.
    /// Root (`peer_uid == 0`) → `Err(PerUidCap)` (never used: root has no per-UID cap).
    pub fn try_acquire_auth_reserve(&self, peer_uid: u32)
        -> Result<PeerConnectionPermit, ConnectionRejection>;

    /// Distinct unprivileged UIDs holding a regular or reserved permit (bounded by capacity).
    pub fn tracked_uids(&self) -> usize; // now counts the union of both maps
}

impl PeerConnectionPermit {
    pub const fn peer_uid(&self) -> u32;          // unchanged
    pub const fn class(&self) -> AdmissionClass;  // NEW
}
```

`Drop` releases the matching map entry (removed at zero) and the counters with saturating arithmetic, as today.
No `unwrap`/`expect`; the poisoned-lock recovery is unchanged.

### 4.3 Dispatcher (`ConnectionDispatcher::handle_connection` / `read_and_process`)

1. `try_acquire(peer.uid)`; on `Err(PerUidCap)` only, try `try_acquire_auth_reserve(peer.uid)`; any other
   rejection, or a failed reserve, keeps today's path (warn line, `ConcurrencyLimitReached`, closed unread).
2. Reserve admission: one `debug!` line (`peer_uid` only). No separate readiness wait (round 1's
   `stream.readable()` bounded only the first byte, PE-SR3). Instead the frame read of `read_and_process` (Steps 2–3
   today: `read_exact` of the 4-byte prefix, the `MAX_MESSAGE_SIZE`/zero check, `read_exact` of the body) is
   factored into a private `async fn read_client_frame(stream: &mut UnixStream) -> Result<Vec<u8>, DaemonError>`
   (byte-for-byte the same checks and errors). `read_and_process` calls it directly for `Regular` and, for
   `AuthReserve`, as `timeout(Duration::from_millis(AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS), read_client_frame(stream))`:
   on elapse, `warn!` (constant message, `peer_uid`) and `Err(DaemonError::Timeout)` (closed, no byte written,
   permit released). An EOF inside the window keeps today's `UnexpectedEof` handling (clean close). The outer
   `connection_timeout` window and `request_started` are unchanged (the reserve window is nested inside it, so no
   budget grows). Because a reserve connection serves at most one request (step 3–4: `Auth` is one-shot, anything
   else closes), the reserve window always covers the first and only frame.
3. `read_and_process` gains a parameter `admission: AdmissionClass`. After `decode_client_message`, when
   `admission == AuthReserve` and the message is not `ClientMessage::Request` with `kind == RequestKind::Auth`, it
   returns `Err(DaemonError::Protocol(..))` **before** `handle_request`/`handle_event` (no processing, no reply,
   connection closed). `PasswordFailed` events are not admitted on the reserve (telemetry only; residual §13).
4. An `Auth` on the reserve goes through the unchanged path (wire validation, `verify_peer_credentials`,
   `authorize_auth`, rate limit, pipeline) and is one-shot as today.

Latency: the regular path and every root path are unchanged; `crates/pam` is not modified. A reserve connection
adds no wait of its own: the frame read returns as soon as PAM's request is complete in the socket buffer. That
happens **after** PAM's `on_connected` callback (`crates/pam/src/ipc.rs` `authenticate_before_with_progress`:
connect → `on_connected()` → nonce → encode → write; `crates/pam/src/lib.rs` sends `PAM_TEXT_INFO` through the
application's conversation in that callback), so a lock screen whose conversation call takes ≥ 200 ms loses the
reserve: the daemon closes unanswered, PAM's read fails, `PAM_IGNORE`, password fallback (fail-safe; the regular
path without contention is unaffected because it has no such window). Typical conversation calls (swaylock-plugin,
GDM, `sudo`'s terminal) return in well under 200 ms; residual §13.3. Moving `on_connected` after the write was
rejected: it would change the PAM path and its tests for a fail-safe edge case.

Bound held by one same-UID process on the reserve (PE-SR3): ≤ 200 ms unless it delivers a complete frame; a
complete non-`Auth` frame closes at once; a complete `Auth` frame is served once under the unchanged
`connection_timeout`, rate limiter and one-shot rule (the same cost the UID can already impose on a regular
permit).

### 4.4 Companion (`crates/remote/src/camera_ipc.rs`)

In `DaemonPreviewClient::exchange`, the retry branch replaces `drop(conn)` with `graceful_close(conn.stream).await`
before `self.connect()` (the daemon already closed an idle connection, so the drain ends at once on EOF; bounded by
`CAMERA_DAEMON_CLOSE_WAIT_MS`). No constant changes.

### 4.5 Docs

`Docs/REMOTE_COMPANION.md` §2f *Interaction with face unlock* and §7 troubleshooting row: a face request of a
user-level lock screen gets the auth-reserved connection even when the view and `soos-gui` hold both regular
connections (`max_connections_per_uid` still mentioned, RLC-S8 needle). `Docs/DAEMON.md` `[peer_limits]` (L144,
L151) and `Docs/IPC_PROTOCOL.md` admission step 2 (L339): the reserve, its 200 ms whole-first-frame window, its
`Auth`-only rule, and (IPC_PROTOCOL and REMOTE_COMPANION §2f) the dependency on the lock screen's PAM conversation
answering the "Looking for face" message within that window (PE-SR2). `AI/ARCHITECTURE.md` §13 last sentence of the live camera paragraph.

---

## 5. S-4 — Android lock-screen privacy (docs only)

`Docs/REMOTE_COMPANION.md` §2h: new item **"Lock-screen privacy"** inserted after item 3 (renumber the following
items; item 10 becomes 11 and keeps "matrix row RAN12"):

> Notifications are shown on the **locked** phone with their full text by default (`push_previews` defaults to
> `"detailed"`: source, account and count). To hide it: Settings → Notifications → **Notifications on lock
> screen** → *Hide sensitive notifications* (or *Show conversations, default and silent* without content; names
> vary by vendor: Samsung "Lock screen → Notifications → Hide content"), and per app: Settings → Apps → Chrome →
> Notifications → the site's channel → *Lock screen* → *Hide sensitive content*. Or set
> `push_previews = "generic"` in `remote.toml`: the text is then only "Security alert on your PC — Open soos for
> details" on every phone.

§2d keeps its iPhone sentence and gains "(Android: section 2h)".

---

## 6. S-5 — Preview only for processes inside a local seat session

### 6.1 Types (`crates/daemon/src/session_policy.rs`)

```rust
/// Where a peer process runs, from its `/proc/<pid>/cgroup` (systemd hierarchies only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerPlacement {
    /// Inside the logind session scope `session-<id>.scope` (id validated by `is_valid_session_id`).
    Session(String),
    /// Under `user@<uid>.service` (strict, consistent UIDs; `parse_user_manager_uid_from_cgroup`).
    UserManager(u32),
    /// Neither (system services with `User=`, containers, the root cgroup, CI runners).
    Other,
}

/// `parse_session_id_from_cgroup` first (`Some(id)` → `Session(id)`), then
/// `parse_user_manager_uid_from_cgroup` (`Ok(Some(u))` → `UserManager(u)`, `Ok(None)` → `Other`).
/// # Errors
/// `SessionDenial::UserManagerCgroupMalformed` for a malformed user-manager path or disagreeing lines.
pub fn classify_peer_placement(cgroup: &str) -> Result<PeerPlacement, SessionDenial>;

impl LocalSessionPolicy {
    /// Preview placement rule for an unprivileged `peer_uid` (ADR 2026-10-12, S-5).
    /// Disabled policy → `Ok(())`. Otherwise:
    /// - `Session(id)`: `source.session(id)` must exist and pass `check_local_seat_session_of(peer_uid)`;
    ///   absent record → `CallerSessionUnresolved`, read error → `LogindUnavailable` (fail closed);
    /// - `UserManager(u)`: `u == peer_uid` (else `UserManagerUidMismatch`), then exactly the rule of
    ///   `authorize_user_manager_caller` (no record of the UID, or without UID, other than `REMOTE=0`
    ///   → `UserManagerCallerRemoteSessionActive`; a local seat session required);
    /// - `Other`: `Ok(())` (today's rule: `has_local_seat_session` stays the gate, §6.3).
    /// # Errors
    /// The `SessionDenial` reason.
    pub fn authorize_preview_placement(&self, peer_uid: u32, placement: &PeerPlacement)
        -> Result<(), SessionDenial>;
}
```

`authorize_user_manager_caller`'s record checks are factored into one private helper used by both the root `Auth`
path and the preview path; the root `Auth` behaviour is unchanged (same denials, same order).

### 6.2 Dispatcher

- `PreviewPeerState` gains `placement: Option<PeerPlacement>`; `classify_preview_peer` reads the cgroup **once per
  connection** (unchanged source `preview_peer_source`) and returns both the origin
  (`classify_preview_peer_cgroup`) and the placement (`classify_peer_placement`); either error refuses
  (unchanged refusal, `ProtocolError`/`UidMismatch`).
- `handle_preview_request` order: 6c-1 `authorize_preview`; 6c-1a origin/placement; 6c-1b `remote_view`;
  6c-2 `has_local_seat_session(peer_uid)` (**kept**, RLC invariant needle); **6c-2a NEW**
  `self.session_policy.authorize_preview_placement(peer_uid, &placement)` evaluated on **every** preview request
  (the session may end or turn remote), refusal = `ProtocolError`/`UidMismatch`, one `warn!` with
  `peer_uid` and `denial.as_str()`; 6c-3 rate limit … unchanged. Root peers skip 6c-2/6c-2a as today.
- The policy is the dispatcher's `session_policy` (built from the session validator: same sessions directory and
  enforcement flag), so `with_session_validator(SessionValidator::disabled())` disables the new rule too.

### 6.3 Why `Other` is allowed

A process of the UID outside both its session scopes and its user manager can only be placed there by root
(a system unit with `User=`, a container runtime, a CI runner). From an SSH session the owner reaches
`user@<uid>.service` (through `systemd-run --user`), which the user-manager rule refuses while the SSH session
exists. Refusing `Other` would also break the existing contract
`test_preview_frame_allowed_for_configured_uid_serves_frames` on CI runners (the test process runs in a system
unit). Residual, §13.

### 6.4 Consequences

- `soos-gui` started from a TTY-launched compositor (session scope) and from GNOME/KDE (app scope under the user
  manager) keeps working; `soos-remote` (user manager) keeps working **unless the owner has a remote (SSH) session
  open at the same time** — the same rule the root `Auth` path applies; documented in §2f and §7. A process left
  running from an SSH login (for example `tmux`, with logind's default `KillUserProcesses=no`) keeps that session
  record (`STATE=closing`, `REMOTE=1`) after logout, so the refusal lasts until that process ends (PE-SR11).
- `Docs/IPC_PROTOCOL.md` decision table: new row 6c-2a; `Docs/DAEMON.md` §1.5 one paragraph;
  `Docs/REMOTE_COMPANION.md` §2f requirements list and troubleshooting row "Camera: `403 camera_refused` while
  an SSH session of yours is open" (PE-SR11: worded "an SSH session of yours, **or a process left running from
  one** (e.g. `tmux`), is open"; check with `loginctl list-sessions` (a `remote` session of your user, possibly in
  state `closing`), fix by ending that process or with `loginctl terminate-session <id>`; the same row notes that a
  `soos-gui` started as a user unit is refused for the same reason).

---

## 7. S-6 — Hardening response headers

`crates/remote/src/http.rs`, directly after the untouched `MANDATORY_HEADERS`:

```rust
/// Browser hardening headers carried by every response (ADR 2026-10-12, S-6), after
/// `MANDATORY_HEADERS`. Every reachable path is HTTPS behind `tailscale serve`/`funnel`
/// (`--http` is refused with `421`), so HSTS is always correct; WebAuthn
/// (`publickey-credentials-*`) keeps its default `self` allowance.
const HARDENING_HEADERS: &str = "Strict-Transport-Security: max-age=31536000\r\n\
    Cross-Origin-Opener-Policy: same-origin\r\n\
    Cross-Origin-Resource-Policy: same-origin\r\n\
    Permissions-Policy: accelerometer=(), camera=(), display-capture=(), geolocation=(), \
    gyroscope=(), magnetometer=(), microphone=(), midi=(), payment=(), usb=()\r\n";
```

`encode_response_head`, `encode_sse_head` and `encode_camera_stream_head` emit `{MANDATORY_HEADERS}{HARDENING_HEADERS}`.
`MANDATORY_HEADERS` (and the CSP) stay byte-identical (`PINNED_MANDATORY_HEADERS`, RLC-S6). No
`includeSubDomains`, no `preload` (the node name is not ours to preload). Static assets, notification icons and the
manifest are same-origin fetches, unaffected by CORP `same-origin`; the owner verifies the Android notification
icon and the iPhone home-screen icon (RSH18).

---

## 8. S-7 — Sender sandbox self-check

### 8.1 Home anchor without the environment (PE-SR1)

RMC-S34 (`test_rmc_s34_sender_policy_literals`, `tests/invariants/src/remote_push_contract.rs:429-439`) requires
every `env::var(` / `env::var_os(` / `std::env::vars` of `crates/push-sender/src` to read `"XDG_RUNTIME_DIR"`. The
round-1 design read `$HOME` and would have broken it; that contract may not be relaxed. The home directory is
therefore derived **from the path of the running binary**, never from the environment and never through
`std::env::home_dir()` (or `dirs`/`home` crates, or a `"HOME"` literal):

- The unit runs exactly `ExecStart=%h/.local/bin/soos-push-sender` and bind-mounts that same path read-only
  (`BindReadOnlyPaths=%h/.local/bin/soos-push-sender`), so inside the sandbox `std::env::current_exe()`
  (`readlink /proc/self/exe`, no environment, no NSS) is `<home>/.local/bin/soos-push-sender`.
- The home anchor is that path with the exact trailing components `.local/bin/soos-push-sender` removed
  (`Path::ends_with(SENDER_EXE_HOME_RELATIVE)`, which compares whole components, then three `parent()` steps;
  never `str::ends_with` on the text, so `x.local/bin/soos-push-sender` does not match). The result must be absolute, not `/`, and contain no `..` component.
- Rejected alternative: the passwd entry (`nix::unistd::User::from_uid(getuid())`; the workspace `nix` already has
  the `user` feature). Inside the sandbox `TemporaryFileSystem=/run:ro` hides `/run/systemd/userdb`, so for
  `systemd-homed`, LDAP or other non-`files` NSS users the lookup fails and the sender would refuse to start for a
  reason unrelated to its sandbox. It would also add an NSS module load into the only network-capable process.
- **Unavailable anchor = fail closed**: `current_exe()` error, a path not ending in those three components (binary
  run from elsewhere, a `" (deleted)"` suffix after the binary was replaced while running, a renamed binary), or an
  empty/relative/`/` remainder → `SandboxError::HomeUnresolved` → exit 78 (`EXIT_CONFIG`, in
  `RestartPreventExitStatus=78`). A manual run from `~/.local/bin` resolves the anchor but fails rule 2 (no tmpfs over
  the home) → exit 78 as well.
- **Home outside `/home`**: rule 2 is not tied to `/home`; it accepts any read-only tmpfs whose mount point (≠ `/`)
  is a component-wise prefix of the anchor. `ProtectHome=tmpfs` mounts its tmpfs over `/home`, `/root` and
  `/run/user`, resolving symlinks; `current_exe()` is the symlink-resolved path. So (a) a home in `/home/<u>` or
  `/root` is accepted; (b) on image-based distributions where `/home` → `/var/home`, the anchor is
  `/var/home/<u>` and is accepted exactly when the mount table shows the tmpfs at `/var/home` (the symlink-resolved
  mount point); (c) a home outside every `ProtectHome=` path (e.g. `/srv/<u>`, `/data/<u>`) is **not** hidden by the
  unit, so the sender correctly refuses to start (exit 78, `HomeVisible`). Case (c) and an unverified (b) are
  documented in `Docs/REMOTE_COMPANION.md` §2d *Limits* ("the push sender needs a home directory under `/home`
  (or a path `ProtectHome=` covers); elsewhere it refuses to start, because its sandbox could not hide it").

### 8.2 API (`crates/push-sender/src/lib.rs`)

```rust
/// Process mount table read by the start-up sandbox check.
pub const SANDBOX_MOUNTINFO_PATH: &str = "/proc/self/mountinfo";
/// Larger mount tables fail closed (the owner host: ~16 KiB).
pub const MAX_MOUNTINFO_BYTES: usize = 1_048_576;
/// Directory the unit makes inaccessible (`InaccessiblePaths=`).
pub const SANDBOX_HIDDEN_LOG_DIR: &str = "/var/log";
/// Path of the binary relative to the home directory, exactly as in the unit's
/// `ExecStart=%h/.local/bin/soos-push-sender` (pinned by RSH12).
pub const SENDER_EXE_HOME_RELATIVE: &str = ".local/bin/soos-push-sender";

/// Why the sandbox was not detected. Fixed texts, no path echoed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SandboxError {
    #[error("mount table unreadable")]   MountInfoUnreadable, // open/read error, > MAX, not UTF-8
    #[error("home not resolved")]        HomeUnresolved,      // see §8.1 (replaces round-1 `HomeUnset`)
    #[error("/run not hidden")]          RunVisible,
    #[error("home not hidden")]          HomeVisible,
    #[error("system logs readable")]     LogsReadable,
}

/// Pure: the home anchor of §8.1 from the running binary's path.
/// # Errors
/// `HomeUnresolved` when `exe` does not end with the three components of
/// `SENDER_EXE_HOME_RELATIVE`, or the remainder is relative, `/`, empty or contains `..`.
pub fn home_from_exe(exe: &Path) -> Result<PathBuf, SandboxError>;

/// Pure decision. `mountinfo`: content of /proc/self/mountinfo; `exe`: the running binary's path;
/// `logs_listable`: whether `read_dir(SANDBOX_HIDDEN_LOG_DIR)` succeeded.
/// Rules (order = error order):
/// 0. `home_from_exe(exe)` (→ `HomeUnresolved`);
/// 1. some entry with mount point exactly `/run`, fstype `tmpfs` and `ro` in the per-mount options;
/// 2. some entry T whose mount point P ≠ "/" equals the home or is a component-wise prefix of it, with fstype
///    `tmpfs` and `ro` in the per-mount options (ProtectHome=tmpfs), **and** (PE-SR9) no entry whose parent
///    mount ID (field 2) is T's mount ID (field 1) has a mount point that equals the home, is a component-wise
///    prefix of it, or lies below it, except exactly `<home>/SENDER_EXE_HOME_RELATIVE` (the unit's binary
///    bind); otherwise `HomeVisible`;
/// 3. `!logs_listable`.
/// Mount points are octal-unescaped (`\040`, `\011`, `\012`, `\134`); malformed lines are skipped; options
/// are matched as a comma-separated list (`ro` exactly, never a substring); mount IDs are decimal `u64`
/// (a line whose ID fields do not parse is malformed and skipped).
pub fn evaluate_sandbox(mountinfo: &str, exe: &Path, logs_listable: bool) -> Result<(), SandboxError>;

/// I/O wrapper: bounded read of SANDBOX_MOUNTINFO_PATH (take(MAX+1)), `std::env::current_exe()`
/// (error → `HomeUnresolved`), `std::fs::read_dir(SANDBOX_HIDDEN_LOG_DIR).is_ok()` (NotFound and
/// PermissionDenied both count as hidden). Reads no environment variable.
pub fn probe_sandbox() -> Result<(), SandboxError>;
```

Order-independent on purpose: the mount table lists the host's rw `/run` as well as the sandbox's ro `/run`
(observed, §1.1); a non-sandboxed host never has a read-only tmpfs at `/run` nor over the home. The resolver
drop-in of §2d (`BindReadOnlyPaths=-/run/systemd/resolve`) keeps rule 1 true (observed). No `unwrap`/`expect`/
`panic!`, no format argument in tracing (RMC-S36).

**Re-exposure on top of the tmpfs (PE-SR9).** Rules 1–2 as written in round 2 only proved that the tmpfs exists, so
a drop-in that *adds* `BindPaths=%h` (or `BindReadOnlyPaths=%h/.config`) kept them true. The second half of rule 2
closes this with the mount table alone (pure, unit-testable, no extra I/O): every mount systemd stacks on the home
tmpfs is a child of it in `/proc/self/mountinfo` (its parent ID is the tmpfs's mount ID), and the packaged unit
creates exactly one such child inside the home: the read-only bind of `<home>/.local/bin/soos-push-sender`. A bind
of the whole home (mount point = home), of a parent of it, or of any other path below it is refused (`HomeVisible`
→ exit 78). Deeper stacking chains through one of those children, so checking direct children suffices. The
directory-listing variant suggested by the evaluator (`read_dir(home)` lists only `.local`, …) was not chosen:
it depends on how systemd materialises intermediate directories in the tmpfs, which cannot be verified by an agent
on this branch (no unit may be started), whereas the mount relation is defined by the kernel. The owner hardware
check RSH18-1 (a test notification still arrives after reinstall) confirms the packaged unit passes this rule.
Re-exposure under `/run` (e.g. a drop-in `BindReadOnlyPaths=/run/dbus`) is **not** detected: the runtime directory
and the resolver drop-in legitimately add children there, and their exact set depends on the host; recorded in
§13.6.

### 8.3 `main.rs`

Right after `check_not_root` and **before** reading `XDG_RUNTIME_DIR` or binding: `if probe_sandbox().is_err()
{ tracing::error!("push sender sandbox not detected; refusing to start"); return exit_code(EXIT_CONFIG); }`
(one constant string, RMC-S36 hygiene; no reason variant logged beyond that constant). `install_logging();` stays the
first statement of `main` (RMC-S42). Exit 78 is in `RestartPreventExitStatus=78`, so a stripped drop-in or a manual
run stops for good. No override switch (an environment variable would need `Environment=`, forbidden by test 45, and
would be a second environment read, forbidden by RMC-S34). The only environment read of the sender stays the
existing `std::env::var_os("XDG_RUNTIME_DIR")` in `main.rs` (pinned by RSH12).

### 8.4 Docs

`Docs/REMOTE_COMPANION.md` §2d *Limits*: the sender refuses to start (exit 78, `journalctl --user -u
soos-push-sender` shows "sandbox not detected") when run by hand or when a drop-in removes `ProtectHome=tmpfs`,
`TemporaryFileSystem=/run:ro` or `InaccessiblePaths=`, when a drop-in mounts the home directory (or a path inside
it other than the sender binary) back on top of the hidden home (`BindPaths=%h`, PE-SR9), when its binary is not at
`~/.local/bin/soos-push-sender`, and when the home directory lies outside the paths `ProtectHome=` covers (§8.1
case c). It also says that a bind added under `/run` is not detected (§13.6).

---

## 9. S-8a … S-8d

### 9.1 S-8a installer (`scripts/install_remote.sh`)

Replace the `if [[ ! -e "${CONFIG_PATH}" ]]` block head with:

```bash
    if [[ -L "${CONFIG_PATH}" ]]; then
        echo "install_remote.sh: ${CONFIG_PATH} is a symbolic link; refusing to write through it" >&2
        exit 1
    elif [[ ! -e "${CONFIG_PATH}" ]]; then
        ...
        mkdir -p "${CONFIG_DIR}"
        (set -C; cat > "${CONFIG_PATH}") <<'TEMPLATE'
        ...
TEMPLATE
        chmod 0600 "${CONFIG_PATH}"
```

`set -C` (noclobber) creates with `O_EXCL`, which never follows a symlink planted between the test and the write.
`chmod` then targets the regular file just created. RMC-S6 needles (`allowed_logins = []`, `umask 077`, `0600`)
stay; the template text is unchanged (RLC/RAN template needles). `install -Dm…` lines are unchanged (they replace a
symlink, verified §1.2).

### 9.2 S-8b stop before the first part (`crates/remote/src/server.rs` `camera_stream`)

After `runtime.mark_streaming(ticket.view);` and before `write_bounded(.. encode_camera_stream_head())`:

```rust
if runtime.stop_requested(ticket.view) {
    log_unshown_end(ViewEnd::Stopped);
    drop(jpeg);
    drop(view);
    drop(guard);
    return camera_first_frame_refusal(&runtime, ViewEnd::Stopped); // audit_refused, None
}
```

The view never counts as shown; no head, no part.

Second check (PE-SR5): the head write can be pending (full socket buffer) while a stop is processed. After a
successful `write_bounded(.. encode_camera_stream_head())` and **before** `guard.set_shown()` /
`audit::camera_view_started()` / `write_part`:

```rust
if runtime.stop_requested(ticket.view) {
    runtime.audit_refused();
    drop(jpeg);
    finish_view(&mut write_half, ViewEnd::Stopped).await; // logs the end, writes the bounded trailer
    drop(view);
    drop(guard);
    return None; // head already sent: no JSON answer, no part, view not counted as shown
}
```

So no JPEG part is ever written after a stop observed before the first part. A stop raised after this second check
is handled by the existing stream loop (at most the already-checked first part is sent: the part write itself is
not interruptible, residual §13.9).

### 9.3 S-8c renewal replaces the stale endpoint

Server (`crates/remote/src/push.rs`):

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SubscribeBody {
    endpoint: String,
    #[serde(default)] expiration_time: Option<serde::de::IgnoredAny>,
    keys: SubscribeKeys,
    /// NEW (S-8c): endpoint of the subscription this one renews (`pushsubscriptionchange`).
    #[serde(default)] replaces_endpoint: Option<String>,
}

impl PushStore {
    /// One locked update: removes `old` when present and different from `sub.endpoint`, then
    /// upserts `sub` (capacity counted after the removal; `TooMany` writes nothing).
    pub fn replace_or_upsert(&self, sub: &PushSubscription, old: Option<&PushEndpoint>,
        random: &RandomSource) -> Result<RenewResult, PushStoreError>;
}
pub enum RenewResult { Added, Replaced, Renewed /* old removed, new added */ }
```

`subscribe`: `replacesEndpoint` absent → today's path byte for byte; present → parsed with `PushEndpoint::parse`;
**a parse error is treated exactly as an absent field** (PE-SR4: plain upsert of the new subscription with today's
`TooMany`/`409` rule, no `400`, no oracle on why), because the *old* endpoint may legitimately no longer parse (an
allowlist change, a browser format change) and the renewal must not lose the *new* subscription; then
`replace_or_upsert(sub, parsed_old_or_none, ..)`. The *new* endpoint and keys keep today's validation (`400` on
error). A `replacesEndpoint` of a non-string JSON type is a malformed body (`400`, as any wrongly typed field today);
the worker only ever sends a string. `Renewed` → `audit::push_subscription_removed()`
then `audit::push_subscription_added()`; answer `200 subscribed` in every success case (no oracle on whether the
old endpoint existed). Gates unchanged (path class, CSRF `X-Soos-Action: push-subscribe`, session/identity,
`MAX_PUSH_SUBSCRIBE_BODY_BYTES` = 2048 unchanged). An authenticated caller could already remove any endpoint
through `/api/push/unsubscribe`, so naming an arbitrary old endpoint grants nothing new.

Worker (`crates/remote/assets/sw.js`): new constant `const SUBSCRIBE_BODY_LIMIT = 2048;` (equal to
`MAX_PUSH_SUBSCRIBE_BODY_BYTES`, pinned by an invariant) and a helper

```js
// The renewed subscription, naming the endpoint it replaces when the body still fits the limit.
function renewalPayload(subscription, old) {
  const json = subscription.toJSON();
  if (old && typeof old.endpoint === "string" && old.endpoint !== json.endpoint) {
    const withOld = Object.assign({}, json, { replacesEndpoint: old.endpoint });
    if (new TextEncoder().encode(JSON.stringify(withOld)).length <= SUBSCRIBE_BODY_LIMIT) {
      return { toJSON: () => withOld };
    }
  }
  return subscription;
}
```

The listener becomes `renewedSubscription(event).then((subscription) => (subscription === null ? null :
postSubscription(renewalPayload(subscription, event.oldSubscription)))).catch(() => null)`. `postSubscription` is
unchanged (still `JSON.stringify(subscription.toJSON())`, one `fetch`, the two headers): every RAN-S2 assertion stays
true. The page (`app.js`) is unchanged.

`event.oldSubscription` may be `null` (some browsers do not provide it, PE-SR10); `renewalPayload` then returns the
subscription unchanged, i.e. today's request, and the stale slot stays until the next 404/410 or `soos-remote push
remove N`. Documented in `Docs/REMOTE_COMPANION.md` next to the `push remove` note (Android and iPhone) and in
§13.7.

### 9.4 S-8d frames the companion can always convert (daemon side)

`crates/daemon/src/preview_image.rs`:

```rust
/// Bounds of a preview prepared for the remote companion (`soos-remote` converts only Grey,
/// YUYV and RGB24 up to 640x480; it has no decoder by design, RLC-S10).
pub const REMOTE_PREVIEW_MAX_WIDTH: u32 = 640;
pub const REMOTE_PREVIEW_MAX_HEIGHT: u32 = 480;

/// Preview for a `PreviewPeerOrigin::RemoteCompanion` connection: the IR rule first (greyscale),
/// then a Grey/RGB24 frame within 640x480 (and YUYV with an even width) is forwarded unchanged;
/// anything else (MJPEG, NV12, odd-width YUYV, taller or wider frames) is decoded with the existing
/// bounded `soos_vision::convert_to_rgb` (MJPEG only) and decimated by the integer step
/// `max(ceil(w/640), ceil(h/480), 1)` into Grey (Grey sources) or RGB24; an unconvertible frame is
/// the explicit empty image. Output is never larger than 640x480x3 = 921 600 bytes
/// (= soos-remote `CAMERA_MAX_SCRATCH_BYTES`).
pub fn remote_preview_image_for_frame(frame: &Frame) -> PreviewImage;
```

**IR branch (PE-SR8).** The GUI function converts an IR-stamped non-Grey frame to Grey and recurses into
*itself*, which forwards any Grey frame ≤ 640 wide unchanged whatever its height. The remote function must not
reuse that branch as-is: its IR branch builds the same zeroizing luma `Frame` (`infrared_luma`, `SensorType::Infrared`,
`PixelFormat::Grey`) and recurses into **`remote_preview_image_for_frame`** (never `preview_image_for_frame`), so the
640x480 bound applies to the luma frame too; an IR frame already in Grey goes straight to the bounds rule.
Example: 640x512 YUYV `Infrared` → 320x256 Grey. An undecodable IR frame (`infrared_luma` → `None`) is the empty
image.

Implementation: `downscale` is generalised to `downscale_within(frame, max_width, max_height: Option<u32>)`;
`preview_image_for_frame` calls it with `(MAX_PREVIEW_WIDTH, None)` and stays bit-for-bit identical (GUI path and its
tests unchanged). Re-exported from `crate::preview` with the existing items. Dispatcher: `let mut image = if origin ==
PreviewPeerOrigin::RemoteCompanion { remote_preview_image_for_frame(&captured) } else { preview_image_for_frame(&captured) };`.
Cost: one bounded MJPEG decode per remote frame (≤ `camera_fps` ≤ 10/s, under the shared preview rate limit),
inline like the existing downscale and IR paths. It runs only in `handle_preview_request` for a remote-companion
origin, holds no shared lock (no `Mutex`/`RwLock` guard of the dispatcher, rate limiter, camera or pipeline is
alive across the call: the captured `Frame` is owned by then), and never runs on the `Auth` path (PE-SR6; checked
by RSH16's static assertion). `soos-remote` is unchanged: its strict validation stays the fail-closed
backstop (a malformed frame is still `Bad`/`Unsupported`). Example outcomes: 640x480 MJPEG → 640x480 RGB24;
640x512 Grey → 320x256 Grey; 1280x720 YUYV → 640x360 RGB24; 640x480 NV12 → 640x480 RGB24.

Docs: `Docs/REMOTE_COMPANION.md` §2f "Only Grey, YUYV and RGB24 …" becomes "the daemon converts MJPEG, NV12 and
frames larger than 640x480 for the companion; `soos-remote` itself still contains no image decoder"; §7 row
`camera_format_unsupported` updated (remaining cause: a daemon older than this change, or an unconvertible frame);
§9 drops "NV12/MJPEG" from the out-of-scope line (the line naming `live camera` and `recording` stays, RLC-S8).
`Docs/IPC_PROTOCOL.md` *Preview frame size*: one bullet for the remote-companion rule.

---

## 10. Invariants touched

| Invariant | Effect |
|---|---|
| ARCHITECTURE §2 fail closed / `PAM_IGNORE` | S-3 reserve refusals close without answer → PAM fallback unchanged; no new allow path. Root PAM path untouched. |
| §13 RC-1 (no network socket, never root) | Unchanged; sender sandbox tightened (S-1, S-7). |
| RC-4 (bounded) | S-3 per-UID bound `cap + 1`, 200 ms window; S-7 mount table ≤ 1 MiB; S-8c body limit unchanged. |
| RC-5 (no identity in logs) | New log lines carry `peer_uid` and constant texts only. |
| LC-3a (preview needs a local seat session) | Strengthened by the placement rule (S-5); ADR amends LC-3a. |
| RLC-S6 CSP byte-identical | Kept; hardening headers in a separate constant. |
| RLC-S10 no decoder in `soos-remote` | Kept; decoding happens in the daemon (S-8d). |
| RMC-S6 installer never runs `tailscale` | Kept (S-2 prints only). |
| Zeroization of pixel buffers | `remote_preview_image_for_frame` uses the existing zeroizing buffers (`PreviewImage` drop, `Zeroizing` decode buffer). |

---

## 11. Test plan (Phase 2 writes these; all must fail before the implementation)

### 11.1 Static invariants — new `tests/invariants/src/remote_security_hardening_contract.rs`

| Test | Row | Asserts |
|---|---|---|
| `test_rsh_s1_sender_unit_hides_system_logs` | RSH1 | exact line `InaccessiblePaths=-/var/log -/var/lib/systemd/coredump` present once, after `TemporaryFileSystem=/run:ro`; §2d mentions `/var/log` and `coredump`; no `TemporaryFileSystem=/var` |
| `test_rsh_s2_tailscale_min_version_is_single_sourced` | RSH3 | `TAILSCALE_MIN_VERIFIED_VERSION` = `"1.102.4"` in `crates/remote/src/lib.rs`; `Docs/REMOTE_COMPANION.md` §2b contains it and `1.38.3` appears nowhere in the file; installer has an `echo` line containing it and `tailscale version`; RMC-S6 logic re-applied: no non-echo line names `tailscale` |
| `test_rsh_s3_reserve_constants_and_dispatch` | RSH5 | `AUTH_RESERVED_CONNECTIONS_PER_UID = 1`, `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS = 200` in `limits.rs`, parsed as an integer and asserted `<= 250` (PE-SR7); the name appears neither in `crates/daemon/src/config.rs` nor inside the `PeerLimitsConfig` struct body (not configurable); `handle_connection` calls `try_acquire_auth_reserve(` only after a `PerUidCap` match; `read_and_process` wraps `read_client_frame(` in `timeout(` with `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS` for `AuthReserve` and compares the admission with `RequestKind::Auth` before `handle_request(`; no `.readable()` call in `dispatcher.rs` (PE-SR3); `camera_ipc.rs` retry branch calls `graceful_close(` and contains no `drop(conn)` |
| `test_rsh_s4_android_lock_screen_privacy_documented` | RSH7 | §2h contains `push_previews`, `"generic"`, `lock screen`, `Hide sensitive`; still contains `RAN12` |
| `test_rsh_s5_preview_placement_is_checked` | RSH9 | `handle_preview_request` body still contains `has_local_seat_session(` (RLC needle) **and** `authorize_preview_placement(`, the latter after the former and before the rate limiter; `session_policy.rs` defines `pub fn classify_peer_placement(` |
| `test_rsh_s6_hardening_headers_exact_and_csp_unchanged` | RSH10 | `PINNED_MANDATORY_HEADERS` still present; exact `HARDENING_HEADERS` constant; the three head encoders interpolate `{MANDATORY_HEADERS}{HARDENING_HEADERS}`; one CSP definition |
| `test_rsh_s7_sender_checks_its_sandbox_first` | RSH12 | `main.rs` calls `probe_sandbox()` after `check_not_root(` and before `XDG_RUNTIME_DIR` and `bind_socket(`; refusal returns `EXIT_CONFIG`; constant log string; `SENDER_EXE_HOME_RELATIVE = ".local/bin/soos-push-sender"` and the unit's `ExecStart=` line is exactly `ExecStart=%h/` followed by that value; `probe_sandbox` body contains `current_exe(` |
| `test_rsh_s7_sender_reads_no_other_environment_variable` | RSH12 | PE-SR1, stronger companion of RMC-S34: across `crates/push-sender/src`, the total count of `env::var(` + `env::var_os(` is exactly 1 and that occurrence reads `"XDG_RUNTIME_DIR"` in `main.rs`; no `env::vars`, `env::vars_os`, `home_dir`, `"HOME"`, `getenv`, `User::from_uid`, `getpwuid`; `lib.rs` contains no `env::var` at all |
| `test_rsh_s8b_stop_rechecked_before_head_and_before_first_part` | RSH14 | in `camera_stream`: one `stop_requested(` after `mark_streaming(` and before `encode_camera_stream_head()`, and a second after `encode_camera_stream_head()` and before `set_shown()` (PE-SR5); `write_part(` appears after both |
| `test_rsh_s8a_installer_refuses_symlinked_config` | RSH13 | `[[ -L "${CONFIG_PATH}" ]]` branch exits non-zero before any write; the template write is `(set -C; cat > "${CONFIG_PATH}")`; no other `cat >` to `CONFIG_PATH`; `bash -n` passes |
| `test_rsh_s8c_worker_names_the_replaced_endpoint` | RSH15 | `sw.js`: `SUBSCRIBE_BODY_LIMIT = 2048` equals `MAX_PUSH_SUBSCRIBE_BODY_BYTES`; listener passes `renewalPayload(subscription, event.oldSubscription)`; `replacesEndpoint` only in `renewalPayload`; still exactly one `fetch(` |
| `test_rsh_s8d_remote_conversion_off_the_auth_path` | RSH16 | PE-SR6: `remote_preview_image_for_frame(` is called only inside `handle_preview_request` in `dispatcher.rs`, after the `preview_limiter` guard's block has closed (its `lock().await` appears before and the call is not inside that block) |
| `test_rsh_docs_adr_and_walkthrough` | RSH17 | ADR title `[2026-10-12] soos-remote Security Hardening` in `AI/DECISIONS.md`; `AI/ARCHITECTURE.md` §13 mentions the system logs and the auth-reserved connection; walkthrough 194 exists; `Docs/IPC_PROTOCOL.md` has row `6c-2a` |

### 11.2 Behaviour tests

| File | Tests | Row |
|---|---|---|
| `crates/daemon/tests/peer_limits_tests.rs` (append) | `test_rsh_limiter_auth_reserve_admits_one_extra_per_uid` (2 regular, 1 reserve, 2nd reserve `PerUidCap`, other UID unaffected), `test_rsh_limiter_auth_reserve_respects_unprivileged_and_global_capacity`, `test_rsh_limiter_auth_reserve_released_on_drop_and_tracked`, `test_rsh_limiter_regular_cap_unchanged` (third `try_acquire` still `PerUidCap` while a reserve is held), `test_rsh_permit_reports_admission_class` | RSH4 |
| same file (socket, unprivileged test UID) | `test_rsh_auth_admitted_on_reserve_while_two_connections_held` (two held connections, third sends `Auth` → a `Response` with the request id), `test_rsh_reserve_closes_non_auth_without_payload` (Status, PreviewFrame, `PasswordFailed` event: closed, zero bytes, permit released), `test_rsh_reserve_idle_connection_closed_within_window` (nothing sent: closed within 500 ms), `test_rsh_reserve_partial_frame_closed_within_window` (PE-SR3; two cases: 1 byte then stall, and the 4-byte length prefix of a valid `Auth` plus half its body then stall: each closed within 500 ms of connect, zero bytes received, `available_permits()`/`tracked_uids()` back to the pre-connection values), `test_rsh_reserve_auth_after_100ms_is_answered` (PE-SR2; the reserve client sleeps 100 ms after connect, as a PAM conversation would, then writes a valid `Auth`: a `Response` with its request id is received), `test_rsh_reserve_auth_is_one_shot` | RSH5 |
| `crates/daemon/tests/preview_placement_tests.rs` (new) | `test_rsh_classify_peer_placement` (session scope, user manager, other, nested `session-*.scope` under the manager = `UserManager`, malformed manager path = error, disagreeing v1/v2 lines), `test_rsh_session_peer_needs_its_own_local_seat_session` (own record local → served; own record `REMOTE=1` while another local seat record exists → refused; own record absent → refused; inactive/greeter → refused), `test_rsh_user_manager_peer_refused_while_remote_session_exists` (served without; refused with a `REMOTE=1` or UID-less record; foreign manager UID refused), `test_rsh_other_placement_keeps_seat_rule`, `test_rsh_placement_rule_disabled_with_validator`, `test_rsh_placement_rechecked_per_request` (record turns remote between two requests on one connection → second refused), `test_rsh_root_auth_user_manager_rule_unchanged` | RSH8 |
| `crates/daemon/tests/preview_downscale_tests.rs` (append; dev-dependency `jpeg-encoder` from the workspace for valid MJPEG fixtures) | `test_rsh_remote_preview_decodes_mjpeg_640x480`, `test_rsh_remote_preview_bounds_tall_grey_ir_frame` (640x512 → 320x256 Grey), `test_rsh_remote_preview_bounds_wide_yuyv`, `test_rsh_remote_preview_converts_nv12`, `test_rsh_remote_preview_odd_width_yuyv_becomes_rgb`, `test_rsh_remote_preview_malformed_mjpeg_is_empty`, `test_rsh_remote_preview_ir_stamp_is_grey` (PE-SR8: a 640x512 YUYV `SensorType::Infrared` frame → format 1 Grey, 320x256, 81 920 bytes), `test_rsh_remote_preview_output_always_within_640x480` (proptest over format, size **and `sensor_type`** (`Rgb`, `Infrared`, `Unknown`), PE-SR8), `test_rsh_local_preview_unchanged_for_mjpeg` (GUI path still format 4) | RSH16 |
| `crates/daemon/tests/preview_remote_view_tests.rs` (append) | `test_rsh_remote_origin_receives_converted_mjpeg` (mock camera MJPEG, remote cgroup → format 0, 640x480), `test_rsh_local_origin_receives_mjpeg_unchanged` | RSH16 |
| `crates/remote/tests/http_tests.rs` (append) | `test_rsh_hardening_headers_on_every_head` (response, HEAD, SSE, camera stream: each of the four headers exactly once with the exact value; mandatory headers unchanged) | RSH10 |
| `crates/push-sender/tests/sandbox_tests.rs` (new) | `test_rsh_sandbox_accepts_the_unit_mount_table` (fixture captured in §1.1, with and without the resolver drop-in; PE-SR9: the fixture includes the `BindReadOnlyPaths=%h/.local/bin/soos-push-sender` entry as a child of the `/home` tmpfs), `test_rsh_sandbox_refuses_host_mount_table`, `test_rsh_sandbox_requires_ro_tmpfs_run`, `test_rsh_sandbox_requires_tmpfs_over_home` (prefix is component-wise: `/home2` never covers `/home/u`), `test_rsh_sandbox_refuses_listable_logs`, `test_rsh_sandbox_unescapes_mount_points`, `test_rsh_sandbox_option_match_is_exact` (`rootcontext=…`, `nosuid` never count as `ro`), `test_rsh_sandbox_home_anchor_unresolved_refused` (PE-SR1, replaces round-1 `test_rsh_sandbox_relative_or_empty_home_refused`: `home_from_exe` refuses a relative path, `/.local/bin/soos-push-sender` (remainder `/`), `/usr/bin/soos-push-sender`, `/home/u/.local/bin/soos-push-sender (deleted)`, `/home/u/x.local/bin/soos-push-sender`, a `..` component; accepts `/home/u/.local/bin/soos-push-sender` → `/home/u`; `evaluate_sandbox` returns `HomeUnresolved` before any other rule), `test_rsh_sandbox_home_outside_home_dir` (PE-SR1: `/var/home/u` accepted with a ro tmpfs at `/var/home`, refused `HomeVisible` with the tmpfs only at `/home`; `/srv/u` refused `HomeVisible`; `/root` accepted with a ro tmpfs at `/root`), `test_rsh_sandbox_refuses_mount_over_hidden_home` (PE-SR9: the accepted unit fixture, which carries the binary bind as a child of the `/home` tmpfs, plus one child entry of that tmpfs at `/home/u` (a `BindPaths=%h` drop-in), at `/home` itself, or at `/home/u/.config` → each `HomeVisible`; the same extra entry whose parent ID is **not** the tmpfs's mount ID (a host mount below the tmpfs) does not change the verdict; a child at `/home/other` (outside the home, not a prefix) does not change it either), `test_rsh_sandbox_binary_bind_is_the_only_allowed_child` (PE-SR9: the binary bind at exactly `<home>/.local/bin/soos-push-sender` is accepted; a child at `<home>/.local/bin` or `<home>/.local/bin/soos-push-sender2` is `HomeVisible`; a line with a non-numeric mount ID is skipped) | RSH11 |
| `crates/remote/tests/camera_server_tests.rs` (append) | `test_rsh_stop_after_first_frame_sends_no_part` (test source that calls `runtime.stop()` while producing the first frame; repeated 20 times: never a `200` head, never a JPEG part, audit `refused`, view not counted as shown). The second check (after the head, PE-SR5) has no deterministic socket seam (the head is far smaller than any socket buffer); it is pinned statically by `test_rsh_s8b_*` | RSH14 |
| `crates/remote/tests/push_server_tests.rs` (append) | `test_rsh_subscribe_with_replaces_endpoint_renews_in_place` (store full with the old endpoint → `200`, old gone, new present, 4 entries), `test_rsh_replaces_endpoint_absent_from_store_is_plain_upsert` (full store → `409`, nothing written), `test_rsh_replaces_endpoint_equal_is_plain_upsert`, `test_rsh_replaces_endpoint_invalid_is_plain_upsert` (PE-SR4: unparsable old endpoint → `200`, new subscription stored, nothing removed; on a full store without the old endpoint → `409`, nothing written; a non-string `replacesEndpoint` → `400`), `test_rsh_subscribe_without_replaces_endpoint_unchanged`, `test_rsh_renewal_audit_lines` | RSH15 |

### 11.3 Amendments to existing tests (exhaustive)

| Id | Test | Change | Justification |
|---|---|---|---|
| **CM-SR1** (owner-approved) | `tests/invariants/src/remote_push_contract.rs` test 45 `test_rmc_s35_sender_unit_is_sandboxed` | `SENDER_UNIT_LINES` `[&str; 37]` → `[&str; 38]`, one element inserted after `"TemporaryFileSystem=/run:ro"` (§2.2) | The exact sandbox list must name the new directive; additive only. |
| **AM-SR1** | `crates/daemon/tests/preview_remote_view_tests.rs` `test_rlc_local_peer_unaffected_by_remote_view` | **Setup only, additive**: one extra `write_session(&sessions, "4", &seat_session(uid));` next to the existing record `"3"` in the first scenario (its peer cgroup fixture is `session-4.scope`) | Under S-5 the peer's own session must be a local seat session; the fixture named a session id with no record. Assertions untouched. |
| **AM-SR2** | `crates/daemon/tests/preview_authorization_tests.rs` `test_preview_frame_allowed_for_configured_uid_serves_frames` | **Setup only, additive**: after writing record `"3"`, also write the same local-seat record under the test process's own session id when `/proc/self/cgroup` has one (`soos_daemon::session_policy::parse_session_id_from_cgroup`) | The test uses the real `/proc` cgroup; on a developer machine the test runs inside a TTY session scope whose id has no record in the temporary directory. Assertions untouched; CI (system unit, `Other`) unaffected. |

No other existing test changes. Checked compatible without change: `test_rlc_preview_requires_local_seat_session`
(record `"4"` matches `session-4.scope`), `test_rlc_default_peer_source_reads_proc_cgroup` (validator disabled),
`test_rlc_view_connection_leaves_room_for_auth` (silent third connection closed by the 200 ms window < 500 ms),
`test_same_uid_connection_over_cap_is_closed_without_blocking_others` and
`test_idle_polling_peer_cannot_exhaust_global_capacity` (non-`Auth` on the reserve: closed, no payload, permit
released before the stream closes), `test_limiter_*` (unchanged `try_acquire`), RAN-S2 (unchanged
`postSubscription`), RLC-S6/RLC-S10/RMC-S6/RMC-S36, and (PE-SR1) **RMC-S34** `test_rmc_s34_sender_policy_literals` (the sender's only environment read stays `XDG_RUNTIME_DIR`; the home anchor comes from `current_exe()`; `current_exe`, `read_dir` and `/proc/self/mountinfo` are not in its forbidden list), **RMC-S41** (no forbidden name: the probe names no key type and no `remote.*` file), **RMC-S42** (`install_logging();` stays the first statement of `main`) and the RMC forbid-attribute check (`#![forbid(unsafe_code)]` first line of `lib.rs`/`main.rs`, kept). If the tester finds another existing test that must change,
it stops and reports instead of editing it.

### 11.4 Test hooks

- `PeerConnectionLimiter::try_acquire_auth_reserve`, `PeerConnectionPermit::class` (pure).
- `classify_peer_placement`, `LocalSessionPolicy::authorize_preview_placement` with any `LogindSource`.
- `ConnectionDispatcher::with_preview_peer_source` (existing) + `with_session_validator` for placement scenarios.
- `remote_preview_image_for_frame` (pure, re-exported from `soos_daemon::preview`).
- `soos_push_sender::evaluate_sandbox` and `soos_push_sender::home_from_exe` (pure; the binary path is injected, no environment).
- `PushStore::replace_or_upsert` and the existing push-server harness.
- The existing camera `PreviewSource` test seam and `CameraRuntime::stop` for S-8b.

---

## 12. Verification matrix rows (component `remote-security-hardening`)

| Row | Criterion | Evidence |
|---|---|---|
| RSH1 | Sender unit makes `/var/log` and `/var/lib/systemd/coredump` inaccessible; docs say so | `test_rsh_s1_*`, test 45 (CM-SR1) |
| RSH2 | Host check of the directive (agent, transient unit, 2026-10-12): journal read 1 → 0 lines, DNS ok | §1.1 of this spec |
| RSH3 | Tailscale minimum 1.102.4 single-sourced, documented, printed by the installer; installer never runs `tailscale` | `test_rsh_s2_*`, RMC-S6 |
| RSH4 | Limiter: one auth-reserved permit per unprivileged UID within the global/unprivileged capacity | `test_rsh_limiter_*` |
| RSH5 | Dispatcher: reserve serves only one `Auth` whose whole first frame arrives within 200 ms (≤ 250 ms, not configurable); closes anything else, idle and partial-frame connections without payload; a 100 ms-delayed `Auth` is still answered | `test_rsh_reserve_*`, `test_rsh_auth_admitted_on_reserve_*`, `test_rsh_s3_*` |
| RSH6 | Companion retry path closes gracefully | `test_rsh_s3_*` (static) |
| RSH7 | Android lock-screen privacy documented | `test_rsh_s4_*` |
| RSH8 | Preview needs the peer's own local seat session, or its user manager without a remote session | `preview_placement_tests.rs` |
| RSH9 | Placement rule wired after the seat check, per request | `test_rsh_s5_*` |
| RSH10 | HSTS, COOP, CORP, Permissions-Policy on every response; CSP byte-identical | `test_rsh_hardening_headers_on_every_head`, `test_rsh_s6_*` |
| RSH11 | Sandbox evaluation pure rules; home anchor from the binary path (never the environment), homes outside `/home` handled; no mount re-exposes the home on top of its tmpfs except the binary bind (PE-SR9) | `sandbox_tests.rs` |
| RSH12 | Sender exits 78 before binding when the sandbox is not detected; it reads no environment variable other than `XDG_RUNTIME_DIR` | `test_rsh_s7_*`, RMC-S34 |
| RSH13 | Installer refuses a symlinked `remote.toml`, creates it with noclobber | `test_rsh_s8a_*` |
| RSH14 | A stop between the first frame and the head sends nothing; a stop seen after the head sends no part | `test_rsh_stop_after_first_frame_sends_no_part`, `test_rsh_s8b_*` |
| RSH15 | Renewal replaces the stale endpoint atomically; worker names it within the body limit | `test_rsh_subscribe_*`, `test_rsh_replaces_*`, `test_rsh_s8c_*` |
| RSH16 | Remote companion receives Grey/YUYV/RGB24 ≤ 640x480 for MJPEG, NV12, large and IR-stamped frames (PE-SR8); GUI path unchanged | `test_rsh_remote_preview_*`, `test_rsh_remote_origin_*`, `test_rsh_local_*`, `test_rsh_s8d_*` |
| RSH17 | ADR, ARCHITECTURE §13, Docs and walkthrough 194 updated | `test_rsh_docs_adr_and_walkthrough` |
| RSH18 | **Owner hardware check**: (1) after reinstall, `systemctl --user show soos-push-sender -p InaccessiblePaths` lists both paths and a test notification still arrives; (2) running `~/.local/bin/soos-push-sender` by hand exits 78; (3) with a live camera view **and** `soos-gui` open, the lock screen face unlock works; (4) `soos-gui` preview and the phone view still work; with an SSH session of the owner open, the phone view is refused (`camera_refused`) and works again after logout; (5) a 640x480 MJPEG or 640x512 IR camera shows a live view; (6) the Android notification icon and the iPhone home-screen icon still load (CORP); (7) `tailscale version` ≥ 1.102.4 | Manual (walkthrough 194); agents never install or restart units |

---

## 13. Residual risks

1. **S-1**: the sender still reads world-readable files outside the hidden paths and same-UID `/proc` entries;
   outbound network and abstract sockets unchanged (2026-10-06 ADR residuals). `PrivatePIDs=` is a follow-up.
2. **S-2**: the installer cannot verify the Tailscale version (it must not run `tailscale`); a downgrade below
   1.102.4 or a future release that stops stripping the headers is undetectable by the service.
3. **S-3**: a same-UID process can occupy the single reserve for ≤ 200 ms per connection unless it sends a complete
   frame (the window covers the whole first frame, PE-SR3); a complete non-`Auth` frame is closed at once; a
   complete `Auth` is served once under `connection_timeout` and the rate limiter. Repeated reconnects can still
   delay a concurrent lock-screen face request into the password fallback; same-UID processes can already do
   worse. The reserve also depends on the lock screen's PAM conversation: PAM shows "Looking for face" through it
   between `connect` and its write (PE-SR2), so a conversation call of ≥ 200 ms makes an honest `Auth` on the
   reserve miss the window (closed unanswered → `PAM_IGNORE` → password; fail-safe, and only when both regular
   connections are held). A
   `PasswordFailed` event of a user-level PAM caller is still dropped when both regular connections are held
   (evidence snapshot only).
4. **S-5**: placements outside session scopes and the user manager (root-configured services, containers) keep
   the UID-level seat rule; the cgroup is not a security boundary against the same UID with root help. The owner's
   own SSH session blocks the phone view and user-manager GUIs while it is open (intended, same as root `Auth`),
   and so does a process left running from one (`tmux`, `STATE=closing`) until it ends (PE-SR11; documented in §7
   with `loginctl list-sessions` / `loginctl terminate-session <id>`).
5. **S-6**: CORP `same-origin` could hide a notification icon if a browser fetches it cross-origin (owner check
   RSH18-6; the notification is still shown without it).
6. **S-7**: the self-check proves the three hiding directives, not the whole unit (address families, syscall
   filter, capabilities remain trusted to systemd). The home anchor is the binary's own path: a home outside every
   `ProtectHome=` path (or an image-based layout where the tmpfs mount point is not the resolved home prefix)
   makes the sender refuse to start (exit 78, documented in §2d), which disables Web Push but never weakens it.
   A drop-in that re-exposes the home on top of its tmpfs is refused (PE-SR9), but one that binds a host path
   under `/run` (e.g. `/run/dbus`) is not detected, and neither are binds outside the home and `/run`
   (documented in §2d *Limits*).
7. **S-8c**: a renewal whose two endpoints exceed 2048 bytes, or whose `pushsubscriptionchange` event carries a
   `null` `oldSubscription` (some browsers, PE-SR10), falls back to today's behaviour (stale slot until the next
   404/410 or `push remove`).
8. **S-8d**: the daemon decodes one MJPEG frame per remote request inline on a Tokio worker (bounded size, ≤ 10/s,
   preview rate limit), as its existing downscale and IR paths already do; decimation can halve resolution for
   tall sensors (640x512 → 320x256) instead of cropping. The decode is off the `Auth` path and holds no shared
   lock (PE-SR6).
9. **S-8b**: a stop raised while the first part is already being written cannot interrupt that write (one part,
   already authorised by both checks).

---

## 14. ADR draft (to add to `AI/DECISIONS.md`)

* **[2026-10-12] soos-remote Security Hardening (security review of 2026-10-12; no backlog id; branch
  `fix/remote-security-hardening`; spec `AI/architect_spec_remote_security_hardening.md`; walkthrough 194):**
  (1) **Sender sandbox**: `packaging/soos-push-sender.service` adds `InaccessiblePaths=-/var/log
  -/var/lib/systemd/coredump`; *corrects* the 2026-10-06 Web Push ADR, whose sandbox hid `$HOME` and `/run` but let a
  sender of a `wheel`/`adm`/`systemd-journal` owner read the whole system journal (reproduced on the owner host,
  systemd 262). The sender refuses to start (exit 78) unless `/run` is a read-only tmpfs, its home directory
  (derived from its own binary path `~/.local/bin/soos-push-sender`, never from the environment, so the sender
  still reads only `XDG_RUNTIME_DIR`) lies under a read-only tmpfs with nothing mounted back on top of it except
  its own binary, and `/var/log` is not listable. (2) **Tailscale minimum** 1.102.4 (the verified release) for Serve
  and Funnel, single-sourced in `TAILSCALE_MIN_VERIFIED_VERSION`; printed by the installer, which still never runs
  `tailscale`. (3) **Auth-reserved daemon connection**: one connection per unprivileged UID above
  `max_connections_per_uid`, admitted only for an `Auth` request whose whole first frame arrives within 200 ms
  (after PAM's "Looking for face" conversation message); anything else is closed unanswered; root admission and
  `pam_soos.so` unchanged. (4) **Preview placement** (*amends LC-3a of the 2026-10-07 live camera
  ADR*): an unprivileged preview peer must also run in a local seat session scope of its UID, or under its own user
  manager while the UID owns no remote session; other placements keep the UID-level rule. (5) **Response headers**:
  HSTS (1 year), COOP and CORP `same-origin`, a deny-all Permissions-Policy for sensors, camera, microphone,
  geolocation, payment and USB; CSP unchanged. (6) **Push renewal** names the replaced endpoint
  (`replacesEndpoint`), removed atomically with the upsert; an unparsable old endpoint is ignored (plain upsert).
  (7) **Camera**: no part after a stop observed before the first part (checked before and after the head); the daemon converts MJPEG, NV12 and frames beyond 640x480 into Grey/RGB24 within 640x480 for the
  `soos-remote.service` origin only, IR-stamped frames included (`soos-remote` stays decoder-free). (8) **Installer** refuses a symlinked
  `remote.toml` and writes it with noclobber. (9) Docs: Android lock-screen privacy. Test amendments: CM-SR1
  (test 45, one exact line added), AM-SR1 and AM-SR2 (additive session-record setup). Residuals as listed in the
  spec §13. Matrix RSH1–RSH18 (RSH18 owner hardware check).

---

## 15. Blast radius

| Crate / file | Change | Consumers |
|---|---|---|
| `packaging/soos-push-sender.service` | +1 directive, comment | installer (copies unchanged), test 45 |
| `crates/push-sender` (`lib.rs`, `main.rs`) | sandbox probe | binary only |
| `crates/remote` (`lib.rs`, `http.rs`, `server.rs`, `push.rs`, `camera_ipc.rs`, `assets/sw.js`) | S-2 const, S-6, S-8b, S-8c, S-3 retry | none outside the crate |
| `crates/daemon` (`limits.rs`, `dispatcher.rs`, `session_policy.rs`, `preview_image.rs`, `preview.rs`) | S-3, S-5, S-8d | `soos-gui` (preview client: unchanged behaviour for local peers), PAM (wire unchanged) |
| `scripts/install_remote.sh` | S-2 echo lines, S-8a | — |
| Docs | `REMOTE_COMPANION.md` §2b, §2d, §2f, §2h, §4, §7, §8, §9; `DAEMON.md`; `IPC_PROTOCOL.md`; `AI/ARCHITECTURE.md` §13; `AI/DECISIONS.md`; `AI/VERIFICATION_MATRIX.md` (traceability phase) | — |

No new crate, no new runtime dependency (daemon dev-dependency `jpeg-encoder` is already a workspace dependency and
in `Cargo.lock`), no protocol or config-key change, `#![forbid(unsafe_code)]` crates unchanged.

---

## 16. Round 2 — resolution of `AI/plan_evaluator_report.md` (round 1, REVISION_REQUIRED)

| Item | Severity | Resolution | Where |
|---|---|---|---|
| PE-SR1 | MAJOR | The home anchor is derived from `std::env::current_exe()` (the unit's fixed `%h/.local/bin/soos-push-sender`), never from the environment, never `std::env::home_dir()`; passwd/NSS lookup rejected (hidden `/run/systemd/userdb` would break homed/LDAP users). Unavailable anchor → `HomeUnresolved` → exit 78. Homes outside `/home` specified (accepted when a `ProtectHome=` tmpfs covers the resolved path, otherwise documented refusal). `HomeUnset` → `HomeUnresolved`; test renamed `test_rsh_sandbox_home_anchor_unresolved_refused`, plus `test_rsh_sandbox_home_outside_home_dir`. RMC-S34 (and RMC-S41/S42) listed as checked compatible without change; new static `test_rsh_s7_sender_reads_no_other_environment_variable`. | §8.1–§8.4, §11.1, §11.2, §11.3, §12 RSH11/RSH12, §13.6, §14 |
| PE-SR2 | MINOR | Wording corrected: PAM runs `on_connected` (a `PAM_TEXT_INFO` through the conversation) between `connect` and its write. Residual documented (fail-safe password fallback when the conversation takes ≥ 200 ms while both regular connections are held); docs to mention it. New test `test_rsh_reserve_auth_after_100ms_is_answered`. PAM path untouched. | §4.1, §4.3, §4.5, §11.2, §13.3, §14 |
| PE-SR3 | MINOR | The 200 ms window now bounds the read of the whole first frame (`timeout` around the factored `read_client_frame`), not readiness; `stream.readable()` dropped. Real bound stated. New test `test_rsh_reserve_partial_frame_closed_within_window` (1 byte; prefix + half body). | §4.3, §11.1, §11.2, §13.3 |
| PE-SR4 | MINOR | An unparsable `replacesEndpoint` is treated as absent (plain upsert); test renamed `test_rsh_replaces_endpoint_invalid_is_plain_upsert`. | §9.3, §11.2, §14 |
| PE-SR5 | MINOR (optional) | Second `stop_requested` check after the head and before `set_shown`/`write_part` (ends with `finish_view(Stopped)`, `audit_refused`, view not shown); pinned by static `test_rsh_s8b_*`; remaining part-write window listed. | §9.2, §11.1, §12 RSH14, §13.9 |
| PE-SR6 | observation | MJPEG decode stays in `handle_preview_request` for the remote origin only, outside the preview-limiter guard, never on the `Auth` path; static `test_rsh_s8d_remote_conversion_off_the_auth_path`. | §9.4, §11.1, §13.8 |
| PE-SR7 | observation | `AUTH_RESERVE_FIRST_REQUEST_TIMEOUT_MS` stays a non-configurable constant; RSH5 static test asserts `<= 250` and its absence from `config.rs`/`PeerLimitsConfig`. | §4.1, §11.1 |

Test-integrity status unchanged: the only amendments to existing tests remain CM-SR1 (owner-approved, additive),
AM-SR1 and AM-SR2 (setup only, additive). No assertion of RMC-S34 or any other existing test is relaxed.

---

## 17. Round 2 approval — non-blocking items of `AI/plan_evaluator_report.md` (round 2, APPROVED)

| Item | Severity | Resolution | Where |
|---|---|---|---|
| PE-SR8 | MINOR | The remote IR branch recurses into `remote_preview_image_for_frame`, never the GUI function, so an IR luma frame is bounded to 640x480 (640x512 YUYV `Infrared` → 320x256 Grey). `test_rsh_remote_preview_ir_stamp_is_grey` uses that exact frame; the proptest also varies `sensor_type`. | §9.4, §11.2, §12 RSH16, §14 |
| PE-SR9 | MINOR (recommended) | Applied as a pure mount-table rule (second half of rule 2): no child mount of the accepted home tmpfs may cover or lie inside the home except the exact binary bind, so `BindPaths=%h` (or a subdirectory bind) on top is refused (`HomeVisible`, exit 78). Directory-listing variant rejected (host-dependent, unverifiable without starting a unit). Binds under `/run` stay a documented residual. Two new tests, accepted fixture extended with the binary bind entry. | §8.2, §8.4, §11.2, §12 RSH11, §13.6, §14 |
| PE-SR10 | observation | A `null` `event.oldSubscription` falls back to today's request (stale slot until 404/410 or `push remove`); recorded in residual §13.7 and in the `Docs/REMOTE_COMPANION.md` `push remove` note. No design change. | §9.3, §13.7 |
| PE-SR11 | docs | The §7 `camera_refused` troubleshooting row names "an SSH session, or a process left running from one (`tmux`)", with `loginctl list-sessions` as the check and ending the process or `loginctl terminate-session <id>` as the fix; user-unit `soos-gui` mentioned. | §6.4, §13.4 |

Test-integrity status unchanged: the items add new tests and strengthen new-test definitions only. No existing
test changes beyond CM-SR1 (owner-approved, additive), AM-SR1 and AM-SR2 (setup only, additive).
