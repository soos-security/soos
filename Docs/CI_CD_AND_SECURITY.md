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
  so `target/` is never uploaded to the Docker daemon.

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
- **Conventional Commits Generation**: Generates standard Conventional Commit headers automatically in English based on the staged files (`feat(...)`, `test:`, `docs:`, `chore:`).
- **Push & PR Automation**: When invoked with `--auto-merge` (or via `scripts/pr_loop.sh`), it pushes, opens the Pull Request, watches CI with fail-fast, and squash-merges into `main` with `--match-head-commit`.

Usage:
```bash
# Local verification and commit with auto-generated message:
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
- **Licenses**: Only permissive open-source licenses are permitted for dependencies (MIT, Apache-2.0, BSD-2/3, ISC, ...). Workspace internal crates under AGPL-3.0 are isolated with `publish.workspace = true` and `[licenses.private] ignore = true`.
- **Sources**: Only the official `crates.io` index is permitted (no unverified git repositories or alternate registries).
- **Duplicates**: `multiple-versions = "deny"`; every tolerated duplicate is listed in `skip` with a reason.
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
  `crates/pam/src` file at the merge base and in the working tree. An awk filter blanks
  comments and literal contents and drops exactly the item gated by `#[cfg(test)]` (up to its
  `;` or its matching `}`); there is no `grep -v tests`, so a production line that merely
  contains the substring `tests` is still audited.
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
reviewed tree (untracked files included, the report itself excluded). It is identical before and
after committing, and any later change to the code invalidates it. A missing report, a report
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
