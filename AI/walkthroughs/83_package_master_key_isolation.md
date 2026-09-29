# Walkthrough 83 — Package Master Key Isolation (ONB-01)

- **Date**: 2026-09-29
- **Issue**: Full project review finding ONB-01 (GitHub #144) — **Branch**: `fix/package-master-key`
- **Matrix criteria**: PK1 (corrected), PMK1–PMK6 (new)

---

## 1. Context & Objectives

`scripts/install.sh` step 3 generated `/var/lib/soos/master.key` whenever the file was absent, with
no guard on `DESTDIR`. `scripts/build_deb.sh`, `scripts/build_arch.sh` and `packaging/debian/rules`
all stage the package tree through `install.sh --destdir <stage> --skip-models --skip-systemd`, so
every `.deb` and every Arch package built by those scripts contained a 32-byte AES-256-GCM master
key next to the binaries. On the target, `packaging/debian/postinst` and `packaging/arch/soos.install`
skipped generation because the file already existed: the shipped key was the live key.

Consequences (all confirmed by the review verifier and re-verified here before any change):

1. Every machine installed from the same artifact shared the key protecting all biometric templates,
   and anyone holding the package could decrypt any `<uid>.cbor.enc` copied from any host
   (breaks `AI/ARCHITECTURE.md` §9 "encrypted at rest, root-only key").
2. The key was a package-owned file: overwritten on every upgrade (all enrolled templates become
   undecryptable) and deleted by `dpkg -r`, contradicting `postrm` ("Retain biometric vector store").
3. `tests/invariants` asserted the defect (`master.key must be created` under `--destdir`).

Repro before the fix (scratchpad): `bash scripts/install.sh --destdir <stage> --skip-models
--skip-systemd` → exit 0, `find <stage> -type f` lists `var/lib/soos/master.key` (32 bytes, 0600).

Objectives: (a) a staging tree never contains key material; (b) every package generates the key on
the target host at first install, `0600 root:root`, without ever overwriting an existing key;
(c) builders fail closed if a key is staged; (d) tests written first, Docker evidence on real packages.

## 2. Architect Design

Scope is infra/packaging only; no Rust production crate changes.

| Item | Location | Specification |
|---|---|---|
| Staging guard in installer | `scripts/install.sh` step 3 | `DESTDIR` non-empty ⇒ no key generation (informational message only). `DESTDIR` empty ⇒ `sh scripts/provision_master_key.sh --state-dir "${TARGET_STATE_DIR}"`. |
| Shared key generator | `scripts/provision_master_key.sh` → installed as `/usr/libexec/soos/provision-master-key` (0755) | POSIX `sh`, `set -eu`; `--state-dir <DIR>` (default `/var/lib/soos`), `-h`. Sentinels: symlink at key path ⇒ exit 1; existing non-regular file ⇒ exit 1; existing regular file ⇒ `chmod 0600` (+ `chown root:root` when uid 0), exit 0, content untouched; absent ⇒ `mkdir -p`, `umask 077`, write `master.key.tmp.$$` from `openssl rand 32` or `head -c 32 /dev/urandom`, verify exactly 32 bytes, `chmod 0600`/`chown`, publish with `ln` (fails if the destination appeared meanwhile ⇒ keep the concurrent key), temporary file removed by `trap`. Output: paths and statuses only. |
| Build guard | `scripts/check_no_key_material.sh <dir>` | bash, `set -euo pipefail`; exit 1 if `<dir>` missing or if `find <dir> \( -type f -o -type l \) -name '*.key'` yields anything (result captured in a variable, not piped to `grep -q`). Called by `build_deb.sh`, `build_arch.sh` (after staging) and `packaging/debian/rules` (`override_dh_auto_install`). |
| Scriptlets | `packaging/debian/postinst` (`configure`), `packaging/arch/soos.install` (`post_install`, reused by `post_upgrade`), `packaging/rpm/soos.spec` (`%post`) | Replace the inline generator with `/usr/libexec/soos/provision-master-key --state-dir <state dir>`; RPM `%install` stages the helper and `%files` lists it; `master.key` stays `%ghost %attr(0600, root, root)`. `PKGBUILD package()` installs the helper. |
| Uninstaller | `scripts/uninstall.sh` | Removes `${TARGET_LIBEXEC_DIR}/provision-master-key`; key retention semantics unchanged (`--keep-data` default). |

One source of truth: the only `openssl rand` / `/dev/urandom` invocation in the repository's
installers and scriptlets lives in `provision_master_key.sh`; the invariant test greps for it.

Invariants touched: `AI/ARCHITECTURE.md` §9 (root-only key, encrypted at rest), runtime path table
(`/var/lib/soos/master.key` 0600 root:root), auditor checklist item 6 (secrets created 0600 at
creation, no chmod-after-write window at the final path).

Documentation drift / ADR: none. `AI/VERIFICATION_MATRIX.md` PK1 described `install.sh` as
"generates 32-byte master.key" without the live-only qualification; corrected in place.

## 3. Plan Evaluation

Condensed for an infra-only change (dev-workflow §1). Adversarial checks performed against the code:

- RPM path already used `%ghost` + `%post` generation and was unaffected, but `%post` used `|| :` on
  the generator (a failed generation was silent). The helper call is now unguarded so a failure is
  reported by `rpm`.
- `postinst` runs on every `configure` (install and upgrade): the helper must be idempotent, which
  PMK2 proves.
- Debian's `/bin/sh` is `dash`: the helper and `postinst` are POSIX `sh` (verified in the
  `ubuntu:24.04` container, `/bin/sh -> /usr/bin/dash`).
