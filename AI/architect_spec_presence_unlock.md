# Architect Spec — GitHub #323: Continuous Face Presence Auto-Unlock of Locked Local Sessions

- **Branch**: `feat/presence-auto-unlock` (GitHub-only issue, not registered in `scripts/sync_issue.py`; commits carry `Closes #323`)
- **Base commit**: `f76a80b`
- **Revision**: 3 — revision 1 was evaluated `REVISION_REQUIRED` (round 1, findings F1–F7), revision 2 resolved them (changes marked *[R2: Fn]*, round 2 `APPROVED`). Revision 4 applies the Phase 3 auditor items A1–A6 (`AI/auditor_constraints_presence_unlock.md`, changes marked *[R4: An]*). Revision 3 applies the owner answers of 2026-10-02 to Q1–Q3 (changes marked *[R3: owner Q1]*: account guard for pam_faillock and account/password expiry) and was re-evaluated (rounds 3–4 in `AI/plan_evaluator_report.md`).
- **Owner decisions (2026-10-02, binding, not reopened here)**: enabled by default; global per-UID rate limit raised to 40 attempts / 60 s for every face request including `sudo`; the unchanged capture → detection → PAD consensus → match pipeline; unlock through systemd-logind.

---

## 0. Evidence Gathered Before Designing

| Question | Evidence (this host: systemd 262, GNOME/Wayland, GDM) | Consequence |
|---|---|---|
| Does `/run/systemd/sessions/<id>` expose the lock state? | `cat /run/systemd/sessions/*` shows `UID ACTIVE IS_DISPLAY STATE REMOTE USER TYPE CLASS SCOPE SEAT TTY SERVICE VTNR LEADER ...`, no lock key; `strings /usr/lib/systemd/systemd-logind` and `libsystemd-shared-262-1.so` contain `property_get_locked_hint` / `method_set_locked_hint` but **no** `LOCKED_HINT` serialization key | The lock state is available **only** through the D-Bus property `org.freedesktop.login1.Session.LockedHint`. A D-Bus client is mandatory. |
| Can a Rust D-Bus client read it here? | Scratch probe with `zbus` 5.19.0 (`default-features = false, features = ["tokio"]`): `LidClosed=false`, `GetSession("4")` → `/org/freedesktop/login1/session/_34`, `LockedHint=false Remote=false Class=user Active=true uid=1000 seat=seat0`, `ListSessions` → `[("1",1000,""),("4",1000,"seat0")]` | zbus reads every needed property. |
| Supply chain | Scratch copy of the workspace + `cargo add zbus@5 --no-default-features -F tokio -p soos-daemon`: 22 new crates (`zbus`, `zbus_macros`, `zbus_names`, `zvariant`, `zvariant_derive`, `zvariant_utils`, `zcheapstr`, `endi`, `enumflags2(+_derive)`, `event-listener(+-strategy)`, `async-broadcast`, `async-recursion`, `async-trait`, `futures-io`, `futures-lite`, `hex`, `ordered-stream`, `parking`, `serde_repr`, `uds_windows` (Windows only)); `cargo deny --locked check` → `advisories ok, bans ok, licenses ok, sources ok` (no new duplicate version, all MIT/Apache-2.0); `cargo check --locked -p soos-daemon` green on toolchain 1.98.1; zbus reuses the locked `rustix 1.1.4`, `tokio`, `serde`, `tracing` | Acceptable new dependency, no `deny.toml` skip needed. |
| Unit sandbox (`packaging/soos-daemon.service`) | `RestrictAddressFamilies=AF_UNIX` (the system bus is `AF_UNIX`), `PrivateNetwork=yes` (filesystem `AF_UNIX` sockets keep working, the unit comment already says so), `ProtectSystem=strict` (read-only `/run` does not block `connect(2)` on a socket inode; `/run/systemd/sessions` is already read today), `SystemCallFilter=@system-service` (includes `socket`/`connect`/`sendmsg`/`recvmsg`), `MemoryDenyWriteExecute=yes` (zbus has no JIT), `NoNewPrivileges=yes`, `CapabilityBoundingSet` without `CAP_SYS_ADMIN`; `ProtectKernelTunables=yes` keeps `/sys` and `/proc/acpi` **readable** (read-only) | **No unit change is needed** for D-Bus or sysfs reads. Spawning `loginctl` would also be possible (no `NoExecPaths`), but brings nothing once a D-Bus client exists. |
| logind authorization of `UnlockSession` from root | systemd ≥ 255 `bus_verify_polkit_async_full` → `sd_bus_query_sender_privilege(call, -1)` grants a sender whose euid equals logind's (0), so no `CAP_SYS_ADMIN` and no polkit round trip are needed; older systemd checks `CAP_SYS_ADMIN` and falls back to polkit, which always authorizes uid 0 | Works without changing `CapabilityBoundingSet`. Any refusal is an error → no unlock (fail closed); confirmed on hardware by PAU20. |
| Display / lid signals | `/sys/class/drm/card1-eDP-1/dpms` = `On`, `card1-DP-1` / `card1-HDMI-A-1` = `Off` (disconnected); `/proc/acpi/button/lid/*/state` = `open`; logind `Manager.LidClosed` = `false` | Lid: use logind `Manager.LidClosed` (same D-Bus client, portable, no procfs ACPI dependency). Screen: DRM connector `status` + `dpms` in sysfs. |
| *[R3: owner Q1]* pam_faillock tally | Linux-PAM 1.7.1 sources (`modules/pam_faillock/faillock.h`, `faillock.c`, `faillock_config.c`, `pam_faillock.c`; host has pam 1.7.3): file `<dir>/<user>`, default dir `/run/faillock`, records `struct tally { char source[52]; uint16_t reserved; uint16_t status; uint64_t time; }` = 64 bytes, native endianness, `TALLY_STATUS_VALID = 0x1`, `time` = `time(NULL)` (wall clock, seconds); `read_tally` reads whole records only and keeps at most `MAX_RECORDS = 1024`; `pam_faillock` takes `flock(LOCK_EX)` and `update_tally` truncates then rewrites. Defaults `deny = 3`, `fail_interval = 900`, `unlock_time = 600`, `root_unlock_time = unlock_time`; `fail_interval`/`unlock_time`/`root_unlock_time` ≤ `MAX_TIME_INTERVAL = 604800`, `"never"` = 0. `check_tally`: `latest = max(valid.time)`; `failures = #valid with latest − time < fail_interval`; locked iff `deny != 0 && failures >= deny` and not (`unlock_time != 0 && latest + unlock_time < now`); `is_admin` (UID 0, or member of `admin_group`) is never locked without `even_deny_root` and uses `root_unlock_time`. Options come from `/etc/security/faillock.conf` (fallback vendor `/usr/etc/security/faillock.conf`), or `conf=<file>`, and are overridden by arguments on the `pam_faillock.so` line. Host: `/etc/security/faillock.conf` all comments (defaults), `/run/faillock` `0755 root:root`, `/run/faillock/willi363` `0660 willi363:root`, `/etc/pam.d/system-auth` lines `pam_faillock.so preauth` / `authfail` / `authsucc` (no policy arguments) | The daemon can evaluate the lock exactly like `pam_faillock` from root-owned, local files; options on PAM lines and group membership are the only parts it cannot see (handled conservatively, §2.6 `presence/account.rs`). |
| *[R3: owner Q1]* Account / password expiry | `/etc/shadow` `0600 root:root` (host); fields `name:pw:lastchg:min:max:warn:inactive:expire:reserved` (shadow(5)); `pam_unix` account phase refuses an expired account (`expire`), an inactive account (password expired more than `inactive` days), and requires a new password when `lastchg = 0` or the password is past `max` | *[R4: A1]* Readable by the root daemon through the owner bit where the file is `0600`/`0640` (Arch, Debian/Ubuntu), but Fedora/RHEL ship `/etc/shadow` as `0000 root:root`, readable by root **only** through `CAP_DAC_OVERRIDE`; NSS-only users (LDAP/SSSD, systemd-homed) have no `/etc/shadow` line ⇒ "cannot be determined" ⇒ no presence unlock (owner rule). |
| *[R3: owner Q1]* Sandbox for the account guard | `ProtectSystem=strict` leaves `/etc` and `/run` readable; no `InaccessiblePaths=`/`ReadOnlyPaths=` in the unit; `ProtectHome=yes` is irrelevant; the daemon runs `User=root Group=soos`, so reading `/run/faillock/<user>` (`0660 <user>:root`) needs `CAP_DAC_OVERRIDE` or `CAP_DAC_READ_SEARCH` — `CAP_DAC_OVERRIDE` is in `CapabilityBoundingSet` (effective for UID 0); *[R4: A1]* `/etc/shadow` needs `CAP_DAC_OVERRIDE` where it is `0000 root:root` (Fedora/RHEL) and only the owner bit where it is `0600`/`0640` | **No unit change.** `CAP_DAC_OVERRIDE` becomes load-bearing for both the tally and (Fedora/RHEL) `/etc/shadow` (documented, guarded by invariant C9); without it either read fails ⇒ `Undeterminable` ⇒ no unlock (fail closed). The unit's comment on `CAP_DAC_OVERRIDE` ("administrator-modified state files") is updated to name these two uses; no directive changes. *[R4: A1]* The workspace `nix` already enables the `fs` feature (`Flock`). |
| Rate-limit default | `crates/policy/src/rate_limit.rs` `RateLimitConfig::DEFAULT_MAX_ATTEMPTS = 5`, window 60 s; no test asserts the value 5 (`grep -rn DEFAULT_MAX_ATTEMPTS crates tests`), `Docs/DAEMON.md` §1.4 table says `5` | One-line constant change in the policy crate is the single source; docs follow. |
| Consensus loop | Inline in `ConnectionDispatcher::handle_request` Step 8d-1…8f (`crates/daemon/src/dispatcher.rs` ~ lines 896–1195), `PadAggregator::with_defaults(*pipe.policy.read().await.thresholds())` | Must be extracted into one shared function so presence can never diverge (§2.4). |
| Inference gate | `InferenceGate` (`crates/daemon/src/inference.rs`): Tokio FIFO `Semaphore`, `MAX_CONCURRENT_INFERENCES = 1`, `acquire_within(max_wait)`, EMA estimator; owned by value in `ConnectionDispatcher` (`inference: InferenceGate`) | Needs sharing (`Clone` over the same `Arc`s) and a non-waiting background acquisition that yields to PAM (§2.3). |

---

## 1. Scope & Blast Radius

### 1.1 Crates and modules

| Crate | Item | Change |
|---|---|---|
| `soos-policy` | `crates/policy/src/rate_limit.rs` | `DEFAULT_MAX_ATTEMPTS` 5 → **40**; new `RateLimiter::check_and_record_with_reserve`. |
| `soos-policy` | `crates/policy/src/decision.rs` | New `AuthorizationEngine::record_attempt_with_reserve`. |
| `soos-daemon` | `crates/daemon/src/consensus.rs` (**new**) | Shared camera-wake + multi-frame PAD consensus, extracted verbatim from dispatcher Step 8d-1…8e. |
| `soos-daemon` | `crates/daemon/src/dispatcher.rs` | Step 8d-1…8e replaced by calls into `consensus`; registers interactive inference demand for every `Auth` request in Step 8; `capture_spoof_evidence` **stays** in the dispatcher (invariant `test_architecture_doc_matches_daemon_code_claims` greps for it). |
| `soos-daemon` | `crates/daemon/src/inference.rs` | `InferenceGate: Clone` (shared state), `InferencePriority`, `InteractiveDemandGuard`, `try_acquire_background`. |
| `soos-daemon` | `crates/daemon/src/pipeline.rs` | `#[derive(Clone)]` on `PipelineComponents` (all fields are `Arc`); manual `Debug` unchanged. |
| `soos-daemon` | `crates/daemon/src/session_policy.rs` | `is_valid_session_id` → `pub(crate)`; `non_empty` → `pub(crate)` (reused by the D-Bus mapping). No behaviour change. |
| `soos-daemon` | `crates/daemon/src/presence/` (**new**) | `mod.rs`, `config.rs`, `logind.rs` (trait + `ZbusLogind`), `display.rs`, `switch.rs`, `tracker.rs`, `worker.rs`. |
| `soos-daemon` | `crates/daemon/src/presence/account.rs` (**new**) *[R3: owner Q1]* | `AccountGuard` trait, `SystemAccountGuard`, pure parsers for `faillock.conf`, tally records, PAM-stack option scan and `/etc/shadow`. |
| `soos-daemon` | `crates/daemon/src/config.rs` | `[presence]` table (`PresenceConfigFile`), `DaemonConfig::presence`, validation, warnings. |
| `soos-daemon` | `crates/daemon/src/lib.rs` | `pub mod consensus; pub mod presence;` + re-exports. |
| `soos-daemon` | `crates/daemon/src/main.rs` | Spawns the presence worker after `READY=1`, stops it at shutdown. |
| `soos-daemon` | `crates/daemon/Cargo.toml` | `zbus = { workspace = true }`. |
| workspace | `Cargo.toml` | `[workspace.dependencies] zbus = { version = "5", default-features = false, features = ["tokio"] }`; `Cargo.lock` gains the 22 crates of §0. |
| `soos-invariants` | `tests/invariants/src/presence_unlock_contract.rs` (**new**) + `lib.rs` `mod` line | Static contracts (§8). |
| `soos-invariants` | `tests/invariants/src/daemon_docs_contract.rs` (**existing test, contract migration**) *[R2: F1]* | `CONFIG_FILE_TABLES: [(&str, &str); 9]` becomes `[(&str, &str); 10]` with the added entry `("PresenceConfigFile", "presence")`. Justification: `test_daemon_doc_documents_every_daemon_toml_key` panics on any unmapped `*ConfigFile` struct and its own panic message mandates this mapping; no assertion, loop or message is changed, and the test becomes **stricter** (it now also requires every `[presence]` key in `Docs/DAEMON.md`). This is the only existing test file the plan edits. |
| docs | `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, `AI/MOCK_STRATEGY.md`, `Docs/DAEMON.md`, `Docs/POLICY_CRATE.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md`, `Docs/PACKAGING_AND_PROVISIONING.md` §8.1, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` §4, `tests/physical/screensaver_test.md` | §9. |

