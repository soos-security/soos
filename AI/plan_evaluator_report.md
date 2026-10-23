# Plan Evaluation Report
- **Date**: 2026-10-23
- **Issue**: GitHub-only (no backlog id) — owner hardware report of 2026-10-23: `soos-push-sender.service`
  exits `226/NAMESPACE` on Ubuntu 24.04 (`kernel.apparmor_restrict_unprivileged_userns=1`)
- **Branch**: `fix/push-sender-system-service`
- **Base commit**: `e7826de`
- **Plan evaluated**: `AI/architect_spec_push_sender_system_service.md` §1–§13
- **Mode**: self-check by the implementing agent (the independent candid review runs separately before any merge).

## 1. Coverage Matrix

| Requirement (owner decision / lead constraints) | Spec element | Status |
|---|---|---|
| Sandboxed system service, no unprivileged user namespace needed | §2.2 (system manager, `DynamicUser=yes`) | Covered |
| Never root at runtime, no capabilities | §2.2 `DynamicUser=yes`, empty bounding/ambient sets; §4.1 identity check | Covered |
| Same or stronger hardening than the user unit | §2.3 comparison table | Covered |
| Network egress unchanged | §2.2 same families; code unchanged (resolver, allowlist) | Covered |
| Only the owning user's `soos-remote` may talk to it; multi-user: one instance per user | §2.1 socket `0600 %i`, §4.3 `SO_PEERCRED` = instance uid, template per uid | Covered |
| `probe_sandbox` still passes (adapted additively) | §4.2 `probe_system_sandbox`, user-mode probe unchanged | Covered |
| No secret logged; socket not world-writable | §4.4 constant log lines; §2.1 `SocketMode=0600` | Covered |
| System unit installed by the root phase, not `install_remote.sh` | §6 `install.sh`; §7.2 system phase | Covered |
| User phase points `soos-remote` at the socket; default resolves to the new path; backward compatible | §5 candidates (system first, legacy second) | Covered |
| Setup disables the old user unit when present | §7.3 migration | Covered |
| Uninstall removes it; dry run writes nothing; read-path rule | §6, §7.2, §7.4 | Covered |
| Existing tests immutable; contradictions listed for the owner | §13, this report §6 | Covered (migration list) |
| ADR amendment, matrix rows, docs, walkthrough 255 | §11, §12, §1 | Covered |

## 2. Facts Verified Against Code

| Fact cited by the plan | Code location | Actual value | Match |
|---|---|---|---|
| User unit sandbox line set pinned | `tests/invariants/src/remote_push_contract.rs` `SENDER_UNIT_LINES` (38 lines) | equals `packaging/soos-push-sender.service` | yes (file left unchanged) |
| Sender reads only `XDG_RUNTIME_DIR` | RSH12 `test_rsh_s7_sender_reads_no_other_environment_variable` | exactly one `env::var_os("XDG_RUNTIME_DIR")` in `main.rs` | yes — the plan adds none (`stdin()` instead of `LISTEN_FDS`) |
| `main` order root → probe → XDG → bind | RSH11 `test_rsh_s7_sender_checks_its_sandbox_first` | first occurrences in `fn main` body | yes — the dispatch inserted between `check_not_root(` and `probe_sandbox()` contains none of the pinned needles; the system path lives in another function |
| `serve_connection(stream, own_uid, …)` compares the peer with a passed uid | `crates/push-sender/src/lib.rs` | `credentials.uid() == own_uid` | yes — reused with the instance uid |
| Client checks socket owner and peer uid = owner | `crates/remote/src/push.rs` `UnixPushTransport::exchange` | `meta.uid() != owner_uid`, `peer.uid() != owner_uid` | yes — with socket activation the peer is PID 1 (uid 0): the client must accept uid 0 for the system socket |
| Config default socket | `crates/remote/src/config.rs` `push_config` | `$XDG_RUNTIME_DIR/soos-push/push.sock`, needs the runtime dir | yes — kept (config tests pin it), main maps it to candidates |
| Daemon binary location | `packaging/soos-daemon.service` | `/usr/libexec/soos/soos-daemon` | sender follows `/usr/libexec/soos/soos-push-sender` |
| Packages stage through install.sh | `packaging/debian/rules`, `packaging/rpm/soos.spec` | install.sh `--destdir`; rpm `%files` is an explicit list | rpm `%files` must gain 3 entries (else "unpackaged files") |
| Setup stub harness: system `systemctl` | `setup_user_phase_contract.rs` `STUB_SYSTEMCTL` | system verbs share the daemon's state; `enable` exits 0 | the new system step runs in the harness without a new stub |
| `nix` 0.29 `getsockopt` takes `AsFd`; `getsockname` takes `RawFd` | `Cargo.lock` nix 0.29.0 | yes | safe calls, no `unsafe` |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario: another local user connects to `/run/soos-push/1000/push.sock` to send pushes as uid 1000 → the
  inode is `0600 uid 1000` (EACCES at connect); a socket reached anyway (root) is refused by the peer check unless
  root, which is trusted. PASS.
- Failure scenario: a local user plants a fake sender socket where `soos-remote` looks → `/run/soos-push` and its
  per-uid directory are root-owned `0755`; the legacy fallback is in the user's own `0700` runtime directory; the
  client also requires owner uid and the per-candidate peer uid. PASS.
