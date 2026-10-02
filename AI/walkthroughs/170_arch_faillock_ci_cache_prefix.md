# Walkthrough 170 — Arch Face Login Through faillock `authsucc`, Pinned systemd Image, Cached ONNX Runtime Download, Prefix-Aware GUI Paths

- **Date**: 2026-10-02
- **Issue**: GitHub #318 (follow-ups of the 2026-10-02 review batch, PR #317), items "Arch face
  login and faillock", "CI supply chain" and "Absolute program paths assume `/usr`"; no backlog
  issue (GitHub-only, not registered in `scripts/sync_issue.py`) — **Branch**:
  `fix/arch-faillock-ci-prefix`
- **Matrix criteria**: AFC1–AFC8 (component `arch-faillock-ci-cache-prefix`, after the
  `packaging-ownership-and-arch-pam` section); POA5 and POA9 wording updated

## 1. Context & Objectives

1. **Arch face login and faillock.** The Arch primary rule was `auth [success=done default=ignore]
   pam_soos.so`. A face match ended the auth stack before `pam_env.so` and `pam_faillock.so
   authsucc`, so earlier wrong passwords stayed in the faillock tally (a face login did not count as
   a successful login). Owner decision 2026-10-02: on Arch a face match must go through `authsucc`
   and `pam_env` like a correct password, while a locked account still fails at `preauth`.
2. **`tests/docker/Dockerfile.systemd`** used the bare tag `FROM ubuntu:24.04` (exempted from the
   digest pinning of walkthrough 166 because `systemd_unit_acceptance_contract` pinned that line).
   Owner decision: pin it by digest and make the invariant stricter.
3. **`ort-sys` ONNX Runtime download** used by required checks was neither cached nor retried: a
   transient CDN/TLS error (`native-tls: unexpected EOF`, matrix PK7/DV3) failed a whole job.
4. **Absolute program paths assume `/usr`.** `soos-gui` runs `/usr/bin/soos-enroll` through
   `pkexec`; `install.sh --prefix /opt/soos` installed it elsewhere and the GUI's privileged
   actions failed (closed). Owner decision: derive `SOOS_ENROLL_PROGRAM` from a build-time
   `SOOS_BINDIR` (default `/usr/bin`), exported by `install.sh --build`; packages stay at `/usr`;
   `pkexec` / `systemctl` stay `/usr/bin`.

Out of scope (other agent): the daemon, `soos-enroll` and `soos-admin` items of #318.

## 2. Architect Design

- **Arch stack** (`packaging/pam/arch/system-auth`, `system-auth.snippet`): primary rule
  `auth  [success=4 default=ignore]  pam_soos.so`. Counted from the soos rule, the four skipped
  rules are `-auth pam_systemd_home.so`, `pam_unix.so`, the event line and `[default=die]
  pam_faillock.so authfail`; the landing rule is `auth optional pam_permit.so`, followed by
  `pam_env.so` and `pam_faillock.so authsucc` — the path a correct password takes through the
  widened `success=3` / `success=2` jumps. `authsucc` resets the tally only when the account is not
  locked (`check_tally` first), so a locked account still fails twice over (`preauth` is
  `required` and runs before soos; `authsucc` is `required`). `default=ignore` is unchanged: an
  error, timeout, panic, missing module or `PAM_IGNORE` still falls through to the password.
  ADR "Arch Face Match Runs the Stock Success Path" amends ARCHITECTURE §5 for Arch only; Debian,
  Fedora and GDM keep `success=done`.
- **systemd image**: `FROM ubuntu:24.04@sha256:008173c2…3ca3`, the digest of
  `tests/docker/Dockerfile.ubuntu` (the release build image; same glibc).
- **ORT cache**: ort-sys extracts the verified archive into `ORT_CACHE_DIR` (default
  `~/.cache/ort.pyke.io`, `dfbin/<target>/<sha256>`); a present directory is never downloaded
  again. New `scripts/prefetch_onnxruntime.sh` (`cargo check --locked -p ort`, same default ort
  features as every crate, `SOOS_ORT_FETCH_ATTEMPTS` 1..5 default 3, `SOOS_ORT_FETCH_DELAY_S` 0..60
  default 15, linear back-off; exit 0/1/2). CI: `actions/cache/restore` / `save` pinned to
  `55cc8345863c7cc4c66a329aec7e433d2d1c52a9` (v6.1.0), key `ort-<os>-<hash(Cargo.lock)>`, saved by
  `main` only on a miss. Docker harnesses: optional `SOOS_ORT_CACHE_DIR` (existing absolute
  directory) mounted on `/ort-cache` with `ORT_CACHE_DIR=/ort-cache`; the systemd harness otherwise
  uses the named volume `soos-sua-ort-cache` (its prefetch and build run in two containers).
  `run_distro_validation.sh` also forwards `CARGO_BUILD_JOBS` when set.