### 1.2 Not touched (explicitly)

- `crates/pam` (no D-Bus, no Tokio, no new code path; `pam_soos.so` behaviour unchanged except that the shared rate-limit default is now 40).
- `crates/protocol` (no new request kind, no `StatusResponse` change; presence is daemon-internal, nothing crosses the socket).
- `packaging/soos-daemon.service` (no directive change, §0).
- `scripts/install.sh`, packaging scriptlets (no new binary, no packaged `daemon.toml`; the feature is on through the built-in default).
- `capture_spoof_evidence`, `EvidenceStore` (presence never writes evidence, §2.1 D7).

### 1.3 Downstream consumers of changed public items

| Item | Consumers (`grep -rn`) | Update |
|---|---|---|
| `RateLimitConfig::DEFAULT_MAX_ATTEMPTS` | `crates/policy/src/rate_limit.rs` only (no test asserts 5); `RateLimitConfig::default()` in `crates/daemon/src/config.rs:264`, `crates/daemon/tests/pipeline_init_tests.rs:89`, `crates/policy/tests/rate_limit_tests.rs:169` (capacity only) | none in code; docs `Docs/DAEMON.md` §1.4, `Docs/POLICY_CRATE.md`. |
| `PipelineComponents` (gains `Clone`) | daemon `main.rs`, `dispatcher.rs`, every daemon integration test building it by struct literal | additive derive, struct literals keep compiling. |
| `InferenceGate` (gains `Clone` + methods) | `dispatcher.rs`, `pipeline::warmed_inference_gate`, `crates/daemon/tests/*inference*` | additive. |
| `DaemonConfig` (gains `presence` field) | every `DaemonConfig { .. }` struct literal — `grep -rn "DaemonConfig {" crates tests` must be checked by the developer; tests use `DaemonConfig::default()` / `from_toml_str` | add the field where a literal exists (setup-only). |

---

## 2. Design

### 2.1 Decisions (resolving the six design questions)

| # | Decision | Rejected alternative and why |
|---|---|---|
| D1 Lock state source | D-Bus: `Manager.ListSessions` (`a(susso)`), then `org.freedesktop.DBus.Properties.GetAll("org.freedesktop.login1.Session")` on each seat-attached session; lock state = `LockedHint` | Session files: they do not carry the lock state (§0). Mixed sources (file for binding, D-Bus for lock) would read two snapshots; one D-Bus snapshot per session is coherent. |
| D2 Unlock mechanism | D-Bus `Manager.UnlockSession(s id)` via `zbus` 5 (`default-features = false`, `tokio`), the exact call `loginctl unlock-session <id>` makes | Spawning `/usr/bin/loginctl`: a fork/exec of a root child every unlock and text parsing, while a D-Bus client is mandatory anyway (D1). `dbus` crate: links C `libdbus-1` into the root daemon. Hand-written D-Bus wire client: security-critical marshalling code to maintain. |
| D3 Bus address | Pinned `SYSTEM_BUS_ADDRESS = "unix:path=/run/dbus/system_bus_socket"` through `zbus::connection::Builder::address`, never `Connection::system()` (which honours `DBUS_SYSTEM_BUS_ADDRESS`) | Environment-derived address. |
| D4 Binding predicate | The **existing** `SessionRecord::check_local_seat_session_of(uid)` (UID match, active, explicit `REMOTE=0`, non-empty seat, `CLASS=user`), applied to a `SessionRecord` built from the D-Bus properties | A second, presence-specific predicate (would drift). The user-manager rule (c) "target owns no remote session" is **not** applied: logind already lets every process of the session owner unlock that session (polkit `lock-sessions` passes `good_user = owner`), so a remote session of the owner gains nothing from presence; the only new threat is an impostor at the camera, covered by PAD + match. |
| D5 Candidate selection | Exactly one eligible session (bound, locked, past grace, enrolled with a `Current` template, not marked `locker_ignored`) — zero or ≥ 2 → no scan | Scanning several templates per frame (would turn one camera into a multi-identity matcher and leave ambiguous "which seat is the face at"). |
| D6 PAM priority | Presence never waits for the inference slot (`try_acquire_background`), never starts or continues a scan while any `Auth` request holds an `InteractiveDemandGuard`, and is preempted between captures | A priority semaphore (more code, same effect). |
| D7 Evidence | Presence spoof vetoes are logged (warn, transition-limited) but **not** sealed as evidence | Reusing `capture_spoof_evidence`: continuous scans would spend the per-UID daily cap on false PAD rejections of the legitimate user. Reversible later (follow-up, §10 Q2). |
| D8 Lid / screen | Lid from logind `Manager.LidClosed`; screen from sysfs DRM connectors; `Unknown` never gates (owner: "when detectable") | `/proc/acpi/button/lid` (deprecated, absent on many kernels). |
| D9 Kill switch | `/etc/soos/disabled` (existing global PAM flag, now also stops presence) or `/etc/soos/presence.disable` (presence only), re-checked every tick | A new directory or a daemon signal. `gdm.disable` does **not** stop presence (presence is not a PAM service; documented). |
| D10 Rate limit | Every scan records one attempt for the session owner's UID in the **shared** per-UID limiter (owner decision), through a reserve-keeping call that never consumes the last `PRESENCE_RESERVED_ATTEMPTS` (= 5) attempts of the window, so PAM always keeps at least the pre-#323 budget | Separate presence quota (rejected by the owner). The reserve is not a separate quota: it only stops presence earlier. |

### 2.2 Module layout

```text
crates/daemon/src/
├── consensus.rs            # shared camera wake + PAD consensus (extracted from dispatcher)
└── presence/
    ├── mod.rs              # re-exports, constants, PresenceTick / SkipReason / ScanOutcome
    ├── config.rs           # PresenceConfig, bounds, validation
    ├── logind.rs           # PresenceLogind trait, SessionId, LogindSessionState,
    │                       # PresenceLogindError, ZbusLogind, session_state_from_properties
    ├── display.rs          # DisplayProbe trait, DisplayState, SysfsDisplayProbe, parse helpers
    ├── switch.rs           # PresenceSwitch (kill-switch flag files)
    ├── tracker.rs          # LockTracker (pure, clockless) + candidate selection
    ├── account.rs          # [R3: owner Q1] AccountGuard: pam_faillock + shadow expiry
    └── worker.rs           # PresenceWorker<L, D> (async loop, one tick = one decision)
```

All of `presence::config`, `presence::tracker`, `presence::display` parse helpers and the candidate selection are **pure** (no I/O, monotonic `u64` ns passed in), mirroring the `soos-policy` style; they stay in the daemon because they depend on daemon-only types (`SessionRecord`, `SessionDenial`).

*[R2: F3]* Unsafe policy: `crates/daemon/src/lib.rs` keeps `#![deny(clippy::undocumented_unsafe_blocks)]` (needed by `mlock.rs`). `crates/daemon/src/presence/mod.rs` and `crates/daemon/src/consensus.rs` each start with the inner attribute `#![forbid(unsafe_code)]` (a lint attribute is valid at module level and covers the whole `presence` module tree), so no `unsafe` can ever enter the new code; invariant C2 (§8) checks both the attribute and the absence of the `unsafe` keyword.

### 2.3 Inference gate changes (`crates/daemon/src/inference.rs`)

```rust
/// Who asks for the single inference slot (GitHub #323).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferencePriority {
    /// A PAM `Auth` request: may wait up to its remaining budget (`acquire_within`).
    Interactive,
    /// The presence scanner: never waits, never queues, yields to any interactive demand.
    Background,
}

#[derive(Debug, Clone)]          // NEW: clones share the semaphore, estimator and demand counter
pub struct InferenceGate {
    permits: Arc<Semaphore>,
    max_concurrent: usize,
    estimator: Arc<InferenceEstimator>,
    interactive_demand: Arc<AtomicUsize>,   // NEW
}

impl InferenceGate {
    /// Registers one interactive (PAM `Auth`) request until the guard is dropped.
    #[must_use]
    pub fn register_interactive(&self) -> InteractiveDemandGuard;
    /// Number of live `InteractiveDemandGuard`s.
    #[must_use]
    pub fn interactive_demand(&self) -> usize;
    /// Background acquisition: `None` when `interactive_demand() > 0` or no slot is free
    /// right now. Never waits. The demand is re-checked after a successful `try_acquire_owned`;
    /// if a guard appeared in between, the permit is dropped and `None` returned.
    #[must_use]
    pub fn try_acquire_background(&self) -> Option<OwnedSemaphorePermit>;
}

/// RAII interactive-demand registration; decrements (saturating) on drop. `!Clone`.
#[derive(Debug)]
pub struct InteractiveDemandGuard { counter: Arc<AtomicUsize> }
```

Bounds: the counter is incremented once per `Auth` request admitted to Step 8, so it is bounded by `max_concurrent_connections`; `fetch_add` / saturating `fetch_update` decrement, `Ordering::SeqCst` (correctness over speed, two atomics per request). `Default` and `new` initialise the counter at 0. Existing `acquire_within`, `run`, `warm_up` unchanged.

**Dispatcher wiring**: at the top of Step 8 (`if let Some(ref pipe) = self.pipeline {`), before the template lookup: `let _interactive = self.inference.register_interactive();` held until the function returns (all early returns included).

**Worst-case PAM delay added by presence**: one in-flight presence inference job, which cannot be preempted once started (it holds the permit inside `spawn_blocking`). Bound: the measured job duration, tracked by the same EMA (`InferenceGate::estimate()`, clamped to `MAX_INFERENCE_ESTIMATE_MS` = 1000 ms); typical dev-host cost SCRFD + PAD + SFace well under 100 ms (SFace alone p95 11.0 ms, ARCHITECTURE §7). The existing `Auth` admission already handles a busy slot (`acquire_within(remaining − estimate)`, then finalize), so the PAM deadline is never exceeded; the worst effect is one fewer capture inside the 900 ms budget.

### 2.4 Shared consensus (`crates/daemon/src/consensus.rs`)

The code of dispatcher Step 8d-1 (camera `notify_activity`), 8d-2 (bounded camera wake wait) and 8e (consensus loop, inference admission, `process_frame` result mapping, `PadAggregator` decision) moves **verbatim** into this module; the dispatcher keeps Steps 1–8c, 8f rendering, 8g evidence and the lock-screen `notify_activity` tail. Behaviour for `Auth` must be byte-for-byte identical (same logs, same verdicts/reasons, same early returns); the whole existing daemon test suite is the regression contract and **no existing test may be edited** (the only existing test file the plan touches is the invariant table migration of §1.1, F1).

