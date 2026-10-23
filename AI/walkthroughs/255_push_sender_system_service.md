# Walkthrough 255 — Push Sender as a Sandboxed System Service

- **Date**: 2026-10-23
- **Issue**: owner hardware report of 2026-10-23 (no backlog id, GitHub-only) — **Branch**: `fix/push-sender-system-service`
- **Matrix criteria**: PSS1–PSS14 (PSS15 is the owner's check on Ubuntu 24.04)
- **ADR**: "[2026-10-23] Push Sender as a Sandboxed System Service" (amends 2026-10-06 Web Push and 2026-10-12 Security Hardening)

## 1. Context & Objectives

On Ubuntu 24.04 (`kernel.apparmor_restrict_unprivileged_userns=1`, the default since 23.10)
`scripts/setup.sh` failed in its companion phase: the user unit `packaging/soos-push-sender.service`
exited `226/NAMESPACE` ("PrivateIPC=yes is configured, but IPC namespace setup failed",
"Failed to set up mount namespacing: Operation not supported"). The systemd user manager is
unprivileged, so every mount sandbox directive needs an unprivileged user namespace, which
AppArmor denies. The owner had to run setup with `--push off --offline-alerts off`.

Owner decision: run `soos-push-sender` as a sandboxed **system** service (privileged manager, no
user namespace), with isolation at least as strong as before; relaxing the sandbox or disabling
push on Ubuntu were rejected.

## 2. Architect Design

Spec: `AI/architect_spec_push_sender_system_service.md`.

- **Units** (new): `packaging/soos-push-sender@.socket` (instance = numeric desktop uid; PID 1
  listens on `/run/soos-push/<uid>/push.sock`, `SocketUser=%i`, `SocketMode=0600`, root-owned
  `0755` directories) and `packaging/soos-push-sender@.service` (`ExecStart=/usr/libexec/soos/
  soos-push-sender --system-instance %i`, `StandardInput=socket`, journal output,
  `DynamicUser=yes`, no capability, every mount directive of the user unit plus
  `ProtectControlGroups=`, `ProtectProc=invisible`, `ProcSubset=pid`, `RemoveIPC=`,
  `KeyringMode=private`, `SystemCallFilter=~@privileged`, the two systemd-resolved
  `resolv.conf` files bound read-only). No `[Install]` in the service: socket-activated only.
  `PrivateUsers=` is excluded (it would turn the client uid into the overflow uid and break
  `SO_PEERCRED`).
- **Sender** (`crates/push-sender`): additive `RunMode`, `InstanceError`, `parse_run_mode`,
  `check_instance_identity`, `evaluate_system_sandbox`/`probe_system_sandbox`,
  `listener_from_fd`; `main` dispatches after the root check and before the unchanged user-mode
  path. The socket is cloned from stdin (`stdin().as_fd().try_clone_to_owned()`): no `unsafe`,
  no `LISTEN_FDS` read. `serve_connection(stream, allowed_uid, …)` is reused unchanged.
- **Protocol** (`crates/push-protocol`): `PUSH_SYSTEM_SOCKET_ROOT`, `PUSH_SYSTEM_INSTANCE_ARG`,
  `system_push_socket_path(uid)`.
- **Client** (`crates/remote`): `PushSocketCandidate { path, peer_uid }`,
  `push_socket_candidates`, `UnixPushTransport::with_candidates`. Without an explicit
  `push_socket_path`: the system socket (peer uid 0, PID 1 listens) then the legacy user
  socket (peer = owner); first existing candidate only. `config.rs` unchanged.
- **Installation**: `install.sh` (optional artifact → `<prefix>/libexec/soos/soos-push-sender`
  + both units, never enabled), `uninstall.sh` (disables instances, removes files), RPM spec and
  PKGBUILD; `setup.sh` root half `--push-sender on` → `systemctl enable --now
  soos-push-sender@<uid>.socket`; companion phase verifies it and disables a legacy user unit.
  `install_remote.sh` keeps the user unit as the root-free alternative (printed steps updated).
- **Invariants touched**: RMC-S34/S36/S41/S42 and RSH11/RSH12 (sender hygiene, single
  environment read, sandbox-first order) hold unchanged; SET24/SET33/SET37 conflict (§4).

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`: APPROVED with three MAJOR findings resolved in the plan before the
tests (F-1 the system listener is PID 1, so the client must expect peer uid 0 for the system
socket; F-2 `StandardInput=socket` needs explicit journal output; F-3 Ubuntu's
`resolv.conf` lives under the hidden `/run`, so the two resolved files are bound read-only) and a
process finding F-6 (the setup change contradicts three existing contracts, §4).

## 4. Tester Contract

| Test (path::name) | Criterion | Red evidence (commit `521b559`, signature stubs) |
|---|---|---|
| `crates/push-protocol/tests/system_socket_tests.rs::test_pss3_system_socket_path` | PSS3 | `left: "" right: "/run/soos-push"` |
| `crates/push-sender/tests/system_instance_tests.rs::test_pss4_parse_run_mode` | PSS4 | panicked at line 97 (`Err(Usage)` for no argument) |
| `…::test_pss5_instance_identity` | PSS5 | panicked at line 160 (root accepted) |
| `…::test_pss6_*` (5 tests) | PSS6 | 4 red (stub accepted every table, the probe passed); `accepts_the_unit_table` green against the stub |
| `…::test_pss7_listener_from_fd` | PSS7 | panicked at line 344 (connected stream accepted) |
| `…::test_pss8_*` (3 tests) | PSS8 | 2 red (arguments ignored); `exits_78_outside_its_system_sandbox` green through the legacy path |
| `crates/remote/tests/push_system_socket_tests.rs::test_pss10_*` (6 tests) | PSS10 | 4 red (empty candidates, fall-through); 2 green (no candidate, owner check) |
| `tests/invariants/src/push_sender_system_contract.rs::test_pss1/2/9/11/13` | PSS1, 2, 9, 11, 13 | all red (units, constants, dispatch, staging, docs missing) |
| `tests/invariants/src/setup_user_phase_contract.rs::test_pss12_*` (5 tests) | PSS12 | 3 red (no system enable, no migration, no plan line); `push_off` and `rollback_restores_the_legacy_user_unit` green before the change (they guard regressions) |
| `tests/docker/push_sender_system_unit_test.sh` | PSS14 | first run red: "push sender sandbox not detected" (see §6) |

Tester corrections before green (new tests of this PR only, with evidence):
- `test_pss11_scripts_and_packages_statically`: "install.sh never names `soos-push-sender@`"
  also forbade the template names install.sh must install; replaced by the stated intent (no
  `systemctl` line naming the sender, no instance name `soos-push-sender@$…`), which is stricter
  about enabling.
- `SERVICE_LINES` (PSS1) and the PSS6 fixture: `ProtectHome=tmpfs` → `ProtectHome=yes` and the
  `/home`, `/root` lines as captured under systemd 255 (§6). The read-write tmpfs case stays
  refused.

### Existing assertions contradicted (owner-approved migration required; NOT changed here)

Running `cargo test -p soos-invariants` with the setup change leaves exactly these three existing
tests red (all other 887 invariants pass):

1. `tests/invariants/src/setup_user_phase_contract.rs::test_set_u6_full_run_sets_up_the_default_on_companion` (SET24)
   - Current: `find_line(&lines, &["uid=1000 systemctl --user enable --now soos-push-sender.service"])` (line 1020–1023) and `assert!(h.user_unit("soos-push-sender.service", "active"))` (line 1029).
   - Observed: `no log line with ["uid=1000 systemctl --user enable --now soos-push-sender.service"]`.
   - Required: `find_line(&lines, &["uid=0 systemctl enable --now soos-push-sender@1000.socket"])` placed between `uid=0 install.sh` and `uid=1000 install_remote.sh`, and `assert!(!h.user_unit("soos-push-sender.service", "enabled"))`.
   - Why: the owner decision moves the sender to the system manager; enabling the user unit is exactly the Ubuntu failure.
2. `…::test_set_u22_rollback_stops_a_unit_started_by_the_run` (SET33)
   - Current: removes `user_soos-push-sender.service_active` (line 1424–1426, panics `NotFound`), sets `STUB_USER_FAIL_UNIT=soos-push-sender.service` (lines 1427, 1449) to fail the *second* user unit after `soos-remote.service` started, asserts `h.user_unit("soos-push-sender.service", "enabled")` (line 1441).
   - Required: the companion no longer starts a second user unit, so the failure after the `soos-remote.service` start needs another trigger. Proposal: a stub knob making the companion's `systemctl is-active soos-push-sender@<uid>.socket` (run as the user, not as root) fail, e.g. `STUB_USER_SYSTEM_INACTIVE_UNIT`, used instead of `STUB_USER_FAIL_UNIT`; drop the three sender-unit state-file lines and the sender `enabled` assertion; keep every `soos-remote.service` start/stop/restart assertion unchanged.
   - Why: same decision; the rollback contract of `soos-remote.service` is unchanged.
3. `…::test_set_u21_push_switched_on_later_enables_the_sender` (SET37)
   - Current: `assert!(h.user_unit("soos-push-sender.service", "enabled"))` / `"active"` after `--push on` (lines 1584–1585), removal of the user-unit state files (1591–1599, would panic `NotFound`), and the text `soos-push-sender.service is not enabled` (line 1603).
   - Observed: `assertion failed: h.user_unit("soos-push-sender.service", "enabled")`.
   - Required: after `--push on`, `find_line(&lines, &["uid=0 systemctl enable --now soos-push-sender@<uid>.socket"])`; for the "kept off" half, a stub knob that reports the socket disabled to the user-half query (the harness shares one state for every system unit), and the text `soos-push-sender@<uid>.socket is not enabled`.
   - Why: same decision; the "switching push on enables the sender like a first write" rule is kept, applied to the socket.

4. `tests/docker/setup_root_preview_test.sh::companion_ran` (SET34 harness, CI job `setup-root-preview`)
   - Current (line 536): `expect_out "[DONE] companion: soos-remote.service soos-push-sender.service running" "$1"`.
   - Observed: `A1: output lacks: [DONE] companion: soos-remote.service soos-push-sender.service running` (its stub `install.sh` installs no sender unit, so the root half prints `[SKIP] push sender: not applied`, and the container has no system bus for the user's query, so the companion prints `[SKIP] push sender: the state … could not be read`).
   - Required: `expect_out "[DONE] companion: soos-remote.service running" "$1"`. Evidence: with only that line changed locally (reverted, not committed) all cases A1 A2 B C1 C2 D E1 E2 F1 F2 pass.
   - Why: the companion no longer starts the sender user unit.

## 5. Auditor Constraints

`AI/auditor_constraints_push_sender_system.md` (A1–A15, CLEARED). How they are met: no panic
construct or lint allowance in the new code (clippy `-D warnings`, RMC-S36); no `unsafe`, no raw
fd, stdin clone (PSS9); single environment read kept (RSH12); constant log lines (RMC-S36);
exit 78 before serving for every refusal (PSS8, PSS14 uid 0); peer authentication through the
unchanged `serve_connection` with the argv uid (PSS9, PSS14 bob/root); listening AF_UNIX stream
only (PSS7); bounded mount-table read (code review); client first-candidate rule and per-candidate
peer uid (PSS10, PSS14 `SO_PEERCRED` 0); `push.rs` reads no environment (the derived default
comes from `main.rs`); unit exact (PSS1); install journaled and never enabling (PSS11);
uninstall guards (`-z DESTDIR`, `--skip-systemd`, `^soos-push-sender@[0-9]+\.socket$`);
setup bounded (`SENDER_UNIT_TIMEOUT_S`, SET40 respected), journaled, dry run read-only (PSS12).

## 6. Implementation

- `crates/push-protocol/src/lib.rs`, `crates/push-sender/src/{lib,main}.rs`,
  `crates/remote/src/{push,main}.rs`, `crates/gui/src/onboarding.rs` (wording).
- `packaging/soos-push-sender@.socket`, `packaging/soos-push-sender@.service`,
  `packaging/rpm/soos.spec`, `packaging/arch/PKGBUILD`; the legacy user unit is unchanged.
- `scripts/install.sh`, `scripts/uninstall.sh`, `scripts/setup.sh`, `scripts/install_remote.sh`
  (printed steps and comments only).
- `tests/docker/push_sender_system_unit_test.sh` and CI job `push-sender-unit`.

Notable decision from the Docker evidence: the first harness run failed with "sandbox not
detected". Under systemd 255 `ProtectHome=tmpfs` mounts `/home` and `/root` as **read-write**
tmpfs in a system service (per-mount options `rw`), while `ProtectHome=yes` mounts read-only
binds of systemd's inaccessible directory (`/systemd/inaccessible/dir … ro … - tmpfs`). The unit
uses `ProtectHome=yes` (stronger: mode 000, nothing listable) and the self-check keeps requiring
a read-only tmpfs.

## 7. Candid Review

Not run by this agent: the lead spawns an independent reviewer (`AI/candid_review_report.md`).
`./scripts/candid_review.sh` layer 1 passes in the pre-commit hook.

## 8. Verification Results

- `cargo fmt --all --check`: clean.
- `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean.
- `cargo test --locked -p soos-push-protocol -p soos-push-sender -p soos-remote --all-features`:
  506 passed, 0 failed. `cargo test -p soos-gui --test onboarding_feature_tests`: 28 passed.
- `cargo test --locked -p soos-invariants`: 887 passed, 3 failed (the SET24, SET33, SET37
  migrations of §4).
- `bash tests/docker/push_sender_system_unit_test.sh` (ubuntu:24.04, systemd 255 as PID 1):
  passed — socket `1500 600` in `0 755` directories, listener `SO_PEERCRED` uid 0, service as
  dynamic uid 65182 with `CapEff` 0 and its sandbox check passed, `web.push.apple.com` answered
  HTTP 403 through the sandbox (DNS, TLS), bob `Permission denied`, root peer closed without a
  reply, uid 0 instance exit 78, `systemd-analyze security` exposure 1.4 OK, host sysctls
  unchanged.
- `bash tests/docker/setup_root_preview_test.sh`: red on A1 only (migration 4 of §4); with that
  one expected line migrated locally (reverted afterwards) every case passes.
- `bash -n` on every touched script. `shellcheck` is not installed on the development host: it
  runs in the CI `lint` job.

## 9. Known Limitations / Follow-ups

- Not verified on a real Ubuntu 24.04 host with the AppArmor restriction (PSS15, owner): the
  container proves the system-manager path only.
- The four migrations of §4 (three SET invariants and one line of the setup-root-preview Docker
  harness) need the owner's approval; until the lead applies them the branch is red on exactly
  those tests (CI jobs `test` and `setup-root-preview`).
- `scripts/setup.sh` enables the socket idempotently; a `--prefix` other than `/usr` installs the
  binary where the unit does not look (warned by install.sh, same rule as the daemon unit).
- A `resolv.conf` under `/run` other than the two systemd-resolved files still needs a root
  drop-in (documented in `Docs/REMOTE_COMPANION.md` §2d).

## 10. Candid Review Round 1 Follow-up

The independent review (CHANGES_REQUESTED) came after the lead applied the owner-approved
migrations (SET24, SET33, SET37, `companion_ran`). Addressed, tests first (commit `bca9e03`, red
on the three new tests):

- **`system_up_to_date`** (PSS16): compares the sender binary under `<prefix>/libexec/soos` with
  the artifact and both template units under `<sysconfdir>/systemd/system` with `packaging/`,
  when the build has the sender and the checkout has both units (the lock-screen pattern). Red:
  `[PLAN] system: already done` with nothing of the sender installed.
- **Companion rollback** (PSS17): the socket enabled by the root half is kept (idle) and the
  failure output prints `sudo systemctl disable --now soos-push-sender@<uid>.socket`; documented
  in `Docs/REMOTE_COMPANION.md` §2d and `Docs/PACKAGING_AND_PROVISIONING.md`.
- **Package removal** (PSS18): RPM `%preun` (erase only), Arch `pre_remove()` and Debian `prerm`
  (`remove`) disable and stop every `soos-push-sender@<digits>.socket`.
- `# shellcheck disable=SC2016` with its reason above the Perl client of the Docker harness.

**Not done — owner decision needed (read-path rule for `--prefix`)**: skipping the socket when
`PREFIX` is not `/usr` would turn the migrated SET24 and SET37 red, because the setup harness
always runs with a fake `--prefix <tmp>/root/usr` and both now expect the
`uid=0 systemctl enable --now soos-push-sender@<uid>.socket` line. See the lead's report for the
options.

Verification of this round: `cargo test -p soos-invariants` 893 passed;
`shellcheck --severity=error` (CI level, `koalaman/shellcheck:stable`) clean on the touched
scripts, and the full-severity run is clean on `setup.sh`, `install_remote.sh`, the Docker
harness, `packaging/debian/prerm` and `packaging/arch/soos.install` (the remaining warnings of
`install.sh`/`uninstall.sh` are pre-existing lines).
