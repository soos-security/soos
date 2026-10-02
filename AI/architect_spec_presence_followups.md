# Architect Spec — GitHub #325: presence auto-unlock review follow-ups (#324)

- **Branch**: `fix/presence-review-followups` (GitHub-only issue, not registered in
  `scripts/sync_issue.py`; commits carry `Closes #325`)
- **Base**: `origin/main` `35a708a` (presence auto-unlock, GitHub #323 / PR #324)
- **Revision**: 3 (round-1 F1–F6, O1–O2 and round-2 R2-F1, R2-F2 of `AI/plan_evaluator_report.md` addressed, see §11)

## 1. Scope & Blast Radius

| Item | Production files | Consumers / tests touched |
|---|---|---|
| 1 fresh attempt stamp | `crates/daemon/src/presence/worker.rs` (step 10) | none (internal) |
| 2 reachable connect bound | `presence/logind.rs` (trait method `connect`, `ZbusLogind`), `presence/worker.rs` (step 4), `presence/mod.rs` (docs of `DBUS_CALL_TIMEOUT_MS` / `DBUS_CONNECT_TIMEOUT_MS`) | `PresenceLogind` impls: `ZbusLogind`, `MockPresenceLogind` (`crates/daemon/tests/common/mod.rs`, unchanged thanks to the default method) |
| 3 PAM line scan superset + `/etc/pam.conf` | `presence/account.rs`, `presence/mod.rs` (`DEFAULT_PAM_CONF`) | every test that builds a `SystemAccountGuard` over existing PAM directories (setup migration, §9) |
| 4 one account check in flight | none (behaviour exists, `check_account`) | new behavioural test only |
| 5 no logind polling without templates | `crates/biometric-store/src/store.rs` (`has_enrolled_template`, `MAX_ENROLLMENT_PROBE_ENTRIES`), `presence/worker.rs` (step 3b) | daemon only; `soos-enroll`, `soos-gui` unaffected (additive API) |
| 6 flaky admin-cli test | **none** (diagnosis §8: test-only timing assumption) | open question for the owner |

Docs (Phase 6): `Docs/DAEMON.md` §6 (tick steps 1–2, account-guard bullet), ADR amendment line in
`AI/DECISIONS.md` (presence ADR), `AI/VERIFICATION_MATRIX.md` new component, walkthrough 176.
`crates/daemon/src/main.rs` is unchanged (the worker reaches the store through
`PipelineComponents::biometric_store`). No new dependency, no PAM/IPC/auth-path change.

## 2. Types & Signatures

### 2.1 Item 1 — `worker.rs` step 10

The attempt is stamped with a clock read taken **under the policy write lock, immediately
before** `record_attempt_with_reserve`; the same value is passed to `mark_scan_started`.

```rust
// 10. One attempt in the shared limiter, keeping the PAM reserve, stamped with a fresh read.
let policy = Arc::clone(&self.pipeline.policy);
let mut engine = policy.write().await;
let attempt_ns = match (self.clock_fn)() {
    Ok(ns) if ns >= now_ns => ns,
    // Clock error, or a reading older than step 3: no attempt, no scan.
    _ => { drop(engine); return self.skip(SkipReason::ClockUnavailable); }
};
if engine.record_attempt_with_reserve(uid, attempt_ns, PRESENCE_RESERVED_ATTEMPTS).is_err() {
    drop(engine);
    return self.skip(SkipReason::RateLimited);
}
drop(engine);
self.tracker.mark_scan_started(&candidate.id, attempt_ns);
```

- Clock error at the re-read ⇒ `Skipped(ClockUnavailable)`: no attempt recorded, no camera wake,
  no inference, no unlock (the candidate template is dropped/zeroized with the candidate).
- A re-read **older** than the step-3 reading (`attempt_ns < now_ns`, impossible on
  `CLOCK_MONOTONIC`, reachable only through an injected clock) ⇒ `ClockUnavailable` (fail closed;
  a backwards stamp would let the attempt expire early).
- `attempt_ns == now_ns` is accepted.

### 2.2 Item 2 — connection opened outside the call bound

Decision: **connect outside `bounded()`** (option "a"). Lowering `DBUS_CONNECT_TIMEOUT_MS`
below the call bound (option "b") was rejected: the connect step would then still eat into the
500 ms snapshot budget, and the documented meaning of each bound would stay entangled.

```rust
pub trait PresenceLogind: Send + Sync + 'static {
    /// Opens the system-bus connection when none is held (no-op when one is). The worker
    /// calls it once per tick, right before the snapshot, bounded by `DBUS_CONNECT_TIMEOUT_MS`
    /// and outside the `DBUS_CALL_TIMEOUT_MS` bound. The default (doubles without a
    /// connection) succeeds at once.
    fn connect(&self) -> impl Future<Output = Result<(), PresenceLogindError>> + Send {
        std::future::ready(Ok(()))
    }
    // seat_sessions / session_state / lid_closed / unlock_session unchanged
}
```

`ZbusLogind`:
- `async fn connect(&self)`: slot held ⇒ `Ok(())`; otherwise the existing builder
  (`Builder::address(SYSTEM_BUS_ADDRESS)`, `method_timeout(DBUS_CALL_TIMEOUT_MS)`,
  `max_queued(MAX_QUEUED_MESSAGES)`) and `build()` under
  `tokio::time::timeout(DBUS_CONNECT_TIMEOUT_MS)` (kept as defence in depth); build error ⇒
  `BusUnavailable`, expiry ⇒ `Timeout`; success stores the connection.
- **Only `connect` opens a connection**: `Builder::address(` and `.build()` appear in
  `logind.rs` exactly once each, inside the body of `ZbusLogind`'s `async fn connect(&self)` (not
  in a helper it calls; enforced textually, F6). The former lazy `connection()` helper becomes a pure
  accessor (e.g. `current()`) that returns the held connection or `Err(BusUnavailable)` and never
  builds one; `call()` uses it. A transport failure still drops the connection (`reset`); a later
  call of the same tick then fails with `BusUnavailable` (fail closed: the snapshot step ends the
  tick, a failed re-check is `SessionChanged`, a failed lid read does not gate as today), and the
  next tick reconnects through `connect()` after the usual backoff.