- `find | grep -q` under `pipefail` can return the SIGPIPE status of `find` and silently pass: the
  guard captures `find` output in a variable instead.

Verdict: APPROVED (no revision needed).

## 4. Tester Contract

| Test (`tests/invariants/src/lib.rs`) | Acceptance line / matrix ID | Red evidence (before implementation) |
|---|---|---|
| `test_install_script_creates_required_directories` (migrated) | #144.1 / PK1, PMK1 | `master.key must NOT be generated under --destdir (GitHub #144)` |
| `test_install_script_destdir_stages_no_key_material` | #144.1 / PMK1 | `staged tree must contain no *.key file, found: [".../var/lib/soos/master.key"]` |
| `test_provision_master_key_helper_generates_0600_key_once` | #144.2 / PMK2 | `scripts/provision_master_key.sh must exist` |
| `test_provision_master_key_helper_refuses_symlink_and_non_regular` | #144.2 / PMK3 | `scripts/provision_master_key.sh must exist` (assertion added so the test cannot pass vacuously) |
| `test_package_scriptlets_provision_key_via_shared_helper` | #144.2 / PMK4 | `debian postinst configure branch must call /usr/libexec/soos/provision-master-key` |
| `test_uninstall_restores_pam_config` (strengthened) | #144.2 / PMK4 | `provision-master-key helper must be removed (GitHub #144)` |
| `test_package_builders_refuse_staged_key_material` | #144.3 / PMK5 | `scripts/check_no_key_material.sh must exist` |
| `tests/docker/test_packages.sh` (`verify_package_has_no_key_material`, `verify_key_survives_removal`, `verify_fresh_install_generates_distinct_key`) | #144.3 / PMK6 | not runnable on the host (requires a container with release binaries); equivalent checks executed manually in Docker, see §8 |

Red run: `cargo test --locked -p soos-invariants --all-features -- install_script provision_master_key
package_scriptlets package_builders uninstall_restores` → `1 passed; 6 failed`, every failure on the
assertion listed above (the single pass was the symlink test passing vacuously, fixed by the added
existence assertion before implementation).

### Migrated existing tests

| Test | Old assertion | New assertion | Mandating acceptance line |
|---|---|---|---|
| `test_install_script_creates_required_directories` | `master_key.is_file()`, mode `0600`, `len == 32` under `--destdir` | `!master_key.exists()` under `--destdir` (mode/size assertions moved to the helper test PMK2, run against a temporary state dir) | GitHub #144 / #144.1: a staged tree is package content and must not contain key material |
| `test_uninstall_restores_pam_config` | — | additionally asserts `usr/libexec/soos/provision-master-key` is removed | #144.2 |

No assertion was weakened: the mode/size checks now run on the actual generated key (PMK2) and the
Docker test checks them on the installed key.

### Flakiness check

