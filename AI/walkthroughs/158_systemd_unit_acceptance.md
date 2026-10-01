# Walkthrough 158 — soos-daemon.service Under a Real systemd (PID 1)

- **Date**: 2026-10-01
- **Issue**: GitHub #211 (ONB-15), residual acceptance row IRP8 (no backlog id)
- **Branch**: `test/systemd-unit-acceptance`
- **Matrix criteria**: IRP8 (now verified), SUA1–SUA7 (new component `systemd-unit-acceptance`)
- **ADR**: 2026-10-01 "systemd Unit Acceptance in a Privileged systemd Container, Models Downloaded in CI"

---

## 1. Context

Walkthroughs 116 and 139 added the start guards of `packaging/soos-daemon.service`
(`ConditionPathExists=/var/lib/soos/models/manifest.toml`, bounded restarts) and checked them
textually and with `systemd-analyze condition`. Walkthrough 140 switched the unit to
`Type=notify`; walkthrough 147 raised `StartLimitIntervalSec` to 320 so that five timed-out
starts still reach the limit. What no test had observed is the unit under a real systemd
manager: IRP8 stayed `⬜ Pending (booted systemd host or privileged systemd container required)`.

This branch adds that evidence. No production code and no shipped unit value changes; no
existing test is modified.

## 2. Design

- `tests/docker/Dockerfile.systemd`: `ubuntu:24.04` plus `systemd`, `systemd-sysv`, `curl`,
  `ca-certificates`; gettys, logind, timers and network units masked; `STOPSIGNAL SIGRTMIN+3`;
  `CMD` deletes `/dev/video*`, `/dev/media*` and `/dev/v4l`, then `exec /sbin/init`. No `COPY`
  (empty build context).
- `tests/docker/systemd_unit_acceptance_test.sh` (host mode):
  1. builds the `tests/docker/Dockerfile.ubuntu` image and runs
     `cargo build --locked --release -p soos-daemon -p soos-admin-cli` in it every time (target
     and registry in Docker volumes, workspace mounted read-only): same glibc as the runtime
     image, and never a stale artifact (artifact freshness, QFX1);
  2. boots the runtime image `--privileged --cgroupns=private --tmpfs /run --tmpfs /run/lock`
     with the workspace and the target volume read-only;
  3. stage `install`: live `scripts/install.sh --artifact-dir /target/release --allow-missing
     --skip-models --distro none` (creates the `soos` group, the directories, `master.key`,
     installs the unit and enables it), then parts 4 and 1;
  4. `docker restart` (systemd shutdown, fresh `/run`, persistent `/var`), stage
     `after-reboot`: part 1 at boot, part 3, then parts 2 and 5 when models are available.
- Models (`--models`): `auto` (default) uses `host` when `/var/lib/soos/models/manifest.toml`
  is readable on the host, else `none`; `host` bind-mounts that directory read-only and copies
  the `.onnx` files into the container, after which `scripts/download_models.sh` finds them
  "Already deployed and verified" and writes the manifest (no network); `download` lets
  `download_models.sh` fetch them (CI); `none` prints `SKIPPED` for parts 2 and 5.
- Camera: `/etc/soos/daemon.toml` with `[pipeline] use_mock_camera = true` for part 2.
- Pipeline safety: `has()` replaces `| grep -q` so that an early grep exit never SIGPIPEs the
  writer under `pipefail`; every wait is bounded (boot 90 s, start limit 120 s).

## 3. Tests First

`tests/invariants/src/systemd_unit_acceptance_contract.rs` (SUA1–SUA6) pins: the harness exists,
is executable strict bash and passes `bash -n`; host isolation (`--privileged
--cgroupns=private`, read-only workspace and host models, no `sudo`, mock camera, Dockerfile
removes the V4L2 nodes); the unconditional locked build and the `scripts/install.sh` install, the
`cmp` with the shipped unit, `systemd-analyze verify`, no drop-in; the condition path equals the
unit's `ConditionPathExists=`; `SHIPPED_START_LIMIT_BURST` / `SHIPPED_START_LIMIT_INTERVAL_S`
equal the unit's values; the socket check follows `systemctl start` in part 2, the journal order
bind → READY → Started, `MAX_READY_MS` at most half of `TimeoutStartSec`; the log lines the
harness keys on still exist in `crates/daemon/src/main.rs`; the CI job and its `CI Success`
membership.

