# Plan Evaluation Report (round 2)
- **Date**: 2026-10-02
- **Issue**: GitHub #327 — documented and tested upgrade path for local installs (GitHub-only, no `AI/BACKLOG.md` entry)
- **Branch**: `chore/upgrade-procedure`
- **Base commit**: `bcb73de`
- **Plan**: `AI/architect_spec_upgrade_procedure.md`

## 0. Findings of Round 1 and Resolution
| Round | Finding | Resolution | Status |
|---|---|---|---|
| 1 | P1 MAJOR — F1 did not say what happens with `--skip-systemd` or under `--destdir` while the host unit is active | Spec §2 F1: the state is recorded only on a live install with systemd, and step 9 requires `UNIT_ENABLED = true`; `--skip-systemd` / `--destdir` never restart anything | Resolved |
| 1 | P2 MAJOR — F3 "disabled by the administrator" must not match a first install on a locally modified stack (pam-auth-update exits before writing `seen`) nor a reinstall after `dpkg -r` (`--remove` drops the profile from `seen`) | Verified against `/usr/sbin/pam-auth-update` of ubuntu:24.04 (lines 132–181, 261: `seen` is rewritten from the profile list minus removals, after the local-modification exit); UPG5 tests both cases | Resolved |
| 1 | P3 MINOR — the postinst unit tests (QFU4) run the PAM block on the host; a new state-directory lookup must be redirectable | F3 reads one `PAM_STATE_DIR=/var/lib/pam` variable, which the new tests replace with a scratch directory | Resolved |
| 1 | P4 MINOR — UPG2 must prove the restarted process runs the new file, not only that it answers | `/proc/<MainPID>/exe` must equal `/usr/libexec/soos/soos-daemon` without ` (deleted)` and `MainPID` must change | Resolved |

## 1. Coverage Matrix
| Acceptance line (issue #327) | Spec element | Status |
|---|---|---|
| Upgrade section per method in README / Docs | UPG7, UPG8 | Covered |
| Re-run of `install.sh` documented and tested in CI | UPG1–UPG3, UPG9 (CI job `systemd-unit`) | Covered |
| `master.key`, templates, `daemon.toml`, PAM activation unchanged | UPG1 (install.sh), UPG4 (.deb), UPG6 (RPM, Arch) | Covered |
| Daemon left healthy | UPG2 | Covered |
| Re-enroll only when `soos-enroll list` reports a foreign template | UPG7 | Covered |

## 2. Architecture / ADR Alignment
- PAM activation stays explicit (GitHub #145, #209): F3 only *keeps* an administrator's choice; nothing new is activated.
- No key material in packages (GitHub #144): untouched; the key is compared by digest, never printed.
- Fail-closed installer (GitHub #164): F2 narrows the rollback to what the run changed — consistent with the journal model.
- Readiness bound (GitHub #211): the restart reuses `scripts/wait_daemon_ready.sh` (30 s bound) and exit 70.
- No Rust production code, no IPC or PAM module change: latency budget and PAM invariants unaffected.

## 3. Risks Accepted
- A restart briefly makes PAM fall back to the password (`PAM_IGNORE`), which is the documented fail-safe.
- `.deb` / Arch upgrades still do not restart the daemon (owner decision OD1); documented as a manual step.

VALIDATION_VERDICT: APPROVED
