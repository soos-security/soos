# Walkthrough 92 — Distro Validation Matrix: Valid PAM Stacks, Safe Runner, Socket Invariant

- **Date**: 2026-09-30
- **Issues**: Review findings ONB-04, ONB-05, ONB-10 (GitHub #162, #163, #168) — **Branch**: `test/distro-validation-matrix`
- **Matrix criteria**: DVM1–DVM8 (new)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, area Installation &
Onboarding) found that the multi-distribution test infrastructure could not prove what the
verification matrix claimed:

1. **ONB-04 (#162)** — `tests/docker/Dockerfile.fedora` and `Dockerfile.arch` wrote their PAM
   stacks with `RUN echo "...\n..."`. Docker runs `RUN` through `/bin/sh -c`, which is bash on
   `fedora:40` and `archlinux:latest`; bash's builtin `echo` does not expand `\n`, so
   `/etc/pam.d/test-soos` and `system-auth` were one comment line with no module. The Fedora and
   Arch matrix was never run by CI.
2. **ONB-05 (#163)** — `tests/distro/run_distro_validation.sh` ran
   `bash /workspace/tests/distro/${distro}_test.sh` (scripts that do not exist) in Docker, and when
   Docker was missing it printed "falling back to dry-run verification" but ran the live suite
   (dpkg, install.sh, pam-auth-update, useradd) on the host. After a `dpkg -i` in `auto` mode the
   rollback branch still used `uninstall.sh`, leaving dpkg's database inconsistent.
3. **ONB-10 (#168)** — every deployment test did `chmod 777 /run/soos` and the mock daemon made
   its socket `0666`, contradicting the `0750 root:soos` / `0660` invariant; the distro tests
   "enrolled" by writing 512 random bytes to `<user>.bin`, a file name the store never reads
   (it writes `<uid>.cbor.enc`), and `soos-enroll --mock` was never used.

Objectives: valid multi-line PAM files in every sandbox image plus a static check that fails on
the `echo "\n"` pattern; a runner that only calls existing scripts and whose `--dry-run` (and
no-Docker path) never does anything privileged; deployment tests that respect the socket
invariant and exercise real enrollment hardware-free.

Scope: test infrastructure only (`Dockerfile`, `tests/docker/**`, `tests/distro/**`,
`tests/physical/pam_integration_test.sh`, the CI workflow). `scripts/install.sh` and the
packaging profiles are owned by parallel changes (#161, #164–#167); product bugs revealed by the
new tests are reported in §9, not fixed here.

## 2. Architect Design

No Rust production code is touched.

- **PAM stacks** — every sandbox Dockerfile (root `Dockerfile` used by CI, and
  `tests/docker/Dockerfile.{ubuntu,fedora,arch}`) writes its stacks with
  `printf '%s\n' '<line>' ...`, which behaves identically under dash and bash. The root and Ubuntu
  images were not broken (dash expands `\n`), but the rule is uniform so that the check can be
  simple and a future base-image change cannot reintroduce the bug.
- **In-container guard** — `tests/docker/test_suite.sh` gains `assert_pam_stack_file` (no
  literal `\n`, at least 2 module lines, separate `pam_soos.so` and `pam_unix.so` auth lines),
  called on `test-soos` and the distro stack before T1. It always runs `cargo build` (no reuse of
  a stale `.so`), and `run_matrix.sh` mounts a per-distribution Docker volume over
  `/workspace/target` so an artifact linked on one distribution is never loaded on another.
- **Runner** — `run_distro_validation.sh` maps distributions to scripts explicitly
  (`distro_script`), resolves Docker through `SOOS_DOCKER` (default `docker`), fails with exit 1
  when Docker is missing, and passes `--allow-host-changes` only to the script running inside the
  disposable container (with a `soos-distro-target-<distro>` volume over `target/`).
  `--skip-docker` live runs require the operator to pass `--allow-host-changes`.
- **Distro scripts** — a consent guard (`--allow-host-changes`, exit 2) runs before the root
  check, the EXIT trap or any command; the package branches set `INSTALL_MODE` to
  `deb`/`rpm`/`pkgbuild`; `/run/soos` is created with `install -d -m 0750 -o root -g soos`;
  `start_mock_daemon` waits (bounded) for the socket and `assert_socket_modes` checks
  `750 root:soos` and `660 root:soos`; enrollment is
  `soos-enroll --mock enroll --username "${TEST_USER}" --yes`, followed by a
  `600 root:root` check of `/var/lib/soos/biometrics/<uid>.cbor.enc` and
  `soos-enroll --mock verify`.
- **Mock daemon** — `mock_daemon.py` gains `--socket-group` (default `soos`) and
  `--socket-mode` (default `0660`); any "other" bit or a missing group is a hard error; the socket
  is bound under `umask 0117` so it is never briefly world-accessible.
- **CI** — new job `distro-pam-matrix` (`fedora`, `arch`) runs `tests/docker/run_matrix.sh` on
  push to `main` and manual dispatch. It is not in `ci-success` because it is skipped on pull
  requests; the static half of the contract runs in the `test` job on every PR.

## 3. Plan Evaluation

Condensed (test-infrastructure change, no production crate touched). Checked against the review
recommendations: heredoc/`printf` fix and the lint from ONB-04; explicit mapping, no silent
fallback and a host-mutation consent flag from ONB-05; `install -d -m 0750 -o root -g soos`,
`soos-enroll --mock enroll --username testuser`, `<uid>.cbor.enc` mode `0600` and CI wiring from
ONB-10. `COPY <<EOF` heredocs were rejected: the repository `.dockerignore` sends an empty build
context and BuildKit heredocs are not guaranteed on every local `docker build`; `printf` has
neither constraint.

## 4. Tester Contract

New module `tests/invariants/src/distro_matrix.rs` (registered with one `mod` line in
`tests/invariants/src/lib.rs` to avoid conflicts with parallel edits):

| Test | Contract |
|---|---|
| `test_sandbox_dockerfiles_produce_multiline_pam_stacks` | Extracts each `RUN` writing `/etc/pam.d/` (Docker continuation and comment rules), runs it with `bash -c` into a scratch dir, parses every generated file (DVM1) |
| `test_sandbox_dockerfiles_never_rely_on_echo_escape_expansion` | No `RUN` combines `echo` and `\n`, no `echo -e` (DVM1) |
| `test_pam_matrix_suite_validates_stack_files_before_running` | `assert_pam_stack_file /etc/pam.d/test-soos` is called before T1 (DVM2) |
| `test_ci_runs_fedora_and_arch_pam_matrix` | `ci.yml` job `distro-pam-matrix` covers fedora and arch via `run_matrix.sh` (DVM3) |
| `test_distro_runner_references_only_existing_scripts` | Every `.sh` path in the runner exists, none is interpolated (DVM4) |
| `test_distro_runner_dry_run_executes_nothing_privileged` | `--dry-run all` with 30 recording shims first in `PATH`: exit 0, zero invocations, 3 plans (DVM5) |
| `test_distro_runner_without_docker_refuses_live_host_run` | `SOOS_DOCKER=/nonexistent`: each target fails, zero invocations, no distro banner (DVM5) |
| `test_distro_scripts_refuse_host_mutation_without_consent` | `--skip-docker ubuntu` and each distro script without consent: refusal naming the flag, exit 2, zero invocations (DVM5) |
| `test_distro_runner_grants_consent_only_inside_container` | The flag appears in the `docker run --rm` invocation (DVM5) |
| `test_distro_scripts_record_the_install_mode_actually_used` | `dpkg -i` / `rpm -i` / `pacman -U` branches set `INSTALL_MODE` (DVM6) |
| `test_deployment_tests_never_widen_socket_permissions` | No `chmod 777/666/a+w/o+w`, `0o666`, `0o777` in the deployment test tree (DVM7) |
| `test_deployment_tests_create_socket_dir_with_invariant_modes` | `install -d -m 0750 -o root -g soos /run/soos` and `assert_socket_modes` in the suite and the 3 distro scripts (DVM7) |
| `test_mock_daemon_socket_is_group_restricted` | Spawns `mock_daemon.py`: socket mode `0660`; `0666`, `0777`, `0662` refused without creating a socket (DVM7) |
| `test_distro_tests_enroll_through_soos_enroll_mock` | No `/dev/urandom`, no `.bin`; `soos-enroll --mock enroll`, `.cbor.enc`, `600 root:root` (DVM8) |

**Red evidence** (`cargo test -p soos-invariants distro_matrix`, commit `ddf0b20`, before any
fix): 12 failed, 1 passed. Selected messages:

- `Dockerfile -> /etc/pam.d/common-auth contains a literal '\n' sequence` (bash rebuild of the
  root image stack; the Fedora/Arch stacks fail the same way);
- `run_distro_validation.sh builds a script path from a variable ('/workspace/tests/distro/${distro}_test.sh')`;
- `runner 'ubuntu' without Docker executed: docker build -f .../Dockerfile.ubuntu ...` (the shim
  caught the real call: `SOOS_DOCKER` did not exist and the runner used whatever `docker` it found);
- `mock_daemon.py:93 uses '0o666'`; `mock_daemon.py: error: unrecognized arguments: --socket-group`;
- `debian_ubuntu_test.sh fabricates a template from /dev/urandom`.

The one test green before the fix (`--dry-run all` executes nothing) is a regression guard: the
dry-run branch itself was already inert, the bug was the no-Docker fallback.
`test_ci_runs_fedora_and_arch_pam_matrix` was added with the CI job (the job did not exist, so it
could not pass before).

No existing test was modified. `test_pam_docker_suite_proves_release_panic_returns_pam_ignore`
pins `--target-dir target/fault-injection`; the per-distribution isolation therefore uses a Docker
volume over `target/` rather than `CARGO_TARGET_DIR`.

## 5. Auditor Constraints

1. The consent guard runs before the root check, the EXIT trap and any command, so a refused run
   cannot even delete `/run/soos/daemon.sock` on the host.
2. `mock_daemon.py` fails closed: a missing group or a mode with an "other" bit exits non-zero
   before any directory or socket is created; the socket is bound under `umask 0117`.
3. The runner never downgrades a live request to anything else: no Docker means exit 1 and zero
   executed commands.
4. No secret, embedding or frame is logged; the only credential is the documented throwaway
   `testuser:password123` inside disposable images.
5. `printf '%s\n'` uses a constant format string; stack lines are arguments, so `%` or `\` in a
   PAM line cannot be interpreted.
6. Containers write root-owned files only into Docker volumes, never into the host `target/`.
7. Cleanup stops the mock by PID first (`fedora:40` ships without `pkill`; the Fedora image now
   installs `procps-ng`).

## 6. Implementation

- `Dockerfile`, `tests/docker/Dockerfile.{ubuntu,fedora,arch}`: `printf '%s\n'` stacks. Sandbox
  dependencies needed to reach the end of the distro tests: Ubuntu `libssl-dev` (openssl-sys) and
  `systemd` (declared `Depends:` of the `.deb`); Fedora `openssl-devel`, `rpm-build`,
  `systemd-rpm-macros`, `cargo`, `rust` (RPM `BuildRequires`; `RUSTUP_INIT_SKIP_PATH_CHECK=yes`
  keeps the rustup toolchain first in `PATH`, asserted in the image), `systemd` (RPM `Requires`),
  `authselect`, `procps-ng`.
- `tests/docker/test_suite.sh`: `assert_pam_stack_file`, always-build, `soos` group,
  `install -d -m 0750 -o root -g soos /run/soos`, `start_mock_daemon` + `assert_socket_modes`,
  PID-based cleanup.
- `tests/docker/run_matrix.sh`: `soos-matrix-target-<distro>` volume over `target/`.
- `tests/docker/mock_daemon.py`: `--socket-group`, `--socket-mode`, fail-closed validation, umask.
- `tests/distro/run_distro_validation.sh`: explicit mapping, `SOOS_DOCKER`, no live fallback,
  `--allow-host-changes`, per-distro target volume.
- `tests/distro/{debian_ubuntu,fedora_rhel,arch_linux}_test.sh`: consent guard, `INSTALL_MODE`
  recording, socket invariant, real mock enrollment and verification; Fedora selects the main
  RPM (`soos-[0-9]*.rpm`), never `soos-debuginfo`/`soos-debugsource`.
- `tests/physical/pam_integration_test.sh`: socket dir `0750`, mock socket group = invoking user's
  group.
- `.github/workflows/ci.yml`: `distro-pam-matrix` job.
- Docs: `Docs/PAM_DOCKER_TEST_MATRIX.md` (sandbox invariants, per-distro volumes, CI job),
  `Docs/DISTRIBUTION_DEPLOYMENT.md` (harness safety rules, correct `soos-enroll enroll --username`
  command and `<uid>.cbor.enc` path), `Docs/CI_CD_AND_SECURITY.md` (job table).

## 7. Candid Review

`./scripts/candid_review.sh` (layer 1) passed on every commit. The fingerprint-bound layer 2
(`candid_subagent.sh`) is run by the orchestrator before push, per the task instructions.

## 8. Verification Results

Gate: `cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets
--all-features -- -D warnings`, `cargo test --locked --workspace --all-targets --all-features
--no-fail-fast` (47 invariant tests, 14 new), `./scripts/candid_review.sh`, `bash -n` and
ShellCheck (`--severity=warning`, no new finding) on every touched script.

Docker runs (from a scratch copy of the tree, never the worktree):

| Run | Result |
|---|---|
| `run_matrix.sh fedora` | PAM stack files validated (`test-soos` 4 lines, `system-auth` 5 lines); T1–T10 pass |
| `run_matrix.sh arch` | PAM stack files validated; T1–T10 pass |
| `run_matrix.sh ubuntu` | T1–T10 pass |
| Root `Dockerfile` (CI `pam-integration` image) + `test_suite.sh` | T1–T10 pass |
| `run_distro_validation.sh ubuntu` (`.deb` path) | First run: package built and installed, then **stopped at "pam_soos.so module not found"** (product bug P1, §9). **Fixed by `2fbe969`; re-run passes end to end (§8.2)** |
| `debian_ubuntu_test.sh --install-sh` (ubuntu:24.04 image) | Passes end to end: install, `soos-enroll --mock enroll` → `1001.cbor.enc` `600 root:root`, `verify` OK, socket modes asserted, facial auth, fallback, wrong password rejected, rollback with template preserved |
| `run_distro_validation.sh arch` (`pacman -U` path) | First run: package built, then **`pacman -U` refused: `/usr/lib64 exists in filesystem (owned by filesystem)`** (product bug P1, §9). **Fixed by `2fbe969`; re-run passes end to end (§8.2)** |
| `arch_linux_test.sh --install-sh` (archlinux image) | Passes end to end (enrollment `1000.cbor.enc` `600 root:root`, swaylock/hyprlock facial unlock and fallback, rollback) |
| `run_distro_validation.sh fedora` (RPM path) | Passes end to end (§8.1) |

### 8.1 Fedora distribution run

`bash tests/distro/run_distro_validation.sh fedora` in the rebuilt `fedora:40` image: RPM built
(`rpmbuild`) and installed with `rpm -i`, `INSTALL_MODE=rpm` recorded, filesystem invariants
verified, `authselect select custom/soos with-faillock --force` activated with faillock order and
`nsswitch.conf` intact, `soos-enroll --mock enroll` → `1000.cbor.enc` `600 root:root` and `verify`
accepted, socket modes asserted, sudo and gdm stacks authenticated by the mock daemon and fell
back to password, rollback through `rpm -e soos` with the `local with-silent-lastlog` profile
restored. Earlier attempts in this change surfaced the sandbox gaps fixed in §6 (missing OpenSSL
headers, RPM `BuildRequires`/`Requires`, `pkill`, `authselect`) and the debuginfo glob (P2).

### 8.2 Post-fix re-runs and CI wiring (batch `fix/p1-install-batch`)

P1 was fixed by commit `2fbe969` (`fix(packaging): stage pam_soos.so in the distribution pam
directory`): `build_deb.sh` and `debian/rules` pass `--pam-dir
/usr/lib/<DEB_HOST_MULTIARCH>/security`, `build_arch.sh` passes `--pam-dir /usr/lib/security`
(pinned by `test_packaging_passes_explicit_distro_pam_dir`). After that commit, in Docker from a
scratch clone:

| Run | Result |
|---|---|
| `./tests/distro/run_distro_validation.sh ubuntu` | exit 0, end to end; module staged and installed at `/usr/lib/x86_64-linux-gnu/security/pam_soos.so` |
| `./tests/distro/run_distro_validation.sh arch` | exit 0, end to end; `pacman -U` accepted, module at `/usr/lib/security/pam_soos.so` |

The review round then closed the root cause too: `install.sh --destdir` without `--pam-dir` no
longer consults the build host (stage-only probe, then `/usr/lib/security` with a warning;
`test_install_destdir_never_guesses_pam_dir_from_build_host`, walkthrough 93 §8).

CI (GitHub #168): the new `package-deploy` job in `.github/workflows/ci.yml` runs on every pull
request after `lint` and is part of `CI Success`. It runs `./tests/distro/run_distro_validation.sh
ubuntu` (release workspace build, `.deb` built and installed with `dpkg -i`, filesystem
invariants, `soos-enroll --mock enroll`, facial auth, password fallback, rollback), then
`tests/docker/test_packages.sh` in the same `soos-distro-val-ubuntu` image on the same
`soos-distro-target-ubuntu` volume (no key material in the `.deb`, `0600` 32-byte key generated
on the host, key kept after `dpkg -r`, distinct keys across fresh installs). The job and its
membership in `CI Success` are pinned by `test_ci_runs_ubuntu_package_deployment_on_pull_requests`
(red before the job existed: "ci.yml must define the package-deploy job"). Both job commands were
run locally from a scratch clone of the branch head: see the results recorded in walkthrough 93 §8.

## 9. Known Limitations / Follow-ups

Product bugs revealed by the new tests (reported here; P1 was fixed in the batch integration,
see §8.2):

- **P1 (FIXED by `2fbe969`, §8.2) — packages install `pam_soos.so` into `/usr/lib64/security`.** `scripts/install.sh`
  resolves `PAM_DIR` inside the empty `--destdir` stage, finds nothing and falls back to
  `/usr/lib64/security` because the *build host* has `/usr/lib64`. On Debian/Ubuntu Linux-PAM
  loads modules from `/usr/lib/x86_64-linux-gnu/security`, so the `.deb` ships a module PAM never
  loads (facial auth silently absent); on Arch `/usr/lib64` is a symlink owned by `filesystem`, so
  `pacman -U` refuses the package. Fix: pass an explicit `--pam-dir` from `build_deb.sh`
  (`/usr/lib/$(dpkg-architecture -qDEB_HOST_MULTIARCH)/security`), `build_arch.sh`
  (`/usr/lib/security`) and the RPM spec (`%{_libdir}/security`), and never guess under
  `--destdir`.
- **P2 — `build_rpm.sh` copies `soos-debuginfo` and `soos-debugsource` next to the main RPM** in
  `target/packages`; any `soos-*.rpm` glob picks the wrong one (the test now selects
  `soos-[0-9]*.rpm`).
- The `.deb`/RPM `Depends`/`Requires: systemd` and the RPM `BuildRequires: cargo, rust` are
  satisfied in the sandbox images by installing those packages; `build_rpm.sh --skip-build` still
  requires distribution `cargo`/`rust` packages for `rpmbuild`'s dependency check.

Other limitations:

- The native-package paths of DV1 (Ubuntu `.deb`) and DV3 (Arch package) are green again since
  `2fbe969` (§8.2).
- CI coverage (GitHub #168): Ubuntu runs on every pull request in `package-deploy` (§8.2). The
  Fedora (RPM) and Arch deployment suites and the RPM/Arch branches of
  `tests/docker/test_packages.sh` are NOT in CI yet (one full release build per distribution);
  their evidence is the manual Docker runs above, so matrix rows PK6, PK7, DV2 and DV3 stay
  `⬜ Pending` until a push-to-`main` job (like `distro-pam-matrix`) runs them.
- Docker volumes `soos-matrix-target-*` and `soos-distro-target-*` persist between runs as a
  build cache; remove them with `docker volume rm` when needed.
