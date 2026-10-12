# Plan Evaluation Report

- **Date**: 2026-10-12
- **Issue**: GitHub-only security review of 2026-10-12 (S-1 … S-8d), no backlog id — soos-remote Security Hardening
- **Spec**: `AI/architect_spec_remote_security_hardening.md` (round 2, commit `d91582b`)
- **Branch**: `fix/remote-security-hardening`
- **Base commit**: `f9234ea` (`origin/main`)
- **Round**: 2 (round 1 `f5058b6`: REVISION_REQUIRED, PE-SR1 MAJOR, PE-SR2 to PE-SR7 MINOR/observations)

## 1. Coverage Matrix

| Finding / acceptance line | Spec element | Status |
|---|---|---|
| S-1 sender reads the system journal; `InaccessiblePaths=`; correct docs; CM-SR1 on test 45 | §1.1 (host reproduction, journal 1 → 0 lines, DNS ok), §2.1–§2.4, §11.3 CM-SR1, RSH1/RSH2/RSH18-1 | Covered |
| S-2 raise documented Tailscale minimum; installer warns/refuses; absent tailscale never breaks install; agents never run tailscale | §3: `TAILSCALE_MIN_VERIFIED_VERSION = "1.102.4"`, docs §2b/§2/§4/§8, installer prints (two `echo` lines) | Covered. "Refuse"/"parse `tailscale version`" credibly refuted: RMC-S6 (`tests/invariants/src/remote_companion_contract.rs:735-790`) rejects any non-`echo`/`printf` line containing the word `tailscale`; an exemption would relax it |
| S-3 face auth not starved by preview connections, per-UID DoS bound kept | §4: one auth-reserved permit per unprivileged UID (cap + 1), whole first frame within 200 ms, `Auth` only, companion graceful close on retry | Covered (round-1 PE-SR2/PE-SR3 resolved) |
| S-4 Android lock-screen privacy + `push_previews = "generic"` | §5 | Covered |
| S-5 preview only for processes inside a local seat session; soos-gui and soos-remote keep working | §6: `PeerPlacement`, `classify_peer_placement`, `authorize_preview_placement` (step 6c-2a, per request) | Covered; see PE-SR11 (docs) |
| S-6 CORP, COOP, Permissions-Policy, HSTS; CSP byte-identical | §7 `HARDENING_HEADERS` appended in the three head encoders | Covered |
| S-7 sender fails closed (78) unless the sandbox is detected; robust, testable probes | §8: `home_from_exe` (binary path, no environment), `evaluate_sandbox` (pure), `probe_sandbox`, call before `XDG_RUNTIME_DIR`/bind | Covered (round-1 PE-SR1 resolved); see PE-SR9 |
| S-8a dangling symlink followed by the installer | §9.1 `-L` refusal + `(set -C; cat > …)` | Covered |
| S-8b JPEG sent after a stop | §9.2 check after `mark_streaming` and a second one after the head (PE-SR5) | Covered |
| S-8c stale subscription slot on renewal | §9.3 `replacesEndpoint`, `replace_or_upsert`, invalid old endpoint = plain upsert | Covered; see PE-SR10 |
| S-8d MJPEG / NV12 / > 480 px | §9.4 daemon-side `remote_preview_image_for_frame` for the remote origin only | Covered; see PE-SR8 |
| Zero test weakening: only CM-SR1 owner-approved; anything else additive and recorded as AM-SRn | §11.3 CM-SR1 (37 → 38, one line inserted), AM-SR1, AM-SR2 (setup only); RMC-S34/S41/S42 listed as compatible unchanged | Covered (verified below) |
| Daemon changes keep fail-closed PAM behaviour | §4.3, §10, §13.3 | Covered: every reserve refusal closes unanswered → PAM read error → `PAM_IGNORE`; root path, `crates/pam` and the wire format unchanged |

### Round-1 items

