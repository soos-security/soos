# Quality Pipeline, CI/CD, and Security Auditing

> Project: `soos`  
> Status: Operational  

---

## 1. Quality Gates Overview

The project enforces compliance across four concentric verification levels. The same commands
and flags run at every level, so a green local run predicts a green CI run.

```
┌─────────────────────────────────────────────────────────────┐
│  Level 1: git hooks (.githooks/, core.hooksPath)            │
│  - pre-commit : anti-commit main, secret scan (staged),     │
│                 candid layer 1 (deterministic invariants)   │
│  - commit-msg : Conventional Commits 1.0.0                  │
│  - pre-push   : anti-push main, secret scan (unpushed       │
│                 commits), candid layer 2 (fingerprint)      │
├─────────────────────────────────────────────────────────────┤
│  Level 2: save.sh (local quality pipeline)                  │
│  - cargo fmt                                                │
│  - cargo clippy --locked --workspace --all-targets          │
│                 --all-features -- -D warnings               │
│  - cargo test   --locked --workspace --all-targets          │
│                 --all-features                              │
│  - cargo deny --locked check                                │
│  - scripts/candid_subagent.sh (layers 1 + 2)                │
└──────────────────────────────┬──────────────────────────────┘
                               │ git push / Pull Request
                               ▼
┌─────────────────────────────────────────────────────────────┐
│  Level 3: GitHub Actions CI (.github/workflows/ci.yml)      │
│  parallel: lint │ clippy │ test │ security                  │
│  after lint: pam-integration │ authselect-profile           │
│              pam-rollback │ package-deploy                  │
│  after all:  ci-success (aggregate gate)                    │
│  main/dispatch only: distro-pam-matrix │ distro-deploy      │
│  separate:   pr-title.yml (PR title convention)             │
└──────────────────────────────┬──────────────────────────────┘
                               │ green "CI Success"
                               ▼
┌─────────────────────────────────────────────────────────────┐
│  Level 4: scripts/pr_loop.sh                                │
│  - wait until CI started on the exact pushed SHA, then      │
│    gh pr checks --watch --fail-fast (45 min ceiling)        │
│  - poll 'CI Success' on that SHA until completed = success  │
│  - gh pr merge --squash --match-head-commit <that sha>      │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. GitHub Actions Workflow (`.github/workflows/ci.yml`)

Triggers: pull requests to `main`, pushes to `main`, a daily schedule (supply-chain job only),
and manual dispatch. Superseded pull-request runs are cancelled automatically; every other run
has its own concurrency group (event + commit), so runs on `main` are never cancelled — they
seed the shared caches. A separate workflow, `.github/workflows/pr-title.yml`, validates the PR
title with `.githooks/commit-msg` on `opened`, `edited`, `reopened` and `synchronize`, so
retitling a PR re-validates it without re-running the whole pipeline.

| Job | Purpose | Notes |
|---|---|---|
| `lint` | `cargo fmt --check`, `bash -n` + ShellCheck (errors) on every script and hook, candid layer 1, candid layer 2 on the PR head, secret scan of every commit in the PR/push range | No compilation (~1 min) |
| `clippy` | `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | Cached `target/` |
| `test` | `cargo test --locked --workspace --all-targets --all-features` (build step separated from run step) | Cached `target/` |
| `security` | `cargo deny --locked check` with cargo-deny 0.20.2 | Also runs daily for new RustSec advisories |
| `pam-integration` | Dockerized PAM matrix T1–T15 (`tests/docker/test_suite.sh`): every case exits non-zero on a failed expectation; T2/T2b assert the elapsed time against the `timeout_ms` of the stack under test; T13–T15 cover `Deny`, a truncated body and malformed responses (GitHub #189) | Starts after `lint`; Buildx layer cache |
| `authselect-profile` | Fedora `authselect` profile activation, `authselect check`, generated stack ordering, `nsswitch.conf` preservation, password fallback and rollback in `fedora:40` (`tests/docker/authselect_profile_test.sh`) | Starts after `lint`; stock image, no build |
| `pam-rollback` | Failing live `scripts/install.sh` runs roll back completely (D0a/D0b), Debian `pam-auth-update` stack order (password-failed hook before `pam_deny`) and byte-for-byte PAM rollback by `scripts/uninstall.sh` (sha256 of every `/etc/pam.d` entry, `authselect current`) in `ubuntu:24.04` and `fedora:40` (`tests/docker/pam_rollback_test.sh`) | Starts after `lint`; stock images, no build |
| `package-deploy` | Ubuntu packaging and deployment path (GitHub #168): `tests/distro/run_distro_validation.sh ubuntu` (release workspace build, `.deb` built and installed with `dpkg -i`, filesystem invariants, `soos-enroll --mock enroll`, facial auth against the mock daemon, password fallback, rollback) in the `tests/docker/Dockerfile.ubuntu` image, then `tests/docker/test_packages.sh` on the same target volume (no key material in the `.deb`, `0600` 32-byte key generated on the host, key survives `dpkg -r`, distinct keys across fresh installs) | Starts after `lint`; runs on every pull request; full release build (~15–25 min, 75 min ceiling) |
| `ci-success` | Fails unless every job above succeeded | Single check to require in branch protection |
| `distro-pam-matrix` | PAM matrix T1–T15 in the Fedora and Arch sandbox images (`tests/docker/run_matrix.sh fedora\|arch`, GitHub #162) | Push to `main` and manual dispatch only (rebuilds toolchain and module per image); not part of `ci-success` |
| `distro-deploy` | Fedora and Arch packaging and deployment paths (GitHub #274): `tests/distro/run_distro_validation.sh fedora\|arch` (release build with `--locked`; Fedora: RPM built with `rpmbuild`, `rpm -i`, `authselect select custom/soos with-faillock`, `sudo`/`gdm` stacks; Arch: `build_arch.sh`, `pacman -U`, `system-auth`/screen-locker stacks; both: `soos-enroll --mock`, facial auth, password fallback, rollback), then the RPM or Arch branch of `tests/docker/test_packages.sh` in the same image and target volume | Push to `main` and manual dispatch only (full release build per image, 90 min ceiling); `fail-fast: false`; not part of `ci-success` because it is skipped on pull requests |

### Performance Design
- **Parallel jobs**: clippy, test, security and lint run concurrently; the critical path is the
  test job instead of the sum of all steps. The previous serial pipeline took ~8.5 minutes, of
  which 1.6 minutes were spent deleting preinstalled runner software.
- **Compilation cache**: `Swatinem/rust-cache` caches the registry and `target/` per job, keyed
  on the toolchain and `Cargo.lock`. Only `main` writes caches; pull requests restore them.
- **No debuginfo in CI** (`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`): faster
  links and a smaller cache (a full test build fits in ~3 GB, so no disk cleanup is required).
- **Docker layer cache**: the PAM sandbox image is built with Buildx and the GitHub Actions cache
  backend, so an unchanged `Dockerfile` costs seconds instead of a full `apt-get` + `rustup` install.
- **Empty Docker build context** (`.dockerignore`): the sandbox Dockerfiles never `COPY` sources,
  so `target/` is never uploaded to the Docker daemon. The only file sent is
  `scripts/install_rustup.sh` (verified rustup bootstrap, below).

### Pinned Rust Toolchain
`rust-toolchain.toml` pins `channel = "1.98.1"` (components `clippy`, `rustfmt`; user decision
2026-09-30, see `AI/DECISIONS.md`). Every CI job installs it with
`rustup toolchain install --profile minimal`, which reads that file, and the sandbox images
install the same release with `--default-toolchain 1.98.1`. A floating `stable` channel let a new
compiler release introduce lints or behaviour changes between two runs of the same commit; the
pin makes local, CI and Docker results reproducible. Moving to a newer release is a deliberate
change: update `rust-toolchain.toml` and the four Dockerfiles together (enforced by
`tests/invariants/src/toolchain_pin_contract.rs`), then run the full quality gate.

### Verified rustup Bootstrap (GitHub #260)
Neither the sandbox images nor the onboarding documentation pipe `https://sh.rustup.rs` into a
shell. `scripts/install_rustup.sh` downloads `rustup-init` of a pinned rustup release
(`RUSTUP_VERSION`, currently 1.29.1) for the host triple (`x86_64` or `aarch64`
`unknown-linux-gnu`) from `https://static.rust-lang.org/rustup/archive/<version>/<triple>/`,
over HTTPS / TLS 1.2+ only, into a private `mktemp -d` directory. It compares the SHA-256 of the
download with the digest committed in the script **before** the file is made executable; a
mismatch aborts with exit 1 and nothing is executed. It then runs `rustup-init -y --profile
minimal --default-toolchain <x.y.z>`, where the toolchain defaults to the release of
`rust-toolchain.toml` and must be an exact `x.y.z` (a floating `stable` / `beta` / `nightly`
channel is a usage error, exit 2). Each Dockerfile `COPY`s the script and runs it with
`--default-toolchain 1.98.1`. On a fresh host, run `./scripts/install_rustup.sh` from a checkout
instead of the rustup.rs one-liner. Bumping rustup means changing `RUSTUP_VERSION` and both
digests together (from the archive's `rustup-init.sha256`, cross-checked with a local
`sha256sum`); `tests/invariants/src/rustup_bootstrap_contract.rs` enforces the pins, the
Dockerfile usage and the fail-closed checksum path.

**Shell profiles are never edited (GitHub #287).** `rustup-init` runs with `--no-modify-path`, so
it never appends to `~/.profile`, `~/.bashrc`, `~/.bash_profile`, `~/.zshenv` or the fish
`conf.d` (an earlier test run on a developer host had appended such lines). The script instead
exports `${CARGO_HOME:-$HOME/.cargo}/bin` in `PATH` for its own post-install `rustc --version`
check and prints the `export PATH="<cargo_home>/bin:$PATH"` line for the operator to add where
wanted. The sandbox Dockerfiles already set `PATH` with an `ENV` instruction.
`tests/invariants/src/rustup_path_contract.rs` runs the installer hermetically (stub `curl`,
`sha256sum` and `uname`; scratch `HOME`, `CARGO_HOME` and `RUSTUP_HOME` under `target/`) and checks
the flag, untouched profiles and the printed `PATH` line.

**Digest bump procedure** (manual, one commit, also in the script header):
1. Pick the new rustup release `<version>`.
2. For each triple (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`), fetch
   `https://static.rust-lang.org/rustup/archive/<version>/<triple>/rustup-init.sha256` and the
   `rustup-init` binary next to it with `curl --proto '=https' --tlsv1.2`.
3. Run `sha256sum rustup-init` on each download and check it equals the published `.sha256`
   value. Never take a digest from a mirror or a third party.
4. Update `RUSTUP_VERSION`, `RUSTUP_INIT_SHA256_X86_64`, `RUSTUP_INIT_SHA256_AARCH64` in
   `scripts/install_rustup.sh` and the version quoted above.
5. Run `cargo test -p soos-invariants rustup` and rebuild a sandbox image (for example
   `./run_tests.sh`) so a real download is checked against the new digests.

### Security Design
- **Least privilege**: the workflow token is `contents: read`; `actions/checkout` does not persist
  credentials.
- **Pinned actions**: every third-party action is pinned to a full commit SHA with the release tag
  in a comment. Dependabot (`.github/dependabot.yml`) proposes weekly grouped updates with
  Conventional Commit titles (`ci(deps): ...`, short group name `actions`) and a 7-day cooldown
  before adopting a release. A Dependabot title over 72 characters fails the `PR Title` check and
  must be retitled by hand.
- **Injection-safe**: untrusted event data (PR title, SHAs) reaches shell steps only through `env:`.
- **Reproducible dependencies**: `--locked` on every Cargo command rejects an outdated `Cargo.lock`.
- **Time-bounded**: every job declares `timeout-minutes`.
- **Review freshness**: the layer 2 gate recomputes the diff fingerprint of the PR head and rejects
  a missing, template, stale or copied candid review report (see §5).
- The workflow is validated with `actionlint` and audited with `zizmor` (both clean).

---

## 3. Local Automation Script: `save.sh`

The [`save.sh`](../save.sh) script is the primary tool for both human contributors and AI agents to validate, stage, and commit changes cleanly:
- **Sequential Execution**: Strict fail-fast pipeline (`set -euo pipefail`). Any failure immediately halts execution.
- **CI Parity**: clippy, test and deny use exactly the CI flags (`--locked --workspace --all-targets --all-features`).
- **Conventional Commit Subject**: `-m` (or the positional message) is used as is. Without it, [`scripts/commit_message.sh`](../scripts/commit_message.sh) infers exactly one subject line: the type is the branch prefix (`feat/`, `fix/`, `test/`, `chore/` only), the scope is the crate directory when every staged `crates/<dir>/` file belongs to one crate, and the description is the branch suffix (lowercased, other characters turned into spaces). Any other branch, an empty description or a subject above 72 characters is refused before the pipeline runs, and no body bullets or file counters are generated (GitHub #245). Prefer `-m`: the subject becomes the squash-merge subject on `main`.
- **Staging**: `git add -u` (tracked modifications) plus new files under `crates/`, `tests/`, `Docs/`, `AI/`, `scripts/` and `packaging/`. Other untracked files are listed and never staged; `git add` them explicitly beforehand when they belong in the commit.
- **Push & PR Automation**: When invoked with `--auto-merge` (or via `scripts/pr_loop.sh`), it pushes, opens the Pull Request, watches CI with fail-fast, and squash-merges into `main` with `--match-head-commit`.

Usage:
```bash
# Local verification and commit, subject inferred from the branch
# (on fix/pam-timeout touching only crates/pam: "fix(pam): pam timeout"):
./save.sh

# Local verification and commit with custom message:
./save.sh "feat(policy): implement per-uid rate limiting"

# Full autonomous loop (push, PR creation, CI watch, auto-merge):
./save.sh --auto-merge -m "fix(pam): handle timeout gracefully"
```

---

## 4. Dependency & Supply Chain Auditing: `cargo-deny`

The [`deny.toml`](../deny.toml) configuration enforces strict third-party dependency rules in alignment with our threat model and licensing requirements:

### Enforced Rules:
- **Advisories**: Any known unpatched RustSec advisory immediately fails the build (checked on every PR and daily).
- **Licenses**: Only permissive open-source licenses are permitted for dependencies (MIT, Apache-2.0, BSD-2/3, ISC, ...). Workspace internal crates under AGPL-3.0-or-later (root `LICENSE`) are isolated with `publish.workspace = true` and `[licenses.private] ignore = true`.
- **Sources**: Only the official `crates.io` index is permitted (no unverified git repositories or alternate registries).
- **Duplicates**: `multiple-versions = "deny"`; every tolerated duplicate is listed in `skip` with a reason that names the direct dependents of the skipped version, as printed by `cargo tree --locked -i <crate>@<version> --target all -e normal,build,dev --depth 1`. A version used directly by a workspace crate is never described as transitive, and every version of a crate locked three times is documented in `deny.toml` (enforced by `tests/invariants/src/dependency_tooling_contract.rs`, GitHub #243). Rewrite the reason whenever `cargo update` changes who pulls the old version in.
- **Pre-release pins**: a pre-release workspace dependency is pinned with `=` (today `ort = "=2.0.0-rc.13"`), so a `cargo update` never adopts the API changes of the next release candidate silently; upgrade it deliberately and rerun the real-model tests. Workspace crates use `getrandom` 0.4 (0.2 and 0.3 remain only through `rand_core` and `ahash`).
- **Cargo updates**: Dependabot covers GitHub Actions only; crate updates go through the normal dev-workflow (see `.github/dependabot.yml`).
- **Banned Crates**: `opencv` and `nokhwa` (see [`AI/ARCHITECTURE.md`](../AI/ARCHITECTURE.md)).

To execute the security audit locally (cargo-deny ≥ 0.20):
```bash
cargo deny --locked check
```

---

## 5. Dual-Layer Candid Review Gate

| Layer | Script | Enforced in |
|---|---|---|
| 1 — deterministic invariants | `scripts/candid_review.sh` | pre-commit, `save.sh`, CI `lint` |
| 2 — AI reviewer report | `scripts/candid_subagent.sh` | `save.sh`, pre-push, CI `lint` (pull requests) |

Layer 1 checks the diff for `unsafe` in the nine business crates, removal of
`#![forbid(unsafe_code)]`, panics/prints in PAM production code, async runtimes in `crates/pam`,
banned crates, shell syntax and the English-only policy.

Layer 1 lexing rules (GitHub #242, walkthrough 129):

- Every audit diffs against `git merge-base origin/main HEAD` (plus the working tree), never
  against the moving tip of `origin/main`, so commits merged after the branch point are not
  audited as reversed branch changes.
- The PAM panic and print audits compare the *production code* of each changed
  `crates/pam/src` file at the merge base and in the working tree. An awk character lexer,
  whose state carries across lines, removes line comments and nested block comments and blanks
  the contents of string, raw strings (`r"..."`, `r#"..."#`, `br#"..."#`) and char literals,
  then drops exactly the item gated by `#[cfg(test)]` (up to its `;` or its matching `}`);
  braces inside comments or literals never move that boundary, and a panic written inside a
  comment or literal is not reported (GitHub #285). There is no `grep -v tests`, so a
  production line that merely contains the substring `tests` is still audited.
- `pour` and `attention` are English words and are not French markers.
- The static invariants add the same hardened extractor (`tests/invariants/src/lexing_contract.rs`),
  the full PAM panic-construct check, and a resolved-graph check that no async runtime is a
  normal dependency of `soos-pam` (`cargo tree -e normal --target all --all-features`),
  transitive dependencies included.

Layer 2 binds the review to the exact code reviewed:

```bash
./scripts/candid_subagent.sh --prepare     # writes target/candid_diff.patch, prints the fingerprint
# reviewer writes AI/candid_review_report.md including:
#   - **Reviewed-Diff-Fingerprint**: `<sha256>`
#   **VERDICT: APPROVED**
./scripts/candid_subagent.sh               # gate on the working tree
./scripts/candid_subagent.sh --rev HEAD --skip-layer1   # gate on a commit (pre-push / CI)
```

The fingerprint is the SHA-256 of the diff between the merge-base with `origin/main` and the
reviewed tree (untracked files included). The two review-report singletons are excluded:
the report itself and `AI/plan_evaluator_report.md`, the per-issue plan evaluation that every
issue rewrites (GitHub #267), so rewriting either report never changes the reviewed diff. It is
identical before and after committing, and any later change to the code invalidates it. Both
reports are overwritten per branch: on `main` they record the last merged pull request, not an
open one. A missing report, a report
without `VERDICT: APPROVED`, a `CHANGES_REQUESTED` report, or a report written for another diff
fails the gate. There is no bot exemption (actor checks are spoofable): Dependabot pull requests
are reviewed like any other and receive their report on the Dependabot branch.

---

## 6. Dockerized PAM Integration Testing: `run_tests.sh`

To guarantee that experimental PAM modules never compromise the host operating system, all PAM integration tests run inside an isolated, ephemeral Ubuntu 24.04 Docker container:
```bash
./run_tests.sh             # Ubuntu sandbox, T1–T15
./run_tests.sh --matrix    # Ubuntu, Fedora and Arch Linux
./run_tests.sh authselect  # Fedora authselect profile activation and rollback (fedora:40)
./run_tests.sh rollback    # Debian stack order + byte-for-byte PAM rollback (ubuntu:24.04, fedora:40)
```

Before building, `tests/docker/test_suite.sh` runs `rustup toolchain install --profile minimal`
in `/workspace`, which reads `rust-toolchain.toml` and installs the pinned release (`1.98.1`)
that the other CI jobs install. The sandbox images already install that release as their
default toolchain (`--default-toolchain 1.98.1` in `Dockerfile` and
`tests/docker/Dockerfile.{ubuntu,fedora,arch}`), so the sync is normally a no-op; it still
protects a layer-cached image built before a toolchain bump. CI passes `SOOS_REQUIRE_TOOLCHAIN_SYNC=1`, so a failed sync fails the job;
a local offline run warns and uses the image toolchain. `rustc --version` is logged. The suite
then always runs `cargo build --locked --release -p soos-pam` (a no-op when up to date), so a
stale `target/release/libpam_soos.so` from the bind-mounted host checkout is never deployed
(GitHub #244). The package and distribution harnesses (`tests/docker/test_packages.sh`,
`tests/distro/{debian_ubuntu,fedora_rhel,arch_linux}_test.sh`) likewise always run
`cargo build --locked --release --workspace` before packaging, unless the caller passes
`--skip-build` on purpose; an existing release binary is never a reason to skip the build.

The matrix (see [`PAM_DOCKER_TEST_MATRIX.md`](PAM_DOCKER_TEST_MATRIX.md)) covers nominal facial
authorization, daemon timeout and crash fallbacks with valid and invalid passwords, native
distribution stack integration, an absent socket, a missing module, `Verdict::Deny`, a
truncated response and malformed responses (undecodable verdict, mismatched `request_id`,
unsupported version, oversized and empty frames). Every case is a hard failure; the timeout
cases read `timeout_ms` from the PAM stack under test and assert the elapsed time
(`tests/docker/pam_case_lib.sh`, GitHub #189).

---

## 7. Secret Scanning and Git Guardrails

[`scripts/secret_scan.sh`](../scripts/secret_scan.sh) is shared by the pre-commit hook
(`--staged`), the pre-push hook (`--unpushed <sha>`: every commit not yet on any remote) and CI
(`--range A..B`). Range modes scan **each commit individually**, so a secret added in one commit
and deleted in a later one is still caught. An unresolvable range fails closed (exit 2). It
inspects only added lines and added/modified file names (case-insensitive), and reports the file
and pattern name without echoing the secret:
- PEM private keys; GitHub, AWS, Slack and generic `sk-` API tokens.
- Key and certificate files (`*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.jks`, SSH keys, `.env`).
- Biometric and evidence artifacts (`*.enc`, `master.key`, `evidence.key`).
- ONNX model weights (`*.onnx`, `*.ort`), which are downloaded and attested instead.

Additional guardrails:
- **Branch protection (local)**: pre-commit refuses commits on `main`; pre-push refuses pushes to `main`.
- **Commit format**: `.githooks/commit-msg` enforces Conventional Commits 1.0.0 locally, and the
  `PR Title` workflow applies the same hook to the PR title (the squash commit subject), rejecting
  git-generated subjects (`Merge ...`, `Revert "..."`).
- **Recommended repository settings** (not enforceable from the repository itself): require the
  `CI Success` and `PR Title` checks on `main` (branch protection or a ruleset) and add a CODEOWNERS
  entry for `scripts/`, `.githooks/` and `.github/`. Until then, a pull request can modify the gate
  scripts it is checked by; `pr_loop.sh` never uses `--admin`, so enabling protection is sufficient.