```rust
/// Upper bound of the camera wake wait for one decision (was the literal 1200 / 1000 ms in
/// dispatcher Step 8d-2; single source now).
pub const MAX_CAMERA_WAKE_WAIT_MS: u64 = 1200;
/// Wake wait used when the client sent no usable deadline (was the literal 1000 ms).
pub const DEFAULT_CAMERA_WAKE_WAIT_MS: u64 = 1000;
/// Poll interval of the camera wake wait (was the literal 15 ms).
pub const CAMERA_WAKE_POLL_MS: u64 = 15;

/// Everything one consensus run reads; borrowed, nothing owned.
pub struct ConsensusContext<'a> {
    pub camera: &'a Arc<dyn CameraManager>,
    pub vision: &'a Arc<VisionPipeline>,
    pub inference: &'a InferenceGate,
    pub thresholds: ThresholdConfig,          // copied from the shared AuthorizationEngine
    pub clock: fn() -> Result<u64, DaemonError>,
    pub priority: InferencePriority,
    pub uid: u32,                             // for log lines only (already logged today)
}

/// Waits (bounded) for the camera after `notify_activity`; returns `camera.is_ready()`.
pub async fn wake_camera(camera: &Arc<dyn CameraManager>, max_wake: Duration) -> bool;

/// Outcome of one consensus run.
pub enum ConsensusRun {
    /// The loop ended normally (Allow, SpoofVetoed, or Pending at the deadline).
    Decided {
        decision: ConsensusDecision,
        frames_evaluated: usize,
        consecutive_passing: usize,
        last_capture_stale: bool,
        /// First spoof-classified capture (only used by the `Auth` evidence path).
        spoof_capture: Option<Arc<Frame>>,
    },
    /// A failure that the dispatcher answers with this verdict today (clock error,
    /// inference job panic/cancel, `VisionError::Inference`, other `VisionError`).
    Aborted { verdict: Verdict, reason: ReasonClass, completion_error: Option<DaemonError> },
    /// Background priority only: an interactive request appeared; nothing was decided.
    Preempted,
}

/// Runs the multi-frame PAD consensus against `template` until `deadline`.
/// `Interactive`: identical to the former Step 8e (acquire_within, never `Preempted`).
/// `Background`: before every inference it returns `Preempted` if
/// `inference.interactive_demand() > 0`, and acquires only through `try_acquire_background`
/// (a busy slot ⇒ poll again at `FRAME_POLL_INTERVAL_MS`, never wait).
pub async fn run_face_consensus(
    ctx: &ConsensusContext<'_>,
    template: &[f32],
    deadline: RequestDeadline,
) -> ConsensusRun;
```

Invariants of the extraction:
- `PadAggregator::with_defaults(ctx.thresholds)` is constructed **only** in `consensus.rs` (invariant test, §8 C4); `ctx.thresholds` comes from `pipe.policy.read().await.thresholds()` in both callers — no threshold literal anywhere in `presence/`.
- `is_frame_fresh` / `MAX_FRAME_AGE_NS` / `FRAME_POLL_INTERVAL_MS` / decay-on-zero-frames rule unchanged.
- `ConsensusRun::Decided { decision: Allow, .. }` is the **only** value from which presence may proceed to an unlock.

### 2.5 Policy additions (`soos-policy`, zero I/O)

```rust
impl RateLimitConfig {
    /// Default maximum attempts per window: 40 (GitHub #323, owner decision 2026-10-02:
    /// ~30 presence scans per minute plus PAM headroom; applies to every face request).
    pub const DEFAULT_MAX_ATTEMPTS: u32 = 40;
}

impl RateLimiter {
    /// Records an attempt only while more than `reserve` attempts remain in the window
    /// (evaluated and recorded atomically in one `&mut self` call). Returns
    /// `RateLimitExceeded` without recording when `remaining_attempts(uid, now) <= reserve`,
    /// and in every case `check_and_record` would refuse (`max_attempts == 0`, capacity 0).
    /// `reserve == 0` behaves exactly like `check_and_record`.
    pub fn check_and_record_with_reserve(
        &mut self, uid: u32, now_monotonic_ns: u64, reserve: u32,
    ) -> Result<(), PolicyError>;
}

impl AuthorizationEngine {
    /// `record_attempt` keeping `reserve` attempts for other callers; `Ok(())` without a
    /// limiter (same as `record_attempt`).
    pub fn record_attempt_with_reserve(
        &mut self, uid: u32, now_monotonic_ns: u64, reserve: u32,
    ) -> Result<(), PolicyError>;
}
```

Edge semantics: `reserve >= max_attempts` ⇒ always refused (presence never scans); expired timestamps are pruned before evaluating `remaining`, exactly as `check_and_record`; LRU eviction unchanged.

### 2.6 Presence types

#### `presence/mod.rs` constants (single source; no other copy)

| Constant | Value | Meaning / bound behaviour |
|---|---|---|
| `LOCK_POLL_INTERVAL_MS` | 1000 | Tick period of the worker (logind snapshot). Grace is therefore effective in `[lock_grace, lock_grace + 1 s]`. |
| `PRESENCE_RESERVED_ATTEMPTS` | 5 | Attempts of the shared window presence never consumes (D10). |
| `DBUS_CALL_TIMEOUT_MS` | 500 | `tokio::time::timeout` around every D-Bus call (and `Builder::method_timeout`); expiry ⇒ `PresenceLogindError::Timeout`. |
| `DBUS_CONNECT_TIMEOUT_MS` | 1000 | Bound of one connection attempt. |
| `DBUS_RECONNECT_BACKOFF_MIN_MS` / `_MAX_MS` | 1000 / 30000 | Exponential backoff (×2, saturating, clamped) after a connection or call failure; reset on the first successful snapshot. |
| `MAX_PRESENCE_SEAT_SESSIONS` | 16 | Seat-attached sessions queried per tick; more ⇒ `SkipReason::TooManySessions` (no scan). |
| `MAX_TRACKED_LOCKED_SESSIONS` | 16 | `LockTracker` capacity; overflow ⇒ tracker cleared, `SkipReason::TrackerOverflow` this tick (fail closed: every grace restarts). |
| `MAX_ALLOW_TO_UNLOCK_MS` | 1000 | Maximum time between the consensus `Allow` and the `UnlockSession` call; exceeded ⇒ the `Allow` is discarded (single-use, like `RESPONSE_VALIDITY_NS` for PAM). *[R3: F11]* The re-check (`session_state` ≤ 500 ms + `AccountGuard::check` ≤ 500 ms) fits only when the calls are not both at their timeout; a slow host therefore ends in `AllowExpired` (fail closed) and the next scan retries — deliberately not widened. |
| `UNLOCK_CONFIRM_TIMEOUT_MS` | 5000 | After a successful `UnlockSession`, a session still `LockedHint=true` this long is marked `locker_ignored` (no further scans until observed unlocked; one `warn`). |
| `MAX_LOGIND_ERROR_LEN` | 256 | Bytes of a D-Bus error name/message kept in `PresenceLogindError::Call` (truncated on a char boundary). |
| `MAX_DRM_CONNECTORS` | 64 | Directory entries of `/sys/class/drm` examined; more ⇒ `DisplayState::Unknown`. |
| `MAX_SYSFS_ATTR_BYTES` | 64 | Bytes read from one `status` / `dpms` attribute; longer ⇒ that connector ignored. |
| `ACCOUNT_CHECK_TIMEOUT_MS` *[R3]* | 500 | Bound of one `AccountGuard::check` on the blocking pool; expiry ⇒ `Undeterminable`. |
| `MAX_USER_NAME_LEN` *[R3]* | 256 | `UserName::parse` bound. |
| `DEFAULT_FAILLOCK_CONF` / `VENDOR_FAILLOCK_CONF` *[R3]* | `/etc/security/faillock.conf` / `/usr/etc/security/faillock.conf` | Lookup order of `faillock_config.c`. |
| `DEFAULT_FAILLOCK_DIR` *[R3]* | `/run/faillock` | `FAILLOCK_DEFAULT_TALLYDIR`. |
| `DEFAULT_FAILLOCK_DENY` / `_FAIL_INTERVAL_S` / `_UNLOCK_TIME_S` *[R3]* | 3 / 900 / 600 | `pam_faillock.c` defaults; `root_unlock_time` defaults to `unlock_time`. |
| `MAX_FAILLOCK_TIME_INTERVAL` *[R3]* | 604800 | `MAX_TIME_INTERVAL` (7 days); larger ⇒ `Undeterminable`. |
| `TALLY_RECORD_BYTES` / `TALLY_STATUS_VALID` *[R3]* | 64 / `0x1` | `struct tally`; `status` at byte 54, `time` at byte 56. |
| `MAX_TALLY_BYTES` *[R3]* | 69 632 (1088 records) | `MAX_RECORDS` 1024 + one chunk; larger ⇒ `Undeterminable`. |
| `MAX_FAILLOCK_CONF_BYTES` / `MAX_PAM_FILE_BYTES` *[R3]* | 65 536 / 65 536 | Larger ⇒ `Undeterminable`. |
| `DEFAULT_PAM_DIRS` / `MAX_PAM_DIR_ENTRIES` *[R3]* | `/etc/pam.d`, `/usr/lib/pam.d`, `/usr/etc/pam.d` / 512 | Option scan (all files, a superset of any stack). |
| `DEFAULT_SHADOW_PATH` / `MAX_SHADOW_BYTES` / `MAX_SHADOW_LINES` *[R3]* | `/etc/shadow` / 4 MiB / 65 536 | Larger ⇒ `Undeterminable`. |
| `SYSTEM_BUS_ADDRESS` | `"unix:path=/run/dbus/system_bus_socket"` | D3. |
| `DEFAULT_DRM_SYSFS_DIR` | `"/sys/class/drm"` | D8. |
| `DEFAULT_KILL_SWITCH_DIR` | `"/etc/soos"` | Must equal `soos_pam::config::DEFAULT_FLAG_DIR` (textual invariant, §8 C6; the daemon cannot depend on `soos-pam`). |
| `GLOBAL_DISABLE_FLAG` / `PRESENCE_DISABLE_FLAG` | `"disabled"` / `"presence.disable"` | D9. |

Scan budget: `RequestDeadline::compute(now_ns, 0, Instant::now(), Duration::from_millis(DECISION_BUDGET_MS + RESPONSE_WRITE_MARGIN_MS))` ⇒ exactly `DECISION_BUDGET_MS` (900 ms) — no new budget constant. Camera wake for a scan: `wake_camera(camera, Duration::from_millis(MAX_CAMERA_WAKE_WAIT_MS))`.

#### `presence/config.rs`

```rust
pub const DEFAULT_PRESENCE_ENABLED: bool = true;          // owner decision
pub const DEFAULT_SCAN_INTERVAL_MS: u64 = 2000;
pub const MIN_SCAN_INTERVAL_MS: u64 = 1000;
pub const MAX_SCAN_INTERVAL_MS: u64 = 60_000;
pub const DEFAULT_LOCK_GRACE_MS: u64 = 3000;
pub const MIN_LOCK_GRACE_MS: u64 = 1000;
pub const MAX_LOCK_GRACE_MS: u64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceConfig {
    pub enabled: bool,
    /// Minimum time between the starts of two scans of the same session.
    pub scan_interval: Duration,
    /// Time a session must have been observed locked before any scan or unlock.
    pub lock_grace: Duration,
}
impl Default for PresenceConfig { /* the three defaults */ }
impl PresenceConfig {
    /// # Errors
    /// `DaemonError::Config` naming `[presence] scan_interval_ms` / `lock_grace_ms` when
    /// outside its `[MIN, MAX]` range (0 included: 0 is never "immediately" nor "disabled";
    /// disabling is `enabled = false`).
    pub fn validate(&self) -> Result<(), DaemonError>;
}
```

TOML (`crates/daemon/src/config.rs`):

```rust
#[derive(Debug, Deserialize)]
struct PresenceConfigFile {
    enabled: Option<bool>,
    scan_interval_ms: Option<u64>,
    lock_grace_ms: Option<u64>,
}
// DaemonConfigFile gains: #[serde(default)] presence: Option<PresenceConfigFile>
// DaemonConfig gains:     pub presence: PresenceConfig
```

Sentinels: missing table / missing key ⇒ default; wrong type ⇒ TOML parse error ⇒ startup error (same rule as every other key); `0` or out of range ⇒ startup error naming the key. `DaemonConfig::validate` calls `self.presence.validate()`.

Startup warnings (pushed to `DaemonConfig::warnings`, never errors, so an upgraded host that pinned `max_attempts = 5` still starts):
- `presence.enabled && rate_limit.max_attempts <= PRESENCE_RESERVED_ATTEMPTS` ⇒ "presence auto-unlock can never scan: `[pipeline.rate_limit] max_attempts` must exceed 5".
- `presence.enabled && ceil(window / scan_interval) > max_attempts - PRESENCE_RESERVED_ATTEMPTS` ⇒ "presence scans will be throttled by the rate limit".

Worker start rule (`main.rs`): spawned iff `config.presence.enabled && config.dispatcher.enforce_active_session` (the harness mode `enforce_active_session = false`, only accepted with the mock camera, never starts presence: one `info` line says why).