- **GUI paths**: `crates/gui/build_support/bindir.rs` (`DEFAULT_BINDIR`, `MAX_BINDIR_LEN` = 256,
  `validate_bindir`, `enroll_program_for`) shared by `crates/gui/build.rs` and the contract test.
  `build.rs` emits `rerun-if-env-changed=SOOS_BINDIR`, fails the build (`cargo::error`) on an
  invalid value and enables `cfg(soos_custom_bindir)` for a valid value other than `/usr/bin`;
  `SOOS_ENROLL_PROGRAM` is then `concat!(env!("SOOS_BINDIR"), "/soos-enroll")`, otherwise the
  unchanged literal `/usr/bin/soos-enroll`.

## 3. Plan Evaluation

Condensed (owner decisions fixed the design). Checks: the `success=4` count was verified against
the stock pambase `20260616-1` file and is proven by the Docker harness on the real stack (a wrong
count fails the byte-equality step or the faillock scenarios); `authsucc` on a locked account does
not reset the tally (asserted in Docker); no PAM code changed, so the `PAM_IGNORE` fallback paths
are untouched; the ORT cache is only written from `main` (no cache poisoning from pull requests)
and ort-sys verifies the archive hash before extraction; the GUI path value is validated before it
can reach a root argument vector.

## 4. Tester Contract

| Test | Criterion | Red evidence |
|---|---|---|
| `arch_faillock_ci_contract::test_arch_face_match_lands_on_faillock_authsucc_and_pam_env` | AFC1 | `left: "[success=done default=ignore]" right: "[success=4 default=ignore]"` |
| `arch_faillock_ci_contract::test_arch_harness_face_login_resets_tally_and_lock_still_fails` | AFC2 | harness lacked `print "auth  [success=4 default=ignore]  pam_soos.so"` |
| Docker `tests/distro/run_distro_validation.sh arch` (new scenarios 6g/6h, `success=done` stack) | AFC2 | `[ERROR] face Allow after 2 wrong password(s): faillock tally is 2, expected 0` |
| `arch_faillock_ci_contract::test_systemd_runtime_image_uses_the_build_image_digest` | AFC3 | `Dockerfile.systemd must pin ubuntu:24.04 by digest` |
| `arch_faillock_ci_contract::test_ci_caches_and_retries_the_onnxruntime_download` | AFC4 | `clippy: must restore the ORT cache (uses: actions/cache/restore@55cc… # v6.1.0)` |
| `arch_faillock_ci_contract::test_onnxruntime_prefetch_retries_a_bounded_number_of_times` | AFC5 | `2 failures then success` (script absent, exit 127) |
| `arch_faillock_ci_contract::test_docker_harnesses_prefetch_and_share_the_onnxruntime_cache` | AFC6 | `tests/distro/debian_ubuntu_test.sh must prefetch ONNX Runtime` |
| `crates/gui/tests/program_path_tests.rs::test_soos_enroll_program_follows_build_time_bindir` | AFC7 | with `SOOS_BINDIR=/opt/soos/bin`: `left: "/usr/bin/soos-enroll" right: "/opt/soos/bin/soos-enroll"` |
| `crates/gui/tests/program_path_tests.rs::test_bindir_validation_accepts_normalized_absolute_directories` | AFC7 | stub: `left: Ok("/usr/bin/soos-enroll") right: Ok("/opt/soos/bin/soos-enroll")` |
| `crates/gui/tests/program_path_tests.rs::test_bindir_validation_rejects_relative_and_unsafe_paths` | AFC7 | stub: `"" must be rejected` |
| `crates/gui/tests/program_path_tests.rs::test_system_tools_stay_under_usr_bin` | AFC7 | regression guard (passes before and after) |
| `arch_faillock_ci_contract::test_install_build_exports_the_prefix_bindir` | AFC7 | runuser call `… cargo build --release --locked --workspace _ <checkout>` without `SOOS_BINDIR` |
| `arch_faillock_ci_contract::test_package_builders_pin_soos_bindir_to_usr_bin` | AFC8 | `scripts/build_deb.sh must contain export SOOS_BINDIR=/usr/bin` |

