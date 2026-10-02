# Plan Evaluation Report

- **Date**: 2026-10-02
- **Issue**: GitHub #323 — feat: continuous face presence auto-unlock of locked local sessions (GitHub-only, no backlog id; owner decisions 2026-10-02)
- **Branch**: `feat/presence-auto-unlock`
- **Base commit**: `f76a80b`
- **Plan evaluated**: `AI/architect_spec_presence_unlock.md` — revision 1 (round 1: `REVISION_REQUIRED`), revision 2 (round 2: `APPROVED`), revision 3 with the owner answers Q1–Q3 of 2026-10-02 (round 3: `REVISION_REQUIRED`, round 4: `APPROVED`), revision 4 with the Phase 3 auditor items A1–A6 (round 5, delta only: verdict below)

## 1. Coverage Matrix

| Issue / owner line | Spec element | Status |
|---|---|---|
| Scan the face while a local seat session of an enrolled user is locked | §2.6 tick steps 4–12, D1, D5 | Covered (PAU5, PAU7, PAU8) |
| Unlock through `org.freedesktop.login1` (`loginctl unlock-session` equivalent) | D2, `PresenceLogind::unlock_session` → `Manager.UnlockSession` | Covered (PAU11, PAU21) |
| Generic for systemd distros; GDM/GNOME primary; GNOME/KDE/Cinnamon/MATE; swayidle `unlock` hook documented | §0, ADR (2), §9.1 deployment doc | Covered (PAU20, PAU21) |
| Existing pipeline unchanged, same thresholds | §2.4 shared `run_face_consensus`, C4 | Covered (PAU10) |
| Enabled by default; `daemon.toml` key; kill switch | §2.6 config (`[presence] enabled = true`), D9 flags | Covered (PAU1, PAU4, PAU19) |
| Global per-UID rate limit raised to 40/60 s, applies to sudo | §2.5 `DEFAULT_MAX_ATTEMPTS = 40`, D10 | Covered (PAU2, PAU3) |
| Binding: local, active, `REMOTE=0`, seat, `CLASS=user`, `LockedHint=yes` | D4 reuses `check_local_seat_session_of` | Covered (PAU5, PAU18) |
| Grace ~3 s after lock | `lock_grace_ms` 3000, `LockTracker::due` | Covered (PAU6) |
| Screen on / lid open when detectable; sandbox readability evaluated | D8, §0 sandbox row, `SysfsDisplayProbe`, `LidClosed` | Covered (PAU9; hardware PAU21) |
| Fail closed (any error / unknown / logind failure ⇒ no unlock) | §2.6 tick algorithm, §4 taxonomy, C3 single call site | Covered (PAU11, PAU14) |
| Never log frames/embeddings; bounded everything | §2.6 constants, logging rules | Covered (PAU14, PAU16) |
| Camera idle/standby correct when no session locked | §6 camera-lifecycle bullet | Covered (PAU13) |
| Design Q1 lock-state source with evidence | §0 rows 1–2 | Answered (D-Bus only; files lack the key) |
| Design Q2 unlock mechanism + `deny.toml` check | §0 rows 3–5, D2/D3 | Answered (`zbus` 5, deny clean) |
| Design Q3 loop placement, gate sharing, PAM priority, rate-limit recording | §2.2, §2.3, D6, D10 | Answered |
| Design Q4 several locked sessions, UID mapping, remote refusal | D4, D5 | Answered |
| Design Q5 config keys/defaults/validation/docs, ADR with accepted risks | §3, §9 | Answered |
| Owner Q1: respect pam_faillock and account expiry; locked / expired / undeterminable ⇒ no unlock | §2.6 `presence/account.rs` (`AccountGuard`, `SystemAccountGuard`), tick steps 7 and 13 (fresh re-check, nothing cached) | Covered (PAU22–PAU29, PAU21 hardware steps) |
| Q1: tally location/format, `dir=`, `struct tally`, deny / unlock_time / fail_interval / even_deny_root / root_unlock_time | §0 faillock row (Linux-PAM 1.7.1 sources), account.rs steps 3–5 | Covered (PAU22–PAU25) |
| Q1: `/etc/shadow` expiry and password expiry, decision justified | account.rs step 6 (a)–(d) mapped to `pam_unix` account refusals | Covered (PAU26) |
| Q1: sandbox readability, capabilities | §0 sandbox-for-guard row: no unit change, `CAP_DAC_OVERRIDE` load-bearing | Covered (PAU29, C9) |
| Q1: bounded reads, strict parsing, mockable trait | account.rs constants, `AccountGuard` trait, tempdir-injectable paths | Covered (PAU22–PAU28) |
| Q1: tally reset decision | account.rs "No tally reset" | Covered (PAU29, C8) |
| Q1: ADR risk no longer accepted; new guard recorded | ADR (10), risk (d) narrowed, invariant 3 clause, invariant 6 text | Covered (PAU20, C7) |
| Owner Q2 (no presence evidence), Q3 (`gdm.disable` scope) | D7, D9, ADR (9), (11), §10 | Covered |
| Design Q6 mock strategy | §8 hooks (`MockPresenceLogind`, `StaticDisplayProbe`, test clock) | Answered |

