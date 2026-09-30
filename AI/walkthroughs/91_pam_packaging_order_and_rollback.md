# Walkthrough 91 — Debian Password-Failed Hook Order and Byte-for-Byte PAM Rollback

- **Date**: 2026-09-30
- **Issues**: Review findings ONB-03 (GitHub #161) and ONB-08 (GitHub #166) — **Branch**: `fix/pam-packaging-rollback`
- **Matrix criteria**: DNP1–DNP3, PRB1–PRB4 (new)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, area Installation &
Onboarding, §3.3 plan item `fix/pam-profiles`) confirmed two defects in the PAM packaging:

1. **ONB-03**: `packaging/pam/debian/soos-notify` was an `Auth-Type: Additional` profile.
   `pam-auth-update` emits Additional profiles after `auth requisite pam_deny.so`, which ends the
   stack on a wrong password. The `event=password-failed` hook (ARCHITECTURE §5, "reached only if
   pam_unix fails") therefore only ran after a successful login and never on a wrong password on
   Debian/Ubuntu. Nothing tested the generated position.
2. **ONB-08**: nothing created the `*.soos-backup` files that `scripts/uninstall.sh` restores and
   that the docs present as the rescue path; `soos-admin gdm enable` edited `/etc/pam.d/gdm-password`
   with a non-atomic `fs::write` and no backup; uninstall never removed the GDM line. (The Fedora
   part, deleting the selected authselect profile, was already fixed by walkthrough 84 / GitHub #145.)

Objectives: place the Debian hook between `pam_unix` and `pam_deny` and prove it with the real
module; record the pre-install PAM state; make every PAM edit restorable and atomic; make uninstall
return the PAM configuration to its exact pre-install bytes, and fail safe when it cannot.

## 2. Architect Design

- **Debian notify profile**: `Auth-Type: Primary`, `Priority: 12`, control `[default=ignore]` on
  `Auth` and `Auth-Initial`. Primary profiles are sorted by descending priority and all precede
  `pam_deny`; `pam-auth-update` rewrites each `success=end` of the modules above so that a success
  jumps past the hook and `pam_deny` (`pam_unix` becomes `success=2`). `[default=ignore]` means the
  hook can neither grant nor deny. Priority 12 instead of the review's suggested 250: 12 sorts after
  every standard primary method (`unix` 256, `winbind` 192, `sss` 128, ...), so an LDAP/SSSD user
  whose password `pam_unix` does not know is not reported as a password failure when a later
  primary module accepts it. It stays inside the value set the existing contract
  `test_pam_config_ordering_matches_spec` accepts (`Priority: 128` or `Priority: 12…`); that test
  is unchanged.
- **Snapshot helper** `scripts/pam_snapshot.sh {snapshot|verify|discard} [--destdir --sysconfdir
  --localstatedir]`: `/var/lib/soos/state/pam-backup/` (`0700`, `umask 077`, built in a `mktemp -d`
  and renamed) with dereferenced copies of `/etc/pam.d/*` and `/etc/nsswitch.conf`, `SHA256SUMS`, and
  `authselect.current` on live hosts. An existing snapshot is never overwritten; symlinked state
  paths are refused. `verify` compares in pure bash (associative arrays) and returns 0 identical,
  1 differences (listed as `changed|removed|added <rel>`), 2 no snapshot; hashing errors count as a
  difference (fail closed).
- **install.sh** (minimal, separate block `6a`): calls `pam_snapshot.sh snapshot` on live installs
  before any PAM template is installed.
- **gdm.rs**: `GDM_PAM_LINE` / `PAM_BACKUP_SUFFIX` constants, `pam_backup_path()`,
  `ensure_gdm_pam_line()` (refuses symlinks/non-regular files, byte-exact backup created once,
  mode `& 0o7755`) and `write_atomic()` (exclusive `create_new` temp file `0600`, `fchown` to the
  original owner, final mode, `sync_all`, `rename`, directory `fsync`, temp removed on failure).
  The GDM control keyword stays `sufficient` (code constant, project-facts §2; the review noted the
  drift from ARCHITECTURE §5 `[success=done default=ignore]` — both ignore a `PAM_IGNORE` or
  failure return and end the stack on success; changing it is out of scope and would need an ADR).
- **uninstall.sh** rollback order: `pam-auth-update --package --remove` → authselect restore (84) →
  atomic restore of every regular `*.soos-backup` → residual scan of `/etc/pam.d` for active
  `pam_soos.so` lines: strip only if the stripped file equals its snapshot copy **or** the file has
  no numeric `success=N` jump; otherwise leave it, set `PAM_ROLLBACK_INCOMPLETE`, **keep**
  `pam_soos.so` and exit 1 → `pam_snapshot.sh verify` → discard the snapshot only when identical.
  All writes go through `atomic_replace` (same-directory `mktemp`, reference mode/owner, `chmod go-w`,
  `sync`, `mv -f`). Content comparison uses `sha256sum` (no `diffutils` dependency).
- **Arch scriptlet**: post-install message asks for a `system-auth.soos-backup` before the manual
  edit; `pre_remove` warns if `system-auth` still loads `pam_soos.so`.

## 3. Plan Evaluation

Condensed (review-issue workflow, no backlog item). Checked against ARCHITECTURE §5 (universal
ordering), the "never convert an error into PAM_SUCCESS" invariant (the hook is `[default=ignore]`;
no rollback path adds or promotes any control), and the lock-out risk of uninstall: removing a
module still referenced by a `required` line would fail that stack, keeping it returns
`PAM_IGNORE`, hence "keep the module when the rollback is incomplete". Verdict: approved.

## 4. Tester Contract

| Test | Criterion | Red evidence (before the fix) |
|---|---|---|
| `soos-invariants::tests::test_debian_notify_profile_sits_between_pam_unix_and_pam_deny` | DNP1 | `soos-notify must be a Primary profile: Additional profiles are emitted after 'auth requisite pam_deny.so' ...` |
| `tests/docker/test_suite.sh` T11 (real `pam_soos.so`, `mock_daemon.py --record`) | DNP3 | `T11 failed: password-failed hook (line 26) is not before pam_deny (line 20).` (generated stack: unix `success=1`, deny, permit, then `optional pam_soos.so event=password-failed`) |
| `tests/docker/pam_rollback_test.sh` D1–D5 (ubuntu:24.04), F1–F2 (fedora:40) | DNP2, PRB1, PRB4 | `bash: /workspace/scripts/pam_snapshot.sh: No such file or directory` → `[FAIL] D1: snapshot failed` |
| same, F1 `assert_verify_detects_drift` (added after the first green run) | PRB1 | first implementation used `diff`, absent from `fedora:40`: `diff: command not found` yet "identical" → `[FAIL] F1: pam_snapshot.sh verify did not report 'changed pam.d/system-auth'` |
| `soos-invariants::tests::test_pam_snapshot_helper_records_and_verifies_pre_install_state` | PRB1 | `scripts/pam_snapshot.sh must exist` |
| `soos-invariants::tests::test_uninstall_restores_pre_install_pam_state_byte_for_byte` | PRB3 | `assertion failed: snapshot` (helper missing) |
| `soos-invariants::tests::test_uninstall_keeps_module_when_pam_rollback_is_unsafe` | PRB3 | `assertion failed: snapshot` (helper missing) |
| `soos-invariants::tests::test_pam_rollback_is_wired_and_documented` | PRB1–PRB4 | `install.sh: 'pam_snapshot.sh' not found` |
| `soos-admin-cli` `test_gdm_enable_creates_byte_exact_backup_with_original_mode` | PRB2 | `backup must exist: Os { code: 2, kind: NotFound, ... }` |
| `soos-admin-cli` `test_gdm_enable_is_atomic_and_idempotent` | PRB2 | `only the PAM file and its backup may exist (no temporary file left)` (no backup) |
| `soos-admin-cli` `test_gdm_enable_refuses_a_symlinked_pam_file` | PRB2 | `a symlinked PAM file must be refused` |
| `soos-admin-cli` `test_gdm_enable_never_overwrites_an_existing_backup` | PRB2 | passed before the fix (no backup logic at all); guards the new code against overwriting |

No existing test was modified. `mock_daemon.py` gained `--record` (events are recorded and never
answered; requests unchanged), used only by T11. Test fixtures `D4` replace the hook by a
`pam_exec` probe at the same position and control to prove reachability without the module.

### Flakiness check
T11 waits 200 ms after each PAM call before counting recorded events (the event is fire-and-forget
within the module's 20 ms budget). The event classifier requires an exact postcard `Event` decode to
the frame length, so a `Request` cannot be counted. Docker tests were run red, green, red (drift
check), green.

## 5. Auditor Constraints

| # | Constraint | Met by |
|---|---|---|
| 1 | The hook can never grant or deny (`[default=ignore]`, never `success=`/`optional`/`sufficient`) | DNP1 invariant; T11 (wrong password still rejected) |
| 2 | No rollback path converts an error into success or removes a password path | only `pam_soos.so` lines are removed, only when provably safe; module kept otherwise |
| 3 | PAM files are written atomically; nothing group/world-writable | `write_atomic` (Rust), `atomic_replace` (bash, `chmod go-w`); invariant + gdm tests check modes |
| 4 | Snapshot is root-only, contains no secret, never overwritten, no symlink following | `umask 077`, `0700`, `refuse_symlinks`, PAM/NSS config only; PRB1 test |
| 5 | Verification fails closed | bash-only comparison, hashing errors → difference; Docker F1 drift assertion |
| 6 | No `unwrap`/`expect` in production Rust; `#![forbid(unsafe_code)]` in admin-cli kept | `gdm.rs` uses `?`/`map_err`; clippy `-D warnings` |
| 7 | CI job hardening | `pam-rollback`: SHA-pinned checkout, `persist-credentials: false`, 15 min, workflow `contents: read` |

Pre-existing, not changed: `tests/docker/test_suite.sh` makes `/run/soos` `0777` and the mock socket
`0666` (sandbox only); `scripts/install.sh` installs the Arch snippet as `/etc/pam.d/soos.snippet`
on every distribution. Clearance: CLEARED.

## 6. Implementation

- `packaging/pam/debian/soos-notify`: Primary, Priority 12, `[default=ignore]`.
- `scripts/pam_snapshot.sh` (new), `scripts/install.sh` block 6a, `scripts/uninstall.sh` (backup
  restore made atomic, residual scan, snapshot verification, module kept + exit 1 when incomplete).
- `crates/admin-cli/src/gdm.rs`: backup + atomic replacement, symlink refusal.
- `packaging/arch/soos.install`: backup advice and removal warning.
- Tests: see §4; `run_tests.sh rollback`; CI job `pam-rollback` in the `CI Success` aggregate.
- Docs: `Docs/DISTRIBUTION_DEPLOYMENT.md` §3.2 (generated stack), §3.5, §5.2/§5.4 (Arch backup),
  §6, §7 and new §7.1 (rollback guarantees); `Docs/PACKAGING_AND_PROVISIONING.md` §4/§5;
  `Docs/CI_CD_AND_SECURITY.md`, `Docs/PAM_DOCKER_TEST_MATRIX.md` (T11),
  `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `Docs/DEVELOPMENT_WORKFLOW.md` (job lists).

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`): PASSED. Layer 2 (fingerprint-bound report): not produced on
this branch (to be run by the integrating agent before push).

## 8. Verification Results

```bash
cargo fmt --all -- --check                                                        # clean
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings     # exit 0
cargo test --locked --workspace --all-targets --all-features --no-fail-fast       # 570 passed, 0 failed
./scripts/candid_review.sh                                                        # PASSED
./tests/docker/pam_rollback_test.sh        # D1–D5 OK (ubuntu:24.04), F1–F2 OK (fedora:40)
./tests/docker/authselect_profile_test.sh  # A1–A7 OK (regression, uninstall changed)
docker build -t <tag> . && docker run ... tests/docker/test_suite.sh               # T1–T11 OK
koalaman/shellcheck:stable scripts/pam_snapshot.sh scripts/uninstall.sh tests/docker/pam_rollback_test.sh
                                           # only pre-existing SC2034 (BOLD unused) in uninstall.sh
```

Green D3 output: `soos(1) < unix(2, success=2 -> permit) < hook(3) < deny(4) < permit(5)`; D5/F2:
PAM state byte-identical to the pre-install baseline (21 / 13 entries). The Debian
`pam-auth-update --remove` regenerates `common-*` byte-identically, so no copy is needed there.

## 9. Known Limitations / Follow-ups

- Native packages (`.deb`, `.rpm`, Arch) do not ship `pam_snapshot.sh`: they rely on the native
  rollback (`prerm` → `pam-auth-update --remove`, RPM `%preun` → recorded authselect profile) and
  pacman never edits `/etc/pam.d`. Shipping the helper as `/usr/libexec/soos/pam-snapshot` is a
  follow-up for the §3.3 one-command bootstrap.
- Arch `system-auth` uses numeric jumps (`pam_systemd_home.so success=2`); a manual integration that
  adjusted them cannot be reverted automatically: uninstall keeps the module, exits 1 and points to
  the backup / snapshot. The Arch path was not exercised in Docker (manual integration only).
- Real-distro checks still recommended: Debian 12 with `libpam-sss`/`libpam-ldapd` (confirm the
  hook is emitted after them), a real GDM `gdm-password`, RHEL 9 authselect 1.2.
- The GDM line control (`sufficient`) still differs from ARCHITECTURE §5 (`[success=done
  default=ignore]`); left unchanged (code is the source of truth, ADR needed to change it).

### Follow-ups from the batch candid review (`fix/p1-install-batch`)

- **Finding 6 — `pam_snapshot.sh snapshot` does not detect an already-activated state.** A
  reinstall over a host activated by an earlier version (or with a leftover `soos.snippet`)
  records that state as the "pristine" baseline. Safe (uninstall only restores on byte
  equality), but every later `verify` reports drift and the snapshot is never discarded. Warn or
  refuse to record when an entry already references `pam_soos.so`.
- **Finding 8 — GDM line and backup freshness.** `crates/admin-cli/src/gdm.rs` still uses
  `auth sufficient` instead of ARCHITECTURE §5 `[success=done default=ignore]` (needs an ADR or an
  alignment change), and an existing `gdm-password.soos-backup` is always kept, even when it
  predates a later distribution update of `gdm-password`; consider refreshing a backup whose file
  no longer contains `pam_soos.so`.

## 10. Batch Integration and Review Round

**P1 packaging fix (`2fbe969`).** Packages staged through `install.sh --destdir` used to receive
`pam_soos.so` in `/usr/lib64/security` (guessed from the build host). `scripts/build_deb.sh` and
`packaging/debian/rules` now pass `--pam-dir /usr/lib/<DEB_HOST_MULTIARCH>/security`,
`scripts/build_arch.sh` passes `--pam-dir /usr/lib/security`, and the RPM spec keeps
`%{_libdir}/security` (`test_packaging_passes_explicit_distro_pam_dir`). After the fix,
`./tests/distro/run_distro_validation.sh ubuntu` and `... arch` both passed end to end in Docker
(exit 0; module at `/usr/lib/x86_64-linux-gnu/security` and `/usr/lib/security`), see
walkthrough 92 §8.2.

**Finding 2 — `--purge-data` deleted the operator's only pre-install copy.** It removed
`state/pam-backup` after an incomplete or unverified rollback, although the error message points
to it. `scripts/uninstall.sh` now keeps `state/pam-backup` whenever it survived step 2 (it is
discarded only after a verified, complete rollback), purges everything else, and prints
"Keeping .../state/pam-backup ...". Tests: `test_uninstall_purge_keeps_snapshot_when_pam_rollback_is_incomplete`
(red: "the snapshot must survive --purge-data: NotFound") and the regression guard
`test_uninstall_purge_removes_state_after_verified_rollback` (green before and after).

**Finding 3 — snapshot failing part-way.** `scripts/pam_snapshot.sh snapshot` now removes its
`state/.pam-backup.XXXXXX` temp directory from an `EXIT`/`INT`/`TERM` trap, and `discard` also
drops stale temp directories. `scripts/install.sh` block 6a marks the snapshot as created
*before* invoking the helper when none pre-existed, so the rollback discards it even when the
helper fails midway (a pre-existing snapshot is still never journaled or discarded). Tests:

- `test_pam_snapshot_failure_leaves_no_temporary_directory` (non-root, `cp` shim failing copies
  into the temp directory; red: `leftovers [".pam-backup.6PTt6T"]`).
- `test_install_journals_pam_snapshot_before_invoking_helper` (static order in block 6a, and the
  Docker test drives install.sh; red before the change).
- `tests/docker/pam_rollback_test.sh` D0a/D0b drive a failing **live** `scripts/install.sh`
  (stub artifacts) as root in `ubuntu:24.04`: D0a fails after 6a on a model digest mismatch, D0b
  fails inside the helper (`cp` shim). Both must leave the PAM state byte-identical, no
  `/var/lib/soos`, no binaries, no module, no `soos` group. Block 6a cannot run in the
  `cargo test` invariants: it is live-only and a live install requires real root (the preflight
  refuses non-root, and that check is itself a contract, INS4). Red evidence: the new
  `pam_rollback_test.sh` against the pre-fix scripts (`2fbe969`) fails with
  `D0b: /var/lib/soos left behind: ... /var/lib/soos/state/.pam-backup.VxFOlT/pam.d`; green on
  the fixed branch.
