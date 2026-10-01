# PAM Docker Test Matrix Specification — soos

## 1. Overview & Objectives

The PAM Docker Test Matrix provides a comprehensive, multi-distribution verification environment for `pam_soos.so` under real Linux-PAM stacks. It validates that:
1. **Nominal Verification**: When `soos-daemon` is running and authorizes the user, Linux-PAM grants access non-interactively (`PAM_SUCCESS`, 0 password prompts).
2. **Timeout Resilience (PA2)**: When the daemon answers after the `timeout_ms` of the PAM line, `pam_soos.so` degrades cleanly to `PAM_IGNORE` within that deadline (asserted elapsed bound), allowing password fallback; a late `Allow` is never honored.
3. **Crash Resilience (PA9)**: When the daemon terminates or severs connections mid-request, `pam_soos.so` detects disconnect/EOF and degrades to `PAM_IGNORE`.
4. **Cross-Distribution Portability (PA10)**: Validates PAM stack inclusion patterns across Ubuntu (Debian), Fedora (RHEL), and Arch Linux.

---

## 2. Supported Distributions & Configurations

| Distribution | Base Image | PAM Modules Directory | Target PAM Service File |
|---|---|---|---|
| **Debian / Ubuntu** | `ubuntu:24.04` | `/lib/x86_64-linux-gnu/security` | `/etc/pam.d/common-auth` |
| **RHEL / Fedora** | `fedora:40` | `/usr/lib64/security` | `/etc/pam.d/system-auth` |
| **Arch Linux** | `archlinux:latest` | `/usr/lib/security` | `/etc/pam.d/system-auth` |

### PAM Service Inclusion Patterns

#### Debian / Ubuntu (`/etc/pam.d/common-auth`)
```pam
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250
auth  [success=1 default=ignore]    pam_unix.so nullok
auth  requisite                      pam_deny.so
auth  required                       pam_permit.so
```

#### RHEL / Fedora (`/etc/pam.d/system-auth`)
```pam
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250
auth  sufficient                     pam_unix.so try_first_pass nullok
auth  required                       pam_deny.so
```

#### Arch Linux (`/etc/pam.d/system-auth`)
```pam
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250
auth  required                       pam_unix.so try_first_pass nullok
```

---

## 3. Test Scenarios Matrix