- Failure scenario: `PrivateUsers=yes` added "for strength" → `SO_PEERCRED` would map the client to 65534 and every
  connection would be refused (or, worse, a check against 65534 would accept anyone unmapped). Excluded explicitly
  (§2.3). PASS.
- Failure scenario: a downgrade from the system socket to the legacy path when the system socket is present but
  refused → the plan uses only the first existing candidate. PASS.

### Pillar 2 — PAM deadline & concurrency
- Not touched (no PAM, no daemon change). PASS.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario: `--system-instance 0`, `--system-instance root`, an extra argument, or the instance uid equal to
  the sender's uid → exit 78 before any socket use. Failure scenario: fd 0 is `/dev/null` (manual run) →
  `listener_from_fd` refuses (not a socket). PASS.

### Pillar 4 — Dependencies
- No new crate; `nix` features `socket`, `fs`, `user` already enabled. `forbid(unsafe_code)` kept (no `from_raw_fd`). PASS.

### Pillar 5 — Data confidentiality
- Failure scenario: a logged socket path or uid → every new `tracing` call has one constant string (RMC-S36 enforces
  it). The resolver bind exposes two read-only text files; no credential. PASS.

### Pillar 6 — Test integrity
- Tests can fail against plausible wrong implementations: `listener_from_fd` is tested with a TCP listener (catches
  a missing family check) and a connected stream (catches a missing `SO_ACCEPTCONN` check); `parse_run_mode` with
  `0`, `4294967295`, leading `+`/zero, extra args; the client with a root-peer requirement on the system candidate and
  an owner-peer requirement on the legacy one (a test socket bound by the test process, peer = own uid, must be
  refused when it sits at the system candidate position).
- **The setup change legitimately contradicts existing tests** (owner decision: the sender is no longer a user
  unit). Listed in §6; not migrated by this branch.

## 4. Findings

- **[MAJOR → resolved in plan]** F-1: With socket activation the listening socket belongs to PID 1, so the existing
  client peer check (peer = owner) would refuse the system sender. Plan §5 makes the expected peer uid a property of
  each candidate (0 for the system socket).
- **[MAJOR → resolved in plan]** F-2: `StandardInput=socket` makes stdout/stderr inherit the socket by default; the
  unit must set `StandardOutput=journal` and `StandardError=journal` (§2.2), otherwise log lines would be written into
  the client connection.
- **[MAJOR → resolved in plan]** F-3: Ubuntu 24.04 `/etc/resolv.conf` → `/run/systemd/resolve/stub-resolv.conf`, hidden
  by `TemporaryFileSystem=/run:ro`: push would still fail on the reporting host after the fix. Plan binds the two
  resolved files read-only (§2.3), narrower than the documented opt-in drop-in.
- **[MINOR]** F-4: `--prefix` other than `/usr` installs the binary where the unit does not look. Plan: install.sh
  warns; setup reports the step "not applied" (read-path rule).
- **[MINOR]** F-5: rpm `%files` must list the new files or rpmbuild fails on unpackaged files (§6).
- **[PROCESS, owner decision needed]** F-6: the existing assertions of §6 pin the user-unit flow of `setup.sh`. They
  are left untouched; the branch is red on them until the owner approves the migration and the lead applies it.

## 5. Verdict
VALIDATION_VERDICT: APPROVED

(Approved for the additive parts and for the setup change as designed; merging is conditional on F-6: the owner must
approve the contract migrations of §6, applied by the lead.)

## 6. Existing assertions contradicted by the setup change (owner-approved migration required)

Observed after the implementation (`cargo test -p soos-invariants`: every other test passes). None of them is changed on
this branch; the full current/required text is in `AI/walkthroughs/255_push_sender_system_service.md` §4.

| Test (`tests/invariants/src/setup_user_phase_contract.rs`) | Current assertion | Required change | Why |
|---|---|---|---|
| `test_set_u6_full_run_sets_up_the_default_on_companion` (SET24) | `uid=1000 systemctl --user enable --now soos-push-sender.service` logged; user unit `active` | `uid=0 systemctl enable --now soos-push-sender@1000.socket` logged between `install.sh` and `install_remote.sh`; user unit not enabled | The owner decision moves the sender to the system manager |
| `test_set_u22_rollback_stops_a_unit_started_by_the_run` (SET33) | Fails the second user unit (`STUB_USER_FAIL_UNIT=soos-push-sender.service`) after `soos-remote.service` started; sender user unit `enabled` | Another failure trigger after the `soos-remote.service` start (a stub knob failing the user's `is-active` of the socket); sender-unit lines removed; `soos-remote.service` assertions unchanged | Only one user unit is started by the companion now |
| `tests/docker/setup_root_preview_test.sh` `companion_ran` (SET34 harness, CI `setup-root-preview`) | `expect_out "[DONE] companion: soos-remote.service soos-push-sender.service running"` | `expect_out "[DONE] companion: soos-remote.service running"` (verified locally: every case passes with only that line changed) | The companion no longer starts the sender user unit |
| `test_set_u21_push_switched_on_later_enables_the_sender` (SET37) | Sender user unit `enabled`/`active` after `--push on`; text `soos-push-sender.service is not enabled` | `uid=0 systemctl enable --now soos-push-sender@<uid>.socket` after `--push on`; a stub knob for a disabled socket and the text `soos-push-sender@<uid>.socket is not enabled` | Same rule ("switching push on enables it like a first write") applied to the socket |