The new tests are filesystem-only (no timing). `cargo test --locked -p soos-invariants --all-features`
run 10× in a row: 26 passed each time.

## 5. Auditor Constraints

| # | Constraint | Applies to | How it was met / verified |
|---|---|---|---|
| 1 | No key generation when `DESTDIR` is non-empty | `scripts/install.sh` step 3 | `if [[ -z "${DESTDIR}" ]]` guard; PMK1 tests |
| 2 | Key is `0600` from inception, no world-readable window at the final path | `provision_master_key.sh` | `umask 077` before the temporary file is created, publish by `ln` (the final path only ever exists as a 0600 file); `umask 077` grep in PMK2 |
| 3 | Never overwrite an existing key; never follow symlinks; verify 32 bytes before publishing; remove temporary file on failure | `provision_master_key.sh` | early `-L` / `! -f` checks, `ln` instead of `mv -f`, `wc -c` check, `trap cleanup EXIT INT TERM HUP`; PMK2/PMK3 |
| 4 | No key bytes in any output or log | `provision_master_key.sh`, scriptlets | messages contain paths only; reviewed by grep |
| 5 | Builders fail closed on staged key material; no `find | grep -q` pipefail trap | `check_no_key_material.sh`, `build_deb.sh`, `build_arch.sh`, `debian/rules` | variable capture, exit 1 on missing dir; PMK5 functional test (clean / `master.key` / nested `evidence.key` / missing dir) |
| 6 | Scriptlet failure is visible (no `|| :` around the key step) | `postinst`, `soos.install`, `soos.spec %post` | unguarded helper call; `postinst` runs under `set -e` |
| 7 | POSIX `sh` compatibility for `postinst` and the helper (Debian `dash`) | `provision_master_key.sh`, `postinst` | `sh -n` locally; executed under `dash` in the `ubuntu:24.04` container |
| 8 | `bash -n` syntax for every touched script (candid layer 1, audit 5) | all scripts | `./scripts/candid_review.sh` PASSED |
| 9 | Uninstall removes the helper, keeps the key by default | `scripts/uninstall.sh` | strengthened uninstall test |
| 10 | Tests never touch `/var/lib/soos`, `/run`, `/etc`; fixture keys are synthetic bytes in temporary dirs | `tests/invariants/src/lib.rs` | all paths under `std::env::temp_dir()` |
| 11 | English only in code, comments, docs | all files | candid layer 1 audit 7 PASSED |

Pre-existing violations found (not introduced by this change): see §9.

Clearance: CLEARED.

## 6. Implementation

Files changed:

- `scripts/install.sh` — step 3 rewritten (live-only provisioning through the helper, staging message),
  helper installed to `${TARGET_LIBEXEC_DIR}/provision-master-key`, header comments updated.
- `scripts/provision_master_key.sh` (new, POSIX `sh`) — single key generator.
- `scripts/check_no_key_material.sh` (new, bash) — fail-closed packaging guard.
- `scripts/build_deb.sh`, `scripts/build_arch.sh`, `packaging/debian/rules` — guard invoked right after staging.
- `packaging/debian/postinst`, `packaging/arch/soos.install`, `packaging/rpm/soos.spec` — inline
  generators replaced by the helper call; RPM installs and lists the helper.
- `packaging/arch/PKGBUILD` — installs the helper.
- `scripts/uninstall.sh` — removes the helper.
- `tests/docker/test_packages.sh` — archive listing has no `.key` entry (`dpkg-deb -c`,
  `rpm -qlp --noghost`, `bsdtar -tf`), installed key size check, helper presence, key survives removal,
  second fresh install yields a distinct key (deb, rpm, arch branches).