No unmapped line.

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Session files carry no lock state | `/run/systemd/sessions/*` on host; `strings` of `systemd-logind` / `libsystemd-shared-262-1.so` | no `LOCKED_HINT` key; only D-Bus `property_get_locked_hint` | ✅ |
| zbus reads `LockedHint`, `LidClosed`, `ListSessions` | scratch probe (`zbus` 5.19.0) | values printed for session `4` | ✅ |
| `cargo deny` with zbus | scratch workspace copy | `advisories ok, bans ok, licenses ok, sources ok`; `cargo check -p soos-daemon` green | ✅ |
| `RateLimitConfig::DEFAULT_MAX_ATTEMPTS` | `crates/policy/src/rate_limit.rs:19` | 5 (no test asserts it) | ✅ |
| `DECISION_BUDGET_MS` | `crates/daemon/src/pipeline.rs` | 900 | ✅ |
| `RESPONSE_WRITE_MARGIN_MS`, `MAX_INFERENCE_ESTIMATE_MS`, `MAX_CONCURRENT_INFERENCES` | `crates/daemon/src/inference.rs` | 50, 1000, 1 | ✅ |
| `InferenceGate` owned by value, FIFO semaphore, no priority | `inference.rs:247`, `dispatcher.rs:186` | as stated | ✅ |
| Consensus loop inline in dispatcher Step 8e; wake literals 1200 / 1000 / 15 ms | `dispatcher.rs` ~896–1195 | as stated | ✅ |
| `MAX_SCANNED_SESSIONS`, `MAX_SESSION_ID_LEN` | `session_policy.rs:46,49` | 1024, 64 | ✅ |
| Binding predicate | `SessionRecord::check_local_seat_session_of` | UID, active, `REMOTE==Some(false)`, seat, `CLASS=user` | ✅ |
| `DEFAULT_FLAG_DIR` | `crates/pam/src/config.rs:22` | `/etc/soos` | ✅ |
| Default `connection_timeout_ms` | `config.rs:32` | 2500 | ✅ |
| Camera `idle_timeout` default | `crates/camera-v4l/src/config.rs:58` | 10 s | ✅ |
| Unit directives (AF_UNIX, PrivateNetwork, ProtectSystem=strict, @system-service, MDWE, no CAP_SYS_ADMIN) | `packaging/soos-daemon.service` | as stated | ✅ |
| `capture_spoof_evidence` must stay in dispatcher; `EvidenceStore (` line names both reasons | `daemon_docs_contract.rs:285–305` | enforced | ✅ (F5) |
| Docs key contract uses a fixed table list | `daemon_docs_contract.rs:37` `CONFIG_FILE_TABLES: [(&str,&str); 9]`, panics on unmapped struct | as found | ❌ in rev 1 → F1 |
| `struct tally` layout and `TALLY_STATUS_VALID` | Linux-PAM 1.7.1 `faillock.h` (downloaded) | `char source[52]; uint16 reserved; uint16 status; uint64 time` = 64 B, VALID `0x1` | ✅ |
| Lock rule, defaults, bounds | `pam_faillock.c` `check_tally`, `get_pam_user`; `faillock_config.c` `set_conf_opt` | as in the spec; `MAX_TIME_INTERVAL` 604800; `never` = 0; `root_unlock_time` defaults to `unlock_time`; `is_admin` never locked without `even_deny_root` | ✅ |
| `EACCES` on the tally is "not locked" in PAM | `check_tally` (`errno == EACCES \|\| ENOENT` ⇒ success) | spec deliberately stricter (`EACCES` ⇒ undeterminable) | ✅ (intended divergence) |
| Record limit / reader | `faillock.c` `MAX_RECORDS 1024`, `CHUNK_SIZE 64 records`, whole records only, `flock(LOCK_EX)` on open | spec bound 1088 records, shared non-blocking flock | ✅ |
| Vendor conf fallback is build-dependent | `faillock_config.c` `#ifdef VENDOR_FAILLOCK_DEFAULT_CONF` | rev 3 initially treated it as unconditional | ❌ → F9 |
| Host files | `/run/faillock` `0755 root:root`, tally `0660 willi363:root`, `/etc/shadow` `0600 root:root`, `faillock.conf` comments only, `system-auth` faillock lines without policy args | as cited | ✅ |
| `nix` `Flock` available to the daemon | workspace `nix` features `socket, fs, user`; `soos-biometric-store` already uses `Flock` | as cited | ✅ |
| Systemd acceptance container has no D-Bus | `tests/docker/Dockerfile.systemd` (no dbus package, `systemd-logind.service` masked) | as found | ⚠ rev 1 silent → F7 |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: user A's session is locked on seat0; B fast-user-switches. If presence used "any locked session of an enrolled user", B's face (enrolled) could unlock B's own background session — harmless — but A's inactive locked session must never be scanned against A's template while B sits there. The binding requires `active`, so A is dropped when inactive and re-enters with a fresh grace when re-activated (PAU5/PAU6). Remote/greeter sessions are refused by the reused predicate.
- Failure scenario considered: a process of the owner sets `LockedHint` to light the camera. Result: scans, but unlocking grants nothing beyond `loginctl unlock-session` the owner can already run (logind `good_user`); recorded as accepted risk (f).
- Failure scenario considered (rev 1, **F2**): unlock + re-lock during a scan. Rev 1 claimed the epoch re-check proved "same lock period"; it cannot (no tick runs during a scan). Rev 2 drops the claim and widens accepted risk (e).
- Result: PASS after F2.

