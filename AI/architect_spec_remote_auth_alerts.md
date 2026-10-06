# Architect Spec — GitHub #339 follow-up: Failed-Password Alerts in `soos-remote` (feature level 1)

- **Date**: 2026-10-06
- **Branch**: `feat/remote-auth-alerts` (from `feat/remote-companion` at `ea862cc`); nothing is committed, pushed,
  stashed, deployed or restarted by agents (O-5).
- **Inputs**: owner decisions O-1 to O-5 (2026-10-06), `AI/research_alerts.md` (read-only host research, systemd 262,
  Linux-PAM 1.7.3), ADRs 2026-10-05 and 2026-10-06 (x3) on `soos-remote`, `Docs/REMOTE_COMPANION.md`, walkthroughs
  183–185, `crates/remote/src/*.rs`, `crates/remote/tests/common/harness.rs`, `packaging/soos-remote.service`,
  `scripts/install_remote.sh`, `tests/invariants/src/remote_{companion,passkey}_contract.rs`.
- **ADR**: "[2026-10-06] Failed-Password Alerts in `soos-remote` From the System Journal" (drafted in
  `AI/DECISIONS.md`; the text there is authoritative, §14 summarises it).
- **Matrix**: new rows RMC45–RMC59 (§13), placed after RMC44.
- **Scope**: feature level 1 only (in-page alerts: JSON route, SSE event, banner, history, acknowledge). Any later
  level (for example push notifications to a closed page) needs its own spec and ADR.
- **Revision**: round 2, answering every finding of `AI/plan_evaluator_report.md` round 1 (F-1 to F-11 and the
  observation); §18 maps each finding to the sections changed.

## 0. Owner decisions, the relayed request, and the spec-level decisions

### 0.1 The relayed request versus O-2 (must be confirmed by the owner)

The relayed user request reads "see on the phone the passwords that were tested". **This spec does not show the
typed text and cannot**, for three independent reasons:

1. No journal producer contains it (`AI/research_alerts.md` §4: `unix_chkpwd` reads it from a pipe and overwrites
   it, `pam_unix` never logs it).
