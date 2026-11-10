# soos — Full Security Review, 2026-11-10

| | |
|---|---|
| Base commit | `0cce947` (`main`) |
| Scope | The whole repository (18 crates, installers, packaging, CI, Android app, documentation) and a read-only posture snapshot of the maintainer's own installation |
| Method | Multi-agent static review (297 agent runs, 3 rounds) with independent adversarial verification, a manual re-check by the lead reviewer, then an independent fact-check of this document (5 agent runs) |
| Result | 156 confirmed findings: 0 CRITICAL, 0 HIGH, 13 MEDIUM, 108 LOW, 35 INFO. 17 attack chains, 10 of them HIGH behind a precondition |
| Status of this document | **Private working material.** The findings that concern the repository were filed as public issues on 2026-11-10 at the owner's request (Appendix E). This file keeps what was not published: section 9 on the maintainer's installation and the findings `LIVE-02`, `LIVE-03` and `SECR-03`. Do not commit it or the `target/security-review/` folder (section 10) |

Finding identifiers (`RIDN-01`, `PSTK-01`, ...) point to `target/security-review/findings/<ID>.md`. Verification notes are in `verify/<ID>.<lens>.md`, coverage tables in `areas/<key>.md`, the two chain analyses in `chains/remote.md` and `chains/local.md`. Identity values of the live host are masked everywhere.

---

## 1. Bottom line

1. **No unauthenticated compromise was found.** No attacker on the Internet, on the tailnet or under another local account can unlock the PC, see camera pixels, authenticate as the user or become root. Every unlock path and every camera-pixel path of the remote companion requires a valid passkey assertion (user presence and user verification flags, RP ID hash, CSRF gates), passkey registration is refused on the Funnel path, and no reviewed error path of `pam_soos.so` turns into `PAM_SUCCESS`.
2. **No single finding is rated above MEDIUM.** The two findings reported HIGH in round 1 (`RIDN-01`, `XDOC-01`) were lowered to MEDIUM by three independent verifiers each. 115 of the 156 confirmed findings are new; 41 are already tracked or are documented accepted residual risks. 7 raw findings were refuted; all 7 criticised a *proposed fix*, not the code (section 7).
3. **The risk sits in combinations.** 17 attack chains were assembled from confirmed findings. 10 are rated HIGH on the review scale, where HIGH means a serious outcome behind a precondition: a minute at an unlocked desk, code already running as the user, a stolen phone or a copied passkey, physical access to a locked PC with presentation artefacts, a supply-chain position, or access to the disk.
4. **Six themes explain most findings:**
   - *A remote passkey is a silent, durable, Internet-reachable key.* One minute as the user is enough to register an extra passkey. Nothing notifies the owner. With the setup defaults (Funnel, remote unlock and camera view over Funnel all on) that key unlocks the PC and opens the camera from anywhere, also while the owner is working. The passkey and the companion survive a password change and `uninstall.sh`; after `uninstall.sh` only the unlock still works, because the camera view needs the daemon, and a reinstall brings the camera back.
   - *Local identity confusion on the tailnet path* (`RIDN-01`). Any local process is treated as the owner. This was demonstrated on the live host.
   - *Face authentication has no intent signal and no escalating back-off.* `sudo` is approved by the face of whoever sits at the screen (accepted residual), presence unlocks on mere visibility, and a locked PC offers unlimited PAD trials with a timing oracle and a text oracle.
   - *PAM stack integration.* `success=done` ends the whole stack and skips second factors placed after the include; the generic per-service disable flag matches the exact service name only.
   - *Lifecycle and documentation.* Uninstall and purge leave the Internet-facing companion and the biometric data behind, the revocation guidance is partly wrong, and the threat model still describes a local-only product.
   - *Supply chain.* The update channel is an unsigned `main` fed by an autonomous merge loop whose gate is client-side.
5. **The review is not exhaustive.** The deepening loop stopped at its round limit while eight areas were still producing findings (section 8). More LOW and MEDIUM findings should be expected there.

---

## 2. Method and confidence

**Process.** Round 1 ran 47 reviewers: 31 attack-surface areas, 7 cross-cutting lenses (unsafe and panics, filesystem races, subprocess and environment, cryptography, logging and privacy, denial of service and lockout, documentation claims), 8 attacker personas (Internet, tailnet, web content, other local user, same uid, physical, phone, supply chain) and one reviewer for the live posture snapshot. Two further rounds were driven by a completeness critic (10 gap tasks) and by 24 second and third passes over the areas that kept producing findings. Raw findings were de-duplicated (185 raw, 22 merged), then each distinct finding went to independent verifiers told to refute it: three lenses (facts, exploitability, refutation) for HIGH, two for MEDIUM, one for LOW and INFO, with a tie-break vote where needed. Two final analyses composed confirmed findings into attack chains.

**Rules for the agents.** Read-only, static analysis only: no build, no test run, no connection to the daemon or to the companion, no privilege use. Evidence had to cite `file:line`.

**Lead-reviewer check (this document).** I re-read in the code at `0cce947` every fact behind the 13 MEDIUM findings and behind the HIGH chains (table in section 4.1). `RIDN-01` was additionally demonstrated on the live host with read-only requests (`poc/RIDN-01-live-check.txt`). Two statements of the chain analyses were corrected from the verifier notes and the code (section 5).

**How much to trust each class.**

| Class | Evidence behind it |
|---|---|
| MEDIUM findings (13) | 2 or 3 independent verifiers each, plus the lead-reviewer code check |
| LOW and INFO findings (143) | One independent verifier each. Those used by the HIGH chains were re-checked by the lead reviewer, and a sample of 14 others was re-verified from scratch after this report was written: all 14 held. Treat the rest as probable, and read the verify note before acting on the 34 marked *partial* |
| Attack chains (17) | One analysis each, key code facts re-checked by the lead reviewer. Severities are the analysts' and are conditional on the stated precondition |

**Independent fact-check of this document.** After the report was written, five further agents checked it (`factcheck_result.json`). Three compared 188 statements of this document with the code and the documents at the base commit and raised 22 corrections, 3 of them factual errors (how `save.sh` stages files, which sandbox directives are safe in a user unit, and the address-list caveat of `RIDN-01`). All 22 are applied in this version. They found no counter-example to the two central claims: no unlock or camera-pixel path without a verified passkey assertion, and no error path of `pam_soos.so` that returns `PAM_SUCCESS`. Two agents re-verified from scratch 14 LOW findings that had a single verifier: 12 confirmed, 2 confirmed in part (both were already marked partial), none refuted, none re-rated.

**Limits.** Nothing was built or executed. PAD strength was not tested (no presentation attack was attempted; the chains that end in a spoofed face are conditional on an artefact that passes PAD). Tailscale, logind, sudo and su behaviour comes from public sources and manual pages, except for the one live check. The Android app was reviewed statically. Installed binaries on the live host are older than `main`, so the posture section mixes repository facts and host facts.

---

## 3. What held up

About 3,150 scenario rows were played across 81 coverage files; roughly five out of six ended SAFE. The strongest results:

- **Remote HTTP server and identity classification.** 161 of 165 scenario rows SAFE over two passes each (request smuggling, header duplication, Host and forwarded-header handling, body limits, slow clients). The only confirmed defects are the capacity pin `RHTTP-01` and the confused deputy `RIDN-01`.
- **WebAuthn.** 112 of 115 rows SAFE over two passes (challenge binding and single use, origin and RP ID checks, signature and counter handling, CBOR and DER parsing bounds).
- **PAM module.** Panic containment at every C entry point and bounded reads and allocations were found sound (the 10 PAM rows of the unsafe-and-panic lens are all SAFE; the lens as a whole has 41 of 43 rows SAFE across the workspace). Deadline handling is bounded on every path, with one confirmed LOW gap: it ignores time spent in suspend (`PAM-04`). No error path was found that returns `PAM_SUCCESS`: it is returned only for an `Allow` verdict that passed the freshness, request-identifier, version and deadline checks.
- **IPC codec.** Bounded lengths and no recursion (39 of 42 rows SAFE).
- **Cryptography.** 59 of 62 rows SAFE (AEAD use, nonce handling, key file modes, VAPID and RFC 8291). The three other rows are the missing rekey path (`XCRY-01`), the push-metadata leak (`RPSH-01`) and the `crypto-bigint` timing advisory (`CISC-02`). The structural weakness that `master.key` sits beside the templates comes from the biometric-store area (`BSTO-03`).
- **Subprocess and environment handling.** 48 of 51 rows SAFE.
- **Secrets.** The gitleaks history scan reports 3 generic hits, all in redaction test fixtures. `cargo deny` reports one advisory, `RUSTSEC-2026-0387`, against `crypto-bigint 0.5.5`, which `soos-remote` pulls in through `p256 0.13.2`; the real exposure is low (`CISC-02`).

---

## 4. MEDIUM findings

### 4.1 Lead-reviewer verification table