### Pillar 2 — PAM deadline & concurrency
- Failure scenario considered: a GDM `Auth` arrives while a presence inference runs. Result: the Tokio FIFO semaphore would make PAM wait; the plan bounds it to one in-flight job (presence never queues: `try_acquire_background`, demand guard, preemption between captures), absorbed by the existing `acquire_within(remaining − estimate)` admission. No change to `crates/pam`; no Tokio in PAM.
- Failure scenario considered: the guard is not dropped on an early return (leaks demand ⇒ presence starved forever). Result: RAII guard at the top of Step 8 covers every return path (PAU12 asserts it).
- Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered: `lid_closed` errors ⇒ treated as "scan allowed". Result: gating only (owner "when detectable"); the unlock still needs a full `Allow` and an error-free re-check. A worker panic ends the task, logged, no unlock. A single `unlock_session` call site guarded by C3.
- Failure scenario considered: logind returns `Remote` as a non-boolean. Result: `remote = None` ⇒ binding refuses (PAU18 power check).
- Result: PASS.

### Pillar 4 — Dependencies
- Failure scenario considered: `zbus` reads `DBUS_SYSTEM_BUS_ADDRESS` from the environment and connects to a rogue bus. Result: rev 1 forbade `Connection::system` but not `Builder::system()` (**F4**); rev 2 forbids both and pins `Builder::address`.
- Failure scenario considered: rev 1 §2.2 described the unsafe policy in a garbled, self-contradictory sentence (**F3**). Rev 2: inner `#![forbid(unsafe_code)]` on `presence/mod.rs` and `consensus.rs` + C2.
- `cargo deny` verified clean in a scratch copy; licenses MIT/Apache-2.0; no duplicate versions; no `opencv`/`nokhwa`.
- Result: PASS after F3/F4.

### Pillar 5 — Data confidentiality
- Failure scenario considered: continuous scans flood the journal or evidence store with biometric data. Result: no evidence from presence (D7), transition-only logging, no score above `debug`, template `Zeroizing` dropped after each scan; nothing crosses the IPC socket.
- Result: PASS.

