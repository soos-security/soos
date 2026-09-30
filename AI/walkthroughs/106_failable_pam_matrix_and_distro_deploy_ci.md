# Walkthrough 106 — A Failable Docker PAM Matrix and Fedora / Arch Deployment CI

- **Date**: 2026-09-30
- **Issues**: Review finding TCI-06 (GitHub #189), ONB-10b (GitHub #274), remainder of ONB-10
  (GitHub #168, mostly fixed by walkthrough 92)
- **Branch**: `test/pam-matrix-and-distro-ci`
- **Matrix criteria**: PMX1–PMX9 (new); PK6, PK7, DV2, DV3 updated

---

## 1. Context & Objectives

The review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`) found that the Docker PAM matrix
(`tests/docker/test_suite.sh`) could not fail where it mattered:

1. **T2** computed the elapsed time of the timeout case and only printed it; the mock delay was
   a hard-coded 0.5 s, so the deadline itself was never asserted (and a stack with
   `timeout_ms=1000` would have turned T3 red for the wrong reason).
2. **T6** (distribution stack) and **T8** (absent module) ended in `warn` and never exited
   non-zero.
3. `mock_daemon.py` supported `deny` and `crash-truncated`, but no case used them, and nothing
   exercised a well-framed but malformed answer (`Verdict::Deny → PAM_IGNORE → password` had no
   in-stack test).
4. The Fedora and Arch deployment paths (`run_distro_validation.sh fedora|arch`) and the RPM /
   Arch branches of `tests/docker/test_packages.sh` never ran in CI; `test_packages.sh` silently
   exited 0 without `rpmbuild` and fell back to a dry run on an unknown distribution.
5. The deployment scripts built without `--locked`, and `debian_ubuntu_test.sh` ignored a
   failing `pam-auth-update` (`|| true`).

The `chmod 777 /run/soos` part of #189/#168 was already fixed by walkthrough 92 (DVM7).

## 2. Specification (Phases 1–1.5, condensed: test and CI change only)

- **`tests/docker/pam_case_lib.sh`** (new, sourced by the suite): `pam_stack_timeout_ms <file>`
  reads `timeout_ms=` from the first `pam_soos.so` auth line that is not the password-failed
  hook, applying the module's own default (1000 ms) and clamp (10..5000 ms) from
  `crates/pam/src/config.rs`; `deadline_bound_ms t = t + 1000` (tolerance for process start,
  `pam_unix` hashing, shared runners); `timeout_mock_delay_ms t = t + 2500` (always above the
  bound); `assert_elapsed_within_deadline`, `assert_no_facial_authorization` (a password-less
  run must fail) and `assert_password_fallback` (valid accepted, wrong rejected).
  Nothing is hard-coded to 250 ms, so the parallel packaging timeout work (#185/#186) cannot
  conflict: both values move with the stack under test.
- **T2** asserts the bound on the password path; **T2b** (new) runs a copy of `test-soos` with
  `pam_unix.so nodelay` without a password: the late `Allow` must not authenticate and the run
  must also stay within the bound (the `nodelay` copy removes the ~2 s `pam_unix` failure
  delay, which would otherwise dominate).
- **T6**: a missing `common-auth`/`system-auth` with `pam_soos.so` is a failure; facial `Allow`
  with 0 prompts, password fallback and wrong-password rejection are hard failures.
- **T8**: with the module removed and a mock answering `Allow`: valid password accepted, wrong
  password rejected, no password-less authentication; the module is restored before exiting.
- **T13** `deny`, **T14** `crash-truncated`, **T15** `malformed` (undecodable verdict
  discriminant 0x7F), `wrong-request-id` (Allow for another request), `bad-version` (Allow with
  version 2), `oversized` (prefix 4097), `empty` (prefix 0).
- **CI**: new job `distro-deploy` (matrix `fedora`, `arch`, `fail-fast: false`, push to `main`
  and manual dispatch, 90 min) running `run_distro_validation.sh "$DISTRO"` then
  `test_packages.sh` in the same `soos-distro-val-$DISTRO` image and
  `soos-distro-target-$DISTRO` volume. It is not a `CI Success` dependency because it is
  skipped on pull requests (a skipped need would fail the aggregate), like `distro-pam-matrix`.
- **Scripts**: `--locked` on every `cargo build` of the suite, the three distro scripts and the
  package harness; `pam-auth-update --package --force --enable soos soos-notify` must succeed
  and the generated `common-auth` must contain the password-failed hook (stack order and
  rollback remain pam-rollback D2/D3); `test_packages.sh` fails closed (missing `rpmbuild`,
  unknown distribution) and selects `soos-[0-9]*.rpm`, never `soos-debuginfo`. The Fedora
  script no longer pins the packaged `timeout_ms=250` when locating the facial line.

## 3. Tester Contract (Red Evidence)

Eleven invariant tests in `tests/invariants/src/distro_matrix.rs` (commit `1bded41`), all red
before the implementation:

| Test | Red message (before) |
|---|---|
| `test_pam_case_lib_reads_timeout_from_the_stack_under_test` | library missing (bash `source` failed) |
| `test_pam_case_lib_deadline_assertion_can_fail` | library missing |
| `test_pam_matrix_t2_asserts_the_deadline` | `test_suite.sh must source tests/docker/pam_case_lib.sh` |
| `test_pam_matrix_t6_and_t8_can_fail` | `T6 must not downgrade a failed expectation to a warning` |
| `test_pam_matrix_exercises_every_mock_daemon_mode` | `mock_daemon.py must support --mode malformed` |
| `test_mock_daemon_malformed_modes_put_one_defect_on_the_wire` | `mock_daemon.py --mode wrong-request-id never created its socket` |
| `test_pam_matrix_rejection_cases_assert_no_authorization` | `test_suite.sh must contain a '# T13:' case header` |
| `test_deployment_and_package_tests_build_with_locked` | `tests/docker/test_packages.sh must build with --locked` |
| `test_debian_deployment_never_ignores_pam_auth_update` | `pam-auth-update failure must fail the deployment test: ... \|\| true` |
| `test_package_harness_fails_closed` | `test_packages.sh may only exit 0 after every check passed` (2 found) |
| `test_ci_runs_fedora_and_arch_deployment` | `ci.yml must define the distro-deploy job` |

The matcher of `test_deployment_and_package_tests_build_with_locked` was narrowed during
Phase 4 from "line contains `cargo build`" to "line starts with `cargo build`": it matched the
`--help` text "Skip cargo build if binaries are already present" (a false positive of the new
test itself, not a relaxed contract).

### 3.1 Docker red evidence (strengthened cases fail against broken behavior)

Throwaway patches (`red_patch.py`, scratch clone only, never committed) applied to the root
`Dockerfile` image under a private tag:

| Variant | Broken behavior | Previous suite (`83ad422`) | New suite |
|---|---|---|---|
| R2 `stack` | `common-auth` facial line `[success=ok default=ignore]` (not terminal) | passes, `[WARN] T6: ... non-interactive prompt differed` | `[FAIL] T6 failed: Distro stack (common-auth) did not honor the daemon Allow without a password prompt.` |
| R3 `unknown` | `test-soos` line `[success=done module_unknown=die default=ignore]` (absent module fatal) | passes, `[WARN] T8: Stack failed without module.` | `[FAIL] T8 failed: valid password rejected while pam_soos.so is absent.` |
| R4 `requestid` | module skips the response `request_id` check | passes (no mode exercised it) | `[FAIL] T15 (wrong-request-id) failed: a malformed response authenticated the user!` |
| R1 `timeout` | module ignores `timeout_ms`, always waits 5000 ms | fails only indirectly at T3 (wrong password accepted) | `[FAIL] T2: PAM run took 2757 ms, above timeout_ms=250 + 1000 ms tolerance (1250 ms)` |

The previous suite ran R2 + R3 + R4 at once and printed `ALL IN-CONTAINER PAM MATRIX TESTS
PASSED SUCCESSFULLY!` (exit 0).

## 4. Audit (Phase 3)

1. No test knob or bypass: the library only measures and asserts; the mock gains wire-format
   modes, no environment toggle.
2. Mock modes send fixed synthetic bytes (no frame, embedding or credential data); `--record`
   still logs classifications only.
3. The mock socket invariant (0660 group `soos`, refusal of any "other" bit) is unchanged and
   still pinned by `test_mock_daemon_socket_is_group_restricted`.
4. T8 restores `pam_soos.so` on every exit path; later cases run with the real module.
5. CI: SHA-pinned checkout, `persist-credentials: false`, the distribution reaches the steps
   through `env: DISTRO` only, no `${{ }}` inside any `run:` of the job (asserted), read-only
   token inherited from the workflow.
6. `pam-auth-update --force` only runs inside the disposable container; production packaging
   (`packaging/debian/postinst`) is untouched.

## 5. Docker Results (scratch clone of `8dbec7d`, not the worktree)

| Command | Result |
|---|---|
| `./run_tests.sh` (root `Dockerfile`, ubuntu:24.04) | T1–T15 pass; T2 355 ms, T2b 260 ms (bound 1250 ms for `timeout_ms=250`) |
| `./tests/docker/run_matrix.sh fedora` | T1–T15 pass; T2 367 ms, T2b 272 ms; T6 on `system-auth` |
| `./tests/docker/run_matrix.sh arch` | T1–T15 pass; T2 319 ms, T2b 267 ms; T6 on `system-auth` |
| `./tests/distro/run_distro_validation.sh fedora` + `test_packages.sh` (RPM branch) | Deployment passed (`--locked` release build, `rpm -i`, authselect `custom/soos with-faillock`, enrollment, `sudo`/`gdm` facial auth and fallback, `rpm -e`, profile restored). **Package harness FAILED** on its first ever RPM execution: `master.key must survive package removal` — see §5.1 |
| `./tests/distro/run_distro_validation.sh arch` + `test_packages.sh` (Arch branch) | Both pass (`pacman -U`, swaylock/hyprlock, `pacman -R`; no `*.key` in the package, key kept on removal, distinct key after a fresh install) |
| `./tests/distro/run_distro_validation.sh ubuntu` + `test_packages.sh` (`.deb` branch) | Both pass; `pam-auth-update --force` enabled both profiles and the hook assertion held |

### 5.1 Finding: `rpm -e` deletes the host master key (packaging, open)

`packaging/rpm/soos.spec` lists `%ghost %attr(0600, root, root) %{_sharedstatedir}/soos/master.key`.
RPM removes `%ghost` files on erase, so `rpm -e soos` deletes `/var/lib/soos/master.key` and every
enrolled `<uid>.cbor.enc` becomes undecryptable after a reinstall — the opposite of the ONB-01
contract ("the key is host state, it survives removal") that the `.deb` and Arch packages honor.
The assertion is correct and was not relaxed. The fix belongs to `packaging/rpm/soos.spec`
(outside this change's scope); until it lands, the fedora leg of `distro-deploy` fails on `main`
by design, and PK6 stays Pending.

### 5.2 Observation: the Debian postinst does not integrate PAM on a modified stack

During `dpkg -i`, `packaging/debian/postinst` runs `pam-auth-update --package --enable soos
soos-notify || true`, which printed `Local modifications to /etc/pam.d/common-*, not updating.`:
on a host with a locally edited `common-auth` the package silently leaves PAM unconfigured. The
deployment test now enables the profiles itself with `--force` and fails on error, so the
test no longer depends on that path; the postinst behavior is recorded here for a packaging
follow-up.

## 6. Documentation Updated

- `Docs/PAM_DOCKER_TEST_MATRIX.md`: T2, T2b, T3, T6, T8 rows rewritten, T13–T15 added,
  "Failable Assertions" section, `pam_case_lib.sh` in the harness tree, `distro-deploy`.
- `Docs/CI_CD_AND_SECURITY.md`: pipeline diagram, `pam-integration` (T1–T15),
  `distro-pam-matrix` (T1–T15), new `distro-deploy` row, `run_tests.sh` scope.
- `AI/VERIFICATION_MATRIX.md`: new component `pam-matrix-failable-assertions` (PMX1–PMX9);
  PK6, PK7, DV2, DV3 updated with the new CI job and the local Docker runs.

## 7. Verification (gate)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test --locked --workspace --all-targets --all-features --no-fail-fast` | all green (149 test binaries). A first run executed concurrently with the Docker builds failed only in load-sensitive timing tests of `camera-v4l`, `daemon`, `gui`, `vision` and `inference-ort` (none touched here, tracked by #280); the rerun on an idle host passed |
| `./scripts/candid_review.sh` | PASSED |
| `bash -n` on every touched script, `shellcheck --severity=error` (koalaman/shellcheck:stable) | clean |
| Docker | see §5 |

## 8. Left Open

- **RPM master-key removal (§5.1)**: `packaging/rpm/soos.spec` must stop listing
  `master.key` as `%ghost` (or otherwise keep it on erase); PK6 stays Pending and the fedora
  leg of `distro-deploy` fails until then.
- **Debian postinst (§5.2)**: `pam-auth-update ... || true` without `--force` silently skips
  PAM integration on a locally modified stack.
- No CI run URL exists yet for `distro-deploy` or `package-deploy` (nothing was pushed from
  this branch); the #274 checklist item "add the first green run URL to DV1, PK5, DVM9" and the
  CI-link half of PK6/PK7/DV2/DV3 need the first run on `main`.
- `packaging/debian/postinst` still runs `pam-auth-update ... || true` (packaging, not a test;
  out of scope here). The deployment test now enables the profiles itself and fails on error.
- The branch is not registered in `BRANCH_TO_ISSUE` of `scripts/sync_issue.py` (owned by the
  parallel #188 work).
- Rows PA2 and PA10 still describe the older evidence; they were left untouched because the
  parallel #187 work edits existing rows in bulk. PMX1–PMX9 carry the new evidence.