#### `presence/logind.rs`

```rust
/// Validated logind session ID (ASCII alphanumeric, 1..=MAX_SESSION_ID_LEN bytes, reusing
/// `session_policy::is_valid_session_id`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionId(String);
impl SessionId {
    /// `None` for an invalid ID (never sent to logind).
    pub fn parse(raw: &str) -> Option<Self>;
    pub fn as_str(&self) -> &str;
}

/// One logind snapshot of a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogindSessionState {
    pub id: SessionId,
    /// Same struct and predicate as the `Auth` path (`check_local_seat_session_of`).
    pub record: SessionRecord,
    /// `LockedHint`; `false` when the property is absent or ill-typed (never scans).
    pub locked: bool,
    /// [R3: owner Q1] Session property `Name` (owner's user name, set by logind from the
    /// user record), validated by `UserName::parse`; `None` when absent, ill-typed or
    /// invalid ⇒ the account guard answers `Undeterminable` (no unlock).
    pub user_name: Option<UserName>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PresenceLogindError {
    #[error("system bus unavailable")]            BusUnavailable,
    #[error("logind call timed out")]             Timeout,
    #[error("logind call failed: {0}")]           Call(String),   // ≤ MAX_LOGIND_ERROR_LEN
    #[error("logind reply malformed")]            Malformed,
    #[error("too many logind sessions")]          TooManySessions, // > MAX_SCANNED_SESSIONS
}

/// Mockable logind access for the presence worker (tests: `MockPresenceLogind`).
pub trait PresenceLogind: Send + Sync + 'static {
    /// Every session of `ListSessions` with a non-empty seat, mapped through
    /// `session_state_from_properties`. More than `MAX_SCANNED_SESSIONS` listed ⇒
    /// `TooManySessions`; invalid IDs are skipped; a session that vanished between
    /// `ListSessions` and `GetAll` (`UnknownObject` / `NoSuchSession`) is skipped; any other
    /// per-session error fails the whole snapshot.
    fn seat_sessions(&self)
        -> impl Future<Output = Result<Vec<LogindSessionState>, PresenceLogindError>> + Send;
    /// Fresh state of one session; `Ok(None)` when it no longer exists.
    fn session_state(&self, id: &SessionId)
        -> impl Future<Output = Result<Option<LogindSessionState>, PresenceLogindError>> + Send;
    /// `Manager.LidClosed`.
    fn lid_closed(&self) -> impl Future<Output = Result<bool, PresenceLogindError>> + Send;
    /// `Manager.UnlockSession(id)`.
    fn unlock_session(&self, id: &SessionId)
        -> impl Future<Output = Result<(), PresenceLogindError>> + Send;
}

/// Pure mapping of a `GetAll` reply (`Id`, `User (uo)`, `Name s`, `Active b`, `State s`, `Remote b`,
/// `Seat (so)`, `Class s`, `LockedHint b`) to a state. `active = Active || State == "active"`
/// (same rule as `SessionRecord::parse`); `remote = Some(Remote)` only when the property is
/// a boolean, else `None` (denied by the binding); `seat`/`class` through `non_empty`;
/// `Id` must equal `expected_id` else `Malformed`.
pub fn session_state_from_properties(
    expected_id: &SessionId,
    properties: &HashMap<String, zbus::zvariant::OwnedValue>,
) -> Result<LogindSessionState, PresenceLogindError>;

/// Production implementation over the pinned system bus (D3). Holds
/// `tokio::sync::Mutex<Option<zbus::Connection>>`; connects lazily, drops the connection on
/// any transport error; registers no object server and no signal match.
pub struct ZbusLogind { /* connection slot */ }
impl ZbusLogind { pub fn new() -> Self; }
```

The system bus default policy denies method calls from other users to the daemon's unique name; zbus serves only the standard `Peer` interface. Message size is bounded by the D-Bus specification and the peer is the root-owned `org.freedesktop.login1` (a well-known name only root may own); our own bounds apply after decoding (`MAX_SCANNED_SESSIONS`, `MAX_PRESENCE_SEAT_SESSIONS`).

#### `presence/display.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayState { On, Off, Unknown }

pub trait DisplayProbe: Send + Sync + 'static {
    fn display_state(&self) -> DisplayState;
}

/// Reads `<dir>/card<N>-<connector>/{status,dpms}` (symlinks of the sysfs class directory are
/// followed; names must match `card[0-9]+-[A-Za-z0-9-]+`, `renderD*`, `cardN`, `version` are
/// ignored). `On` iff some connector has `status == "connected"` and `dpms == "On"`; `Off`
/// iff at least one connector is connected and none is `On`; `Unknown` when the directory is
/// unreadable, has more than `MAX_DRM_CONNECTORS` entries or no connected connector.
pub struct SysfsDisplayProbe { dir: PathBuf }
impl SysfsDisplayProbe {
    pub fn new(dir: PathBuf) -> Self;   // production: DEFAULT_DRM_SYSFS_DIR
}
/// Pure classifier over `(status, dpms)` pairs (unit-testable without a filesystem).
pub fn classify_connectors(connectors: &[(&str, &str)]) -> DisplayState;
```

Gating (owner: "when detectable"): `Off` ⇒ no scan; `On` and `Unknown` ⇒ scan allowed. Lid: `Ok(true)` ⇒ no scan; `Ok(false)` and `Err(_)` ⇒ scan allowed (an error on this single property only affects gating, never authorization; any error elsewhere still blocks the unlock).

#### `presence/switch.rs`

```rust
pub struct PresenceSwitch { dir: PathBuf }
impl PresenceSwitch {
    pub fn new(dir: PathBuf) -> Self;     // production: DEFAULT_KILL_SWITCH_DIR
    /// `true` when `<dir>/disabled` or `<dir>/presence.disable` exists as **any** entry
    /// (`symlink_metadata`, so a dangling symlink counts), and also on any stat error other
    /// than `NotFound` (fail closed toward "disabled").
    pub fn is_engaged(&self) -> bool;
}
```

#### `presence/tracker.rs` (pure, clockless)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockEntry {
    pub uid: u32,
    /// Monotonic ns of the first tick that observed this lock period.
    pub locked_since_ns: u64,
    /// Incremented each time the session is (re)observed transitioning to locked.
    pub epoch: u64,
    pub last_scan_started_ns: Option<u64>,
    pub unlock_requested_ns: Option<u64>,
    pub locker_ignored: bool,
}

#[derive(Debug, Default)]
pub struct LockTracker { entries: BTreeMap<SessionId, LockEntry>, next_epoch: u64 }

impl LockTracker {
    /// Folds one snapshot in. Sessions that are bound **and** locked are inserted (new lock
    /// period: `locked_since_ns = now_ns`, fresh epoch) or kept; every other session, and
    /// every tracked session absent from the snapshot, is removed (an unlock, a VT switch
    /// making it inactive, or logout ends the lock period, so a later lock restarts grace).
    /// A session whose UID changed is treated as a new lock period.
    /// # Errors
    /// `TrackerOverflow` when more than `MAX_TRACKED_LOCKED_SESSIONS` would be tracked
    /// (the tracker is cleared first).
    pub fn observe(&mut self, now_ns: u64, snapshot: &[LogindSessionState])
        -> Result<(), SkipReason>;
    /// Sessions whose grace has elapsed (`now_ns - locked_since_ns >= lock_grace`, saturating;
    /// the boundary itself is eligible), not `locker_ignored`, and whose
    /// `last_scan_started_ns` is `None` or `>= scan_interval` ago.
    pub fn due(&self, now_ns: u64, lock_grace: Duration, scan_interval: Duration)
        -> Vec<(SessionId, LockEntry)>;
    pub fn mark_scan_started(&mut self, id: &SessionId, now_ns: u64);
    /// After a successful `UnlockSession`: records `unlock_requested_ns`.
    pub fn mark_unlock_requested(&mut self, id: &SessionId, now_ns: u64);
    /// Called each tick: entries still locked `UNLOCK_CONFIRM_TIMEOUT_MS` after
    /// `unlock_requested_ns` become `locker_ignored` (returns their IDs for one warn each).
    pub fn expire_unconfirmed_unlocks(&mut self, now_ns: u64) -> Vec<SessionId>;
    pub fn get(&self, id: &SessionId) -> Option<&LockEntry>;
}

/// Exactly-one rule (D5) over the due sessions already filtered for enrollment.
pub fn select_candidate(due: Vec<(SessionId, LockEntry)>)
    -> Result<(SessionId, LockEntry), SkipReason>;   // 0 ⇒ NoCandidate, ≥2 ⇒ AmbiguousCandidates
```

#### `presence/account.rs` — account guard *[R3: owner Q1]*

Owner rule (2026-10-02): a presence unlock must respect `pam_faillock` and account expiry; locked, expired **or undeterminable** ⇒ no unlock, the password stays the only path. The guard replaces former accepted risk (d).

```rust
/// Validated user name from logind `Name`: 1..=MAX_USER_NAME_LEN bytes of
/// [A-Za-z0-9._-] plus a final optional '$', not starting with '-' or '.', never "." / "..",
/// no '/' (it becomes a path component of the tally file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserName(String);
impl UserName { pub fn parse(raw: &str) -> Option<Self>; pub fn as_str(&self) -> &str; }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountRefusal {
    Faillocked,          // pam_faillock would deny now
    AccountExpired,      // shadow `expire` reached (0 included)
    AccountInactive,     // password expired for more than `inactive` days
    PasswordExpired,     // `lastchg + max` passed (pam_unix: new password required)
    PasswordChangeForced,// `lastchg == 0`
    PasswordLocked,      // password field starts with '!' or is '*' (passwd -l / usermod -L / no login)
    RootAccount,         // UID 0: presence never unlocks a root session
    Undeterminable,      // any read/parse/bound/clock/option problem below
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountState { Usable, Refused(AccountRefusal) }

/// Mockable (tests: `StaticAccountGuard`, `ScriptedAccountGuard`). Synchronous, bounded file
/// reads; the worker runs it through `spawn_blocking` under `ACCOUNT_CHECK_TIMEOUT_MS`
/// (timeout or join error ⇒ `Refused(Undeterminable)`). Never writes anything.
pub trait AccountGuard: Send + Sync + 'static {
    fn check(&self, user: &UserName, uid: u32) -> AccountState;
}

/// Production guard; every path is injectable for tests (tempdir trees).
pub struct SystemAccountGuard {
    faillock_conf: PathBuf,          // DEFAULT_FAILLOCK_CONF
    vendor_faillock_conf: PathBuf,   // VENDOR_FAILLOCK_CONF (read only when the first is absent)
    pam_dirs: Vec<PathBuf>,          // DEFAULT_PAM_DIRS
    shadow: PathBuf,                 // DEFAULT_SHADOW_PATH
    realtime: fn() -> Result<u64, DaemonError>, // CLOCK_REALTIME seconds (tally/shadow are wall clock)
}
```

