# soos — Zero-Trust Local Facial Verification PAM Module for Linux

**soos** is an enterprise-grade Linux PAM (Pluggable Authentication Module) and privileged daemon designed for secure, local facial verification. It eliminates brittle camera access inside sensitive PAM processes by enforcing strict privilege separation, bounded binary IPC, and multi-stage anti-spoofing (PAD).

---

## Quick Start

### 1. Install from Source

Supported hosts: Debian 12, Ubuntu 24.04, Fedora 40 / RHEL 9 and Arch Linux (x86_64, aarch64), with systemd.

```bash
# 1. Build dependencies (per-distribution lists: Docs/PACKAGING_AND_PROVISIONING.md §3.1)
./scripts/check_build_deps.sh --print-packages build     # e.g. then: sudo apt-get install ...
./scripts/check_build_deps.sh                            # read-only preflight
#    Rust toolchain: https://rustup.rs (rust-toolchain.toml selects the channel)

# 2. Build release artifacts and install (fails closed, rolls back on any error)
sudo ./scripts/install.sh --build                         # runs the preflight and the release build first
#    or, with artifacts already built by 'cargo build --release --locked --workspace':
sudo ./scripts/install.sh
```

`scripts/install.sh --dry-run` runs the same read-only preflight without touching the system. The installer
refuses debug-profile artifacts, exits non-zero when an artifact is missing, deploys and verifies the attested
models (`scripts/download_models.sh`, needs `curl`, no Python) before enabling `soos-daemon.service`, and undoes
every change it made if any step fails. It installs only the PAM template of the detected distribution
(`--distro` overrides). `soos-daemon.service` does not start before the models manifest is deployed;
`sudo ./scripts/install.sh --start` (or `sudo ./scripts/wait_daemon_ready.sh` after `systemctl start soos-daemon`)
waits at most 30 s for the daemon and prints its JSON status. PAM activation stays a separate, explicit step
(see [`Docs/DISTRIBUTION_DEPLOYMENT.md`](Docs/DISTRIBUTION_DEPLOYMENT.md)).

### 2. Start the Daemon and Enroll

```bash
sudo systemctl start soos-daemon                 # install.sh enables the unit but does not start it
sudo soos-admin add-user alice                   # adds alice to the 'soos' group (socket access)
sudo soos-enroll enroll --username alice         # captures and encrypts the template
sudo soos-enroll list                            # templates are stored as /var/lib/soos/biometrics/<uid>.cbor.enc
```

### 3. Verify Before Activating PAM

```bash
soos-admin status                                # daemon, socket and unit health
soos-admin test-pam                              # simulated authentication round trip and latency
```

### 4. Activate PAM (Explicit Step)

PAM is never modified by `install.sh`. Activate the profile for your distribution, keeping a root
shell open until a password login has been tested:

```bash
sudo pam-auth-update --package --enable soos soos-notify      # Debian / Ubuntu
sudo authselect select custom/soos with-faillock --force      # Fedora / RHEL
```

Arch Linux, GDM (`soos-admin gdm enable`) and the full per-distribution procedure are described in
[`Docs/DISTRIBUTION_DEPLOYMENT.md`](Docs/DISTRIBUTION_DEPLOYMENT.md). The password prompt always
remains available: when the daemon is stopped or the face does not match, `pam_soos.so` returns
`PAM_IGNORE` and the stack falls through to `pam_unix.so`.

### 5. Rescue and Uninstall

```bash
sudo touch /etc/soos/disabled                    # immediate kill switch: pam_soos.so returns PAM_IGNORE
sudo touch /etc/soos/gdm.disable                 # disable facial login for GDM only
sudo ./scripts/uninstall.sh --keep-data          # restores the PAM stack, keeps templates and master key
sudo ./scripts/uninstall.sh --purge-data         # also erases templates, master key and evidence
```

---

## Technical Documentation

Detailed guides and specifications are maintained in the [`Docs/`](Docs/) directory:

| Document | Description |
|---|---|
| [**IPC Protocol Specification**](Docs/IPC_PROTOCOL.md) | Bounded binary framing (Postcard over Unix domain socket), type definitions, and security bounds. |
| [**CI/CD & Security Auditing**](Docs/CI_CD_AND_SECURITY.md) | Quality gates, `cargo-deny` dependency audits, and Docker PAM sandbox testing. |
| [**Development Workflow & Branching**](Docs/DEVELOPMENT_WORKFLOW.md) | Topic branch rules, multi-agent TDD cycle, PR loop, and automated review. |
| [**Conventional Commits Specification**](Docs/COMMIT_CONVENTION.md) | Standardized commit message format and git hook enforcement rules. |

---

## Architecture & Security Reference

- [`AI/ARCHITECTURE.md`](AI/ARCHITECTURE.md): Threat model, latency budgets, privilege boundaries, and security invariants.
- [`AI/DECISIONS.md`](AI/DECISIONS.md): Architectural Decision Records (ADRs).
- [`AI/MOCK_STRATEGY.md`](AI/MOCK_STRATEGY.md): Hardware mocking and camera-free testing strategy.
- [`AI/ROLES_AND_WORKFLOW.md`](AI/ROLES_AND_WORKFLOW.md): Multi-agent operational rules and quality assurance.
- [`AI/VERIFICATION_MATRIX.md`](AI/VERIFICATION_MATRIX.md): Traceable component acceptance matrix.
- [`AI/walkthroughs/`](AI/walkthroughs/): Step-by-step verifiable implementation walkthroughs.