### Migrated existing tests (owner-approved 2026-10-02)

| Test | Old assertion | New assertion | Red on the old stack |
|---|---|---|---|
| `soos-invariants::tests::test_pam_config_ordering_matches_spec` (Arch branch only) | Arch primary rule and snippet line match `[success=done default=ignore] pam_soos.so` | Arch primary rule and snippet line match `[success=4 default=ignore] pam_soos.so` (new helpers `soos_rule_offset_with_control` / `is_default_timeout_soos_rule_with_control`; Debian and Fedora still use `[success=done default=ignore]`) | `no primary [success=4 default=ignore] pam_soos.so rule relying on the default timeout` |
| `packaging_ownership_contract::test_arch_system_auth_is_stock_pambase_with_exact_soos_edit` | rule 1 control `[success=done default=ignore]`; `ARCH_PRIMARY_LINE` = `auth  [success=done default=ignore]  pam_soos.so` | rule 1 control `[success=4 default=ignore]`; `ARCH_PRIMARY_LINE` = `auth  [success=4 default=ignore]  pam_soos.so` (its existing "every `success=N` lands on `pam_permit.so`" loop now also covers the soos rule) | `auth rule 1 … left: "[success=done default=ignore]" right: "[success=4 default=ignore]"` |
| `systemd_unit_acceptance_contract::test_systemd_acceptance_harness_exists_and_isolates_the_host` | `dockerfile.contains("\nFROM ubuntu:24.04\n")` | a line `FROM ubuntu:24.04@sha256:` + exactly 64 hex digits (stricter: the bare tag fails) | `must use ubuntu:24.04 pinned by digest` |
| `packaging_ownership_contract::test_docker_base_images_are_pinned_by_digest` | list without `tests/docker/Dockerfile.systemd` (documented exemption) | `tests/docker/Dockerfile.systemd` added to the list (stricter) | `base image not pinned by digest: FROM ubuntu:24.04` |

Test-infrastructure only: `scratch`, `combined`, `root_shims`, `AuthRule`, `auth_rules` and
`success_jump` of `packaging_ownership_contract` became `pub(crate)` (no assertion changed). No
other existing assertion changed: GCV19 (`gcv_review_contract`) still finds the literal
`pub const SOOS_ENROLL_PROGRAM: &str = "/usr/bin/soos-enroll";` (the default `cfg` branch), and the
`import_privacy_tests` expectations of `/usr/bin/soos-enroll` hold for every default build (they
fail by design when the test binary itself is built with a non-default `SOOS_BINDIR`).

## 5. Auditor Constraints

1. No change to `crates/pam`: every PAM failure path keeps returning `PAM_IGNORE`; the Arch rule
   keeps `default=ignore`, so a missing module or `PAM_IGNORE` never grants nor denies — met.
2. A face match must never bypass a lock: `preauth` stays before soos; asserted in Docker (6h:
   `result=7`, tally kept at 3) — met.
3. No new permission in CI; every new `uses:` pinned by a 40-hex SHA; the cache is written by
   `main` only; nothing from `github.event` is interpolated — met (AFC4).
4. Bounded retry: attempts ≤ 5, delay ≤ 60 s, validated before anything runs — met (AFC5).
5. `SOOS_ORT_CACHE_DIR` must be an existing absolute directory, else the harness fails before
   Docker runs — met (AFC6). The privileged systemd container never sees the ORT cache (only the
   unprivileged builder containers mount it); the host-isolation guards are untouched.
6. `SOOS_BINDIR` reaches a root `pkexec` argument vector: strict allow-list validation at build
   time, failure is a build error, no fallback to an unvalidated value — met (AFC7).
7. Packages are not affected by a stray caller environment (`SOOS_BINDIR=/usr/bin` exported by
   every builder) — met (AFC8).
8. English only, repository-relative paths — met.

## 6. Implementation

- PAM: `packaging/pam/arch/system-auth`, `packaging/pam/arch/system-auth.snippet`,
  `packaging/arch/soos.install` (comment), `tests/distro/arch_linux_test.sh` (edit applier,
  `faillock_deny_limit`, `wrong_passwords`, scenarios 6g/6h); docs `Docs/DISTRIBUTION_DEPLOYMENT.md`
  §2/§5.2, `Docs/PACKAGING_AND_PROVISIONING.md` §4, `AI/ARCHITECTURE.md` §5, `AI/DECISIONS.md`.
