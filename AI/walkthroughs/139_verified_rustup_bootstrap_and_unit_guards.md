# Walkthrough 139 — Verified rustup Bootstrap and systemd Start Guards

- **Date**: 2026-09-30
- **Issues**: GitHub #260 (ONB-16), #211 (ONB-15)
- **Branch**: `fix/p3-install-bootstrap`
- **Matrix criteria**: IRP1–IRP8 (new component `install-rustup-bootstrap-and-unit-guards`)
- **ADR**: 2026-09-30 "Verified rustup Bootstrap With Committed Digests; Only the Models Manifest Gates the Daemon Start"

---

## 1. Findings

| Issue | Finding | State on `origin/main` before this branch |
|---|---|---|
| #260 | Every sandbox image ran `curl https://sh.rustup.rs \| sh`; the toolchain channel floated | Channel already pinned to 1.98.1 (ADR "Pinned Rust Toolchain 1.98.1"); the unverified `curl \| sh` remained in `Dockerfile`, `tests/docker/Dockerfile.{ubuntu,fedora,arch}`, and the README pointed to rustup.rs |
| #211 | `soos-daemon.service` had no start guards; no readiness signal | Already fixed by walkthrough 116: `ConditionPathExists=/var/lib/soos/models/manifest.toml`, `StartLimitIntervalSec=60` / `StartLimitBurst=5`, `install.sh --start` + `scripts/wait_daemon_ready.sh`, fatal missing `soos` group (IDT6–IDT8). Only textual checks existed: systemd silently ignores a malformed condition line and starts the unit anyway |

## 2. Specification

- `scripts/install_rustup.sh [--default-toolchain <x.y.z>]` (bash, `set -euo pipefail`):
  - constants `RUSTUP_VERSION="1.29.1"`, `RUSTUP_INIT_SHA256_X86_64`,
    `RUSTUP_INIT_SHA256_AARCH64`, `PINNED_TOOLCHAIN="1.98.1"`;
  - argument validation first: the toolchain must match `^[0-9]+\.[0-9]+\.[0-9]+$`
    (exit 2 otherwise, nothing downloaded); a release other than the pin only warns;
  - `uname -m` → triple and digest; any other architecture exits 1 before downloading;
  - `curl --proto '=https' --tlsv1.2 --fail --location --max-time 300` into `mktemp -d`
    (removed by an `EXIT` trap);
  - `sha256sum` of the download compared with the committed digest **before** `chmod`;
    mismatch exits 1 with "checksum mismatch ... refusing to execute it";
  - `rustup-init -y --profile minimal --default-toolchain <toolchain>`.
- Dockerfiles: `COPY scripts/install_rustup.sh /usr/local/lib/soos/install_rustup.sh`, then
  `RUN bash /usr/local/lib/soos/install_rustup.sh --default-toolchain 1.98.1 && rustup component add clippy rustfmt`
  (Fedora keeps `RUSTUP_INIT_SKIP_PATH_CHECK=yes`). `.dockerignore` becomes
  `*` + `!scripts/install_rustup.sh`, so the build context is still a single small file.
- Design decisions (ADR): digests are committed in the script, not fetched from the archive's
  `.sha256` (same-host checksums only prove transport integrity); no `ConditionPathExists=` for
  `daemon.toml` (optional) or `master.key` (created on first start).

## 3. Tests First (TDD Red)

New modules, registered in `tests/invariants/src/lib.rs`:

- `rustup_bootstrap_contract.rs` (IRP1–IRP5): pins, Dockerfile usage, `.dockerignore`, and two
  hermetic executions with a stub `curl` first in `PATH` that "downloads" a tampered
  `rustup-init` which would create a marker file if run, plus a stub `uname` printing `riscv64`.
- `unit_start_guard_contract.rs` (IRP6–IRP7): only the models manifest is a path condition, and
  `systemd-analyze condition` itself evaluates each `ConditionPathExists=` (re-rooted into a
  scratch directory) as failed without the file and satisfied with it. The test skips when a
  probe shows `systemd-analyze` cannot evaluate conditions in the environment.

Red evidence on the unmodified tree
(`cargo test --locked -p soos-invariants --all-features -- rustup_bootstrap_contract unit_start_guard_contract`):
`2 passed; 5 failed` — all five `rustup_bootstrap_contract` tests failed (script missing,
Dockerfiles still `curl | sh`, `.dockerignore` without the exception, README without the
script); the two `unit_start_guard_contract` tests passed because #211 was already implemented
(they add systemd-evaluated evidence, not a new behaviour).

## 4. Audit

- No Rust production code changed; no PAM, FFI, IPC or logging path touched.
- The installer never executes or marks executable an unverified file; temporary files live in a
  private `mktemp -d` directory removed on every exit path; downloads are HTTPS-only and bounded
  in time; argument values are validated by a strict regex before use (no injection into
  `rustup-init`).
- Supply chain: the trust root moves from "whatever sh.rustup.rs serves today" to two digests
  reviewed in git. Note that `rustup-init` modifies the invoking user's shell profiles
  (`~/.profile`, `~/.bashrc`, `~/.zshenv`, ...) exactly like the rustup.rs script; in Docker
  this is irrelevant (PATH is set by `ENV`).

## 5. Implementation (TDD Green)

`scripts/install_rustup.sh` (new, 0755), the four Dockerfiles, `.dockerignore`, and the
documentation: `README.md` quick start, `Docs/CI_CD_AND_SECURITY.md` "Verified rustup
Bootstrap", `Docs/PACKAGING_AND_PROVISIONING.md` §3 and the `scripts/check_build_deps.sh`
missing-toolchain hint.

Green: `cargo test --locked -p soos-invariants --all-features` → 253 passed. A real run of
`scripts/install_rustup.sh --default-toolchain 1.98.1` with scratch `CARGO_HOME` / `RUSTUP_HOME`
verified the x86_64 digest against the live archive and installed `rustc 1.98.1`.

## 6. Remaining Work

- IRP8: observe `condition failed` on a booted systemd host or a privileged systemd container.
- No `bootstrap.sh` one-liner exists in the repository; publishing a SHA-256 of such a script
  with releases (ONB-16 recommendation) applies once it exists.
- A `--use-system-rust` path (accept a distribution `rustc` at or above an MSRV) needs a
  `rust-version` in the workspace manifest first; not introduced here.
