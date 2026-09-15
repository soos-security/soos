# PAM Docker Test Matrix Specification — soos

## 1. Overview & Objectives

The PAM Docker Test Matrix provides a comprehensive, multi-distribution verification environment for `pam_soos.so` under real Linux-PAM stacks. It validates that:
1. **Nominal Verification**: When `soos-daemon` is running and authorizes the user, Linux-PAM grants access non-interactively (`PAM_SUCCESS`, 0 password prompts).
2. **Timeout Resilience (PA2)**: When the daemon hangs or exceeds its 200–250ms deadline, `pam_soos.so` degrades cleanly to `PAM_IGNORE`, allowing password fallback.
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
| **T2** | Daemon Timeout | Daemon delays response > 250ms | `PAM_IGNORE` (25) | Fallback to `pam_unix`, valid password accepted |
| **T3** | Timeout Password Rejection | Daemon delays response > 250ms | `PAM_IGNORE` (25) | Fallback to `pam_unix`, invalid password rejected |
| **T4** | Daemon Immediate Crash | Daemon closes stream upon `accept()` | `PAM_IGNORE` (25) | Fallback to `pam_unix`, valid password accepted |
| **T4b** | Daemon Partial Crash | Daemon closes stream after 2 header bytes | `PAM_IGNORE` (25) | Fallback to `pam_unix`, valid password accepted |
| **T5** | Crash Password Rejection | Daemon crashes mid-request | `PAM_IGNORE` (25) | Fallback to `pam_unix`, invalid password rejected |
| **T6** | Distro Stack Integration | Native `common-auth` / `system-auth` | `PAM_SUCCESS` / Fallback | Verified on native distribution stack |
| **T7** | Absent Socket | `/run/soos/daemon.sock` does not exist | `PAM_IGNORE` (25) | Fallback to `pam_unix`, valid password accepted |
| **T8** | Missing Module Resilience | `pam_soos.so` missing from disk | `PAM_IGNORE` | PAM stack continues functional operation |

---

## 4. Test Harness Architecture

```
tests/docker/
├── Dockerfile.ubuntu         # Debian/Ubuntu container image
├── Dockerfile.fedora         # RHEL/Fedora container image
├── Dockerfile.arch           # Arch Linux container image
├── pam_test_runner.c        # Native C non-interactive & interactive PAM test harness
├── mock_daemon.py           # Socket simulator for allow, timeout, and mid-stream crash
├── test_suite.sh            # In-container test suite executing T1..T8
└── run_matrix.sh            # Host driver orchestrating multi-distro builds & runs
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
