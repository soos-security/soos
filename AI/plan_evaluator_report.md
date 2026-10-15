# Plan Evaluation Report
- **Date**: 2026-10-15 (round 3: re-evaluation of `## 10. Revision 2`)
- **Issue**: GitHub-only (no backlog id) — `soos-lockstatus` bridge and swaylock-plugin / hyprlock adapters (GitHub #401, PR 2 of epic #352)
- **Branch**: `feat/lockscreen-adapters`
- **Base commit**: `2025738`
- **Plan evaluated**: `AI/architect_spec_lockscreen_adapters.md` §1–§8 as amended by §9 Revision 1 and §10 Revision 2
- **History**: rounds 1 and 2 (both REVISION_REQUIRED) are kept below as history. Only the round-3 verdict line is machine-readable.
- **Worktree note**: the untracked `crates/lockstatus` and `crates/lock-widget` are Phase-2 signature stubs, with no behaviour (statement of §10). This evaluation did not review them.

# Round 3

## 1. Coverage Matrix (delta against round 2)

| Round-2 finding | Revision 2 element | Status |
|---|---|---|
| R2-1 (MAJOR) star snaps at every new scan | §10 R2-1: anchor kept while a turn runs; `stop` on half-turn (`H = 2 · T_PENDING_MS`) boundaries; `angle = (90·(q mod 2) + 90·ease(frac)) mod 180`; 30 s cadence-replay test plus rest-at-0° test | Resolved (verified below) |
| R2-2 cache not keyed by socket | `fnv1a64(socket path)` in the record, mismatch = miss | Resolved |
| R2-3 cache readable by others / check-then-open | dir `mode & 0o077 == 0`, post-open `fstat` (regular, owner, 1 link, `& 0o077 == 0`), five negative tests; `openat` not used | Resolved (residual race acceptable, see Pillar 1; Finding R3-3 optional) |
| R2-4 cache clock / `None` clock | `CLOCK_BOOTTIME` completion stamp, `None` disables cache read and write, future stamp = miss | Resolved |
| R2-5 auth-reserve residual | recorded in ADR and docs | Resolved |
| R2-6 shared back-off, diagnostics | `failures` in the record, back-off freshness, `Source::{Daemon, Cache, LockBusy, NoRuntimeDir}`, widget stderr line | Resolved, one misclassification left (Finding R3-2) |
| R2-7 motion wording | owner-to-confirm item 7 states continuous turning while scans repeat | Resolved |
| R2-8 swaylock-plugin uncovered outputs | assumption, owner row LKA14 | Resolved |

## 2. Facts Verified (round 3)

| Fact cited by revision | Check | Actual | Match |
|---|---|---|---|
| Angle continuity at quarter boundaries | end of quarter `q` even: `0 + 90·1 = 90`, start of `q+1`: `90 + 0 = 90`; end of `q` odd: `90 + 90 = 180 ≡ 0`, start of `q+1`: `0` | continuous | Yes |
| Continuity at `stop` and at a fresh anchor | `stop` is `anchor + k·H`, so the angle there is `0 ≡ 180`; a fresh anchor starts at 0 | continuous | Yes |
| Leaving Scanning ends "at most two quarters later" | `up(t) − t < H = 2400 ms` = two quarters | as cited | Yes |
| Presence cadence keeps the anchor | re-entry at +4240 ms < `stop` (≥ `up(4800)` = 4800) | anchor kept, `stop` extended | Yes |
| Record size "at most 95 bytes" | `failures` as `u32`: 88 bytes; as `u64`: 98 bytes (`MAX_CACHE_BYTES` 128 bounds both) | depends on the unspecified width | Partial (Finding R3-1) |
| `openat` needs `unsafe` with nix 0.29 | nix 0.29 `fcntl::openat` returns `RawFd`; `OwnedFd::from_raw_fd` is `unsafe` | as stated | Yes. A safe alternative exists: `rustix` 1.1.4 (already in `Cargo.lock` through smithay-client-toolkit 0.20) offers a safe `rustix::fs::openat -> OwnedFd` (Finding R3-3, optional) |
| `CLOCK_BOOTTIME` precedent | `crates/daemon/src/presence/worker.rs:185-188` | exists | Yes |

Existing-test impact: unchanged from rounds 1–2. No existing assertion has to change, so there is no CRITICAL finding.

## 3. Pillar Analysis (round 3)

### Pillar 1 — Architecture & threat model
- **Residual check-then-open race (R2-3 without `openat`).** Swapping the runtime directory between the `symlink_metadata` check and the `open` needs write permission on its *parent*. For the standard `/run/user/<uid>`, the parent `/run/user` belongs to root, so only root can do it.
  - **Non-standard `XDG_RUNTIME_DIR`** (for example under a directory another user owns): the swap can only redirect the open to a different file named `soos-lockstatus.lock`. The post-open `fstat` on the opened descriptor then still requires that file to be regular, owned by the user, mode 0600 and single-link.
  - The worst outcome is truncating another lock file of the same user, or Unavailable. There is no cross-user read or write, no daemon contact on failure, and no success state.
  - Result: PASS, residual acceptable as recorded in the trust-boundary paragraph. The race-free safe variant is listed as optional R3-3.
- **Same-user poisoning, starvation, success state.** Covered by the trust boundary (ADR); no success value exists. Result: PASS.

### Pillar 2 — Deadlines, concurrency and slots
- **Shared back-off misclassifies a budget-starved exchange.**
  - A `--once` (budget 250 ms) waits about 240 ms for the lock because the holder's exchange was slow. It acquires the lock and finds a stale or missing record, for example because the holder's clock returned `None`, it crashed, or it used another socket.
  - It runs the exchange with about 10 ms left, gets `Timeout`, and that counts as `Source::Daemon`. It writes `failures = 1`, which makes every client of the user render Idle for `ERROR_BACKOFF_MS[0]` = 1 s, and escalates to 2 s and 4 s on repeats, although the daemon is healthy.
  - Result: FINDING (MINOR, R3-2).
- Rate and slot bounds unchanged from round 2 (≤ ~4/s, one connection per user). Result: PASS.

### Pillar 3 — Panic safety & fail-quiet
- `now − anchor` grows without bound while scans repeat. It is a `u64` difference and `q` is an integer, so there is no overflow in practice. Out-of-order clocks saturate (§2.2). Result: PASS, as long as `frac` is computed from the integer remainder and not from an `f32` of `now − anchor`. That property is already implied by the 30 s replay test.

### Pillar 4 — Dependencies
- No change. If R3-3 is adopted, `rustix` becomes a direct dependency at the already-locked 1.1.4 version: no new crate, but the dependency allow-list invariant (LKA12) must name it. Result: PASS.

### Pillar 5 — Data & memory
- The record holds the coarse view, sequence, failures, stamp and socket hash: nothing biometric, read bounded to 128 bytes, private mode enforced post-open. Result: PASS.

### Pillar 6 — Test integrity & power
- The cadence-replay test fails against the round-2 behaviour (re-anchor at entry) and against a 90° domain. The negative cache tests cover 0644, 0755, symlinked directory, symlinked file and hard link. The concurrency test is unchanged. Result: PASS.
- One recommendation: add a test where the lock is held almost to the budget, so a starved exchange does not raise `failures` (ties to R3-2).

## 4. Findings (round 3)

1. **[MINOR] R3-1 — Record width.** Specify the type of `failures` (e.g. `u8` saturating at 3, since only `min(n, 3)` is used) so the stated maximum record length is exact. It is 88 bytes with `u32` and 98 with `u64`, while the spec says "at most 95". `MAX_CACHE_BYTES` = 128 is safe either way. Test with every field at its maximum.
2. **[MINOR] R3-2 — Budget-starved exchanges count as daemon failures.** When the lock wait leaves less than a minimum exchange budget (e.g. < 50 ms), classify the result as `Source::LockBusy` (no daemon contact, no `failures` increment, no record write). Alternatively, only `Connect` refusals, `Refused`, EOF and decode errors increment `failures`, and a `Timeout` increments it only when the exchange had at least that minimum budget. Add a test with the lock held until 240 ms of a 250 ms budget.
3. **[MINOR, optional] R3-3 — A race-free safe open exists.** `rustix` 1.1.4 is already locked (via smithay-client-toolkit 0.20). Its `rustix::fs::openat` and `fstat` work on `OwnedFd`/`BorrowedFd` without `unsafe`. The flow would be: open the directory with `O_DIRECTORY | O_NOFOLLOW`, `fstat` it, then `openat` the lock file `O_NOFOLLOW | O_CREAT`. This removes the residual window without violating `forbid(unsafe_code)`. Adopting it is not required for approval, because the residual is limited to processes that can rename entries of the runtime directory's parent and is bounded by the post-open `fstat`.

## 5. Verdict (round 3)
VALIDATION_VERDICT: APPROVED

All round-1 and round-2 CRITICAL/MAJOR findings are resolved; R3-1 to R3-3 are MINOR. The tester-agent should encode R3-1 and R3-2 in the Phase-2 contract (the record length test at maximum field values, and the lock-starved exchange test). R3-3 is left to the architect's discretion.

---

# Round 2 (history)

## R2-1. Coverage Matrix (delta against round 1)

| Round-1 finding / acceptance line | Revision element | Status |
|---|---|---|
| F1 aggregate per-UID load, auth-reserve claim | R1 `shared.rs` (per-user `flock` + 250 ms cache), claim withdrawn, snippet pinned (`--once`, `update:` ≥ 500), concurrency test | Resolved, with residuals: Findings R2-2 to R2-6 |
| F2 star symmetry | R2 `[0, 180)` domain, 90°-differs / 180°-equals pixel tests | Resolved for one scan; **new defect when the loop restarts** — Finding R2-1 |
| F3 coverage | R3 no-panic / no-print invariant, JSON key-set invariant, LKA1–LKA17 table mapping every #352 line (incl. `--command-each`, kill/restart, password and presence unchanged, hyprlock hardware, idle CPU) | Resolved |
| F4 JSON on the locked screen | R4 `OutputFormat` without `Json`; JSON only in `--watch` | Resolved |
| F5 line length | R5 76 bytes, tested with `u64::MAX` | Resolved (spec says `available:false` gives 76; `false` is only emitted with `idle`/`0`, the 76-byte line uses `true` — cosmetic) |
| F6 `--once` silence | R6 `write_all` + `flush`, errors ignored, no print macro, `try_parse_from`, nothing to stderr when `--once` present | Resolved |
| F7 forbid in `main.rs` | R7 + LKA12 | Resolved |
| F8 touched files | R8 | Resolved |
| F9 duty cycle | R9 `PENDING_LOOP_MAX_MS` 4800, owner-to-confirm restart | Partially: see Finding R2-1 (restart snap) and R2-7 (continuous motion wording) |
| F10 robustness | R10 poller death / panic → Idle + delayed exit 1; one `SlotPool` per surface; scale 1; shake once per sequence | Resolved except the unverified swaylock-plugin fallback claim — Finding R2-8 |
| F11 token naming | R11 `_MS` suffix, values compared with `Docs/UI_MOTION.md` | Resolved (deviation recorded; `theme.rs` cross-check deferred to `feat/gui-motion`, noted) |
| F12 test power | R12 → LKA1, LKA3, LKA6 | Resolved |
| F13 dlopen | R13 | Resolved |
| F14 kill switch | R14 deferred to the GNOME PR, ADR records it | Resolved |

## R2-2. Facts Verified Against Code (round 2)

| Fact cited by revision | Code / computation | Actual value | Match |
|---|---|---|---|
| A UID at its regular cap falls back to the auth-reserved permit | `crates/daemon/src/dispatcher.rs:473-479` | as cited | Yes |
| Longest cache record ≤ `MAX_CACHE_BYTES` 64 | `v1 not_recognized 18446744073709551615 18446744073709551615\n` | 60 bytes | Yes |
| `CACHE_FRESH_MS` 250 = `MIN_POLL_INTERVAL_MS` ⇒ ≤ ~4 daemon requests / s per user | requests start only on a miss, under the lock, after a record ≥ 250 ms old | ≤ 4 / s + one edge request, below `LOCK_STATE_MAX_REQUESTS_PER_SEC` 8 | Yes |
| Every presence-failing cycle restarts the loop ~4.24 s after the previous start | `Docs/IPC_PROTOCOL.md` §14, matrix LKS17 (floor 3240 ms + `LOCK_POLL_INTERVAL_MS` 1 s) | as cited | Yes |
| `PENDING_LOOP_MAX_MS` 4800 = 4 quarters of `T_PENDING` 1200, a multiple of 180° | arithmetic | 4 × 1200 = 4800, 4 × 90° = 360° | Yes |
| Project precedent for a suspend-proof clock | `crates/daemon/src/presence/worker.rs:185-188` (`CLOCK_BOOTTIME`) | exists | — (used in Finding R2-4) |
| `flock` on separately opened descriptions excludes threads of one process (the planned concurrency test) | Linux `flock(2)` semantics: locks belong to the open file description | holds | Yes |

Existing-test impact: R1–R14 add files, manifests and invariants only; none of the existing assertions listed in round 1 must change. No CRITICAL finding.

## R2-3. Pillar Analysis (round 2, focused on `shared.rs` and the revised scene)

### Pillar 1 — Architecture & threat model
- **Cache poisoning by the same user.** A process of the same UID writes `v1 scanning …` into the lock file, so the locked screen shows Scanning. It cannot produce a success state, unlock, or affect PAM, and a same-UID process could already kill or replace the widget. Result: PASS. Record the trust boundary ("the cache is as trusted as the user's own processes") in the ADR.
- **Cross-user leak through the cache.** Steps 1–2 check that the directory and the file are owned by `uid`, that the directory has no group/other *write* bit, and that the file is regular with one link. If the user has a pre-existing lock file with mode 0644, or `XDG_RUNTIME_DIR` points to a 0755 directory, another local user can read the user's lock state. That is exactly the data the daemon refuses cross-UID with `UidMismatch`. `O_CREAT` mode 0600 does not fix the mode of an existing file. Result: FINDING (MINOR, R2-3).
- **Cache poisoning across sockets.** The record is not keyed by `--socket`. A diagnostic or test run of `soos-lockstatus --once --socket /tmp/mock.sock` in the real `$XDG_RUNTIME_DIR` writes a mock answer that the real widget serves for 250 ms, and vice versa. Result: FINDING (MINOR, R2-2).

### Pillar 2 — Deadlines, concurrency and slots
- **Deadline accounting.** The `flock` wait is charged to the same `budget`, then the exchange gets "the budget left", so a `--once` stays within 250 ms plus one 5 ms retry overshoot, and a cache hit needs no budget. Result: PASS.
- **Lock starvation.**
  - A holder killed with SIGKILL releases the lock.
  - A holder stopped with SIGSTOP, or a hostile same-UID process holding the lock, makes every client render Idle (fail quiet). This is the same-UID trust boundary. Result: PASS (document it).
  - A normal holder blocks waiters for at most one exchange (≤ 250 ms). Waiters that time out render Idle for that one poll. The widget's `Backoff` then counts it as a failure and waits 1 s, so lock contention (not a daemon fault) delays the widget by a second. Acceptable, folded into R2-6.
- **Residual slot use.** At most one lockstatus connection per user now. With two other persistent user-level clients (soos-gui preview and the soos-remote camera view, 2 regular permits), that single connection still lands on the auth-reserve permit, and a PAM connect at that instant is refused. The revision's wording ("never both taken *by these clients*") is correct, but the ADR should state this residual. Result: FINDING (MINOR, R2-5).
- **Old daemon.** A pre-#380 daemon closes on `kind = 3` and logs a warning. Cached `unavailable` records only last 250 ms, so all of the user's clients together still make about 4 requests per second, each producing a daemon warning, and the §14 back-off on EOF never happens across processes. Result: FINDING (MINOR, R2-6).
- **`XDG_RUNTIME_DIR` absent.** Every client renders Idle and never contacts the daemon. This is safe, but the widget is then silently useless with no diagnostic. Result: FINDING (MINOR, R2-6).

### Pillar 3 — Panic safety & fail-quiet
- Every cache parse or I/O error is a miss and the daemon path stays the source of truth. An unreadable clock is not specified (`now_ms` returns `None`). Result: FINDING (MINOR, R2-4).
- **TOCTOU.** The directory is checked with `symlink_metadata`, then the file is opened by path. The directory belongs to the user, so only the user could swap it in between; an `openat` on an `O_DIRECTORY | O_NOFOLLOW` descriptor that has passed `fstat` closes the gap cheaply. Result: folded into R2-3.

### Pillar 4 — Dependencies
- `flock`, `fstat` and `openat` are available in `nix` 0.29 (`fs` feature, already enabled in the workspace), so no new crate is needed and no `unsafe`. Result: PASS.

### Pillar 5 — Memory and data
- Bounded cache read (64 bytes), one record, no biometric content. Shm budget unchanged from round 1. Result: PASS.

### Pillar 6 — Test integrity & power
- **Restart snap.** R2 fixes the angle domain, and R9 restarts the loop for every new scan (~4.24 s). Leaving Scanning at ~3.24 s, the star must "continue to the next multiple of 180°", which with `T_PENDING` 1200 ms ends at ~4.8 s. The next scan is published at ~4.24 s and seen by a 500 ms poll before or after that. On the new `(Scanning, seq)` key the scene re-anchors at the scan entry (§2.2 "anchored at the scan entry"), so the angle jumps from about 120–170° back to 0° in one frame. This happens in nearly every cycle while presence keeps failing, the most common locked-screen situation. A tester following the spec would assert "angle 0 at entry" and make the snap an immutable contract. "(two quarters)" in R2 also contradicts "next multiple of 180°", which can be one quarter plus a fraction. Result: FINDING (MAJOR, R2-1).
- The concurrency test (N callers, max concurrency 1, one request per 250 ms with the injected clock) can fail against an implementation that skips the lock or reads the cache outside it. Result: PASS.

## R2-4. Findings (round 2)

1. **[MAJOR] R2-1 — The pending star snaps back to 0° every time a new scan starts while the previous turn is still finishing.**
   - **Why:** R9 restarts the loop at every new scan (~4.24 s cadence while presence fails). The R2 finish rule ("continue to the next multiple of 180°") runs until ~4.8 s. A re-anchor "at the scan entry" therefore resets the angle from about 120–170° to 0° in one frame, almost every cycle.
   - **Required plan change:**
     - Specify the re-entry rule: a new Scanning key arriving while the star is still turning keeps the current phase (the anchor is shifted so the angle stays continuous, and the 4.8 s cap counts from the new entry). Or: the new loop starts only once the finishing turn has reached its multiple of 180°.
     - Fix the "(two quarters)" wording: the finish is at most two quarters, to the next multiple of 180°.
     - Add a scene test that replays the 4.24 s cadence (Scanning 0–3.24 s, NotRecognized, Scanning at 4.24 s and at 4.74 s detection) and asserts no frame-to-frame angle jump larger than one easing step at a 33 ms frame period, in the `[0, 180)` circular metric.
2. **[MINOR] R2-2 — The cache is not keyed by the socket.** `--socket` (tests, diagnostics) shares the lock file and record with the real daemon's clients. Required change: bypass the shared cache when the socket is not `DEFAULT_SOCKET_PATH`, or put the socket path (or a hash of it) in the record and treat a mismatch as a miss. Test both directions.
3. **[MINOR] R2-3 — Lock-file and directory checks allow other users to read the cache.** Required change:
   - Require `mode & 0o077 == 0` on the opened file (`fstat`), otherwise Unavailable.
   - Require the directory to have no group/other bits at all (or at least no read/exec bits), matching `/run/user/<uid>` 0700.
   - Open the directory with `O_DIRECTORY | O_NOFOLLOW`, `fstat` that descriptor, then `openat` the lock file relative to it, removing the check-then-open race.
   - Test a 0644 pre-existing file, a 0755 directory, a symlinked directory, and a hard-linked file.
4. **[MINOR] R2-4 — Cache clock and the `None` clock are unspecified.**
   - `CLOCK_MONOTONIC` stops during suspend, so a record written just before suspend reads as fresh for up to 250 ms after resume. Use `CLOCK_BOOTTIME` for the stamp, the project precedent in `presence/worker.rs`. The scene clock may stay monotonic.
   - Specify that `now_ms() == None` means no cache read and no cache write, only a direct bounded exchange still under the lock (or Unavailable), and test it.
   - State that the stamp is the completion time of the exchange.
5. **[MINOR] R2-5 — Document the residual auth-reserve case.** With soos-gui preview and the soos-remote camera view both connected (2 regular permits), the single lockstatus connection still uses the auth-reserve permit for its duration. Record this in the ADR and in `Docs/LOCKSCREEN_WIDGET.md` as an accepted residual (bounded to one short exchange per 250 ms), so the corrected claim stays exact.
6. **[MINOR] R2-6 — Back-off across processes and missing-runtime-dir diagnostics.**
   - Store consecutive-failure back-off in the cache. For example, an `unavailable` record stays valid for `ERROR_BACKOFF_MS[n]` with a failure counter field (record still ≤ 64 bytes). An old daemon or a refusal then gets one request per back-off step from the whole user, not ~4 per second with a daemon warning each. A lock timeout must not count as a daemon failure in `Backoff`.
   - When `XDG_RUNTIME_DIR` is absent or unsafe, the widget writes one stderr line at start (never the state) so the owner can diagnose a permanently idle widget. `--once` stays silent.
7. **[MINOR] R2-7 — R9 wording.** With a restart for every new scan, the star turns almost continuously while presence keeps failing (3.24 s of Scanning plus the finishing turn in every ~4.24 s cycle). The per-scan 4.8 s cap does not by itself meet the WCAG 2.2.2 rationale cited. Keep it as an explicit owner-to-confirm item stating this, and list it in the PR body.
8. **[MINOR] R2-8 — Unverified locker behaviour.** R10 states that outputs beyond `MAX_OUTPUTS` or over budget make "swaylock-plugin show its own colour there". This is not established by the man page facts. Phrase it as an assumption and add it to the owner row LKA14 (e.g. a check with a budget forced low via a test-only flag, or a documented manual observation).

## R2-5. Verdict (round 2)
Round-2 verdict (superseded by round 3 above): REVISION_REQUIRED

One MAJOR finding remains (R2-1). R2-2 to R2-8 are MINOR and can be absorbed in the same revision.

---

# Round 1 (history)


## R1-1. Coverage Matrix

| Acceptance line / TDD test | Spec element | Status |
|---|---|---|
| #352 ADR: rendering dependencies cargo-deny clean, licences listed, why no zbus | §8 new ADR items (5), (6); §2.2 dependency list | Mapped |
| #352 invariant: the bridge JSON has no score / reason / frame / embedding / template / session field | `format::json` prose (§2.1 "No output contains …") | **Partial**: not in the §7 static-invariant list (only "dependency allow-lists, forbid-unsafe, no seat, no stderr, docs, pam untouched") — Finding 3 |
| #352 invariant: new crates carry `#![forbid(unsafe_code)]` | §1 `test_business_crates_forbid_unsafe_code` + `BUSINESS_CRATES`; §7 | Mapped (lib.rs only; bin roots not covered — Finding 7) |
| #352 invariant: no `unwrap`/`expect` in production paths | — | **Unmapped** — Finding 3 |
| #352 invariant: nothing biometric logged | §2.1 "the crate has no logger"; dependency allow-list excludes `tracing` | Mapped |
| `--once` exits in ≤ 300 ms incl. daemon down / socket missing / stalled reply | `EXCHANGE_BUDGET_MS` 250, `ONCE_EXIT_BUDGET_MS` 300, §7 binary tests | Mapped (test design — Finding 12) |
| `--once` on failure: exit 0, nothing on stderr | §2.1 `main.rs`; §4 | Mapped with a recorded deviation (clap usage errors on stderr) and a stdout-panic gap — Finding 6 |
| `--once` failure prints "the Unavailable text" (epic) vs Q2 three visible states | §8 drift: Unavailable renders as Idle; owner-to-confirm 1 | Mapped (drift recorded) |
| `--watch` ≤ 4 Hz, bounded line length | `MIN_POLL_INTERVAL_MS` 250, `MAX_JSON_LINE_BYTES` 96, `Backoff` | Mapped (line-length rationale wrong — Finding 5) |
| `--watch` / bridge tested against the mock daemon | §7 `UnixListener` mock | Mapped (test power — Finding 12) |
| §14 client contract: strict decode order, ≤ 1 connection, ≤ 4 Hz, back off on refusal / EOF | §2.1 exchange steps 4–6, `Backoff`, one connection per poll | Mapped for `--watch` / widget; **not achievable for `--once`** (stateless processes cannot back off) — Finding 1 |
| Widget renders under `swaylock-plugin --command-each` (headless/nested test or documented manual check) | §2.2 `wayland.rs` "owner rows"; plan documents `--command` as default | **Partial**: LKA rows not enumerated; `--command-each` load not analysed — Findings 1, 3 |
| Widget frame rate capped, CPU idle when state stable | `AMBIENT_REPAINT_MS` 33, `FrameSpec::animating`, timer not re-armed | Mapped (duty cycle under continuous presence scans not analysed — Finding 9) |
| Reduced motion supported | `MotionPrefs`, `--reduce-motion`, `SOOS_REDUCE_MOTION`; owner item 4 (no portal) | Mapped |
| Killing the widget never affects the locker; swaylock-plugin restarts it; password unlock keeps working | §2.2 exit codes, `SETUP_FAILURE_EXIT_DELAY_MS` | **Partial**: no named matrix row — Finding 3 |
| hyprlock snippet documented and verified on hardware | `packaging/lockscreen/hyprlock-soos.conf`, `Docs/LOCKSCREEN_WIDGET.md` | Partial: owner row not enumerated — Finding 3 |
| Fail-closed: no adapter can unlock, start a scan or send input | §2.2 no seat, §4 "no component can unlock …", §7 "no seat binding" invariant | Mapped |
| Removing / crashing an adapter leaves password and presence unlock byte-for-byte unchanged; PAM untouched | §1 "Not touched", §7 `pam_soos.so` untouched by the diff | Mapped |
| Epic risk: polling must not starve PAM | §5 "auth-reserved slot … never needed" | **Incorrect claim** — Finding 1 |
| Epic risk: kill switch `/etc/soos/lockwidget.disable` | — | Unmapped (stated for GNOME in the epic) — Finding 14 |
| Docs: `Docs/LOCKSCREEN_WIDGET.md`, `IPC_PROTOCOL.md`, `DISTRIBUTION_DEPLOYMENT.md` §5 | §1 docs list | Mapped (`Docs/README.md` index missing — Finding 8) |
| Matrix rows + walkthrough | LKA1–LKA16, LKS17, walkthrough 214 | Partial: rows only counted, not specified — Finding 3 |
| Q1 swaylock-plugin + hyprlock first | §1 scope | Mapped |
| Q2 only Scanning / Idle / Not recognized at the locked screen | `Presented`, `format`, `Scene` | Mapped, except `--once --format json` showing `available` and `sequence` — Finding 4 |
| Q3 animate for PAM and presence scans alike | consumes the daemon board unchanged | Mapped |
| Brand + motion system (#358) | `palette.rs`, `motion.rs`, `scene.rs` | Mapped; star-symmetry error — Finding 2; token naming — Finding 11 |

## R1-2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `smithay-client-toolkit` 0.20.0 already locked with `default-features = false, features = ["calloop"]` through `smithay-clipboard` 0.7.3; no new crate | `Cargo.lock` (sctk 0.19.2 and 0.20.0 both locked); `~/.cargo/registry/.../smithay-clipboard-0.7.3/Cargo.toml` `[dependencies.sctk] version = "0.20.0", features = ["calloop"], default-features = false` | as cited; licence MIT; `reexports::{calloop, calloop_wayland_source, client, protocols_wlr}` exist under `calloop` | Yes |
| sctk duplicates do not disturb `deny.toml` | `deny.toml` skips only `smithay-client-toolkit@0.19.2`, `calloop@0.13.0`, `calloop-wayland-source@0.3.0`, `thiserror@1.0.69`; new crates depend on the newest versions | no skip reason becomes stale; the comment "getrandom@0.4.3 kept: direct dependency of …" (deny.toml:72) gains a dependent | Yes (comment drift — Finding 8) |
| `MAX_MESSAGE_SIZE` 4096, `MAX_SERVICE_LEN` 64, `REQUEST_ID_LEN` 32, `CURRENT_VERSION` 1 | `crates/protocol/src/types.rs:14,18,43,46` | 1 / 4096 / 32 / 64 | Yes |
| `LockWidgetState` Idle 0 / Scanning 1 / NotRecognized 2, raw byte; `LockStateResponse { version, state, sequence }`; `is_current()` | `crates/protocol/src/types.rs:341-406` | as cited; `LOCK_STATE_RESPONSE_MAX_PAYLOAD` 12 | Yes |
| `encode_request` = tagged client frame | `crates/protocol/src/message.rs:101` (trailer `0xA0`) | as cited | Yes |
| Strict decode, `LockStateResponse` and `Response` never decode as each other | `codec::decode_payload` (no tag tolerated for responses), `Docs/IPC_PROTOCOL.md` §14 | as cited | Yes |
| Daemon quota 8 `LockState` / s per UID | `crates/daemon/src/lock_state.rs:61,64` | `LOCK_STATE_MAX_REQUESTS_PER_SEC` 8, window 1 s; refusal = `Response` `ProtocolError`/`RateLimited` (`dispatcher.rs:1221`) | Yes |
| Session check bound 250 ms | `lock_state.rs:71` `LOCK_STATE_SESSION_CHECK_TIMEOUT_MS` | 250 | Yes |
| Per-UID regular cap 2 | `crates/daemon/src/limits.rs:23` | `DEFAULT_MAX_CONNECTIONS_PER_UID` 2; `AUTH_RESERVED_CONNECTIONS_PER_UID` 1 | Yes |
| "The auth-reserved slot of `pam_soos.so` is never needed by these clients" | `dispatcher.rs:473-479`: on `PerUidCap` **every** peer falls back to `try_acquire_auth_reserve`; a non-`Auth` message on it is closed with `warn!` (`dispatcher.rs:632-643`); refusals log `warn!` "Connection limit reached" | any third concurrent connection of the UID consumes the single reserve | **No** — Finding 1 |
| Daemon closes a connection after a client EOF without error; refusal on cap closes without answer | `handle_connection` loop (`UnexpectedEof` → `break`); `Err(ConcurrencyLimitReached)` → stream dropped | as cited (client sees EOF → `Io`) | Yes |
| Bounded client pattern | `crates/admin-cli/src/test_pam.rs:313-460` (`CONNECT_RETRY_INTERVAL` 5 ms, `EAGAIN|EINTR|EINPROGRESS|EALREADY`, re-armed `SO_*TIMEO`, `nix::time::clock_gettime`) | as cited; admin-cli uses `nix` features `time`, `signal` | Yes |
| Palette `BLUE` 00 47 BB, `PALE` ED F1 FF, `PALE_4` C9 D6 FA, `PINK` E5 9B DC, `WARN_BORDER` F5 B5 47 | `crates/gui/src/theme.rs:24,30,36,46,69` | as cited | Yes |
| `STAR_SPIKES`: two concave spikes, top-right and bottom-left quadrants minus quarter discs of radius 0.5 centred on the outer corners | `crates/gui/src/brand.rs:155-171` | as cited (kappa 140.28 = 0.5523 × 254) | Yes |
| "a 90° turn of the four-fold mark is identical to 0°" (§2.2 Star rule) | same: only the top-right and bottom-left quadrants are filled | the mark has **2-fold (180°)** symmetry only; at 90° the filled quadrants are bottom-right and top-left (mirror image) | **No** — Finding 2 |
| Motion tokens 180 / 260 / 1200 ms, `EASE_STANDARD` (0.2, 0, 0, 1), `SLIDE_PX` 8, ambient 33 ms (≤ 30 Hz) | `Docs/UI_MOTION.md` §2 table, §9 | as cited | Yes (naming/units — Finding 11) |
| `SOOS_REDUCE_MOTION` semantics (`1`, `0`, else / > 8 bytes ignored) | `Docs/UI_MOTION.md` §8 | as cited | Yes |
| Longest `--watch` JSON line 62 bytes | computed | `{"state":"not_recognized","available":true,"sequence":18446744073709551615}\n` = **76 bytes** | **No** (bound 96 still holds) — Finding 5 |
| `NOT_RECOGNIZED_HOLD_MS` 3000, `MAX_SCANNING_MS` 10 000, reveal floor 3240 ms, presence cadence ≈ 4.24 s | `crates/daemon/src/lock_state.rs:48-58`; `Docs/IPC_PROTOCOL.md` §14; matrix LKS17 | as cited | Yes |
| `test_business_crates_forbid_unsafe_code` list (13 crates), `candid_review.sh` `BUSINESS_CRATES` | `tests/invariants/src/lib.rs:451-486` (checks `src/lib.rs` only); `scripts/candid_review.sh:203` | adding two names is a strengthening; `portable_crates_contract` and `remote_companion_contract` only require `frame` / `remote` to stay listed | Yes |
| Walkthrough 214 | `AI/walkthroughs/` highest is 212 | 213 must exist (reserved elsewhere) before 214 is used | Note |
| `.claude/skills/dev-workflow/references/project-facts.md` | `.claude/skills` → `../.agents/skills` (symlink); invariants read `.agents/...` | same file | Yes |

**Existing-test impact check (owner rule: no existing assertion may change).** I read `lib.rs` (forbid list, PAM print scan), `dependency_tooling_contract.rs` (skip-entry rules), `portable_crates_contract.rs` (forbidden list, business lists), `presence_unlock_contract.rs` PAU17 (`!content.contains("zbus")` on every `crates/*/Cargo.toml`), `lock_state_contract.rs` (IPC doc needles, PAM untouched), `ui_motion_spec_contract.rs` (UIM-S2 / S5 needles in `Docs/UI_MOTION.md`), `licence_contract.rs` (`license.workspace = true` for every member), `remote_companion_contract.rs` RMC-S4 (`!content.contains("soos-remote")`), `maintainer_hygiene_contract.rs` (workspace trees of `AGENTS.md` / `AI/ARCHITECTURE.md`, `Docs/README.md` index) and `system_onnxruntime_contract.rs` (`rust-version.workspace = true`, MSRV 1.92). **None of these assertions has to change.** The plan does require edits to files the tests read: `Docs/README.md`, the two new manifests (exact keys), and the `deny.toml` comment (Finding 8). No CRITICAL finding.

## R1-3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: a user-level widget is the only new code on the locked screen. Can it unlock, cover the prompt or take input? It binds no seat, draws on `Layer::Background` inside swaylock-plugin's nested session, and `ext-session-lock-v1` stays owned by the locker. The socket, its modes and `SO_PEERCRED` checks are unchanged, and the request carries `uid_hint = getuid()` (an unprivileged peer gets `UidMismatch` for any other UID). Result: PASS.
- Failure scenario considered: information disclosure beyond Q2. If hyprlock is set to `soos-lockstatus --once --format json`, the locked screen shows `"available":false` (daemon health) and `"sequence":N` (2 × the number of scans since the daemon started, so a person at the screen can count face attempts). Result: FINDING (MINOR, Finding 4).
- Failure scenario considered: the rate of transport errors leaking state. Every error renders Idle, the same as a real Idle, and the daemon reveal floor is consumed unchanged. Result: PASS.

### Pillar 2 — Deadlines, concurrency and resource slots (PAM untouched; applied to the new clients)
- Failure scenario considered: hyprlock with three monitors and the snippet label (`monitor =` empty, so one instance per monitor) at `update:500`. The three `--once` processes fire together. Each holds a connection for up to about 250 ms (the daemon's blocking-pool session check). Connections 1 and 2 take the UID's two regular permits. Connection 3 falls back to the **auth-reserve** permit (`dispatcher.rs:473-479`), where its `LockState` is refused with a `warn!`. If hyprlock's own `pam_soos.so` connects in that window, it gets `PerUidCap`, the daemon closes it, and PAM returns `PAM_IGNORE`. The face attempt is silently skipped and only the password works. The same happens with one regular permit already held by PAM's own in-flight `Auth` or by the `soos-remote` live camera view (a persistent user-level connection, typically used while the PC is locked), plus two overlapping polls. The quota fails too: 3 instances × 2 Hz = 6 req/s, and 2 labels give 12 > 8, so `RateLimited` makes Scanning flicker to Idle. `--once` processes cannot back off as §14 requires. `--command-each` with 5 or more outputs gives N pollers × 2 Hz > 8. Result: FINDING (MAJOR, Finding 1).
- Failure scenario considered: an unbounded exchange. One cumulative deadline covers connect, write and read, with `budget == 0` → `Timeout` before any syscall, the length checked before allocation, and one request per connection. Result: PASS.

### Pillar 3 — Panic safety & fail-closed (fail-quiet here)
- Failure scenario considered: `--once` with stdout closed (hyprlock reaped the reader), or a `print!` used for output. `print!` panics on a write error. The `catch_unwind` handler then "prints the Idle rendering" and panics again outside the guard, so the exit code is 101, not 0. The plan says "ignores a stdout write error" but does not forbid the panicking macros. Result: FINDING (MINOR, Finding 6).
- Failure scenario considered: the widget poller thread panics. The channel disconnects, the main loop keeps the last view forever (a stale Scanning with an animating star), and nothing restarts it. Result: FINDING (MINOR, Finding 10).
- Failure scenario considered: no success state. There is no `Recognized` rendering, an `Allow` refusal `Response` maps to Unavailable → Idle, and `LockView::UNAVAILABLE` is the only error sink. Result: PASS, provided the test contains an `Allow`-verdict `Response` (Finding 12).

### Pillar 4 — Dependencies
- Failure scenario considered: a new crate enters `Cargo.lock` or a duplicate-version skip goes stale. The claim was verified (sctk 0.20.0 has the exact feature set from smithay-clipboard). Using the `reexports` avoids naming `calloop` 0.13/0.14 directly. The skip reasons name only 0.19/0.13/1.x dependents, which are unchanged. Result: PASS.
- Failure scenario considered: runtime linkage. Workspace feature unification turns on `wayland-backend` `client_system` (smithay-clipboard) and `wayland-sys` with `dlib` (eframe, `Cargo.lock:4557-4566`). The widget therefore **dlopens `libwayland-client.so`** at run time, as `soos-gui` does (PKGBUILD optdepends). The plan says nothing about this or about the missing-library path. Result: FINDING (MINOR, Finding 13).
- `forbid(unsafe_code)`: declared for both lib roots. The bin roots (`main.rs`) are separate crate roots that the lib-only invariant never reads. Result: FINDING (MINOR, Finding 7).

### Pillar 5 — Data confidentiality & memory bounds
- Failure scenario considered: shm memory. Two 8K outputs give 2 × 2 × 33.5 M px × 4 B = 512 MiB of memfd for a lock-screen background. The bound holds, but the plan does not say whether one shared `SlotPool` (which only grows) or one pool per surface is used, so hot-plug churn could keep peak capacity alive. Outputs beyond `MAX_OUTPUTS` or over budget get **no surface or buffer**, which breaks swaylock-plugin's documented requirement ("promptly provide a full-size background layer surface for each output"). HiDPI (`buffer_scale`) is unspecified. Result: FINDING (MINOR, Finding 10).
- Failure scenario considered: secrets in logs. There is no logger, the request carries no secret, and the answer is three states plus a counter. Result: PASS.

### Pillar 6 — Test integrity & power
- Failure scenario considered: the plausible wrong implementation "angle wraps at 90°". The plan's own `FrameSpec::star_angle_deg in [0, 90)` contract would **enforce** the visual defect of Finding 2 as an immutable test. Result: FINDING (MAJOR, Finding 2).
- Failure scenario considered: an implementation that keeps a persistent connection, or that sends an untagged frame or a wrong `uid_hint`, passes a mock that only answers. The plan does not require the mock to assert one request per connection or the request fields. Result: FINDING (MINOR, Finding 12).
- Coverage: "no `unwrap`/`expect` in production" and "JSON field allow-list" have no invariant, and LKA1–LKA16 are only counted. Result: FINDING (MAJOR, Finding 3).
- No existing assertion has to change (§2). Result: PASS.

## R1-4. Findings

1. **[MAJOR] Aggregate per-UID load is unbounded across processes. The plan's claim that "the auth-reserved slot of `pam_soos.so` is never needed by these clients" is false.**
   - **Why:** the daemon puts any connection over the regular cap of 2 on the single auth-reserve permit. hyprlock creates one label instance per monitor when `monitor` is empty. `--command-each` runs one poller per output. `--once` processes cannot back off on `RateLimited`, as §14 requires.
   - **Failure scenarios:** a lock-screen face attempt refused with `PerUidCap` (silent `PAM_IGNORE`); `warn!` spam in the daemon journal; Scanning flickering to Idle above 8 req/s.
   - **Required plan change:**
     - (a) Remove the claim and state the real budget: regular cap 2 + 1 reserve, shared with `pam_soos.so` in the locker and with the `soos-remote` camera view.
     - (b) Bound aggregate concurrency per UID to one in-flight `LockState` exchange. For example, a non-blocking `flock` single-flight on a per-user file under `$XDG_RUNTIME_DIR` (0600, `O_NOFOLLOW`): a contended `--once` renders the last cached view if it is younger than one interval, otherwise Idle. Or justify an equivalent.
     - (c) The hyprlock snippet pins one `monitor` (or documents the per-monitor arithmetic) and uses `update:1000`. Scanning lasts at least 3240 ms and the hold is 3000 ms, so 1 Hz loses no state. An invariant pins the snippet's period.
     - (d) Give the `--command` vs `--command-each` arithmetic, with the maximum output count kept under 8 req/s.
     - (e) Add a contract test: N concurrent `--once` clients never hold more than one daemon connection, and never touch the auth-reserve permit, against a mock or a daemon-backed limiter test.
2. **[MAJOR] The star rotation rests on a false symmetry.**
   - **Why:** `STAR_SPIKES` fills only the top-right and bottom-left quadrants (`brand.rs:155-171`), so the mark is 180°-symmetric, not four-fold. With `star_angle_deg ∈ [0, 90)` and "finish the quarter then rest at 0°", the star snaps from 90° (a mirrored star) back to 0° at every quarter boundary (every 1.2 s while scanning) and when it stops.
   - **Required plan change:**
     - Use an angle domain of [0, 180).
     - The pending turn accumulates quarters, and leaving Scanning finishes to the next multiple of 180° (at most two quarters), consistent with UI_MOTION §5: "0° in every state".
     - Tests assert that the rendered star at 90° differs from 0°, equals it at 180°, and that consecutive frames never jump by more than one easing step.
3. **[MAJOR] Coverage gaps against #352 acceptance.**
   - **Unmapped or partial lines:**
     - (a) No invariant forbids `unwrap`/`expect` in the production code of the two crates.
     - (b) No invariant pins the JSON key set (`state`, `available`, `sequence` only) and the absence of score / reason / frame / embedding / template / session fields.
     - (c) LKA1–LKA16 are only counted. The owner checks the epic requires have no named rows: renders under `--command-each`; killing the widget never affects the locker and swaylock-plugin restarts it; password and presence unlock unchanged; hyprlock snippet verified on hardware; CPU idle when stable; reduced motion on hardware.
   - **Required plan change:** list LKA1–LKA16 with the acceptance line and test or owner check each one covers, and add (a) and (b) to §7.
4. **[MINOR] `--once --format json` can put non-Q2 data on the locked screen** (`available`, and `sequence` = 2 × scans since daemon start). Required change: either remove `json` from `--once`, or have `Docs/LOCKSCREEN_WIDGET.md` and the snippet invariant state that hyprlock labels use `text` / `glyph` / `label` only, and record this in the ADR.
5. **[MINOR] The `MAX_JSON_LINE_BYTES` rationale is wrong.** The longest line is 76 bytes (`not_recognized`, `true`, `u64::MAX`), not 62; 96 still bounds it. Required change: fix the figure and test it with `sequence = u64::MAX`.
6. **[MINOR] `--once` output and exit-0 guarantee are under-specified.**
   - **Why:** `print!`/`println!` panic on a closed stdout, and a second panic in the handler exits 101. clap usage errors write to stderr, while epic #352 says `--once` "never writes to stderr".
   - **Required plan change:**
     - Mandate `std::io::stdout().lock().write_all` with errors ignored.
     - Forbid `print!`/`println!`/`eprint!`/`eprintln!`/`dbg!` in `crates/lockstatus/src`. One `--watch` panic line can be allowed through a single named function.
     - Test `--once` with stdout closed (exit 0, empty stderr).
     - Record the clap-on-misconfiguration deviation in the ADR, or use `try_parse` and render Idle when `--once` is present.
7. **[MINOR] `#![forbid(unsafe_code)]` must also be in `crates/lockstatus/src/main.rs` and `crates/lock-widget/src/main.rs`** (separate crate roots; `test_business_crates_forbid_unsafe_code` reads only `lib.rs`). The new contract module should assert both roots.
8. **[MINOR] Touched-file list is incomplete for existing invariants** (no assertion changes, but these files must be edited, or tests fail):
   - `Docs/README.md` must index `LOCKSCREEN_WIDGET.md` (`maintainer_hygiene_contract::test_docs_index_lists_every_docs_page`).
   - Both new manifests need:
     - `license.workspace = true` (licence contract);
     - `rust-version.workspace = true` (SORT-S2);
     - `publish.workspace = true` and `[lints] workspace = true`.
   - Neither new manifest may contain the substrings `zbus` (PAU17 raw-text scan, comments included) or `soos-remote` (RMC-S4).
   - Update the `deny.toml:72` comment listing the direct dependents of `getrandom@0.4.3`.
   - Walkthrough 214 assumes 213 is taken; the highest on `main` is 212.
9. **[MINOR] Motion duty cycle under continuous presence scanning is not analysed.**
   - **Why:** while the screen is locked and on, a failing presence scan repeats about every 4.24 s (LKS17). The widget therefore shows the pending star for at least 3.24 s, then a shake, almost continuously, and up to 10 s (`MAX_SCANNING_MS`) for a long PAM scan. UI_MOTION §5 keeps the pending loop at 5 s or less (WCAG 2.2.2), and the epic expects idle CPU when stable.
   - **Required plan change:** bound the pending loop per scan (for example static after 5 s) or record an owner-confirmed deviation, and state the expected repaint duty cycle.
10. **[MINOR] Widget robustness gaps.**
    - (a) Poller-thread death (channel disconnected) is unspecified. Required: render Idle, then exit 1 after `SETUP_FAILURE_EXIT_DELAY_MS` so swaylock-plugin restarts it.
    - (b) The widget's panic exit code is unspecified.
    - (c) Outputs beyond `MAX_OUTPUTS` or over the pixel budget get no buffer, contrary to swaylock-plugin's "full-size background surface for each output". Specify the fallback and test it.
    - (d) State one `SlotPool` per surface, dropped with the surface, and the `buffer_scale` / HiDPI handling.
    - (e) A single transient poll error (Unavailable → Idle key) followed by the same `(NotRecognized, seq)` replays the shake and re-anchors the star. Specify whether Unavailable keeps the previous key for one back-off, and test it.
11. **[MINOR] Motion token naming and units deviate from `Docs/UI_MOTION.md` §2** ("same names … Rust durations as `f32` seconds"). The plan uses `T_SHORT_MS: u64`, `T_PENDING_MS`, and so on. Required change: either mirror `T_SHORT` / `T_MEDIUM` / `T_PENDING` (f32 s) or record the lock-screen naming in the ADR. Make the new invariant also cross-check `crates/gui/src/theme.rs` once its tokens exist (forward-compatible, as UIM-S2 does).
12. **[MINOR] Test power.**
    - (a) The mock daemon must assert:
      - a tagged `Request` with `kind = LockState`, `uid_hint = getuid()`, `service = "soos-lockstatus"`, and `version = 1`;
      - exactly one request per connection (EOF after the answer).
    - (b) The refusal case must include a `Response` with verdict `Allow` and assert Unavailable.
    - (c) Frame-length boundaries: 0 refused and 4097 refused, both before allocation; 4096 read and then decoded.
    - (d) Unavailable and Idle renderings are byte-identical in every non-JSON format.
    - (e) `--once` timing with a stalled listener, at ≤ 300 ms wall clock, with exit 0 and empty stderr.
13. **[MINOR] Runtime library dependency.** In the workspace build, the widget dlopens `libwayland-client.so` (feature unification: `client_system` + `wayland-sys`/`dlib`). Document it in `Docs/LOCKSCREEN_WIDGET.md` for the PR 6 packaging step, and make the missing-library path (`connect_to_env` error) the delayed exit 1.
14. **[MINOR] Kill switch.** The epic's risk section names `/etc/soos/lockwidget.disable`. Record in the ADR whether the swaylock-plugin widget honours it in this PR (it should render the plain background rather than exit, because swaylock-plugin restarts exits) or whether it is deferred to the GNOME PR.

## R1-5. Verdict
Round-1 verdict (superseded by round 2 above): REVISION_REQUIRED