- No other change (pinned address, no object server, no name, no match).

Worker step 4 becomes:

```rust
// 4a. Connection (own bound, outside the call bound).
match tokio::time::timeout(Duration::from_millis(DBUS_CONNECT_TIMEOUT_MS), self.logind.connect()).await {
    Ok(Ok(())) => {}
    Ok(Err(err)) => return self.connect_failed(&err),            // see below
    Err(_) => return self.connect_failed(&PresenceLogindError::Timeout),
}
// 4b. Snapshot: unchanged, `bounded(self.logind.seat_sessions())`.
```

`connect_failed` = the existing snapshot-failure path: `logind_failed(err)` (backoff + warn on
transition), `tracker.clear()`, `Skipped(LogindUnavailable)` (never `TooManySessions`).

Documented bounds (doc comments in `presence/mod.rs`, values unchanged):
- `DBUS_CALL_TIMEOUT_MS = 500`: bound of **each logind step of the worker** on an established
  connection — the whole `seat_sessions` **snapshot** as one unit (one `ListSessions` plus one
  `GetAll` per seat session, at most 1 + `MAX_PRESENCE_SEAT_SESSIONS` round trips),
  `session_state` (`GetSession` + `GetAll`), `lid_closed` and `unlock_session`; every single
  D-Bus **round trip** inside a step is bounded by the same value (zbus `method_timeout` and the
  per-call timeout of `ZbusLogind::call`), so the step bound is the binding one. It never covers
  opening the connection. The doc comment must contain the words `snapshot` and `round trip`.
- `DBUS_CONNECT_TIMEOUT_MS = 1000`: bound of `PresenceLogind::connect()` (authentication
  handshake and `Hello`), applied by the worker before the snapshot, **outside** the call bound;
  only `connect()` opens a connection. The doc comment must contain `connect()` and `outside`.