| Item | Resolution verified |
|---|---|
| PE-SR1 MAJOR (RMC-S34) | Resolved. The only environment read stays `std::env::var_os("XDG_RUNTIME_DIR")` (`crates/push-sender/src/main.rs:26`, the only `env::var*` hit in `crates/push-sender/src`); the anchor comes from `current_exe()` (Linux: `read_link("/proc/self/exe")`, no environment, no NSS) matched component-wise against `.local/bin/soos-push-sender`, consistent with the unit's `ExecStart=%h/.local/bin/soos-push-sender` and `BindReadOnlyPaths=` of the same path. A `" (deleted)"` suffix or another location fails closed (78). New static test `test_rsh_s7_sender_reads_no_other_environment_variable` is strictly stronger than RMC-S34. RMC-S41 forbidden names (`VapidKey`, `p256`, `hmac`, `remote.toml`, …) are not needed by the probe. |
| PE-SR2 | Resolved: wording corrected, residual §13.3, docs, `test_rsh_reserve_auth_after_100ms_is_answered`. |
| PE-SR3 | Resolved: `timeout(200 ms, read_client_frame)` covers prefix and body; `.readable()` banned statically; partial-frame test added. |
| PE-SR4 | Resolved: unparsable `replacesEndpoint` = absent. |
| PE-SR5 | Resolved: second check after the head, before `set_shown`/`write_part`; static pin. |
| PE-SR6 / PE-SR7 | Resolved: static assertions (decode only in `handle_preview_request`, outside the limiter guard; constant ≤ 250 ms and not configurable). |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Unit `ExecStart=%h/.local/bin/soos-push-sender`, `BindReadOnlyPaths=` same path, `TemporaryFileSystem=/run:ro`, `ProtectHome=tmpfs` | `packaging/soos-push-sender.service` | as cited | Yes |
| `SENDER_UNIT_LINES: [&str; 37]`; forbidden prefixes | `tests/invariants/src/remote_push_contract.rs:452-525` | 37; `InaccessiblePaths=` matches no forbidden prefix (`ReadOnlyPaths=`, `BindPaths=` are prefix-anchored) | Yes |
| RMC-S34 reader rule | `remote_push_contract.rs:429-439` | every `env::var(`/`env::var_os(`/`std::env::vars` must be followed by `"XDG_RUNTIME_DIR")` | Spec complies |
| RMC-S36 one-literal tracing | `remote_push_contract.rs:550-590` | as cited | Spec §8.3 complies (one constant string) |
| `thiserror` available to the sender | `crates/push-sender/Cargo.toml` | dependency present | Yes |
| `try_acquire` order Global → ReservedForPrivileged → PerUidCap; `max_connections_per_uid >= 1` | `crates/daemon/src/limits.rs:201-224, 99-101` | as cited | Yes (a `PerUidCap` rejection implies the global and unprivileged checks passed) |
| Every `Auth` is one-shot, including error answers | `dispatcher.rs:572` (`one_shot = req.kind == RequestKind::Auth`) | as cited | Yes, so a reserve connection serves at most one request |
| Inner reserve timeout returns `Err(Timeout)` through `Ok(Err(e)) => return Err(e)` | `dispatcher.rs:461-482` | closes without writing | Yes |
| Permit dropped before the stream | `handle_connection(&self, mut stream)`, local `_permit` | locals drop before the by-value parameter | Yes: `test_same_uid_connection_over_cap_is_closed_without_blocking_others` (`available_permits() == 6` after the reserve refusal) and `test_idle_polling_peer_cannot_exhaust_global_capacity` (refused = 6) stay exact |
| `test_rlc_view_connection_leaves_room_for_auth` third connection | `preview_remote_view_tests.rs:1167-1193` | silent third connection is now closed by the 200 ms window, within the 500 ms `closed_promptly`; later fresh `Auth` still admitted | Compatible unchanged |
| `authorize_user_manager_caller` rule | `session_policy.rs:589-621` | UID-less or non-`REMOTE=0` record → refuse; a local seat record required | Yes (factoring as a shared helper is behaviour-preserving) |
| Tests using an enforced session validator for preview | `preview_remote_view_tests.rs:812, 885, 1093, 1116`; `preview_authorization_tests.rs:456, 483` | others use `enforce_active_session: false` (disabled policy) | AM-SR1 covers line 1093 (record `"3"`, cgroup `session-4.scope`); line 1116 (user manager, record `"3"` local) passes unchanged; line 885 uses record `"4"`; `:483` covered by AM-SR2 |
| Agent test process placement | `/proc/self/cgroup` | `user@1000.service/session.slice/tmux-spawn-….scope` → `UserManager` | AM-SR2 harmless here |
| `parse_session_id_from_cgroup` first-unit rule | `session_policy.rs:249-290` | first unit component | Yes (nested `session-*.scope` under the manager = `UserManager`) |
| Head encoders; no other `HTTP/1.1` literal | `crates/remote/src/http.rs:263-316` | three encoders interpolate `{MANDATORY_HEADERS}`; no other response-head source in `crates/remote/src` | Yes: HSTS/COOP/CORP reach every response |
| S-8b seam and helpers | `server.rs:2341-2420`, `camera.rs:249, 271, 312, 676-689`, `camera_first_frame_refusal` `server.rs:2443` | `Stopped` → `audit_refused`, `None` | Yes |
| RAN-S2 worker assertions | `remote_android_contract.rs:559-693` | listener needs `renewedSubscription(event)`, `postSubscription(`, terminal `.catch(() => null)`; one `fetch(`; `postSubscription` body unchanged | Spec §9.3 complies (`renewalPayload` returns an object whose `toJSON()` is used unchanged) |
| Installer template write | `scripts/install_remote.sh:72-77` | `[[ ! -e … ]]` + `cat >`; no invariant pins the `cat >` text | Yes, §9.1 change is safe for existing tests |
| IR stamp recursion | `crates/daemon/src/preview_image.rs:84-110` | an IR non-Grey frame is converted to Grey and recursed into `preview_image_for_frame` (forwarded unchanged when ≤ 640 wide, any height) | Relevant to PE-SR8 |
| Highest walkthrough | `AI/walkthroughs/193_remote_android.md` | 193 | Yes (194 is next) |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Scenario: an SSH-only process of the owner asks for frames directly → `Session(id)` with `REMOTE=1` → refused; through `systemd-run --user` → `UserManager` → refused while the SSH record exists. Through the `soos-remote` socket with forged `Tailscale-User-Login` (same-UID boundary accepted by ADR 2026-10-06): the companion itself is `UserManager` and is refused while that SSH session exists, so the bypass is closed by the same rule. Moving its own process into the local `session-N.scope` needs write access to root-owned `cgroup.procs` of the common ancestor (`user-<uid>.slice`): not possible.
- Scenario: compromised sender reads `/var/log/journal` via the `wheel`/`adm` ACL → hidden by `InaccessiblePaths=`; `/run/log/journal` and the journald sockets are already under the `/run` tmpfs.
- Result: PASS (docs observation PE-SR11).