2. Obtaining it would need a new PAM component capturing `PAM_AUTHTOK`, which `AGENTS.md` forbids outright ("NEVER
   log … credentials", "NEVER accept, store, or transmit passwords over the IPC socket") and which O-2 forbids for
   this feature (no text, part, length or hash).
3. The owner's own typos are near-copies of the real password; shipping them to a page reachable over Tailscale
   Funnel would turn a monitoring feature into a password-leak channel.

What the phone shows instead is the safe subset: **when**, **where** (lock screen / sudo / login / other),
**which account class** (yours / root / another account), **what happened** (wrong password, or an attempt while
`pam_faillock` had locked the account) and **how many times**. The orchestrator must surface this point to the
owner; if the owner insists on the typed text, that is a different feature that needs an amendment of `AGENTS.md`
and a new ADR, not this spec. **Status: open — the orchestrator must obtain the owner's explicit confirmation of
the safe subset before Phase 2 (tests) starts.** Nothing in this spec is a partial step towards showing the typed
text: no field, route or buffer is designed to carry it later.

### 0.2 Owner decisions restated (binding)

| Id | Decision |
|---|---|
| O-1 | Tell the owner on the phone, centralised in the soos-remote web app, when someone types a wrong password on the PC (lock screen, sudo, GDM login, any PAM password failure for local accounts). |
| O-2 | Never capture, store, log, display or transmit the attempted password or any part, length or hash of it. Alerts carry only time, source class, target account, count. No raw log line. |
| O-3 | Detection source is the system journal, read by the owner without root. Every field is untrusted input; prefer journald-set trusted fields; document the residual spoofing risk (false alerts only). |
| O-4 | Everything bounded (history, alert rate, memory), fail-closed, visible only to authenticated callers (tailnet identity or Funnel passkey session), never to anonymous Funnel requests. |
| O-5 | Agents never commit, push, stash, open PRs, run `tailscale`, restart/reinstall host services or touch `~/.config`, `/etc` or the journal configuration. English only; no `unwrap`/`expect` in production; the crate-level forbid lint stays. Tester tests are immutable contracts. |

### 0.3 Spec-level decisions (each restated in the ADR)

| Id | Decision | Rationale |
|---|---|---|
| A-1 | **Mechanism**: a child `journalctl` process (absolute path `/usr/bin/journalctl`, environment cleared, no shell, stdin and stderr `/dev/null`, `kill_on_drop`), started through `tokio::process` (new tokio feature `process` in `crates/remote/Cargo.toml` only), JSON output restricted with `--output-fields`, match `SYSLOG_FACILITY=10` only. | No C binding (O-3, research §5: `sd-journal` FFI, a pure-Rust journal-file reader, Varlink and `systemd-journal-gatewayd` all excluded). `journalctl` needs no new socket family, no JIT, no setuid; `RestrictAddressFamilies=AF_UNIX` and the whole unit stay **unchanged**. `tokio::process` gives `kill_on_drop` and async bounded reads on the current-thread runtime; it reuses `signal-hook-registry` already in `Cargo.lock` (confirmed by `cargo deny check` in Phase 4). |
| A-2 | **Opt-in**: `password_alerts = true` in `remote.toml` (default `false`). While `false` nothing is spawned, `GET /api/alerts` answers `{"state":"disabled",…}`, the ack route `403 alerts_disabled`, and the event stream never sends an `alerts` event (every existing test stays valid). | Same upgrade rule as `allow_unlock` and `allow_funnel`: a new binary never starts reading the journal by itself. |
| A-3 | **Fail-closed visibility**: the feature state is always part of the view (`starting`, `active`, `unavailable` + reason, `disabled`). The page never shows "0 attempts" unless the state is `active`, and `active` means **caught up** (the 24 h backlog has been read, §5 step 3), not merely "child spawned". A startup probe requires that the reader sees root-side entries (`_UID=0`); without `wheel`/`adm`/`systemd-journal` membership the state is `unavailable` / `no_journal_access`. **Lock-screen coverage is part of the view too** (`lock_screen: "monitored" \| "not_configured"`, §4.4): `monitored` iff at least one configured `lock_screen_programs` path exists as a regular file at follower start; otherwise the page says "lock screen not monitored — see setup" instead of a plain "No failed password attempts". | Research §1: without the group `journalctl` exits 0 but shows only the user journal; a silent partial view would claim "no attempts" falsely. Plan-evaluator F-2: the owner's own locker (`~/.local/bin/swaylock-plugin`) is not a default, so a coverage gap must be visible, never silent. F-5: a page loaded during the replay must not read "0 attempts". |
| A-4 | **Counting unit** = one real password check (`unix_chkpwd: password check failed for user (…)`), merged with the `pam_unix(<svc>:auth): authentication failure` line of the same attempt (pairing window 2 s) so an attempt is never counted twice; `pam_unix` lines alone also count (a client that reads the shadow file itself logs no helper line). | Research §2.2: `unix_chkpwd` is complete (11/11), `pam_unix` undercounts (first failure per PAM handle only), the `soos-daemon` telemetry line has no service name, misses restarts and is quota-limited: it is **not** used. |
| A-5 | **Trust allow-list** on journald-set fields (`_TRANSPORT`, `_UID`, `_EXE`, `_COMM`) and the facility; `SYSLOG_IDENTIFIER` and `MESSAGE` are only grammar inputs. Root-side signals (`_UID=0`, or a setuid authenticator whose `_EXE` is `/usr/bin/sudo` or `/usr/bin/su`) are trusted; owner-UID lock-screen signals are trusted only when `_EXE` is in `lock_screen_programs`; any other UID is ignored. Every `_EXE` comparison first strips **exactly one** trailing ` (deleted)` (the kernel's suffix for a binary replaced while running, e.g. a locker or `sudo` upgraded under a running process); nothing else is normalised. | O-3; research §3. A UID-1000 process can never produce `_UID=0`; `_EXE` of the setuid binaries cannot be forged without running them (which is a real attempt). Stripping the suffix does not widen trust: the remaining path must still be byte-equal to an allow-listed path, and a process whose binary was at that path was that program. |
| A-6 | **Source class** of a helper-only check comes from the most recent trusted `pam_unix` failure ("anchor") of the same helper side and account within 1 h; a root-side check without an anchor is counted as `other`; an **owner-side check without a trusted anchor is dropped** (it is the signature of developer test binaries, research §2.3). A check that pairs with an *untrusted* `pam_unix` failure is dropped too (the test binary's own helper line). | Removes the `drift_verrou` / `drift-verrou` noise while still counting the lock screen's 2nd, 3rd, … attempts that `pam_unix` does not log. |
| A-7 | **Attempts while locked out** (`pam_faillock(<svc>:auth): User … is temporarily locked out due to N consecutive failed login attempts`, trusted by the same allow-list) are counted as a separate kind `locked_out`; `Consecutive login failures … account temporarily locked` (a consequence, not an attempt), `User unknown`, `conversation failed`, `auth could not identify password` and every other line are ignored. | Research §2.2 and open point 3: an attempt during a lockout is an attempt even though no password was checked; it must not inflate the wrong-password count. |
| A-8 | **Account privacy (O-2)**: the raw account field is mapped at parse time to `owner` (equal to the service owner's login name, resolved once at startup from the process uid), `root`, or `other`, and dropped. The JSON, the page, the logs and the memory past parsing never hold any other name; `other` is shown as "another account". The owner's login name itself is not sent either (the page says "your account"). | Research §4: at GDM or tty login a password typed into the user-name box lands in `user=`; mapping to three classes makes it impossible to leak. It also keeps the existing rule "the body never contains the uid or user name" (`Docs/REMOTE_COMPANION.md` §6). |
| A-9 | **History**: rebuilt from the journal at every start (`--since` = now − 24 h) instead of persisting alert records; only the acknowledgement marker (`acknowledged_until_us`) is persisted, in `remote-alerts.json` (`0600`, ≤ 256 bytes) next to the credential store. The marker is a **derived** value, always strictly below the time of every attempt the service knows to be unacknowledged (§4.3 marker rule), and it is applied only to attempts **replayed at start-up** (journal time before the book's start time), never to an attempt resolved live. In memory: at most 32 records, consecutive attempts of the same class/account/kind within 60 s coalesce into one record with a saturating count. | No alert data at rest (the journal is already the record); acknowledgements survive restarts; memory is bounded. F-3 (b): a pending lock-screen check older than an acknowledged record must still be shown, now and after a restart. |
| A-10 | **Exposure**: `GET /api/alerts` (status-like read) and `POST /api/alerts/ack` (state change, lock-style CSRF with `X-Soos-Action: alerts-ack`, 1 s rate limit, no body). The acknowledged snapshot is named by **two request headers**, exactly one each: `X-Soos-Alerts-Epoch: <16 lowercase hex>` and `X-Soos-Alerts-Through: <1..=20 ASCII digits>`; the query string is never read (the HTTP parser drops it, `RequestHead` and every existing test stay unchanged). Both routes require a tailnet identity or a Funnel web session (never in `is_funnel_public`, so anonymous Funnel is `403 login_required`). The existing `/api/events` stream additionally carries `event: alerts` (at most one per second per stream); on a Funnel session stream the session is re-validated (`Touch::Keep`) **before every `alerts` event**, and the stream ends when it is gone. | O-4. F-1: `RequestHead.path` has no query (`http.rs:35`, `:150`); headers need no parser change and add a second custom header a cross-site form cannot send. F-4: alert data never reaches a revoked session. |
| A-13 | **Per-start epoch**: every start of the service draws a 64-bit random `epoch` (from the `ServerState` `RandomSource`, i.e. `getrandom`, the crate's only CSPRNG) when the follower is set up; the view carries it as 16 lowercase hex characters; an acknowledgement whose epoch differs answers `409 stale_view` and acknowledges nothing. The epoch is not a secret (only authenticated callers see it), is never logged and never persisted. An RNG failure leaves alerts `unavailable` / `rng_failed` (nothing spawned, ack `503 unavailable`). | F-3 (a): attempt sequence numbers restart at 1 on every start, so a stale page could otherwise send a `through` that covers newer attempts it never displayed. |
| A-11 | **Logging**: `journal.rs` and `alerts.rs` contain no `tracing` macro at all; state transitions emit three fixed-text audit events through `audit.rs` (`password alerts active` INFO, `password alerts unavailable` WARN, `password alert acknowledgement not persisted` WARN), never with a field. | O-2 and the branch lesson: macro callsites can lose their interest cache in parallel tests; fixed-text events are deterministic. |
| A-12 | Two `RemoteConfig { … }` struct literals in tests (`crates/remote/tests/common/harness.rs:771`, `crates/remote/tests/server_tests.rs:762`) gain the line `alerts: AlertsConfig::default(),` — **setup only, no assertion changes**; recorded in the ADR as the test-integrity rule requires. `RequestHead` (`http.rs:31-39`) is **not** changed, so its three test literals (`routes_tests.rs:25`, `:487`, `http_tests.rs:95`) and every parsed-head assertion stay valid. No other existing test changes. | `RemoteConfig` has no `Default`; adding a field is a compile-only change for literals. F-1: the header transport avoids any `RequestHead` migration. |
| A-14 | **No typed text in memory longer than needed** (O-2): the bounded line reader owns its buffer as `Zeroizing<Vec<u8>>` and reads from the child's pipe without an intermediate `BufReader`; each line is handed out as `Zeroizing<Vec<u8>>`; `JournalEntry.message` (and every other owned string of an entry) is `Zeroizing<String>`; the JSON visitor wraps every owned (unescaped) string in `Zeroizing` before it is checked. Residual: the kernel pipe buffer and `serde_json`'s internal scratch buffer for escaped strings are not under the crate's control (documented, §15). | F-10. `zeroize` is already a dependency (`crates/remote/Cargo.toml:30`). |

## 1. Scope & blast radius

### 1.1 Crates, modules, binaries

| Path | Change |
|---|---|
| `crates/remote/Cargo.toml` | `tokio = { workspace = true, features = ["process"] }` (dependency line only; the workspace pin is unchanged; dev-dependency line unchanged). |
| `crates/remote/src/lib.rs` | `pub mod journal; pub mod alerts;` and every constant of §3.1 (single source). |
| `crates/remote/src/journal.rs` (new) | Bounded JSON-line parsing (`parse_entry`), trust classification (`classify_entry`), the `JournalSource` / `JournalLines` seam, the production `JournalctlSource`, the pure argv builders, the bounded line reader. |
| `crates/remote/src/alerts.rs` (new) | `Correlator`, `AlertBook`, `AlertsView`, ack-file read/write, the follower task (`run_follower`). |
| `crates/remote/src/config.rs` | Keys `password_alerts`, `lock_screen_programs`; `AlertsConfig`; errors `TooManyLockScreenPrograms`, `InvalidLockScreenProgram`; `resolve_alerts_ack_path`. |
| `crates/remote/src/routes.rs` | `Route::Alerts`, `Route::AlertsAck`, paths, `check_alerts_ack_csrf`, `parse_alerts_ack_headers` (§6.1). `accepts_body` unchanged (the ack has no body). `is_funnel_public` unchanged (the new routes are not public). |
| `crates/remote/src/http.rs` | `encode_sse_alerts_event(json)`; `encode_sse_event`, `RequestHead` and `parse_request_head` **unchanged** (A-12). |
| `crates/remote/src/server.rs` | `ServerState::with_password_alerts`; `Shared` gains the alert book, its `watch` version, the ack gate, the marker flush state; `serve` draws the epoch (A-13) and supervises the follower; the two routes; `serve_stream` forwards alert versions and re-validates a Funnel session before each `alerts` event (A-10). |
| `crates/remote/src/audit.rs` | Three new fixed-text events (A-11). |
| `crates/remote/src/main.rs` | Resolves the owner login (`nix::unistd::User::from_uid`) and the ack path, wires `JournalctlSource` when `password_alerts` is set. Reads no new environment variable. |
| `crates/remote/assets/{index.html,app.js,style.css}` | Banner, history list, acknowledge button (§9). |
| `packaging/soos-remote.service` | **Unchanged** (A-1). Description line may stay as is. |
| `scripts/install_remote.sh` | Unchanged, except an informational note printed when the user is not in `wheel`, `adm` or `systemd-journal` (optional; `id -nG` only, never `usermod`). |
| `Docs/REMOTE_COMPANION.md` | New §2c "Failed-password alerts" (with the **mandatory** `lock_screen_programs` step for a locker outside `/usr/bin`, F-2); §5 rows; §6 rows; §8 residual limitations. |
| `AI/DECISIONS.md` | New ADR (§14). |
| `tests/invariants/src/remote_alerts_contract.rs` (new) + `mod` line in `tests/invariants/src/lib.rs` | Static contracts RMC-S24–RMC-S30 (§12.8). |

### 1.2 Consumers

`soos-remote` is a leaf crate (RMC-S4): no other crate consumes these items. `soos-daemon`, `pam_soos.so`, the IPC
protocol, `soos-admin`, the GUI and packaging other than the user unit are untouched. Test consumers: the two
`RemoteConfig` literals (A-12).

### 1.3 Out of scope

Push notifications or any delivery to a closed page; the `soos-daemon` telemetry line; a change of `pam_soos.so` or
the daemon to add a service name; alerts for non-PAM authentication (SSH keys, Tailscale, passkeys); remote
SSH failures are counted as `other` only if they arrive through the same `pam_unix` grammar and trust rules
(`sshd` runs as root); a pure-Rust journal reader; any change to `journald.conf` or group membership.

## 2. Journal reading (`journal.rs`)

### 2.1 Exact `journalctl` invocations (pure argv builders)

```rust
/// Absolute path; never resolved through `PATH` (the environment is cleared).
pub const JOURNALCTL_PATH: &str = "/usr/bin/journalctl";

/// Where a follower starts.
#[derive(Clone, PartialEq, Eq)]
pub enum FollowStart {
    /// `--since=@<unix_s>` (first start, or after a refused cursor).
    Since { unix_s: u64 },
    /// `--after-cursor=<cursor>` (restart without a gap).
    AfterCursor(JournalCursor),
}

/// Pure. The probe argv (without the program):
/// `--no-pager --quiet --lines=1 --output=json --output-fields=_UID _UID=0`
#[must_use] pub fn probe_args() -> Vec<String>;

/// Pure. The follower argv (without the program), in this exact order:
/// `--follow --no-pager --quiet --lines=all --output=json
///  --output-fields=MESSAGE,SYSLOG_IDENTIFIER,SYSLOG_FACILITY,_UID,_COMM,_EXE,_TRANSPORT`
/// then `--since=@<unix_s>` or `--after-cursor=<cursor>`, then the match `SYSLOG_FACILITY=10`.
#[must_use] pub fn follow_args(start: &FollowStart) -> Vec<String>;
```

Never used: a shell, `--user`, `-g`/`--grep` (PCRE2), `--all`, `--merge`, `--directory`, `--file`, `--root`,
`--cursor-file` (no file written by the child). `__CURSOR` and `__REALTIME_TIMESTAMP` are always printed by
`journalctl` (research §5).

### 2.2 `JournalCursor`

```rust
/// A `__CURSOR` value: 1..=MAX_CURSOR_LEN bytes of `[A-Za-z0-9=;_-]` (observed: 125 bytes of
/// `[a-z0-9=;]`). Anything else is not kept (the next restart falls back to `Since`).
#[derive(Clone, PartialEq, Eq)]
pub struct JournalCursor(String);
impl JournalCursor { #[must_use] pub fn parse(raw: &str) -> Option<Self>; #[must_use] pub fn as_str(&self) -> &str; }
// Debug: "JournalCursor(<redacted>)" (a cursor is an opaque position, not a secret, but never logged).
```

### 2.3 Entry parsing (pure)

```rust
/// One accepted journal entry. No `Debug` (MESSAGE may hold typed text in the account field).
/// Every owned string is `Zeroizing` (A-14): wiped when the entry is dropped.
pub struct JournalEntry {
    pub realtime_us: u64,                       // __REALTIME_TIMESTAMP, decimal, 1..=20 digits, fits u64
    pub cursor: Option<JournalCursor>,
    pub message: Zeroizing<String>,             // MESSAGE, 1..=MAX_MESSAGE_BYTES, no byte < 0x20 and no 0x7F
    pub identifier: Option<Zeroizing<String>>,  // SYSLOG_IDENTIFIER (grammar input only, never trusted)
    pub facility: u8,                           // SYSLOG_FACILITY; parse_entry only accepts "10"
    pub uid: u32,                               // _UID, decimal
    pub comm: Option<Zeroizing<String>>,        // _COMM
    pub exe: Option<Zeroizing<String>>,         // _EXE (raw, ` (deleted)` suffix kept; stripped at comparison, §2.4)
    pub transport: Zeroizing<String>,           // _TRANSPORT
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EntryError {
    #[error("journal line too long")]      TooLong,        // > MAX_JOURNAL_LINE_BYTES (checked before JSON)
    #[error("journal line is not a JSON object")] NotJson,
    #[error("journal field missing")]       MissingField,  // __REALTIME_TIMESTAMP, MESSAGE, _UID, _TRANSPORT, SYSLOG_FACILITY
    #[error("journal field has an unexpected shape")] Shape,
    #[error("journal entry is not on the authpriv facility")] Facility,
}

/// Pure; never panics on any input (proptest). Field value rules:
/// - a JSON string → the text;
/// - a JSON array whose every element is an integer 0..=255 → those bytes, which must be valid UTF-8;
/// - anything else (null = field over 4096 bytes, an array of strings = duplicated field, nested values,
///   numbers, booleans, invalid UTF-8) → `Shape` when the field is one of the nine read fields, ignored otherwise.
/// Unknown keys are ignored (their values skipped with `serde::de::IgnoredAny`, never materialised).
/// A read key present twice in the object → `Shape` (hand-written visitor, see below).
pub fn parse_entry(line: &[u8]) -> Result<JournalEntry, EntryError>;
```

**Duplicate keys (F-11).** `serde_json::Map` and `Value` keep the last duplicate silently, so `parse_entry` does
**not** go through `Value`/`Map`. It runs `serde_json::Deserializer::from_slice(line)` with a hand-written
`serde::de::Visitor` whose `visit_map` iterates `next_key::<Cow<str>>()`; the nine read keys
(`__REALTIME_TIMESTAMP`, `__CURSOR`, `MESSAGE`, `SYSLOG_IDENTIFIER`, `SYSLOG_FACILITY`, `_UID`, `_COMM`, `_EXE`,
`_TRANSPORT`) map to a fixed index in a `[bool; 9]` "seen" set; a key already seen → `Shape`; any other key →
`next_value::<IgnoredAny>()`. Each read value is deserialised into a private `FieldValue` enum (string, or
sequence of `u8`, with a per-field length bound enforced **while** visiting: a string or a sequence longer than
the field bound stops the visit with `Shape`, so no unbounded allocation). After the map, `Deserializer::end()`
must succeed (trailing bytes → `NotJson`). Duplicates of unknown keys are not detected (their values are never
used). Owned strings are wrapped in `Zeroizing` as soon as they exist (A-14).

Bounds: `MAX_JOURNAL_LINE_BYTES` = 24 576 (a 4096-byte field rendered as a byte array is ≤ 16 384 bytes; research
max line 4001 bytes); `MAX_MESSAGE_BYTES` = 4096; `MAX_FIELD_BYTES` = 4096 for every other read field. A message
containing a control character (ANSI colour codes included) is `Shape`: none of the three grammars below contains
one, so such a message can never be a signal.

**Blind spot of the bounds (F-9, documented in §15 and the ADR).** An attempt whose journal line exceeds a bound
(an over-4096-byte or non-UTF-8 user-name field rendered by `journalctl` as `null` or as an oversized byte array, or
a line over `MAX_JOURNAL_LINE_BYTES`) is skipped and therefore not alerted. Such a field can only name an account
other than the owner's (the owner's login is at most 32 bytes, §2.4), so no attempt against the owner's account can
be hidden this way. The matching `unix_chkpwd` line (if any) carries the same over-long name and is refused by
its own 256-byte name bound, so the attempt is not counted from that side either; this is accepted.

### 2.4 Signals and trust (pure)

```rust
/// Where the attempt was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceClass { LockScreen, Sudo, Login, Other }

/// Account class (A-8); the raw name never leaves `classify_entry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountClass { Owner, Root, Other }

/// UID the password helper runs as for this attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HelperSide { Root, Owner }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// `unix_chkpwd`: one real password check.
    Check { side: HelperSide, account: AccountClass, at_us: u64 },
    /// `pam_unix(<svc>:auth): authentication failure; …`.
    Failure { class: SourceClass, side: HelperSide, account: AccountClass, at_us: u64, trusted: bool },
    /// `pam_faillock(<svc>:auth): User <n> is temporarily locked out due to <d> consecutive failed login attempts`.
    LockedOut { class: SourceClass, account: AccountClass, at_us: u64 },
}

/// What `classify_entry` needs besides the entry.
#[derive(Clone)]
pub struct TrustContext {
    pub owner_uid: u32,
    pub owner_login: OwnerLogin,
    pub lock_screen_programs: Vec<String>, // absolute paths from the configuration
}

/// Pure; `None` for every entry that is not a signal.
pub fn classify_entry(entry: &JournalEntry, ctx: &TrustContext) -> Option<Signal>;
```

Common gate (all three): `entry.transport == "syslog"`, `entry.facility == 10`, `entry.uid ∈ {0, owner_uid}`
(any other UID → `None`).

**Grammars** (exact, byte-wise, on `entry.message`):

| Signal | Message grammar | Account field | Extra trusted-field gate |
|---|---|---|---|
| `Check` | `password check failed for user (` NAME `)`, NAME = everything between the first `(` after the fixed prefix and the final byte `)`, 1..=256 bytes | NAME | `identifier == Some("unix_chkpwd")`; `comm` absent or `unix_chkpwd`; `exe` absent or `exe_for_comparison(exe)` in `UNIX_CHKPWD_PATHS` = {`/usr/bin/unix_chkpwd`, `/usr/sbin/unix_chkpwd`}; side = `Root` if `uid == 0` else `Owner` |
| `Failure` | message **starts with** `pam_unix(` SVC `:auth): authentication failure;` (prefix-anchored: a `sudo` command-log line whose `COMMAND=` text contains this grammar never matches) … ; NAME = the text after the ` user=` that follows `rhost=`, to the end; no ` user=`, or more than one ` user=` after `rhost=` (ambiguous typed text, candid review 2026-10-06) → account `Other` | NAME | see trust table |
| `LockedOut` | `pam_faillock(` SVC `:auth): User ` NAME ` is temporarily locked out due to ` DIGITS ` consecutive failed login attempts` (DIGITS 1..=5 ASCII digits) | NAME | trusted only (an untrusted `LockedOut` → `None`) |

SVC: 1..=64 bytes of `[A-Za-z0-9._-]`; anything else → `None`.

**Service → class** (constants in `journal.rs`, the only copy):

| Class | Services |
|---|---|
| `LockScreen` | `LOCK_SCREEN_SERVICES` = `swaylock`, `hyprlock`, `gtklock`, `waylock`, `i3lock`, `xscreensaver`, `kde` |
| `Sudo` | `SUDO_SERVICES` = `sudo`, `sudo-i` |
| `Login` | `LOGIN_SERVICES` = `gdm-password`, `login`, `sddm`, `lightdm`, `greetd` |
| `Other` | every other syntactically valid SVC (`polkit-1`, `su`, `su-l`, `sshd`, …) |

**Trust of `Failure` / `LockedOut`** (`uid` is `_UID`, `exe` is `_EXE`):

| Condition | trusted | side |
|---|---|---|
| `uid == 0` | yes | `Root` |
| `uid == owner_uid` and `exe ∈ SETUID_AUTH_PROGRAMS` = {`/usr/bin/sudo`, `/usr/bin/su`} | yes | `Root` |
| `uid == owner_uid`, class `LockScreen`, `exe ∈ lock_screen_programs` | yes | `Owner` |
| `uid == owner_uid`, anything else (no `_EXE`, test binaries, other programs) | no | `Owner` |

**`_EXE` normalisation (F-2 b).** Every comparison of `exe` above (against `UNIX_CHKPWD_PATHS`,
`SETUID_AUTH_PROGRAMS` and `lock_screen_programs`) uses `exe_for_comparison(exe)`:

```rust
/// The kernel's suffix for an executable unlinked or replaced while running.
pub const EXE_DELETED_SUFFIX: &str = " (deleted)";

/// Pure. Strips exactly one trailing `EXE_DELETED_SUFFIX`; everything else byte-for-byte unchanged
/// (`"/a (deleted) (deleted)"` → `"/a (deleted)"`, never matched by a configured path because configured
/// paths are validated without that suffix, §3.2; `" (deleted)"` alone → `""`, never matched).
#[must_use] pub fn exe_for_comparison(exe: &str) -> &str;
```

**Account mapping** (A-8): NAME byte-equal to `owner_login` → `Owner`; NAME == `root` → `Root`; else `Other`. NAME
is a borrowed slice of `entry.message` and is never copied, stored, compared after mapping, formatted or logged.

```rust
/// The owner's local login name (from `getpwuid(getuid())` at startup): 1..=32 bytes,
/// `[a-z_][a-z0-9_-]*` with an optional final `$`. Debug is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct OwnerLogin(String);
impl OwnerLogin { #[must_use] pub fn parse(raw: &str) -> Option<Self>; }
```

### 2.5 Source seam (object-safe, injectable)

```rust
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JournalError {
    #[error("journal reader not found")]       NotFound,      // spawn ENOENT
    #[error("journal reader failed to start")] Spawn,         // any other spawn error
    #[error("system journal not readable")]    NoAccess,      // probe saw no `_UID=0` entry
    #[error("journal probe timed out")]        ProbeTimeout,  // JOURNAL_PROBE_TIMEOUT_MS
}

pub enum LineRead {
    /// One complete line without its `\n`, at most `MAX_JOURNAL_LINE_BYTES`; wiped on drop (A-14).
    Line(Zeroizing<Vec<u8>>),
    /// A line longer than the bound: discarded up to and including its `\n`.
    Overlong,
    /// EOF or a read error: the follower ended.
    End,
}

pub trait JournalLines: Send {
    fn next_line(&mut self) -> BoxFuture<'_, LineRead>;
    /// Test seam only (F-8): the PID of the child process behind these lines, if any.
    /// Default `None` (scripted sources); `JournalctlSource` returns the spawned child's PID.
    #[doc(hidden)]
    fn child_pid(&self) -> Option<u32> { None }
}

pub trait JournalSource: Send + Sync + 'static {
    /// Bounded probe (A-3): `Ok(())` iff one line with `"_UID":"0"` was read within
    /// JOURNAL_PROBE_TIMEOUT_MS.
    fn probe(&self) -> BoxFuture<'_, Result<(), JournalError>>;
    /// Starts a follower.
    fn follow(&self, start: FollowStart) -> BoxFuture<'_, Result<Box<dyn JournalLines>, JournalError>>;
}

/// Production source: `tokio::process::Command::new(JOURNALCTL_PATH)` with `env_clear()`,
/// `current_dir("/")`, `stdin(Stdio::null())`, `stderr(Stdio::null())`, `stdout(Stdio::piped())`,
/// `kill_on_drop(true)`, the argv of §2.1. The child is owned by the returned lines object;
/// dropping it kills and reaps the child.
#[derive(Debug, Default)]
pub struct JournalctlSource;
```

The bounded line reader (public only as a test seam) owns its buffer so that it can be wiped (A-14, F-10); it does
**not** wrap the pipe in a `tokio::io::BufReader` (whose internal buffer could not be wiped):

```rust
// JOURNAL_READ_CHUNK_BYTES (4096) is defined in lib.rs (§3.1).
pub struct BoundedLineReader<R: AsyncRead + Unpin> { /* inner: R, buf: Zeroizing<Vec<u8>>, discarding: bool */ }
impl<R: AsyncRead + Unpin> BoundedLineReader<R> {
    #[must_use] pub fn new(inner: R) -> Self;
    /// Next line. The internal buffer never exceeds MAX_JOURNAL_LINE_BYTES + JOURNAL_READ_CHUNK_BYTES bytes;
    /// bytes handed out or discarded are wiped (`zeroize` on the drained range) before the buffer is reused;
    /// `Overlong` discards the rest of the line up to and including its `\n` and the next line is read intact;
    /// a partial last line at EOF is wiped and discarded (`End`); a read error is `End`.
    pub async fn next_line(&mut self) -> LineRead;
}
```

The capacity is reserved once at construction (`MAX_JOURNAL_LINE_BYTES + JOURNAL_READ_CHUNK_BYTES`), so the buffer
is never reallocated (a reallocation would leave an unwiped copy behind); the buffer is wiped on drop.

## 3. Constants and configuration

### 3.1 Constants (`crates/remote/src/lib.rs`, the only definition of each)

| Constant | Value | Meaning / at the bound |
|---|---|---|
| `MAX_JOURNAL_LINE_BYTES: usize` | 24_576 | longer line → `LineRead::Overlong`, skipped, nothing parsed |
| `MAX_MESSAGE_BYTES: usize` | 4096 | longer `MESSAGE` → `EntryError::Shape` |
| `MAX_FIELD_BYTES: usize` | 4096 | any other read field longer → `Shape` |
| `MAX_CURSOR_LEN: usize` | 256 | longer cursor → not kept |
| `JOURNAL_PROBE_TIMEOUT_MS: u64` | 2000 | probe exceeded → `ProbeTimeout`, child killed |
| `JOURNAL_RESTART_MIN_MS: u64` | 1000 | first backoff after a failure |
| `JOURNAL_RESTART_MAX_MS: u64` | 60_000 | backoff doubles up to this cap |
| `JOURNAL_STABLE_RUN_MS: u64` | 60_000 | a follower that ran this long resets the backoff to the minimum |
| `JOURNAL_LINES_PER_BATCH: usize` | 256 | after this many lines the follower sleeps `JOURNAL_BATCH_PAUSE_MS` (back-pressure on the pipe, ≤ ~5 000 lines/s) |
| `JOURNAL_BATCH_PAUSE_MS: u64` | 50 | see above |
| `HISTORY_REBUILD_WINDOW_S: u64` | 86_400 | first start reads `--since=@(now_s − 86 400)` (saturating at 0) |
| `PAIR_WINDOW_US: u64` | 2_000_000 | a `Check` and a `Failure` of the same side and account within 2 s are one attempt |
| `ANCHOR_WINDOW_US: u64` | 3_600_000_000 | a `Check` inherits the class of an anchor at most 1 h old (refreshed by each attributed attempt) |
| `MAX_PENDING_CHECKS: usize` | 16 | a 17th pending check resolves the oldest at once (by the anchor rule) |
| `MAX_RECENT_FAILURES: usize` | 16 | oldest evicted (it can then no longer absorb a check: worst case one extra attempt, never one fewer) |
| `MAX_ALERT_HISTORY: usize` | 32 | oldest record evicted; its unacknowledged count moves to `evicted_unacknowledged` |
| `ALERT_COALESCE_WINDOW_US: u64` | 60_000_000 | same class, account, kind and acknowledgement state within 60 s → same record |
| `ALERT_EVENT_MIN_INTERVAL_MS: u64` | 1000 | at most one `alerts` SSE event per stream per second (the latest view is sent at the end of the interval) |
| `MIN_ALERT_ACK_INTERVAL_MS: u64` | 1000 | sooner → `429 rate_limited` |
| `MAX_ALERTS_ACK_FILE_BYTES: usize` | 256 | larger file → treated as absent (fail-safe, A-9) |
| `ALERTS_ACK_FILE_NAME: &str` | `"remote-alerts.json"` | sibling of the credential store |
| `MAX_LOCK_SCREEN_PROGRAMS: usize` | 4 | more → `ConfigError::TooManyLockScreenPrograms` |
| `ACTION_ALERTS_ACK: &str` | `"alerts-ack"` | `X-Soos-Action` of the ack route |
| `ALERTS_EPOCH_HEADER: &str` | `"x-soos-alerts-epoch"` | lowercase header name (the parser lowercases names); exactly one, else `400 bad_request` |
| `ALERTS_THROUGH_HEADER: &str` | `"x-soos-alerts-through"` | lowercase header name; exactly one, else `400 bad_request` |
| `ALERTS_EPOCH_HEX_LEN: usize` | 16 | epoch rendering: exactly 16 lowercase hex characters; any other length or character → `400 bad_request` |
| `MAX_ALERTS_THROUGH_DIGITS: usize` | 20 | `through` value: 1..=20 ASCII digits, value ≤ `u64::MAX`; else `400 bad_request` (no sign, no whitespace, no leading `+`) |
| `JOURNAL_READ_CHUNK_BYTES: usize` | 4096 | one `read` of the bounded reader; buffer capacity `MAX_JOURNAL_LINE_BYTES + JOURNAL_READ_CHUNK_BYTES`, reserved once |
| `JOURNAL_IDLE_TICK_MS: u64` | 2000 | idle tick of the follower: expiry of pending checks with the wall clock (§4.2 rule 6) and the catch-up rule (§5 step 3) |
| `ALERTS_MARKER_FLUSH_MIN_INTERVAL_MS: u64` | 1000 | at most one ack-file write per second caused by a marker **lowering** (§4.3); an acknowledgement writes at once (it is already rate-limited by `MIN_ALERT_ACK_INTERVAL_MS`) |

Compile-time relations (added next to the existing ones): `MAX_MESSAGE_BYTES * 4 <= MAX_JOURNAL_LINE_BYTES`;
`JOURNAL_RESTART_MIN_MS < JOURNAL_RESTART_MAX_MS`; `PAIR_WINDOW_US < ALERT_COALESCE_WINDOW_US`;
`ALERT_COALESCE_WINDOW_US < ANCHOR_WINDOW_US`; `MAX_CURSOR_LEN <= MAX_FIELD_BYTES`;
`JOURNAL_IDLE_TICK_MS * 1000 == PAIR_WINDOW_US`; `JOURNAL_BATCH_PAUSE_MS < JOURNAL_IDLE_TICK_MS` (back-pressure
pauses never look like "caught up"); `JOURNAL_READ_CHUNK_BYTES <= MAX_JOURNAL_LINE_BYTES`;
`ALERTS_EPOCH_HEX_LEN == 16` (64 bits).

### 3.2 Configuration (`remote.toml`)

```rust
/// Password-alert settings; `Default` = off, built-in lock-screen programs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertsConfig {
    /// `password_alerts`; default `false` (A-2).
    pub enabled: bool,
    /// `lock_screen_programs`: 0..=MAX_LOCK_SCREEN_PROGRAMS absolute paths, each 1..=4096 bytes, no
    /// trailing `/`, no `..` component, not ending in ` (deleted)`, no control byte; duplicates removed, order
    /// kept. Trust (§2.4) compares the configured strings; coverage (§4.4 `lock_screen`) additionally
    /// requires that at least one of them exists as a regular file when the follower starts.
    pub lock_screen_programs: Vec<String>,
}
impl Default for AlertsConfig { /* enabled: false, lock_screen_programs: DEFAULT_LOCK_SCREEN_PROGRAMS */ }

/// Built-in default when the key is absent.
pub const DEFAULT_LOCK_SCREEN_PROGRAMS: [&str; 4] =
    ["/usr/bin/swaylock", "/usr/bin/hyprlock", "/usr/bin/gtklock", "/usr/bin/waylock"];

pub struct RemoteConfig { /* existing fields */ pub alerts: AlertsConfig }
```

New `FileConfig` keys: `password_alerts: Option<bool>`, `lock_screen_programs: Option<Vec<String>>`. New
`ConfigError` variants (exit 78): `TooManyLockScreenPrograms { max }`, `InvalidLockScreenProgram { index }` (the
path is never echoed).

```rust
/// Pure. `<parent of credentials_path>/ALERTS_ACK_FILE_NAME`.
/// # Errors  `ConfigError::InvalidCredentialsPath` when there is no parent or the result exceeds
/// `MAX_CREDENTIALS_PATH_LEN`.
pub fn resolve_alerts_ack_path(credentials_path: &Path) -> Result<PathBuf, ConfigError>;
```

### 3.3 Sentinel semantics

| Value | Meaning |
|---|---|
| `password_alerts` absent | `false` |
| `lock_screen_programs` absent | `DEFAULT_LOCK_SCREEN_PROGRAMS` |
| `lock_screen_programs = []` | no owner-UID lock-screen line is ever trusted (only root-side signals and owner-side checks anchored by a trusted root-side failure count); `lock_screen` = `not_configured` |
| no configured lock-screen program exists as a regular file at follower start (the default list on a host whose locker lives elsewhere, e.g. `~/.local/bin/swaylock-plugin`) | `lock_screen` = `not_configured`; the page shows "lock screen not monitored — see setup" (F-2); trust rules unchanged |
| `_EXE` with one trailing ` (deleted)` | compared without that suffix (§2.4) |
| owner login unresolvable or invalid at startup | alerts `unavailable` / `owner_unresolved`; the service still starts (status, lock, unlock unaffected) |
| ack file absent | marker 0 (nothing acknowledged) |
| ack file larger than 256 bytes, not JSON, wrong owner, not `0600`, a symlink, `version ≠ 1` | marker 0 and one `password alert acknowledgement not persisted` WARN (fail-safe: more alerts shown, never fewer) |
| `X-Soos-Alerts-Through: 0` (with the current epoch) | no-op, `200` with the current view |
| `X-Soos-Alerts-Through` > highest attempt seq of the current epoch | `400 bad_request` |
| `X-Soos-Alerts-Epoch` well-formed but not the current epoch | `409 stale_view`, nothing acknowledged |
| a `?through=` query string on the ack | ignored (dropped by the HTTP parser); the headers decide |
| RNG failure when drawing the epoch | `unavailable` / `rng_failed`; nothing spawned; ack `503 unavailable` |
| attempt seq overflow (`checked_add`) | state `unavailable` / `overflow`; follower stops; the service keeps serving |
| wall clock before the epoch | `now_s = 0`; `--since=@0` is the rebuild window (bounded by the journal itself) |

## 4. Correlation and history (`alerts.rs`, pure parts)

### 4.1 Types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptKind { WrongPassword, LockedOut }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt { pub class: SourceClass, pub account: AccountClass, pub kind: AttemptKind, pub at_us: u64 }

/// Pure state machine (no clock, no I/O). Bounded by MAX_PENDING_CHECKS + MAX_RECENT_FAILURES + 6 anchors.
pub struct Correlator { /* pending checks, recent failures, anchors keyed by (HelperSide, AccountClass) */ }
impl Correlator {
    #[must_use] pub fn new() -> Self;
    /// Feeds one signal; returns the attempts it completes (0 or 1, plus up to one resolved by overflow).
    pub fn push(&mut self, signal: Signal) -> Vec<Attempt>;
    /// Resolves every pending check with `at_us + PAIR_WINDOW_US < now_us`.
    pub fn expire(&mut self, now_us: u64) -> Vec<Attempt>;
    /// Journal time of the oldest pending (unresolved) check, if any; input of the marker rule (§4.3).
    #[must_use] pub fn oldest_pending_us(&self) -> Option<u64>;
}
```

### 4.2 Rules (each one a test, §12.2)

1. `Failure` (trusted or not): if an unpaired pending `Check` with the same side and account and `|Δt| ≤
   PAIR_WINDOW_US` exists, it is removed (paired). The failure is stored in `recent_failures` as paired-capable.
   Trusted → emit `Attempt{class, account, WrongPassword, at_us}` and set `anchor[(side, account)] = (class, at_us)`.
   Untrusted → emit nothing, no anchor.
2. `Check`: if an unpaired recent failure with the same side and account within `PAIR_WINDOW_US` exists, mark it
   paired and emit nothing. Otherwise append to pending (overflow: resolve the oldest pending check now by rule 3).
3. Resolving a pending check (by `expire` or overflow): an anchor of the same side and account with
   `at_us − anchor.at_us ≤ ANCHOR_WINDOW_US` (saturating; a check older than the anchor counts as within) → emit
   with the anchor's class and refresh `anchor.at_us = max(anchor.at_us, at_us)`; else side `Root` → emit with class
   `Other`; else (side `Owner`) → drop.
4. `LockedOut` (always trusted by construction): emit `Attempt{class, account, LockedOut, at_us}`; no anchor change.
5. One pending check pairs with at most one failure and vice versa.
6. `expire` is called by the follower with the newest entry time after every line, and with the wall clock only
   when no line was read for at least `JOURNAL_IDLE_TICK_MS` (monotonic), so a fast backlog replay never expires
   a check before its own `pam_unix` line has been read.

Scenario table (each row is a correlator test):

| Input (in arrival order) | Attempts |
|---|---|
| lock screen: `Check(Owner,Owner,t)`, trusted `Failure(LockScreen,Owner,Owner,t+5ms)` | 1 × `lock_screen` |
| then `Check(Owner,Owner,t+60s)` alone, expire | +1 × `lock_screen` (anchor) |
| `Check(Owner,Owner)` alone, no anchor, expire | 0 (dropped, A-6) |
| test binary: untrusted `Failure(LockScreen,Owner,Owner,t)`, `Check(Owner,Owner,t+3ms)` | 0 |
| sudo: trusted `Failure(Sudo,Root,Owner,t)` (UID 1000, `_EXE=/usr/bin/sudo`), `Check(Root,Owner,t−4ms)` | 1 × `sudo` |
| then `Check(Root,Owner,t+7s)`, expire | +1 × `sudo` |
| `Check(Root,Other)` alone, no anchor, expire | 1 × `other` |
| gdm: trusted `Failure(Login,Root,Owner)` with `_UID=0` + `Check(Root,Owner)` | 1 × `login` |
| `LockedOut(Login,Owner)` ×3 | 3 × `login` / `locked_out` |
| `Check` older than 1 h after the last anchor, side `Owner` | 0 |
| 17 pending checks | the oldest is resolved immediately |

### 4.3 History (`AlertBook`)

```rust
/// Per-start view identity (A-13). 64 random bits; rendered as ALERTS_EPOCH_HEX_LEN lowercase hex.
/// Debug is redacted ("AlertsEpoch(<redacted>)"); never logged, never persisted.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AlertsEpoch(u64);
impl AlertsEpoch {
    /// Draws 8 bytes from the crate's `RandomSource` (`getrandom`).
    /// # Errors  `RandomError` when the source fails (→ `unavailable` / `rng_failed`).
    pub fn draw(random: &RandomSource) -> Result<Self, RandomError>;
    /// Test seam and pure constructor.
    #[must_use] pub fn from_u64(value: u64) -> Self;
    /// Exactly 16 lowercase hex characters (zero-padded).
    #[must_use] pub fn to_hex(self) -> String;
    /// Exactly 16 characters of `[0-9a-f]`; anything else (uppercase included) → `None`.
    #[must_use] pub fn parse_hex(raw: &[u8]) -> Option<Self>;
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AlertRecord {
    pub id: u64,               // seq of the first attempt of the record
    pub first_unix_ms: u64,    // first_us / 1000
    pub last_unix_ms: u64,     // last_us / 1000
    pub source: SourceClass,
    pub account: AccountClass,
    pub kind: AttemptKind,
    pub count: u32,            // saturating
    pub acknowledged: bool,
    #[serde(skip)] pub last_seq: u64,
    #[serde(skip)] pub first_us: u64,  // min journal time of the attempts in the record
    #[serde(skip)] pub last_us: u64,   // max journal time of the attempts in the record
}

pub struct AlertBook { /* epoch, started_us, VecDeque<AlertRecord> ≤ MAX_ALERT_HISTORY, next_seq: u64,
                          ack_high_water_us: u64, evicted_unacknowledged: u32, evicted_max_seq: u64,
                          evicted_first_us: Option<u64> */ }
impl AlertBook {
    /// `acknowledged_until_us`: the marker read from the ack file (0 when absent/invalid);
    /// `started_us`: wall-clock time (µs since the epoch, saturating at 0) when the book is created at service start.
    #[must_use] pub fn new(acknowledged_until_us: u64, epoch: AlertsEpoch, started_us: u64) -> Self;
    /// Records one attempt; `Err(Overflow)` when the seq counter cannot advance.
    pub fn record(&mut self, attempt: Attempt) -> Result<(), BookError>;
    /// Acknowledges every record whose `last_seq <= through`, for the current epoch only.
    /// # Errors  `StaleView` when `epoch` differs (nothing changes); `BeyondNewest` when `through` exceeds
    /// the highest seq recorded (nothing changes).
    pub fn acknowledge(&mut self, epoch: AlertsEpoch, through: u64) -> Result<(), BookError>;
    /// The marker to persist (rule M below); pure.
    #[must_use] pub fn marker(&self, oldest_pending_us: Option<u64>) -> u64;
    #[must_use] pub fn view(&self, state: AlertsState, lock_screen: LockScreenCoverage) -> AlertsView;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BookError {
    #[error("alert counter overflow")] Overflow,
    #[error("through is beyond the newest attempt")] BeyondNewest,
    #[error("the acknowledged view belongs to an earlier start")] StaleView,
}
```

Rules:

- **R (replay only, F-3 b).** `record` marks an attempt acknowledged on arrival **iff** `at_us <=
  acknowledged_until_us` (the loaded marker) **and** `at_us < started_us`. Every other attempt — in particular every
  attempt whose journal time is after the service start, and every pending check resolved after an
  acknowledgement of this run — is recorded unacknowledged. The in-run acknowledgement never applies to future
  attempts: it only flips the flag of records that exist when `acknowledge` runs.
- **A (acknowledge).** `acknowledge(epoch, through)`: epoch mismatch → `StaleView`; `through == 0` → no-op `Ok`;
  `through > highest seq` → `BeyondNewest`; else every record with `last_seq <= through` becomes acknowledged,
  `ack_high_water_us = max(ack_high_water_us, max last_us of the records acknowledged now)`, and the evicted
  counters (`evicted_unacknowledged`, `evicted_first_us`) are reset when `through >= evicted_max_seq`. A record that
  grew after the snapshot (`last_seq > through`) stays unacknowledged as a whole. Within one epoch the page's
  `through` is the highest seq it displayed, so the acknowledgement covers exactly the attempts the banner counted.
- **M (marker, F-3 b).** `marker(oldest_pending_us) = min(ack_high_water_us, earliest_unacked − 1)` (saturating at
  0), where `earliest_unacked` = the minimum of `first_us` over unacknowledged records, `evicted_first_us`, and
  `oldest_pending_us`; when none exists, the marker is `ack_high_water_us`. Hence the persisted marker is always
  strictly below every attempt the service knows to be unacknowledged, including checks still pending, and a
  restart never turns them into acknowledged ones (rule R then re-shows them). The marker may **decrease** (an
  attempt resolved late, below the previous marker): that is the fail-safe direction (more alerts after a restart,
  never fewer).
- **P (persistence).** After every acknowledgement the server computes `marker(...)` and writes it at once when it
  differs from the last written value. After any book or correlator change in the follower, the same comparison
  runs and a differing value is written at most once per `ALERTS_MARKER_FLUSH_MIN_INTERVAL_MS` (the latest value
  at the end of the interval; a pending flush is executed on the next follower tick). `ack_path == None` (no
  credential directory) keeps everything in memory. A write failure keeps the in-memory state and emits the WARN
  audit event once per failure transition.
- **Coalescing** joins the attempt to the **newest** record only, when source, account, kind and acknowledged flag
  are equal and `|at_us − last_us| ≤ ALERT_COALESCE_WINDOW_US` (`count` saturating `+1`, `first_us = min`,
  `last_us = max`, `last_seq = seq`); otherwise a new record with `id = seq`. Since live attempts are always
  unacknowledged (rule R) a late attempt never joins an acknowledged record.
- **Eviction**: the oldest record; when it was unacknowledged its count is added (saturating) to
  `evicted_unacknowledged`, `evicted_max_seq` is updated and `evicted_first_us = min(evicted_first_us, first_us)`.

### 4.4 The view (JSON of `GET /api/alerts` and of `event: alerts`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertsState { Disabled, Starting, Active, Unavailable }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnavailableReason { NoJournalAccess, JournalReaderFailed, OwnerUnresolved, Overflow, RngFailed }

/// Lock-screen coverage (A-3, F-2), evaluated at every follower start (step 1 of §5) with
/// `std::fs::metadata(path).is_ok_and(|m| m.is_file())` on each configured path (at most 4 `stat` calls, no
/// path echoed anywhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LockScreenCoverage { Monitored, NotConfigured }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AlertsView {
    pub state: AlertsState,
    pub reason: Option<UnavailableReason>,          // Some iff state == Unavailable
    pub epoch: Option<String>,                      // 16 lowercase hex (A-13); None iff Disabled or RngFailed
    pub lock_screen: Option<LockScreenCoverage>,    // None iff Disabled, or before the first follower start
    pub unacknowledged_wrong_password: u32,         // saturating, includes evicted
    pub unacknowledged_locked_out: u32,
    pub last_unix_ms: Option<u64>,                  // newest unacknowledged attempt
    pub last_source: Option<SourceClass>,
    pub through: u64,                               // highest attempt seq recorded (0 = none)
    pub history: Vec<AlertRecord>,                  // newest first, ≤ MAX_ALERT_HISTORY
}
```

`Disabled` view: every count 0, `None`s, `through` 0, empty history. The JSON never contains a user name, uid,
service name, executable, cursor, message or any raw field (§12.3 needle test). Exact top-level key set (always
present, `null` for `None`): `state`, `reason`, `epoch`, `lock_screen`, `unacknowledged_wrong_password`,
`unacknowledged_locked_out`, `last_unix_ms`, `last_source`, `through`, `history`; exact record key set: `id`,
`first_unix_ms`, `last_unix_ms`, `source`, `account`, `kind`, `count`, `acknowledged`.

## 5. The follower task (`alerts::run_follower`)

```rust
/// Settings resolved by `main` (production) or a test.
pub struct AlertSettings {
    pub owner_login: Option<OwnerLogin>,     // None → Unavailable(OwnerUnresolved), no spawn
    pub lock_screen_programs: Vec<String>,
    pub ack_path: Option<PathBuf>,           // None → acknowledgements live in memory only
}
```

Set-up (once, in `serve`, before the accept loop): draw the epoch with `AlertsEpoch::draw(state.random())`
(failure → `Unavailable(RngFailed)`, audit WARN, no follower, no book: the view has `epoch: null` and the ack is
`503 unavailable`); read the ack file (marker, A-9); create `AlertBook::new(marker, epoch, now_us)`.

Loop (never returns while the server runs; aborted at shutdown with the other supervised tasks; a panic is
`ServeError::TaskPanicked` like the poller):

1. State `Starting`. Evaluate `lock_screen` coverage (`LockScreenCoverage`, §4.4) from the configured paths.
   `probe()`; failure → state `Unavailable` (`NotFound`/`Spawn`/`ProbeTimeout` → `journal_reader_failed`,
   `NoAccess` → `no_journal_access`), audit WARN, sleep backoff, retry from 1.
2. `follow(start)`: first start `Since{now_s − HISTORY_REBUILD_WINDOW_S}`; later starts `AfterCursor(last)` when a
   valid cursor was seen, else `Since{last_seen_us / 1_000_000}` with every entry whose `realtime_us ≤
   last_seen_us` skipped. A follower started `AfterCursor` that ends within 1 s without a line clears the cursor
   (journal vacuumed or rotated past it). Record `follow_started_us` = wall clock when `follow` returned.
3. **Catch-up (F-5).** The state stays `Starting` while the backlog is read. It becomes `Active` (audit INFO once
   per transition) at the first of: (a) an accepted entry (any parse-valid line, signal or not) with
   `realtime_us ≥ follow_started_us`; (b) an idle tick: no line read for `JOURNAL_IDLE_TICK_MS` (monotonic) since
   the last line or since `follow` returned. Back-pressure pauses (`JOURNAL_BATCH_PAUSE_MS` < the idle tick, §3.1)
   never count as idle. Rationale: `journalctl --follow` prints the whole `--since`/`--after-cursor` backlog before
   waiting, so the first idle gap or the first entry newer than the spawn is the end of the backlog; no second
   child process and no exit-status plumbing are needed. Residual: a `journalctl` stalled for more than 2 s inside
   the backlog (extremely slow storage) can switch to `Active` early; later backlog lines still count normally.
   For each line: `Overlong` → skip; `parse_entry` error → skip; update `last_seen_us` and the cursor;
   `classify_entry` → `Correlator::push`; `expire(newest_us)`; each attempt → `AlertBook::record` (overflow →
   `Unavailable(Overflow)`, audit WARN, stop for good); a changed book bumps the `watch` version; then the
   marker comparison of rule P (§4.3) with `Correlator::oldest_pending_us()`. Every `JOURNAL_LINES_PER_BATCH`
   lines → sleep `JOURNAL_BATCH_PAUSE_MS`. Idle tick (rule 6 of §4.2) every `JOURNAL_IDLE_TICK_MS`; it also runs a
   pending marker flush.
4. `End` → state `Unavailable(JournalReaderFailed)`, audit WARN, backoff (`JOURNAL_RESTART_MIN_MS` doubling to
   `JOURNAL_RESTART_MAX_MS`; reset after a run ≥ `JOURNAL_STABLE_RUN_MS`), go to 1. The book, the epoch, the
   correlator and the acknowledgement state survive restarts of the child (the epoch changes only with a new
   service start).

The book lives in `Shared` behind a `std::sync::Mutex` (never held across an `.await`; a poisoned lock is treated
as `Unavailable(JournalReaderFailed)` for reads and `503 unavailable` for the ack, never a panic). A
`watch::Sender<u64>` carries the version.

## 6. HTTP surface

| Method + path | Gate order (after the existing host, classification, Funnel capacity and Funnel session steps) | Responses |
|---|---|---|
| `GET`/`HEAD /api/alerts` | none beyond authentication; anonymous Funnel → `403 login_required` (route not in `is_funnel_public`) | `200 application/json` `AlertsView` (disabled → the `Disabled` view) |
| `POST /api/alerts/ack` | CSRF (`check_alerts_ack_csrf`: `X-Soos-Action: alerts-ack`, `Sec-Fetch-Site` absent or `same-origin`, `Origin` absent or `https://<effective host>`) `403 forbidden` → disabled `403 alerts_disabled` → `parse_alerts_ack_headers` (§6.1) `400 bad_request` → rate gate `429 rate_limited` (consumed by every request that reached it) → no book (`rng_failed`) or poisoned book mutex `503 unavailable` → `StaleView` `409 stale_view` → `BeyondNewest` `400 bad_request` → apply in memory, release the mutex, then persist per rule P (failure: audit WARN, still `200`) | `200` `AlertsView` after the acknowledgement; any body → `413 body_not_allowed` (unchanged head rule); a query string is ignored |
| `GET /api/events` | unchanged | when alerts are not `Disabled`: one `event: alerts` right after the first `event: status`, then one per version change, at most one per `ALERT_EVENT_MIN_INTERVAL_MS` per stream (the newest view at the end of the interval). **On a Funnel session stream, `shared.session_still_valid(hash)` (revocation first, `Touch::Keep`) runs immediately before every `alerts` event is written; an invalid session ends the stream without writing the event** (F-4). Status events, keep-alive timing, the 15 s session re-check of status-only periods and every existing stream rule are unchanged; alert events never reset the status keep-alive deadline. Tailnet streams need no re-check (the identity is per request and per stream, as today). |

`encode_sse_alerts_event(json) = "event: alerts\ndata: <json>\n\n"`. `Route::Alerts` / `Route::AlertsAck` paths:
`ALERTS_PATH = "/api/alerts"`, `ALERTS_ACK_PATH = "/api/alerts/ack"`; `allow_header` returns `POST` for the ack
path and `GET, HEAD` for `/api/alerts`.

### 6.1 Acknowledgement target (headers, F-1)

```rust
/// The snapshot an acknowledgement names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AckTarget { pub epoch: AlertsEpoch, pub through: u64 }

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AckHeaderError {
    #[error("acknowledgement epoch header missing, repeated or malformed")] Epoch,
    #[error("acknowledgement through header missing, repeated or malformed")] Through,
}

/// Pure (routes.rs). Uses the existing `optional_single` rule: exactly one `x-soos-alerts-epoch` whose bytes
/// `AlertsEpoch::parse_hex` accepts, and exactly one `x-soos-alerts-through` of 1..=MAX_ALERTS_THROUGH_DIGITS ASCII
/// digits whose value fits `u64` (leading zeros accepted). Absent, repeated, empty, signed, whitespace-padded
/// beyond the parser's trimming, non-ASCII or overflowing → the matching error (→ `400 bad_request`).
/// Never reads `head.path` (the query is not there) and never echoes a value.
pub fn parse_alerts_ack_headers(head: &RequestHead) -> Result<AckTarget, AckHeaderError>;
```

The page sets both headers from the view it displays (`epoch`, `through`). `RequestHead` and `parse_request_head`
are unchanged; a client that still appends `?through=` gains nothing (no header → `400`).

The ack file write (temp file `O_CREAT|O_EXCL|O_NOFOLLOW` mode `0600` in the same directory, `fsync`, `rename`,
directory `fsync`; content `{"version":1,"acknowledged_until_us":<u64>}`) happens after the book mutex is
released, synchronously like the credential store (single small local write). The read at follower start uses
`O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC`, `fstat` owner = service uid and mode `0600`, ≤ 256 bytes.

## 7. Error taxonomy (→ observable result)

| Failure | Result |
|---|---|
| `journalctl` missing / spawn error / probe timeout | `unavailable` / `journal_reader_failed`, retried with backoff |
| no system-journal access | `unavailable` / `no_journal_access`, retried with backoff (a group change needs a new user-manager session, i.e. log out and in, not a service restart; G-8) |
| child exit, read error | `unavailable` / `journal_reader_failed`, history kept, restart after backoff |
| malformed, overlong, foreign or untrusted line | skipped silently (no log: the line is untrusted input) |
| owner login unresolved | `unavailable` / `owner_unresolved`, nothing spawned |
| seq overflow | `unavailable` / `overflow`, follower stopped |
| ack file unreadable or invalid | marker 0 + WARN (more alerts shown, never fewer) |
| ack persist failure | in-memory acknowledgement kept, WARN, `200` |
| poisoned book mutex | view `unavailable` / `journal_reader_failed`; ack `503 unavailable` |
| RNG failure for the epoch | `unavailable` / `rng_failed`, nothing spawned, ack `503 unavailable` |
| acknowledgement from an earlier start (epoch mismatch) | `409 stale_view`, nothing acknowledged; the page re-fetches the view |
| acknowledgement headers missing, repeated or malformed | `400 bad_request`, nothing acknowledged |
| no configured lock-screen program on disk | `lock_screen: not_configured` (visible on the page), counting of other sources unaffected |
| Funnel session revoked while an event stream is open | the stream ends before the next `alerts` event (and at the next 15 s status re-check, unchanged) |

No failure of this feature can change a status, lock or unlock outcome, end `serve`, or reach `PAM`; the feature
never reports `active` with zero attempts unless the probe succeeded, the follower is running and the backlog has
been read (§5 step 3).

## 8. Latency

Trusted `pam_unix`/`pam_faillock` line → attempt immediately; helper-only check → after `PAIR_WINDOW_US` (2 s) plus
at most one idle tick (`JOURNAL_IDLE_TICK_MS`, 2 s). The Funnel session re-check
before each `alerts` event runs `session_still_valid`, whose `revoke()` step reads the credential store file
(bounded by `MAX_CREDENTIAL_STORE_BYTES`, at most once per second per Funnel stream because of the event throttle;
G-3). Journal → `journalctl --follow` is inotify-driven (sub-second). SSE throttle ≤ 1 s.
**Budget: an alert reaches an open page within 5 s** of the attempt (hardware row RMC59). Not on the PAM path: no
PAM, daemon or camera latency changes.

## 9. Page (`assets/`)

- A banner `#alerts` hidden while `state == "disabled"`. `active` with unacknowledged attempts: text
  `"<n> failed password attempt(s), last at HH:MM (<source>)"`, followed by `" and <m> attempt(s) while locked
  out"` when `m > 0`; `active` with none: `"No failed password attempts"`; `starting`: `"Password alerts
  starting"`; `unavailable`: `"Password alerts unavailable"` (reason mapped: `no_journal_access` → "the account
  cannot read the system journal", `journal_reader_failed` → "journal reader stopped", `owner_unresolved`,
  `overflow`, `rng_failed`). In `starting` and `unavailable` the counts already known are still shown when
  non-zero, prefixed by the state line, and the text "No failed password attempts" is never shown (A-3).
- Lock-screen coverage (F-2): when `lock_screen == "not_configured"` and the state is not `disabled`, a second line
  `"Lock screen not monitored — see setup"` is always shown (also next to "No failed password attempts", which then
  reads `"No failed password attempts (sudo, login, other)"`). Source labels: `lock_screen` → "lock screen", `sudo` → "sudo", `login` → "login", `other` →
  "other". Account labels: `owner` → "your account", `root` → "root", `other` → "another account".
- `HH:MM` from `last_unix_ms` in the phone's locale and time zone
  (`toLocaleTimeString([], {hour: "2-digit", minute: "2-digit"})`).
- History list (newest first, at most 10 shown): `HH:MM <source>, <account>, <count> wrong password(s)` or
  `… attempt(s) while locked out`; acknowledged rows dimmed.
- Button "Acknowledge" (only with unacknowledged attempts): `POST /api/alerts/ack` without a body, with the
  headers `X-Soos-Action: alerts-ack`, `X-Soos-Alerts-Epoch: <epoch of the view displayed>` and
  `X-Soos-Alerts-Through: <through of the view displayed>`; the `200` response view replaces the displayed one; on
  `409 stale_view` the page re-fetches `GET /api/alerts` and shows the fresh view without acknowledging (the owner
  presses again after seeing it).
- Updates: `fetch('/api/alerts')` after a successful status load and on `visibilitychange`/`pageshow`;
  `source.addEventListener("alerts", …)` on the existing `EventSource`.
- `textContent` only; no `innerHTML`; nothing stored in `localStorage`/`sessionStorage`; anonymous Funnel pages
  (sign-in view) never call the alert routes.

## 10. Invariants touched

| Invariant | How it holds |
|---|---|
| No passwords anywhere (`AGENTS.md`, O-2) | no producer carries them; account mapped to 3 classes at parse time; no raw field in JSON/logs; needle tests. |
| No network socket (RMC-S2, ADR 2026-10-05 item 2) | the child reads journal files; no new address family; unit unchanged. |
| Leaf crate (RMC-S4), no Tokio in `pam_soos.so` | only `crates/remote` gains the `process` feature; the PAM crate is untouched. |
| Environment read only by `main.rs` (RMC-S9) | the child gets `env_clear()`; no `env::var` in `journal.rs`/`alerts.rs`. |
| No panic in production (RC-5) | every parse is fallible; `checked_add`/`saturating_*`; mutex poison handled. |
| Bounded everything (O-4) | §3.1. |
| Authenticated only (O-4) | routes not Funnel-public; SSE already authenticated; Funnel session re-checked before each `alerts` event (F-4). |
| Fail-closed (A-3) | state always in the view; probe; `active` only after catch-up; lock-screen coverage in the view; no "0 attempts" when not active. |
| An acknowledgement never hides an unseen attempt (A-9, A-10, A-13) | per-start epoch; `last_seq <= through` within the epoch; replay-only marker rule R; marker M below every known unacknowledged attempt. |
| Test immutability (`AGENTS.md`) | `RequestHead` unchanged; only the two `RemoteConfig` literals gain a setup line (A-12). |
| The crate-level forbid lint | unchanged; `tokio::process` needs no raw-memory API (no `pre_exec`, no `PR_SET_PDEATHSIG`). |

## 11. Test hooks

- `ServerState::with_password_alerts(settings: AlertSettings, source: Arc<dyn JournalSource>) -> Self` (production
  wiring and tests; enabled only when `config.alerts.enabled`; with `enabled == false` the call is ignored).
- `crates/remote/tests/common/journal.rs` (new, tester-owned): `ScriptedJournal` implementing `JournalSource`
  (settable probe result; `follow` records every `FollowStart`; lines pushed by the test; `end()` ends the current
  follower) and line builders `chkpwd_line(uid, name, realtime_us)`, `pam_unix_line(service, uid, exe, name,
  realtime_us)`, `faillock_locked_line(…)`, `with_field(line, key, value)`.
- Clock: the existing `with_unix_clock` (wall clock for `--since` and the idle expiry); tokio paused time for
  backoff and throttling.
- Audit capture: the existing log-capture writer of the harness (fixed-text events).
- Epoch: drawn from the `ServerState` `RandomSource`, so `with_random` (existing seam) makes it deterministic or
  failing (`rng_failed`) in server tests; `AlertsEpoch::from_u64` for pure book tests.
- Marker inputs: `Correlator::oldest_pending_us` and `AlertBook::marker` are pure and public.
- Child PID: `JournalLines::child_pid` (`#[doc(hidden)]`, default `None`) for test 43.
- Header parsing: `routes::parse_alerts_ack_headers` is pure and public (test 40).

## 12. Test list for the tester (Phase 2)

Every test name below is a contract; all must fail before Phase 4 (the items do not exist yet).

### 12.1 `crates/remote/tests/journal_tests.rs` (new, pure)

1. `test_rmc_alerts_probe_and_follow_args_are_exact` — `probe_args()` and `follow_args()` for `Since{1}` and
   `AfterCursor`, element by element; no `--user`, `-g`, `--grep`, `--all`, `--cursor-file`; match last.
2. `test_rmc_alerts_journal_cursor_bounds` — accepted charset, 256 accepted / 257 refused, empty refused, Debug redacted.
3. `test_rmc_alerts_parse_entry_accepts_string_and_byte_array_fields` — the observed shapes (string `MESSAGE`; byte-array `MESSAGE` of printable UTF-8).
4. `test_rmc_alerts_parse_entry_refuses_bad_shapes` — null, array of strings, nested, number, non-UTF-8 byte array, control characters (ANSI), duplicate read key (each of the nine, first and last value both valid, so a last-wins parser would accept it), a duplicated unknown key accepted, trailing bytes after the object, a string field one byte over its bound, missing required field, facility ≠ 10, overlong line, `_UID` not decimal, timestamp with 21 digits.
5. `test_rmc_alerts_parse_entry_never_panics` (proptest, arbitrary bytes ≤ 30 000).
6. `test_rmc_alerts_classify_unix_chkpwd` — `_UID` 0 → `Root`, owner → `Owner`, other UID → `None`; identifier, `_COMM`, `_EXE` gates; grammar edge cases (missing `)`, empty name, 257-byte name).
7. `test_rmc_alerts_classify_pam_unix_trust_table` — every row of the §2.4 trust table, including `_EXE` absent, test-binary `_EXE`, `/usr/bin/sudo`, `/usr/bin/su`, `lock_screen_programs` empty, and the owner's real locker path `/home/<owner>/.local/bin/swaylock-plugin` trusted only when configured.
8. `test_rmc_alerts_classify_service_classes` — every service of the three lists plus `polkit-1`, `su`, an invalid SVC (`swaylock)`), 65-byte SVC.
9. `test_rmc_alerts_classify_faillock_locked_out_only` — the locked-out grammar is a signal; `Consecutive login failures …`, `User unknown`, `Error sending audit message`, untrusted locked-out → `None`.
10. `test_rmc_alerts_ignored_lines` — `conversation failed`, `auth could not identify password`, `session opened`, `soos-pam:` lines, `_TRANSPORT=journal`/`stdout` copies of a valid message → `None`; a `sudo` command-log line (`_UID=0`, identifier `sudo`, message `<owner> : TTY=… ; COMMAND=/usr/bin/echo pam_unix(sudo:auth): authentication failure; … user=root`) → `None` (prefix anchoring).
11. `test_rmc_alerts_account_is_mapped_to_three_classes` — owner name, `root`, a password-looking name, a name equal to the owner login with different case (→ `Other`), absent `user=` → `Other`.
12. `test_rmc_alerts_owner_login_bounds` — `OwnerLogin::parse` accepted/refused forms; Debug redacted.

### 12.2 `crates/remote/tests/alerts_tests.rs` (new, pure)

13. `test_rmc_alerts_correlator_scenarios` — every row of the §4.2 scenario table.
14. `test_rmc_alerts_correlator_pairs_in_both_orders` — check-then-failure and failure-then-check, 1 attempt each; Δt = 2 s pairs, 2 s + 1 µs does not.
15. `test_rmc_alerts_correlator_bounds` — 17 pending checks, 17 recent failures; memory stays bounded (sizes observable through the attempts emitted).
16. `test_rmc_alerts_book_coalescing_and_eviction` — 60 s window, different kind/source/account split, 33 records evict, `evicted_unacknowledged` counted.
17. `test_rmc_alerts_book_counts_saturate` — `count` and totals at `u32::MAX`.
18. `test_rmc_alerts_book_acknowledge_through` — `through` semantics: a record that grew after the snapshot stays unacknowledged; `through=0` no-op; beyond newest → `BeyondNewest`; another epoch → `StaleView` with nothing changed; `ack_high_water` never decreases.
19. `test_rmc_alerts_book_rebuild_respects_marker` — with `started_us = S`: attempts at or before the loaded marker and before `S` recorded acknowledged; an attempt at or before the marker but at or after `S` recorded unacknowledged.
20. `test_rmc_alerts_book_seq_overflow` — `Overflow`, no panic.
21. `test_rmc_alerts_view_json_shape` — exact top-level and record key sets (§4.4) and enum spellings (`rng_failed`, `monitored`, `not_configured` included); `epoch` 16 lowercase hex; `Disabled` view (`epoch` and `lock_screen` null); `reason` iff `unavailable`.
22. `test_rmc_alerts_ack_file_round_trip_and_fail_safe` — `0600` written atomically; absent → 0; symlink, wrong mode, 257 bytes, bad JSON, `version: 2` → 0.

### 12.3 `crates/remote/tests/alerts_server_tests.rs` (new, harness + `ScriptedJournal`)

23. `test_rmc_alerts_disabled_by_default` — default config: `GET /api/alerts` → disabled view; ack → `403 alerts_disabled`; `ScriptedJournal` never probed; an event stream carries no `event: alerts`.
24. `test_rmc_alerts_routes_require_authentication` — tailnet allowed identity `200`; foreign identity `403`; anonymous Funnel `403 login_required` for both routes; Funnel with a session `200`; F-4 case: a Funnel session stream receives one `alerts` event, the session is then revoked (logout from another connection, or its passkey removed), an attempt is pushed, and the stream ends **without** a further `alerts` event, well before the 15 s keep-alive re-check (paused time: no time advance beyond the 1 s throttle).
25. `test_rmc_alerts_end_to_end_lock_screen_burst` — research §2.2 day replayed (lock-screen, sudo, polkit, gdm, locked-out lines, test-binary noise) with `lock_screen_programs` set explicitly to a regular file created in the test's temp directory, used as `_EXE` of the locker lines (one of them with the ` (deleted)` suffix): exact counts per source and kind, `lock_screen: "monitored"`. Second case, same lines with `lock_screen_programs` absent (defaults, none of which is the locker): zero `lock_screen` attempts, the other sources unchanged (coverage is not asserted here: it depends on whether the host has e.g. `/usr/bin/swaylock`). Third case, `lock_screen_programs` = one path inside the temp directory that does not exist: `lock_screen: "not_configured"` and zero `lock_screen` attempts (host-independent).
26. `test_rmc_alerts_probe_failure_is_unavailable_never_zero` — `NoAccess` → `unavailable`/`no_journal_access`; `NotFound` → `journal_reader_failed`; then success → `active`; backoff 1 s, 2 s, 4 s (paused time).
27. `test_rmc_alerts_follower_restart_resumes_after_cursor` — `end()` → `unavailable`, restart with `AfterCursor(last)`; history and epoch kept; a cursor-start ending within 1 s without a line falls back to `Since` and skips already-seen entries.
28. `test_rmc_alerts_first_start_rebuilds_24h` — `Since{now_s − 86 400}`; saturating at 0; while backlog lines (journal time before the spawn) keep arriving without a 2 s gap the view stays `starting` even when it already holds attempts; it becomes `active` at the first entry newer than the spawn, and, in a second case, after `JOURNAL_IDLE_TICK_MS` without a line (paused time); a backlog read with batch pauses only stays `starting`.
29. `test_rmc_alerts_owner_unresolved` — `owner_login: None` → `unavailable`/`owner_unresolved`, never probed.
30. `test_rmc_alerts_ack_csrf_headers_and_rate` — missing/wrong action `403`; cross-site `403`; bad `Origin` `403`; header variants `400`: epoch absent, repeated, 15 or 17 characters, uppercase hex, non-hex; through absent, repeated, empty, `a`, `-1`, `+1`, 21 digits, `18446744073709551616`; a `?through=1` query with no through header → `400` (the query is never read); valid headers with a query → `200`; second ack within 1 s `429`; a body `413`.
31. `test_rmc_alerts_ack_persists_and_survives_restart` — ack, stop the server, start a new one on the same ack file: rebuilt attempts from before the new start are acknowledged; the new view has a different epoch.
32. `test_rmc_alerts_sse_event_order_and_throttle` — first `status` then `alerts`; 10 attempts in 100 ms → at most 2 alert events in 1 s with the final counts; status keep-alive timing unchanged.
33. `test_rmc_alerts_never_expose_raw_fields` — scripted random source (fixed epoch) and test-chosen journal times whose decimal and millisecond renderings cannot contain the needles; foreign UID `3141592653`; the needles (owner login, a password-looking user name, service names, `_EXE` paths, cursor, the foreign UID, the raw message) never appear in any response body, SSE byte or captured log line; additionally every JSON object of every view has exactly the key set of §4.4.
34. `test_rmc_alerts_audit_lines` — exactly one `password alerts active` per transition, one `password alerts unavailable` per transition, one `password alert acknowledgement not persisted` on a failed write; none carries a field.
35. `test_rmc_alerts_unavailable_never_affects_status_or_lock` — with the follower `unavailable` (probe failing) and after `ScriptedJournal` ends repeatedly, `/api/status`, `/api/events` status events and `/api/lock` behave exactly as with alerts disabled, and `serve` does not return. (Seq overflow itself is covered by test 20 on `AlertBook`.)
36. `test_rmc_alerts_existing_behaviour_unchanged_when_enabled` — with alerts enabled and lines flowing, `/api/status`, `/api/lock` and the status SSE events behave as in `server_tests.rs` (spot checks).

### 12.4 `crates/remote/tests/config_tests.rs` (new tests only)

37. `test_rmc_alerts_config_keys` — absent → `AlertsConfig::default()`; `password_alerts = true`; `lock_screen_programs` 4 accepted, 5 → `TooManyLockScreenPrograms`, relative, trailing `/`, `..`, empty string, 4097 bytes → `InvalidLockScreenProgram{index}`; duplicates removed; `[]` kept empty.
38. `test_rmc_alerts_ack_path_resolution` — sibling of the credential store; no parent → error; over-long → error.

### 12.5 `crates/remote/tests/routes_tests.rs` / `http_tests.rs` (new tests only)

39. `test_rmc_alerts_routes_table` — `GET`/`HEAD /api/alerts` → `Alerts`; `POST /api/alerts/ack` → `AlertsAck`; wrong methods `405` with the right `Allow`; `parse_request_head` of `POST /api/alerts/ack?through=5` yields path `/api/alerts/ack` (existing parser behaviour, unchanged); neither route is Funnel-public nor accepts a body.
40. `test_rmc_alerts_ack_csrf_rules` — `check_alerts_ack_csrf` table (mirrors the lock table with `alerts-ack`), and the `parse_alerts_ack_headers` table: valid pair (`0000000000000000`/`0`, `ffffffffffffffff`/`18446744073709551615`, leading zeros), and each malformed epoch / through variant of test 30 → the matching `AckHeaderError`; header names matched only in their lowercased parsed form (the parser lowercases).
41. `test_rmc_alerts_sse_alerts_event_encoding` — exact bytes of `encode_sse_alerts_event`.

### 12.6 `crates/remote/tests/journal_process_tests.rs` (new; Linux, real child process, no journal needed)

42. `test_rmc_alerts_bounded_line_reader` — `journal::BoundedLineReader` over a `tokio::io::duplex` with writes split at arbitrary points (including inside a line and across `JOURNAL_READ_CHUNK_BYTES`): exact lines, a line of exactly `MAX_JOURNAL_LINE_BYTES` accepted, one byte more → `Overlong` and the next line is read intact, partial last line → `End`; the buffer capacity observed through a `#[doc(hidden)]` `capacity()` accessor never changes after construction.
43. `test_rmc_alerts_journalctl_source_kills_child_on_drop` — runs only when `/usr/bin/journalctl` exists (otherwise returns early with a printed note): `JournalctlSource.follow(Since{now})`, read `child_pid()` (must be `Some(pid)`), wait ≤ 2 s for at most one line, drop the lines object, then keep the runtime running (yield/sleep in the runtime, so tokio's orphan reaping proceeds) and within 2 s require that `/proc/<pid>` is gone, or that `/proc/<pid>/stat` names a parent other than this process (PID reuse). It never scans `/proc/self/task/*/children`. The program path is not injectable (production wiring only).

### 12.6b Tests added in round 2 (new numbers, appended so the numbering of tests 1–50 is stable)

51. `journal_tests.rs`: `test_rmc_alerts_exe_deleted_suffix_is_stripped` — `exe_for_comparison` table (one suffix stripped; two suffixes → one left; suffix alone → empty; suffix in the middle unchanged); a lock-screen `pam_unix` line with `_EXE=<configured> (deleted)` is trusted; `/usr/bin/sudo (deleted)` trusted; `/usr/bin/sudo (deleted) (deleted)` and `/usr/bin/sudox (deleted)` untrusted; a `unix_chkpwd` line with `_EXE=/usr/bin/unix_chkpwd (deleted)` is a `Check`.
52. `alerts_tests.rs`: `test_rmc_alerts_book_marker_never_covers_unacknowledged` — `marker()` table: no records → high water; an unacknowledged record at `t` → `min(high water, t − 1)`; a pending check at `t` older than an acknowledged record ending at `t + 1 s` → marker `t − 1`; evicted unacknowledged attempts keep the marker below them until acknowledged; saturation at 0 for `t = 0`; a late attempt below the previous marker lowers it.
53. `alerts_server_tests.rs`: `test_rmc_alerts_ack_stale_epoch_is_refused` — scripted random source; an acknowledgement with a well-formed epoch of another start → `409 stale_view` and the view is unchanged; after a server restart on the same ack file, the previous view's epoch and `through` are refused although `through` is within the new run's range.
54. `alerts_server_tests.rs`: `test_rmc_alerts_pending_check_older_than_ack_stays_unacknowledged` — owner-side lock-screen check at `t` (anchored, still pending), a trusted lock-screen failure at `t + 1 s` recorded; acknowledge through that record; the pending check resolves → an **unacknowledged** record; the persisted marker is `< t`; after a restart on the same ack file the check is again shown unacknowledged.
55. `alerts_server_tests.rs`: `test_rmc_alerts_rng_failure_is_unavailable` — failing random source: view `unavailable` / `rng_failed`, `epoch` null, journal never probed, ack `503 unavailable`, status/lock unaffected.
56. `tests/invariants/src/remote_alerts_contract.rs`: `test_rmc_s31_line_buffers_are_zeroizing` — `journal.rs` declares `Line(Zeroizing<Vec<u8>>)` and `message: Zeroizing<String>`, contains no `BufReader` and no `serde_json::Value`/`serde_json::Map` (F-10, F-11).
57. `config_tests.rs` (extends the bounds of test 37, new function): `test_rmc_alerts_lock_screen_program_validation` — a path ending in ` (deleted)` and a path with a control byte → `InvalidLockScreenProgram{index}`; the error message never contains the path.

### 12.7 Superseded / setup-only changes in existing tests (A-12)

Only `crates/remote/tests/common/harness.rs` and `crates/remote/tests/server_tests.rs` gain
`alerts: AlertsConfig::default(),` in their `RemoteConfig` literals. No assertion of any existing test changes.
`RequestHead` is not changed (F-1), so `routes_tests.rs:25`, `routes_tests.rs:487` and `http_tests.rs:95` are
untouched; the candid reviewer checks that `git diff` shows no line removed or changed in any existing test file
other than those two setup lines.

### 12.8 `tests/invariants/src/remote_alerts_contract.rs` (new) + `mod remote_alerts_contract;`

44. `test_rmc_s24_journal_reader_is_spawned_safely` — exactly one `Command::new(` in `crates/remote/src`, in `journal.rs`, with `JOURNALCTL_PATH`; `env_clear()`, `kill_on_drop(true)`, `Stdio::null()` present; no `"sh"`, `"bash"`, `"-c"`, `pre_exec`, `"--user"`, `"--grep"`, `"-g"` literal in the crate.
45. `test_rmc_s25_alert_modules_never_log` — no `TRACING_MACROS` invocation in `journal.rs` and `alerts.rs`; `audit.rs` declares the three fixed messages.
46. `test_rmc_s26_unit_unchanged_for_alerts` — `RestrictAddressFamilies=AF_UNIX` still the only family line; no `SupplementaryGroups=`, `PrivateNetwork=`, `ProtectSystem=`, `ReadWritePaths=` added by this change.
47. `test_rmc_s27_process_feature_only_in_remote` — `crates/remote/Cargo.toml` tokio line has `"process"`; the workspace tokio line and every other crate manifest do not; `crates/pam/Cargo.toml` has no tokio.
48. `test_rmc_s28_alert_types_without_redaction_have_no_debug` — `JournalEntry` has no `Debug`; `OwnerLogin` and `JournalCursor` have a redacted `Debug` only.
49. `test_rmc_s29_alerts_are_documented` — `Docs/REMOTE_COMPANION.md` has §2c with the needles `password_alerts`, `lock_screen_programs`, `wheel`, `systemd-journal`, `/api/alerts`, `alerts-ack`, `never the typed password`, `false alerts`; `AI/DECISIONS.md` has the ADR title.
50. `test_rmc_s30_page_alert_ui` — `app.js` contains `/api/alerts`, `/api/alerts/ack`, `alerts-ack`, `X-Soos-Alerts-Epoch`, `X-Soos-Alerts-Through`, `stale_view`, `"alerts"` listener, `Acknowledge`, `Password alerts unavailable`, `Lock screen not monitored`, `your account`, `another account`; and none of `localStorage`, `sessionStorage`, `innerHTML`, `?through=`.

## 13. New matrix rows (after RMC44)

| ID | Criterion | Evidence |
|---|---|---|
| RMC45 | Opt-in: `password_alerts` defaults to `false`; while off nothing is spawned, `GET /api/alerts` returns the disabled view, the ack is `403 alerts_disabled`, no `alerts` SSE event; `lock_screen_programs` bounded (≤ 4 absolute paths, no ` (deleted)` suffix) | tests 23, 37, 57 |
| RMC46 | Journal lines bounded (24 KiB line, 4 KiB fields), every field shape outside string / printable byte array refused, a repeated read key refused (hand-written visitor), never panics | tests 3–5, 42 |
| RMC47 | Trust allow-list on `_TRANSPORT`, `_UID`, `_EXE` (one ` (deleted)` suffix stripped), `_COMM`, facility; other UIDs ignored; `SYSLOG_IDENTIFIER`/`MESSAGE` only as prefix-anchored grammar | tests 6–10, 51 |
| RMC48 | O-2: no password, part, length or hash anywhere; account mapped to `owner`/`root`/`other` at parse time; no raw field, user name, uid, service, executable or cursor in any response, event or log; line buffers and entry strings wiped on drop | tests 11, 21, 33, 45, 48, 56 |
| RMC49 | One attempt per real check; pairing window 2 s in both orders; anchors 1 h; owner-side checks without a trusted anchor and checks paired with an untrusted failure dropped; root-side checks without anchor counted `other` | tests 13–15, 25 |
| RMC50 | Attempts during a `pam_faillock` lockout counted as `locked_out`, separately; consequence and unknown-user lines ignored | tests 9, 13, 25 |
| RMC51 | History bounded: 32 records, 60 s coalescing, saturating counts, evicted unacknowledged kept in totals, seq overflow → `unavailable`/`overflow` | tests 16–20, 35 |
| RMC52 | Fail-closed reader: probe requires `_UID=0` visibility; `unavailable` + reason (incl. `rng_failed`), never "0 attempts" while not active; `active` only after the backlog is caught up; lock-screen coverage (`monitored`/`not_configured`) in the view and on the page; exact argv; environment cleared; backoff 1 s→60 s; cursor resume; 24 h rebuild; child killed and reaped on drop | tests 1–2, 25–29, 43–44, 55 |
| RMC53 | Routes authenticated: tailnet identity or Funnel session; anonymous Funnel `403 login_required` | tests 24, 39 |
| RMC54 | Acknowledge: lock-style CSRF with `alerts-ack`, strict `X-Soos-Alerts-Epoch` / `X-Soos-Alerts-Through` headers (query never read, `RequestHead` unchanged), 1 s rate limit, per-start epoch (`409 stale_view`), never acknowledges an unseen attempt (replay-only marker, marker below every known unacknowledged attempt incl. pending checks), persisted in a `0600` file next to the credential store, fail-safe read | tests 18–19, 22, 30–31, 38, 40, 52–54 |
| RMC55 | SSE `event: alerts` after the first status event, ≤ 1 per second per stream; a Funnel session re-validated before every `alerts` event; status events unchanged | tests 24, 32, 36, 41 |
| RMC56 | Logging: no tracing in the alert modules; three fixed-text audit events without fields | tests 34, 45 |
| RMC57 | Page: banner, history, acknowledge (headers, stale-view re-fetch), unavailable states, "Lock screen not monitored — see setup", phone-local HH:MM, `textContent` only, no storage | test 50 |
| RMC58 | Unit unchanged (`RestrictAddressFamilies=AF_UNIX`), `process` feature only in `crates/remote`, no shell, documented ADR and §2c | tests 46, 47, 49 |
| RMC59 | Hardware (owner): with `password_alerts = true`, `lock_screen_programs = ["/home/<owner>/.local/bin/swaylock-plugin"]` (mandatory on this host: the default list does not contain the locker, and the page must show `lock screen` monitored) and the owner in `wheel`, a wrong password at the lock screen, then with `sudo`, then at GDM, shows on the phone (tailnet and Funnel session) within 5 s with the right source, count and "your account"; running the `drift_verrou` tests adds nothing; *Acknowledge* clears the banner and survives a service restart | Manual check (`Docs/REMOTE_COMPANION.md` §2c); agents never restart the service (O-5) |

## 14. ADR (summary; full text in `AI/DECISIONS.md`)

"[2026-10-06] Failed-Password Alerts in `soos-remote` From the System Journal": decisions A-1 to A-14, the
relayed-request/O-2 point of §0.1, the residual risks (§15) and the setup-only test amendment.

## 15. Residual risks (documented in the ADR and in `Docs/REMOTE_COMPANION.md` §8)

1. **False alerts and suppression from the owner's own UID (F-6)**: any process running as the owner can write a
   lock-screen-looking `pam_unix` line with `_EXE` of an owner-writable lock-screen program, and `unix_chkpwd`
   lines of its own UID, so it can create false lock-screen alerts. It can also, **for as long as it keeps emitting
   forged untrusted owner-side `pam_unix` failures** (one within 2 s of each real check), make every real
   owner-side check pair with a forged failure and be dropped (rule 1/2 of §4.2): this suppresses **all**
   lock-screen attempts that only `unix_chkpwd` logs (the 2nd, 3rd, … attempt of a lock-screen PAM handle), not
   just one, and it does so **invisibly** (the state stays `active`), unlike stopping the service (which shows
   `unavailable`). The first failure of each lock-screen handle is still counted when the locker is configured
   (its own trusted `pam_unix` line emits the attempt regardless of pairing), and root-side attempts (`sudo`,
   login, polkit) cannot be suppressed this way (an untrusted failure is always owner-side and only pairs with
   owner-side checks). Developer test binaries that write real `pam_unix(swaylock:auth)` lines while the owner
   mistypes at the lock screen can cause the same suppression innocently. Accepted: such a process already runs
   as the owner (it can read `remote.toml`, reach the socket, unlock its own session through logind); root-side
   only `_UID=0` lines cannot be forged without root: `_EXE`-trusted lines (`sudo`, `su`, configured lockers) can be
   forged by an owner process through journald's PID-reuse race (false alerts and class relabelling only, never the
   suppression of a trusted failure; G-5).
2. **Misattribution**: a helper-only check inherits the most recent anchor of its side and account; mixed
   `sudo`/`polkit` attempts within an hour can be labelled with the other class (the count stays right).
3. **Groups**: membership in `wheel`/`adm`/`systemd-journal` is required and is a broad read right on the
   system journal for the service (already true for the owner's shell today); without it the feature reports
   `unavailable`. A new group membership takes effect only in a new user-manager session (log out and in), never
   through a service restart (G-8).
4. **Journal retention**: the 24 h rebuild depends on persistent journal storage; with volatile storage alerts
   before the last boot are lost (the page still shows `active` for new attempts).
5. **Unverified sandbox run**: `journalctl` under the exact unit sandbox was not run by agents (O-5); RMC59 covers
   it on the owner's go.
6. **Oversized fields (F-9)**: an attempt whose journal line or user-name field exceeds the bounds of §2.3 is
   skipped; this can only concern accounts other than the owner's (whose login is ≤ 32 bytes).
7. **Memory hygiene limits (A-14, F-10)**: line buffers, entry strings and owned JSON strings are wiped; the kernel
   pipe buffer, `journalctl`'s own memory and `serde_json`'s internal scratch buffer for escaped strings are not
   under the crate's control. The journal itself already stores the same text on disk.
8. **Lock-screen coverage depends on configuration (F-2)**: a locker outside the configured list is not
   monitored; the page shows it (`not_configured`) only when **no** configured path exists on disk, so a host
   with an installed but unused default locker and a custom real locker still shows `monitored` — §2c makes the
   `lock_screen_programs` step mandatory for any locker outside `/usr/bin`.
9. **Catch-up heuristic (F-5)**: a `journalctl` stalled for more than `JOURNAL_IDLE_TICK_MS` inside the backlog can
   switch the view to `active` before the backlog is fully read; later backlog lines still count. The same holds
   for a cold start in which `journalctl` prints no line during the first 2 s after the spawn (cold page cache;
   G-4).
10. **Stale page within the same start**: a page that displayed a view and acknowledges later covers exactly the
   seqs it displayed (A, §4.3); newer attempts stay unacknowledged; an acknowledgement from an earlier start is
   refused (`409 stale_view`).

## 16. Documentation drift

- `Docs/REMOTE_COMPANION.md` §1 ("what it is") gains alerts; §5 two keys; §6 two routes and the `alerts` event;
  §8 the residual risks (all ten of §15); new §2c with setup (`password_alerts = true`; **mandatory**
  `lock_screen_programs = ["/home/<you>/.local/bin/swaylock-plugin"]` for any locker outside `/usr/bin`, otherwise
  the page reports "Lock screen not monitored"; group check `id -nG`), what is shown, what is never shown ("never
  the typed password"), the acknowledge semantics (epoch, `409 stale_view`).
- ADR 2026-10-05 item (5) said "push notifications … out of scope": unchanged; this ADR adds in-page alerts only.
- Walkthrough `AI/walkthroughs/186_remote_auth_alerts.md` (traceability phase).

## 17. Exit criteria

Every O-decision maps to a decision of §0.3 and a matrix row: O-1 → A-4–A-7, RMC47–RMC50, RMC57, RMC59; O-2 →
A-8, A-11, A-14, RMC48, RMC56; O-3 → A-1, A-5, RMC46–RMC47, §15; O-4 → A-2, A-3, A-9, A-10, A-13, §3.1, RMC45,
RMC51–RMC55; O-5 → unit unchanged, no host action, no existing test assertion changed (A-12), RMC58–RMC59. Every
new field has a stated bound (§3.1, §3.2, §4). The relayed request (§0.1) stays open for the owner's confirmation.

## 18. Round-2 revision log (plan-evaluator round 1 findings)

| Finding | Resolution | Sections |
|---|---|---|
| F-1 MAJOR — ack `?through=` dropped by the HTTP parser | Headers `X-Soos-Alerts-Epoch` + `X-Soos-Alerts-Through`, exactly one each, parsed by the pure `parse_alerts_ack_headers`; query never read; `RequestHead`, `parse_request_head` and their three test literals unchanged (no test migration) | A-10, A-12, §1.1, §3.1, §3.3, §6, §6.1, §9, tests 30, 39, 40, 50, §12.7, RMC54 |
| F-2 MAJOR — lock-screen alerts silently absent under defaults; ` (deleted)` suffix | (a) `lock_screen: monitored \| not_configured` in the view, `stat` of the configured paths at each follower start, page line "Lock screen not monitored — see setup"; (b) `exe_for_comparison` strips exactly one ` (deleted)` for every `_EXE` comparison, configured paths may not end in it; (c) test 25 uses an explicit configured locker (with a deleted-suffix line) plus a defaults case asserting `not_configured` and zero lock-screen attempts; (d) RMC59 and §2c make `lock_screen_programs` mandatory for this host | A-3, A-5, §2.4, §3.2, §3.3, §4.4, §5, §9, tests 7, 25, 51, 57, RMC47, RMC52, RMC59, §15.8, §16 |
| F-3 MAJOR — ack can cover unseen attempts | (a) per-start 64-bit epoch from the crate CSPRNG, in the view and the ack, mismatch `409 stale_view`; (b) rule R: the loaded marker applies only to attempts with journal time before the book's start, never to live-resolved ones; rule M: persisted marker = `min(ack high water, earliest known unacknowledged − 1)` including pending checks (`Correlator::oldest_pending_us`), so it may decrease (fail-safe); rule P: write on ack, otherwise at most once per second | A-9, A-13, §4.1, §4.3, §5, §6, tests 18, 19, 31, 52–55, RMC54 |
| F-4 MINOR — alerts to a revoked Funnel session for up to 15 s | Session re-validated (`Touch::Keep`) before every `alerts` event; stream ends without the event | A-10, §6, §7, test 24, RMC55 |
| F-5 MINOR — `active` during the replay | `starting` until the first entry newer than the spawn or `JOURNAL_IDLE_TICK_MS` without a line; batch pauses shorter than the tick (compile-time relation) | A-3, §3.1, §5 step 3, test 28, RMC52, §15.9 |
| F-6 MINOR — residual risk understated | Wording corrected: continuous, invisible suppression of all helper-only owner-side checks; first failure per handle and root-side attempts unaffected; innocent trigger by test binaries | §15.1, ADR accepted risks, `Docs/REMOTE_COMPANION.md` §8 |
| F-7 MINOR — test 33 needle collisions | Fixed epoch, test-chosen times, foreign UID `3141592653`, exact key sets of every object | test 33 |
| F-8 MINOR — test 43 scans all threads' children | `JournalLines::child_pid` seam; `/proc/<pid>` gone or reparented; runtime kept running | §2.5, §11, test 43 |
| F-9 MINOR — oversized fields invisible | Documented blind spot (non-owner accounts only) | §2.3, §15.6, ADR |
| F-10 MINOR — line buffers not wiped | `BoundedLineReader` with a fixed-capacity `Zeroizing` buffer, no `BufReader`; `Zeroizing` lines and entry strings; residual documented | A-14, §2.3, §2.5, §3.1, tests 42, 56, §15.7 |
| F-11 MINOR — duplicate keys | Hand-written `Visitor` with a seen-set for the nine read keys, `IgnoredAny` for others, bounded while visiting, `end()` required | §2.3, test 4, test 56 |
| Observation — `sudo` command-log line | Prefix anchoring stated; negative case in test 10 | §2.4, test 10 |
| Relayed request (see the tested passwords) | Unchanged answer: the safe subset per O-2; marked **open** for the owner's explicit confirmation before Phase 2 | §0.1 |

## Round 3 — clear acknowledged entries (owner request 2026-10-06)

### R3.0 Request and interpretation (binding)

Owner request, 2026-10-06, in the owner's own words: "once the acknowledge button is clicked, delete the entries".
Interpretation agreed in chat: acknowledged entries **disappear** from everything the page and the API show
(history list and any acknowledged-only data), immediately after a successful acknowledgement **and** after a
service restart (an attempt replayed at start-up whose journal time is at or before the persisted marker never
reappears); only attempts newer than the acknowledgement are shown.

**Refused and out of scope**: deleting entries of the system journal. `journald` cannot delete single entries;
the only removal tools (`journalctl --vacuum-*`, `--rotate`, deleting journal files) are root-only, act on whole
files and would destroy unrelated logs and the evidence of the very intrusion attempts the feature reports. The
journal stays the authoritative, untouched record; `soos-remote` only stops showing what was acknowledged.
Nothing else changes: no password is captured, stored, logged or displayed anywhere (O-2 unchanged), every bound
of §3.1 holds, the epoch and stale-view rules (A, A-13) are unchanged, Web Push is unchanged, authorization and
CSRF are unchanged, and no new log line exists.

### R3.1 Decision: drop at once, never filter

Acknowledged records are **removed from memory** in the same critical section that acknowledges them, and an
attempt that rule R would have recorded acknowledged on arrival is **discarded** without creating a record. A
view-only filter was rejected: it would keep acknowledged data in memory (and in a 32-slot history whose
acknowledged rows would still evict nothing useful), and any future view, SSE path or debug seam could leak it.
After this round the book holds **only unacknowledged records**; the `acknowledged` flag ceases to exist.

What the marker logic still needs is kept as scalars that already exist: `ack_high_water_us` (maximum `last_us`
over every record acknowledged in this start, initialised to the loaded marker) and the `loaded_marker_us` /
`started_us` pair of rule R. Rule M is unchanged in value: it already used only unacknowledged records,
`evicted.first_us` and the pending checks, plus `ack_high_water_us`. Dropping records therefore never makes a
persisted marker lower than in round 2, and it stays strictly below every unacknowledged attempt; the values are
identical in all migrated tests (every term of `marker()` is computed from data that is still present; the dropped
records only ever contributed through `ack_high_water_us`, which is updated before they are dropped). The marker
can only be higher than in round 2 when late journal lines make journal time and seq order disagree and an
acknowledged record that round 2 would have evicted without counting now raises the high water (plan-evaluator F-4).

### R3.2 Data-structure and API changes (`crates/remote/src/alerts.rs`)

```rust
/// One history record (several coalesced unacknowledged attempts).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AlertRecord {
    pub id: u64,
    pub first_unix_ms: u64,
    pub last_unix_ms: u64,
    pub source: SourceClass,
    pub account: AccountClass,
    pub kind: AttemptKind,
    pub count: u32,
    // `pub acknowledged: bool` is REMOVED (field and JSON key).
    #[serde(skip)] pub last_seq: u64,
    #[serde(skip)] pub first_us: u64,
    #[serde(skip)] pub last_us: u64,
}

/// What `AlertBook::record` did with an attempt (no `#[must_use]`: tests call `.unwrap()` and drop it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recorded {
    /// Stored (new record or coalesced into the newest one).
    Kept,
    /// Replayed from before the start and covered by the loaded marker (rule R): not stored.
    Discarded,
}

impl AlertBook {
    /// # Errors  `BookError::Overflow` when the seq counter cannot advance (nothing changes).
    pub fn record(&mut self, attempt: Attempt) -> Result<Recorded, BookError>;   // was Result<(), BookError>
    pub fn acknowledge(&mut self, epoch: AlertsEpoch, through: u64) -> Result<(), BookError>; // unchanged signature
    #[must_use] pub fn marker(&self, oldest_pending_us: Option<u64>) -> u64;     // unchanged
}
```

`Recorded` is re-exported from `alerts` like `BookError`. No other public type changes; `AlertsView` keeps its
exact top-level key set (`state`, `reason`, `epoch`, `lock_screen`, `unacknowledged_wrong_password`,
`unacknowledged_locked_out`, `last_unix_ms`, `last_source`, `through`, `history`); the record key set becomes
exactly `id`, `first_unix_ms`, `last_unix_ms`, `source`, `account`, `kind`, `count` (7 keys).

### R3.3 Rules (replacing R, A, coalescing and eviction of §4.3; M and P unchanged)

- **R′ (replay discard).** `record(attempt)`:
  1. `seq = next_seq`; `next_seq = seq.checked_add(1)` or `Err(Overflow)` with nothing changed (unchanged order:
     overflow is reported before anything else, also for an attempt that would be discarded).
  2. `highest_seq = seq` (the seq is **consumed** even when the attempt is discarded, so `through` keeps counting
     every attempt the book has seen in this start; an acknowledgement whose `through` covers only discarded seqs
     is a harmless no-op, and the round-2 `through` values of replay scenarios stay identical).
  3. If `attempt.at_us <= loaded_marker_us && attempt.at_us < started_us` → `Ok(Recorded::Discarded)`: no record
     is created or modified, no counter, total, `evicted` field or `ack_high_water_us` changes.
  4. Otherwise coalesce or append as below → `Ok(Recorded::Kept)`.
- **Coalescing**: joins the newest record when source, account and kind are equal and `|at_us − last_us| ≤
  ALERT_COALESCE_WINDOW_US` (the acknowledged-flag condition disappears: every stored record is unacknowledged).
  An attempt arriving after an acknowledgement therefore never joins a removed record: it starts a new record
  with `id = seq` (the removed record is gone).
- **A′ (acknowledge = remove).** `acknowledge(epoch, through)`: epoch mismatch → `StaleView` (nothing changes);
  `through == 0` → no-op `Ok`; `through > highest_seq` → `BeyondNewest` (nothing changes); otherwise, for every
  record with `last_seq <= through`: `ack_high_water_us = max(ack_high_water_us, record.last_us)`, then the record
  is removed (`VecDeque::retain(|r| r.last_seq > through)`, order of the survivors preserved); the evicted
  counters are reset exactly as in round 2 when `through >= evicted.max_seq` (`max_seq` kept). A record that grew
  after the snapshot (`last_seq > through`) stays as a whole (unchanged semantics).
- **Eviction**: the oldest record is popped when the length exceeds `MAX_ALERT_HISTORY`; its count is always
  added to `evicted` (saturating) with `max_seq`/`first_us` updated (the "skip if acknowledged" branch is deleted:
  no stored record is acknowledged).
- **View**: unchanged formulas minus the `!r.acknowledged` filters (every record counts): totals = sum of record
  counts per kind + evicted totals (saturating); `last_unix_ms`/`last_source` from the record with the greatest
  `last_us`; `through = highest_seq`; `history` = records newest first. After a full acknowledgement the view is
  `history: []`, both totals 0, `last_* = null`, `through` unchanged.
- **M, P**: unchanged text and values (§4.3). Memory: at most `MAX_ALERT_HISTORY` records as before; in practice
  fewer, since acknowledged records no longer occupy slots.

### R3.4 Runtime, routes, SSE and push (`alerts.rs` runtime, `server.rs`, `push.rs`)

- `AlertsRuntime::acknowledge` is unchanged in structure: book update under the `inner` mutex, then `flush(true)`
  (marker written at once), then `bump()`, then the response view. The `200` body of `POST /api/alerts/ack` is
  therefore the view **without** the removed records; `GET /api/alerts` and the next `event: alerts` (version
  bump, throttle of one per second per stream unchanged) show the same. No new route, header, status code or
  response key.
- `AlertsRuntime::record` hands an attempt to the live sink only when `book.record` returned
  `Ok(Recorded::Kept)` **and** `is_live(...)` holds. (A discarded attempt has `at_us < started_us` while
  `is_live` requires `at_us >= started_us`, so the two are already disjoint; the explicit `Kept` check makes this
  independent of that arithmetic.) `changed`/`bump()` is set only for `Kept`.
- **Push is unaffected**: the push scheduler accumulates live attempts through the sink at record time and never
  reads the book; acknowledging neither cancels, reduces nor sends a notification, a pending 3 s summary or a
  carried undelivered count (the owner acknowledges on the page; the phone notification is a separate, already
  delivered or pending message). No push count changes because of R′ either (discarded attempts are replayed, and
  replayed attempts were never pushed).
- **Restart / marker semantics** (stated for the docs and the walkthrough): after a restart, every replayed
  attempt with journal time `<=` the persisted marker is discarded (never shown); every other attempt is shown.
  The persisted marker is `min(ack high water, earliest known unacknowledged − 1)` (rule M), so in the normal case
  (acknowledge with nothing older still pending) it equals the newest acknowledged attempt and the page after a
  restart shows only newer attempts. **Fail-safe residual** (unchanged from round 2, now stated explicitly): when,
  at the last marker write, an attempt older than an acknowledged one was still unacknowledged (or a lock-screen
  check still pending), the marker stays below that older attempt, and acknowledged attempts above the marker are
  shown again after a restart — never fewer alerts than the truth, and the banner is non-empty in that case
  anyway. Two further residuals err the same way (plan-evaluator F-2): when `remote-alerts.json` cannot be written
  (`password alert acknowledgement not persisted`), the entries are removed from the page at once but reappear
  after a restart; when more than `MAX_ALERT_HISTORY` records arrive between the displayed view and the
  acknowledgement, the evicted attempts stay counted in the totals (counts only) until the next acknowledgement. A scalar marker is kept on purpose (≤ 256 bytes, bounded); persisting per-attempt identities is
  rejected (unbounded, and a journal cursor list would add raw journal data to the file).
- Logging: none added; `alerts.rs` keeps no `tracing` macro; no audit event changes.

### R3.5 Page (`crates/remote/assets/`)

- `app.js` `renderAlerts`: the `if (record.acknowledged === true) { item.className = "acknowledged"; }` branch is
  deleted; the page no longer reads any `acknowledged` field. After a `200` acknowledgement the page renders the
  returned view (history now only newer attempts, usually empty) and shows the feedback "Acknowledged"; the
  Acknowledge button stays hidden while both totals are 0 (unchanged rule). An empty history renders an empty
  list (no placeholder text is needed: the summary line already reads "No failed password attempts" in `active`).
- `style.css`: the `.alerts-history li.acknowledged` rule is deleted.
- `index.html`: unchanged. The CSP and every pinned asset header are unchanged.

### R3.6 Documentation

- `Docs/REMOTE_COMPANION.md` §2c: *Acknowledge* removes the acknowledged entries from the page and the API at
  once and after a restart (replayed attempts at or before the stored marker are not shown again; the fail-safe
  residual of R3.4 in one sentence); the system journal is never modified: "journal entries are never deleted"
  (root-only, whole-file vacuum would destroy unrelated logs and intrusion evidence; use `journalctl` as root if
  the owner wants to look at the raw history).
- §4.3/§4.4/§9 of this spec are superseded by R3.2–R3.5 where they mention `acknowledged` records or dimmed rows.
- ADR amendment (AI/DECISIONS.md, alerts ADR, dated item 2026-10-06) — added in this phase.
- Traceability phase: matrix row **RMC75** (new, below), RMC54 evidence updated, RMC59 updated with the owner's
  hardware results of 2026-10-06: a wrong `sudo` password produced a notification on the iPhone; *Acknowledge*
  resets the banner and it stays reset after a service restart; GDM login not tested (owner: not important); the
  test notification and the lock-screen notification already recorded in RMC59/RMC74; the page lists the phone
  under devices (host check: `GET /api/push` shows one `apple` device). RMC74 gains the `sudo` notification
  result. Walkthrough: next free number in `AI/walkthroughs/`.

New matrix row:

| ID | Criterion | Evidence |
|---|---|---|
| RMC75 | A successful acknowledgement removes the acknowledged records from memory, from the `200` view, from `GET /api/alerts` and from the next `event: alerts`; replayed attempts at or before the persisted marker are discarded at start-up and never shown; records carry no `acknowledged` key; push notifications and counts are unaffected; journal entries are never deleted | tests 58–62 below + migrated tests 18, 19, 21, 31, 54 and the `alerts-ack` CSRF test |

### R3.7 Tests (Phase 2, numbering continues the alerts series after 57)

New tests:

| # | File::name | Asserts |
|---|---|---|
| 58 | `crates/remote/tests/alerts_tests.rs::test_rmc_alerts_book_acknowledge_removes_records` | `acknowledge(epoch, through)` removes exactly the records with `last_seq <= through` from `view().history` (survivor ids and order checked), a record grown past `through` stays whole; after acknowledging everything: `history == []`, both totals 0, `last_unix_ms`/`last_source` `None`, `through` unchanged; a later attempt within 60 s of a removed record starts a new record with `id = its seq` and count 1; `through == 0` no-op, `StaleView` and `BeyondNewest` change nothing; `marker()` values identical to the round-2 rule-M expectations of test 18; bounded: 1 000 attempts then acknowledge-all leaves `history` empty, then 40 more attempts leave exactly `MAX_ALERT_HISTORY` records; evicted totals still reset only when `through >= evicted max seq` |
| 59 | `alerts_tests.rs::test_rmc_alerts_book_replay_covered_attempts_are_discarded` | `AlertBook::new(marker, epoch, started)`: an attempt with `at_us <= marker && at_us < started` returns `Ok(Recorded::Discarded)`, is absent from `history` and from the totals, but advances `through`; `at_us == started` (or later) with `marker == u64::MAX` returns `Ok(Recorded::Kept)`; `at_us == marker + 1` before the start is `Kept`; an `Overflow` at `next_seq == u64::MAX` is returned even for a covered attempt; a discarded attempt between two coalescible kept attempts does not break the coalescing; `marker(None)` stays the loaded marker after discards |
| 60 | `crates/remote/tests/alerts_server_tests.rs::test_rmc_alerts_ack_clears_history_end_to_end` | harness + `ScriptedJournal`, SSE open: two attempts (sudo, polkit), ack with the displayed epoch/through → `200` view with `history == []`, totals 0; `GET /api/alerts` identical; the next `event: alerts` data has `history == []`; a new attempt after the ack shows a history of exactly that one record; no record of any view has an `acknowledged` key; restart with the same journal lines plus one newer line → history shows only the newer attempt (the two acknowledged ones never reappear), file marker unchanged |
| 61 | `crates/remote/tests/push_server_tests.rs::test_rwp_acknowledge_never_changes_push` | push enabled, one subscription, `FakeTransport`: a live attempt, then an acknowledgement before the 3 s summary is due → exactly one notification with `wrong = 1` is still delivered; a second live attempt after the ack → its own notification counts 1 (no carry from the acknowledged one, no loss); acknowledging with nothing pending sends nothing; replayed attempts (covered or not) are never pushed |
| 62 | `tests/invariants/src/remote_alerts_contract.rs::test_rmc_s43_acknowledged_entries_are_not_kept` | `crates/remote/src/alerts.rs` does not contain `pub acknowledged:`; `assets/app.js` contains neither `.acknowledged` nor `"acknowledged"`; `assets/style.css` contains no `.acknowledged`; `Docs/REMOTE_COMPANION.md` §2c contains `journal entries are never deleted`; `AI/DECISIONS.md` contains `clear acknowledged entries` |

Contract migrations (each recorded in `AI/tester_contract_alerts.md`, section "Contract migration — owner request
2026-10-06 (clear acknowledged entries)", citing the owner's words; no assertion unrelated to the `acknowledged`
flag changes, and every marker/count/through assertion keeps its value):

| Existing test | Old assertion | New assertion |
|---|---|---|
| `alerts_tests::test_rmc_alerts_book_acknowledge_through` (18) | after `through = snapshot` all rows `!acknowledged`; after `through = 2` `history[1].acknowledged`, `!history[0].acknowledged`; after `through = 3` all rows acknowledged; after the live attempt `history.len() == 3`, `!history[0].acknowledged`; final "2 acknowledged rows stay in the history" | after `through = snapshot`: `history.len() == 2` (both still present); after `through = 2`: `history.len() == 1`, `history[0].id == 3`; after `through = 3`: `history.is_empty()`; after the live attempt: `history.len() == 1`, `history[0].id == 4`; final: the history holds only the unacknowledged records (ids 5 then 4, newest first by insertion), no acknowledged row exists. All counts, `last_*` and `marker()` assertions unchanged |
| `alerts_tests::test_rmc_alerts_book_rebuild_respects_marker` (19) | `(source, acknowledged)` list `[(Sudo,true),(Login,true),(Other,false),(LockScreen,false),(Sudo,false)]` | oldest-first source list `[Other, LockScreen, Sudo]` (the two covered replays are absent); `through == 5`; totals and every `marker()` assertion unchanged; marker-0 case: `history.len() == 1` |
| `alerts_tests::test_rmc_alerts_view_json_shape` (21) | record key set includes `acknowledged`; `history[0]` literal has `"acknowledged": false`; `AlertRecord { …, acknowledged: false, … }` literal | record key set without `acknowledged` (7 keys); literal without it; struct literal without the field |
| `alerts_server_tests` helpers `counts`/`key`, `RECORD_KEYS` | `counts` keyed by `(source, kind, acknowledged)` reading `r["acknowledged"]`; `RECORD_KEYS` has 8 entries | `counts` keyed by `(source, kind)` and asserts that no record has an `acknowledged` key; `key(source, kind)`; `RECORD_KEYS` has the 7 keys; every `key(…, false)` call site becomes `key(…)` with the same expected count |
| `alerts_server_tests::test_rmc_alerts_ack_csrf_headers_and_rate` | `counts[(sudo, wrong_password, true)] == 1` after `through = 1` | the sudo record is absent from `history` and `counts[(other, wrong_password)] == 1`; `wrong_count == 1` unchanged |
| `alerts_server_tests::test_rmc_alerts_ack_persists_and_survives_restart` (31) | runs 2 and 3 expect `{(sudo,wp,true):1, (other,wp,true):1, (login,wp,false):1}` | runs 2 and 3 expect `{(login, wrong_password): 1}` only; `wrong_count == 1`, file marker `t2` and epochs unchanged |
| `alerts_server_tests::test_rmc_alerts_pending_check_older_than_ack_stays_unacknowledged` (54) | `newest["acknowledged"] == false`; after restart `(lock_screen,wp,true) == 1` and `(lock_screen,wp,false) == 1`; filter on `acknowledged == false` | `newest` has no `acknowledged` key; after restart exactly one `lock_screen` record, `first_unix_ms == t / 1000` (the acknowledged first failure at `t − 20 s` is below the marker and absent); `through_of(v) == 3` unchanged (discarded replays consume a seq) |

Not migrated (verified unaffected): tests 16, 17, 20, 52 (no `acknowledged` read; values unchanged under R′/A′),
the audit test ("acknowledged in memory" reads only the total), the SSE, raw-field and push suites.

Setup-only changes: none (`AlertRecord` literals exist only in test 21).

### R3.8 Auditor focus

No panic path added (`retain` and the enum are total); memory strictly bounded (≤ `MAX_ALERT_HISTORY` records,
fewer than before); no new I/O, file, field or log line; the ack file format and write path unchanged; push lock
order unchanged (the sink is still called after the `inner` mutex is released); O-2 unchanged.