### 2.3 Item 3 — `account.rs`

New constant (`presence/mod.rs`): `pub const DEFAULT_PAM_CONF: &str = "/etc/pam.conf";`

`SystemAccountGuard` gains `pam_conf: PathBuf` (default `DEFAULT_PAM_CONF`) and
`#[must_use] pub fn with_pam_conf(mut self, path: PathBuf) -> Self`.

libpam facts (verified in Linux-PAM v1.7.1 `libpam/pam_handlers.c` `_pam_init_handlers` and
`pam.conf(5)`): the directory mode is used when `/etc/pam.d`, `/usr/lib/pam.d` **or**
`VENDORDIR/pam.d` is a directory (`stat` + `S_ISDIR`, symlinks followed); `/etc/pam.conf` is read
**only when none is**, except in builds with the non-default `read-both-confs` option, where
`pam.conf` is also consulted for a service that has no file in the directories.

New step at the start of `check_pam_stacks` (step 3), before the directory scan:
1. `any_dir` = some entry of `pam_dirs` for which `std::fs::metadata` (follows symlinks) says
   "directory". `NotFound` or a non-directory = not a directory; any other `metadata` error ⇒
   `Undeterminable`. (`pam_dirs` mirrors libpam's three directories; an empty list means none.)
2. `std::fs::symlink_metadata(pam_conf)`: `NotFound` ⇒ nothing more; other error ⇒
   `Undeterminable`; present (any type, dangling symlink included):
   - `!any_dir` ⇒ `Undeterminable` (libpam reads only `pam.conf`, which the guard does not model —
     issue item 3);
   - `any_dir` ⇒ **scanned like a stack file** with `read_source(pam_conf, MAX_PAM_FILE_BYTES)`
     + UTF-8 + `scan_pam_faillock_options` (the leading service field is just one more token
     before the module token): a policy option ⇒ `Undeterminable`; `Directory`, `Absent` (race),
     a special file, an oversized or non-UTF-8 file ⇒ `Undeterminable`; no option ⇒ continue.
     Decision for "both exist": libpam's default build ignores `pam.conf`, but scanning it is
     cheap, covers `read-both-confs` builds and a libpam without `VENDORDIR` that falls back to
     `pam.conf` while `/usr/etc/pam.d` exists, and only ever over-refuses (fail closed). A
     comment-only `pam.conf` (Debian/Ubuntu ship one) stays usable.
3. The existing directory scan is unchanged.

**PAM line scan: a true superset of libpam (behaviour change, fail closed).** The round-1
evaluation (F1) showed that the current token-based matcher can *under*-detect compared with
libpam v1.7.1, on malformed but root-written files: `auth required pam_faillock.so x\u{a0}[a deny=1`
(splitting at the no-break space opens a `[` token that libpam never sees, and the unterminated
bracket swallows `deny=1`) and `auth [default=1 \ # note` followed by
`auth required pam_faillock.so deny=1` (the comment is dropped before the continuation check, the
lines are joined and the unterminated bracket swallows the second line; libpam ends any physical
line holding a `#`, `libpam_internal/pam_line.c` `_pam_str_prepare`). Both make the guard
evaluate the tally against `faillock.conf` instead of `deny=1`.

New rule for one logical line (the logical-line assembly is unchanged: `#` comment dropped per
physical line, a remaining trailing `\` joins the next line): the line sets faillock policy iff it
contains the substring `pam_faillock.so` and the text **after its first occurrence** contains, as
a plain substring, one of `PAM_POLICY_ARGUMENT_PREFIXES` (`dir=`, `deny=`, `fail_interval=`,
`unlock_time=`, `root_unlock_time=`, `admin_group=`, `conf=`) or `even_deny_root` — whatever the
tokenization, brackets or whitespace. Why it is a superset: libpam joins two physical lines only
when the first has no `#` and ends with `\` (after ASCII blanks), and the guard joins in every
such case (and more), so each libpam logical line lies inside one guard logical line with the
same comment stripping; libpam's module token ends with `pam_faillock.so` and every argument
follows it literally (the continuation `\` becomes a space, `\]` inside brackets only affects
`]`), so any option libpam passes to `pam_faillock.so` appears as a substring after the first
`pam_faillock.so` of the guard line. All existing positive and negative scan tests keep their
verdicts (verified against a reference: options before the module, other modules, commented
options and option-free lines stay undetected). `pam_tokens` and the token-based
`line_sets_faillock_policy` are removed or reduced to the substring rule (no dead code).

**Stacks the guard cannot follow (round-2 R2-F1).** libpam's `_pam_open_config_file`
(`pam_handlers.c` v1.7.1) opens an `include` / `substack` target that is absolute
(`/etc/security/site-auth`), nested (`sub/file`) or relative (`../x`); the guard scans only the
top level of its three directories (subdirectories are skipped, PAU25). So a logical line also
"sets faillock policy" (⇒ `Undeterminable`) when, after whitespace splitting
(`char::is_whitespace`, a superset of libpam's ASCII blanks), some token equals `include`,
`substack` or `@include`, all ASCII case-insensitive (upstream libpam compares `include` /
`substack` with `strcasecmp`; Debian's `031_pam_include` patch, still applied in pam
1.7.0-8, compares `@include` with `strcasecmp` too — verified on sources.debian.org, closes
round-3 observation O4), and a later token contains `/`. Plain names (`auth include system-auth`,
`@include common-auth`, `auth substack password-auth`) resolve inside the scanned directories
and stay usable. A path anywhere else on the line (`pam_env.so envfile=/etc/environment`,
`/usr/lib/security/pam_unix.so`) is not affected.

Doc comments: `scan_pam_faillock_options` must state that the scan **deliberately
over-detects** — it is a **superset** of what libpam passes to `pam_faillock.so` and must never
be narrowed (any option substring after the module name counts, independent of ASCII or
**Unicode whitespace**, brackets and quoting; a `\` left before a `#` comment joins the next
line although libpam ends that line; an `include` / `substack` of a path is refused), and that a
false positive only refuses presence. It must also state the known limit (R2-F2): the module is
recognised by its name `pam_faillock.so`, so a renamed copy or a differently named symlink to it
is not seen (root-only configuration; recorded in the ADR amendment as part of the accepted risk
(d) of the presence ADR). The
`check_pam_stacks` doc (or the module doc) states the `/etc/pam.conf` rule. Required phrases in
`account.rs` doc comments: `over-detect`, `superset`, `Unicode whitespace`, `pam.conf`.

