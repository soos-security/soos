# Walkthrough 84 — Fedora authselect Profile: Activation, faillock Order and Rollback

- **Date**: 2026-09-29
- **Issue**: Review finding ONB-02 (GitHub #145) — **Branch**: `fix/fedora-authselect-profile`
- **Matrix criteria**: FAP1–FAP5 (new), DV2 (updated evidence)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`, finding ONB-02,
severity CRITICAL) established, and this change re-verified in a stock `fedora:40` container,
that the Fedora/RHEL integration could not be activated as documented and was dangerous when
activated:

1. `authselect select custom/soos with-faillock --force` failed with
   `[error] Unknown profile feature [with-faillock]` (rc=1): the profile `README` declared no
   feature, so authselect rejected the documented command.
2. `authselect select custom/soos --force` succeeded but regenerated `/etc/nsswitch.conf` from a
   profile that shipped no `nsswitch.conf` template: the file went from 14 effective lines to 0
   (loss of `passwd`, `group`, `shadow`, `hosts`, ... sources).
3. The `{?with-faillock:...}` lines are not authselect syntax and were copied literally into
   `/etc/pam.d/system-auth` and `password-auth`. Linux-PAM treats `{?with-faillock:auth` as an
   unknown module type and installs a must-fail handler, so every password authentication through
   `system-auth` failed after following `Docs/PACKAGING_AND_PROVISIONING.md` (denial of
   authentication for sudo/login/gdm), while `authselect check` still said the configuration was valid.
4. `tests/distro/fedora_rhel_test.sh` only grepped the template text and ran
   `authselect check || true`, so none of this could be caught.

Objectives: ship a complete, activatable profile; keep `pam_faillock` in the correct order; make
the documented activation command the one authselect accepts; provide and test a rollback path;
validate all of it in a Fedora container and in CI.

## 2. Architect Design

No Rust production code is touched; the change is packaging, scripts, tests and documentation.

- **Profile layout** (`packaging/pam/fedora/soos/`): the full Fedora 40 `local` profile layout —
  `README` (with the `AVAILABLE OPTIONAL FEATURES` list, `with-faillock::` first), `REQUIREMENTS`,
  `system-auth`, `password-auth`, `nsswitch.conf`, `fingerprint-auth`, `smartcard-auth`,
  `postlogin`, `dconf-db`, `dconf-locks`. Copies (not symlinks) so the profile is self-contained on
  RHEL 9, which does not ship a `local` profile.
- **Only two additions** to the `auth` sections of `system-auth` and `password-auth`:
  `auth [success=done default=ignore] pam_soos.so timeout_ms=250` immediately before
  `pam_unix.so` (`sufficient`), and `auth optional pam_soos.so event=password-failed timeout_ms=20`
  after `pam_faillock.so authfail`. `authfail` stays `required` (the former profile used
  `[default=die]`, which would have skipped the password-failed event).
- **Conditional syntax**: `{include if "with-faillock"}` (authselect-profiles(5)); the three
  `pam_faillock` lines (preauth, authfail, account) are the only faillock lines.
- **Rollback state**: `/etc/soos/authselect.previous` holds `authselect current --raw` (profile id
  + features, e.g. `local with-silent-lastlog`). Written by `scripts/install.sh` and RPM `%post`
  only when the live profile is not already `custom/soos`; read by `scripts/uninstall.sh` and RPM
  `%preun`, which re-select it (fallback `local` → `minimal` → `sssd`) **before** removing
  `/etc/authselect/custom/soos`. If no restoration succeeds, the profile directory is kept
  (fail-closed: authselect must never point at a deleted profile).
- **Sentinels**: `DESTDIR` non-empty → no `authselect` invocation (staging install); no
  `authselect` binary → skipped; empty or `custom/soos*` recorded value → fallback list; the
  recorded line is filtered to `[A-Za-z0-9/_. -]` before being word-split into `authselect select`.
- **Never auto-activate**: install paths print the activation command instead of running it.

## 3. Plan Evaluation

Condensed (review-issue workflow, no backlog item). Checked against `AI/ARCHITECTURE.md` §5
(universal ordering `pam_soos` → `pam_unix` → `event=password-failed`), the auditor checklist
item 8 (`pam_faillock` preauth ordering + password fallback) and the existing invariant
`test_pam_config_ordering_matches_spec` (first `pam_unix.so` occurrence must follow the
`pam_soos.so timeout_ms=250` line — the template header comment therefore names no module).
Verdict: approved; the profile is a strict superset of the previous behavior with the `local`
profile's features available under the same names.

## 4. Tester Contract

| Test | Criterion | Red evidence (before the fix) |
|---|---|---|
| `soos-invariants::tests::test_fedora_authselect_profile_is_complete` | FAP1 | `'system-auth' uses the unsupported '{?feature:...}' syntax; use '{include if "feature"}'` |
| `soos-invariants::tests::test_fedora_authselect_profile_preserves_faillock_ordering` | FAP2 | `pam_faillock line must end with {include if "with-faillock"}: '{?with-faillock:auth required pam_faillock.so preauth silent}'` |
| `soos-invariants::tests::test_fedora_authselect_activation_and_rollback_are_scripted` | FAP4, FAP5 | `Docs/DISTRIBUTION_DEPLOYMENT.md must not show the unsupported '{?with-faillock:...}' syntax` |
| `tests/docker/authselect_profile_test.sh` A1–A7 (fedora:40) | FAP1–FAP4, DV2 | A1: `[error] Unknown profile feature [with-faillock]` / `Unable to activate profile [custom/soos] [22]: Invalid argument` → `[FAIL] A1: documented activation command failed` (exit 1) |
| `tests/distro/fedora_rhel_test.sh` Step 4 / Step 7 | DV2 | Live VM script (root); the `authselect check \|\| true` tolerance was removed and real activation, generated-file ordering, `nsswitch.conf` databases and the rollback of the recorded profile are asserted |

Docker checks: A1 activation rc=0; A2 `authselect check` rc=0 and `authselect current --raw` =
`custom/soos with-faillock`; A3 generated `system-auth`/`password-auth` contain no `{` and satisfy
`preauth < soos < unix < authfail < event < deny` with exactly three `pam_faillock` lines and the
account-phase line; A4 `/etc/nsswitch.conf` effective lines identical to the `local with-faillock`
baseline (14 lines); A5 `pamtester` correct password accepted / wrong password rejected on both
stacks with `pam_soos.so` absent; A6 activation without features valid and free of `pam_faillock`;
A7 `scripts/uninstall.sh --keep-data --skip-systemd` restores the recorded `local with-faillock`,
removes the profile directory and the record, `authselect check` rc=0, password login works.

### Migrated existing tests
| Test | Old assertion | New assertion | Mandating line |
|---|---|---|---|
| `tests/distro/fedora_rhel_test.sh` Step 4 | Template grep for `pam_faillock.so preauth/authfail`; `authselect check \|\| true` | Real `authselect select custom/soos with-faillock --force` (rc=0), `authselect check` (rc=0), ordering by line numbers in the generated files, no unresolved `{` template syntax, account-phase faillock, `nsswitch.conf` keeps `passwd/shadow/group/hosts` (and is unchanged when the original profile was `local`) | GitHub #145 "Validate ... `authselect select custom/soos with-faillock`, then `authselect check`, inspect generated system-auth and password-auth" |
| `tests/distro/fedora_rhel_test.sh` Step 7 | Binary removal only | Additionally `authselect current --raw` no longer `custom/soos*`, equals the recorded original, `authselect check` rc=0, profile directory removed | GitHub #145 "rollback path" |

Fixture note: the first green Docker run failed at A7 because `pamtester` runs only the `auth`
phase, so the deliberate wrong-password attempts of A5/A6 reached `pam_faillock`'s `deny = 3` and
locked the test user once `with-faillock` came back. The fixture now resets the tally
(`faillock --user <u> --reset`) before every password assertion; no assertion was weakened.

### Flakiness check
The Docker script is deterministic (no timing); it was run three times (red, green with the
faillock lock-out, green) with identical results per state.

## 5. Auditor Constraints

| # | Constraint | Met by |
|---|---|---|
| 1 | Zero `{?` syntax in packaging, docs, scripts and tests | `test_fedora_authselect_profile_is_complete`, `test_fedora_authselect_activation_and_rollback_are_scripted`; Docker A3 (`grep '{'` on generated files) |
| 2 | `pam_faillock` preauth before `pam_soos`, authfail after `pam_unix`, account line present, all gated by `{include if "with-faillock"}` | `test_fedora_authselect_profile_preserves_faillock_ordering`; Docker A3/A6 |
| 3 | Password fallback reachable after `PAM_IGNORE` (`pam_unix` `sufficient`, no `[default=die]`, `pam_deny` last) | Same invariant test; Docker A5 with `pam_soos.so` absent |
| 4 | Uninstall never deletes the selected profile without restoring another one first; a failed restoration keeps the directory | `scripts/uninstall.sh` (`AUTHSELECT_RESTORED` guard), RPM `%preun` fallback chain; ordering asserted by the invariant test; Docker A7 |
| 5 | Scripts: `set -euo pipefail`, `bash -n`, ShellCheck `--severity=error` clean; no secret in the recorded file (profile id + feature names, filtered) | `bash -n` via `scripts/candid_review.sh`; ShellCheck run through `koalaman/shellcheck:stable` (exit 0) |
| 6 | New CI job: SHA-pinned `actions/checkout`, `permissions: contents: read` (workflow-level), `persist-credentials: false`, 15-minute timeout, no event data interpolated | `.github/workflows/ci.yml` `authselect-profile` job |
| 7 | No automatic activation of the profile by install paths (an operator decision) | `scripts/install.sh` prints the command; RPM `%post` records only |

Pre-existing violations found: `tests/docker/Dockerfile.fedora` writes a hand-made
`/etc/pam.d/system-auth` (outside authselect) for the T1–T9 matrix — acceptable for that sandbox,
not changed here.

Clearance: CLEARED.

## 6. Implementation

- `packaging/pam/fedora/soos/README`, `REQUIREMENTS`, `system-auth`, `password-auth` rewritten;
  `nsswitch.conf`, `fingerprint-auth`, `smartcard-auth`, `postlogin`, `dconf-db`, `dconf-locks` added
  (Fedora 40 `local` profile content).
- `scripts/install.sh`: records `authselect current --raw` in `/etc/soos/authselect.previous`
  (live installs only) and prints the activation command.
- `scripts/uninstall.sh`: restores the recorded/fallback profile before removing the custom
  profile; keeps it when restoration is impossible.
- `packaging/rpm/soos.spec`: `%install` creates `/etc/soos`; `%post` records the profile; `%preun`
  (erase only) restores it; `%files` owns `%dir /etc/soos` and `%ghost /etc/soos/authselect.previous`.
  Parsed with `rpmspec -P` in `fedora:40`; expanded scriptlets pass `bash -n`.
- `tests/docker/authselect_profile_test.sh` (new, A1–A7; host mode runs `fedora:40` with the
  workspace mounted read-only and re-executes itself with `--in-container`).
- `run_tests.sh authselect` mode; CI job `authselect-profile` (after `lint`) added to the
  `CI Success` aggregate.
- `tests/distro/fedora_rhel_test.sh`: Step 4 activates and inspects; Step 7 verifies the rollback.
- `tests/invariants/src/lib.rs`: three invariant tests + helpers (`AUTHSELECT_PROFILE_FILES`,
  `offset_of`, `referenced_features`).
- Docs: `Docs/DISTRIBUTION_DEPLOYMENT.md` §4.1–4.4 and §6, `Docs/PACKAGING_AND_PROVISIONING.md`
  (Fedora section, rollback capability), `Docs/CI_CD_AND_SECURITY.md` (job table, diagram,
  `run_tests.sh` modes), `Docs/PAM_DOCKER_TEST_MATRIX.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`,
  `Docs/DEVELOPMENT_WORKFLOW.md` (job list).

Notable decision: the password-failed event line is placed after `pam_faillock.so authfail`
(documentation order "failure accounting, then intrusion notification"); with `authfail` being
`required` the event is always reached on a wrong password.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`): PASSED (unsafe, PAM panic safety, async runtime,
forbidden dependencies, shell syntax, PAM output isolation, English policy). Layer 2
(fingerprint-bound report): pending — to be produced by the candid reviewer before push.

## 8. Verification Results

```bash
cargo fmt --all                                                         # clean
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings   # exit 0
cargo test --locked --workspace --all-targets --all-features            # all green, see below
./scripts/candid_review.sh                                              # PASSED
./tests/docker/authselect_profile_test.sh                               # A1–A7 OK in fedora:40
docker run --rm -v "$PWD:/mnt:ro" koalaman/shellcheck:stable --severity=error \
  scripts/install.sh scripts/uninstall.sh tests/distro/fedora_rhel_test.sh \
  tests/docker/authselect_profile_test.sh run_tests.sh                  # exit 0
rpmspec -P packaging/rpm/soos.spec                                      # rc=0 (fedora:40)
```

`cargo test -p soos-invariants --all-features`: 24 passed (the three new tests included). Full
workspace results are recorded in the structured report of this change.

## 9. Known Limitations / Follow-ups

- RHEL 9 (authselect 1.2.x) was not exercised: no RHEL container image is available offline. The
  profile avoids any dependency on the host's `local` profile, and the `{if ... and/not ...}`
  expressions used in `nsswitch.conf` are the ones the Fedora 40 `local` profile ships.
- `tests/distro/fedora_rhel_test.sh` remains a root-only live VM script; only the Docker
  validation runs in CI.
- Operators migrating from `sssd`/`winbind` profiles must note that `custom/soos` is derived from
  `local` (files-only identity sources); a `sssd`-based variant is a possible follow-up.