Red evidence: with the harness moved away,
`cargo test --locked --all-features -p soos-invariants systemd_unit_acceptance` gave
`1 passed; 5 failed` (only the CI test passed, the job being already written); green afterwards.

Harness red evidence: with `ConditionPathExists=` commented out in a scratch edit of the unit
(reverted), the run failed at part 1 with
`systemctl start with an unmet condition exited 1 (expected 0: a skipped start is not a failure)`
and the journal showed the daemon running and failing.

## 4. Audit

- Host isolation: no host path is written; the workspace, the target volume and the host models
  are mounted read-only; the release build writes only into Docker volumes. The privileged
  container gets host device copies, so the image removes the V4L2 nodes before systemd starts
  (`install.sh`'s informational camera listing then reports `probe_failed: not_found`) and the
  daemon uses the mock camera.
- Models are never committed: they are copied from a read-only mount or downloaded inside the
  container and vanish with it.
- No production code, no PAM or IPC path, no logging change.

## 5. Results (2026-10-01, i7-13620H, 16 threads, systemd 255.4-1ubuntu8.17)

Five green runs: three `--models host`, one `--models download`, one `--models none`.

| Part | Evidence (harness output) |
|---|---|
| 4 | `systemd-analyze verify soos-daemon.service: exit 0, no unknown, ignored or invalid directive`; all hardening properties and the shipped values loaded; `Overall exposure level for soos-daemon.service: 1.7 OK` |
| 1 | `systemctl start` exit 0, `ConditionResult=no`, `inactive (dead)`, `Condition: start condition unmet`, 0 executions, `NRestarts=0` after 6 s; journal `skipped because of an unmet condition check (ConditionPathExists=/var/lib/soos/models/manifest.toml)` |
| 1 (boot) | after `docker restart`: enabled unit skipped at boot, `ConditionResult=no`, no execution |
| 3 | `ran exactly 5 times (NRestarts=5)`, `Start request repeated too quickly` after 11 321–11 397 ms; a manual start is refused without running the daemon; journal `ChecksumMismatch` and `refusing to start daemon (fail-closed)` |
| 2 | `systemctl start` returned after 706–851 ms wall; socket `660 root:soos`, `/run/soos` `750 root:soos`; journal bind → `Reported readiness to systemd` → `Started`; **start-to-ready 685, 689, 755 ms (host models) and 826 ms (downloaded)**; `soos-admin` JSON `is_healthy: true`, `memory_locked: true` |
| 5 | `systemctl stop` 65–72 ms (`TimeoutStopSec=1min 30s`), `Result=success`, exit 0, socket removed, no SIGKILL |

Finding (no bug): systemd 255 keeps `Result=exit-code` after the start limit is hit, because a
service only records `start-limit-hit` when no earlier failure result is set. The start limit is
therefore asserted through the journal line "Start request repeated too quickly", the exact run
count, `ActiveState=failed` and the refused manual start; the harness accepts
`Result=start-limit-hit` too.

## 6. CI Decision

Job `systemd-unit` (after `lint`, SHA-pinned checkout, `persist-credentials: false`, 30 min) runs
`--models download`: without models, part 5 (stop) and part 2 (the only proof that the daemon
reaches `READY=1` inside the real sandbox) could not run. The download is bounded and
checksum-verified by `scripts/download_models.sh`. The job is part of `CI Success` because it was
deterministic in every local run; its new external dependency is the model hosts (Hugging Face,
GitHub), comparable to the apt, crates.io and rustup downloads of the other Docker jobs.

## 7. Remaining Work

- SUA7: measure start-to-ready on the slowest supported target CPU with the real camera (the
  CI runner measurement appears in the job log).
- DHX4 (GDM waiting for the socket on a real host) stays pending.