`check` order (first refusal wins; all-or-nothing):
1. `uid == 0` ⇒ `RootAccount`.
2. `realtime()` error ⇒ `Undeterminable`.
3. **PAM-stack option scan** (`scan_pam_faillock_options`, every call): *[R4: A5]* for each entry directly in each existing directory of `DEFAULT_PAM_DIRS` (≤ `MAX_PAM_DIR_ENTRIES` per directory) that is, or resolves through symlinks to, a regular file — symlinks are **followed**, as libpam does (authselect ships `/etc/pam.d/system-auth`, `password-auth`, … as symlinks into `/etc/authselect/` on Fedora/RHEL, and those targets may live outside the PAM directory); the entry is opened following symlinks with `O_RDONLY | O_NONBLOCK | O_CLOEXEC`, then `fstat` on the opened descriptor decides: a regular file is read (≤ `MAX_PAM_FILE_BYTES`, valid UTF-8; otherwise `Undeterminable`), a directory (direct or through a symlink) is skipped, and a dangling symlink, a symlink loop, any open error, or an entry resolving to anything else (FIFO, socket, device) ⇒ `Undeterminable`. Filtering with `symlink_metadata().is_file()` is forbidden (it would silently skip authselect stacks — fail open). For every file read, *[R3: F8]* after joining backslash-continued lines (Linux-PAM accepts `\` line continuation) and dropping `#` comments, every line containing a whitespace-separated token that ends with `pam_faillock.so` (so the control field may be a bracketed `[success=1 default=bad]` with spaces, a `-` prefixed type, or a full module path) and whose tokens after that module token include an argument in `dir=`, `deny=`, `fail_interval=`, `unlock_time=`, `root_unlock_time=`, `admin_group=`, `conf=` or `even_deny_root` ⇒ `Undeterminable`. Rationale: such arguments override `faillock.conf` per stack and the daemon cannot know which stack the locker uses; refusing is the fail-closed reading, and administrators keep policy in `faillock.conf` (documented). `preauth`, `authfail`, `authsucc`, `silent`, `audit`, `no_log_info`, `local_users_only`, `nodelay` are harmless. A missing directory is skipped; an unreadable one ⇒ `Undeterminable`.
4. **`faillock.conf`** (`parse_faillock_conf`): `DEFAULT_FAILLOCK_CONF` when it exists; otherwise *[R3: F9]* the built-in defaults **and**, when `VENDOR_FAILLOCK_CONF` exists, also the vendor policy — whether PAM falls back to the vendor file depends on how Linux-PAM was built (`--enable-vendordir`), so the guard evaluates the tally under both policies and refuses if either denies (the strictest reading); a read error on either file ⇒ `Undeterminable`. Each file: ≤ `MAX_FAILLOCK_CONF_BYTES`, UTF-8; `#` starts a comment; `name [=] value` or a bare flag, the grammar of `faillock_config.c`. Known keys exactly as `set_conf_opt`: `dir` (absolute, no `..` component, ≤ 4096 bytes), `deny` (u16), `fail_interval` / `unlock_time` / `root_unlock_time` (u32 ≤ `MAX_FAILLOCK_TIME_INTERVAL`, `never` = 0 for the two unlock keys), `admin_group` (non-empty), flags `even_deny_root audit silent no_log_info local_users_only nodelay`. **Stricter than PAM**: a malformed value, an unknown key or an oversized file is `Undeterminable` (PAM only logs and keeps the default).
5. **Tally** (`read_tally_records` + `faillock_denies`): *[R4: A5, auditor constraints 9–18]* the tally directory `<dir>` itself must not be a symlink (`symlink_metadata`) and must be a directory that is not group- or other-writable, else `Undeterminable`; open `<dir>/<user>` with `O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC` (tally files are never followed through symlinks); `ENOENT` ⇒ zero records (not locked, as PAM); any other open error (`EACCES` included, which PAM treats as "not locked") ⇒ `Undeterminable`; `fstat` must be a regular file; take `Flock` `LockSharedNonblock` (`EWOULDBLOCK` ⇒ `Undeterminable`, a `pam_faillock` writer is mid-update and a truncated read could show zero failures); size must be a multiple of `TALLY_RECORD_BYTES` (64) and ≤ `MAX_TALLY_BYTES` (1088 records), else `Undeterminable`; decode native-endian `status` (offset 54) and `time` (offset 56); `source` is never kept or logged.
   `faillock_denies(policy, records, now_s) -> bool` replicates `check_tally`: `latest = max(time of VALID)`; `failures = #VALID with latest.saturating_sub(time) < fail_interval` (a future-dated record counts); locked iff `deny != 0 && failures >= deny` and **not** (`unlock_time != 0 && latest + unlock_time < now`, checked arithmetic, overflow ⇒ locked). Admin handling (the daemon performs no group lookup): with `admin_group` set, the user is evaluated as possibly-admin **and** possibly-non-admin and refused if either would be locked — i.e. as non-admin with the stricter of `unlock_time` / `root_unlock_time` (`0` = never is strictest); `even_deny_root` only matters for admins and is therefore irrelevant once both readings are checked. `local_users_only` is ignored (evaluating anyway can only refuse more).
6. **Shadow** (`find_shadow_entry` + `shadow_refusal`): `/etc/shadow` ≤ `MAX_SHADOW_BYTES` and ≤ `MAX_SHADOW_LINES`, read into a `Zeroizing<Vec<u8>>`, UTF-8; exactly one line whose first field equals the user (0 or ≥ 2 ⇒ `Undeterminable`; NSS-only and systemd-homed users therefore never get presence unlock); exactly 9 `:` fields; numeric fields are empty or decimal `i64` ≥ 0 (anything else ⇒ `Undeterminable`). Only the needed fields are copied out; the password hash is never copied, only classified (`starts_with('!') || == "*"` ⇒ `PasswordLocked`). With `today = now_s / 86400` *[R3: F10, rule restated]*, evaluated in this order: (a) `expire` set and `today >= expire` ⇒ `AccountExpired` (`expire = 0` counts as expired, the conservative reading of shadow(5)); (b) `lastchg = 0` ⇒ `PasswordChangeForced`; (c) `max` and `lastchg` set and `inactive` set and `today > lastchg + max + inactive` ⇒ `AccountInactive`; (d) `max` and `lastchg` set and `today > lastchg + max` ⇒ `PasswordExpired`. These are the `pam_unix` account-phase refusals (`PAM_ACCT_EXPIRED` for (a)/(c), `PAM_NEW_AUTHTOK_REQD` for (b)/(d)); presence must not become a way around a forced password change. `min`/`warn` ignored. Checked arithmetic, overflow ⇒ `Undeterminable`.
7. Otherwise `Usable`.

**No tally reset** (decision): a successful presence unlock never writes `/run/faillock` (the unit keeps it read-only, `ReadWritePaths` unchanged). (1) The tally is reset by `pam_faillock authsucc` after a PAM authentication; presence is not one, and the guard already refused while locked, so there is nothing to clear on a lockout. (2) Keeping sub-threshold failures means further wrong passwords reach `deny` sooner — the conservative direction. (3) Writing a user-owned file from the root daemon would add a write path and a race with `pam_faillock`'s own `flock`. This differs from the Arch PAM face path (ADR 2026-10-02 "Arch Face Match Runs the Stock Success Path"), where `authsucc` runs inside PAM.

Residuals (documented, not mitigated): the tally file is owned by the user (`0660 user:root`, pam_faillock design), so the user or malware running as the user can truncate it — exactly as for PAM; a lock driven by `pam_tally2`, `pam_tally`, `pam_access`, `pam_time` or other account modules is not seen (only `pam_faillock` and shadow expiry are honoured, as decided by the owner).

#### `presence/worker.rs`

```rust
/// Why a tick ended without a scan (stable, value-free `as_str()` codes for logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    KillSwitch, LogindUnavailable, TooManySessions, TrackerOverflow, NoLockedSession,
    InGrace, NotDue, NotEnrolled, ForeignTemplate, TemplateStoreError, NoCandidate,
    AmbiguousCandidates, LidClosed, DisplayOff, InteractiveDemand, RateLimited,
    ClockUnavailable, Backoff, AccountRefused, // [R3: owner Q1]
}

/// Result of a scan that started (an attempt was recorded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanOutcome {
    Unlocked { session: SessionId, uid: u32 },
    NoMatch,                 // Decided Pending / Deny-class decision
    SpoofVetoed,
    Preempted,
    CameraUnavailable,
    Aborted(ReasonClass),    // ConsensusRun::Aborted
    SessionChanged,          // re-check refused (unbound, unlocked, other UID, gone)
    AllowExpired,            // MAX_ALLOW_TO_UNLOCK_MS exceeded
    UnlockFailed,            // UnlockSession error or timeout
    AccountRefused(AccountRefusal), // [R3: owner Q1] faillocked / expired / undeterminable at re-check
    KillSwitchEngaged,       // [R4: A6] kill switch engaged between step 1 and the unlock call
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresenceTick { Skipped(SkipReason), Scanned(ScanOutcome) }

pub struct PresenceWorker<L: PresenceLogind, D: DisplayProbe, A: AccountGuard> { /* config, logind, display,
    switch, account_guard: A [R3], pipeline: PipelineComponents, inference: InferenceGate, expected_model: Option<String>,
    clock_fn, tracker: LockTracker, backoff */ }

impl<L: PresenceLogind, D: DisplayProbe, A: AccountGuard> PresenceWorker<L, D, A> {
    pub fn new(config: PresenceConfig, logind: L, display: D, switch: PresenceSwitch, account_guard: A,
               pipeline: PipelineComponents, inference: InferenceGate) -> Self;
    pub fn with_expected_embedding_model(self, id: impl Into<String>) -> Self;
    pub fn with_clock_fn(self, f: fn() -> Result<u64, DaemonError>) -> Self;   // test hook
    /// One deterministic iteration (test hook).
    pub async fn tick(&mut self) -> PresenceTick;
    /// `tick` every `LOCK_POLL_INTERVAL_MS` (plus backoff) until `shutdown` turns `true`.
    pub async fn run(self, shutdown: tokio::sync::watch::Receiver<bool>);
}
```

**Tick algorithm** (every step that fails ends the tick with no unlock; order matters):

1. `switch.is_engaged()` ⇒ `Skipped(KillSwitch)` (tracker cleared, so re-enabling restarts every grace).
2. Inside backoff window ⇒ `Skipped(Backoff)`.
3. `now_ns = clock()`; error ⇒ `Skipped(ClockUnavailable)`.
4. `logind.seat_sessions()` (timeout) ; error ⇒ backoff, tracker cleared, `Skipped(LogindUnavailable | TooManySessions)`. More than `MAX_PRESENCE_SEAT_SESSIONS` ⇒ `Skipped(TooManySessions)`.
5. `tracker.observe(now_ns, &snapshot)` (binding = `record.check_local_seat_session_of(record.uid?)` and `locked`); `expire_unconfirmed_unlocks` (warn once per ID). Nothing tracked ⇒ `Skipped(NoLockedSession)`.
6. `due = tracker.due(now_ns, lock_grace, scan_interval)`; empty ⇒ `Skipped(InGrace | NotDue)`.
7. For each due entry: `biometric_store.get(uid)`: `Ok(None)` ⇒ drop (`NotEnrolled`), `Err` ⇒ drop (`TemplateStoreError`), `Foreign` (`classify_template` with the expected model, same rule as dispatcher 8b) ⇒ drop (`ForeignTemplate`). No attempt, no camera. *[R3: owner Q1]* Then `account_guard.check(&user_name, uid)` (on the blocking pool, bounded by `ACCOUNT_CHECK_TIMEOUT_MS`): anything but `AccountState::Usable` ⇒ drop (`AccountRefused`), so a faillocked or expired account costs no attempt and never wakes the camera. `select_candidate` on the rest ⇒ `NoCandidate` / `AmbiguousCandidates`.
8. `logind.lid_closed()` = `Ok(true)` ⇒ `Skipped(LidClosed)`; `display.display_state() == Off` ⇒ `Skipped(DisplayOff)`.
9. `inference.interactive_demand() > 0` ⇒ `Skipped(InteractiveDemand)`.
10. `policy.write().await.record_attempt_with_reserve(uid, now_ns, PRESENCE_RESERVED_ATTEMPTS)`; `Err` ⇒ `Skipped(RateLimited)`. From here the tick is a scan (`mark_scan_started`).
11. `camera.notify_activity()`; `wake_camera(.., MAX_CAMERA_WAKE_WAIT_MS)` false ⇒ `Scanned(CameraUnavailable)`.
12. `thresholds = *policy.read().await.thresholds()`; `run_face_consensus(ctx{priority: Background}, &template.embedding, deadline)`; the template is dropped (zeroized) right after. `Preempted` / `Aborted` / `Decided(SpoofVetoed | Pending)` ⇒ corresponding outcome.
13. `Decided(Allow)`: `allow_ns = clock()`; `logind.session_state(id)` (fresh) must be `Some`, `locked`, and `record.check_local_seat_session_of(candidate_uid)` OK, and its `user_name` must equal the one checked in step 7; else `Scanned(SessionChanged)`. *[R3: owner Q1]* In the same re-check, a **fresh** `account_guard.check(&user_name, candidate_uid)` (files re-read, nothing cached from step 7 or from any earlier call of the same guard instance) must return `AccountState::Usable`; else `Scanned(AccountRefused(kind))`. *[R4: A6]* Finally, immediately before `unlock_session`, `switch.is_engaged()` is evaluated again: engaged (`/etc/soos/disabled` or `/etc/soos/presence.disable` created during the scan, or a stat error) ⇒ no unlock, `Scanned(KillSwitchEngaged)`, tracker cleared. Then `clock() - allow_ns <= MAX_ALLOW_TO_UNLOCK_MS` else `Scanned(AllowExpired)`. *[R2: F2]* The re-check proves the session is **still** bound, locked and owned by the scanned UID; it cannot prove that the lock period did not end and restart during the scan (the tracker is only updated by ticks and no tick runs while the single worker task scans), so no epoch comparison is made here and the residual is stated as accepted risk (e). The `epoch` field is kept for the tracker's own bookkeeping and logs only.
14. `logind.unlock_session(id)` (timeout): `Err` ⇒ backoff, `Scanned(UnlockFailed)`; `Ok` ⇒ `mark_unlock_requested`, one `info!` (`session_id`, `uid`, `captures_evaluated`), `Scanned(Unlocked)`.

There is exactly one call site of `unlock_session` in production code, reachable only through step 14 (invariant test §8 C3). `run` catches nothing itself; `main.rs` keeps the `JoinHandle` and, if the task ends with a panic, logs one `error!` ("presence worker stopped; auto-unlock disabled until restart") — the daemon and PAM keep running, and a dead worker never unlocks.

Logging rules: `debug!` for `SkipReason` only on change from the previous tick's reason (no 1 Hz spam); `warn!` for logind unavailability on transition and for `SpoofVetoed` / `locker_ignored`; `info!` for `Unlocked`; *[R2: F6]* `info!` (value-free: `display_state`, `lid_closed`) on every transition of the lid/screen gate between "scan allowed" and "scan gated", so a driver that misreports DPMS (presence silently never scanning) is diagnosable from `journalctl -u soos-daemon`; never a frame, an embedding, a template or a PAD/match score above `debug` (the existing `debug!(score, threshold)` lines inside the moved consensus code are unchanged).

### 2.7 Main wiring (`crates/daemon/src/main.rs`)

- Before `ConnectionDispatcher::with_pipeline(.., components)`: `let presence_components = components.clone();` and keep a clone of `inference_gate` before `.with_inference_gate(inference_gate)`.
- After `sd_notify::notify_ready()` (startup and `READY=1` never wait for D-Bus): if the start rule holds, `let (presence_stop, presence_rx) = watch::channel(false);` and `tokio::spawn(PresenceWorker::new(..ZbusLogind::new(), SysfsDisplayProbe::new(DEFAULT_DRM_SYSFS_DIR.into()), PresenceSwitch::new(DEFAULT_KILL_SWITCH_DIR.into()), ..).with_expected_embedding_model(EMBEDDING_MODEL_ID).run(presence_rx))`.
- Shutdown: right after `accept_until_shutdown` returns, `presence_stop.send(true)`, then `tokio::time::timeout(min(drain_budget, 500 ms), handle)`, then `abort()` (an aborted worker cannot reach step 14 afterwards; a scan's inference job left on the blocking pool is bounded by `shutdown_runtime`).
- No ordering change in the unit: D-Bus (`dbus.socket`, `sockets.target`) is up before any default-dependency service; the lazy connect + backoff covers a bus restart.

---

## 3. Constants & Config Summary

| Key | Type | Default | Range / sentinel | Location |
|---|---|---|---|---|
| `[presence] enabled` | bool | `true` | — ; `false` ⇒ worker not spawned | `presence::config::DEFAULT_PRESENCE_ENABLED` |
| `[presence] scan_interval_ms` | integer ms | `2000` | `[1000, 60000]`; `0` rejected | `DEFAULT_SCAN_INTERVAL_MS` |
| `[presence] lock_grace_ms` | integer ms | `3000` | `[1000, 60000]`; `0` rejected | `DEFAULT_LOCK_GRACE_MS` |
| `[pipeline.rate_limit] max_attempts` | integer | **`40`** (was 5) | `>= 1` (unchanged) | `soos_policy::RateLimitConfig::DEFAULT_MAX_ATTEMPTS` |
| `/etc/soos/disabled` | flag | absent | any entry ⇒ face (PAM) and presence off | existing + D9 |
| `/etc/soos/presence.disable` | flag | absent | any entry ⇒ presence off | D9 |

---

## 4. Error Taxonomy

| Source | Variant | Presence effect | PAM effect |
|---|---|---|---|
| D-Bus transport / call | `PresenceLogindError::{BusUnavailable, Timeout, Call, Malformed, TooManySessions}` | tick skipped or scan ends `SessionChanged`/`UnlockFailed`; backoff; **no unlock** | none |
| Clock | `DaemonError::Clock` | `Skipped(ClockUnavailable)` or `Aborted(InternalError)`; **no unlock** | unchanged |
| Store | `get` error / `None` / `Foreign` | candidate dropped, no attempt, no camera | unchanged |
| Rate limit | `PolicyError::RateLimitExceeded` | `Skipped(RateLimited)` | unchanged (`ProtocolError`/`RateLimited`) |
| Camera | not ready after wake | `Scanned(CameraUnavailable)` | unchanged |
| Vision / inference job | `ConsensusRun::Aborted` | `Scanned(Aborted(reason))` | unchanged (`Unavailable`/…) |
| Consensus | `Pending`, `SpoofVetoed` | `NoMatch`, `SpoofVetoed` | unchanged |
| Priority | `Preempted` | `Scanned(Preempted)` | n/a (`Interactive` never preempted) |
| Re-check | binding/lock/UID mismatch, `None` | `Scanned(SessionChanged)` | n/a |
| Account *[R3]* | `AccountState::Refused(_)` (faillocked, expired, inactive, password expired / forced / locked, root, undeterminable) | step 7: candidate dropped, no attempt, no camera; step 13: `Scanned(AccountRefused)`; **no unlock** | none |
| Worker panic | task ends | logged, presence off until restart | none |

There is no variant, default or `unwrap_or` from which `unlock_session` is called; `Verdict`/`ReasonClass` values are not created by presence (it only reads `ConsensusDecision`).

---

## 5. Latency Budget

PAM side (unchanged arithmetic, one new term):

```text
sudo / console:  pam timeout_ms 1000 (clamped 10–5000)
                 daemon budget = min(client − 50 ms, start + connection_timeout 2500 − 50 ms)
                               ≈ 950 ms  ≥  DECISION_BUDGET_MS-class consensus (~300 ms for k = 3)
                 + NEW worst case: one in-flight presence inference ≤ estimate (typ. < 100 ms,
                   clamp 1000 ms) absorbed by acquire_within(remaining − estimate)
GDM:             timeout_ms 2500 ⇒ ≈ 2450 ms daemon budget; same single-job term.
```

Presence side: one scan ≤ camera wake (`MAX_CAMERA_WAKE_WAIT_MS` 1200 ms, only from standby) + `DECISION_BUDGET_MS` 900 ms + re-check (≤ 500 ms) + unlock (≤ 500 ms) ⇒ ≤ 3.1 s worst case, typically ~0.4 s; scans start at most every `scan_interval` (2 s) ⇒ ≤ 30 attempts / min at the defaults, below `40 − PRESENCE_RESERVED_ATTEMPTS = 35`. End-to-end unlock latency after the user sits down: ≤ tick (1 s) + scan interval phase (≤ 2 s) + scan (~0.4 s).

CPU: ~0.3 s of inference per 2 s while a session is locked with the screen on (~15 % of one core); zero when no eligible session is locked.

---

## 6. Invariants Touched

- **ARCHITECTURE §2 invariant 1** ("PAM returns `PAM_SUCCESS` only upon a fresh `Allow`") is unchanged for PAM; presence is a **new authorization path that bypasses PAM**. New invariant **6** (to add to §2) *[R2: F2 wording]*: *"The daemon asks logind to unlock a session only after a fresh, single-use `Allow` consensus of the unchanged pipeline for that session owner's UID, obtained after the lock grace period, and only if a logind re-check made after that `Allow` still shows the session bound (local, active, `REMOTE=0`, seat, `CLASS=user`), locked and owned by that UID, and a fresh account check finds the owner neither locked by `pam_faillock` nor expired (account or password); every error, unknown state or timeout leaves the session locked."* *[R3: owner Q1]*
- *[R3: owner Q1]* **ARCHITECTURE §2 invariant 3** says "no user database lookup is made". It stays true for IPC requests; presence reads `/etc/shadow` and `/run/faillock/<name>` for the logind-provided name of a session owner, never resolving a payload value. The invariant gets the clause "(the presence account guard reads the shadow and faillock records of a logind session owner, never of a payload identity)"; it must not mention `/etc/passwd` (`test_architecture_doc_matches_daemon_code_claims`).
- Invariant 3 (no payload trust): presence has no payload; the UID comes from logind, the template from the store.
- Invariant 5 (degrade to password): every presence failure leaves the locker and its PAM password path untouched.
- No IPC schema change (`MAX_MESSAGE_SIZE`, codec untouched); no frame/embedding in logs; `Zeroizing` template dropped after each scan.
- `MAX_CONCURRENT_INFERENCES = 1` preserved; PAM priority (D6).
- Camera lifecycle: `notify_activity` is called **only** at tick step 11, i.e. only while an eligible, enrolled, due session exists and the lid/screen gates pass; otherwise the existing `idle_timeout` auto-standby applies unchanged.
- Matrix rows affected: DMN-10 reservation (rate-limit semantics extended, not changed), PAD-02 consensus (moved, not changed).

---

## 7. Acceptance Criteria — new matrix section `presence-auto-unlock` (GitHub #323)

| # | Criterion |
|---|---|
| PAU1 | `[presence]` defaults `enabled = true`, `scan_interval_ms = 2000`, `lock_grace_ms = 3000`; absent file, empty file and empty table give the same `PresenceConfig`; `scan_interval_ms` / `lock_grace_ms` of `0`, `999`, `60001` are startup errors naming the key; `1000` and `60000` accepted. |
| PAU2 | `RateLimitConfig::DEFAULT_MAX_ATTEMPTS` is 40 (window 60 s) and is the daemon default: the 41st `Auth` attempt of one UID inside 60 s is `ProtocolError`/`RateLimited`, the 40th is not; documented in `Docs/DAEMON.md` and `Docs/POLICY_CRATE.md`. |
| PAU3 | `check_and_record_with_reserve` refuses without recording when `remaining <= reserve`, records otherwise, equals `check_and_record` for `reserve = 0`, always refuses for `reserve >= max_attempts`; property test over arbitrary sequences: presence-style calls never leave fewer than `reserve` attempts for `check_and_record`. |
| PAU4 | Kill switch: `<dir>/disabled` or `<dir>/presence.disable` as a regular file, directory or dangling symlink, or a stat error other than `NotFound`, engages it; the worker then performs no logind snapshot, no attempt, no `notify_activity` and no unlock, and resumes at the next tick after removal (no restart), with every grace restarted. |
| PAU5 | Only sessions passing `check_local_seat_session_of(uid)` and `LockedHint = true` are tracked: table test over remote (`Remote = true`), unknown remote (property missing/ill-typed), seatless, inactive, `CLASS=greeter`/`manager`, foreign-UID, unlocked and invalid-ID sessions ⇒ never scanned, never unlocked. |
| PAU6 | Grace: at `locked_since + lock_grace − 1 ns` no scan; at exactly `lock_grace` the session is due; a session observed unlocked (or absent, or inactive) and then locked again restarts grace with a new epoch; after a presence unlock the next lock period starts a fresh grace. |
| PAU7 | Two eligible sessions (two seats, or two locked active sessions of enrolled users) ⇒ no scan, no attempt, no unlock (`AmbiguousCandidates`); one eligible + one not enrolled ⇒ the enrolled one is scanned. |
| PAU8 | Not enrolled, `Foreign` template and store error ⇒ no attempt recorded, no `notify_activity`, no inference. |
| PAU9 | `LidClosed = true` ⇒ no scan; DRM classification `Off` ⇒ no scan; `Unknown` (unreadable dir, > 64 entries, no connected connector) and `On` ⇒ scan allowed; parser ignores `renderD*`, `cardN`, oversized attributes. |
| PAU10 | Pipeline unchanged: `PadAggregator::with_defaults` is constructed only in `consensus.rs`; dispatcher and presence both call `run_face_consensus` with `policy.thresholds()`; a presence scan unlocks only after `k = 3` consecutive passing captures; one spoof capture vetoes (no unlock); thresholds from `[pipeline.thresholds]` apply to presence (a raised `match_threshold` makes the same mock score fail). The full pre-existing daemon test suite passes unmodified. |
| PAU11 | Unlock only on `Allow` + fresh re-check: `unlock_session` is called exactly once per `Allow` and never when the re-check returns unlocked, unbound, other UID, `None`, a logind error, or when `MAX_ALLOW_TO_UNLOCK_MS` is exceeded; fault injection over every `PresenceLogindError` variant at every logind call, clock failure, inference panic, `VisionError::Inference`, camera not ready ⇒ zero unlock calls. |
| PAU12 | PAM priority: with a live `InteractiveDemandGuard` `try_acquire_background` returns `None` and a tick returns `Skipped(InteractiveDemand)`; a background consensus returns `Preempted` before its next inference once a guard appears; the dispatcher holds a guard for every `Auth` request in Step 8 (dropped on every return path); an `Auth` request started while a presence job runs still renders `Allow` within its deadline in the mock pipeline. |
| PAU13 | Camera standby: with no tracked locked session (or only in-grace, gated, not-enrolled or kill-switched sessions) `notify_activity` is called zero times over 20 ticks. |
| PAU14 | Bounds: > `MAX_SCANNED_SESSIONS` listed ⇒ `TooManySessions`; > `MAX_PRESENCE_SEAT_SESSIONS` seat sessions ⇒ skip; tracker overflow clears and skips; every D-Bus call is wrapped in `DBUS_CALL_TIMEOUT_MS` (a never-resolving mock call ends the tick within the timeout); backoff doubles from 1 s and saturates at 30 s, reset on success; `PresenceLogindError::Call` text ≤ 256 bytes. |
| PAU15 | A session still locked `UNLOCK_CONFIRM_TIMEOUT_MS` after a successful unlock call is marked `locker_ignored` (one warn), is not scanned again, and becomes eligible again only after it is observed unlocked and re-locked. |
| PAU16 | Logging: the presence and consensus modules log no frame, embedding, template or score field above `debug`; unlock logs one `info` with session ID and UID; repeated identical skip reasons log once (transition-only). |
| PAU17 | Dependency: `zbus` 5 is declared once in `[workspace.dependencies]` with `default-features = false, features = ["tokio"]` and used only by `soos-daemon`; `crates/pam/Cargo.toml` has no `zbus`/`dbus`; the production code connects only to `SYSTEM_BUS_ADDRESS` (no `Connection::system`/`session`); `cargo deny --locked check` passes. |
| PAU18 | `session_state_from_properties` maps a GetAll map exactly like `SessionRecord::parse` (`Active` or `State = active`, boolean `Remote` only, empty seat/class ⇒ `None`), rejects an `Id` mismatch as `Malformed`, and treats a missing/ill-typed `LockedHint` as not locked. |
| PAU19 | Worker start rule: not spawned with `enabled = false` or with `enforce_active_session = false`; startup and `READY=1` never wait for D-Bus; SIGTERM stops the worker within the drain budget and no unlock is issued after the stop signal; *[R2: F7]* with no system bus (the `tests/docker/Dockerfile.systemd` acceptance container installs no D-Bus and masks `systemd-logind`) the worker logs one `warn` and backs off, and `tests/docker/systemd_unit_acceptance_test.sh` (READY ordering, clean stop) passes unchanged. |
| PAU20 | Documentation: ADR "Presence Auto-Unlock Through logind" in `AI/DECISIONS.md` with the owner decisions and accepted risks; ARCHITECTURE invariant 6 and diagram; `Docs/DAEMON.md` documents every `[presence]` key (enforced by `daemon_docs_contract::test_daemon_doc_documents_every_daemon_toml_key`) and the presence path in §3; `Docs/DISTRIBUTION_DEPLOYMENT.md` lists desktop support, the kill switches and the swayidle `unlock` / `SetLockedHint` hooks; `AI/MOCK_STRATEGY.md` describes `MockPresenceLogind`. |
| PAU22 | *[R3]* `parse_faillock_conf`: absent `/etc` and vendor files ⇒ defaults 3 / 900 / 600 / `/run/faillock`, `root_unlock_time = unlock_time`; when the `/etc` file is absent and a vendor file exists, the tally is evaluated under both the defaults and the vendor policy and refused if either denies (vendor `deny = 10` with 3 failures ⇒ refused by the defaults); `never`, `name value` and `name=value` forms, comments and blank lines parsed like `faillock_config.c`; an unknown key, a non-numeric or out-of-range value (`fail_interval = 604801`, `deny = 65536`), a relative or `..` `dir`, non-UTF-8 or > 64 KiB ⇒ `Undeterminable`. |
| PAU23 | *[R3]* Tally reading: missing file ⇒ zero records; `EACCES`, symlink (`O_NOFOLLOW`), FIFO/directory, size not a multiple of 64, > `MAX_TALLY_BYTES`, or a held exclusive `flock` ⇒ `Undeterminable`; *[R4: A5]* a tally directory that is a symlink or group/other-writable ⇒ `Undeterminable`; records built byte-for-byte as `struct tally` (native endian, `status` at 54, `time` at 56) decode to the expected `(status, time)`; the `source` bytes are never returned or logged. |
| PAU24 | *[R3]* `faillock_denies` equals a reference port of `check_tally` on arbitrary records/policies (proptest, non-admin) and on table cases: `failures = deny − 1` not locked, `= deny` locked; `deny = 0` never locked; a record exactly `fail_interval` older than the latest not counted; `latest + unlock_time == now` still locked, `< now` unlocked; `unlock_time = 0` permanently locked; invalid-status records ignored; future-dated records counted; with `admin_group` set the stricter of `unlock_time` / `root_unlock_time` applies. |
| PAU25 | *[R3]* PAM option scan: a `pam_faillock.so` line in any file of `/etc/pam.d`, `/usr/lib/pam.d` or `/usr/etc/pam.d` with `deny=`, `dir=`, `fail_interval=`, `unlock_time=`, `root_unlock_time=`, `admin_group=`, `conf=` or `even_deny_root` (also after a `[success=1 default=bad]` control containing spaces, a `-auth` type, a `/full/path/pam_faillock.so` module, and an argument carried on a `\`-continued line) ⇒ `Undeterminable`; `preauth`/`authfail`/`authsucc`/`silent`/`audit` only, commented lines and other modules carrying `deny=` ⇒ no effect; > 512 entries or a file > 64 KiB ⇒ `Undeterminable`; *[R4: A5]* a symlinked stack file (authselect style, target outside the PAM directory) is followed: target with `deny=1` ⇒ `Undeterminable`, target with only `preauth` ⇒ usable; a dangling symlink or a symlink to a FIFO ⇒ `Undeterminable`; a symlink to a directory is skipped. |
| PAU26 | *[R3]* Shadow: `expire` reached (and `expire = 0`) ⇒ `AccountExpired`; `lastchg = 0` ⇒ `PasswordChangeForced`; `today > lastchg + max` ⇒ `PasswordExpired`, beyond `+ inactive` ⇒ `AccountInactive`; `!hash`, `!`, `!!`, `*` ⇒ `PasswordLocked`; empty numeric fields mean "not set"; user absent, duplicated, wrong field count, negative or non-numeric field, oversize ⇒ `Undeterminable`; boundary `today == lastchg + max` still usable; no field of the file appears in any log or error string. |
| PAU27 | *[R3]* `UserName::parse` accepts `alice`, `a.b-c_d`, `host$`, rejects empty, `.`, `..`, `-x`, `.x`, `a/b`, `../x`, non-ASCII, 257 bytes; a session whose `Name` is missing or invalid is never unlocked; a UID 0 session is never scanned or unlocked (`RootAccount`). |
| PAU28 | *[R3]* Worker: a refused account at step 7 records no attempt, never calls `notify_activity` and never runs inference; at step 13 the guard is called again after the `Allow` with no cached result (a `ScriptedAccountGuard` returning `Usable` then `Refused(Faillocked)` yields `Scanned(AccountRefused)` and zero unlock calls); a guard that blocks longer than `ACCOUNT_CHECK_TIMEOUT_MS` ⇒ `Undeterminable`, zero unlock calls; a changed `Name` between step 7 and the re-check ⇒ `SessionChanged`; *[R4: A6]* a kill-switch flag created after the `Allow` and before the unlock call ⇒ `Scanned(KillSwitchEngaged)`, zero unlock calls. |
| PAU29 | *[R3]* No writes and no reset: `crates/daemon/src/presence/**` never opens `/run/faillock`, `/etc/shadow` or `/etc/pam.d` for writing (no `OpenOptions::write`/`append`/`create`/`truncate`, no `std::fs::write`, no `faillock --reset`); `packaging/soos-daemon.service` keeps `CAP_DAC_OVERRIDE` in `CapabilityBoundingSet`, keeps `ProtectSystem=strict` with `ReadWritePaths=/var/lib/soos /run/soos`, and has no `InaccessiblePaths=` covering `/etc/shadow`, `/etc/security`, `/etc/pam.d` or `/run/faillock`; the decision is documented (ADR, `Docs/DAEMON.md`, `Docs/PACKAGING_AND_PROVISIONING.md` §8.1). |
| PAU21 | Physical (⬜ Pending until run on hardware): GNOME/GDM — lock, leave, return ⇒ unlocked within ~4 s; lock while seated ⇒ unlocked after grace (expected); photo / phone replay at the lock screen ⇒ never unlocked; lid closed / screen blanked ⇒ camera LED off after `idle_timeout`; `presence.disable` ⇒ no unlock; `UnlockSession` accepted from the sandboxed unit (`journalctl -u soos-daemon` shows the unlock line); KDE Plasma lock honoured; *[R3]* three wrong passwords at the lock screen (default `deny = 3`) ⇒ no presence unlock until `unlock_time` (600 s) has passed, then presence unlocks again; `chage -E 0 <user>` (expired) and `passwd -l <user>` ⇒ no presence unlock. |

## 8. Test Hooks for the Tester

| Hook | Purpose |
|---|---|
| `MockPresenceLogind` (in `crates/daemon/tests/common/` or the test file): scripted `seat_sessions` / `session_state` / `lid_closed` results per call, programmable errors and a never-resolving future, an `unlock_calls` counter (`AtomicUsize`) and recorded IDs | PAU4–PAU7, PAU11, PAU14, PAU15 |
| `StaticDisplayProbe(DisplayState)` | PAU9 |
| `PresenceSwitch::new(tempdir)` | PAU4 |
| `PresenceWorker::tick()` + `with_clock_fn` (a test clock: `fn` pointer reading a `static AtomicU64`) | deterministic grace/interval tests (PAU6) without sleeping |
| Existing `MockCameraManager` with a `notify_activity` counter (add a counting wrapper in the test if the mock has none) | PAU13 |
| Existing mock vision backends (`soos-test-fixtures`, mock detector/PAD/extractor as in `template_check_order_tests.rs`) to force live/spoof/match sequences | PAU10, PAU11 |
| `InferenceGate::register_interactive` / `interactive_demand` / `try_acquire_background` | PAU12 |
| `classify_connectors`, `SysfsDisplayProbe::new(tempdir)` with fake `card0-eDP-1/{status,dpms}` files | PAU9 |
| `session_state_from_properties` with hand-built `zvariant::OwnedValue` maps | PAU18 (no bus needed) |
| `RateLimiter::check_and_record_with_reserve` (policy unit + proptest) | PAU3 |
| *[R3]* `StaticAccountGuard(AccountState)`, `ScriptedAccountGuard` (per-call results, optional blocking delay, call counter) | PAU28 |
| *[R3]* `SystemAccountGuard` with tempdir paths (fake `faillock.conf`, `pam.d/`, tally files built from `struct tally` bytes, `shadow`) and an injected `realtime` fn | PAU22–PAU26 |
| *[R3]* Pure `parse_faillock_conf`, `decode_tally_records(&[u8])`, `faillock_denies`, `scan_pam_faillock_options(&str)`, `find_shadow_entry`, `shadow_refusal`, `UserName::parse` | PAU22–PAU27 (proptest reference port for PAU24) |

Proposed test files: `crates/policy/tests/rate_limit_reserve_tests.rs`, `crates/daemon/tests/presence_config_tests.rs`, `crates/daemon/tests/presence_tracker_tests.rs`, `crates/daemon/tests/presence_display_tests.rs`, `crates/daemon/tests/presence_worker_tests.rs`, `crates/daemon/tests/inference_priority_tests.rs`, `crates/daemon/tests/presence_account_tests.rs` *[R3]*, `tests/invariants/src/presence_unlock_contract.rs`. Test names prefixed `test_pau_` / `prop_pau_`.

Invariant contracts (`presence_unlock_contract.rs`):
- C1 `zbus` only in root `[workspace.dependencies]` and `crates/daemon/Cargo.toml`; never in `crates/pam`.
- C2 `crates/daemon/src/presence/mod.rs` and `consensus.rs` start with `#![forbid(unsafe_code)]`; `crates/daemon/src/presence/**` and `consensus.rs` contain no `unsafe` keyword, no `.unwrap()`, no `.expect(`, no `Connection::system` / `Connection::session` and *[R2: F4]* no `Builder::system` / `Builder::session` (both read `DBUS_*_BUS_ADDRESS` from the environment), and `presence/` uses `SYSTEM_BUS_ADDRESS` through `Builder::address`.
- C3 exactly one production call of `.unlock_session(` in `crates/daemon/src` (in `presence/worker.rs`).
- C4 `PadAggregator::with_defaults` appears only in `crates/daemon/src/consensus.rs`; `dispatcher.rs` and `presence/worker.rs` call `run_face_consensus`.
- C5 `RateLimitConfig::DEFAULT_MAX_ATTEMPTS` is 40 and `Docs/DAEMON.md` states `40` for `max_attempts`.
- C6 `DEFAULT_KILL_SWITCH_DIR` in the daemon equals `DEFAULT_FLAG_DIR` in `crates/pam/src/config.rs`.
- C8 *[R3]* `presence/**` contains no write-mode file API and no `faillock --reset` (PAU29).
- C9 *[R3]* `packaging/soos-daemon.service` keeps `CAP_DAC_OVERRIDE`, `ProtectSystem=strict`, `ReadWritePaths=/var/lib/soos /run/soos` and no `InaccessiblePaths=` over the guard's sources (PAU29).
- C7 `AI/DECISIONS.md` contains the ADR title and the accepted-risk keywords (default-on, 40, sudo, LED, presentation) and the guard keywords (`pam_faillock`, `/etc/shadow`, `no tally reset`); `AI/ARCHITECTURE.md` contains invariant 6.

Tests that must fail before the implementation: all of the above (new symbols do not exist; the default is 5). Power checks the tester must include: the grace boundary (`>=`, not `>`), `REMOTE` unknown ⇒ refused (not treated as local), `0` interval rejected (not "immediately"), `unlock_calls == 0` asserted after every failure-path test (not only "no panic").

---

## 9. Documentation Drift / ADR

### 9.1 Drift found

- `Docs/DAEMON.md` §1.4 rate-limit table states `5` (becomes `40`); §3 has no presence row; §1.7 example has no `[presence]`.
- `AI/ARCHITECTURE.md` §2 assumes PAM is the only authorization path (invariant 6 added), §3 diagram, §4 async boundaries (background inference priority), §1 verified crate versions (add `zbus` 5.19.0).
- `AI/MOCK_STRATEGY.md` has no logind mock section.
- `Docs/PACKAGING_AND_PROVISIONING.md` §8.1: state that the unchanged sandbox allows the system bus (AF_UNIX) and sysfs reads, and that no `CAP_SYS_ADMIN` is needed on systemd ≥ 255.
- `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` §4: list `zbus` as an audited daemon-only dependency.
- `tests/physical/screensaver_test.md`: add the PAU21 procedure (including the faillock and expiry steps).
- *[R4: A1]* `Docs/PACKAGING_AND_PROVISIONING.md` §8.1, `Docs/DAEMON.md`, the ADR and the comment above `CapabilityBoundingSet` in `packaging/soos-daemon.service` (comment text only) state that `CAP_DAC_OVERRIDE` is required for the `0660 user:root` faillock tally and for `/etc/shadow` on distributions that ship it `0000 root:root` (Fedora, RHEL); without it presence refuses every unlock.
- *[R4: A2]* ADR accepted risk (g): see §9.2.
- *[R4: A3]* `Docs/DAEMON.md` presence subsection: any user able to hold more than `MAX_SCANNED_SESSIONS` (1024) logind sessions (for example over SSH) makes every tick `TooManySessions`, disabling presence for everyone (fail closed; PAM unaffected).
- *[R4: A4]* `Docs/DAEMON.md` presence subsection: the session owner can set `LockedHint=false` while the locker is up (presence never scans, a self-inflicted denial) or `true` while unlocked (camera on, risk (f)); logind refuses `SetLockedHint` on another user's session, so no other user can do either.
- *[R3]* `Docs/DAEMON.md`: a presence subsection describing the account guard, its sources, the "policy in `faillock.conf`, not on PAM lines" rule and the NSS/homed consequence; `Docs/PACKAGING_AND_PROVISIONING.md` §8.1: `CAP_DAC_OVERRIDE` is now required (faillock tally `0660 user:root`); `Docs/DISTRIBUTION_DEPLOYMENT.md`: same rule for administrators; `AI/ARCHITECTURE.md` invariant 3 clause and invariant 6 text above.
- *[R2: F5]* When the `AI/ARCHITECTURE.md` §3 diagram gains the presence path, the line containing `EvidenceStore (` must keep naming both `PasswordFailed` and `PadFailed` (enforced by `daemon_docs_contract::test_architecture_doc_matches_daemon_code_claims`, which also requires `capture_spoof_evidence` to stay in `dispatcher.rs`); presence adds no evidence reason to that line (D7).

### 9.2 ADR draft (to append to `AI/DECISIONS.md`)

* **[2026-10-02] Presence Auto-Unlock Through logind (GitHub #323, owner decisions 2026-10-02; matrix PAU1–PAU21):** `soos-daemon` unlocks a locked local session when the face of its owner is verified, without any keypress and without PAM, through systemd-logind. (1) *Lock state*: logind does not serialize `LockedHint` into `/run/systemd/sessions/<id>` (verified on systemd 262), so the daemon reads it over the system bus (`ListSessions`, then `Properties.GetAll` on each seat-attached session) with `zbus` 5 (`default-features = false`, `tokio`; MIT; `cargo deny` clean, 22 new crates) connected to the pinned address `unix:path=/run/dbus/system_bus_socket`, never an environment-derived one. (2) *Unlock*: `Manager.UnlockSession(id)`, the call of `loginctl unlock-session`; GNOME, KDE Plasma, Cinnamon and MATE honour it, wlroots lockers need the swayidle `unlock` hook (and a `SetLockedHint` call, since swaylock/hyprlock do not set the hint; without it presence never scans). The unit is unchanged: `AF_UNIX`, `PrivateNetwork` and `ProtectSystem=strict` keep the system bus reachable, and logind ≥ 255 grants a root sender without `CAP_SYS_ADMIN`. Spawning `loginctl` was rejected (a D-Bus client is required for the lock state anyway). (3) *Binding*: only a session passing the `Auth` binding predicate `SessionRecord::check_local_seat_session_of` (owner UID, active, explicit `REMOTE=0`, seat, `CLASS=user`) **and** `LockedHint = true` is eligible; the user-manager "no remote session" rule is not applied because logind already lets any process of the owner unlock its own session. Exactly one eligible, enrolled session (current template binding) may be scanned; zero or several ⇒ no scan. (4) *Pipeline*: the unchanged camera wake + multi-frame PAD consensus (`k = 3` of `n = 5`, any spoof vetoes) + cosine match with the `[pipeline.thresholds]` values, extracted into `consensus::run_face_consensus` and shared with the `Auth` path. (5) *Guards*: no scan or unlock until `[presence] lock_grace_ms` (default 3000 ms, 1000–60000) after the lock is first observed (ticks every 1 s; a lock period ends when the session is seen unlocked, inactive or gone); scans at most every `scan_interval_ms` (default 2000 ms, 1000–60000); no scan while logind reports the lid closed or every connected DRM connector reports DPMS off (an undetectable state does not gate); the `Allow` is single-use and must be followed within 1000 ms by a fresh logind re-check (still bound, locked, same UID) before `UnlockSession`; a locker that ignores the request (still locked 5 s later) is not scanned again until its next lock period; every error, timeout, unknown property or worker panic leaves the session locked. (6) *PAM priority*: one inference slot stays shared; presence never waits for it, yields to any in-flight `Auth` request and is preempted between captures, so a PAM request waits at most one in-flight presence inference. (7) *Enabled by default* (owner decision over opt-in): `[presence] enabled = true` without any `daemon.toml`; disabled by `enabled = false`, by the new flag `/etc/soos/presence.disable`, or by the existing global `/etc/soos/disabled` (re-checked every second, no restart); `gdm.disable` does not stop presence; the harness mode `enforce_active_session = false` never starts it. (8) *Rate limit* (owner decision over a separate quota): `RateLimitConfig::DEFAULT_MAX_ATTEMPTS` rises from 5 to 40 per 60 s for every face request including `sudo`, GDM and presence scans (one attempt per scan); presence never consumes the last 5 attempts of the window (`PRESENCE_RESERVED_ATTEMPTS`), so PAM keeps at least the former budget; a host that pinned a lower `max_attempts` gets a startup warning and fewer (or no) scans. (9) *Evidence* (owner answer Q2, 2026-10-02): presence spoof vetoes are logged, never sealed as evidence. (10) *Account guard* (owner answer Q1, 2026-10-02 — supersedes the former accepted risk "account phase not consulted"): before every scan and again in the fresh re-check that precedes `UnlockSession`, the daemon refuses the unlock when the owner (logind `Name`, never a payload value) is locked by `pam_faillock` (tally `/run/faillock/<user>` or the `faillock.conf` `dir`, 64-byte `struct tally` records read under a shared non-blocking `flock`, the exact `check_tally` rule with `faillock.conf` or the built-in defaults 3 / 900 s / 600 s), when `/etc/shadow` shows the account expired or inactive, the password expired, a forced change (`lastchg = 0`) or a locked password (`!`/`*`), for UID 0, and whenever the state cannot be determined: missing or duplicated shadow line (NSS-only and systemd-homed users therefore never get presence unlock), any read, size, parse or clock problem, an unknown `faillock.conf` key, or any `pam_faillock.so` line in `/etc/pam.d`, `/usr/lib/pam.d` or `/usr/etc/pam.d` that sets policy options (`deny=`, `dir=`, `fail_interval=`, `unlock_time=`, `root_unlock_time=`, `admin_group=`, `conf=`, `even_deny_root`) — administrators keep faillock policy in `faillock.conf`. With `admin_group` set the stricter unlock time applies (no group lookup). The unit directives are unchanged; `CAP_DAC_OVERRIDE` (already granted) is now required to read the `0660 user:root` tally and, on Fedora/RHEL where it is `0000 root:root`, `/etc/shadow` *[R4: A1]*. PAM stack files are read following symlinks like libpam (authselect), a dangling or non-regular target being undeterminable; the tally is never followed and its directory must be a non-symlink, not group/other-writable *[R4: A5]*. The kill switch is re-checked right before `UnlockSession` *[R4: A6]*. A presence unlock never resets the tally (it is not a PAM authentication, the guard already refused while locked, and keeping sub-threshold failures is conservative), unlike the Arch PAM face path whose `authsucc` runs inside PAM. Invariant 3 gains the presence clause. (11) *`gdm.disable`* (owner answer Q3): does not stop presence; `/etc/soos/presence.disable` and `/etc/soos/disabled` do. **Accepted risks (owner, 2026-10-02)**: (a) default-on means every enrolled user's locked screen is scanned after installation, with the camera LED on and ~15 % of one CPU core used while a session is locked and the screen is on; (b) the attack window grows: an unattended locked screen offers up to ~30 presentation attempts per minute with no keypress, and every face path (including `sudo`) now allows 40 attempts per minute instead of 5, so the false-accept exposure over `t` minutes is about `1 − (1 − FMR)^(40·t)` instead of `^(5·t)` and PAD carries the whole defence against photos, screens and masks; (c) locking while seated unlocks again after the grace period, by design; (d) the unlock bypasses the locker's PAM stack: `pam_faillock` and shadow expiry are honoured by the account guard (10), but other account modules (`pam_nologin`, `pam_access`, `pam_time`, `pam_tally2`) are not consulted for a session that is already open, and the user (or malware running as the user) can truncate its own `0660` tally file, exactly as against PAM; (e) a lock period that ends and restarts between two 1 s ticks, or during one scan (at most ~3 s), is not seen as a new period, so its grace is skipped (only someone who just unlocked the session, i.e. the owner or a person who knew the password, can cause this); (f) any process of the session owner can set `LockedHint` and thereby make the daemon scan (camera on), which grants nothing that owner could not already do (`loginctl unlock-session`); conversely the owner can set it `false` under a running locker and so only deny presence to itself, and logind refuses `SetLockedHint` from any other user *[R4: A4]*; (g) *[R4: A2]* the system-bus default policy lets any local user send unicast signals to the daemon's unique name: zbus reads each such message (bounded by the bus message limit) and drops it because no match or stream is registered — a transient allocation, nothing queued; (h) *[R4: A3]* any user able to open more than 1024 logind sessions (for example over SSH) makes every tick `TooManySessions` and disables presence for everyone until they close (fail closed, PAM unaffected). Walkthrough NN (next number).

---

## 10. Owner Answers (2026-10-02, binding) *[R3]*

- **Q1 (account phase)**: respect `pam_faillock` and account expiry; locked, expired or undeterminable ⇒ no unlock. Implemented by §2.6 `presence/account.rs`, tick steps 7 and 13, PAU22–PAU29; former accepted risk (d) narrowed accordingly.
- **Q2 (evidence)**: keep the default — presence never seals spoof evidence (D7).
- **Q3 (`gdm.disable`)**: keep the default — `gdm.disable` does not stop presence; `/etc/soos/presence.disable` and `/etc/soos/disabled` do (D9).
