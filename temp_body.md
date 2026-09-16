> **Branch**: `fix/cli-security`  
> **Architecture ref**: §11 Installation & CLI

#### Problem Statement

`enrollment-cli` contains a hidden `--skip-root-check` flag bypassing security, omits root checks for `verify` and `list`, and passes unvalidated `PathBuf` arguments risking traversal.

#### Sub-issues

- [ ] **#35.1** — Remove `--skip-root-check` and enforce `check_privileges` on all subcommands
- [ ] **#35.2** — Validate `PathBuf` arguments against FHS paths or sanitize them
- [ ] **#35.3** — Fix systemd `StateDirectory` and correct `Group=soos` ownership in `soos-daemon.service`

---