- `tests/invariants/src/lib.rs` — contract migration + five new invariants (§4).
- `AI/VERIFICATION_MATRIX.md` (PK1 corrected, PMK1–PMK6), `AI/BACKLOG.md` (GitHub #144 entry),
  `Docs/PACKAGING_AND_PROVISIONING.md` (§2, §3, §7.3, new §7.4), `Docs/DISTRIBUTION_DEPLOYMENT.md` (§3.1).

Notable decisions:

- A shipped helper instead of three copies of the generator: the scriptlets stay one line each, the
  generator is functionally testable as a non-root user against a temporary `--state-dir`, and any
  future hardening (for example a TPM-sealed key) lands in one place.
- Atomic publish by hard link rather than `set -C` redirection: a generator failure can never leave a
  truncated or empty `master.key` at the final path.
- The existing `dpkg -r` semantics are now correct by construction: the key is not package-owned, so
  removal and upgrades keep it (verified in Docker), matching `postrm` and `uninstall.sh --keep-data`.

## 7. Candid Review

Layer 1 (`./scripts/candid_review.sh`): PASSED (unsafe, PAM panics/async/prints, forbidden deps,
shell syntax for `build_arch.sh`, `build_deb.sh`, `install.sh`, `uninstall.sh`, `test_packages.sh`,
English policy). Layer 2 (fingerprint-bound `AI/candid_review_report.md`): pending — to be produced by
the independent reviewer before push, per the repository workflow.

## 8. Verification Results

Host (Arch Linux, `cargo 1.96.0`, `--locked --all-features`):

```bash
cargo fmt --all                                                                  # clean
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings    # Finished, 0 warnings
cargo test   --locked --workspace --all-targets --all-features                    # 484 passed; 0 failed (re-run, see note)
cargo test   --locked -p soos-invariants --all-features                          # 26 passed; 0 failed
./scripts/candid_review.sh                                                       # Candid Review PASSED
```

Full-workspace test note: one run reported a single failure in
`crates/daemon/tests/pipeline_init_tests.rs::test_daemon_startup_initializes_all_pipeline_components`
(socket read under full parallel load); it passes when re-run (`cargo test --locked -p soos-daemon
--all-features --test pipeline_init_tests` → 3 passed) and the full gate was re-run green afterwards.
The test and the daemon crate are untouched by this change.

Docker evidence (real packages built with `--skip-build`, so the archives contain the staged tree,
scriptlets and helper without binaries):

- `ubuntu:24.04` (`/bin/sh -> dash`): `bash scripts/build_deb.sh --skip-build -o /tmp/pkgout` →
  guard prints `no key material`; `dpkg-deb -c` lists `./usr/libexec/soos/provision-master-key`,
  `./var/lib/soos/{biometrics,evidence,models}/` and **no** `.key` entry; `dpkg -i` prints
  `provision-master-key: master key generated at /var/lib/soos/master.key (mode 0600).`;
  `stat` → `600 root:root 32`; `dpkg -r` keeps the key and removes the helper; `rm master.key` +
  `dpkg -i` → distinct SHA-256; a further `dpkg -i` over the existing key leaves it unchanged.
- `archlinux:base`: `bash scripts/build_arch.sh --skip-build -o /tmp/pkgout` → guard OK, `bsdtar -tf`
  has no `.key` entry; `pacman -Udd --overwrite '*'` (see §9 for the `/usr/lib64` conflict) →
  `post_install` generates `600 root:root 32`; key survives `pacman -R`; second fresh install → distinct key.

Before the fix, the same `install.sh --destdir` invocation produced `var/lib/soos/master.key` in the
stage (repro in §1).

## 9. Known Limitations / Follow-ups

- Pre-existing, out of scope: `scripts/build_arch.sh` stages through `install.sh`, whose PAM directory
  auto-detection falls back to `/usr/lib64/security` on hosts where `/usr/lib64` exists; the resulting
  package owns `usr/lib64/`, which conflicts with the `filesystem` package's `/usr/lib64 -> lib` symlink
  on Arch (`pacman -U` fails with "conflicting files" without `--overwrite`). The official Arch recipe
  (`packaging/arch/PKGBUILD`) installs to `/usr/lib/security` and is unaffected. Suggested fix:
  `build_arch.sh` should pass `--pam-dir /usr/lib/security`.
- `tests/docker/test_packages.sh` requires release binaries inside the distro containers and is not
  part of the host `cargo test` gate; the Docker runs above exercised the same assertions manually.
- Hosts that were installed from a previously built package already share a key: administrators
  should delete `/var/lib/soos/master.key`, re-run `/usr/libexec/soos/provision-master-key` (or
  reinstall), and re-enroll users. No automatic rotation is attempted, because it would silently
  invalidate existing enrollments.