### 2.4 Item 4 — no production change

`PresenceWorker::check_account` already holds `account_check_in_flight` (swap + `InFlight` drop
in the blocking closure). Only a behavioural test is added (auditor constraint 9 / T11 of #323).

### 2.5 Item 5 — no logind traffic while nobody is enrolled

`crates/biometric-store/src/store.rs`:

```rust
/// Directory entries examined by `BiometricStore::has_enrolled_template`.
pub const MAX_ENROLLMENT_PROBE_ENTRIES: usize = 4096;

impl BiometricStore {
    /// Whether the store holds at least one template file, without reading or decrypting any.
    ///
    /// `true` at the first entry named `<uid>.cbor.enc` (canonical decimal `u32`: no sign, no
    /// leading zero except `0` itself) whose type, not followed, is a regular file; symlinks,
    /// directories, temporary files and other names never count. At most
    /// `MAX_ENROLLMENT_PROBE_ENTRIES` entries are examined: a directory with more entries and no
    /// match among the first ones is an error.
    ///
    /// # Errors
    /// `BiometricStoreError::Io` when the directory cannot be listed (missing included), any
    /// variant (recommended `InvalidPath`) when the bound is exceeded.
    pub fn has_enrolled_template(&self) -> Result<bool, BiometricStoreError>;
}
```

Bounds: exactly `MAX_ENROLLMENT_PROBE_ENTRIES` non-matching entries ⇒ `Ok(false)`; one more ⇒
`Err`. Canonical check: `uid.to_string() == stem`.

Worker step **3b** (after the step-3 clock, before 4a — so before any D-Bus traffic, connection
included), re-evaluated **every tick** (one `getdents` of a small root-only directory per second;
a new enrollment is picked up within one tick, no restart):
- `Ok(true)` ⇒ continue (per-UID checks in step 7 unchanged);
- `Ok(false)` ⇒ `tracker.clear()`, `Skipped(SkipReason::NotEnrolled)`;
- `Err(_)` ⇒ `tracker.clear()`, `Skipped(SkipReason::TemplateStoreError)`.

The existing `NotEnrolled` variant is reused (doc broadened: "the session owner has no template,
or the store holds none at all and logind is not polled"); no new `SkipReason`, so the stable log
codes stay valid. `presence_worker_tests::test_pau_unusable_templates_cost_no_attempt_and_no_camera`
needs a setup-only migration (§9, F2). The probe is a synchronous `read_dir` on the async
worker, deliberately, like the existing per-UID `get()` of step 7: one bounded listing (at most
`MAX_ENROLLMENT_PROBE_ENTRIES` entries) of a local, root-only directory per second (O1). `tracker.clear()` (same rule as the kill switch): the
worker does not observe sessions while skipping, so a lock period seen before cannot be assumed
unbroken; the grace restarts after the next enrollment. The backoff state is untouched.

## 3. Constants & Config

| Name | Location | Value | Sentinel semantics |
|---|---|---|---|
| `DBUS_CALL_TIMEOUT_MS` | `presence/mod.rs` | 500 (unchanged) | bound per worker logind step (doc only) |
| `DBUS_CONNECT_TIMEOUT_MS` | `presence/mod.rs` | 1000 (unchanged) | bound of `connect()`, now reachable |
| `DEFAULT_PAM_CONF` | `presence/mod.rs` | `/etc/pam.conf` | absent = not consulted |
| `MAX_ENROLLMENT_PROBE_ENTRIES` | `biometric-store/src/store.rs` | 4096 | `> bound` ⇒ `Err` ⇒ `TemplateStoreError` skip |

No `daemon.toml` key is added.

## 4. Error Taxonomy

| Condition | Result | Unlock |
|---|---|---|
| clock error / regression at the step-10 re-read | `Skipped(ClockUnavailable)`, no attempt | never |
| `connect()` error or > `DBUS_CONNECT_TIMEOUT_MS` | `Skipped(LogindUnavailable)`, backoff, tracker cleared | never |
| call after a mid-tick transport reset | `BusUnavailable` → existing mapping | never |
| no template in the store | `Skipped(NotEnrolled)`, no D-Bus, tracker cleared | never |
| store listing error / probe bound | `Skipped(TemplateStoreError)`, no D-Bus, tracker cleared | never |
| `pam.conf` present, no PAM directory | `AccountRefusal::Undeterminable` | never |
| `pam.conf` with a faillock option, unreadable, special, oversized, non-UTF-8 | `Undeterminable` | never |

PAM: untouched (`PAM_IGNORE` paths unchanged).

## 5. Latency Budget

The PAM auth path is not touched. Worker tick worst case grows by at most
`DBUS_CONNECT_TIMEOUT_MS` (1000 ms) when a connection must be opened; ticks are sequential and
the shutdown join (500 ms, then abort) is unaffected (aborting a pending connect is safe: no
attempt, no unlock).

## 6. Invariants Touched

ARCHITECTURE invariant 6 (presence fail-closed), ADR "Presence Auto-Unlock Through logind" (1)
bounds, (8) one attempt per scan, (10) account guard; matrix PAU2 (bounded logind), PAU3
(reserve), PAU11 (clock), PAU27/PAU28 (account guard). No PAU row changes status; new rows PFU1–PFU7.

## 7. Acceptance Criteria (new matrix component `presence-review-followups`, rows PFU1–PFU7)

New PFU rows instead of extending PAU rows: PAU1–PAU29 are `✅ Verified` against the #323
contract (`AI/tester_contract_presence_unlock.md`); the follow-ups add evidence of their own, and
the repository practice for review follow-ups is a separate component (e.g.
`p2-review-followups`).

- **PFU1** (item 1) The rate-limit attempt of a presence scan is stamped with a clock read taken
  right before `record_attempt_with_reserve`, after the policy write lock is acquired (a clock
  shift while another task holds the lock is reflected in the stamp), and the same value marks the
  scan start; a clock error or a reading older than the step-3 one skips the tick with
  `ClockUnavailable` — no attempt, no camera wake, no inference, no unlock.
- **PFU2** (item 2) `PresenceLogind::connect()` is awaited by the worker before every snapshot
  under `DBUS_CONNECT_TIMEOUT_MS`, outside `DBUS_CALL_TIMEOUT_MS`: a connect slower than the call
  bound but within the connect bound proceeds; a hung connect ends the tick within the connect
  bound; any connect failure is `LogindUnavailable` with backoff, before any snapshot call; only
  `ZbusLogind::connect` builds a bus connection; both bounds' coverage is documented in
  `presence/mod.rs` and `Docs/DAEMON.md` §6.
- **PFU3** (item 3) `/etc/pam.conf` (injectable `with_pam_conf`): present while no PAM
  directory is a directory ⇒ `Undeterminable`; present next to a PAM directory ⇒ scanned like a
  stack file (option ⇒ `Undeterminable`; non-regular/unreadable/oversized/non-UTF-8 ⇒
  `Undeterminable`; comment-only or option-free ⇒ usable); absent ⇒ unchanged behaviour; PAM
  directories are recognised through symlinks.
- **PFU4** (item 3) The PAM line scan is a superset of libpam: any policy-option substring after
  the first `pam_faillock.so` of a logical line is detected, whatever the whitespace (Unicode
  included), brackets (unterminated included) or a `\` before a `#` comment; the two round-1
  counter-examples are detected; an `include` / `substack` / `@include` whose target contains `/`
  (absolute, nested, `..`) is undeterminable while plain-name includes stay usable; the deliberate
  over-detection and the name-only module match are documented in `account.rs`; existing negative
  cases (options before the module, other modules, comments) stay undetected.
- **PFU5** (item 4) At most one account check is in flight: while a check blocks past
  `ACCOUNT_CHECK_TIMEOUT_MS`, the next tick refuses the candidate (`AccountRefused`) at once without
  entering the guard a second time; the guard is entered again only after the first check ended.
- **PFU6** (item 5) While the store holds no template (bounded, non-decrypting
  `BiometricStore::has_enrolled_template`), a tick makes no D-Bus call (no connect, no snapshot)
  and skips with `NotEnrolled` (asserted through the logind call counters **and** a recording
  `connect()` wrapper); a store error skips with `TemplateStoreError` likewise; the probe never
  decrypts, ignores symlinks, directories, temporary and non-canonical names, and is bounded
  (4096 non-matching entries ⇒ `false`, 4097 ⇒ error, missing directory ⇒ error); the probe
  runs every tick, so a new enrollment is picked up on the next tick without restart; skipping
  clears the lock tracker (the grace restarts).
- **PFU7** (item 6) Diagnosis of the flaky
  `cli_deadline_json_tests::test_simulate_pam_auth_zero_timeout_is_clamped_to_pam_minimum`
  recorded (§8); no production fix (the cumulative deadline started before `connect` is the
  #231 / #312 PAM-parity contract). **Owner-approved assertion migration (owner decision
  2026-10-02, option (a), chosen after option (b) proved ineffective, §8)**: in this one test,
  `simulate_pam_auth` may return `Ok` or `Err(AdminCliError::Timeout)`; any other error still
  fails. The deadline assertion is unchanged and always runs: the mock server captures
  `deadline_monotonic_ns` from the request whatever the client outcome, and the test fails if the
  captured value does not arrive within 5 s. The 250 ms and oversized-timeout tests still require
  success. The option (b) readiness channel was reverted (it does not help the capture).

## 8. Item 6 Diagnosis (no production change)

`simulate_pam_auth` clamps `0` to `MIN_TIMEOUT_MS = 10` and starts **one cumulative deadline
before `connect`** (`ExchangeDeadline::start` is its first statement after the clamp), exactly the
`pam_soos.so` contract of GitHub #312 (STO-NEW-4); inside the window only microsecond work runs
(`socket`, non-blocking `connect`, `getrandom`, encode, `write`, re-armed `SO_RCVTIMEO`). The
helper `capture_deadline` serves the response from a **separate thread** that must be scheduled,
`accept`, read, decode, stamp, encode and write within those 10 ms; the call then `.expect`s
success ("simulated authentication must complete"). Under CPU contention the server thread is not
scheduled in time and the client correctly reports `AdminCliError::Timeout`.

Evidence (2026-10-02, 16-core host, debug build): idle 50/50 pass; 64 concurrent instances pinned
to one CPU (`taskset -c 0`), 3 rounds: **3/192 failures**, all
`panicked at crates/admin-cli/tests/cli_deadline_json_tests.rs:78:10: simulated authentication must
complete: Timeout`; the same harness with the 250 ms test
(`test_simulate_pam_auth_deadline_uses_monotonic_clock`): 0/192. Changing production (starting the
deadline later, or a floor above 10 ms) would break the PAM-parity contract (#231, #312) — not
done. Owner options (test change, owner approval required): (a) let `capture_deadline` accept
`Err(AdminCliError::Timeout)` for the clamp test, since the asserted contract is the
`deadline_monotonic_ns` captured by the server, not completion; (b) have the server thread block
in `accept` before the client starts (a barrier) — reduces but cannot remove scheduling latency;
(c) keep as is (rare, load-only).

**Owner decision 2026-10-02: (b)**, applied as a setup-only change (readiness channel signalled
by the server thread immediately before `accept()`, awaited by the client before `before` and
`simulate_pam_auth`). Same stress harness (64 concurrent instances on CPU 0, 3 rounds = 192 runs
per measurement): before 4/192 and 5/192 failures (earlier session: 3/192); after 2/192, 8/192 and
18/192 (host load varies between measurements). All failures, before and after, are the same
`simulated authentication must complete: Timeout`. Conclusion: (b) removes the "thread not yet
started" share of the window but not the wake-up latency of the server blocked in `accept()`
under single-CPU contention; it does not measurably fix the flake. Only (a) (accept
`Err(Timeout)` in the clamp test, since the contract under test is the captured
`deadline_monotonic_ns`) would remove it; that needs a new owner decision.

**Owner decision 2026-10-02 (second): (a)**, applied as an owner-approved assertion migration
(the option (b) readiness channel is reverted: the client's connect and write go to the listen
backlog and socket buffer whatever the server thread's state, so it does not help the capture).
Same harness after (a): 2/192, 0/192, 0/192 failures; the 250 ms control: 0/192. The 2 residual
failures are `read length: UnexpectedEof` in the mock server followed by the missing capture: the
client itself was descheduled past its 10 ms deadline **before** writing the request (correct
production behaviour: no write after the deadline), so no `deadline_monotonic_ns` exists to check.
The owner rule "fail if the captured value never arrives" keeps this residual; removing it would
mean skipping the deadline assertion when no request was sent (not done: owner decision needed).

## 9. Test Hooks for Tester

- Clock: `with_clock_fn` (fn pointer) with per-test statics; the account guard (`AccountGuard`,
  called at step 7 between step 3 and step 10) is the hook that shifts / fails the clock.
- `PresenceLogind` wrapper in the test (delegates to `MockPresenceLogind`, overrides `connect`
  with a delay / hang / error and records an event order); `PresenceWorker::new` is public.
- Store: `PipelineParts::store` (`delete`, `enroll`), the store directory
  `parts.temp.path().join("biometrics")`.
- `SystemAccountGuard::with_pam_conf`, `with_pam_dirs`.
- Setup-only migration: the three existing builders of a `SystemAccountGuard` over **existing**
  PAM directories (`presence_account_tests::Accounts::guard`,
  `presence_logging_tests::test_pau_account_guard_never_logs_its_sources`,
  `presence_candid_review_tests::guard_tree`) add `.with_pam_conf(<tempdir>/pam.conf)` (absent),
  so they never read the host `/etc/pam.conf` (tester rule: never touch `/etc`). No assertion
  changes. `presence_worker_tests::test_pau_root_session_is_never_scanned_or_unlocked` needs none
  (UID 0 is refused before step 3).
- Setup-only migration (F2, mandated by item 5): in
  `presence_worker_tests::test_pau_unusable_templates_cost_no_attempt_and_no_camera`, the
  "not enrolled" case enrolls another UID (`vec![(1001, Enrollment::LiveIdentity)]` instead of
  `vec![]`): with an empty store the first tick is now `NotEnrolled` (no logind poll) instead of
  the `InGrace` asserted by `tick_past_grace`; enrolling UID 1001 (no session) keeps exercising the
  per-UID not-enrolled path the case was written for. Expected reasons and `assert_nothing_spent`
  are untouched. The empty-store path is covered by the new PFU6 tests.
  `test_pau_camera_stays_in_standby_without_an_eligible_scan` ("not enrolled", empty store) still
  passes unchanged (it only asserts `Skipped(_)` and no camera wake).
- PFU1 lock hook: a task holding `fx.parts.policy.write()` from before the tick, released after
  the guard's step-7 signal plus a delay, with a clock shift just before the release.

## 10. Documentation Drift / ADR

- Issue wording item 3 says libpam uses `pam.conf` "when `/etc/pam.d` does not exist"; the
  source shows "when none of `/etc/pam.d`, `/usr/lib/pam.d`, `VENDORDIR/pam.d` is a directory".
  The spec follows the source and is stricter in both cases.
- ADR amendment line (presence ADR, Phase 6): "Follow-ups #325: the attempt is stamped right
  before recording; the connection is opened by `connect()` under its own 1000 ms bound outside
  the 500 ms per-step call bound; no D-Bus traffic while the store holds no template;
  `/etc/pam.conf` is undeterminable without a PAM directory and scanned otherwise; the PAM line
  scan is a superset of libpam for a module named `pam_faillock.so` (a renamed copy is not seen),
  and an `include` / `substack` of a path is undeterminable."
- `Docs/DAEMON.md` §6 step list: insert the enrollment probe and the connect step, state what
  the 500 ms bound covers, add the `pam.conf` rule and the superset scan to the account-guard
  bullet. Phrases checked by `presence_followups_contract`: `/etc/pam.conf`,
  `DBUS_CONNECT_TIMEOUT_MS`, `whole snapshot`, `no template`.

## 11. Round-1 Evaluation Resolution

| Finding | Resolution |
|---|---|
| F1 MAJOR under-detecting scan | §2.3 substring superset rule; PFU4 tests with both counter-examples |
| F2 MAJOR PAU8 breaks | §9 setup-only migration (enroll UID 1001), justified by item 5 |
| F3 MINOR `Closes #325` vs pending item 6 | closed: owner decision 2026-10-02 (option b) taken, recorded in PFU7 / §8 |
| F4 MINOR probe edge tests, connect counter | PFU6 wording; tests named in the tester contract |
| F5 MINOR "under the lock" untestable | PFU1 wording + lock-holder test hook (§9) |
| F6 MINOR connect-only build untestable | textual invariant `presence_followups_contract::test_pfu_only_connect_opens_a_bus_connection` |
| O1 sync probe on the async worker | §2.5 rationale |
| O2 tests drafted before the gate | drafts are revised to the approved revision before hand-off; none is relied on before approval |
| R2-F1 MAJOR include/substack of a path not followed | §2.3 option (a): such a line is a policy hit; PFU4 tests for `/abs`, `sub/file`, `../x`, `@include`, plain-name negatives |
| R2-F2 MINOR name-only module match | §2.3 doc sentence + ADR amendment wording |