| Finding | Code or document re-read at `0cce947` | Result |
|---|---|---|
| `RIDN-01` | `crates/remote/src/identity.rs:276-303`, `server.rs:1297-1326`; live check | Confirmed, and demonstrated live |
| `XDOC-01` | `Docs/REMOTE_COMPANION.md:162-165`, `identity.rs:300-303`, `config.rs:900-903`, `packaging/soos-remote.service:8-11` | Confirmed in part (see 4.2) |
| `PSTK-01` | `packaging/pam/debian/soos:7-9` | Confirmed |
| `PAM-01` | `crates/pam/src/ipc.rs:507-515` | Confirmed in part (reachable hosts are narrower) |
| `BSTO-01` | `crates/biometric-store/src/store.rs:211-212` | Confirmed |
| `GUI-01` | `crates/gui/src/app.rs:1474-1484`, `:1633-1636` | Confirmed |
| `DIPCC-01` | `crates/daemon/src/limits.rs:226-244` | Confirmed (the subordinate-UID reachability rests on the verifiers' analysis) |
| `DPRSB-01` | `Docs/DISTRIBUTION_DEPLOYMENT.md:709-724`, `server.rs:2048-2050` | Confirmed for the documented recipe |
| `RALR-02` | `crates/remote/src/journal.rs:130-137`, `:770-776` | Confirmed (throughput figures not measured) |
| `VPAD-01` | `crates/daemon/src/consensus.rs:346-355` | Confirmed |
| `RHTTP-01` | `server.rs:1154-1166`, `:1317-1325` | Confirmed (accepted residual) |
| `DDEC-01` | `crates/policy/src/rate_limit.rs:18-24`, `presence/worker.rs:765-777` | Confirmed (accepted residual) |
| `DIPC-04` | `crates/daemon/src/dispatcher.rs:829-836`, `:1162-1167`, `session_policy.rs:592-611` | Confirmed (accepted residual) |

Other chain facts re-read: `enroll-code` needs only the uid (`crates/remote/src/main.rs:273-303`); registration accepts only `none` attestation and trusts the client-set UV flag (`webauthn.rs:316-321`, `:357`); a registration writes one field-less audit line and sends no push (`server.rs:2007-2014`, `audit.rs:38-39`, `:266-268`); the passkey store is validated by owner and mode only and reloads live (`credentials.rs:405`, `:570`); `unlock_flow` has no kill-switch check and `crates/remote/src` never reads `/etc/soos` (`server.rs:2075-2101`); camera start has no lock-state gate (`server.rs:2284-2295`, daemon `dispatcher.rs:1349`); the web-session table has 4 slots and no eviction (`websession.rs:105-108`); alert acknowledgement needs only CSRF headers and a rate gate (`server.rs:2145-2170`); `scripts/uninstall.sh` removes the Web Push sender but never touches `soos-remote`, its user unit, `remote.toml` or the passkey store, and still prints "cleanly uninstalled" (`:577`); the generic per-service disable flag is an exact-name file (`crates/pam/src/config.rs:136-141`) and only `gdm.disable` matches by substring (`:132`); an unfinished consensus whose last capture passed maps to `Unavailable`, which prints a different text (`crates/policy/src/pad_consensus.rs:192-194`, `crates/pam/src/lib.rs:122-129`); purge keeps the biometric store (`packaging/debian/postrm:8-11`); the daemon unit has no `LimitCORE`; the push-sender unit has no IP egress filter (`packaging/soos-push-sender@.service:42`); setup defaults every companion feature to on (`scripts/setup.sh:553-569`).

### 4.2 New findings

#### RIDN-01 — Any local process is "the owner" on the tailnet path (MEDIUM, new, 3/3, reported HIGH)

- **Where:** `crates/remote/src/identity.rs:288`; trust statement in `Docs/REMOTE_COMPANION.md:82-84`.
- **What:** The tailnet class is granted to any request without a Funnel marker that carries one `Tailscale-User-Login` found in `allowed_logins`. The design assumes only `tailscaled` and the owner can produce that header. In kernel-TUN mode `tailscaled` itself listens on the node's own tailnet addresses, port 443, for connections coming from the machine, and stamps the node owner's login on them. A second account, a service account, a host-network container or the `DynamicUser` push sender (`RPSHB-01`) therefore reaches every tailnet-class route with no session and no passkey.
- **Reach:** read status, battery, offline history and alerts; delete the alert history (`/api/alerts/ack`); add, remove or test push subscriptions, which redirects or silences the owner's alerts; lock every 2 s; stop a camera view. Unlock, camera start and passkey registration still need an assertion or an enrollment code (kept in a `0700` directory). The caller nevertheless shares the owner's tailnet limiter key: five malformed or rejected attempts lock the owner's tailnet unlock, camera start and registration out for up to five minutes, and this can be repeated (`PTSN-03`). It can also hold the live status stream (`/api/events`).
- **Lead check:** from a local process on the live host (Tailscale 1.104.5, kernel TUN, listeners present on the node's own addresses at `:443`), `GET /api/status`, `/api/alerts`, `/api/push`, `/api/battery` and `/api/camera` answered `200` with no cookie, passkey or custom header. The request was sent as the owner's uid; a TCP listener cannot tell local uids apart.
- **Fix:** quick mitigation: refuse tailnet-class requests whose `X-Forwarded-For` is one of the node's own Tailscale addresses. Caveat (`G201-01`): a list written once by setup goes stale when the node's addresses change, and an IPv4-only list leaves the IPv6 address open; compare against the host's interface addresses read at request time (short cache, fail closed when the enumeration fails). Robust fix: require a web session or a passkey assertion for state-changing and alert routes on the tailnet path too, as on Funnel. Add `IPAddressDeny=` (loopback, `100.64.0.0/10`, LAN ranges) to the push-sender units. Correct section 2 of the companion documentation.
- **Regression test:** `crates/remote/tests/server_tests.rs`: a tailnet request with an allowed login and a self address in `X-Forwarded-For` is `403` on every route; the same request from a peer address stays `200`.

#### XDOC-01 — Revocation guidance does not always revoke; no incident guide (MEDIUM, new, 3/3 partial, reported HIGH)

- **Where:** `Docs/REMOTE_COMPANION.md:162-165`.
- **What:** The text lists four equivalent ways to revoke remote unlock "at once". Two work on every path: `allow_unlock = false` plus a restart, and `soos-remote passkeys remove N`. "Remove the device from the tailnet" does **not** affect Funnel: the Funnel class never consults tailnet membership or `allowed_logins` (`identity.rs:300-303`), so a stolen phone keeps unlocking over the Internet. "Remove the login from `allowed_logins`" works only by accident in the setup-generated file: the list becomes empty, the service exits with status 78 and the unit does not restart; with several logins configured it revokes nothing on Funnel. There is no procedure for a lost phone, a compromised passkey-sync account or a stolen PC.
- **Fix:** rewrite the paragraph as an ordered procedure whose first step always works (remove the passkey, then turn `allow_unlock` and `allow_funnel` off), state that tailnet removal and `allowed_logins` do not affect Funnel, add an incident section, and add `soos-remote passkeys remove --all` that also ends every web session.
- **Regression test:** an invariant in `tests/invariants` pinning the revocation text and the incident heading.

#### PSTK-01 — `success=done` ends the whole stack and skips second factors (MEDIUM, new, 2/2)

- **Where:** `packaging/pam/debian/soos:7` and the GDM managed block. The Fedora authselect profile carries the same rule (`packaging/pam/fedora/soos/system-auth:9`) with a smaller effect.
- **What:** `done` returns to the application at once. Where the shared file is spliced with `include` or `@include` and the password path does not itself end the stack (the Debian and Ubuntu pam-auth-update layout), and in the GDM managed block, a face match skips every rule the administrator placed after it (`pam_u2f`, `pam_google_authenticator`, `pam_oath`, `pam_shells`), which a password does not. Face then replaces "password and token" instead of "password". In the Fedora authselect profile `pam_unix.so` is already `sufficient`, so a password skips the same rules there, and Fedora's `gdm-password` delegates with `substack`.
- **Fix:** smallest: `soos-admin gdm enable` refuses when a `required` or `requisite` auth rule follows the anchor, and the deployment documentation states the behaviour. Deeper: delegate with `substack` semantics so `done` ends only the delegated part, and add `soos-admin pam check`. Caveat (`G302-03`): do not replace `done` by a literal `success=N` jump under authselect or pam-auth-update, where the count is not stable, and make the refusal parse bracketed controls, `-auth` lines and later `include` or `substack` lines, not only the `required` and `requisite` keywords.
- **Regression test:** Docker PAM matrix, Debian: `@include common-auth` followed by a failing stand-in second factor; a face success must not authenticate.

#### PAM-01 — The conversation runs while an unsent daemon connection is held (MEDIUM, new, 2/2 partial)

- **Where:** `crates/pam/src/ipc.rs:514-515`.
- **What:** The module connects, then calls the application's conversation ("Looking for face..."), and only then writes the request. A setuid host whose conversation writes to a terminal the invoker controls can be blocked there. The daemon admits the root peer against the global capacity of 8 with no short first-frame deadline, so eight stalled processes deny face authentication to everyone for as long as they are respawned; the password fallback is intact.
- **Narrowing found by the verifiers:** `sudo` is not a vehicle by default: it authenticates with PAM module output suppressed (the `pam_silent` option since 1.9.16, on by default). Plain `su user` is not either (util-linux suppresses the info message), and on the Arch layout `su` does not include the shared stack at all (confirmed on the live host). The reachable case identified is `su -` where the `su` stack includes the shared auth stack: the Debian, Ubuntu and Fedora layouts, taken from public knowledge, since their service files were not opened. Other setuid PAM hosts such as `chsh` and `chfn` were not checked.
- **Fix:** write the complete request before calling the conversation, and add a short first-frame deadline for root peers in the daemon. The daemon side is the real fix: the invoker can still send `SIGSTOP` to a setuid host between connect and write.
- **Regression test:** `crates/pam/tests/ipc_hardening_tests.rs`: the request frame reaches a mock daemon before a slow conversation callback starts.

#### BSTO-01 — Templates are keyed only by numeric UID (MEDIUM, new, 2/2)

- **Where:** `crates/biometric-store/src/store.rs:212`.
- **What:** The file name and the AEAD associated data bind only the UID. Nothing removes or flags a template when its account is deleted, and enrollment accepts a UID without an account. An account that later reuses the UID inherits the previous person's face.
- **Fix:** store the account name in the template and in the associated data, compare it with `getpwuid(uid)` at use and answer `Unavailable` on a mismatch; refuse UIDs without a passwd entry at enrollment; flag orphaned templates in `soos-enroll list`. Caveat (`G202-04`): do not use the home inode (invisible under `ProtectHome`) or the shadow last-change field (changes at every password change) as the account marker.
- **Regression test:** dispatcher test with a template recorded for `alice` under a UID that now resolves to `bob`: verdict `Unavailable`, no match attempted.

#### GUI-01 — Guided enrollment binds the face to a free-typed UID (MEDIUM, new, 2/2)

- **Where:** `crates/gui/src/app.rs:1483`, `:1634`.
- **What:** "Username" and "Target UID" are independent inputs and only the UID is stored. Both default to the GUI's own UID (0 in a root session). An administrator who types another person's name without changing the UID binds that person's face to the administrator's account, which then authenticates as the administrator, including for `sudo`.
- **Fix:** make the username the only input, resolve it at save time and show the resolved `name (UID n)` read-only; never default to UID 0; confirm before replacing another person's template.
- **Regression test:** a pure `resolve_enrollment_target` helper with an injected user lookup.

#### DIPCC-01 — Per-UID limits assume one account equals one kernel UID (MEDIUM, new, 2/2)

- **Where:** `crates/daemon/src/limits.rs:236`.
- **What:** The connection cap and the password-failed event quota are keyed on the peer UID. A member of group `soos` can run processes under its subordinate UIDs, which keep the group, and so holds every unprivileged permit (no face unlock on user-level lockers) and exhausts the intruder-evidence quota.
- **Fix:** give regular permits only to UIDs that own a logind session and put every other unprivileged peer in one shared bucket; require a local seat session before capturing evidence for a self-reported failure.
- **Regression test:** `crates/daemon/tests/peer_limits_tests.rs`: six session-less UIDs do not prevent a UID with a local seat session from being admitted.

#### DPRSB-01 — The documented wlroots unlock hook kills the wrapper (MEDIUM, new, 2/2)

- **Where:** `Docs/DISTRIBUTION_DEPLOYMENT.md:724`, repeated in `Docs/REMOTE_COMPANION.md:1436`.
- **What:** The documented hook `pkill -USR1 swaylock` also matches the wrapper script `swaylock-presence`, which dies before clearing `LockedHint`. After a presence or remote unlock the desktop is open while logind, the phone and the remote lock route (`409 already_locked`) all say "locked", and presence never scans the next lock period.
- **Fix:** `pkill -x -USR1 swaylock` in both documents, and `-x` as well for the hyprlock hook (`Docs/DISTRIBUTION_DEPLOYMENT.md:739`, `tests/physical/screensaver_test.md:337`); a wrapper that traps `USR1` and clears the hint on every exit path. Deeper: `lock_flow` always calls `LockSession`. Caveat (`G302-04`): that deeper change alone lets any lock caller clear the hint through a second wrapper instance, so the wrapper must refuse a second instance.
- **Regression test:** an invariant that rejects a `swayidle` unlock hook using `pkill` without `-x` in `Docs/` and `tests/physical/`.

#### RALR-02 — Any local account can delay failed-password alerts past the push window (MEDIUM, new, 2/2)

- **Where:** `crates/remote/src/journal.rs:136`.
- **What:** The follower asks `journalctl` for every authpriv entry of the system and filters by UID afterwards, at a deliberately bounded read rate. A sustained `logger -p authpriv.info` flood makes it fall behind; once the lag exceeds the 5-minute push age limit, genuine attempts are recorded too late and are never pushed.
- **Fix:** move the UID restriction into the journal match (`_UID=0` or the owner's UID) and keep the client-side gate; expose a "reader lagging" state.
- **Regression test:** `follow_args` contains the UID match terms; an integration test with a scripted source serving foreign-UID lines before a real attempt.

#### VPAD-01 — Reply latency separates a PAD rejection from a non-match (MEDIUM, new, 2/2)

- **Where:** `crates/daemon/src/consensus.rs:348`.
- **What:** The denial text was unified to hide the reason, but a spoof veto ends the consensus loop at once while a live non-matching face runs to the deadline. The delay before "Face not recognized." is an oracle for tuning a presentation attack. `VPADB-01` (LOW) is the companion text oracle: "Face verification unavailable." reveals that the last capture passed PAD and matched.
- **Fix:** hold every non-Allow verdict until the request deadline. Caveat (`G202-03`): the variant `not_before + DECISION_BUDGET_MS` leaves the oracle on GDM, whose `timeout_ms` is 2500.
- **Regression test:** dispatcher test with an injected clock: a PAD-failed run and a no-match run are answered no earlier than the same floor.

### 4.3 Documented accepted residuals, confirmed still present

- **RHTTP-01 — One Funnel client pins all anonymous slots** (`crates/remote/src/server.rs:1157`). A successful anonymous response keeps its permit through the 2 s close linger; two to four cheap requests per second keep all four anonymous permits busy and the owner's Funnel login gets `503 busy`. The ADR accepts a *distributed* attacker; one client is enough. Fix: use the 100 ms refusal linger for anonymous served responses and add a per-client-hint cap. Caveat (`G202-02`): releasing only the class permits moves the pin to the 16-slot global connection limit.
- **DDEC-01 — The face attempt budget never escalates** (`crates/policy/src/rate_limit.rs:21`). 40 attempts per 60 s in memory, no back-off, no lockout, reset by a daemon restart; a spoof veto costs the same as a non-match and presence warns once per lock period (`DPRS-01`). Fix: a persisted per-UID counter that weighs spoof vetoes, with exponential back-off ending in "password required". Caveat (`G202-01`): do not count empty-room presence scans or passers-by, and accept the reset signal only from a root peer after a real password success, or the owner gets locked out of face by anyone walking past.
- **DIPC-04 — `sudo` is approved by the face of the person at the desk** (`crates/daemon/src/dispatcher.rs:831`). Any code running as the user can call `sudo` while the owner looks at the screen; sudo suppresses PAM module output by default, so nothing is printed. Fix: make privilege-granting services (`sudo`, `sudo-i`, `su`, `su-l`, `runuser`, `polkit-1`) opt-in, enforced in the daemon on the `service` field, and make the disable flags prefix-aware (`PAM-03`).

---

## 5. Attack chains

No chain reaches an unlock or a camera view without a valid passkey or a matching face. The chains describe where that credential can come from, what it reaches, how long it lasts and what the owner can see or do about it. Severities are conditional on the precondition.

| Chain | Severity | Precondition | End state | Cheapest breaking fix |
|---|---|---|---|---|
| CH-REMOTE-01, CH-LOCAL-01 | HIGH | One minute as the user (unlocked desk, or code that runs once as the user), with the setup-default Funnel, unlock and camera settings | A planted passkey unlocks the PC and opens the camera from the Internet at any lock state. No notification. The passkey survives a password change, `uninstall.sh` and a reinstall (after `uninstall.sh` only the unlock works until the daemon is back) | Push and list every change of the passkey set; step-up for `enroll-code`; separate opt-in for Funnel unlock and Funnel camera |
| CH-REMOTE-02 | HIGH | A stolen phone with a known PIN, a synced or shared copy of the passkey, or a relayed hybrid QR prompt | Unlock and camera; alert history wiped; owner kept out by the 4-slot session table; only a local CLI revokes | Per-event push to the other devices; camera only while locked; correct revocation text; `passkeys remove --all` |
| CH-REMOTE-03, CH-LOCAL-07 | MEDIUM | Any other local uid, or a compromised push sender | Owner identity on tailnet routes: presence surveillance, alert deletion or redirection, lock denial of service, lockout of the owner's tailnet unlock through the shared limiter, unalerted `su` guessing. No unlock, no camera | Refuse self-originated tailnet requests; `IPAddressDeny=` in the sender units |
| CH-REMOTE-04 | MEDIUM | An anonymous Internet client plus an accomplice at the PC | The owner cannot sign in over Funnel while it matters | Release the anonymous permit before the linger; per-hint cap |
| CH-REMOTE-05, CH-LOCAL-05 | MEDIUM to HIGH | A wlroots host using the *documented* hook, or an intruder who sets `LockedHint` | Desktop open while the phone says Locked; remote lock answers 409; entry to CH-REMOTE-01 | Fix the hook (`pkill -x`) and the wrapper; `lock_flow` always calls `LockSession` |
| CH-REMOTE-06 | LOW (a), MEDIUM (b) | (a) any page in the owner's browser; (b) other web content hosted on the same node name | (a) event-stream slots exhausted, page shows Unreachable; (b) same-origin script can lock, acknowledge, subscribe and reuse an approved passkey prompt | `Sec-Fetch-Site` check and per-class caps on `/api/events`; setup warns when the node name serves anything else |
| CH-REMOTE-07 | LOW | A malicious or compromised push service | Metadata profiling, silent drop, stale replay | Sequence and time in the payload, checked by the service worker; padding; generic `Topic` |
| CH-LOCAL-02 | HIGH | Malware running as the user while the owner sits at the camera, face rule in the `sudo` stack | Silent root, decryption of every template with the adjacent `master.key`, PAD model and manifest swapped for a photo-unlock backdoor | Face off by default for privilege-granting services, enforced in the daemon |
| CH-LOCAL-03 | HIGH, conditional on a PAD-passing artefact | Walk-up at a locked PC with prints, screens or masks | Unattended, near-silent PAD tuning with a timing and a text oracle; on success, unlock without the configured second factor | Persisted escalating back-off that counts spoof vetoes |
| CH-LOCAL-04 | HIGH, documented out of scope, conditional | A hostile USB camera hot-swapped at a locked PC | Injected video of the owner feeds presence unlock | Suspend face authentication after a camera re-resolution until a password login |
| CH-LOCAL-06 | HIGH | An enrollment slip in the GUI, or UID reuse | Another person's face authenticates as the account, then root through face `sudo` | Resolve the username to the UID in the GUI; bind templates to the account name |
| CH-LOCAL-08 | MEDIUM | A second account in group `soos` with subordinate UIDs | Face denied on user-level lockers; daemon security log and evidence suppressed | Throttle per-connection warnings; key limits by account |
| CH-LOCAL-09 | HIGH | A malicious contribution, injected issue text steering the agent loop, or the freed former namespace | Attacker code merged to `main` and installed as root by every upgrading user | Server-side branch protection with a human approval; signed releases; fix the stale URLs |
| CH-LOCAL-10 | HIGH, moot with full-disk encryption | The powered-off disk, a backup, or a purged or resold machine | Every face embedding decrypted; with write access, a stealth always-live PAD | Seal `master.key` (systemd-creds, TPM); purge deletes `/var/lib/soos`; model hashes in the binary |

**Corrections to the chain write-ups.**

- `chains/remote.md` (CH-REMOTE-02 step 6) and `chains/local.md` (CH-LOCAL-01 step 5) say that removing the login from `allowed_logins` does nothing. In the setup-generated configuration it stops the service (empty list, exit status 78, no restart), so it does revoke, by accident. Only "remove the device from the tailnet" is ineffective there. See `XDOC-01`.
- `chains/local.md` (CH-LOCAL-02 step 2) cites `su-l` on the live host as an alias that stays face-enabled. On the live Arch layout `su` and `su-l` use `pam_unix` directly and never reach `pam_soos`; the alias problem (`PAM-03`) concerns distributions whose `su` stack includes the shared auth stack, and `sudo-i` where that service file exists.

---

## 6. Remediation plan

Batches are sized for the project's topic-branch workflow. Effort is a rough estimate: S under a day, M a few days, L a week or more. Each batch names the fix-design caveats it must respect (section 7).

### Priority 1 — break the remote chains

| # | Suggested branch | Findings | What to change | Chains affected | Effort |
|---|---|---|---|---|---|
| 1 | `fix/remote-passkey-visibility` | `PPHY-01`, `PUID-01`, `PPHN-01`, `PPHN-02`, `RUNL-02`, `RUNLB-01` | Push "a passkey was added" to every subscription on any change of the credential set seen at store load (covers direct file edits); list passkeys with creation time and fingerprint in the authenticated state view; add credential fingerprint and path class to the audit lines; do not let an inherited `RUST_LOG` silence audit events; push on Funnel login, unlock and camera start to the other devices; once a passkey exists, `enroll-code` issues a code only after an assertion of an existing passkey or a root confirmation through polkit; name the purpose (sign in, unlock, camera) in the page before each passkey prompt | CH-REMOTE-01, -02, CH-LOCAL-01 | M |
| 2 | `fix/remote-self-origin` | `RIDN-01`, `RPSHB-01`, `PTSN-02`, `PTSN-03`, `RWEB-02` | Refuse self-originated tailnet requests; require a session or assertion for state-changing and alert routes on the tailnet path; separate limiter keys so one tailnet caller cannot lock the owner out; `IPAddressDeny=` in the push-sender units; drop a device's push subscription when it signs out. Caveat `G201-01` | CH-REMOTE-02, -03, CH-LOCAL-07 | M |
| 3 | `fix/setup-remote-defaults` | `SETB-01`, `SETB-02`, `SETBB-01`, `SETB-03`, `RCFG-02`, `GUI-03` | `allow_unlock` and `camera_view_funnel` default to off with a dedicated question each; an absent key is never switched on by a re-run; the remote.toml reader fails safe (every feature off) on any construct it cannot model; setup checks the existing Serve configuration before publishing `:443`, pins `allowed_hosts` to `rp_id` and keeps the owner's hardening keys when it rewrites the file; the GUI's "Internet access Off" says that the Funnel listener stays published and shows the command that removes it | CH-REMOTE-01, -02, -06 | M |
| 4 | `fix/remote-gates` | `RCAMC-01`, `RCAM-01`, `XDOC-02`, `RUNLC-01`, `DPRSB-01`, `RUNL-01`, `PINT-02` | Camera view only while the session is locked unless `camera_view_unlocked = true`; `unlock_flow` honours `/etc/soos/disabled` and the presence account guard; remote lock always calls `LockSession` and gets an off switch and a per-path gate; fix the documented hook and wrapper; an on-screen sign at the PC while a remote view runs; acknowledging alerts hides them without deleting the history. Caveat `G302-04` | CH-REMOTE-01, -02, -05, CH-LOCAL-05 | M |
| 5 | `fix/remote-capacity` | `RHTTP-01`, `XDOS-02`, `RWEB-01`, `RWAN-01` | Refusal linger for anonymous served responses plus a per-hint cap; evict the oldest web session of the same credential instead of answering 429; `Sec-Fetch-Site` check and separate tailnet and Funnel caps on `/api/events`; a limiter and challenge pool per Funnel session so one stolen session cannot deny the owner's unlock and camera. Caveat `G202-02` | CH-REMOTE-02, -04, -06 | M |
| 6 | `chore/security-docs` | `XDOC-01`, `XDOC-02`, `XDOC-03`, `XDOC-04`, `SECR-04`, `RCAMB-03`, `PSTK-05`, `PSTKB-02`, `G304-01` to `G304-05` | Revocation procedure and incident guide; threat model updated for the Internet-reachable companion; `SECURITY.md` with a private reporting channel; corrected PAM examples and deployment recipes | CH-REMOTE-02, CH-LOCAL-05 | S |
| 7 | `fix/uninstall-lifecycle` | `INST-01`, `INST-02`, `RCFG-04`, `RPSHC-01`, `PSTK-03`, `XFSR-02`, `G205-01` to `G205-03` | `uninstall.sh` detects and stops the companion and does not print "cleanly uninstalled" while it runs; purge removes or clearly reports the biometric store and keys; updates restart the running companion and sender; PAM restore does not discard later changes or write truncated files | CH-REMOTE-01, CH-LOCAL-01, -10 | M |

### Priority 2 — face authentication policy and local trust

| # | Suggested branch | Findings | What to change | Chains affected | Effort |
|---|---|---|---|---|---|
| 8 | `fix/face-backoff` | `DDEC-01`, `DPRS-01`, `VPAD-01`, `VPADB-01`, `VPAD-02`, `PPHY-03`, `EVID-03` | Persisted escalating back-off counting spoof vetoes; constant-time denial up to the request deadline; one denial text for every non-Allow outcome; an upper face-size gate so a close-up cannot strip the PAD context; evidence capture that an early covered lens cannot exhaust. Needs an ADR update. Caveats `G202-01`, `G202-03` | CH-LOCAL-03, -04 | L |
| 9 | `fix/face-service-scope` | `DIPC-04`, `PAM-03`, `PAM-02`, `PSTKB-01`, `DPRSC-01`, `DPRS-02`, `PPHY-02` | Privilege-granting services opt-in in the daemon; prefix-aware and fail-closed disable flags; face honours a locked password; presence respects a second factor configured for the locker, and gains an intent or absence signal; a way for the owner to require the password from away (lost mode). Needs an ADR update. Caveat `G302-05` | CH-LOCAL-02, -03, -06 | L |
| 10 | `fix/pam-stack-done` | `PSTK-01`, `PSTK-02`, `G304-01`, `PSTKC-01` | Detect and refuse post-include factors; `substack` delegation; `soos-admin pam check`. Caveat `G302-03` | CH-LOCAL-02, -03 | M |
| 11 | `fix/enrollment-identity` | `BSTO-01`, `GUI-01`, `ENRL-01`, `GUI-06`, `SETA-02`, `BSTO-02` | Bind templates to the account name; one resolved enrollment target in the GUI; no mock templates in the production store; an audit record for every template write. Caveat `G202-04` | CH-LOCAL-06 | M |
| 12 | `fix/daemon-admission` | `DIPCC-01`, `PAM-01`, `DRUN-03`, `DIPC-03`, `EVID-02`, `XDOS-01` | Account-keyed limits; request before conversation and a first-frame deadline for root peers; throttled warnings; evidence quotas per reason and per service; the logind scan cap applied after the session filter, so another account's sessions cannot switch face off | CH-LOCAL-08 | M |
| 13 | `fix/remote-journal-filter` | `RALR-02`, `PLOC-01`, `RALR-01`, `RALR-03`, `RALR-04` | UID match in the journal query; classify setuid `su` and `sudo` by executable before the UID gate; replay attempts made while the companion was down; bound clock steps; canonicalise `lock_screen_programs` paths | CH-REMOTE-03, CH-LOCAL-07 | M |
| 14 | `fix/camera-reresolution` | `CAMV-01`, `G302-01`, `VPAD-03`, `G301-01` | After a camera re-resolution at runtime, suspend presence and face authentication until the next password login and log it at warn; never take the PAD modality from device metadata alone; verify camera controls and emitter state; strict IR mode (tracked in #353) | CH-LOCAL-04 | M |

### Priority 3 — hardening backlog

| # | Suggested branch | Findings | What to change | Effort |
|---|---|---|---|---|
| 15 | `chore/unit-hardening` | `RCFG-01`, `DRUN-01`, `DRUN-02`, `RCAMB-02`, `RCFG-03` | Seccomp-level sandbox for the companion (`SystemCallFilter=@system-service`, `RestrictNamespaces=yes`, `SystemCallErrorNumber=EPERM`). No `ProtectKernel*`, `Protect*`, `Private*`, `*Paths=` or `CapabilityBoundingSet=` directive in the user unit: each implies `PrivateUsers=` there and breaks the camera view (caveat `G302-02`). `LimitCORE=0` and a non-dumpable daemon; no swap or core dump of camera frames in the companion; owner and mode checks on `remote.toml` | S to M |
| 16 | `chore/supply-chain` | `CISC-01`, `CISC-02`, `CISC-03`, `PSUP-01`, `SECR-01`, `SECR-03`, `INFR-02`, `INFR-03`, `SETA-01`, `SETAB-01` | Server-side required checks on `main`; signed tags as the update channel; the stale repository URLs (the reviewers counted 19); model hashes compiled into the binary; a clean cargo environment for the build that root installs; a dated exception or a dependency bump for `RUSTSEC-2026-0387`; `secret_scan.sh` extended to the companion's secret shapes; remove the description of the maintainer's host from the public repository | M, plus owner decisions |
| 17 | `fix/key-protection` | `BSTO-03`, `XCRY-01`, `PPHY-04` | Seal `master.key` and `evidence.key` with `systemd-creds` or a TPM; a rekey command; document hibernation images and the root CLIs in the memory-protection page | L |
| 18 | individual LOW and INFO items | Appendix A | Each entry carries its own first fix step | S each |

Batches 8, 9 and 16 change owner decisions: the 40-per-minute budget, face for `sudo` and presence on by default (`AI/DECISIONS.md`), and the autonomous merge loop (`AGENTS.md`, Branching & Pull Request Policy). They need an owner decision before any code.

---

## 7. Fix-design caveats (the 7 refuted items)

All seven refuted findings criticise the *recommended fix* of another finding. Six came from the two fix-safety reviewers (`G202`, `G302`) and `G201-01` from the Tailscale proxy-facts reviewer. The verifiers rejected them as findings because they describe code that does not exist yet. The technical points are still worth keeping when the fixes are written.

| Refuted item | Applies to | Caveat for the implementer |
|---|---|---|
| `G201-01` | `RIDN-01` | A setup-written list of the node's own addresses fails open when the addresses change or when it omits the IPv6 address. Compare against the host's interface addresses read at request time (short cache, fail closed when the enumeration fails), or use the session-based fix |
| `G202-01` | `DDEC-01` | A cumulative lockout that counts empty-room presence scans, passers-by and same-uid callers becomes a sticky owner lockout, and a reset signal any peer can send is forgeable |
| `G202-02` | `RHTTP-01` | Releasing only the class permits while keeping the 2 s linger moves the pin to the 16-slot global connection limit. Shorten the linger |
| `G202-04` | `BSTO-01` | A home-inode marker is invisible under `ProtectHome` and fails closed for everyone; the shadow last-change field breaks at every password change; the name alone misses a same-name re-creation |
| `G302-02` | `RCFG-01` (and its merged duplicate `LIVE-01`) | Namespace and `Protect*` directives in the *user* unit imply `PrivateUsers=`: the camera view stops on every host, pushes stop on hosts that use the system push sender, and a `ReadWritePaths=%t/soos-remote` entry without the `-` prefix keeps the companion down after a reboot. An invariant already rejects `ProtectHome` there |
| `G302-03` | `PSTK-01` | A literal `success=N` jump miscounts under authselect and pam-auth-update; a `gdm enable` refusal that reuses the keyword-only `required` and `requisite` predicate misses bracketed controls, `-auth` lines and later `include` or `substack` lines |
| `G302-05` | `PPHY-02` | A "lost mode" run by the daemon needs write access to `/etc/soos` and would let owner-uid code disable face persistently |

Three more fix-variant warnings were confirmed as INFO findings and are listed in Appendix A: `G202-03` (the `VPAD-01` floor on GDM), `G302-01` (`CAMV-01`'s device pin uses attacker-controlled USB strings) and `G302-04` (always calling `LockSession` lets a lock caller clear `LockedHint`).

---

## 8. Coverage and gaps

- **Scenario tables.** 81 coverage files under `areas/`, about 3,150 scenario rows, each with a result (SAFE, FINDING, DOCUMENTED, UNKNOWN) and evidence. Appendix C gives the count per area.
- **Endpoints.** The routes of `crates/remote/src/routes.rs` were reviewed on both request classes by the HTTP, identity, WebAuthn, unlock, camera, alerts, push and web reviewers and by the Internet, tailnet, web and phone personas. The daemon's IPC request kinds were reviewed by the IPC, decision, presence and runtime reviewers. This is code review, not dynamic testing.
- **Still productive when the loop stopped.** remote-unlock, remote-camera, remote-push, pam-stacks, daemon-ipc, daemon-presence, setup-system, setup-companion. A fourth round over these eight areas is the natural next step.
- **Not done.** Dynamic testing of any kind except the one live check; fuzzing beyond the existing fuzz targets; measurement of PAD strength with real artefacts; a device run of the Android app; real behaviour of lockers, GDM, sudo and su on each distribution; Tailscale proxy behaviour under load (HTTP/2 stalls, header rewriting, source-address fidelity through Funnel relays); journald throughput under flood. The 200 individual "not covered" notes of the reviewers are in the `uncovered` list of `workflow_result.json` and at the end of each coverage file.

---

## 9. The maintainer's installation (snapshot of 2026-11-10)

Facts read from the host, read-only. This section is the reason this document must stay private.

| Fact | Consequence |
|---|---|
| Funnel on, `allow_unlock`, `allow_funnel`, `camera_view` and `camera_view_funnel` all true, presence on by default | The host is in the exact configuration assumed by CH-REMOTE-01, CH-REMOTE-02 and CH-LOCAL-01 (`LIVE-03`) |
| Installed binaries date from 2026-10-12 and 2026-10-13; the legacy user-unit push sender is active and enabled; the sandboxed system sender is not installed | The only network-capable soos process runs unsandboxed under the owner's uid, built before the rustls and webpki advisory fixes (`LIVE-02`) |
| A local process reaches the companion as the owner through the node's own tailnet address | `RIDN-01` is live. The machine has a single interactive account, so the exposed parties are service accounts and host-network containers |
| The account is in `wheel` and `docker` | Whoever sits at the session after any unlock is root-equivalent through docker, without a password. A remote unlock leaves the session open until it is locked again |
| `sudo` reaches the face rule; no `/etc/soos/sudo.disable`; sudo 1.9.17 (`pam_silent` default); no `sudo-i` service file | `DIPC-04` applies silently. A `sudo.disable` flag would cover `sudo -i` on this host |
| No LUKS volume detected (a self-encrypting drive was not checked) | CH-LOCAL-10 is not moot: the disk alone gives the face templates with their key, the passkey store and the VAPID key |
| The owner's own `swayidle` hook uses `pkill -USR1 -x` | The wrapper-kill variant of `DPRSB-01` does **not** affect this host. A same-uid process can still set `LockedHint` (`RUNLC-01`) |
| No idle auto-lock (owner's choice) | The "unlocked desk" precondition depends on manual locking only |
| `su` and `su-l` use `pam_unix` directly; `sshd` is disabled | `PAM-01` is not reachable through `su` here; the remote-login stacks are not exposed |
| `systemd-analyze security`: daemon 1.7 (OK), companion 8.0 (EXPOSED) | `RCFG-01` |
| The public repository describes this host (`AI/research_live_camera.md:155`) and unit `Documentation=` links point to the former namespace | `SECR-03`, `SECR-01` |

**Suggested owner actions, in order. None was performed by this review.**

1. **Check what is registered now.** `soos-remote passkeys list` and, on a current build, `soos-remote push list`. Registration is silent, so this is the only way to know. Remove anything unknown with `passkeys remove N` or `push remove N`.
2. **Decide how much Internet exposure is needed day to day.** Setting `camera_view_funnel = false` and, if remote unlock is not needed, `allow_unlock = false` in `~/.config/soos/remote.toml`, then `systemctl --user restart soos-remote.service`, removes the two capabilities the HIGH remote chains end in. `tailscale funnel --https=443 off` removes the public listener altogether; the phone then needs the tailnet.
3. **Update the installation from `main`.** This installs the sandboxed push sender and the advisory fixes. Read the plan `setup.sh` prints before accepting it: a re-run switches on any companion key that is absent from the file (`SETB-01`), so pass the explicit `off` flags for what should stay off. Afterwards the legacy `soos-push-sender.service` user unit should be disabled and `soos-push-sender@<uid>.socket` active.
4. **Optional:** `sudo touch /etc/soos/sudo.disable` if face for `sudo` is not worth the same-uid risk. On this host docker membership already makes the account root-equivalent, so this matters mostly once that is addressed.
5. **Until `RIDN-01` is fixed,** avoid untrusted workloads in host-network containers or under service accounts on this machine.
6. **Consider disk encryption.** Biometric templates cannot be rotated.

---

## 10. Handling of this review

- **Update, 2026-11-10.** At the owner's request the confirmed findings were filed as 171 public issues (one tracking issue, 17 remediation batches, 153 findings; Appendix E). An automated scrub and two independent privacy gates removed every reference to the maintainer's installation and to the private working files first. `LIVE-02`, `LIVE-03`, `SECR-03` and section 9 were withheld because they describe that installation.
- The repository is public and has no private reporting channel (`SECR-04`). Keep this file and `target/security-review/` out of git. `save.sh` stages tracked modifications plus every new file under `crates/`, `tests/`, `Docs/`, `AI/`, `scripts/` and `packaging/`, so this report must never be placed in one of those directories. `target/` is outside them and is ignored (`/target/`), so the current location is safe from `save.sh`, but `cargo clean` deletes it. A copy outside the repository is advisable.
- The issues now describe the remote chains publicly. Reducing the exposure of the live host (section 9, actions 1 to 3) should not wait for the fixes.
- Enabling GitHub private vulnerability reporting and using draft security advisories would give a private place for this work.
- A previous full review was published in the repository before its fixes (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`). The public edition of this review is the tracking issue. This file should stay private because of section 9.

---

## Appendix A — All confirmed findings

Generated from `workflow_result.json` and `findings/<ID>.md`. Format: identifier, final severity (reported severity when the verifiers changed it), known status, verifier votes, title, location, and the first step of the recommended fix as written by the reviewer. *Partial* means the verifier confirmed the core defect but corrected part of the write-up: read `verify/<ID>.*.md` first. The fix lines of LOW and INFO findings were not re-checked by the lead reviewer. *Re-verified after the report* marks the 14 LOW findings of the independent sample (section 2).

| Severity | Confirmed | New | Accepted residual | Tracked issue |
|---|---|---|---|---|
| CRITICAL | 0 | 0 | 0 | 0 |
| HIGH | 0 | 0 | 0 | 0 |
| MEDIUM | 13 | 10 | 3 | 0 |
| LOW | 108 | 77 | 30 | 1 |
| INFO | 35 | 28 | 7 | 0 |
| Total | 156 | 115 | 40 | 1 |

Raw findings: 185. Duplicates merged: 22. Refuted: 7. Confirmed by round: round 1: 116, round 2: 22, round 3: 18. Marked partial: 34.

### Remote companion, Web Push and Android (45: 3 MEDIUM, 36 LOW, 6 INFO)

- **RALR-02** · MEDIUM · new · 2/2 — Any local account can lag the journal follower past the push age window by flooding authpriv.
  Where: `crates/remote/src/journal.rs:136`.
  Fix: Move the UID restriction into the journalctl match so other users' lines are never emitted: `SYSLOG_FACILITY=10 _UID=0 + SYSLOG_FACILITY=10 _UID=<owner_uid>` (journalctl ORs terms separated by `+`; `owner_uid` is a validated integer), keeping the client-side gate as defense in depth.
- **RHTTP-01** · MEDIUM · accepted residual (AI/DECISIONS.md ADR [2026-10-06] Tailscale Funnel Access and In-House Passkey Authentication, accepted ...) · 2/2 — One Funnel client can pin all anonymous Funnel slots through the served-response linger and block owner login.
  Where: `crates/remote/src/server.rs:1157`. Merged duplicates: PINT-01.
  Fix: In `answer()` (`server.rs:1157`), release `permits` before the lingering close for **every** Funnel response that holds an anonymous permit, not only for refusals. Equivalently, use `refusal_linger()` (100 ms) when `class == Funnel && ctx.session.is_none()`. Caveat: `G202-02` (section 7).
- **RIDN-01** · MEDIUM (reported HIGH) · new · 3/3 — Any local account reaches soos-remote as the owner's tailnet identity through tailscaled's local listener.
  Where: `crates/remote/src/identity.rs:288`. Merged duplicates: PTSN-01.
  Fix: Refuse tailnet-class requests that originate from the node itself. - Serve sets `X-Forwarded-For` to the TCP source address (`r.Out.Header.Set("X-Forwarded-For", c.SrcAddr.Addr().String())`). Caveat: `G201-01` (section 7).
- **ANDR-01** · LOW · accepted residual (Docs/REMOTE_COMPANION.md section 2j Build (owner) and section 8 residuals ("the debug APK is debuggable")) · 1/1 — The only supported Android artifact is a debuggable debug build carrying the login trust-anchor signature.
  Where: `scripts/android_build.sh:34`.
  Fix: Add a `release` build path to `scripts/android_build.sh` (`assembleRelease`, unsigned) and change section 2j to install the release APK re-signed with `apksigner` (the release build type is not debuggable).
- **PINT-02** · LOW · new · 1/1 · re-verified after the report (confirmed) — A stolen Funnel session cookie alone can erase the failed-password alert history.
  Where: `crates/remote/src/server.rs:2145`.
  Fix: Treat acknowledgement as a destructive action on the Funnel path: require a fresh passkey assertion (reuse the unlock-purpose challenge with the web-session binding), or keep acknowledged entries visible in a separate "acknowledged" list for the history window instead of deleting them.
- **PLOC-01** · LOW (reported MEDIUM) · new · 2/2 partial — Password guessing against the owner (or root) by another local account through su never reaches the failed-password alerts.
  Where: `crates/remote/src/journal.rs:773`.
  Fix: In `classify_entry` (crates/remote/src/journal.rs:770), let the uid gate pass any entry whose `_EXE` (after `exe_for_comparison`) is in `SETUID_AUTH_PROGRAMS` (and, if desired, a list of setuid polkit helper paths), keeping the gate for everything else.
- **PPHN-01** · LOW (reported MEDIUM) · accepted residual (ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey Authentication for soos-remote") · 2/2 partial — Synced or shared passkeys can unlock the PC and open the camera with no per-use trace: no device-bound policy, no last use, no attribution, no notification.
  Where: `crates/remote/src/server.rs:2099`.
  Fix: Push a short fixed notice (no identity, no credential id) to every subscribed device on each accepted remote unlock, camera view start and passkey registration ("Your PC was unlocked from the companion"), reusing the alert push path with its own topic.
- **PPHN-02** · LOW · new · 1/1 — Login, unlock and camera passkey prompts are indistinguishable and can be relayed to the owner's phone through the cross-device QR/hybrid flow.
  Where: `crates/remote/assets/app.js:444`.
  Fix: Document it. Add to Docs/REMOTE_COMPANION.md section 8 and to the web and Android feature tours: "soos never asks you to scan a QR code; scanning a soos QR code shown on another screen hands that screen your PC (sign-in, unlock, camera)".
- **PPHY-01** · LOW (reported MEDIUM) · accepted residual (ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey Authentication for soos-remote" item (10), ...) · 2/2 — A few minutes at an unlocked desk plant a scriptable passkey that unlocks the PC from the Internet indefinitely; the owner is never told.
  Where: `crates/remote/src/server.rs:2013`.
  Fix: When a passkey is registered, queue a push notification ("A new unlock passkey was added on <time>") to every existing push subscription, and expose the passkey list (index, creation time, backup state, short fingerprint, as `passkeys list` prints) in `GET /api/auth/state` for authenticated ...
- **PPHY-02** · LOW · new · 1/1 — No lost mode: owner of a stolen, locked PC cannot disable face/presence unlock; offline PC keeps accepting face attempts forever.
  Where: `crates/remote/src/server.rs:2027`.
  Fix: Add an authenticated, passkey-asserted companion action "Require password" that asks the daemon (new privileged-only IPC request from the owner's uid, logged) to create `/etc/soos/presence.disable` and a face-PAM disable flag until an administrator removes them. Caveat: `G302-05` (section 7).
- **PTSN-03** · LOW · new · 1/1 — All tailnet callers share one limiter, lockout and challenge pool.
  Where: `crates/remote/src/server.rs:1140`.
  Fix: Key the tailnet limiter and challenge binding per caller where possible (per login at least; with PTSN-01 fixed, per passkey session), and do not count malformed bodies of register options toward the shared unlock lockout. Optionally let the owner's local CLI reset the limiter.
- **PUID-01** · LOW (reported MEDIUM) · accepted residual (ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey Authentication for soos-remote" (local ...) · 2/2 — A transient same-uid foothold becomes a durable, Internet-reachable unlock and camera backdoor: every soos-remote trust anchor is a plain file of the desktop uid.
  Where: `crates/remote/src/credentials.rs:405`.
  Fix: Smallest safe changes first: 1. Notify on every change of the passkey set: on each store load whose credential-id set differs from the last one seen (covers direct file edits, not only `register_verify`), send a push "A new passkey was added to soos-remote on <date>" to every subscription and show ...
- **PWEB-01** · LOW · new · 1/1 — Any other web content on the companion's node name inherits its authority (other path = same origin; other port = same cookie jar), and nothing warns the owner.
  Where: `crates/remote/src/lib.rs:244`.
  Fix: Documentation and setup output: state in Docs/REMOTE_COMPANION.md §2/§8 and in the printed steps at scripts/setup.sh:3455-3459 that the node name must serve only soos-remote (no other `--set-path`, no other `--https` port), and why.
- **RALR-01** · LOW (reported MEDIUM) · accepted residual (ADR 2026-10-06 "Web Push Notifications for Failed-Password Alerts Through a Separate Sender Unit" item (1)) · 2/2 — Failed-password attempts made while soos-remote is not running (greeter after reboot, restart gaps) are never pushed.
  Where: `crates/remote/src/alerts.rs:800`.
  Fix: Persist a "pushed-through" watermark (journal time of the newest attempt already handed to the push sink, `0600` file next to `remote-alerts.json`) and, at start, treat replayed attempts newer than that watermark and newer than the acknowledgement marker as pushable once (one coalesced "attempts ...
- **RALR-03** · LOW · new · 1/1 — Wall-clock steps can silently hide replayed alerts and suppress pushes (no bound on ack marker or start time).
  Where: `crates/remote/src/alerts.rs:423`.
  Fix: In `AlertsRuntime::setup`, treat a loaded marker greater than `started_us` as invalid (marker 0, emit `alerts_ack_not_persisted`), and in `flush` never persist a marker greater than `now_us`.
- **RALR-04** · LOW · new · 1/1 — Non-canonical lock_screen_programs path reports monitored but never matches _EXE.
  Where: `crates/remote/src/journal.rs:666`.
  Fix: In `main.rs` when building `AlertSettings`, replace each configured path by `std::fs::canonicalize(path)` when it resolves to a regular file (keep the raw value only when resolution fails), and compute coverage from the canonical paths.
- **RCAM-01** · LOW · accepted residual (ADR 2026-10-07 Live Camera View item (8)) · 1/1 — Camera LED, the only at-PC sign of a remote view, already burns for presence scans.
  Where: `Docs/REMOTE_COMPANION.md:713`.
  Fix: Correct the "Awareness" paragraph in Docs/REMOTE_COMPANION.md §2f and ADR 2026-10-07 item (8). They should say that the LED does not indicate a remote view while presence scanning or face authentication keeps the camera awake. Hardening: (a) Add an on-screen indicator during a remote view.
- **RCAMB-01** · LOW · accepted residual (Docs/REMOTE_COMPANION.md:1828 residual (PID reuse "only affects this administrative classification"), plus ...) · 1/1 — Preview peer placement is read lazily from a recycled PID: an SSH-session client can get the camera after its connector PID is reused.
  Where: `crates/daemon/src/dispatcher.rs:1325`.
  Fix: Classify at accept time, before the fd can outlive its connector, and bind the result to the process identity. In `handle_connection` (dispatcher.rs:463-501) obtain a pidfd for the peer (`SO_PEERPIDFD`, Linux 6.5+, falling back to `pidfd_open(pid)` immediately after `accept`), read ...
- **RCAMB-02** · LOW · new · 1/1 — soos-remote holds raw camera frames with no swap or core-dump protection, contrary to 'nothing is written to disk'.
  Where: `packaging/soos-remote.service:5`.
  Fix: In packaging/soos-remote.service add `LimitCORE=0` and `MemorySwapMax=0` (user units get the memory controller delegated by default), and at start-up call the safe `nix::sys::prctl::set_dumpable(false)` in `crates/remote/src/main.rs`.
- **RCAMC-01** · LOW · new · 1/1 partial — Remote camera view is not limited to a locked session: a passkey holder can watch the owner while they work at the unlocked PC.
  Where: `crates/daemon/src/dispatcher.rs:1349`.
  Fix: Refuse `POST /api/camera/start` (and re-check in the view loop at the existing 5 s session-check cadence) unless the owner's selected logind session is locked: in `camera_start` (crates/remote/src/server.rs, before the body read), call `shared.fresh_read()` / `select_session` as the lock route does ...
- **RCFG-01** · LOW (reported MEDIUM) · accepted residual (ADR 2026-10-06 "Web Push Notifications for Failed-Password Alerts Through a Separate Sender Unit" item ...) · 2/2 — Internet-facing soos-remote runs with almost no sandbox: a compromise reaches the whole home, the user's groups and the session.
  Where: `packaging/soos-remote.service:12`. Merged duplicates: LIVE-01.
  Fix: Smallest change (works without user namespaces, seccomp only): add to packaging/soos-remote.service `SystemCallFilter=@system-service`, `SystemCallFilter=~@privileged @resources`, `SystemCallErrorNumber=EPERM`, `RestrictNamespaces=yes`, `ProtectClock=yes`, `ProtectKernelTunables=yes`, ... Caveat: `G302-02` (section 7).
- **RCFG-02** · LOW · accepted residual (ADR 2026-10-23 "One Command Without Manual Steps" part 1 item (3) (AI/DECISIONS.md:161)) · 1/1 — Setup never pins allowed_hosts to rp_id and drops the owner's hardening and privacy keys when it rewrites remote.toml.
  Where: `scripts/setup.sh:1469`. Merged duplicates: SETB-05.
  Fix: In `render_remote_config` (scripts/setup.sh:1469) write `allowed_hosts = ["${COMPANION_RP_ID}"]`. In `parse_user_remote_config`/`classify_remote_config`, carry over (after validation) the keys whose only effect is to narrow exposure or improve privacy: `allowed_hosts`, `push_previews`, ...
- **RCFG-03** · LOW · new · 1/1 · re-verified after the report (confirmed) — remote.toml, the authorization source, is loaded with no owner, mode or symlink check while every store is.
  Where: `crates/remote/src/config.rs:1028`.
  Fix: In `load_config_with_android` (crates/remote/src/config.rs:1028) add `O_NOFOLLOW` and refuse (as `Unreadable`, exit 78) a file whose `fstat` owner is not the process uid or whose mode has `0o022` (group/other write); also check the parent directory with the same rule as `check_parent_dir`.
- **RCFG-04** · LOW · new · 1/1 partial — install_remote.sh update leaves the old Internet-facing binary running; --uninstall leaves Serve/Funnel publishing.
  Where: `scripts/install_remote.sh:155`.
  Fix: After `daemon-reload` in `install_companion`, run `systemctl --user try-restart soos-remote.service` (and the sender user unit if active), or at least print "restart with: systemctl --user restart soos-remote" when the unit is active.
- **RPSH-01** · LOW · new · 1/1 — Push metadata discloses alert details and the PC identity to the push operator.
  Where: `crates/remote/src/webpush.rs:288`.
  Fix: Pad every plaintext to a fixed size before encryption (RFC 8188 allows padding after the `0x02` delimiter with zero bytes; RFC 8291 §4 recommends it), e.g. always to `MAX_PUSH_PLAINTEXT_BYTES` (1024) in `encrypt_with` (`crates/remote/src/webpush.rs:275-277`), so every message has the same length.
- **RPSH-02** · LOW · accepted residual (Docs/REMOTE_COMPANION.md section 2d (line 373: failed delivery retried twice, counts added to the next ...) · 1/1 — Alert counts that failed delivery are never re-sent until another attempt happens.
  Where: `crates/remote/src/push.rs:1920`.
  Fix: When a carry exists, schedule a re-send instead of waiting for a new attempt, e.g. in `retry_or_fail` (`crates/remote/src/push.rs:1920-1923`) note a synthetic pending summary in the scheduler with a later due time (for example every 5 minutes, within the hourly budget), or make `next_wake` return a ...
- **RPSH-03** · LOW · accepted residual (AI/architect_spec_remote_web_push.md W-11 (offline phone receives only newest summary)) · 1/1 — Visible notification shows only the newest summary, letting an attacker understate a burst.
  Where: `crates/remote/assets/sw.js:61`.
  Fix: Make the payload carry a running total since the last acknowledgement (or since the first unacknowledged attempt), not only the attempts since the previous send, so the newest notification always shows the full count (e.g. "16 wrong passwords since 10:02").
- **RPSHB-01** · LOW · accepted residual (ADR 2026-10-06 Web Push separate sender unit, accepted risks) · 1/1 — Push sender sandbox has no IP egress filter: a compromised sender reaches soos-remote as the owner through the PC's own tailnet address.
  Where: `packaging/soos-push-sender@.service:42`.
  Fix: Smallest change in `packaging/soos-push-sender@.service` (and the legacy unit where the user manager supports it): deny every non-public destination at the cgroup level, allowing only the resolver the sandbox binds, for example ...
- **RPSHC-01** · LOW · new · 1/1 — Running system push sender is never restarted after an update, so sender security fixes and sandbox changes do not take effect.
  Where: `crates/push-sender/src/main.rs:98`.
  Fix: After installing a sender binary or unit that differs from the previous one, `scripts/install.sh` (live install) runs `systemctl try-restart 'soos-push-sender@*.service'` (stopping is also enough: socket activation restarts it on the next delivery).
- **RUNL-01** · LOW · new · 1/1 — Remote lock has no step-up, no off switch and one global 2 s gate; shared Funnel unlock limiter/pool.
  Where: `crates/remote/src/server.rs:1382`.
  Fix: Add `allow_lock` (default true, so behaviour is unchanged) in `config.rs` and refuse `403 lock_disabled` in the `Route::Lock` arm, and document the lock loop in §8.
- **RUNL-02** · LOW · accepted residual (Docs/REMOTE_COMPANION.md section 2 ("Denials log their reason class only")) · 1/1 — Refused unlocks, lockouts and remote locks leave no audit record; unlock line has no path.
  Where: `crates/remote/src/server.rs:1777`.
  Fix: Add fixed-text WARN audit events in `audit.rs` (`remote unlock refused`, `remote unlock locked out`) emitted at server.rs:1777 and 1737, and an INFO `remote lock requested` next to the `lock_session` call at server.rs:2052.
- **RUNL-03** · LOW · new · 1/1 — Global 256-session cap lets another local account make remote status/lock/unlock unavailable.
  Where: `crates/remote/src/logind.rs:295`.
  Fix: Bound memory by the D-Bus message size (already capped by the bus) and drop the global count refusal at logind.rs:295, keeping only `MAX_OWN_SESSIONS` after the uid filter; or list only the owner's sessions (`Manager.GetUser(uid)` then the `Sessions` property of the user object).
- **RUNL-04** · LOW · new · 1/1 — Web-session idle/absolute lifetimes use CLOCK_MONOTONIC and ignore suspend time.
  Where: `crates/remote/src/websession.rs:36`.
  Fix: Measure web-session expiry with a suspend-aware clock: `nix::time::clock_gettime(ClockId::CLOCK_BOOTTIME)` behind the existing `unix_clock` seam (or the wall clock already injected in `ServerState`) for `created` and `last_used`.
- **RUNLB-01** · LOW · new · 1/1 — Remote unlock audit trail can be silenced by an inherited RUST_LOG.
  Where: `crates/remote/src/main.rs:142`.
  Fix: In `crates/remote/src/main.rs:142` build the filter from a fixed directive (e.g. `EnvFilter::new("info")`, or `info` plus an optional `SOOS_REMOTE_DEBUG=1` that only raises verbosity) so that the `soos_remote::audit` target can never be disabled.
- **RWAN-01** · LOW · new · 1/1 — A stolen Funnel session can deny every Funnel unlock and camera view through the shared FunnelSession limiter and pools.
  Where: `crates/remote/src/server.rs:1138`.
  Fix: Key the session-holder limiter and pool quota per session: replace `LimitKey::FunnelSession` by ...
- **RWEB-01** · LOW · new · 1/1 partial · re-verified after the report (partial) — Cross-site GETs can hold the 4 global event-stream slots in tailnet mode; page then shows Unreachable and disables Lock now.
  Where: `crates/remote/src/server.rs:1359`.
  Fix: In `handle_connection`, before routing, refuse any request whose single `Sec-Fetch-Site` is present and not `same-origin` or `none` (allow `none` for typed/bookmarked navigations and `same-origin` for the page), except top-level navigations to `/` and `/index.html`.
- **RWEB-02** · LOW · new · 1/1 — Sign out leaves the phone's push subscription active; the phone keeps receiving alerts.
  Where: `crates/remote/assets/app.js:1201`.
  Fix: In `requestLogout`, before posting logout, get the registration's subscription and, when present, `unsubscribe()` it and post `PUSH_UNSUBSCRIBE_PATH` (same code as `disableNotifications`), then log out.
- **XDOS-02** · LOW · new · 1/1 — Never-evicting 4-slot Funnel session table plus the Android app's memory-only cookie lets the owner lock themselves out of Funnel login.
  Where: `crates/remote/src/websession.rs:105`. Merged duplicates: RWANB-01.
  Fix: In `WebSessionStore::create` (`websession.rs:105`), when the table is full, evict the least-recently-used session **of the same `credential_hash`** instead of refusing. That makes a re-login with the same passkey always succeed. Refuse only when all 4 sessions belong to other passkeys.
- **XLOG-01** · LOW · accepted residual (ADR 2026-10-06 "Tailscale Funnel Access and In-House Passkey Authentication" accepted risks ...) · 1/1 — Funnel-exposed soos-remote answers anonymous Internet scanners with a branded, self-identifying page.
  Where: `crates/remote/src/routes.rs:395`.
  Fix: Drop the footer GitHub link and the product name from what an anonymous Funnel client receives — serve a neutral, generic shell (no title/manifest description naming soos, no repository link) until the client has a web session, and make `POST /api/auth/login/options` answer the same status for "no ...
- **G302-04** · INFO (reported LOW) · new · 1/1 partial — DPRSB-01 always-LockSession lets any /api/lock caller clear LockedHint while the PC stays locked.
  Where: `crates/remote/src/server.rs:2048`.
  Fix: Keep the `already_locked` short-circuit in `lock_flow` (or call `LockSession` only when the hint is false or stale for longer than a bound), and make the documented wrapper single-instance: take an exclusive lock first and exit without touching the hint when another wrapper or locker is running, ...
- **PPHN-03** · INFO · new · 1/1 — Push alerts have no freshness check on the phone and no end-to-end delivery confirmation.
  Where: `crates/remote/assets/sw.js:27`.
  Fix: In sw.js, drop (show the generic fallback for) a payload whose `last_unix_ms` / `last_end_unix_ms` is older than `PUSH_TTL_S` relative to the phone clock, and include the event time in the visible body ("at 14:03") so a replay is recognisable.
- **PTSN-02** · INFO (reported LOW) · accepted residual (ADR 2026-10-06 Web Push (accepted risks)) · 1/1 partial — Tailnet identity alone adds a persistent push subscription that survives documented revocation.
  Where: `crates/remote/src/server.rs:1402`.
  Fix: Add `soos-remote push list` / `push remove N` (or `push reset`) to the revocation list of section 2a and extend the section 8 sentence to "a stolen Funnel session or any device with an allowed tailnet identity can register a device".
- **RCAMB-03** · INFO · new · 1/1 — Camera view docs say the installer never changes daemon.toml and both sides are off by default; setup writes [preview] remote_view = true by default.
  Where: `Docs/REMOTE_COMPANION.md:758`.
  Fix: Rewrite Docs/REMOTE_COMPANION.md:758 to "Manual setup (if you did not use `scripts/setup.sh`)" and state at :636 that `scripts/setup.sh` enables both sides by default and how to opt out (`--camera-view off`).
- **RUNLB-02** · INFO · new · 1/1 — An unlock that logind executed can be reported to the phone as 'logind is unavailable'.
  Where: `crates/remote/src/server.rs:2100`.
  Fix: In `unlock_flow` (server.rs:2100-2114) distinguish "call sent, outcome unknown" (a `SourceError::Timeout` from `unlock_session`, or the flow deadline after the gate was recorded) from a refusal, and answer e.g. `202 {"result":"unlock_unconfirmed"}` or `504 unlock_unconfirmed`.
- **RUNLC-01** · INFO · new · 1/1 — Remote lock is skipped whenever LockedHint says locked, a value any process of the session owner can set.
  Where: `crates/remote/src/server.rs:2048`.
  Fix: Do not short-circuit on `LockedHint` in `lock_flow`. Always call `Manager.LockSession` on the selected session (a `Lock` signal to an already-locked locker is harmless).

### PAM module and PAM stacks (14: 2 MEDIUM, 8 LOW, 4 INFO)

- **PAM-01** · MEDIUM · new · 2/2 partial — Conversation callback runs while an unsent daemon connection is held; any local user can pin all root-peer permits via setuid su/sudo.
  Where: `crates/pam/src/ipc.rs:514`.
  Fix: In `authenticate_before_with_progress` (crates/pam/src/ipc.rs:513-534) generate the nonce, encode and write the complete request BEFORE calling `on_connected`, so the daemon receives the request immediately and an application conversation can no longer hold an idle connection.
- **PSTK-01** · MEDIUM · new · 2/2 — success=done face rule ends the whole stack and skips second factors/gates placed after the include.
  Where: `packaging/pam/debian/soos:7`.
  Fix: In `soos-admin gdm enable` (gdm.rs `plan_gdm_enable`), refuse when any `required`/`requisite` auth rule follows the anchor in the edited file, and document in Docs/DISTRIBUTION_DEPLOYMENT.md §2/§3.2/§4 that a face match skips every auth rule after the include. Caveat: `G302-03` (section 7).
- **PAM-02** · LOW · new · 1/1 · re-verified after the report (confirmed) — Administrator kill switches fail open in PAM on stat errors and dangling symlinks.
  Where: `crates/pam/src/config.rs:120`.
  Fix: Replace `.exists()` in `is_disabled` (crates/pam/src/config.rs:114-147) with a helper identical to the daemon's: `match std::fs::symlink_metadata(p) { Ok(_) => true, Err(e) => e.kind() != ErrorKind::NotFound }` so any undeterminable state disables face (fail closed, `PAM_IGNORE`).
- **PAM-03** · LOW · new · 1/1 partial — Per-service disable flag matches exact PAM_SERVICE; aliases sudo-i, su-l, runuser-l stay face-enabled.
  Where: `crates/pam/src/config.rs:137`. Merged duplicates: G202-05.
  Fix: Document the alias names next to every mention of `<service>.disable` (Docs/PAM_MODULE.md §7, onboarding text, ADR follow-up) and add a `soos-admin` subcommand that writes the whole family (`sudo` + `sudo-i`, `su` + `su-l`, `runuser` + `runuser-l`).
- **PAM-04** · LOW · new · 1/1 · re-verified after the report (confirmed) — PAM deadline and response freshness ignore suspend time; Allow issued just before suspend is honoured after resume.
  Where: `crates/pam/src/ipc.rs:229`.
  Fix: In `ExchangeDeadline` also record `CLOCK_BOOTTIME` at start and, before honouring a verdict (ipc.rs:594), reject when `CLOCK_BOOTTIME` elapsed exceeds `timeout_ms` (or differs from the monotonic elapsed by more than a small tolerance), returning `IpcError::Timeout` → `PAM_IGNORE`.
- **PSTK-02** · LOW (reported MEDIUM) · new · 2/2 — Fedora custom/soos is local-only; printed --force activation drops SSSD/Winbind identity and policy features.
  Where: `scripts/install.sh:1099`.
  Fix: In install.sh and `%post`, print the activation hint only when the recorded profile is `local` (or `minimal`) and otherwise warn that custom/soos is local-only; document the same restriction in Docs §4.1/§4.3 and drop the unconditional `--force`. Deeper: ship an sssd-derived variant of the profile.
- **PSTK-03** · LOW · new · 1/1 — uninstall.sh restores every *.soos-backup blindly, discarding later PAM changes.
  Where: `scripts/uninstall.sh:339`.
  Fix: In uninstall.sh, restore a backup only when the current file with its `pam_soos.so` lines (and managed block) removed equals the backup; otherwise keep it, warn, and set `PAM_ROLLBACK_INCOMPLETE=true` (for gdm-password simply call `soos-admin gdm restore` without `--force`).
- **PSTK-04** · LOW · accepted residual (Docs/PAM_MODULE.md Local session binding) · 1/1 — Face rule inherited by remote-login stacks; daemon session binding is the only guard.
  Where: `packaging/pam/arch/system-auth:6`.
  Fix: Have pam_soos return PAM_IGNORE without contacting the daemon when `PAM_RHOST` is set to a non-local value (and for a fixed denylist of network services: sshd, remote, vsftpd...), or add `auth [default=1 success=ignore] pam_succeed_if.so service notin sshd:remote quiet` before the rule in the ...
- **PSTKB-01** · LOW · new · 1/1 partial · re-verified after the report (confirmed) — Face authentication ignores a locked password (passwd -l / usermod -L); presence unlock refuses the same account.
  Where: `packaging/pam/arch/system-auth:6`.
  Fix: In the daemon `Auth` path, before running the pipeline for a root or unprivileged peer, apply the shadow part of the presence account guard (`shadow_refusal`, at least `PasswordLocked`) and return `Deny` (so PAM gets `PAM_IGNORE` and falls back to the locked password).
- **PSTKC-01** · LOW · new · 1/1 — soos-admin gdm disable/status trust --disable-file and --pam-file, not the flag the module reads: a DM stack shows Disabled while face login stays active.
  Where: `crates/admin-cli/src/gdm.rs:115`.
  Fix: Smallest change, in crates/admin-cli/src/gdm.rs (`configure_gdm_with_options` and `get_gdm_status`): derive the flag from what the module reads instead of trusting `--disable-file`. Refuse `--disable-file` unless it equals `/etc/soos/gdm.disable` or `/etc/soos/<basename of --pam-file>.disable`.
- **G305-01** · INFO · new · 1/1 — Face success never hands a secret to homed/ecryptfs/gnome-keyring; homed account checks fall back to pam_unix; undocumented.
  Where: `packaging/pam/fedora/soos/system-auth:9`.
  Fix: Document in packaging/pam/fedora/soos/README (features with-systemd-homed, with-ecryptfs, with-pam-gnome-keyring) and Docs/DISTRIBUTION_DEPLOYMENT.md §4/§5.2/§3 that face login never supplies the password: homed homes are not activated/resumed, ecryptfs is not mounted, keyrings stay locked.
- **PAM-05** · INFO · new · 1/1 — Module ignores PAM_RHOST/PAM_TTY and shows face status messages to unauthenticated remote clients.
  Where: `crates/pam/src/lib.rs:63`. Merged duplicates: PAMB-01.
  Fix: In `authenticate_flow`, read `PAM_RHOST` (and treat a non-empty value other than `localhost`/empty, or a `PAM_TTY` of `ssh`) as remote and return `PAM_IGNORE` silently before contacting the daemon; keep the daemon session policy as the authoritative check.
- **PSTK-05** · INFO · new · 1/1 — ARCHITECTURE §5 universal stack shows [success=done default=bad] pam_unix.so.
  Where: `AI/ARCHITECTURE.md:155`.
  Fix: Replace the line with "the distribution's password rule, unchanged (success jump widened by one)" as in Docs §2.
- **PSTKB-02** · INFO · new · 1/1 — Docs/PAM_MODULE.md §5 repeats the [success=done default=bad] pam_unix.so example with no faillock accounting.
  Where: `Docs/PAM_MODULE.md:107`.
  Fix: Replace the example with the Docs/DISTRIBUTION_DEPLOYMENT.md §2 wording: the distribution's password rule unchanged, its success jump widened by one, the event line before faillock `authfail`/`pam_deny`. Alternatively, link to the per-distribution stacks instead of showing a generic one.

### Daemon, policy and IPC (27: 5 MEDIUM, 18 LOW, 4 INFO)

- **DDEC-01** · MEDIUM · accepted residual (AI/DECISIONS.md "[2026-10-02] Presence Auto-Unlock Through logind" item (8) and Accepted risks (b)) · 2/2 — Face attempt budget never escalates: ~43,000 hands-free PAD/match trials per day at a locked machine, reset by daemon restart.
  Where: `crates/policy/src/rate_limit.rs:21`.
  Fix: Add a cumulative failure counter per UID in `soos_policy::RateLimiter` (or a sibling `LockoutTracker` in `crates/policy/src/rate_limit.rs`) that counts non-Allow outcomes across windows and, after N consecutive failures (e.g. 10) or any M spoof vetoes (e.g. Caveat: `G202-01` (section 7).
- **DIPC-04** · MEDIUM · accepted residual (ADR 2026-09-30 "Local Session Binding for Facial Auth" (#160), amendment "Residual risk accepted" ...) · 2/2 — Same-user code gets sudo approved silently by the face of the person at the desk (no intent signal).
  Where: `crates/daemon/src/dispatcher.rs:831`. Merged duplicates: DDEC-02.
  Fix: Document the difference honestly in Docs/PAM_MODULE.md and setup output, and make setup offer (default on) a per-service scope that excludes `sudo`/`su`/`polkit-1` from face unless the operator opts in (the per-service disable flag mechanism already exists in `crates/pam/src/config.rs:108-137`).
- **DIPCC-01** · MEDIUM · new · 2/2 — Per-UID accounting assumes one account = one kernel UID: a soos member's subordinate UIDs keep the group and defeat the per-UID connection cap and the event quota.
  Where: `crates/daemon/src/limits.rs:236`.
  Fix: Stop treating every kernel UID as a separate account for unprivileged admission. In `PeerConnectionLimiter::try_acquire` (crates/daemon/src/limits.rs:232), let a non-root UID use the per-UID regular permits only when it owns at least one logind session (the `SessionValidator` scan already exists.
- **DPRSB-01** · MEDIUM · new · 2/2 — Documented wlroots unlock hook kills the swaylock-presence wrapper, leaving LockedHint stuck true after presence/remote unlock (phone shows Locked, remote lock refused).
  Where: `Docs/DISTRIBUTION_DEPLOYMENT.md:724`.
  Fix: Smallest safe change, in the documentation, the setup hint and the physical test: - Anchor the hook to the locker process: `unlock 'pkill -x -USR1 swaylock'` (and `-x swaylock-plugin` or `-x hyprlock`). - Make the wrapper clear the hint on every exit path: ... Caveat: `G302-04` (section 7).
- **VPAD-01** · MEDIUM · new · 2/2 — PAM response latency tells a PAD rejection apart from a non-match (presentation-attack oracle).
  Where: `crates/daemon/src/consensus.rs:348`. Merged duplicates: LOCK-01.
  Fix: In the Auth path of `crates/daemon/src/dispatcher.rs` (after `run_face_consensus`), hold every non-Allow verdict until the same instant a full non-matching run would end (request deadline, or `not_before + DECISION_BUDGET_MS`), exactly like the lock widget's reveal floor ... Caveat: `G202-03` (section 7).
- **DIPC-01** · LOW (reported MEDIUM) · accepted residual (ADR [2026-09-30] Local Session Binding for Facial Auth, amendment residual risk ("the same holds for a ...) · 2/2 partial — Root Auth from a local session scope ignores the target's live SSH sessions (tmux/screen bridge to root).
  Where: `crates/daemon/src/session_policy.rs:600`.
  Fix: Apply the same "no remote session of the target" rule to the session-scope branch: in `authorize_root_peer`, after `record.check_local_seat_session_of(target_uid)` succeeds, run the remote-session part of `check_user_manager_records(source, target_uid)` (any record with `uid == target` or no UID ...
- **DIPC-02** · LOW · accepted residual (ADR 2026-09-30 "Local Session Binding for Facial `Auth`" item (2)) · 1/1 — Unprivileged same-UID Auth ignores where the peer runs: remote/background process can light the camera, probe presence and drain the attempt quota.
  Where: `crates/daemon/src/session_policy.rs:694`.
  Fix: Classify the unprivileged peer like the preview path does: read its cgroup once (`classify_peer_placement`) and apply `authorize_preview_placement`-style rules to `Auth` (session scope must be a local seat session of the UID with no remote session of the UID — see DIPC-01.
- **DIPC-03** · LOW · new · 1/1 — PasswordFailed events: unprivileged self-reports trigger evidence photos without session check; all root peers share one event quota.
  Where: `crates/daemon/src/dispatcher.rs:680`.
  Fix: Key the event quota on `(peer_uid, target_uid)` (or on target UID for root peers) so one root service cannot starve another, and for unprivileged peers require that the UID owns a local seat session (`session_validator.has_local_seat_session(peer_uid)`) before capturing.
- **DIPCB-01** · LOW · new · 1/1 · re-verified after the report (confirmed) — Daemon-identity trust rests on an unchecked directory mode: group-writable /run/soos accepted, root-owner check switchable in production, PAM never authenticates the lis….
  Where: `crates/daemon/src/socket.rs:80`.
  Fix: In `open_and_validate_directory` (crates/daemon/src/socket.rs:80) reject any directory with `mode & 0o022 != 0` (group or other write) unless it carries the sticky bit, and refuse `enforce_root_owner = false` in `DaemonConfig::validate` unless `use_mock_camera = true`, exactly like ...
- **DPRS-01** · LOW (reported MEDIUM) · accepted residual (AI/DECISIONS.md:117 ADR "Presence Auto-Unlock Through logind", accepted risk (b) and decision (9)) · 2/2 partial — Unattended presence attack is unbounded in time and almost silent (no cap, no evidence, one warning per lock period).
  Where: `crates/daemon/src/presence/worker.rs:767`.
  Fix: Smallest change (worker.rs, scan outcome handling): count consecutive non-unlocking scans per lock period in `LockEntry` and, after N (e.g. 10) `SpoofVetoed`/`NoMatch` outcomes in one lock period, stop scanning that period (`locker_ignored`-like flag, password required) and emit one `warn!`.
- **DPRS-02** · LOW · accepted residual (AI/DECISIONS.md [2026-10-02] Presence Auto-Unlock Through logind (#323), accepted risks (a) default-on and ...) · 1/1 — Presence unlocks on mere visibility of the owner (no intent or absence signal), default-on for every enrolled user.
  Where: `crates/daemon/src/presence/config.rs:8`.
  Fix: Add a per-lock-period "absence seen" arm in `LockEntry` (crates/daemon/src/presence/tracker.rs): a session becomes scannable only after at least one capture with zero faces (or after the grace) whichever is later, and raise `DEFAULT_LOCK_GRACE_MS` (e.g.
- **DPRSC-01** · LOW (reported MEDIUM) · accepted residual (AI/DECISIONS.md:117 ADR 2026-10-02 "Presence Auto-Unlock Through logind", accepted risk (d) and item (11) ...) · 2/2 — Presence ignores the lock screen's own authentication policy: a configured second factor (pam_u2f 2FA) or a per-service face opt-out is silently replaced by face-only un….
  Where: `crates/daemon/src/presence/account.rs:865`.
  Fix: The account guard treats a second-factor auth module in a PAM stack as `Undeterminable`, which refuses presence for everyone (fail closed, same mechanism as the faillock option scan).
- **DRUN-01** · LOW · new · 1/1 — The daemon's systemd sandbox does not contain a compromised daemon: UID 0 can still reach systemd over the system bus and its private sockets.
  Where: `packaging/soos-daemon.service:31`.
  Fix: Add `InaccessiblePaths=-/run/systemd/private -/run/systemd/io.systemd.Manager -/run/systemd/userdb` to `packaging/soos-daemon.service`. More generally, hide every root varlink or manager socket the daemon does not use.
- **DRUN-02** · LOW · new · 1/1 — Core dumps are not suppressed (no LimitCORE=0, no PR_SET_DUMPABLE): a crash writes keys, templates and frames to disk.
  Where: `packaging/soos-daemon.service:28`.
  Fix: Add `LimitCORE=0` to `packaging/soos-daemon.service`. Also call `prctl(PR_SET_DUMPABLE, 0)` at the start of `main()`, next to the `mlockall` call in `crates/daemon/src/mlock.rs` (the unsafe code is already confined there), so a manual run is covered too.
- **DRUN-03** · LOW · new · 1/1 — Unthrottled per-connection warn logging lets any soos-group process trigger journald suppression of the daemon's security log.
  Where: `crates/daemon/src/shutdown.rs:299`.
  Fix: Rate-limit the noisy per-connection rejection logs. Count rejections per peer UID and log at most one summary line per window (for example "N connections from uid X rejected in the last 10 s"), or move them to `debug`, reusing the existing `event_limiter` pattern.
- **DRUN-04** · LOW · accepted residual (Docs/DAEMON.md "Unknown keys are ignored silently") · 1/1 — Unknown daemon.toml keys are silently accepted: a misspelt opt-out of default-on presence unlock leaves it on, and --check-config reports valid.
  Where: `crates/daemon/src/config.rs:340`.
  Fix: In `DaemonConfig::from_toml_str` (crates/daemon/src/config.rs), parse into `toml::Table` first, compare the keys against the known schema, and push one warning per unknown key, naming the key and never the value, as `ignored_camera_keys` already does for `[camera]`.
- **EVID-01** · LOW (reported MEDIUM) · new · 2/2 — Evidence retention is never enforced unless a new snapshot is written.
  Where: `crates/daemon/src/dispatcher.rs:187`.
  Fix: Call `evidence_store.rotate_retention(&today)` once at daemon startup (next to `sweep_orphaned_store_temp_files` in `crates/daemon/src/pipeline.rs`) and from a periodic (e.g.
- **EVID-02** · LOW · new · 1/1 — PasswordFailed evidence ignores the PAM service: root-peer failures share one quota and are misattributed.
  Where: `crates/daemon/src/dispatcher.rs:673`.
  Fix: In `handle_event`, ignore (do not capture for) events whose `event.service` is a remote-login service or whose peer has no local seat session (reuse `session_policy` as for `Auth`), and key the event quota by `(peer_uid, target_uid)` (or by target UID for root peers) so one target's flood cannot ...
- **EVID-03** · LOW · new · 1/1 — PasswordFailed evidence only captured while the camera is still warm; intruder evades by waiting ~10 s.
  Where: `crates/daemon/src/dispatcher.rs:699`.
  Fix: On an accepted `PasswordFailed` event from a local seat session, call `pipe.camera.notify_activity()` and wait a short bounded time (on the blocking pool, never on the PAM path, which is already fire-and-forget) for a fresh frame before sealing, keeping the existing quota and caps.
- **PROT-01** · LOW · new · 1/1 — Detailed ReasonClass (PadFailed vs ScoreBelowThreshold) reaches any same-UID peer and is printed by soos-admin test-pam.
  Where: `crates/protocol/src/types.rs:184`.
  Fix: In `crates/daemon/src/dispatcher.rs` `build_response`, collapse `reason_class` for non-root peers before encoding. Map every `Deny` reason to one value (for example `ScoreBelowThreshold`) and every `Unavailable` reason to `InternalError`. Keep the detailed reason in the daemon log only.
- **VPADB-01** · LOW (reported MEDIUM) · new · 2/2 — PAM text "Face verification unavailable." reveals that the last capture passed PAD and matched the owner (near-success oracle for spoof tuning).
  Where: `crates/policy/src/pad_consensus.rs:192`.
  Fix: Smallest change (no policy test touched): in `crates/daemon/src/dispatcher.rs` step 8f, render `ConsensusDecision::Pending(Some(FrameClass::Passing))` as `(Verdict::Deny, ReasonClass::ScoreBelowThreshold)` whenever `frames_evaluated > 0`, so every evaluated-but-not-authorized outcome is one `Deny`.
- **XDOS-01** · LOW · new · 1/1 · re-verified after the report (confirmed) — Another local account can switch off lock-screen face unlock, presence, terminal face sudo and the remote camera by holding about 1024 logind sessions.
  Where: `crates/daemon/src/session_policy.rs:505`.
  Fix: Stop scanning other users' sessions. Read `/run/systemd/users/<target_uid>` (its `SESSIONS=` line lists only that user's sessions; bound it with `MAX_SESSION_RECORD_SIZE`), then read only those `/run/systemd/sessions/<id>` records.
- **XFSR-01** · LOW · new · 1/1 · re-verified after the report (confirmed) — Presence faillock guard reads the tally through the daemon's private mount view and treats an invisible tally directory as no failures.
  Where: `crates/daemon/src/presence/account.rs:817`.
  Fix: In `tally_dir_is_safe`, return `Err(AccountRefusal::Undeterminable)` when the configured `dir` lies under a path the unit hides or privatises (`/tmp`, `/var/tmp`, `/home`, `/root`, `/run/user`). Alternatively, fail closed on `NotFound` for any `dir` that is not the default `/run/faillock`.
- **DRUN-05** · INFO · new · 1/1 — Rate-limit keys have no security ceiling: max_attempts up to u32::MAX and max_tracked_uids = 1 are accepted silently.
  Where: `crates/daemon/src/config.rs:229`.
  Fix: In `PipelineConfig::validate` (crates/daemon/src/config.rs), reject `max_attempts` above a documented ceiling (for example 120 per 60 s) or emit a startup warning for it, and also refuse or warn when `max_attempts / window` exceeds the default rate.
- **G202-03** · INFO (reported LOW) · new · 1/1 partial — VPAD-01 fix variant 'not_before + DECISION_BUDGET_MS' leaves the PAD timing oracle on GDM (timeout_ms=2500).
  Where: `crates/daemon/src/dispatcher.rs:906`.
  Fix: Use only the request deadline as the floor: after `run_face_consensus`, for every non-Allow verdict, sleep `deadline.remaining(now)` (the same `RequestDeadline` that bounds the non-match run, both bounds included) before building the response.
- **G303-01** · INFO · accepted residual (AI/DECISIONS.md ADR 2026-10-02 "Decaying Inference Estimate, Session Policy Locked to the Mock Camera..." ...) · 1/1 — use_mock_camera + enforce_active_session=false is a deny-all, not an allow-all, but the production daemon accepts it silently (prior SAFE rationale corrected).
  Where: `crates/daemon/src/config.rs:521`.
  Fix: In `crates/daemon/src/config.rs`, where `pipe.use_mock_camera` is applied (line 673), push a `config.warnings` entry ("[pipeline] use_mock_camera = true: no real camera.
- **XUNS-01** · INFO · new · 1/1 — Root daemon library holds unsafe code outside the forbid(unsafe_code) invariant, contrary to the documented policy.
  Where: `crates/daemon/src/lib.rs:3`.
  Fix: Move the four libc calls into a tiny adapter (for example `camera-v4l`-style documented module or the `nix` crate's `mlockall`, which is already a workspace dependency of `soos-evidence-store`) and add `#![forbid(unsafe_code)]` to `crates/daemon/src/lib.rs`.

### Camera, vision, PAD and models (12: 11 LOW, 1 INFO)

- **CAMV-01** · LOW (reported MEDIUM) · tracked (#353 (strict IR mode split item 4, IRA28 pending)) · 2/2 partial — Auto-selected camera silently fails over to any other capture node (IR to RGB downgrade, rogue USB/UVC-gadget adoption).
  Where: `crates/camera-v4l/src/v4l_impl.rs:539`.
  Fix: After the first successful auto-selection, pin the selection instead of re-running the full auto-selection on loss. Caveat: `G302-01` (section 7).
- **CAMV-02** · LOW · new · 1/1 — Frames timestamped at publication, driver capture timestamp and sequence discarded.
  Where: `crates/camera-v4l/src/capture.rs:745`.
  Fix: Carry the driver timestamp into `BufferMeta` (`meta.timestamp`), require `V4L2_BUF_FLAG_TIMESTAMP_MONOTONIC`, and stamp `Frame::timestamp_mono_ns` with it (falling back to rejecting the buffer when the flag is absent or the stamp is in the future/older than the stream start).
- **DDECB-01** · LOW · new · 1/1 — Liveness and identity are not bound to the same pixels: PAD scores the box crop while the embedding comes from keypoints never checked to lie inside that box.
  Where: `crates/vision/src/pipeline.rs:588`.
  Fix: In `VisionPipeline::process_frame` (crates/vision/src/pipeline.rs, between lines 574 and 578), reject the capture (map to `VisionError::AlignmentFailed`, which the consensus already treats as a fail-closed abort, or better a new quality error mapped to `no_face`) unless all five keypoints lie ...
- **G203-01** · LOW · new · 1/1 — A face consensus in flight at system suspend can finish after resume on pre-suspend frames (V4L2 done buffers survive suspend and are restamped as fresh; the consensus d….
  Where: `crates/camera-v4l/src/capture.rs:745`.
  Fix: Make each consensus run suspend-aware. Record `CLOCK_BOOTTIME` when `run_face_consensus` starts and, before every capture is recorded and before `Allow` is returned, refuse (finalize as `Pending`) when the boot-clock elapsed time exceeds the monotonic elapsed time by more than a small tolerance ...
- **G301-01** · LOW · new · 1/1 — Daemon never sets or verifies camera controls or IR emitter state; another seat account can persistently darken the IR stream and retune the exposure the IR gate relies ….
  Where: `crates/camera-v4l/src/v4l_impl.rs:843`.
  Fix: In `open_and_stream` (crates/camera-v4l/src/v4l_impl.rs, right after `apply_format`), apply a fixed control baseline for the IR node (auto-exposure on or the vendor default via `V4L2_CTRL_FLAG`/`G_EXT_CTRLS` default values, gain default) and, when `[camera.ir_emitter]` is configured, write the ...
- **INFR-02** · LOW · new · 1/1 — Third-party ONNX provenance check compares weights and sampled outputs only, not the graph; SCRFD unchecked.
  Where: `scripts/compare_pad_models.py:103`.
  Fix: Extend `compare_pad_models.py` to compare the graph structurally (node count, op types, input/output edges in topological order, and every `Constant` node attribute and `initializer` by name), and fail on any difference; correct the README/ADR wording until it does.
- **INFR-03** · LOW · new · 1/1 partial · re-verified after the report (confirmed) — Runtime model attestation is anchored in the mutable manifest.toml beside the models, parsed without schema checks.
  Where: `crates/inference-ort/src/registry.rs:70`.
  Fix: Embed the attested manifest at build time (`include_str!("../../../models/manifest.toml")` in `soos-inference-ort` or the daemon) and require the on-disk manifest to be byte-identical (or ignore it and use the embedded one), so model hashes are bound to the signed/packaged binary.
- **PLOC-02** · LOW · new · 1/1 — Daemon 'sole owner of /dev/video*' is not enforced: another seat user keeps a camera descriptor across a user switch to watch the owner or starve face auth.
  Where: `crates/camera-v4l/src/capture.rs:694`.
  Fix: Correct AI/ARCHITECTURE.md and AGENTS.md to "exclusive while streaming; other seat users and `video` members can open the camera", and document it in Docs/DAEMON.md.
- **VPAD-02** · LOW · new · 1/1 — No upper face-size / minimum effective PAD scale gate: close-up presentations strip the 2.7x context.
  Where: `crates/vision/src/crop.rs:268`.
  Fix: In `VisionPipeline::fused_pad_result` (crates/vision/src/pipeline.rs) compute the primary `pad_crop_window` and reject with a new quality error (mapped like `FaceTooSmall`, i.e.
- **VPAD-03** · LOW · new · 1/1 — PAD modality is metadata-only: IR content from a node classified RGB/Unknown takes the colour PAD path.
  Where: `crates/vision/src/ir_liveness.rs:88`.
  Fix: Make `convert_to_rgb_cow` also return a content flag (MJPEG `L8`, or YUYV/NV12 whose chroma is neutral within +-2 on all sampled pixels) and have `PadInputModality::for_frame` (or `process_frame`) treat such frames as `Monochrome` (fail closed toward the stricter policy).
- **XLOG-04** · LOW · new · 1/1 — Raw camera frames carry a derived Debug that prints every pixel byte (Frame, PreviewResponse).
  Where: `crates/frame/src/frame.rs:48`.
  Fix: Replace the derives with hand-written `Debug` impls for `Frame` and `PreviewResponse` that print width, height, format, sequence and `data_len` only (as `EvidenceFrame` does). Tests using `assert_eq!` keep working because `PartialEq` is unchanged.
- **G302-01** · INFO (reported MEDIUM) · accepted residual (AI/ARCHITECTURE.md:36 Threat Actors Considered (hardware camera replacement out of scope)) · 2/2 partial — CAMV-01's pin on sensor type and by-id stem uses only attacker-controlled USB strings; a cloned UVC gadget passes it.
  Where: `crates/camera-v4l/src/resolver.rs:113`.
  Fix: Pin on properties that a plugged-in device cannot choose. At first selection record the sysfs device path of the node's USB interface (the `/dev/v4l/by-path/` topology: bus and port chain), and at every open and reopen (not only on re-resolution) compare the opened node's sysfs path and ...

### Biometric and evidence stores (4: 1 MEDIUM, 1 LOW, 2 INFO)

- **BSTO-01** · MEDIUM · new · 2/2 — Templates keyed only by numeric UID: a deleted account's face is inherited by the next account reusing the UID.
  Where: `crates/biometric-store/src/store.rs:212`.
  Fix: Bind the template to the account identity at enrollment and verify it at use. Store the username (and, where available, a stable account marker such as the home directory inode or the `/etc/shadow` last-change field) in `BiometricTemplate`/`TemplateWire` and in the AAD. Caveat: `G202-04` (section 7).
- **PPHY-03** · LOW · new · 1/1 — Opt-in intruder evidence keeps only the first 10 snapshots per user per day; a covered lens burns them before the real attempt.
  Where: `crates/evidence-store/src/store.rs:525`.
  Fix: When the per-UID cap is reached, keep the most recent snapshots instead of the first ones (evict the oldest of the day, bounded by the same cap), and skip or down-rank frames with no detected face or near-zero luminance so they do not consume the quota.
- **BSTO-02** · INFO (reported LOW) · accepted residual (AI/ARCHITECTURE.md Threat Actors Considered (root attackers out of scope)) · 1/1 partial — Template creation and replacement leave no audit record and notify nobody.
  Where: `crates/biometric-store/src/store.rs:252`. Merged duplicates: ENRL-05.
  Fix: In `soos-enroll` (enroll, import, delete) and in `BiometricStore::write_template`/`delete`, emit one value-free journal record (`uid`, operation, `replaced_existing`, invoking `SUDO_UID`/`PKEXEC_UID`, timestamp) with a stable `MESSAGE_ID` so admins and soos-remote can alert on it.
- **XCRY-01** · INFO · accepted residual (AI/DECISIONS.md ADR 2026-09-30 "Biometric Template Erasure Model" (#179)) · 1/1 partial — No rekey path for master.key / evidence.key: per-user crypto-erasure is impossible and a leaked key stays valid for every future template.
  Where: `crates/biometric-store/src/crypto.rs:74`.
  Fix: Document in `Docs/ENROLLMENT_CLI.md` and the `delete` output that per-user crypto-erasure is not available while other users are enrolled.

### Enrollment CLI, admin CLI and GUI (16: 1 MEDIUM, 6 LOW, 9 INFO)

- **GUI-01** · MEDIUM · new · 2/2 — Guided enrollment binds the face to a free-typed UID unrelated to the Username field (root session defaults to UID 0).
  Where: `crates/gui/src/app.rs:1483`.
  Fix: Smallest safe change in crates/gui/src/app.rs (enrollment card and Save handler): make the username the only input, resolve it with `nix::unistd::User::from_name` at Save time, and use that UID; show the resolved `name (UID n)` read-only. Refuse Save when the name does not resolve.
- **ADMN-02** · LOW · new · 1/1 — logs redaction misses common secret shapes and passes control characters through.
  Where: `crates/admin-cli/src/redact.rs:128`.
  Fix: In `redact_kv_field` (redact.rs:128): - treat a key as matching when it ends a word made of `[A-Za-z0-9_]`, so that `access_token` and `client_secret` match; - let the delimiter scan skip one closing quote (`"` or `'`) before `:` / `=`; - loop `redact_bearer_token` over all occurrences.
- **ENRL-01** · LOW · new · 1/1 · re-verified after the report (confirmed) — soos-enroll --mock writes synthetic templates into the production biometric store.
  Where: `crates/enrollment-cli/src/service.rs:1822`. Merged duplicates: INFR-01.
  Fix: Mirror the GUI fix: in `args.rs` make `--mock` require explicit `--biometrics-dir` and `--key-file` (clap `requires_all`), or refuse in `build_full_service_with_notes` when `cli.mock` and either path resolves to the production default. Deeper: record a distinct model id (e.g.
- **ENRL-02** · LOW · accepted residual (ADR 2026-09-30 "Fresh Enrollment Candidates and PAD Rejections as Invalid Candidates" (#228)) · 1/1 partial — CLI enrollment and verify use weaker, non-configured liveness than the daemon and the GUI.
  Where: `crates/enrollment-cli/src/service.rs:1850`.
  Fix: In `build_full_service_with_notes`, load the validated `[pipeline.thresholds]` from the same `daemon.toml` read (the daemon's `ThresholdConfig::build_with_security_floor`) and build `VisionPipelineConfig` from them; print the applied thresholds.
- **ENRL-03** · LOW · new · 1/1 — debug-vision --output follows intermediate symlinks and races on the parent check.
  Where: `crates/enrollment-cli/src/service.rs:1683`.
  Fix: Open the parent with `O_PATH | O_DIRECTORY | O_NOFOLLOW` component by component (or `openat2` with `RESOLVE_NO_SYMLINKS` where available), check owner root and no group/other write on the pinned descriptor, then `openat(dirfd, name, O_CREAT|O_EXCL|O_NOFOLLOW, 0600)`.
- **GUI-03** · LOW · accepted residual (Docs/REMOTE_COMPANION.md setup step 2 (lines 220-223: allow_funnel = false -> Funnel requests 403)) · 1/1 — Internet access Off only sets allow_funnel = false; public Funnel listener still feeds the pre-auth HTTP parser.
  Where: `crates/gui/src/onboarding.rs:153`.
  Fix: Change the off text (crates/gui/src/onboarding.rs:153) to "Funnel requests are refused with 403; to stop the public listener run `tailscale funnel --https=443 off`", and show that command as a copyable block in the Off route (as is already done for root-only changes).
- **XSUB-01** · LOW · new · 1/1 — Root CLI soos-admin resolves usermod, journalctl and systemctl through the inherited PATH.
  Where: `crates/admin-cli/src/user.rs:73`.
  Fix: In `crates/admin-cli`, spawn absolute paths (`/usr/bin/journalctl`, `/usr/bin/systemctl`, `/usr/sbin/usermod` with `/usr/bin/usermod` fallback on merged-/usr) as constants, as `privileged.rs` does, and call `env_clear()` plus `env("LC_ALL","C")` on each child.
- **ADMN-01** · INFO (reported LOW) · new · 1/1 — Network lock gap list omits the cheapest ways to take the companion offline.
  Where: `Docs/NETWORK_LOCK.md:9`.
  Fix: Add to `Docs/NETWORK_LOCK.md` §1 and §6 (and to `Docs/REMOTE_COMPANION.md` §8) the following: - the companion and push sender run as your user, so anyone at your unlocked session can stop them, and the lock does not prevent that.
- **ADMN-03** · INFO · new · 1/1 — test-pam is presented as a simulation but is a real authentication attempt.
  Where: `crates/admin-cli/src/test_pam.rs:197`.
  Fix: Reword the README line and the clap help to "performs a real face authentication attempt against the running daemon (counts toward the rate limit, may record evidence)". Optionally print that note on stderr before the request.
- **BSTO-03** · INFO · accepted residual (ADR 2026-09-30 "Biometric Template Erasure Model" (AI/DECISIONS.md:33)) · 1/1 — Encryption at rest gives no protection against disk or backup holders since master.key sits next to templates; docs and delete message overstate it.
  Where: `crates/enrollment-cli/src/main.rs:183`.
  Fix: Reword the delete message (e.g. "residual copies remain decryptable while master.key exists; destroy the key or use full-disk encryption for a hard guarantee") and add to Docs/BIOMETRIC_STORE_CRATE.md §1 and Docs/PACKAGING_AND_PROVISIONING.md a "what encryption at rest does not protect" paragraph: ...
- **ENRL-04** · INFO · new · 1/1 — Template metadata strings unvalidated and printed raw to the root terminal by list.
  Where: `crates/enrollment-cli/src/main.rs:202`.
  Fix: Validate `model_version` (and `model_id`) at `BiometricTemplate::new`/`from_cbor`: printable ASCII, bounded length (e.g. 64). In `main.rs` pass the table fields through `soos_camera_v4l::diagnostics::sanitize_display_text`.
- **GUI-04** · INFO (reported LOW) · new · 1/1 partial — Remote feature states read from a guessed remote.toml path, not confirmed against the running soos-remote.
  Where: `crates/gui/src/feature_control.rs:814`.
  Fix: Have soos-remote expose its effective switches (for example in `GET /api/status` over the local socket, or a `soos-remote --print-effective-config` subcommand run through `systemctl --user show -p Environment` + the same resolver), and let the tour display "Unknown" when the file the GUI read is ...
- **GUI-05** · INFO · new · 1/1 — Daemon Pause is a plain systemctl stop that outlives soos-gui; tour still shows face/presence On.
  Where: `crates/gui/src/privileged.rs:327`.
  Fix: On close (eframe `on_exit` / viewport close request) while the GUI paused the daemon, ask "Resume soos-daemon?" and submit `ResumeDaemon`; alternatively pause with a runtime-only mechanism (`systemctl stop` + a transient timer, or a daemon IPC "release camera" request).
- **GUI-06** · INFO · new · 1/1 — --mock only requires --dev-store; root with --dev-store /var/lib/soos writes mock templates into the production store.
  Where: `crates/gui/src/args.rs:58`.
  Fix: In `resolve_gui_store` (crates/gui/src/store_mode.rs) canonicalize `dir` and refuse it when it equals or contains the canonical parent of `key_file` or `biometrics_dir`; also refuse `--mock` when the effective uid is 0.
- **GUI-07** · INFO · new · 1/1 — Face frames copied into non-zeroizing egui ColorImage buffers and GPU textures.
  Where: `crates/gui/src/app.rs:2130`.
  Fix: Document the residual (egui/GPU copies cannot be wiped) in Docs/GUI_APPLICATION.md §camera, and set `prctl(PR_SET_DUMPABLE, 0)` / `RLIMIT_CORE=0` for the GUI process so crash dumps do not capture frames.
- **GUI-08** · INFO · new · 1/1 — Privileged GUI actions use the generic policykit.exec prompt; Polkit mode asks the admin password at every start.
  Where: `crates/gui/src/app.rs:256`.
  Fix: Do not list profiles via pkexec at start-up; load them on an explicit "Load profiles" click. Ship dedicated polkit actions (`org.soos.enroll.import`, `org.soos.daemon.pause`, ...) with `org.freedesktop.policykit.exec.path` / `.argv1` annotations and a soos-specific message so the dialog names the ...

### Setup, install, uninstall and packaging (19: 16 LOW, 3 INFO)

- **G204-01** · LOW (reported MEDIUM) · new · 2/2 partial · re-verified after the report (partial) — Setup, GUI and soos-remote CLI act on the invoker's remote.toml and passkey store while the running unit resolves its own from the user manager's environment.
  Where: `scripts/setup.sh:2030`. Merged duplicates: SETBB-02.
  Fix: Pin the path in the unit and in every tool. (a) Have `install_remote.sh` and setup render the installed unit with the absolute `--config <REMOTE_CONFIG>` they resolved (or write a drop-in `Environment=XDG_CONFIG_HOME=<value>`), and make setup refuse (companion_skip) when ...
- **G205-01** · LOW · new · 1/1 partial — authselect rollback record written with masked errors; missing or failed record silently falls back to a feature-less profile.
  Where: `packaging/rpm/soos.spec:134`.
  Fix: In `%post`, write the record to a temporary file in `/etc/soos`, check the status, fsync and rename it, and print a warning on failure (do not use `|| :`). In `%preun` and `uninstall.sh`, when the record is missing or the recorded selection fails, do not fall back to a feature-less profile.
- **G205-02** · LOW · new · 1/1 — uninstall.sh jump-free removal ignores numeric jumps in parent files that include the edited file.
  Where: `scripts/uninstall.sh:465`.
  Fix: In the jump-free branch, also refuse the edit (set PAM_ROLLBACK_INCOMPLETE=true and keep the module) when any other file in `${TARGET_PAM_D}`, `/usr/lib/pam.d` or `/usr/etc/pam.d` both `include`s or `@include`s `${pam_name}` and contains a numeric jump.
- **INST-01** · LOW (reported MEDIUM) · new · 2/2 — dpkg --purge (and every package removal) leaves biometric templates, intruder evidence and both decryption keys on disk.
  Where: `packaging/debian/postrm:8`.
  Fix: In `packaging/debian/postrm` `purge)`: remove `/var/lib/soos/master.key`, `evidence.key`, `biometrics/`, `evidence/`, `state/` (keep only `state/pam-backup` if non-empty with a warning), then `rmdir /var/lib/soos` and `/etc/soos` if empty.
- **INST-02** · LOW (reported MEDIUM) · new · 2/2 — uninstall.sh reports soos cleanly uninstalled while the Internet-exposed remote-unlock companion keeps running.
  Where: `scripts/uninstall.sh:181`. Merged duplicates: XLOG-03.
  Fix: In scripts/uninstall.sh, before the final message: detect the companion for `SUDO_USER`/each user with `~/.config/systemd/user/soos-remote.service` or `~/.local/bin/soos-remote`.
- **SETA-01** · LOW (reported MEDIUM) · accepted residual (ADR 2026-10-13 "One-Command Setup scripts/setup.sh and Default-On Installer Profile", item (2)) · 2/3 partial — Root-started setup builds as the checkout owner and runs and installs code that account can rewrite.
  Where: `scripts/setup.sh:984`.
  Fix: Smallest change, in `resolve_build_user` (scripts/setup.sh:977): when setup is started as root and there is no SUDO_USER, DOAS_USER or PKEXEC_UID, refuse in preflight instead of falling back to the checkout owner, unless an explicit `--build-user <name>` is given.
- **SETAB-01** · LOW (reported MEDIUM) · new · 2/2 — The build that root installs runs under whatever cargo configuration surrounds the checkout (ancestor .cargo/config.toml, inherited CARGO_BUILD_*).
  Where: `scripts/setup.sh:3534`.
  Fix: Smallest change, in scripts/setup.sh before the build (around line 3534): - Refuse to build when any ancestor of `WORKSPACE_ROOT` up to `/` is writable by group or other without the sticky bit, or holds a `.cargo` directory not owned by root or the build user.
- **SETAB-02** · LOW · accepted residual (AI/DECISIONS.md ADR 2026-10-13 One-Command Setup, item (3) Enrollment from a local seat only (tmux residual)) · 1/1 — Local-seat verdict trusts the inherited XDG_SESSION_ID and ignores a locked desk session; enrollment can run from a re-attached tmux pane.
  Where: `scripts/setup.sh:864`.
  Fix: Smallest change in `session_is_local` (scripts/setup.sh:833): also query `--property=Active --property=LockedHint` and treat `Active!=yes` or `LockedHint=yes` as "not local".
- **SETAC-01** · LOW · new · 1/1 partial — The user-half local-seat fallback borrows the desk session of the --user target, not the operator's.
  Where: `scripts/setup.sh:871`.
  Fix: Smallest change, in `detect_local_seat` (scripts/setup.sh:871): allow the `Display` fallback only when `TARGET_USER` equals the invoking account (`[[ "${TARGET_USER}" == "$(id -un)" ]]`). Otherwise keep the verdict "not local" and print the enrollment command.
- **SETB-01** · LOW (reported MEDIUM) · accepted residual (ADR 2026-10-15 "Setup User Phase: Phone Companion Profile With Default-On Features" item (3)) · 2/2 partial — Setup turns on Internet-reachable unlock and camera view with one aggregate consent, including silently on re-run/upgrade.
  Where: `scripts/setup.sh:553`. Merged duplicates: SETA-03.
  Fix: In `resolve_features`, default `allow_unlock` and `camera_view_funnel` to `false` unless their flag is `on` or the interactive run asks a dedicated yes/no question for each ("Allow unlocking this PC from the phone over the Internet? [y/N]").
- **SETB-02** · LOW · new · 1/1 · re-verified after the report (confirmed) — Parser differential in parse_user_remote_config turns an explicit false into true.
  Where: `scripts/setup.sh:1677`.
  Fix: In the multi-line detection pick the delimiter that occurs first in the value (compare the index of `"""` and `'''` after removing single-line strings), not the first in a fixed list.
- **SETB-03** · LOW (reported MEDIUM) · new · 2/2 partial — Funnel guide exposes the whole :443 listener without checking existing serve configuration.
  Where: `scripts/setup.sh:3453`.
  Fix: In `print_companion_guide`, before the Funnel command, print: run `tailscale serve status` first; if it lists any handler on 443 other than this socket, Funnel will publish it too — move it to another port or stop.
- **SETB-04** · LOW · accepted residual (Docs/REMOTE_COMPANION.md section 2b "Prerequisites for Funnel" and the residual list (lines 1757-1759)) · 1/1 — Tailscale 1.102.4 header-stripping floor is advisory; Funnel config written and unit started anyway.
  Where: `scripts/setup.sh:3450`.
  Fix: In `resolve_companion`, when `TAILSCALE_VERSION` is not at least `MIN_TAILSCALE_VERSION`, resolve `allow_funnel` (and its dependents) to `false` unless `--funnel on` is given explicitly, and say so in the plan; print the Funnel command only when the version check passes.
- **SETBB-01** · LOW · new · 1/1 · re-verified after the report (confirmed) — parse_user_remote_config loses track of strings and arrays in four more ways, so an explicit false becomes true.
  Where: `scripts/setup.sh:1521`.
  Fix: Smallest safe change, at scripts/setup.sh:1675 (after the `case`) and at 1635: when a top-level value line (or a line that fails `re_key` but contains `=`) has a `"""`, a `'''`, an unbalanced `[` or a backslash, set every feature key to `false`.
- **SETBC-01** · LOW · new · 1/1 — Tailscale 1.102.4 floor is checked against the CLI client, not the running tailscaled that strips identity headers.
  Where: `scripts/setup.sh:911`.
  Fix: In `tailscale_version` (scripts/setup.sh:903-914) run `tailscale version --daemon` (still a read-only `tailscale version` call) and compare the daemon line (falling back to "unknown", which already warns, when the daemon does not answer).
- **XFSR-02** · LOW · new · 1/1 — uninstall.sh masks grep write errors and can replace a PAM service file with an empty or truncated copy.
  Where: `scripts/uninstall.sh:459`.
  Fix: Capture grep's status and treat anything above 1 as fatal for that file: `rc=0; grep -Ev ... > "$stripped" || rc=$?; if (( rc > 1 )); then warn ...; PAM_ROLLBACK_INCOMPLETE=true; rm -f "$stripped"; continue; fi`.
- **G205-03** · INFO · new · 1/1 — uninstall.sh atomic_replace ignores fsync failure and never fsyncs the directory.
  Where: `scripts/uninstall.sh:325`.
  Fix: `sync "${tmp}" || { rm -f "${tmp}"; return 1; }`, then after `mv -f` run `sync "$(dirname "${target}")"` (coreutils sync accepts a directory) and check its status.
- **SETA-02** · INFO (reported LOW) · new · 1/1 partial — Setup enrolls whoever sits at the camera into another account (--user).
  Where: `scripts/setup.sh:2745`.
  Fix: In `phase_enrollment` (scripts/setup.sh:2715) and the matching plan line (scripts/setup.sh:3038-3048), skip enrollment and print the command when TARGET_USER differs from the user of the local session. In the user half, that is `id -un`.
- **SETAB-03** · INFO · new · 1/1 — Setup never inspects the PAM stack and always says PAM is not activated, even when pam_soos is live.
  Where: `scripts/setup.sh:2780`.
  Fix: Read-only detection in setup: grep the files of the family (`/etc/pam.d/system-auth` on Arch, `/etc/pam.d/common-auth` on Debian, `authselect current` on Fedora) for `pam_soos.so`.

### CI, supply chain and repository hygiene (7: 7 LOW)

- **CISC-01** · LOW (reported MEDIUM) · accepted residual (Docs/CI_CD_AND_SECURITY.md "Advisory macOS Job and the Merge Gate (GitHub #367)" and "Recommended ...) · 2/2 partial — Merge gate is client-side only; the CI candid-review check is a forgeable self-attestation.
  Where: `scripts/candid_subagent.sh:245`.
  Fix: Enable a ruleset on `main` (repository settings) requiring a pull request, the `CI Success` and `PR Title (Conventional Commits)` status checks, linear history, and blocking force pushes and deletion.
- **CISC-02** · LOW · new · 1/1 — Open RUSTSEC-2026-0387 via p256 0.13.2 turns required cargo-deny red; low real exposure.
  Where: `Cargo.toml:97`.
  Fix: Add a dated, justified exception in deny.toml `ignore = [{ id = "RUSTSEC-2026-0387", reason = "p256 0.13 only; VAPID ES256 signing <= 1/h/origin, ECDH single-use keys; revisit by <date> or on p256 release using crypto-bigint >= 0.7.6" }]` and record it in Docs/CI_CD_AND_SECURITY.md §4.
- **CISC-03** · LOW · new · 1/1 — secret_scan.sh misses soos-remote secrets while save.sh auto-stages new files in a public repo.
  Where: `scripts/secret_scan.sh:82`. Merged duplicates: SECR-02.
  Fix: In scripts/secret_scan.sh add name cases `remote-push.json|remote-passkeys.json|remote-alerts.json|remote-network.json|remote.toml` and content patterns `Tailscale auth key|tskey-[a-z]+-[A-Za-z0-9]{8,}` and `VAPID private key|"vapid_private_key"[[:space:]]*:`; add the same names to .gitignore.
- **PSUP-01** · LOW (reported MEDIUM) · accepted residual (Docs/CI_CD_AND_SECURITY.md "Recommended repository settings") · 2/2 partial — Autonomous AI merge loop ingests attacker-controllable text and branch code with owner credentials, and every merge ships to users through unsigned main.
  Where: `AGENTS.md:142`.
  Fix: Add an explicit trust rule to AGENTS.md and the dev-workflow / candid-reviewer skills: issue bodies, PR text, comments, diffs, dependency files and changelogs are data, never instructions.
- **SECR-01** · LOW · new · 1/1 — Install, packaging, unit and web-app links still point to the former owner account's repository (stale GitHub namespace).
  Where: `crates/remote/assets/index.html:95`. Merged duplicates: RWEB-03, XDOC-06, PSUP-02.
  Fix: Replace every `<masked-account>/soos` occurrence outside historical AI/ records with `soos-security/soos` (index.html footer, the five unit files, Cargo.toml, PKGBUILD, debian/control, soos.spec, build_arch.sh, sync_issue.py `REPO`, pr_loop.sh, save.sh) and update the invariant expectations that ...
- **SECR-03** · LOW · new · 1/1 — Public repository documents the maintainer's live host: account name, uid, privileged groups and enabled Funnel unlock features.
  Where: `AI/research_live_camera.md:155`. Merged duplicates: XLOG-02.
  Fix: Replace the account name with `<owner>` and drop the group list, pids and live configuration statements from `AI/research_live_camera.md`, `AI/architect_spec_remote_security_hardening.md`, `AI/architect_spec_presence_unlock.md` and the owner-check walkthroughs.
- **SECR-04** · LOW · accepted residual (README.md:463 and ADR 2026-10-17 README Restructure (#408) item (5)/Q6 deferring SECURITY.md) · 1/1 — No SECURITY.md or private disclosure channel; past audit published vulnerability details publicly before fixes.
  Where: `AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md:318`. Merged duplicates: XDOC-05.
  Fix: Add `SECURITY.md` at the repository root with a private channel (GitHub "Report a vulnerability" through private vulnerability reporting, enabled in the repository settings) and a disclosure timeline.

### Documentation and procedures (10: 1 MEDIUM, 4 LOW, 5 INFO)

- **XDOC-01** · MEDIUM (reported HIGH) · new · 3/3 partial — Documented revocation steps for remote unlock do not stop a Funnel unlock from a lost or stolen phone; no incident guide.
  Where: `Docs/REMOTE_COMPANION.md:162`.
  Fix: Rewrite the revocation paragraph in Docs/REMOTE_COMPANION.md §2a as an ordered procedure whose first step always works: `soos-remote passkeys remove N` (or remove all), then `allow_unlock = false` / `allow_funnel = false` and restart.
- **G304-01** · LOW (reported MEDIUM) · new · 2/2 partial — Manual GDM recipe for the authselect sssd profile puts a success=done face rule before `substack password-auth`, so a face match skips smartcard-required and U2F 2FA pol….
  Where: `Docs/DISTRIBUTION_DEPLOYMENT.md:268`.
  Fix: In Docs/DISTRIBUTION_DEPLOYMENT.md §2.1 item 6, state that the manual recipe must not be used when the delegated stack contains any `required`/`requisite` rule other than the listed gates (explicitly `pam_deny.so` from `with-smartcard-required` and `pam_u2f.so` from `with-pam-u2f-2fa`), and that ...
- **G304-02** · LOW · new · 1/1 — Documented activation and rescue commands force a fixed PAM profile and feature set, dropping without-nullok, with-pamaccess, U2F 2FA and hand-added faillock.
  Where: `tests/physical/screensaver_test.md:469`.
  Fix: Replace every fixed command with the recorded profile: `sudo authselect select custom/soos $(authselect current --raw | cut -d' ' -f2-) --force` for activation and `sudo authselect select $(cat /etc/soos/authselect.previous) --force` for rescue.
- **XDOC-02** · LOW (reported MEDIUM) · new · 2/2 partial — Documented kill switch and account locks do not stop remote passkey unlock.
  Where: `README.md:197`. Merged duplicates: GUI-02.
  Fix: Make `unlock_flow` (and `unlock_options`, camera start) refuse with `403 unlock_disabled` when `/etc/soos/disabled` (and a new `/etc/soos/remote-unlock.disable`) exists, re-checked on every request (lstat, fail closed on error).
- **XDOC-04** · LOW · new · 1/1 — 'polkit prompts always ask for the password' is false with the setuid polkit helper on supported distributions.
  Where: `Docs/NETWORK_LOCK.md:113`.
  Fix: Correct both docs: state the version (127) and the socket condition, and that on setuid-helper systems face does answer polkit. Better: make the PAM module return `PAM_IGNORE` for `PAM_SERVICE == "polkit-1"` by default (opt-in flag to enable), so the documented behaviour holds everywhere.
- **G304-03** · INFO (reported LOW) · new · 1/1 partial — wlroots presence wrapper is incompatible with `swaylock -f`, and the recipe has no idle or before-sleep lock.
  Where: `Docs/DISTRIBUTION_DEPLOYMENT.md:724`.
  Fix: Document the constraint in §5.5 and REMOTE_COMPANION §3: keep the existing `timeout`/`before-sleep 'swaylock -f'` events. Add the presence line as an extra event (or a second swayidle instance), never as a replacement. Do not use the wrapper from `before-sleep`.
- **G304-04** · INFO (reported LOW) · new · 1/1 — Physical presence test sets HandleLidSwitch=ignore with no restore step.
  Where: `tests/physical/screensaver_test.md:298`.
  Fix: Drop `HandleLidSwitch=ignore` from the precondition (an external monitor already triggers the default `HandleLidSwitchDocked=ignore`). If the setting is still needed, give the exact drop-in file to create and a final step that removes it.
- **G304-05** · INFO · new · 1/1 — Two more 'universal' PAM stack examples have no pam_faillock authfail (siblings of PSTK-05/PSTKB-02).
  Where: `Docs/PACKAGING_AND_PROVISIONING.md:212`.
  Fix: Replace both examples with a pointer to the per-distribution templates, or add `auth [default=die] pam_faillock.so authfail` and `authsucc` and drop "enforce"/"universal". Fix it together with PSTK-05 and PSTKB-02.
- **PPHY-04** · INFO · new · 1/1 — Memory-protection doc lists hibernation images as a threat that mlockall does not address; root CLIs holding the master key are not locked.
  Where: `Docs/MEMORY_PROTECTION_AND_SWAP.md:20`.
  Fix: State in Docs/MEMORY_PROTECTION_AND_SWAP.md §1/§2 that `mlockall` does not protect against hibernation and covers only `soos-daemon`, and that encrypted swap/resume (or no hibernation) is required; add the same line to Docs/PACKAGING_AND_PROVISIONING.md.
- **XDOC-03** · INFO (reported LOW) · new · 1/1 partial — Threat model still describes a local-only Phase 1 product after Internet-reachable unlock became the setup default.
  Where: `AI/ARCHITECTURE.md:36`.
  Fix: Update AI/ARCHITECTURE.md §2: add the remote actors listed above, the remote assets, and an invariant for remote unlock (fresh UV passkey; respects kill switch and account state). Fix §13's "default deployment stays tailnet-only" to reflect setup.sh.

### Live deployment (2: 1 LOW, 1 INFO)

- **LIVE-02** · LOW · new · 1/1 — Stale, unmanaged deployment: 2026-10-12/13 binaries, legacy user-unit push sender without later TLS advisory fixes and without the DynamicUser system service.
  Where: `target/security-review/context/live_posture.txt:363`.
  Fix: If setup is not wanted: `scripts/install.sh --build --reinstall` as root, then `sudo systemctl enable --now soos-push-sender@$(id -u).socket`, `systemctl --user disable --now soos-push-sender.service`, `scripts/install_remote.sh` and `systemctl --user restart soos-remote.service`.
- **LIVE-03** · INFO · accepted residual (ADR 2026-10-15 "Setup User Phase: Phone Companion Profile With Default-On Features" item (3)) · 1/1 — Live host runs every remote feature at maximum exposure with no Wi-Fi network lock, on a wheel/docker account.
  Where: `target/security-review/context/live_posture.txt:558`.
  Fix: Owner, pick what is not needed (each line independent): Repository: state in `Docs/REMOTE_COMPANION.md` §2a that an unlocked session of an account in `docker`/`wheel` (NOPASSWD or cached sudo) is root-equivalent, and have setup warn when the target user is in `docker`.

## Appendix B — Merged duplicates and refuted findings

Merged duplicates (duplicate → kept finding): RWEB-03 → SECR-01, DDEC-02 → DIPC-04, INFR-01 → ENRL-01, ENRL-05 → BSTO-02, GUI-02 → XDOC-02, LOCK-01 → VPAD-01, SETA-03 → SETB-01, SETB-05 → RCFG-02, SECR-02 → CISC-03, XLOG-02 → SECR-03, XLOG-03 → INST-02, XDOC-05 → SECR-04, XDOC-06 → SECR-01, PINT-01 → RHTTP-01, PTSN-01 → RIDN-01, PSUP-02 → SECR-01, LIVE-01 → RCFG-01, G202-05 → PAM-03, G202-06 → G201-01, RWANB-01 → XDOS-02, PAMB-01 → PAM-05, SETBB-02 → G204-01.

Refuted (all are critiques of a proposed fix, not of the code at the base commit; see section 7):

- **G202-01** · reported MEDIUM · votes 0/2 — DDEC-01 fix as written turns empty-room presence scans, passers-by and same-UID callers into a sticky owner face lockout; its reset signal is forgeable.
- **G202-02** · reported MEDIUM · votes 1/3 — RHTTP-01 release-only fix variant moves the pin to the 16-slot global connection semaphore (tailnet and sessions blocked).
- **G302-02** · reported MEDIUM · votes 0/2 — Sandbox directives proposed for the soos-remote user unit silently stop pushes and the camera view, and keep the companion down after reboot.
- **G302-03** · reported MEDIUM · votes 0/2 — PSTK-01 fixes unsafe as written: literal success=N miscounts under authselect/pam-auth-update; gdm enable refusal is keyword-only.
- **G201-01** · reported LOW · votes 0/1 — RIDN-01's recommended fix (a setup-written self-address list) fails open when the node's Tailscale addresses change.
- **G202-04** · reported LOW · votes 0/1 — BSTO-01 fix: suggested account markers fail closed for all (home inode under ProtectHome) or break at password change; username-only misses same-name re-creation.
- **G302-05** · reported LOW · votes 0/1 — PPHY-02 lost mode needs root daemon write access to /etc/soos and lets owner-UID code disable face persistently.

## Appendix C — Scenario coverage per area

Counts of scenario rows in `areas/<key>.md`, by result. `-r2` and `-r3` are second and third passes; `gap-*` are the tasks raised by the completeness critic.

| Area file | Rows | SAFE | FINDING | DOCUMENTED | UNKNOWN | Other |
|---|---|---|---|---|---|---|
| `admin-cli` | 45 | 36 | 6 | 3 | 0 | 0 |
| `android` | 45 | 40 | 2 | 0 | 3 | 0 |
| `biometric-store` | 46 | 39 | 4 | 1 | 1 | 1 |
| `camera` | 40 | 35 | 3 | 0 | 1 | 1 |
| `ci-supply-chain` | 43 | 35 | 7 | 0 | 1 | 0 |
| `daemon-decision-r2` | 37 | 35 | 1 | 1 | 0 | 0 |
| `daemon-decision-r3` | 43 | 40 | 0 | 3 | 0 | 0 |
| `daemon-decision` | 39 | 35 | 4 | 0 | 0 | 0 |
| `daemon-ipc-r2` | 33 | 24 | 1 | 1 | 0 | 7 |
| `daemon-ipc-r3` | 31 | 21 | 3 | 1 | 0 | 6 |
| `daemon-ipc` | 45 | 34 | 4 | 3 | 0 | 4 |
| `daemon-presence-r2` | 38 | 27 | 5 | 0 | 1 | 5 |
| `daemon-presence-r3` | 30 | 26 | 2 | 0 | 1 | 1 |
| `daemon-presence` | 47 | 42 | 4 | 0 | 1 | 0 |
| `daemon-runtime` | 46 | 39 | 6 | 0 | 1 | 0 |
| `enrollment-cli` | 35 | 29 | 6 | 0 | 0 | 0 |
| `evidence-store` | 37 | 31 | 4 | 0 | 0 | 2 |
| `gap-r2-01-tailscale-1104-proxy-facts` | 25 | 21 | 1 | 0 | 0 | 3 |
| `gap-r2-02-fix-proposal-safety` | 33 | 20 | 12 | 0 | 1 | 0 |
| `gap-r2-03-suspend-resume-frame-freshness` | 13 | 8 | 2 | 0 | 1 | 2 |
| `gap-r2-04-remote-config-path-divergence` | 24 | 12 | 8 | 0 | 0 | 4 |
| `gap-r2-05-pam-writer-error-paths` | 30 | 22 | 8 | 0 | 0 | 0 |
| `gap-r3-01-camera-control-tampering-vs-pad` | 16 | 11 | 4 | 0 | 0 | 1 |
| `gap-r3-02-fix-safety-r3` | 42 | 26 | 15 | 0 | 0 | 1 |
| `gap-r3-03-mock-toggle-chain` | 20 | 15 | 2 | 1 | 0 | 2 |
| `gap-r3-04-deployment-docs-recipes` | 27 | 19 | 7 | 0 | 0 | 1 |
| `gap-r3-05-fedora-homed-ecryptfs-face-skip` | 18 | 13 | 3 | 0 | 0 | 2 |
| `gui` | 55 | 46 | 9 | 0 | 0 | 0 |
| `inference` | 46 | 41 | 4 | 0 | 0 | 1 |
| `install-packaging` | 40 | 32 | 4 | 1 | 0 | 3 |
| `live-posture` | 43 | 31 | 9 | 0 | 3 | 0 |
| `lockscreen` | 40 | 39 | 1 | 0 | 0 | 0 |
| `pam-r2` | 37 | 33 | 1 | 0 | 0 | 3 |
| `pam-stacks-r2` | 42 | 36 | 2 | 0 | 0 | 4 |
| `pam-stacks-r3` | 38 | 31 | 1 | 0 | 0 | 6 |
| `pam-stacks` | 37 | 32 | 5 | 0 | 0 | 0 |
| `pam` | 48 | 41 | 5 | 2 | 0 | 0 |
| `persona-internet` | 47 | 39 | 3 | 4 | 1 | 0 |
| `persona-local-user` | 38 | 28 | 7 | 0 | 2 | 1 |
| `persona-phone` | 32 | 20 | 8 | 3 | 1 | 0 |
| `persona-physical` | 33 | 14 | 14 | 1 | 2 | 2 |
| `persona-same-uid` | 21 | 12 | 9 | 0 | 0 | 0 |
| `persona-supply-chain` | 31 | 23 | 7 | 0 | 1 | 0 |
| `persona-tailnet` | 24 | 17 | 5 | 0 | 0 | 2 |
| `persona-web` | 36 | 32 | 4 | 0 | 0 | 0 |
| `protocol` | 42 | 39 | 2 | 0 | 0 | 1 |
| `remote-alerts` | 49 | 43 | 5 | 0 | 0 | 1 |
| `remote-camera-r2` | 36 | 26 | 4 | 5 | 1 | 0 |
| `remote-camera-r3` | 34 | 28 | 1 | 4 | 0 | 1 |
| `remote-camera` | 48 | 41 | 1 | 6 | 0 | 0 |
| `remote-config` | 44 | 36 | 6 | 2 | 0 | 0 |
| `remote-http-r2` | 45 | 44 | 1 | 0 | 0 | 0 |
| `remote-http` | 41 | 40 | 1 | 0 | 0 | 0 |
| `remote-identity-r2` | 41 | 40 | 0 | 0 | 0 | 1 |
| `remote-identity` | 38 | 37 | 1 | 0 | 0 | 0 |
| `remote-push-r2` | 73 | 66 | 2 | 0 | 1 | 4 |
| `remote-push-r3` | 36 | 34 | 1 | 0 | 1 | 0 |
| `remote-push` | 61 | 56 | 5 | 0 | 0 | 0 |
| `remote-unlock-r2` | 39 | 35 | 2 | 0 | 0 | 2 |
| `remote-unlock-r3` | 33 | 32 | 1 | 0 | 0 | 0 |
| `remote-unlock` | 50 | 43 | 6 | 0 | 1 | 0 |
| `remote-web` | 37 | 32 | 4 | 0 | 1 | 0 |
| `remote-webauthn-r2` | 54 | 52 | 1 | 0 | 0 | 1 |
| `remote-webauthn` | 61 | 60 | 1 | 0 | 0 | 0 |
| `secrets-history` | 29 | 24 | 5 | 0 | 0 | 0 |
| `setup-companion-r2` | 37 | 31 | 5 | 1 | 0 | 0 |
| `setup-companion-r3` | 29 | 25 | 2 | 2 | 0 | 0 |
| `setup-companion` | 43 | 34 | 8 | 0 | 1 | 0 |
| `setup-system-r2` | 33 | 25 | 5 | 0 | 2 | 1 |
| `setup-system-r3` | 36 | 27 | 2 | 0 | 0 | 7 |
| `setup-system` | 36 | 28 | 4 | 2 | 1 | 1 |
| `vision-pad-r2` | 45 | 33 | 1 | 3 | 2 | 6 |
| `vision-pad-r3` | 36 | 31 | 0 | 2 | 1 | 2 |
| `vision-pad` | 48 | 35 | 3 | 3 | 2 | 5 |
| `x-crypto` | 62 | 59 | 1 | 0 | 0 | 2 |
| `x-docs-claims` | 24 | 9 | 11 | 0 | 4 | 0 |
| `x-dos-lockout` | 48 | 30 | 4 | 0 | 0 | 14 |
| `x-fs-races` | 43 | 39 | 4 | 0 | 0 | 0 |
| `x-logging-privacy` | 38 | 30 | 4 | 0 | 0 | 4 |
| `x-subprocess-env` | 51 | 48 | 1 | 0 | 0 | 2 |
| `x-unsafe-panics` | 43 | 41 | 1 | 0 | 1 | 0 |
| **Total (81 files)** | **3159** | **2615** | **328** | **55** | **41** | **120** |

Several FINDING rows can point to the same finding, and rows of later passes can repeat a scenario of an earlier pass.

## Appendix D — Tool output and file layout

- `context/cargo_deny.txt`: `cargo deny check` at the base commit. One advisory: `RUSTSEC-2026-0387` (`crypto-bigint 0.5.5` through `p256 0.13.2` in `soos-remote`), see `CISC-02`.
- `context/gitleaks.json`: gitleaks history scan. Three generic hits, all in the redaction tests of `crates/admin-cli`.
- `context/live_posture.txt`: read-only snapshot of the maintainer's installation, identity values masked.
- `context/issues.tsv`: every GitHub issue and pull request at review time, used for the known-status calls.
- `poc/RIDN-01-live-check.txt`: the read-only live check of `RIDN-01`.
- `workflow_result.json`: the machine-readable result (confirmed, refuted, duplicates, chains, coverage files, uncovered notes).
- `factcheck_result.json`: the independent fact-check of this document and the re-verification sample.
- `report_src/`: the hand-written body and the script that assembled this file.
- Not installed on the review machine, so not run: cargo-audit, semgrep, shellcheck, actionlint, zizmor, trufflehog.

## Appendix E — GitHub issues filed from this review

Filed in `soos-security/soos` with the label `review-2026-11-10`. The public issue bodies were generated from the finding files and the verification notes, with every reference to the maintainer's installation and to the private working files removed (removal log: `issues/_scrub_log.md`; pre-gate copies: `issues_pre_gate/`). `LIVE-02`, `LIVE-03` and `SECR-03` describe that installation and were deliberately not filed.

- Tracking issue: #477 (https://github.com/soos-security/soos/issues/477)
- Batch 1: #478 — fix(remote): make passkey changes and remote sessions visible to the owner
- Batch 2: #479 — fix(remote): refuse self-originated tailnet requests and separate the tailnet limiters
- Batch 3: #480 — fix(setup): safe companion defaults and a fail-safe remote.toml reader
- Batch 4: #481 — fix(remote): gate what a passkey reaches (camera view, unlock, lock)
- Batch 5: #482 — fix(remote): keep the companion reachable for the owner under load
- Batch 6: #483 — chore(docs): revocation procedure, incident guide, threat model and SECURITY.md
- Batch 7: #484 — fix(install): uninstall, purge and update lifecycle for the companion and the biometric data
- Batch 8: #485 — fix(policy): escalating face back-off and removal of the PAD oracles
- Batch 9: #486 — fix(daemon): scope face authentication by PAM service and add an intent signal
- Batch 10: #487 — fix(pam): stop success=done from skipping the factors placed after it
- Batch 11: #488 — fix(enrollment): bind templates and enrollment targets to the account
- Batch 12: #489 — fix(daemon): account-keyed admission limits and a first-frame deadline for root peers
- Batch 13: #490 — fix(remote): journal follower filtering and completeness of failed-password alerts
- Batch 14: #491 — fix(camera): fail safe after a camera re-resolution at runtime
- Batch 15: #492 — chore(hardening): unit sandboxing and core-dump protection
- Batch 16: #493 — chore(ci): supply-chain and repository hygiene
- Batch 17: #494 — feat(store): protect master.key and evidence.key at rest and add a rekey path

| Finding | Issue | Batch |
|---|---|---|
| `ADMN-01` | #495 | none |
| `ADMN-02` | #496 | none |
| `ADMN-03` | #497 | none |
| `ANDR-01` | #498 | none |
| `BSTO-01` | #499 | batch 11 (#488) |
| `BSTO-02` | #500 | batch 11 (#488) |
| `BSTO-03` | #501 | batch 17 (#494) |
| `CAMV-01` | #502 | batch 14 (#491) |
| `CAMV-02` | #503 | none |
| `CISC-01` | #504 | batch 16 (#493) |
| `CISC-02` | #505 | batch 16 (#493) |
| `CISC-03` | #506 | batch 16 (#493) |
| `DDEC-01` | #507 | batch 8 (#485) |
| `DDECB-01` | #508 | none |
| `DIPC-01` | #509 | none |
| `DIPC-02` | #510 | none |
| `DIPC-03` | #511 | batch 12 (#489) |
| `DIPC-04` | #512 | batch 9 (#486) |
| `DIPCB-01` | #513 | none |
| `DIPCC-01` | #514 | batch 12 (#489) |
| `DPRS-01` | #515 | batch 8 (#485) |
| `DPRS-02` | #516 | batch 9 (#486) |
| `DPRSB-01` | #517 | batch 4 (#481) |
| `DPRSC-01` | #518 | batch 9 (#486) |
| `DRUN-01` | #519 | batch 15 (#492) |
| `DRUN-02` | #520 | batch 15 (#492) |
| `DRUN-03` | #521 | batch 12 (#489) |
| `DRUN-04` | #522 | none |
| `DRUN-05` | #523 | none |
| `ENRL-01` | #524 | batch 11 (#488) |
| `ENRL-02` | #525 | none |
| `ENRL-03` | #526 | none |
| `ENRL-04` | #527 | none |
| `EVID-01` | #528 | none |
| `EVID-02` | #529 | batch 12 (#489) |
| `EVID-03` | #530 | batch 8 (#485) |
| `G202-03` | #531 | none |
| `G203-01` | #532 | none |
| `G204-01` | #533 | none |
| `G205-01` | #534 | batch 7 (#484) |
| `G205-02` | #535 | batch 7 (#484) |
| `G205-03` | #536 | batch 7 (#484) |
| `G301-01` | #537 | batch 14 (#491) |
| `G302-01` | #538 | batch 14 (#491) |
| `G302-04` | #539 | none |
| `G303-01` | #540 | none |
| `G304-01` | #541 | batch 6 (#483) |
| `G304-02` | #542 | batch 6 (#483) |
| `G304-03` | #543 | batch 6 (#483) |
| `G304-04` | #544 | batch 6 (#483) |
| `G304-05` | #545 | batch 6 (#483) |
| `G305-01` | #546 | none |
| `GUI-01` | #547 | batch 11 (#488) |
| `GUI-03` | #548 | batch 3 (#480) |
| `GUI-04` | #549 | none |
| `GUI-05` | #550 | none |
| `GUI-06` | #551 | batch 11 (#488) |
| `GUI-07` | #552 | none |
| `GUI-08` | #553 | none |
| `INFR-02` | #554 | batch 16 (#493) |
| `INFR-03` | #555 | batch 16 (#493) |
| `INST-01` | #556 | batch 7 (#484) |
| `INST-02` | #557 | batch 7 (#484) |
| `PAM-01` | #558 | batch 12 (#489) |
| `PAM-02` | #559 | batch 9 (#486) |
| `PAM-03` | #560 | batch 9 (#486) |
| `PAM-04` | #561 | none |
| `PAM-05` | #562 | none |
| `PINT-02` | #563 | batch 4 (#481) |
| `PLOC-01` | #564 | batch 13 (#490) |
| `PLOC-02` | #565 | none |
| `PPHN-01` | #566 | batch 1 (#478) |
| `PPHN-02` | #567 | batch 1 (#478) |
| `PPHN-03` | #568 | none |
| `PPHY-01` | #569 | batch 1 (#478) |
| `PPHY-02` | #570 | batch 9 (#486) |
| `PPHY-03` | #571 | batch 8 (#485) |
| `PPHY-04` | #572 | batch 17 (#494) |
| `PROT-01` | #573 | none |
| `PSTK-01` | #574 | batch 10 (#487) |
| `PSTK-02` | #575 | batch 10 (#487) |
| `PSTK-03` | #576 | batch 7 (#484) |
| `PSTK-04` | #577 | none |
| `PSTK-05` | #578 | batch 6 (#483) |
| `PSTKB-01` | #579 | batch 9 (#486) |
| `PSTKB-02` | #580 | batch 6 (#483) |
| `PSTKC-01` | #581 | batch 10 (#487) |
| `PSUP-01` | #582 | batch 16 (#493) |
| `PTSN-02` | #583 | batch 2 (#479) |
| `PTSN-03` | #584 | batch 2 (#479) |
| `PUID-01` | #585 | batch 1 (#478) |
| `PWEB-01` | #586 | none |
| `RALR-01` | #587 | batch 13 (#490) |
| `RALR-02` | #588 | batch 13 (#490) |
| `RALR-03` | #589 | batch 13 (#490) |
| `RALR-04` | #590 | batch 13 (#490) |
| `RCAM-01` | #591 | batch 4 (#481) |
| `RCAMB-01` | #592 | none |
| `RCAMB-02` | #593 | batch 15 (#492) |
| `RCAMB-03` | #594 | batch 6 (#483) |
| `RCAMC-01` | #595 | batch 4 (#481) |
| `RCFG-01` | #596 | batch 15 (#492) |
| `RCFG-02` | #597 | batch 3 (#480) |
| `RCFG-03` | #598 | batch 15 (#492) |
| `RCFG-04` | #599 | batch 7 (#484) |
| `RHTTP-01` | #600 | batch 5 (#482) |
| `RIDN-01` | #601 | batch 2 (#479) |
| `RPSH-01` | #602 | none |
| `RPSH-02` | #603 | none |
| `RPSH-03` | #604 | none |
| `RPSHB-01` | #605 | batch 2 (#479) |
| `RPSHC-01` | #606 | batch 7 (#484) |
| `RUNL-01` | #607 | batch 4 (#481) |
| `RUNL-02` | #608 | batch 1 (#478) |
| `RUNL-03` | #609 | none |
| `RUNL-04` | #610 | none |
| `RUNLB-01` | #611 | batch 1 (#478) |
| `RUNLB-02` | #612 | none |
| `RUNLC-01` | #613 | batch 4 (#481) |
| `RWAN-01` | #614 | batch 5 (#482) |
| `RWEB-01` | #615 | batch 5 (#482) |
| `RWEB-02` | #616 | batch 2 (#479) |
| `SECR-01` | #617 | batch 16 (#493) |
| `SECR-04` | #618 | batch 6 (#483) |
| `SETA-01` | #619 | batch 16 (#493) |
| `SETA-02` | #620 | batch 11 (#488) |
| `SETAB-01` | #621 | batch 16 (#493) |
| `SETAB-02` | #622 | none |
| `SETAB-03` | #623 | none |
| `SETAC-01` | #624 | none |
| `SETB-01` | #625 | batch 3 (#480) |
| `SETB-02` | #626 | batch 3 (#480) |
| `SETB-03` | #627 | batch 3 (#480) |
| `SETB-04` | #628 | none |
| `SETBB-01` | #629 | batch 3 (#480) |
| `SETBC-01` | #630 | none |
| `VPAD-01` | #631 | batch 8 (#485) |
| `VPAD-02` | #632 | batch 8 (#485) |
| `VPAD-03` | #633 | batch 14 (#491) |
| `VPADB-01` | #634 | batch 8 (#485) |
| `XCRY-01` | #635 | batch 17 (#494) |
| `XDOC-01` | #636 | batch 6 (#483) |
| `XDOC-02` | #637 | batch 4 (#481) |
| `XDOC-03` | #638 | batch 6 (#483) |
| `XDOC-04` | #639 | batch 6 (#483) |
| `XDOS-01` | #640 | batch 12 (#489) |
| `XDOS-02` | #641 | batch 5 (#482) |
| `XFSR-01` | #642 | none |
| `XFSR-02` | #643 | batch 7 (#484) |
| `XLOG-01` | #644 | none |
| `XLOG-04` | #645 | none |
| `XSUB-01` | #646 | none |
| `XUNS-01` | #647 | none |