### Pillar 6 — Test integrity
- Failure scenario considered (rev 1, **F1**): adding `PresenceConfigFile` makes the existing invariant `test_daemon_doc_documents_every_daemon_toml_key` panic ("map it in CONFIG_FILE_TABLES"), so the developer would have to edit an existing test the plan never listed. Rev 2 lists it as the only contract migration (array 9 → 10 entries, no assertion changed, test becomes stricter).
- Test power: the plan mandates boundary tests (`>=` at grace), `REMOTE` unknown refused, `0` rejected, `unlock_calls == 0` on every failure path, and a raised `match_threshold` making the same mock score fail (catches a presence path that ignores `[pipeline.thresholds]`).
- Result: PASS after F1.

### Owner-Q1 account guard (rounds 3–4)
- Failure scenario considered (round 3, **F8**): the stack line `auth [success=1 default=bad] pam_faillock.so authfail deny=10` — rev 3's "module is the 3rd field after a `[...]` control" misses it because the control contains a space, so the guard would evaluate `faillock.conf` (deny 3) correctly here but the reverse case (`deny=1` on the line, conf default 3) would let presence unlock an account PAM considers locked; a `\`-continued argument line is missed the same way. Fixed in rev 3: any token ending in `pam_faillock.so`, continuation lines joined (PAU25 power cases).
- Failure scenario considered (round 3, **F9**): no `/etc/security/faillock.conf`, a vendor file with `deny = 10`, Linux-PAM built without `--enable-vendordir` (PAM uses deny 3). Rev 3 would read the vendor file and unlock after 3–9 failures. Fixed: both readings evaluated, strictest wins.
- Failure scenario considered: `admin_group = wheel` and the user is in `wheel`. PAM never locks such an admin without `even_deny_root`; the guard (no group lookup) evaluates as non-admin with the stricter unlock time, so it can only refuse more. PASS.
- Failure scenario considered: `pam_faillock` truncates the tally while the guard reads it (zero failures visible). Excluded by the shared non-blocking `flock` (`EWOULDBLOCK` ⇒ undeterminable). PASS.
- Failure scenario considered: an LDAP/SSSD or systemd-homed user. No shadow line ⇒ undeterminable ⇒ no presence unlock (owner rule "cannot be determined"); documented consequence. PASS.
- Failure scenario considered: a crafted logind `Name` such as `../../etc/x`. Logind names come from the user record, and `UserName::parse` rejects `/`, `.`/`..` and leading `.`/`-` before any path is built (PAU27). PASS.
- Failure scenario considered: the guard result of step 7 reused at unlock time after three wrong passwords during the scan. Step 13 re-runs the guard with no cache (PAU28 scripted Usable→Faillocked). PASS.
- Failure scenario considered: the re-check takes 500 + 500 ms and exceeds `MAX_ALLOW_TO_UNLOCK_MS` (**F11**, MINOR): ends `AllowExpired`, fail closed; documented, not widened.
- Shadow rule wording in rev 3 was garbled (**F10**, MINOR): restated as ordered rules (a)–(d) with their `pam_unix` meaning.
- No unit change; `CAP_DAC_OVERRIDE` becomes load-bearing and is pinned by C9. No write path (C8). PASS.

## 4. Findings

### Round 1 (revision 1) — `REVISION_REQUIRED`
- **[MAJOR] F1** Existing invariant `daemon_docs_contract::CONFIG_FILE_TABLES` (fixed 9-entry array) must change to accept `PresenceConfigFile`; the plan did not list this existing-test change — required: list it as a justified contract migration. **Resolved in rev 2** (§1.1 row, §2.4 sentence).
- **[MAJOR] F2** The pre-unlock "same lock period / epoch" re-check is vacuous (tracker not updated during a scan) and invariant 6 / the ADR overstated the guarantee — required: drop the claim, state the residual. **Resolved in rev 2** (§2.6 step 13, §6 invariant 6 wording, ADR (5) and risk (e), PAU11).
- **[MAJOR] F3** Unsafe policy for the new modules was ambiguous/garbled — required: explicit `#![forbid(unsafe_code)]` inner attributes and an invariant. **Resolved in rev 2** (§2.2, C2).
- **[MINOR] F4** C2 missed `zbus::connection::Builder::system()` / `session()` (environment-derived address). **Resolved** (C2).
- **[MINOR] F5** The ARCHITECTURE diagram edit must preserve the `EvidenceStore (` line checked by an existing invariant. **Resolved** (§9.1).
- **[MINOR] F6** A driver misreporting DPMS would silently disable presence with only `debug` logs. **Resolved** (`info` on gate transitions).
- **[MINOR] F7** The systemd acceptance container has no D-Bus; the plan did not say what presence does there. **Resolved** (PAU19).
- Process note: revision 1's header already announced "Revision 2"; corrected in revision 2.