### Pillar 2 — PAM deadline & concurrency
- `crates/pam` untouched; no Tokio added to PAM. Root peers use the unchanged path.
- Scenario: user-level lock screen while the view and soos-gui hold both regular permits → reserve admitted → `on_connected` conversation < 200 ms → `Auth` served; ≥ 200 ms → closed unanswered → `PAM_IGNORE` (password). Documented residual, tested at 100 ms.
- Scenario: a same-UID process holds the reserve with a partial frame → closed after 200 ms, permit released (test added). Bound per UID: `max_connections_per_uid + 1`, inside the unchanged global/unprivileged capacity.
- Result: PASS.

### Pillar 3 — Panic safety & fail-closed
- Every new refusal (reserve non-`Auth`, reserve timeout, placement denial, logind read error → `LogindUnavailable`, sandbox not detected → 78, unparsable new endpoint → `400`) ends closed/denied; none produces an allow or `PAM_SUCCESS`. The `Other` placement keeps today's UID-level rule (not a new allow).
- Scenario: the sender binary is replaced while running and later restarted by `Restart=on-failure` → the new binary is at the expected path → probe passes; a `" (deleted)"` path at start → 78 (fail closed, `RestartPreventExitStatus=78`).
- Result: PASS.

### Pillar 4 — Dependencies
- No new runtime crate; the daemon dev-dependency `jpeg-encoder` is already pinned and outside the RLC-S1 normal-edge check; the sender keeps `nix`/`thiserror`, adds no NSS lookup. `soos-remote` stays decoder-free (RLC-S10).
- Result: PASS.

### Pillar 5 — Data confidentiality
- New log lines: `peer_uid` and constant strings only; the sender logs one constant line on refusal and no path. `replacesEndpoint` is never logged (push modules have no tracing, RMC-S37). Decoded MJPEG buffers stay in the existing zeroizing path.
- Scenario: a drop-in that **adds** `BindPaths=%h` (or a bind under `/run`) while keeping the hiding directives leaves the sandbox self-check green (it checks the presence of the tmpfs mounts, not what is mounted on top). See PE-SR9 (recommendation; the review asked about removed directives, which the plan covers).
- Result: PASS with observation.