| Test ID | Scenario | Injected Condition | Expected PAM Result | Fallback Behavior |
|---|---|---|---|---|
| **T1** | Nominal Facial Auth | Daemon running, returns `Verdict::Allow` | `PAM_SUCCESS` (0) | None (instant authorization, 0 prompts) |
| **T2** | Daemon Timeout (PA2, GitHub #189) | `timeout_ms` read from `/etc/pam.d/test-soos` (`pam_stack_timeout_ms`, module default 1000 and clamp 10..5000 applied); mock `timeout` mode answers `Allow` after `timeout_ms + 2500` ms | `PAM_IGNORE` (25) | Fallback to `pam_unix`, valid password accepted; the run must finish within `timeout_ms + 1000` ms (`assert_elapsed_within_deadline`), so a module that waits for the daemon fails |
| **T2b** | Late Allow never honored | Same slow daemon; copy of `test-soos` with `pam_unix.so nodelay`; no password supplied | `PAM_IGNORE` (25) | The password-less run must fail (no facial authorization from the late `Allow`) within the same deadline bound |
| **T3** | Timeout Password Rejection | Daemon slower than `timeout_ms` | `PAM_IGNORE` (25) | Fallback to `pam_unix`, invalid password rejected |
| **T4** | Daemon Immediate Crash | Daemon closes stream upon `accept()` | `PAM_IGNORE` (25) | Fallback to `pam_unix`, valid password accepted |
| **T4b** | Daemon Partial Crash | Daemon closes stream after 2 header bytes | `PAM_IGNORE` (25) | Fallback to `pam_unix`, valid password accepted |
| **T5** | Crash Password Rejection | Daemon crashes mid-request | `PAM_IGNORE` (25) | Fallback to `pam_unix`, invalid password rejected |
| **T6** | Distro Stack Integration (GitHub #189) | Native `common-auth` / `system-auth` containing `pam_soos.so` (missing stack is a failure) | `PAM_SUCCESS` with 0 prompts on `Allow`; `PAM_IGNORE` with the daemon offline | Every expectation is a hard failure: facial `Allow` without a password prompt, valid password accepted, wrong password rejected |
| **T7** | Absent Socket | `/run/soos/daemon.sock` does not exist | `PAM_IGNORE` (25) | Fallback to `pam_unix`, valid password accepted |
| **T8** | Missing Module Resilience (GitHub #189) | `pam_soos.so` missing from disk, mock daemon answering `Allow` | Line skipped (`default=ignore` on the unknown module) | Hard failures: valid password accepted, wrong password rejected, no authentication without a password |
| **T9** | Model Deployment Integrity | `scripts/download_models.sh --dry-run` against `models/manifest.toml` | n/a | Manifest and checksums validated in-container |
| **T10** | Release-Build Panic Safety (PAM-01, GitHub #148) | Module rebuilt with the same `[profile.release]` plus the opt-in `fault-injection` feature, loaded as `pam_soos_fault.so` and armed with `fault_inject=panic` then `fault_inject=overflow` | `PAM_IGNORE` (25); host process never killed (exit code < 128, never 134/SIGABRT) | Fallback to `pam_unix`, valid password accepted, invalid password rejected |
| **T11** | Debian password-failed hook (ONB-03, GitHub #161) | Shipped `soos`/`soos-notify` profiles enabled with the real `pam-auth-update --package --force`; mock daemon in `deny` mode with `--record` | `PAM_IGNORE` (25) from both `pam_soos.so` lines | Hook line before `pam_deny.so`; wrong password rejected with exactly one `PasswordFailed` event; correct password accepted with none. Skipped on non-`pam-auth-update` images, fails on a Debian image without it |
| **T12** | `gdm.disable` via `PAM_SERVICE` (PAM-05, GitHub #176) | PAM service `gdm-password` with the `soos-admin` GDM arguments (no `service=`), mock daemon answering `Allow`; control run without the flag, then `/etc/soos/gdm.disable` created | Control: `PAM_SUCCESS` with 0 prompts; with the flag: `PAM_IGNORE` (25) | With the flag the password prompt is reached: no-password run fails, valid password accepted, invalid password rejected |
| **T13** | `Verdict::Deny` (ARCHITECTURE §3, GitHub #189) | Mock `deny` mode | `PAM_IGNORE` (25) | No authentication without a password; valid password accepted, wrong password rejected |
| **T14** | Truncated response (GitHub #189) | Mock `crash-truncated`: length prefix declares 37 bytes, 4 are sent, stream closed | `PAM_IGNORE` (25) | No authentication without a password; valid password accepted, wrong password rejected |
| **T15** | Malformed and stale responses (GitHub #189, #287) | Mock `malformed` (undecodable verdict discriminant), `wrong-request-id` (`Allow` for another request), `bad-version` (`Allow` with version 2), `oversized` (prefix above `MAX_MESSAGE_SIZE`), `empty` (zero-length frame), `expired` (`Allow` whose issued/expires window closed 1 s ago) and an unstamped `Allow` (`--stamps zero`, issued = expires = 0); every other mode runs with `--stamps monotonic` so it carries exactly one defect | `PAM_IGNORE` (25) | For each mode: no authentication without a password and valid password accepted; wrong password rejected after the `wrong-request-id` `Allow` |

T10 exists because `cargo test` runs under `[profile.test]` (always `panic = "unwind"`) and
therefore cannot detect a release profile that aborts; only the release-built shared object
loaded by a real PAM host proves that `catch_unwind` is effective in production. The
fault-injection variant is built into `target/fault-injection/` so the artifact used by every
other case (T1–T9, T11–T15) stays the exact production configuration.

---

## 4. Test Harness Architecture

```
tests/docker/
├── Dockerfile.ubuntu         # Debian/Ubuntu container image
├── Dockerfile.fedora         # RHEL/Fedora container image
├── Dockerfile.arch           # Arch Linux container image
├── pam_test_runner.c        # Native C non-interactive & interactive PAM test harness
├── mock_daemon.py           # Socket simulator: allow, deny, timeout, crashes, malformed and expired frames; `--stamps monotonic` stamps like soos-daemon
├── pam_case_lib.sh          # Shared assertions: stack timeout_ms, deadline bound, no-facial-auth, fallback
├── test_suite.sh            # In-container test suite executing T1..T15
├── run_matrix.sh            # Host driver orchestrating multi-distro builds & runs
└── authselect_profile_test.sh  # Fedora authselect profile activation/rollback (A1..A7)
```

### Execution Commands

- Run full test matrix in default Ubuntu container:
  ```bash
  ./run_tests.sh
  ```
- Run full multi-distribution test matrix across Ubuntu, Fedora, and Arch:
  ```bash
  ./run_tests.sh --matrix
  # Or:
  ./tests/docker/run_matrix.sh all
  ```
- Run targeted distribution test:
  ```bash
  ./tests/docker/run_matrix.sh fedora
  ```
  Each distribution builds into its own Docker volume mounted over `/workspace/target`
  (`soos-matrix-target-<distro>`), so a `pam_soos.so` linked on one distribution is never
  loaded on another. The Fedora and Arch images run in CI in the `distro-pam-matrix` job
  (push to `main` and manual dispatch); the Fedora and Arch deployment paths
  (`tests/distro/run_distro_validation.sh fedora|arch` plus the RPM / Arch branches of
  `tests/docker/test_packages.sh`) run in the `distro-deploy` job on the same triggers.

### Sandbox Invariants (GitHub #162, #168)

- **Multi-line PAM stacks.** The Dockerfiles write `/etc/pam.d/test-soos` and the distro
  stack with `printf '%s\n' 'line' ...`, never `echo "...\n..."`: on `fedora:40` and
  `archlinux:latest` `/bin/sh` is bash, whose builtin `echo` writes a literal `\n`, which
  produced single-line files with no module at all. The invariant test
  `test_sandbox_dockerfiles_produce_multiline_pam_stacks` rebuilds every stack with bash and
  parses it, and `test_suite.sh` (`assert_pam_stack_file`) refuses to run T1–T10 on a
  malformed stack.
- **Production socket modes.** `test_suite.sh` creates the `soos` group and
  `/run/soos` with `install -d -m 0750 -o root -g soos`; `mock_daemon.py` binds its socket
  under a restrictive umask, sets it `0660` and group `soos` (`--socket-group`,
  `--socket-mode`; any "other" bit is refused), and every case asserts `750 root:soos` /
  `660 root:soos` (`assert_socket_modes`) before calling PAM. The socket is bound,
  configured and listening under a private staging name before it is renamed to the
  `--socket` path, so a caller that connects as soon as the path exists is never refused
  (GitHub #293). The `expired` mode closes its window 60 s before the send-time clock read.
- Validate the Fedora `authselect` custom profile (activation with `with-faillock`,
  `authselect check`, generated `system-auth`/`password-auth` ordering, `/etc/nsswitch.conf`
  preserved, password fallback via `pamtester`, rollback through `scripts/uninstall.sh`):
  ```bash
  ./run_tests.sh authselect
  # Or:
  ./tests/docker/authselect_profile_test.sh
  ```

### Failable Assertions (GitHub #189, review TCI-06)

- **No warning escape hatch.** Every case exits non-zero on a failed expectation; T6 and T8
  used to end in `warn` and could never fail.
- **Deadline derived from the stack.** T2/T2b never hard-code a budget: `timeout_ms` is read
  from the stack under test, the mock delay is `timeout_ms + 2500` ms and the accepted
  elapsed time `timeout_ms + 1000` ms (tolerance for process start, `pam_unix` hashing and
  shared CI runners). A packaging change of `timeout_ms` therefore moves both values.
- **Rejection paths.** `assert_no_facial_authorization` runs the stack without a password:
  it must fail, which proves the module returned `PAM_IGNORE` and the late, denied or
  malformed verdict was not honored. `assert_password_fallback` checks the valid and a wrong
  password.
- **Red evidence.** Walkthrough 106 records each strengthened case failing against a
  deliberately broken module or stack (timeout ignored, a mismatched
  `request_id` honored, unknown module fatal, facial line not terminal) while the previous
  suite passed.
