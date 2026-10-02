# Full Project Review — soos (2026-10-02)

- **Base commit**: `095eae9` (`main`, after PR #299)
- **Method**: six independent read-only domain reviewers (PAM & protocol; daemon & policy; vision,
  inference & anti-spoofing; camera & GUI; storage & CLIs; install, packaging, CI & docs). Every
  finding is tied to opened `file:line` evidence and was kept only after the reviewer tried to
  refute it (re-reading callers, tests, invariants; PoCs and reproductions where noted).
- **Context**: second full review after the 2026-09-29 review (`FULL_PROJECT_REVIEW_2026-09-29.md`)
  and the ~25 correction PRs that followed (#272–#299, walkthroughs 78–164).
- **Companion evidence**: `tests/distro/run_distro_validation.sh` passed 100 % on Ubuntu 24.04,
  Fedora 40 and Arch Linux at the same base commit (Docker, non-privileged containers).
- **GitHub tracking**: label `review-2026-10-02`; one issue per CRITICAL/MAJOR finding, one grouped
  issue per domain for MINOR findings and suggestions.

## 1. Summary

| Area | CRITICAL | MAJOR | MINOR | SUGGESTION |
|---|---:|---:|---:|---:|
| PAM module & IPC protocol | 1 | 1 | 2 | 3 |
| Privileged daemon & policy | 0 | 1 | 4 | 2 |
| Vision, inference & anti-spoofing | 0 | 1 | 4 | 3 |
| Camera & GUI | 0 | 3 | 4 | 2 |
| Storage & CLIs | 0 | 1 | 8 | 1 |
| Install, packaging, CI & docs | 1 | 2 | 3 | 1 |
| **Total** | **2** | **9** | **25** | **12** |

No fail-open was found in the daemon's Allow path, the policy crate, the PAD decision logic, the
storage cryptography or the CI hardening. The two CRITICAL findings sit at the edges: identity
resolution inside the PAM module and file ownership in the Arch package.

## 2. CRITICAL and MAJOR findings

| ID | Severity | Issue | Finding |
|---|---|---|---|
| PAM-NEW-1 | CRITICAL | #300 | A failed username-to-UID resolution falls back to the caller's UID (`crates/pam/src/lib.rs:315-323`): in `su B` / polkit flows the daemon authenticates the caller's face for account B (PoC: `PAM_USER=no_such_user -> uid_hint=1000 -> PAM_SUCCESS`). |
| ONB-NEW-1 | CRITICAL | #301 | `scripts/build_arch.sh:144/148` archives without owner 0: a package built by a normal user ships the root daemon and `pam_soos.so` owned by that user (reproduced). |
| PAM-NEW-2 | MAJOR | #302 | The `uid=` module argument overrides PAM_USER without comparison; docs and a log line recommend it. |
| DMN-NEW-1 | MAJOR | #310 | The PasswordFailed evidence write runs inline on a Tokio worker and can wait without bound on the evidence `flock`. |
| VIS-NEW-1 | MAJOR | #304 | GUI guided enrollment (`analyze_frame`) records samples from frames with several faces. |
| CAM-NEW-1 | MAJOR | #305 | The IPC preview drops the IR sensor stamp: IR colour-format frames take the colour PAD path in the GUI. |
| CAM-NEW-2 | MAJOR | #306 | The GUI keeps the last frame and "Ready" when the daemon serves empty previews. |
| CAM-NEW-3 | MAJOR | #307 | The resolver accepts virtual/loopback capture nodes (v4l2loopback, vivid), including on re-resolution. |
| STO-NEW-1 | MAJOR | #303 | Concurrent master/evidence key creation: the second `rename` silently replaces the first key. |
| TCI-NEW-1 | MAJOR | #308 | `./run_tests.sh` writes root-owned `target/release` and `target/fault-injection` into the host checkout. |
| ONB-NEW-2 | MAJOR | #309 | The Arch PAM integration docs and snippets disagree; the documented form reaches `pam_faillock authfail` after a correct password. |

## 3. MINOR findings and suggestions (grouped issues)

| Issue | Domain | Items |
|---|---|---|
| #311 | PAM / protocol | PAM-NEW-3 (tagged encoders re-allocate nonce copies), PAM-NEW-4 (fail-quiet docs drift), PAM-NEW-5..7 (panic-arm accessors, blocking `getrandom`, unlogged IPC rejections, deadline start) |
| #312 | Storage / CLIs | STO-NEW-2..10 (enroll replace race, test-pam exit code and deadline, evidence key always created, FIFO template read, stale `gdm restore`, unbounded `logs --file`, unzeroized guided-enrollment samples, unbounded evidence locks) |
| #313 | Vision / inference | VIS-NEW-2..8 (zero-dimension panic, non-finite embeddings, undeclared input layouts, unwiped buffers, derived Debug, SCRFD channel-order docs, stale docs) |
| #314 | Camera / GUI | CAM-NEW-4..7 + S1/S2 (NUL in `camera_device`, `--mock` writing real templates, unzeroized GUI frames, daemon state probe, relative pkexec programs, mock docs, preview format/length checks) |
| #315 | Daemon | DMN-NEW-2..5 + suggestions (sticky inference estimate, `enforce_active_session=false` in production, config loader hardening, docs drift, socket parent validation, sd_notify docs) |
| #316 | Install / CI | ONB-NEW-3 (two Arch packaging paths, unit under `/etc`, `/run` shipped), ONB-NEW-4 (`Default: yes` pam-configs on the install.sh path), TCI-NEW-2 (tautological priority invariant), TCI-NEW-3 (unretried ORT download, unpinned base images) |

## 4. Areas checked and found clean (highlights)

- **PAM**: catch_unwind on all six entries, `panic = "unwind"` + overflow checks in release, single
  cumulative deadline re-armed per syscall, non-blocking connect, no threads/Tokio, no stdout/stderr,
  response freshness and request_id binding, bounded codec with fuzz targets, bounded config parsing.
- **Daemon/policy**: peer UID and session binding, template binding before reservation, atomic
  rate-limit reservation, PAD consensus with spoof veto, non-finite scores rejected, threshold
  parity, bounded reads and tables, bounded shutdown, readiness ordering, hashed nonces in logs.
- **Vision/inference**: PAD BGR raw 0–255 with live class index 1, SFace RGB raw NCHW with in-graph
  normalization, SCRFD decode and NMS, alignment geometry, attestation of loaded bytes, optional and
  retired manifests never read at runtime.
- **Camera/GUI**: guarded and bounded V4L2 calls, buffer validation, resolver determinism, `O_PATH`
  config reader, UI-thread isolation of store and pkexec work, preview authorization and freshness.
- **Storage/CLIs**: AES-GCM nonces and AAD binding, key file checks, atomic 0600 writes, bounded
  store lock, migration identity checks, descriptor-based temp sweeps, gdm stack editing safety.
- **Install/CI**: transactional installer with rollback, no key material in packages, pinned and
  verified downloads, SHA-pinned actions with least privilege, isolated privileged systemd test.