### Pillar 6 — Test integrity
- CM-SR1 is exactly the owner-approved additive change; the forbidden list is unchanged. AM-SR1/AM-SR2 add only session-record setup; assertions untouched; both justified by the new placement rule. No other existing test needs a change (checked: RMC-S6, RMC-S34, RMC-S36, RMC-S41, RMC-S42, RAN-S2, RLC-S6, the over-cap and room-for-auth tests, the enforced-validator preview tests).
- Test power: S-3 tests fail on today's code (`PerUidCap`); a reserve accepting any request fails `test_rsh_reserve_closes_non_auth_without_payload`; a once-per-connection placement check fails `test_rsh_placement_rechecked_per_request`; a `str::ends_with` home anchor fails the `x.local/bin` case; a substring `ro` match fails the option test; `/home2` guards the prefix test. Gap: the IR path of the remote conversion (PE-SR8).
- Result: PASS with one MINOR test-power gap.

## 4. Findings

- **PE-SR8 [MINOR] S-8d: the remote conversion must bound IR-stamped frames, and no planned test proves it.**
  `preview_image_for_frame` converts an IR non-Grey frame to Grey and **recurses into itself** (`preview_image.rs:84-98`), which forwards a ≤ 640-wide Grey frame of any height unchanged. If `remote_preview_image_for_frame` reuses that IR branch as-is, a 640x512 YUYV IR sensor (the exact device class of S-8d) still reaches `soos-remote` as 640x512 Grey and ends after 10 bad frames. §9.4 says "the IR rule first (greyscale), then …", which is ambiguous.
  *Required (tester/developer, no spec round needed)*: the IR branch of the remote function recurses into `remote_preview_image_for_frame` (or applies `downscale_within(.., Some(480))` to the luma frame); `test_rsh_remote_preview_ir_stamp_is_grey` uses a 640x512 YUYV `SensorType::Infrared` frame and asserts 320x256 Grey; the proptest `test_rsh_remote_preview_output_always_within_640x480` also varies `sensor_type`.

- **PE-SR9 [MINOR, recommended] S-7 detects removed directives but not re-exposure on top of them.**
  Rules 1–2 only require that a read-only tmpfs exists at `/run` and over the home; a drop-in adding `BindPaths=%h` (or `BindReadOnlyPaths=/run/dbus`) keeps both true. Rule 3 is a direct probe and is robust. *Suggested*: add a direct check of the home anchor's content (`read_dir(home)` lists only `.local`, `.local` only `bin`, `bin` only `soos-push-sender`, bounded to a few entries, any error = refuse), or record this explicitly in §13.6 and in the §2d *Limits* text. Not blocking: the review's scenarios (manual run, drop-in removing directives) are covered.

- **PE-SR10 [MINOR, observation] S-8c depends on `event.oldSubscription`, which some browsers leave `null`.**
  When the event carries no old subscription, `renewalPayload` falls back to today's request and the stale slot remains until a 404/410. *Suggested*: add this case to residual §13.7 and to the Android/iPhone docs note on `soos-remote push remove`; no design change.

- **PE-SR11 [MINOR, docs] S-5: a lingering remote session blocks user-manager previews until it is gone.**
  With logind's default `KillUserProcesses=no`, a process started over SSH (for example `tmux`) keeps its session record (`STATE=closing`, `REMOTE=1`) after logout, so the phone view and a user-unit soos-gui stay refused (`camera_refused`) until that process ends. This is the intended rule (same as root `Auth`) but the §7 troubleshooting row should say "an SSH session **or a process left running from one**", with `loginctl list-sessions` / `loginctl terminate-session <id>` as the check and remedy.

No CRITICAL or MAJOR finding. PE-SR8 must be applied in Phase 2/4 (test + implementation detail within the approved design); PE-SR9 to PE-SR11 are recommendations the traceability/docs phase can absorb.

## 5. Verdict

The round-2 spec resolves every round-1 item (PE-SR1 MAJOR included) without relaxing any existing assertion. All eleven review findings are addressed or credibly refuted (S-2 installer "refuse" against RMC-S6). The daemon changes keep the fail-closed PAM behaviour. The only test amendments are CM-SR1 (owner-approved) and the additive setup changes AM-SR1/AM-SR2.

VALIDATION_VERDICT: APPROVED