- Images: `tests/docker/Dockerfile.systemd`.
- ORT: `scripts/prefetch_onnxruntime.sh` (new), `.github/workflows/ci.yml` (clippy, test,
  package-deploy, systemd-unit), `tests/distro/{debian_ubuntu,fedora_rhel,arch_linux}_test.sh`,
  `tests/distro/run_distro_validation.sh`, `tests/docker/systemd_unit_acceptance_test.sh`,
  `Docs/CI_CD_AND_SECURITY.md`.
- GUI: `crates/gui/build.rs` (new), `crates/gui/build_support/bindir.rs` (new),
  `crates/gui/src/privileged.rs`, `scripts/install.sh`, `scripts/build_{deb,arch,rpm,packages}.sh`,
  `packaging/debian/rules`, `packaging/arch/PKGBUILD`, `packaging/rpm/soos.spec`,
  `Docs/GUI_APPLICATION.md`, `Docs/PACKAGING_AND_PROVISIONING.md`.
- Tests: `tests/invariants/src/arch_faillock_ci_contract.rs` (new), `crates/gui/tests/program_path_tests.rs`
  (new), migrations listed in §4.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`) run locally (§8). The layer-2 sub-agent review, `save.sh`,
push and PR were not run in this batch (orchestrator instruction); they remain for the release step.

## 8. Verification Results

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --locked -p soos-gui -p soos-invariants --all-targets --all-features -- -D warnings`:
  clean (also `SOOS_BINDIR=/opt/soos/bin cargo clippy --locked -p soos-gui --lib --all-features`).
- `cargo test --locked --all-features -p soos-invariants -p soos-gui`: all passed (invariants lib
  378 passed; `program_path_tests` 4 passed).
- `SOOS_BINDIR=opt/x cargo check -p soos-gui` → `error: SOOS_BINDIR must be an absolute path (got "opt/x")`;
  `SOOS_BINDIR='/opt/a b'` → `may only contain [A-Za-z0-9._+-] and '/'`.
- `bash -n` and shellcheck (`koalaman/shellcheck:stable`) on every changed script: no finding on a
  changed line (only pre-existing SC2034/SC2329/SC1091/SC2012/SC2035 infos and warnings remain).
- `tests/distro/run_distro_validation.sh arch` (Docker, real stock pambase `20260616-1`): passed.
  Stock + edit == `packaging/pam/arch/system-auth`; face Allow on both lockers; valid password
  (daemon absent / after Deny) with no event and tally 0; wrong password with one event and tally
  1; deny limit 3; 6g: two wrong passwords then face Allow `result=0`, tally reset to 0; 6h: three
  wrong passwords then face Allow `result=7 (Authentication failure) prompts=0`, tally kept at 3;
  `pacman -R` rollback. Prefetch inside the container: `ONNX Runtime is available in the ort-sys cache`.
- `./tests/docker/systemd_unit_acceptance_test.sh --models host` (pinned image): passed — prefetch in the builder container
  (`attempt 1/3`, then `ONNX Runtime is available in the ort-sys cache`), Type=notify readiness
  (start-to-ready 205 ms), healthy `soos-admin status`, clean stop, host sysctls unchanged.
- `./scripts/candid_review.sh` (layer 1): `Candid Review PASSED`.

## 9. Known Limitations / Follow-ups

- The ORT cache and prefetch cannot be exercised before the workflow runs on GitHub; the first
  `main` run after the merge seeds the cache (pull requests only restore it).
- The `distro-deploy` and `distro-pam-matrix` jobs (push to `main` only, not part of `CI Success`)
  do not restore the cache; their harnesses still prefetch with the bounded retry.
- A `soos-gui` test binary built with a non-default `SOOS_BINDIR` fails the existing
  `import_privacy_tests` `/usr/bin/soos-enroll` expectations by design; CI builds with the default.
- A future pambase that adds or removes a line between the soos rule and `pam_permit.so` needs the
  `success=4` count updated; the Arch harness fails on such drift (byte equality and 6g/6h).
- Remaining #318 items (daemon `allow_virtual_camera`, `gdm restore` lock, `logs --file`, i686
  note) belong to the other batch.
