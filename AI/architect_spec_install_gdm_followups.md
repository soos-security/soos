# Architect Spec — GitHub #331: install follow-ups, physical procedure fixes, remove GDM double face verification

- **Issue**: GitHub #331 (GitHub-only, no backlog id; the branch `fix/install-gdm-followups` is **not**
  registered in `scripts/sync_issue.py`; the squash commit carries `Closes #331`).
- **Base**: `origin/main` at `b05477d` (PR #330 merged).
- **Sources read**: issue #331, PR #330 and #322 bodies, walkthrough 178, `crates/admin-cli/src/{gdm,pam_stack,status,main}.rs`,
  `scripts/{install,wait_daemon_ready}.sh`, `crates/daemon/src/presence/{mod,worker}.rs`, `crates/daemon/src/{inference,consensus}.rs`,
  `crates/camera-v4l/src/{manager,mock,v4l_impl}.rs`, `packaging/pam/**`, `tests/docker/pam_rollback_test.sh`,
  `tests/distro/fedora_rhel_test.sh`, `tests/physical/screensaver_test.md`, the existing contract suites listed in §10.
- **Owner decisions (final)**: §3 of the issue as written (no managed block when the delegated stack already reaches an
  active primary `pam_soos.so` rule; an existing redundant block is removed; GDM then uses the shared rule and its deadline,
  1000 ms module default, instead of `timeout_ms=2500`; `gdm status` reports installed). All six §1 items and both §2 items.
- **Matrix component**: `install-gdm-followups`, prefix **IGF** (free: not used by any existing row).

---

## 1. Scope & Blast Radius

| Area | Files | Change |
|---|---|---|
| GDM integration (§3) | `crates/admin-cli/src/pam_stack.rs`, `crates/admin-cli/src/gdm.rs`, `crates/admin-cli/src/main.rs` (table output only) | Shared-rule detection, block removal, status |
| Admin status (P-2) | `crates/admin-cli/src/status.rs` | Bounded `systemctl show` |
| Camera trait (presence settle) | `crates/camera-v4l/src/manager.rs`, `crates/camera-v4l/src/v4l_impl.rs`, `crates/camera-v4l/src/mock.rs` | New defaulted trait method + V4L stamp + mock setter |
| Presence settle | `crates/daemon/src/presence/mod.rs`, `crates/daemon/src/presence/worker.rs` | Settle window keyed on stream start |
| Installer (P-1, preflight) | `scripts/install.sh` | Target dir resolution, read-only files, unreadable subtree |
| Readiness helper | `scripts/wait_daemon_ready.sh` | Documented bound, last stderr, fail fast 126/127 |
| Docs | `AI/ARCHITECTURE.md` (line ~130 and the §5 GDM paragraph ~177-182), `README.md`, `Docs/PACKAGING_AND_PROVISIONING.md`, `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1, `Docs/DAEMON.md`, `Docs/CAMERA_V4L_CRATE.md`, `tests/physical/screensaver_test.md` §3.3 + §6, `AI/DECISIONS.md`, `.claude/skills/dev-workflow/references/project-facts.md` | See §9 |

**Not touched** (must stay byte-identical, auditor to check with `git diff --exit-code origin/main -- …`):
`crates/pam`, `crates/protocol`, `crates/daemon/src/dispatcher.rs`, `crates/daemon/src/consensus.rs`,
`crates/daemon/src/inference.rs`, `crates/daemon/src/config.rs`, `crates/policy`, `packaging/pam/**`,
`tests/docker/**`, `tests/distro/**`.

**Consumers of changed public items**

- `CameraManager` (new **defaulted** method, no breaking change): implementors `MockCameraManager`, `V4lCameraManager`
  (override), GUI `IpcCameraManager`, `SwitchableCamera`, `UnavailableCameraManager` and every test camera
  (`SpyCamera`, `FixedFrameCamera`, `SlowSwapCamera`, `MinimalCamera`, `FakeCamera`) keep compiling unchanged and return `None`.
- `GdmStatus` (new field `shared_stack`): constructed only in `crates/admin-cli/src/gdm.rs`; re-exported by `lib.rs`;
  serialized by `main.rs`. No test builds it by literal or compares its JSON (checked: `grep -rn GdmStatus crates tests`).
- `pam_stack::delegated_gates` (pub(crate)) is replaced; only caller is `gdm.rs`.
- No script calls `soos-admin gdm enable` (`install.sh`, `uninstall.sh`, packaging scriptlets, Docker and distro suites were
  checked). `tests/docker/pam_rollback_test.sh` *simulates* an enable with `sed` (legacy `sufficient` line before
  `@include common-auth`) to test `uninstall.sh` restores; that simulation stays valid (it models a pre-#331 install) and is
  unchanged. `tests/distro/fedora_rhel_test.sh` uses static `test-*` service files: unaffected.

---

## 2. GDM double face verification (issue §3)

### 2.1 Current behaviour and drift found

`plan_gdm_enable` scans the delegated stack with `scan_lines`; a `pam_soos.so` rule met before the credential module is
neither a credential, a gate nor neutral, so it is **unclassified and `enable` refuses**. Consequently, on the owner's Arch
host the managed block exists only because `gdm enable` ran **before** the `system-auth` edit; re-running `enable` today
refuses (`unclassified auth rule 'pam_soos.so'`). The issue text ("inserts a managed block … while `system-auth` already
carries the primary soos rule") describes the resulting state, not what `enable` does on that state. The tester should pin
this with a red test (packaged Arch `system-auth` → today `Err`, after the change `Ok` without block).

### 2.2 Definition: active primary `pam_soos.so` rule

New method in `crates/admin-cli/src/pam_stack.rs`:

```rust
impl PamLine<'_> {
    /// True for an active `auth` rule of `pam_soos.so` (bare name or path) that can grant the
    /// whole auth phase on a face match and is driven by `PAM_SERVICE` (GitHub #331):
    /// - no argument starting with `event=` (the password-failed hook is not primary) and no
    ///   argument starting with `service=` (the `gdm.disable` flag must keep applying);
    /// - control `sufficient` (case-insensitive), or a bracketed control whose `success` value
    ///   is `done` or a decimal jump `N >= 1`, and whose every other `key=value` action is
    ///   `ignore` (keys and values compared case-insensitively).
    /// Anything else (`required`, `requisite`, `optional`, `success=ok`, `default=die`, …) is
    /// not primary. The caller has already checked `is_auth()`.
    pub(crate) fn is_primary_soos_rule(&self) -> bool;
}
```

Bounds/edges: `success=0`, `success=-1`, `success=` (empty), a bracket without `success`, duplicate `success` keys with
conflicting values → **not primary**. Packaged rules that must qualify: Arch `auth  [success=4 default=ignore]  pam_soos.so`,
Debian/pam-auth-update `[success=done default=ignore] pam_soos.so` (and a rewritten `[success=N default=ignore]`),
Fedora `auth [success=done default=ignore] pam_soos.so`, hand-written `auth sufficient pam_soos.so`.

### 2.3 Scan outcome

```rust
/// Outcome of scanning an auth stack.
pub(crate) enum Scan {
    Continue,                 // unchanged
    Stop,                     // unchanged: a credential module was reached first
    SharedSoos(String),       // NEW: a primary pam_soos.so rule was reached first; payload = stack name
}

/// What the auth stack delegated from the GDM file runs before its first credential module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DelegatedAuth {
    /// Gate rules to copy in front of the managed block (former `delegated_gates` result).
    Gates(Vec<String>),
    /// The delegated stack reaches an active primary `pam_soos.so` rule (in the named stack
    /// file) before any credential module: GDM already authenticates through it.
    SharedSoosRule { stack: String },
}

/// Replaces `delegated_gates`; same arguments, same refusals, same depth bound.
pub(crate) fn delegated_auth(dir: &Path, lines: &[&str]) -> Result<DelegatedAuth, AdminCliError>;
```

Classification order inside `scan_lines` (first decisive rule wins, evaluation order, includes followed recursively up to
`MAX_PAM_INCLUDE_DEPTH` = 4 exactly as today): continuation → refuse; non-auth → skip; delegation → recurse
(`Stop` or `SharedSoos` propagate immediately); **credential → `Stop`**; **primary soos → `SharedSoos(<stack name>)`** (new);
gate → collect; neutral → skip; anything else → unclassified refusal (unchanged message). A non-primary `pam_soos.so` rule
before the credential module therefore keeps today's unclassified refusal (fail closed, no new message). Rules after the
shared rule are never inspected. `stack` is the include target name in which the rule was found (e.g. `system-auth`); a
primary rule in the edited file's own tail is unreachable (the pristine content has no soos rule) and must not panic.
`Scan::Continue` at the end keeps the existing "reaches no known credential module" refusal.

### 2.4 `gdm enable` (`plan_gdm_enable` / `ensure_gdm_pam_line`)

```rust
/// Result of `plan_gdm_enable` when the file must be rewritten.
enum EnablePlan {
    /// Insert the managed block (current behaviour).
    Insert { pristine: String, updated: String },
    /// GitHub #331: the delegated stack already reaches a shared primary soos rule; write the
    /// file without its managed rules. Never creates a backup.
    RemoveRedundant { updated: String },
}
```

Algorithm (only the marked steps are new; error texts and their priority are unchanged):

1. Continuation check (unchanged). `pristine = strip_managed_rules(content)?` (unchanged; removes the managed block, the
   bare `GDM_PAM_LINE` and `LEGACY_GDM_PAM_LINE`). If `has_active_pam_soos_rule(&pristine)` → `Ok(None)` (administrator rule,
   untouched — even if the delegated stack also has a shared rule).
2. Anchor scan of `pristine` (unchanged: pre-credential allow-list, unclassified refusal, no-anchor refusal, jump list).
3. **NEW**: if the anchor delegates, compute `delegated = delegated_auth(include_dir, lines[anchor..])` **without `?`**.
   If it is `Ok(SharedSoosRule { .. })`: return `Ok(None)` when `pristine == content` (nothing is inserted or removed, so
   no jump target can move; the jump check is not applied); otherwise apply the jump check of step 4 (refusal with the
   existing jump error, nothing written, backup untouched) and then return
   `Ok(Some(EnablePlan::RemoveRedundant { updated: pristine }))`. *Correction (candid review 2026-10-05, MAJOR)*: removing
   the managed rules shifts every `[...=N]` jump that crosses them; `enable` never inserts a block across a crossing jump,
   so a crossing jump that coexists with a block was written with the block rules counted, and removing the block would
   retarget it (possibly past the delegation). The earlier claim that removal "restores the administrator's original
   targets" was wrong.
4. Jump-crossing check (unchanged message, unchanged position relative to the anchor-scan errors).
5. `gates = match delegated { Some(r) => r? (Gates(v) → v), None => vec![] }` — a delegated-scan error surfaces here, i.e.
   after the jump check, exactly as today.
6. Block construction, `updated == content → Ok(None)`, `Ok(Some(EnablePlan::Insert { .. }))` (unchanged).

`ensure_gdm_pam_line` write semantics:

| Plan | Backup (`<file>.soos-backup`) | PAM file |
|---|---|---|
| `None` | untouched | untouched (no write, no temp file) |
| `Insert` | created from `pristine` only when absent (unchanged) | `write_atomic(updated)` (unchanged) |
| `RemoveRedundant` | **never created; an existing backup is left byte-identical** | `write_atomic(updated)` with the file's own mode `& 0o7755` and owner (same as `Insert`) |

Rationale for keeping, not deleting, an existing backup: after `RemoveRedundant` the file equals `pristine`; when the
backup also equals `pristine` (no edit since the first enable), `gdm restore` restores identical bytes, the backup's mode and
owner, and removes the backup (works, idempotent); when the administrator edited the file after the first enable, the
backup is stale, the edit is preserved by the removal, and `gdm restore` refuses without `--force` exactly as today
(GitHub #312). Deleting the backup would make `gdm restore` fail with "does not exist" and lose the stale-edit protection.
`uninstall.sh` restoring the backup is harmless in both cases. When no managed block ever existed (fresh install with the
shared rule), no backup exists and `gdm restore` fails with the existing "PAM backup '…' does not exist" (nothing to restore;
documented, not an error path to change).

The disable flag is removed after a successful `ensure_gdm_pam_line` in every case (unchanged): with the shared rule GDM
face verification is re-enabled through it, because the shared rule has no `service=` and reads `PAM_SERVICE`
(`gdm-password`), so `gdm disable` / `gdm.disable` keep working.

### 2.5 `gdm status`

```rust
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GdmStatus {
    pub installed: bool,
    pub enabled: bool,
    pub pam_file: PathBuf,
    pub disable_file: PathBuf,
    /// GitHub #331: name of the delegated stack file whose primary `pam_soos.so` rule GDM
    /// uses when the service file carries no soos rule of its own; `None` otherwise.
    pub shared_stack: Option<String>,
}
```

`get_gdm_status`: read the file bounded as today. `direct = has_active_pam_soos_rule(&content)` (unchanged).
If `!direct`, `shared_stack = shared_soos_stack(&content, include_dir)`; `installed = direct || shared_stack.is_some()`;
`enabled = installed && !disabled` (unchanged). Unreadable/oversized/non-UTF-8 file → `installed: false, shared_stack: None`.

```rust
/// Name of the stack holding the shared primary soos rule reached by `content`'s auth stack, using
/// exactly the anchor scan of `plan_gdm_enable` (steps 1–3) and `delegated_auth`. `None` on any
/// refusal condition (continuation lines, unclassified rule, no anchor, non-delegating anchor,
/// unreadable/missing include, depth exceeded) — fail closed, never an error to the caller.
fn shared_soos_stack(content: &str, include_dir: &Path) -> Option<String>;
```

**Single source of truth**: `plan_gdm_enable` and `shared_soos_stack` must share one anchor-scan helper (no second copy of
the pre-credential loop). Consequences (documented in `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1, no code to prevent them):

- **False negative**: on a stack where `enable` refuses (e.g. an unclassified rule before the shared rule) `status` reports
  `installed: false` although libpam would reach the shared rule; `enable` names the reason.
- **False positive (a)**: `is_primary_soos_rule` accepts any jump `success=N` (N >= 1) without checking where it lands. A
  shared rule such as `[success=1 default=ignore] pam_soos.so` followed by `pam_unix.so` and `required pam_deny.so` lands on
  `pam_deny`, so GDM face login never succeeds, yet `enable` succeeds without a block and `status` reports `installed: true`.
- **False positive (b)** *(removed by the candid review 2026-10-05 follow-up)*: `shared_soos_stack` now returns `None` when a
  `[...=N]` jump before the GDM anchor lands beyond the delegation line (it would skip the shared rule on that branch), so
  `status` reports `installed: false` there; a jump landing exactly on the delegation still counts (the shared rule runs).
- Both false positives fail closed (nothing is written, the password path is unchanged, no `PAM_SUCCESS` is created); they
  only make face login unavailable on a mis-built stack. Operators confirm with the physical procedure (§3.3 of
  `tests/physical/screensaver_test.md`: one `Rendered authentication response` line per attempt).
`main.rs` table output: one new line `  Shared soos Rule:  <stack>` printed only when `shared_stack` is `Some`; JSON gains
`"shared_stack": null | "<name>"`.

### 2.6 Latency

GDM through the shared rule: PAM deadline = the shared rule's clamped `timeout_ms` (module default
`DEFAULT_TIMEOUT_MS` = 1000 ms when absent; Arch/Debian/Fedora packaged rules set none) instead of 2500 ms. Daemon side
unchanged (`DECISION_BUDGET_MS` 900 ms + write margin within `connection_timeout_ms` 2500 ms). A GDM unlock that needs a
camera wake from auto-standby may now exceed 1000 ms and fall back to the password (accepted trade-off, owner decision).
`GDM_PAM_LINE` keeps `timeout_ms=2500` for stacks without a shared rule (QFU5 needles stay valid).

---

## 3. Presence wake settle keyed on the camera stream start (issue §1 item 4)

### 3.1 Camera trait (`crates/camera-v4l/src/manager.rs`)

```rust
pub trait CameraManager: Send + Sync {
    // ... existing methods unchanged ...

    /// CLOCK_MONOTONIC instant, in nanoseconds (the domain of `Frame::timestamp_mono_ns`), at
    /// which the current capture stream was set up (buffers mapped), immediately before its first
    /// dequeue, which issues `VIDIOC_STREAMON` (v4l 0.14 starts the stream lazily); `None` when it
    /// is unknown or the camera is not ready (GitHub #331). Never later than the stamp of any
    /// frame of that stream. Used only to settle the sensor's
    /// auto-exposure before presence evaluation; never by the PAM path.
    fn stream_started_mono_ns(&self) -> Option<u64> {
        None
    }
}
```

### 3.2 V4L implementation (`crates/camera-v4l/src/v4l_impl.rs`)

- New shared `stream_started_ns: Arc<AtomicU64>` (0 = unknown) in `V4lCameraManager` and `SupervisorShared`.
- `open_and_stream` stores `monotonic_nanos()` (`Ordering::Release`) **immediately after `start_stream` returns `Ok`**, before
  `run_capture_loop`. Note (plan evaluation finding 2): `start_stream` (`mmap_stream_guarded`) only does REQBUFS + mmap;
  v4l 0.14 issues `VIDIOC_STREAMON` lazily inside the first `CaptureStream::next()` (first `next_buffer` of
  `capture.rs::run_capture_loop`). The stamp therefore precedes STREAMON by microseconds, i.e. it is the stream set-up
  instant, never later than any frame stamp of that stream (frames are stamped at dequeue). The few microseconds
  between set-up and STREAMON shorten the settle by that amount only, which is irrelevant at 1000 ms. A clock failure stores 0.
- `SupervisorShared::withdraw_frames` also stores 0 (suspend, error, shutdown, panic paths).
- Override: `fn stream_started_mono_ns(&self) -> Option<u64>` returns `None` when `!self.is_ready()` or the stored value is 0,
  else `Some(value)` (`Ordering::Acquire`).
- `open_and_stream` already has 7 parameters (`clippy::too_many_arguments` limit): pass the shared state as
  `&SupervisorShared` instead of adding an 8th parameter (private function; `supervisor_tests.rs` uses `spawn_inner` only).
  No new `unsafe` (reuse `monotonic_nanos`).

### 3.3 Mock camera (`crates/camera-v4l/src/mock.rs`)

`MockCameraManager` gains a test hook `pub fn set_stream_started_mono_ns(&self, ns: Option<u64>)` (stored in an `AtomicU64`,
0 = `None`) and overrides the trait method to return it **regardless of readiness**. Default `None`, so the Docker mock-camera
daemon and every existing test keep today's behaviour.

### 3.4 Presence settle window (`crates/daemon/src/presence/mod.rs`)

```rust
/// Evaluation lower bound of one presence scan (GitHub #329, #331).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresenceSettle {
    /// Captures stamped before this CLOCK_MONOTONIC instant are not evaluated (0: no bound).
    pub not_before_ns: u64,
    /// Settle time still to wait after `start_ns`, rounded up to whole ms; `0..=settle_ms`.
    pub wait_ms: u64,
}

/// Computes the settle window of a scan that starts evaluating at `start_ns`.
/// - `woke`: this scan woke the camera (sampled before `notify_activity`, as today);
/// - `stream_started_ns`: `CameraManager::stream_started_mono_ns()` read after the wake;
///   `Some(0)` is `None`; a value later than `start_ns` is clamped to `start_ns`.
/// `bound = max(woke ? start_ns + settle : 0, stream ? stream + settle : 0)` with saturating
/// arithmetic (`settle = settle_ms * 1_000_000`, saturating). If `bound <= start_ns` the result is
/// `{ not_before_ns: 0, wait_ms: 0 }`, else `wait_ms = ceil((bound - start_ns) / 1_000_000)`.
#[must_use]
pub fn presence_settle_window(
    woke: bool,
    start_ns: u64,
    stream_started_ns: Option<u64>,
    settle_ms: u64,
) -> PresenceSettle;
```

Properties (all testable): `settle_ms == 0` → `{0, 0}`; `woke` with no stream stamp → exactly today's bound
`start + settle` and `wait_ms == settle_ms`; streaming camera whose stream started more than `settle_ms` ago → `{0, 0}`
(IWP10 unchanged); camera streaming for `d < settle_ms` (stream started by a PAM request) → bound `stream + settle`,
`wait_ms = ceil(settle - d)`; `wait_ms <= settle_ms` always; `not_before_ns > start_ns` iff `wait_ms > 0`; no overflow panic
at `u64::MAX` inputs. Taking the maximum keeps the presence-caused wake at least as conservative as #329.

### 3.5 Worker (`crates/daemon/src/presence/worker.rs`)

Step 11/12 become: `woke = !camera.is_ready()` before `notify_activity()` (unchanged) → `wake_camera` (unchanged) →
`start_ns = clock_fn()` (unchanged) → `stream = camera.stream_started_mono_ns()` → `settle =
presence_settle_window(woke, start_ns, stream, PRESENCE_WAKE_SETTLE_MS)` → deadline
`RequestDeadline::compute(settle.not_before_ns.max(start_ns), 0, Instant::now(),
Duration::from_millis(DECISION_BUDGET_MS + RESPONSE_WRITE_MARGIN_MS + settle.wait_ms))` (saturating adds)
`.with_not_before(settle.not_before_ns)`. One `debug!` when `wait_ms > 0` with fields `settle_ms = wait_ms` and
`cause = "presence_wake" | "stream_start"` (static strings; `presence_wake` when `woke`). No frame, embedding, uid-to-name or
template data logged. `PRESENCE_WAKE_SETTLE_MS` (1000) and `with_not_before(` stay literally in `worker.rs` (IWP11).

**Unchanged (auditor to verify):** dispatcher (PAM path never reads `stream_started_mono_ns`, never calls
`with_not_before`), `DAEMON_DEFAULT_WARMUP_FRAMES = 0`, PAD thresholds, `run_face_consensus` and its skip rule, rate-limit
accounting (the attempt is still recorded before the scan).

**Latency budget (presence only):** worst-case window `DECISION_BUDGET_MS` 900 + `RESPONSE_WRITE_MARGIN_MS` 50 +
`wait_ms` ≤ 1000 = 1950 ms ≤ `scan_interval_ms` default 2000 ms (same maximum as #329).

**Residual:** a stream restarted (recovery) between the stamp read and the consensus is not re-read; frames of the
restarted stream are withdrawn until ready, and the next scan uses the new stamp.

---

## 4. Installer (`scripts/install.sh`)

### 4.1 P-1 — one resolution of the cargo target directory

Defined once, right after `WORKSPACE_ROOT` and option parsing, **before** `DEFAULT_ARTIFACT_DIR`:

```bash
# The cargo target directory as cargo sees it from the checkout (GitHub #331, P-1).
resolve_cargo_target_dir() {
    local dir="${CARGO_TARGET_DIR:-}"
    if [[ -z "${dir}" ]]; then
        printf '%s' "${WORKSPACE_ROOT}/target"
    elif [[ "${dir}" == /* ]]; then
        printf '%s' "${dir}"
    else
        printf '%s' "${WORKSPACE_ROOT}/${dir}"
    fi
}
BUILD_TARGET_DIR="$(resolve_cargo_target_dir)"
DEFAULT_ARTIFACT_DIR="${BUILD_TARGET_DIR}/release"
```

- Empty `CARGO_TARGET_DIR` = unset (as today). No canonicalization, no symlink resolution (lexical join only).
- The later `BUILD_TARGET_DIR="${CARGO_TARGET_DIR:-${WORKSPACE_ROOT}/target}"` line is removed; `check_build_target_dir`
  uses the same variable (its `./` prefix branch becomes unreachable and may stay).
- The build gets the resolved path explicitly, so a login profile or a `build.target-dir` cargo config can no longer make
  cargo build elsewhere than where the installer checks and copies artifacts:
  - non-root path: `CARGO_TARGET_DIR="${BUILD_TARGET_DIR}" SOOS_BINDIR="${PREFIX}/bin" "${BUILD_CMD[@]}"` (the AFC7 literal
    `SOOS_BINDIR="${PREFIX}/bin" "${BUILD_CMD[@]}"` stays a substring);
  - root path: `SOOS_CARGO_TARGET_DIR="${BUILD_TARGET_DIR}" runuser -u "${build_user}" -- bash -lc 'cd "$1" && ./scripts/check_build_deps.sh && CARGO_TARGET_DIR="${SOOS_CARGO_TARGET_DIR}" SOOS_BINDIR="$2" cargo build --release --locked --workspace' _ "${WORKSPACE_ROOT}" "${SOOS_BUILD_BINDIR}"`
    (positional arguments unchanged: AFC7 requires the logged runuser line to contain
    `SOOS_BINDIR="$2" cargo build --release --locked --workspace` and to end with the bindir; `runuser -u` without `-l`
    preserves the environment variable; the inner shell re-asserts it after the login profile).
- Help text of `--artifact-dir`: default "`<target>/release`, where `<target>` is `$CARGO_TARGET_DIR` (a relative value is
  resolved against the checkout) or `<checkout>/target`".
- `--build` + `--artifact-dir` error message unchanged (it prints the resolved path).

### 4.2 Build-directory preflight (`check_build_target_dir`)

Unchanged: absent target → ok; exists but not a directory → refuse; user resolution and the uid-0 / unresolvable skips;
`find -H`; `printf %q` for every untrusted string; `preflight_fail_verbatim`; read-only (no `chmod`/`chown` executed).
New ordered checks; each `find` runs independently with `2>/dev/null` and `-print -quit`; the **first hit in this priority
order** produces exactly one failure:

| # | Predicate (`find -H "${target}" … -print -quit`) | Failure lines (exact) |
|---|---|---|
| 1 | `! -uid "${uid}"` (unchanged) | unchanged: `… holds files not owned by the build user '<u>' (first: <hit>), e.g. left by an earlier root build; cargo would fail with 'Permission denied'.` / `Fix it with: sudo chown -R <u>: <target>` |
| 2 **new** | `\( -type d ! -perm -u+rx \) -o \( ! -type d ! -type l ! -perm -u+r \)` | `The cargo target directory <target> holds an entry the build user '<u>' cannot read (first: <hit>).` / `Fix it with: chmod -R u+rwX <target>` |
| 3 **extended** | `! -type l ! -perm -u+w` (was `-type d ! -perm -u+w`) | directory hit: unchanged `… holds a directory the build user '<u>' cannot write (first: <hit>).`; other hit: `… holds a file the build user '<u>' cannot write (first: <hit>).`; both followed by unchanged `Fix it with: chmod -R u+w <target>` |
| 4 | no hit, but any of the `find` runs exited non-zero | `The cargo target directory <target> cannot be fully inspected for the build user '<u>' (find failed).` / `Fix it with: sudo chown -R <u>: <target> && chmod -R u+rwX <target>` |

Notes: as a non-root runner, check 1 fails (rc 1) on an unreadable user-owned directory without a hit, and check 2 still
sees that directory's own entry in its readable parent, so the operator gets the `chmod` advice instead of the former wrong
`chown` advice; as root, check 2 is purely mode-based. `-perm` with symbolic `u+…` is the GNU/busybox form already used.
A read-only file under a writable directory rarely breaks cargo (rename over it works), but the owner asked for the check;
the fix is harmless.

---

## 5. Readiness helper (`scripts/wait_daemon_ready.sh`)

### 5.1 Real bound (documentation only, behaviour unchanged)

Derivation (attempt bound active): the stop test `SECONDS - start >= T + 1` is evaluated after each failed attempt; it is
false at most when the real elapsed time is `< T + 1` (integer `SECONDS`, both stamps truncated); then `sleep 0.5`, then one
more attempt bounded by `timeout --kill-after=1 5` (≤ 6 s), then the test is true. Worst case ≈ **`T + 7.5 s`** (plus
process start-up). The socket wait shares `start` and is bounded by `T` polls of 0.5 s. Without a usable
`timeout --kill-after`, a hanging attempt is unbounded (degraded mode, already warned). Required text:

- script header comment and `usage()`: "`--timeout` bounds the socket wait and the start of the last status attempt; the
  worst case is about `--timeout` + 7.5 s (1 s `SECONDS` granularity, 0.5 s poll, one 5 s attempt plus 1 s kill grace).
  Without coreutils `timeout --kill-after` an attempt is not bounded."
- `README.md`: replace "waits at most 30 s" by "waits up to its `--timeout` (default 30 s; worst case about 37.5 s)".
- `Docs/PACKAGING_AND_PROVISIONING.md` "Start and readiness" bullet: same statement (`T + 7.5 s`, 37.5 s with the default).

### 5.2 Last stderr and fail fast

- Each attempt captures **stdout, stderr and the exit status separately, in memory** (no temporary file, no write anywhere,
  no pipe that hides the status; constraint C11/C15 of #329 kept). Recommended technique: inside one command substitution,
  run the attempt as `out="$(… 2>&3)" || rc=$?` within `{ …; } 3>&1`, then append `$'\037'"${rc}"$'\037'"${out}"`; parse
  from the right (`out` = after the last `\037`, `rc` = between the last two, `err` = the rest). Valid `soos-admin` JSON never
  contains a raw `\037`; a report that does is treated as no report.
- **Exit 126 or 127** of an attempt (binary not executable / not found, also what coreutils `timeout` returns when it cannot
  run the command): stop at once, print no report, write to stderr (via `printf '%s\n'`, `--admin` value through `printf %q`):
  - `[ERROR] Cannot run '<admin>' (exit 126: found but not executable); readiness cannot be checked.` or
    `[ERROR] Cannot run '<admin>' (exit 127: not found); readiness cannot be checked.`
  - `        Pass --admin <PATH> with the installed soos-admin binary.`
  - the sanitized stderr tail of that attempt (rules below), if non-empty;
  - exit **1** (so `install.sh` keeps its exit 70 path; exit 2 stays reserved for usage errors).
- **Killed attempt (124/137)**: contributes no report (unchanged) and sets the last error to
  `status attempt killed after 5 s (no answer)`.
- **Other failures**: a non-empty stderr replaces the last error; an empty one leaves it unchanged.
- **Timeout message**: the two existing lines stay verbatim and in place (`[ERROR] soos-daemon did not report healthy within
  <T> s ('<admin> status' kept failing).` and `        Inspect: soos-admin status; journalctl -u soos-daemon.service -n 50`);
  when a last error exists, after the first line print `        Last '<admin> status' error:` followed by the tail, each line
  prefixed with 8 spaces.
- **Sanitization / bound**: `readonly ADMIN_STDERR_TAIL_MAX=1024`; keep the **last** 1024 characters; replace every control
  character except newline by `?` (`${err//[[:cntrl:]]/?}` after protecting `\n`, or equivalent); never `echo -e`.
- Exit codes 0/1/2, the success output (exactly one JSON document on stdout), the missing-manifest fast failure and the
  per-attempt `timeout` probe are unchanged.

---

## 6. P-2 — bounded `systemctl show` (`crates/admin-cli/src/status.rs`)

```rust
/// Upper bound of the `systemctl show` call made by `soos-admin status` (GitHub #331, P-2).
pub const SYSTEMCTL_SHOW_TIMEOUT_MS: u64 = 1000;
/// Bytes of `systemctl show` output read at most (three properties need < 200 bytes).
const MAX_SYSTEMCTL_OUTPUT_BYTES: u64 = 4096;
/// Poll interval of the child while waiting for it.
const SYSTEMCTL_POLL_INTERVAL_MS: u64 = 10;

fn inspect_systemd_unit(unit_name: &str) -> (String, String, Option<u32>) {
    inspect_systemd_unit_with(OsStr::new("systemctl"), unit_name,
        Duration::from_millis(SYSTEMCTL_SHOW_TIMEOUT_MS))
}

/// Test seam: runs `<program> show <unit> --property=ActiveState,SubState,MainPID` with stdin and
/// stderr null and stdout piped, polls `try_wait` every `SYSTEMCTL_POLL_INTERVAL_MS` until
/// `timeout`; on expiry `kill()` then `wait()` (the child is always reaped) and returns
/// `("unknown", "unknown", None)`. On a non-zero exit, a spawn error, a wait error or output
/// above `MAX_SYSTEMCTL_OUTPUT_BYTES`: the same unknown triple. Parsing is unchanged.
pub(crate) fn inspect_systemd_unit_with(program: &OsStr, unit_name: &str, timeout: Duration)
    -> (String, String, Option<u32>);
```

Arguments are unchanged (`show`, unit, `--property=…`). **Bounded read (plan evaluation finding 6)**: stdout is drained
on a helper `std::thread` (`take(MAX_SYSTEMCTL_OUTPUT_BYTES + 1)`, result sent over a `std::sync::mpsc` channel) started right
after the spawn, so a child that fills the pipe never blocks on it; the caller waits on `try_wait` until the deadline, then
receives the reader result with `recv_timeout(remaining deadline, at least 0)`. If the reader has not finished by the
deadline (a descendant inherited the pipe and keeps it open after `systemctl` exited), the call returns the unknown triple
and the reader thread is detached (it ends when the last writer closes; it holds only the pipe and a ≤ 4097-byte buffer).
On timeout of the child: `kill()` then `wait()` (always reaped), unknown triple. Total wall time of the call ≤ `timeout` +
one poll interval. Real `systemctl show` forks no pipe holder, so the detached-reader path is hardening only. Total `soos-admin status` wall time ≤ socket timeout + 1 s,
inside the helper's 5 s attempt bound. No `unwrap`/`expect`; `#![forbid(unsafe_code)]` crate unchanged; no new dependency.

---

## 7. Physical procedure (`tests/physical/screensaver_test.md`)

- **§6 rollback**: replace the `sed` line by a loop over the base stacks that carry soos lines, following symlinks
  (authselect ships `system-auth`/`password-auth` as symlinks; plain `sed -i` would replace them by regular files) and
  skipping missing files:
  ```bash
  for f in /etc/pam.d/common-auth /etc/pam.d/system-auth /etc/pam.d/password-auth; do
      [ -f "$f" ] && sudo sed -i --follow-symlinks 's/^auth.*pam_soos\.so/# &/' "$f"
  done
  ```
  followed by one sentence: `swaylock`, `hyprlock`, `login` and `sudo` keep their distribution files and reach soos only
  through these base stacks (§3.1–§3.5), so they need no edit. Keep the presence and service-rollback bullets. The GDM
  bullet becomes: `sudo soos-admin gdm disable` stops face verification at once (`/etc/soos/gdm.disable`); when GDM uses a
  managed block, `sudo soos-admin gdm restore` puts back `gdm-password.soos-backup`; when it uses the shared base-stack rule
  (`gdm status` shows `Shared soos Rule`) there is no backup (`gdm restore` reports that it does not exist): use `gdm disable`
  or the base-stack loop above.
- **§3.3 PAM configuration**: replace the "tried again by the base-stack line" paragraph: when the delegated stack already
  carries the primary soos rule (Arch `system-auth`, Debian `common-auth` with the soos profile, Fedora `custom/soos`),
  `gdm enable` adds no managed block (and removes one left by an earlier release); `gdm status` shows `Shared soos Rule:
  <stack>`; expect exactly one `Rendered authentication response` line per GDM attempt, with the shared rule's deadline.
- **§3.3 Test Case 2**: add how to find the transient worker: it exists only while the unlock prompt is up, so start
  `sleep 15; pgrep -af 'gdm-session-worker \[pam/gdm-password\]'` in a terminal (or run it from a TTY / SSH session), lock
  with `Super+L`, raise the shield within 15 s, then `cat /proc/<pid>/cgroup` with the pid of the first column.
- Constraint: the document must still contain `swaylock`, `hyprlock`, `gdm`, `login` and `sudo`
  (`tests/invariants/src/lib.rs` screensaver check) — they remain in §3.1–§3.5.

---

## 8. Invariants Touched

| Invariant | Effect |
|---|---|
| Never convert an error into `PAM_SUCCESS` / fail closed | GDM: every scan error still refuses; non-primary soos rules stay unclassified; `status` fails closed to `installed: false`. Presence: settle only removes evaluations, never adds an Allow path; Allow still needs 3 consecutive evaluated live captures, any evaluated spoof vetoes. |
| A face success never skips a lockout/login gate (ADR 2026-09-30 GDM item 1) | **Changed scope, not weakened in practice**: every rule evaluated before the shared rule (GDM file, intermediate stacks, the shared stack up to the rule) must still classify as a gate or neutral, else `enable` refuses. Gates placed **after** the shared rule in the delegated stack (before its credential module) are never inspected: for GDM they are governed by that stack's own ordering, exactly as for `sudo`, `login` and the lockers that include it. This was already the effective behaviour before #331: after a miss at the managed block the shared rule retried, and a match there skipped the same gates. Account-phase gates (ADR item 3) are untouched. |
| PAM module: no Tokio, bounded deadlines | `crates/pam` untouched. |
| PAM path unchanged by presence settle (IWP11) | Dispatcher, consensus, warmup default untouched; new trait method read only by the presence worker. |
| PAM file edits atomic, backup = pristine pre-soos state (GitHub #166/#312/#318) | `RemoveRedundant` uses `write_atomic`, never writes a backup, never alters an existing one; `gdm restore` semantics unchanged. |
| Read-only installer preflight (#164, #329) | New checks only read; advice printed, never executed. |
| No sensitive data in logs | One `debug!` with `settle_ms`/`cause`; stderr tail of `soos-admin status` contains no biometric data (status report fields only). |
| Bounded I/O | `systemctl` output ≤ 4 KiB and 1 s; stderr tail ≤ 1024 chars; PAM reads stay ≤ `MAX_PAM_FILE_BYTES`, depth ≤ 4. |
| `#![forbid(unsafe_code)]` | admin-cli, daemon `main.rs` unchanged; camera-v4l adds no `unsafe`. |

---

## 9. Documentation Drift / ADR

### 9.1 New ADR (append to `AI/DECISIONS.md`)

> * **[2026-10-05] GDM Reuses a Shared Primary soos Rule; Presence Settle Keyed on the Camera Stream Start; Install
>   Readiness Follow-ups (GitHub #331; owner decision 2026-10-05; amends ADR 2026-09-30 "GDM PAM Stack Placement" items (2)
>   and (4) and ADR 2026-10-03 "Presence Wake Settle and Health-Polled Install Readiness (GitHub #329)"; matrix
>   IGF1–IGF19):** (1) **GDM**: when the auth stack delegated from the GDM service file (`include`, `substack`, `@include`,
>   followed up to `MAX_PAM_INCLUDE_DEPTH` = 4) reaches, before any credential module, an active primary `pam_soos.so` rule
>   — `auth` type, no `event=` and no `service=` argument, control `sufficient` or a bracket whose `success` action is `done`
>   or a jump N ≥ 1 and whose every other action is `ignore` — `soos-admin gdm enable` inserts no managed block and removes
>   an existing one (with any bare or legacy managed line) atomically. It never creates a backup in that case and leaves an
>   existing `.soos-backup` untouched, so `gdm restore` still returns the pristine pre-soos bytes (a no-op once the block is
>   gone) and still refuses a stale backup without `--force`. Rules before the shared rule are classified exactly as before
>   (gates, neutral, unclassified → refusal); a `[...=N]` jump in the GDM file does not block this case because nothing is
>   inserted; any other `pam_soos.so` rule before the credential module stays unclassified (refusal). Gates placed after
>   the shared rule in the delegated stack are governed by that stack's own ordering, as for `sudo` and `login`; this was
>   already the effective behaviour, since the shared rule retried after a miss at the managed block. An administrator
>   `pam_soos.so` rule in the GDM file is still never touched. GDM then makes one daemon request and one rate-limit attempt
>   per attempt (the double verification seen on Arch, where the block predated the `system-auth` edit, is gone), with the
>   shared rule's deadline (module default 1000 ms unless that rule sets `timeout_ms=`) instead of `timeout_ms=2500`
>   (accepted trade-off: a GDM unlock that needs a camera wake may fall back to the password); `GDM_PAM_LINE` keeps
>   `timeout_ms=2500` for stacks without a shared rule. `gdm.disable` keeps working because the shared rule reads
>   `PAM_SERVICE`. `gdm status` reports `installed: true` and `shared_stack: "<file>"` when the integration is reached that
>   way (same analysis as `enable`; a stack `enable` refuses reports `installed: false`; a shared jump rule whose target is
>   not checked, or a pre-anchor jump that skips the delegation, can report `installed: true` while face login never
>   succeeds — fail closed, documented). (2) **Presence**: the settle lower
>   bound is `max(scan wake ? scan start + PRESENCE_WAKE_SETTLE_MS : 0, stream start + PRESENCE_WAKE_SETTLE_MS)`, where the
>   stream start is `CameraManager::stream_started_mono_ns()` (CLOCK_MONOTONIC stamp taken by the V4L supervisor when the
>   capture stream is set up, immediately before the first dequeue that issues `VIDIOC_STREAMON` (v4l 0.14 starts the
>   stream lazily); `None` by default); a camera woken by a PAM request less than 1 s before a scan is now covered.
>   The settle only gates presence evaluation; the PAM path, `warmup_frames` (0), PAD thresholds and consensus rules are
>   unchanged. (3) **Install**: a relative `CARGO_TARGET_DIR` is resolved once against the checkout and passed explicitly to
>   cargo; the `--build` preflight also refuses read-only files and an unreadable subtree (advice `chmod -R u+rwX`);
>   `wait_daemon_ready.sh` documents its real worst case (`--timeout` + 7.5 s), keeps the last `soos-admin status` stderr
>   (≤ 1024 characters, control characters replaced) in its timeout error and fails at once on exit 126/127. (4)
>   `soos-admin status` bounds `systemctl show` to `SYSTEMCTL_SHOW_TIMEOUT_MS` = 1000 ms (killed and reaped, fields
>   `unknown`). Walkthrough 179.

Also append to the end of the 2026-09-30 "GDM PAM Stack Placement" entry: ` Amended by ADR 2026-10-05 "GDM Reuses a Shared
Primary soos Rule …" (no managed block when the delegated stack already reaches a primary soos rule).`

### 9.2 Prose drift to fix in the same PR

- `.claude/skills/dev-workflow/references/project-facts.md` §2: `GDM_PAM_LINE` row → append "; omitted (and an existing
  block removed) when the delegated stack already reaches a primary `pam_soos.so` rule, GitHub #331"; add rows
  `PRESENCE_WAKE_SETTLE_MS` (`crates/daemon/src/presence/mod.rs`, 1000 ms, bound keyed on scan wake and stream start) and
  `SYSTEMCTL_SHOW_TIMEOUT_MS` (`crates/admin-cli/src/status.rs`, 1000 ms).
- `AI/ARCHITECTURE.md` (plan evaluation finding 1, MAJOR):
  - line ~130, replace the closing clause "GDM uses `timeout_ms=2500`." by: "GDM uses `timeout_ms=2500` only when
    `soos-admin gdm enable` inserts the managed block; when the delegated stack already carries a primary soos rule, GDM
    uses that rule's deadline (module default 1000 ms), ADR 2026-10-05 \"GDM Reuses a Shared Primary soos Rule\"."
  - §5 GDM paragraph (~177-182), replace by: "**GDM (`/etc/pam.d/gdm-password`)**: when the auth stack it delegates to
    already reaches a primary `pam_soos.so` rule (packaged Arch `system-auth`, Debian `common-auth` with the soos profile,
    Fedora `custom/soos`), `soos-admin gdm enable` writes nothing and removes a managed block left by an earlier release;
    GDM authenticates through that shared rule (one daemon request per attempt, the rule's deadline) and `gdm status`
    reports it in `shared_stack`. Otherwise `gdm enable` inserts `auth  [success=done default=ignore]  pam_soos.so
    timeout_ms=2500` inside a marked block placed before the first credential or shared-stack rule, after every in-file
    `pam_nologin`, `pam_succeed_if`, `pam_shells` and `pam_faillock preauth` rule, with the gates of a delegated stack that
    run before its credential module copied in front of it (ADR 2026-09-30 \"GDM PAM Stack Placement\", amended by ADR
    2026-10-05; `Docs/DISTRIBUTION_DEPLOYMENT.md` section 2.1)."
- `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1: describe the shared-rule case (no block, removal of a redundant block, status line,
  1000 ms trade-off, the false negative and the two false positives of §2.5); keep `timeout_ms=2500`,
  `connection_timeout_ms`, `2500 ms` in §2.1 (QFU5). Plan evaluation finding 3: relabel the example "What `gdm enable`
  writes, e.g. on Fedora 40 (`authselect ... with-faillock`)" as a stack **without** a soos rule (Fedora 40 stock `local`
  profile `with-faillock`, before the soos `custom/soos` profile is selected; the block content stays as shown), and add the
  shared-rule example: with `custom/soos` selected, `gdm enable` leaves `gdm-password` unchanged and `gdm status` prints
  `Shared soos Rule:  password-auth`.
- `Docs/DAEMON.md` presence settle paragraph; `Docs/CAMERA_V4L_CRATE.md` trait table (`stream_started_mono_ns`).
- `README.md` / `Docs/PACKAGING_AND_PROVISIONING.md`: readiness bound, relative `CARGO_TARGET_DIR`, new preflight advice
  (keep the `is_healthy` and `sudo chown -R` needles for IWP13).
- `tests/physical/screensaver_test.md` §3.3 / §6 (§7 above), including the §6 GDM bullet (finding 3).
- Walkthrough `AI/walkthroughs/179_install_gdm_followups.md` (next number after 178).

---

## 10. Existing tests that constrain the design (all must stay green unchanged)

| Test | Constraint honoured |
|---|---|
| `install_presence_warmup_contract::test_iwp_readiness_*` (IWP1–IWP3) | One JSON on stdout; `did not report healthy within 2 s` + `journalctl` kept; killed attempt prints no stdout; elapsed < 15 s. Their fake admin writes no stderr and exits 0/1 (never 126/127). |
| `…::test_iwp_install_keeps_exit_70_on_readiness_failure` (IWP4) | Helper call block in `install.sh` and exit 70 unchanged. |
| `…::test_iwp_build_refuses_foreign_owned_target_dir`, `…dry_run…` (IWP5) | Check 1 text unchanged; absolute targets unchanged by P-1. |
| `…::test_iwp_build_refuses_non_writable_target_dir` (IWP6) | 0555 dir passes check 2 (has `u+rx`), hits check 3 with `chmod -R u+w <target>`. |
| `…::test_iwp_build_proceeds_with_an_owned_target_dir` (IWP7) | No false positive; no `sudo chown -R` text on success. |
| `…::test_iwp_settle_is_presence_only` (IWP11) | `PRESENCE_WAKE_SETTLE_MS: u64 = 1000;` in `presence/mod.rs`; `with_not_before(` and the constant in `worker.rs`; dispatcher/consensus untouched; warmup default 0. |
| `…::test_iwp_docs_and_adr_describe_the_changes` (IWP13) | Needles kept. |
| `presence_wake_settle_tests::*` (IWP8–IWP10), `presence_wake_settle_consensus_tests::*` | `SpyCamera` returns the default `None`: `woke` path identical; streaming camera has no settle. |
| `arch_faillock_ci_contract::test_install_build_exports_the_prefix_bindir` (AFC7) | runuser positional args and literals kept (§4.1). |
| `installer_contract` (`cargo build --release --locked --workspace` literal) | Kept. |
| `gdm_tests::*`, `gdm_stack_order_tests::*`, `gdm_followup_tests::*`, `gdm_commented_rule_tests::*`, `gdm_restore_stale_backup_tests::*`, `gdm.rs::restore_race_tests` | No fixture contains a `pam_soos.so` rule in a delegated stack (checked), so every insert/refusal outcome and every error priority is unchanged (§2.4 keeps the jump check before delegated-scan errors). |
| `lib.rs::test_matrix_gdm_references_resolve_to_real_tests` | New IGF rows may cite `gdm_shared_rule_tests::…` freely (only `gdm_tests`/`gdm_stack_order_tests` citations are resolved by that test; the generic citation check covers the rest). |
| `quality_followups::test_gdm_timeout_documents_daemon_connection_timeout_cap` (QFU5) | §2.1 needles kept. |
| `lib.rs` screensaver document check (`swaylock`, `hyprlock`, `gdm`, `login`, `sudo`) | Kept (§7). |
| `distro_matrix::test_pam_case_lib_reads_timeout_from_the_stack_under_test`, Docker T1–T15 | `crates/pam` and the matrix scripts untouched. |
| `camera_status_tests::test_default_status_is_derived_from_readiness`, `supervisor_tests::*` | Trait method defaulted; supervisor behaviour unchanged except the stamp. |

**No existing test needs an owner-approved change.** One optional **additive** harness extension (no assertion touched):
for the worker-level PAM-wake test (IGF7) the tester may add a `stream_started_ns: AtomicU64` field (0 = `None`) to
`crates/daemon/tests/common/mod.rs::SpyCamera`, set to 0 at its single construction site, plus the trait override returning
it (the spy restamps frames with the test clock, so the stamp must be in the test-clock domain; the mock setter of §3.3 is in
the real clock domain and cannot be used there). Accepted by the orchestrator as setup-only (Revision 1).

---

## 11. Acceptance Criteria → Matrix Rows (one per issue checkbox)

| Issue checkbox | Rows | Acceptance |
|---|---|---|
| §1.1 build preflight read-only files + chmod advice for unreadable subtree | **IGF1** read-only file (target owned, dirs writable, one 0444 file) → exit 2, `holds a file … cannot write`, `chmod -R u+w <target>`, no `cargo`/`runuser`, nothing staged (also `--dry-run`); **IGF2** user-owned unreadable directory (0000 and 0300) as a **non-root runner** → exit 2, `cannot read`, `chmod -R u+rwX <target>`, and the output does **not** contain `sudo chown -R` (the advice replaces the former wrong `chown` advice; skipped when the runner is root); under fake root (`root_shims`, real non-root uid) the same 0300 directory → same `chmod -R u+rwX` advice | §4.2 |
| §1.2 real readiness bound | **IGF3** static: header + `usage()` + README + packaging doc state `--timeout` + 7.5 s; README no longer says "at most 30 s"; a behavioural check: `--timeout 1` with a hanging admin ends in < 1 + 7.5 + 2 s | §5.1 |
| §1.3 last stderr + fail fast 126/127 | **IGF4** fake admin always exits 1 writing `boom-<n>` to stderr → timeout error contains `Last '…' error:` and the last `boom-<n>` only, stdout unchanged; control characters printed as `?`, tail ≤ 1024 chars; **IGF5** `--admin` pointing to a non-executable file (126) and to a missing path (127) → exit 1 well under the bound (< 3 s with `--timeout 30`), message `exit 126`/`exit 127`, no JSON on stdout, exactly one attempt | §5.2 |
| §1.4 settle keyed on stream start | **IGF6** `presence_settle_window` unit properties (§3.4 list incl. saturation) with exact boundaries, `settle_ms = 1000`, not woken: stream age `d == 1000 ms` → `{0, 0}`; `d == 999 ms` → `wait_ms == 1` and `not_before_ns == stream + 1_000_000_000`; `d == 0` → `wait_ms == 1000`; stream stamp later than `start_ns` → clamped (`not_before_ns == start_ns + 1_000_000_000`, `wait_ms == 1000`); `Some(0)` → as `None`; woken + stream older than settle → today's `start + settle`; woken + stream 300 ms old → `start + settle` (max); `u64::MAX` inputs → no panic, `wait_ms <= settle_ms`; **IGF7** worker: camera ready, stream started 300 ms ago (test clock) → no PAD evaluation before ≈ 700 ms, unlocks with k = 3; stream started > 1 s ago → no settle; **IGF8** V4L supervisor (fake backend): stamp `Some` after first frame with `stamp <= frame.timestamp_mono_ns`, `None` while suspended, strictly larger after resume; mock setter; default trait method `None`; **IGF9** static: dispatcher never names `stream_started_mono_ns`, `with_not_before` or `PRESENCE_WAKE_SETTLE`; `consensus.rs`, `config.rs` unchanged | §3 |
| §1.5 P-1 relative `CARGO_TARGET_DIR` | **IGF10** run `install.sh` from a different working directory with a relative `CARGO_TARGET_DIR`: artifact check, `--artifact-dir` default and the ownership check use `<checkout>/<rel>`; the build shims see `CARGO_TARGET_DIR` (non-root) / `SOOS_CARGO_TARGET_DIR` (runuser) equal to that absolute path; AFC7 literals intact | §4.1 |
| §1.6 P-2 `systemctl show` timeout | **IGF11** `inspect_systemd_unit_with` with a fake program `#!/bin/sh\nexec sleep 30` (exec, so no orphan keeps the pipe) → unknown triple within `timeout + 1 s`; a fake `#!/bin/sh\nsleep 30 &\necho ActiveState=active` (descendant holds stdout after exit) → returns within `timeout + 1 s` (unknown or parsed, never blocked); a fake printing `ActiveState=active\nSubState=running\nMainPID=42` → parsed; non-zero exit → unknown; `SYSTEMCTL_SHOW_TIMEOUT_MS == 1000` | §6 |
| §2.1 §6 rollback `sed` | **IGF12** static: §6 command names only `common-auth`, `system-auth`, `password-auth`, uses `--follow-symlinks`, no longer lists `/etc/pam.d/swaylock`, `/etc/pam.d/hyprlock`, `/etc/pam.d/sudo` | §7 |
| §2.2 §3.3 pgrep hint | **IGF13** static: §3.3 Test Case 2 contains `pgrep -af 'gdm-session-worker \[pam/gdm-password\]'` and `/proc/<pid>/cgroup` | §7 |
| §3 remove GDM double verification | **IGF14** enable on the packaged Arch chain (`ARCH_GDM_PASSWORD` → `system-local-login` → `system-login` → `packaging/pam/arch/system-auth`): `Ok`, file byte-identical, no backup, no temp file, `gdm.disable` removed (red today: refusal); same for Debian `@include common-auth` with `[success=done default=ignore] pam_soos.so` and Fedora `substack password-auth` rendered from `packaging/pam/fedora/soos/password-auth` with `with-faillock` (template `{include if …}` markers resolved by hand); **IGF15** existing managed block (from an enable before the shared rule existed) → removed, file == pre-soos pristine, backup byte-identical (content, mode), `gdm restore` then succeeds and removes the backup; second `enable` writes nothing (mtime/inode unchanged); a stale backup still needs `--force`; **IGF16** `gdm status` → `installed: true`, `shared_stack: Some("system-auth")`, `enabled` follows `gdm.disable`; a direct rule keeps `shared_stack: None`; unreadable include → `installed: false`; **IGF17** non-qualifying shared rules (`event=` only, `service=sudo`, `optional`, `required`, `[success=ok …]`, `[success=done default=die]`, `[success=0 default=ignore]`, `[success=-1 default=ignore]`, `[success= default=ignore]`, `[default=ignore]` (no `success`), `[success=done success=bad default=ignore]` (duplicate conflicting `success`)) → the previous refusal, file unchanged, no backup; qualifying edge forms → shared (no block): `SUFFICIENT` (upper case), `[Success=DONE Default=Ignore]`, `[success=done ignore=ignore default=ignore]` (extra `ignore` key), path form `/usr/lib/security/pam_soos.so`. The same table may be covered by `pam_stack.rs` unit tests of `is_primary_soos_rule` plus one end-to-end case each way; **IGF18** error priority: jump crossing + shared rule → `Ok` without edit; jump crossing without shared rule → the existing jump error; unclassified rule before the shared rule → refusal; administrator soos rule in the GDM file + shared rule → untouched; **IGF19** docs/ADR: `AI/DECISIONS.md` contains the 2026-10-05 entry (`GDM Reuses a Shared Primary soos Rule`) and the amendment note; `AI/ARCHITECTURE.md` contains `shared_stack` and `only when` next to `timeout_ms=2500` (needles: `GDM uses \`timeout_ms=2500\` only when` and `shared_stack`), and no longer contains the bare sentence `GDM uses \`timeout_ms=2500\`.`; `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 documents the shared-rule case (`Shared soos Rule`), still contains the QFU5 needles, and its Fedora example is no longer labelled `authselect ... with-faillock)` alone; screensaver §6 mentions `Shared soos Rule`; project-facts row updated | §2 |

New test files suggested: `crates/admin-cli/tests/gdm_shared_rule_tests.rs` (IGF14–IGF18), `#[cfg(test)]` module in
`crates/admin-cli/src/status.rs` (IGF11, uses the `pub(crate)` seam; `tempfile` is already a dev-dependency),
`crates/daemon/tests/presence_stream_settle_tests.rs` (IGF6, IGF7), new test functions in
`crates/camera-v4l/src/v4l_impl/supervisor_tests.rs` (IGF8; may call the parent's private `monotonic_nanos`),
`tests/invariants/src/install_gdm_followups_contract.rs` (IGF1–IGF5, IGF9, IGF10, IGF12, IGF13, IGF19; reuse
`run_build`-style shims and `ready_fixture`-style fakes, written locally since those helpers are private to their module).

## 12. Test Hooks for the Tester

- `PamLine::is_primary_soos_rule`, `delegated_auth`, `DelegatedAuth` (pub(crate): unit tests in `pam_stack.rs` tests module
  are additive); public behaviour through `configure_gdm`, `configure_gdm_with_options`, `get_gdm_status`, `GdmStatus`.
- `presence_settle_window`, `PresenceSettle` (pub in `soos_daemon::presence`).
- `CameraManager::stream_started_mono_ns` (default), `MockCameraManager::set_stream_started_mono_ns`.
- `status::inspect_systemd_unit_with` (pub(crate)), `SYSTEMCTL_SHOW_TIMEOUT_MS` (pub).
- Shell: `wait_daemon_ready.sh --admin <fake>` (stderr / exit code control), `install.sh` with `CARGO_TARGET_DIR`, PATH
  shims that log `$CARGO_TARGET_DIR` / `$SOOS_CARGO_TARGET_DIR`, and a non-default working directory.

## 13. Open Questions

None blocking. Design choices taken without owner input (flag only if the owner disagrees):
1. `RemoveRedundant` keeps an existing backup (restore keeps working; no new backup written).
2. A shared rule with `service=`, `event=` or a non-primary control does not count and keeps today's refusal.
3. `gdm status` uses the `enable` analysis (consistent with `enable`, conservative `installed: false` on refused stacks).
4. Readiness fail-fast exits 1 (not 2) so `install.sh` keeps exit 70.

---

## Revision 1 (2026-10-05, plan evaluation REVISION_REQUIRED, 7 findings)

1. **MAJOR** — `AI/ARCHITECTURE.md` added to §1 and §9.2 with exact replacement text for line ~130 and the §5 GDM
   paragraph; IGF19 gains `AI/ARCHITECTURE.md` needles.
2. Stamp wording corrected (trait doc §3.1, §3.2, ADR item 2): `start_stream` maps buffers; v4l 0.14 issues
   `VIDIOC_STREAMON` lazily at the first dequeue; the stamp is the stream set-up instant, never later than a frame stamp.
3. `Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1 Fedora example relabelled as a stack without a soos rule, shared-rule example
   added; screensaver §6 GDM bullet covers the no-backup shared case.
4. Gate guarantee wording (§8, ADR item 1): gates after the shared rule are governed by the shared stack; already the
   effective behaviour before #331.
5. §2.5 (and the ADR, deployment doc) document the false negative and the two `installed: true` false positives
   (unchecked jump target, pre-anchor jump past the delegation); all fail closed.
6. P-2: stdout drained on a helper thread with `recv_timeout` against the deadline (detached reader if a descendant holds
   the pipe); IGF11 fakes use `exec sleep 30` plus a pipe-holding-descendant case.
7. IGF6 exact boundaries, IGF17 every §2.2 edge and qualifying edge forms, IGF2 asserts `chmod -R u+rwX` instead of
   `sudo chown -R` for a user-owned 0300 directory as a non-root runner. `SpyCamera` field accepted as setup-only (§10).
