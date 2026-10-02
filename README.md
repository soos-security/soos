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
./scripts/install_rustup.sh                              # verified rustup-init, installs the pinned Rust 1.98.1
export PATH="$HOME/.cargo/bin:$PATH"                     # printed by the script; shell profiles are never edited

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

Once a user is enrolled and the daemon runs, **presence auto-unlock** is active by default, even
before PAM is activated: when the screen of that user's local session is locked (GNOME, KDE
Plasma; wlroots lockers need the `swayidle` hooks), the daemon scans for the owner's face after a
3 s grace and unlocks the session through systemd-logind, without any keypress. It respects
`pam_faillock` and account expiry, and stops while the lid is closed or, when the driver
reports it, while the screen is off (screen-off detection is best effort). Turn it
off with `sudo touch /etc/soos/presence.disable` or `[presence] enabled = false` in
`/etc/soos/daemon.toml` (see [`Docs/DISTRIBUTION_DEPLOYMENT.md`](Docs/DISTRIBUTION_DEPLOYMENT.md)
§5.5 and [`Docs/DAEMON.md`](Docs/DAEMON.md) §6).

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
sudo touch /etc/soos/disabled                    # immediate kill switch: pam_soos.so returns PAM_IGNORE, presence auto-unlock stops
sudo touch /etc/soos/presence.disable            # stop presence auto-unlock only (face PAM unchanged)
sudo touch /etc/soos/gdm.disable                 # disable facial login for GDM only (presence auto-unlock keeps running)
sudo ./scripts/uninstall.sh --keep-data          # restores the PAM stack, keeps templates and master key
sudo ./scripts/uninstall.sh --purge-data         # also erases templates, master key and evidence
```

---

## Upgrade

Pull, rebuild and reinstall with the method used for the first install. The master key, the enrolled
templates, `/etc/soos/daemon.toml` and the PAM activation state are kept (details, and how to switch
from `install.sh` to a package: [`Docs/PACKAGING_AND_PROVISIONING.md`](Docs/PACKAGING_AND_PROVISIONING.md) §9).

```bash
git pull --ff-only

# Debian / Ubuntu (.deb; dpkg -i also reinstalls a rebuilt package of the same version)
./scripts/build_deb.sh
sudo dpkg -i target/packages/soos_<version>_<arch>.deb
sudo systemctl restart soos-daemon

# Arch Linux
./scripts/build_arch.sh
sudo pacman -U target/packages/soos-<version>-<release>-<arch>.pkg.tar.zst
sudo systemctl restart soos-daemon

# Fedora / RHEL (the package restarts a running daemon itself)
./scripts/build_rpm.sh
sudo dnf upgrade ./target/packages/soos-<version>-<release>.<arch>.rpm    # same version: sudo dnf reinstall ./target/packages/...

# scripts/install.sh (restarts a running daemon on the new binaries, then waits for it)
sudo ./scripts/install.sh --build --start

# Verify
soos-admin status
sudo soos-enroll list                            # a "[WARN] UID N: ... re-enroll" line marks a foreign template
sudo soos-enroll enroll --username alice         # only for a user reported as foreign above
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

---

## License

soos is free software licensed under the **GNU Affero General Public License v3.0 or later**
(SPDX `AGPL-3.0-or-later`); the full text is in [`LICENSE`](LICENSE). The `license` key of
`[workspace.package]` in the root `Cargo.toml` is the single source of truth: every crate inherits
it (`license.workspace = true`) and the RPM spec and Arch PKGBUILD declare the same expression.
Third-party dependencies are restricted to permissive licenses by `deny.toml`.