### Round 2 (revision 2) — observations, non-blocking
- **[MINOR] O1** The worker polls logind every second on every install, even with nobody enrolled (one `ListSessions` + one `GetAll` per seat session). Negligible cost; a later optimisation could skip ticks while the store holds no template.
- **[MINOR] O2** DPMS semantics per driver, `UnlockSession` from the sandboxed unit on systemd < 255, and KDE/Cinnamon/MATE behaviour are only provable on hardware (PAU21 stays `⬜ Pending` until run).
- **[MINOR] O3** Owner questions Q1 (account-phase bypass), Q2 (presence evidence), Q3 (`gdm.disable` scope) have safe defaults in the spec and do not block Phase 2.

### Round 3 (revision 3, owner answers) — `REVISION_REQUIRED`
- **[MAJOR] F8** PAM-line option scan relied on field positions and missed bracketed controls with spaces and `\` continuations. **Resolved** (token-based scan, joined continuations; PAU25).
- **[MAJOR] F9** Vendor `faillock.conf` fallback treated as unconditional although it is a Linux-PAM build option; could be laxer than the PAM in use. **Resolved** (defaults and vendor both evaluated, strictest wins; PAU22).
- **[MINOR] F10** Shadow expiry rule wording garbled. **Resolved** (ordered rules (a)–(d)).
- **[MINOR] F11** Re-check latency vs `MAX_ALLOW_TO_UNLOCK_MS` not discussed. **Resolved** (documented fail-closed `AllowExpired`).

### Round 4 (revision 3 after fixes) — observations, non-blocking
- **[MINOR] O4** The guard deliberately diverges from PAM in the strict direction (unknown `faillock.conf` key, `EACCES`, partial tally record, NSS-only users, `expire = 0`, possible admins evaluated as non-admin): some hosts will see presence refuse where PAM would allow. Intended by the owner rule; documented.
- **[MINOR] O5** Account modules other than `pam_faillock` and shadow expiry (`pam_nologin`, `pam_access`, `pam_time`, `pam_tally2`) are not honoured; recorded as residual risk (d).

### Round 5 (revision 4, delta only: auditor items A1–A6) — `APPROVED`
Scope: §0 sandbox/expiry rows, §2.6 account steps 3 and 5, tick step 13, `ScanOutcome`, PAU23/PAU25/PAU28, §9.1, ADR (10) and risks (f)–(h). Nothing else changed.
- **A5 (PAM stack symlinks)**. Failure scenario considered: Fedora authselect, where `/etc/pam.d/system-auth` → `/etc/authselect/system-auth` carries `pam_faillock.so ... deny=1`. Rev 3's "regular file" wording allowed a `symlink_metadata().is_file()` filter that skips the file and evaluates `faillock.conf` (deny 3), unlocking an account PAM has locked (fail open). Rev 4 follows symlinks like libpam, decides with `fstat` on the opened descriptor, skips directories and treats dangling, looping or non-regular targets as undeterminable. It explicitly forbids the skip filter, and PAU25 now has the authselect, dangling and FIFO cases. A symlink target outside the PAM directory is safe to read: only root can create entries in the PAM directories. The tally keeps `O_NOFOLLOW`, and its directory must be neither a symlink nor group/other-writable (PAU23). PASS.
- **A6 (kill switch at re-check)**. Failure scenario considered: `touch /etc/soos/presence.disable` during a ~3 s scan. Rev 3 still unlocked. Rev 4 re-checks the switch immediately before `unlock_session` and returns `KillSwitchEngaged` (PAU28). This change only adds refusals. PASS.
- **A1 (`/etc/shadow` `0000` on Fedora/RHEL)**. Fact corrected: root reads it only through `CAP_DAC_OVERRIDE`, which the unit already grants. The behaviour was already fail closed. Docs, the ADR and the unit comment (text only, no directive) now say so; C9 still pins the capability. PASS.
- **A2, A3, A4**. Documented as ADR risks (g), (h) and an extended (f), plus `Docs/DAEMON.md` items. None of them adds an unlock path; A3 is an availability-only, fail-closed effect. PASS.
- Test integrity: the spec delta edits no existing test. The new cases belong to the tester (auditor T1–T5). PASS.

## 5. Verdict

VALIDATION_VERDICT: APPROVED
