# Auditor Constraints — One-Command Setup PR A (`feat/setup-preflight-deps`)

- **Date**: 2026-10-23
- **Issue**: GitHub #362, owner decisions of 2026-10-23 (spec `AI/architect_spec_one_command_setup.md`
  §5, §6, §19, §20; plan evaluation round 3 APPROVED)
- **Contract**: `tests/invariants/src/setup_deps_contract.rs` (SDP1–SDP24),
  `crates/remote/tests/setup_render_tests.rs::test_sdp25_android_render_is_accepted`
- **Verdict**: CLEARED for implementation under the constraints below.

The change is shell only (`scripts/setup.sh`, `scripts/check_build_deps.sh`, `scripts/install.sh`,
`scripts/uninstall.sh`, a new Docker test). No crate production code, no PAM path, no new network
call in any script that setup runs as root.

| # | Constraint | Checked by |
|---|---|---|
| C1 | The backup of a user `remote.toml` never follows a link: the candidate name is taken when it exists **or is a link** (dangling included), the file is created under `umask 077` with `set -C` (`O_EXCL`), then `chmod 0600`, then compared with `cmp -s` before the managed file replaces the original. No `cp` (which follows links), never deleted by setup, at most `MAX_BACKUP_SUFFIX` suffixes then exit 5 with nothing replaced. | SDP5, SDP9, SDP11, SDP12 |
| C2 | Bounded reads only: the user file is read through `head -c MAX_REMOTE_CONFIG_BYTES` (a larger file is refused in preflight before any change when a backup would be needed). No `source`, no `eval`, no TOML library; regular expressions on single lines only. | SDP13, SET20/SCN checks unchanged |
| C3 | No value of the user file is ever printed or logged: plan and result lines name keys only, names are reduced to `[A-Za-z0-9_.-]` and capped (`MAX_REPORTED_KEYS`); the Tailscale login (flag or file) is never printed and never passed to the root half. | SDP9, SDP10, SDP14, SET23/SET24 unchanged |
| C4 | Fail-safe booleans: a user-file key whose value the tolerant rule does not recognize is `false`; a key below any table header is ignored; an explicit `false` is kept unless its own `--<feature> on` flag is given. Funnel is never turned on against a written choice. | SDP14, M1 |
| C5 | `[android]` is carried over only when it passes the strict grammar (exact `[android]` header as the last table, `package = "org.soos.remote"`, 1–2 distinct upper-case colon fingerprints, nothing after it); a binding while Funnel resolves off is refused in preflight (exit 2, nothing changed). | SDP15–SDP18, SDP23, SDP25 |
| C6 | Tool preflight is read-only (`command -v` only, no tool executed to probe it). Base and family scopes fail before any change (exit 2); requested group tools are checked after the package step and before the build; only the `companion` group is non-fatal (R1-3); the toolchain is checked after `install_rustup.sh` (exit 1, packages kept, nothing installed). | SDP4, SDP7, SDP8 |
| C7 | Package names live only in `check_build_deps.sh`; `setup.sh` names a tool only as a `REQUIRED_TOOLS` key or a command word. Every external command word of `setup.sh` has a `REQUIRED_TOOLS` entry (lexer check). | SET2 unchanged, SDP2, SDP3 |
| C8 | `install.sh` and `uninstall.sh` delete `soos-push-sender@.service.d/prefix.conf` only when it is a regular file (not a link) whose first line carries the install.sh marker; an administrator file of that name is kept with a warning. | SDP19, PSS19 unchanged |
| C9 | `system_up_to_date` stays read-only: the drop-in is read only when it is a regular file and not a link. | SDP20 |
| C10 | The Docker package matrix mounts the workspace read-only, uses digest-pinned images, runs no privileged container and writes nothing on the host. | SDP22 |
| C11 | No new `curl`, `wget`, Tailscale command or network access in `setup.sh` (S-1); `set -euo pipefail`, `LC_ALL=C` kept. | SET3, SET41, SCN2/SCN3 unchanged |
