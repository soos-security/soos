# Walkthrough 213 — `scripts/setup.sh` User Phase: Phone Companion Profile

- **Date**: 2026-10-15
- **Issue**: GitHub-only #400 (PR 2 of #362), no backlog id — **Branch**: `feat/setup-user-phase` (not registered in `scripts/sync_issue.py`; the PR carries `Closes #400` and `Refs #362`)
- **Matrix criteria**: SET20–SET38 (verified), SET39 (owner hardware, pending); SET3 text migrated

## 1. Context & Objectives

PR 1 (#372, walkthrough 200) made `scripts/setup.sh` install the face profile and deferred the user phase (ADR 2026-10-13 item (9)). The owner wants the one command to set up everything at once (decision of 2026-10-12), and on 2026-10-15 decided, on #400: no VPN on the phone, so Funnel, remote unlock, the camera view and the camera view over Funnel are written on by default together with password alerts, push and offline alerts; each can be turned off later in soos-gui; an explicit `false` already in `remote.toml` is kept unless its `--<feature> on` option is given; the Tailscale login and node name come from `--tailscale-login` / `--rp-id` or a prompt, never from `tailscale status`. The PR 1 scope contract SET3 banned the words this phase needs; the owner approved its migration, applied by the lead in commit 40b0fb1 before this work started.

## 2. Architect Design

Spec `AI/architect_spec_setup_user_phase.md` (§1–§9, Revision 1 §10, Revision 2 §11):

- **Where it runs**: after the root half returned 0 (or 3), in the user half, as the session owner. Skipped with the reason (root start, another `--user`, no `XDG_RUNTIME_DIR`, relative `XDG_CONFIG_HOME`, symbolic link, foreign or writable `~/.config/soos`, no identity); any companion option in such a case is exit 2 before any change.
- **`remote.toml`**: absent → created; the untouched `install_remote.sh` template → replaced; a file carrying setup's marker line and strict grammar → rewritten only when its render differs; any other file → never edited. 0600, `mktemp` + `mv`, 0600 backup `remote.toml.soos-setup.bak`.
- **Features**: seven boolean keys in prerequisite order; option, else kept `false` of a managed file, else on; a key whose prerequisite is off is off; an `on` whose prerequisite is off is exit 2.
- **Install, validate, start**: `scripts/install_remote.sh` every run (its `-p` build differs from the workspace build, so the restart decision rests on checksums of the installed files before and after); `soos-remote --config <file> passkeys list` validates the whole file (78/1 refuse); `systemctl --user enable --now` only for a configuration written for the first time (and the sender when push is switched on); both units active after a 2 s settle (`Type=exec`).
- **Camera view, daemon side**: the root half receives only `--camera-view on`, creates `daemon.toml` with only the `[preview]` section when the file is absent (0644, hard link), restarts the daemon and waits; an existing file is never edited.
- **Tailscale**: still only `tailscale version` runs; `tailscale funnel` / `serve`, the Funnel node attribute, the 1.102.4 minimum, the passkey enrollment, notifications and the resolver drop-in are printed.
- **Exit 5**: companion failed, its changes rolled back, face install kept.
- **`install_remote.sh`**: copies from `${CARGO_TARGET_DIR:-<checkout>/target}/release`.

ADR: "[2026-10-15] Setup User Phase: Phone Companion Profile With Default-On Features", which amends items (7)–(9) of the #372 ADR and records the SET3 migration.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md`, three rounds. Round 1 REVISION_REQUIRED with three MAJOR findings: F1 the install decision compared workspace artifacts that `install_remote.sh`'s own `-p` build never produces (every real run would reinstall and restart); F2 a re-run would re-enable a unit the owner disabled in the GUI, bringing an internet-reachable service back; F3 the spec's own wording would break SET2/SET3 (unquoted `tailscale`, `curl`, package words). Fourteen MINOR findings (settle after `Type=exec` starts, `passkeys list` exit 1, journal before `enable`, exit codes, stale texts, sticky cascaded `false`, uid source, `daemon.toml` survival, resolver stub hosts, writable directory, a parser-level render test, draft tests, AS-U1, exit 70). Round 2 REVISION_REQUIRED with R2-F1: the sender stayed off after `--push on` on an existing setup. Round 3 APPROVED (one documentation MINOR, fixed in §3.0).

## 4. Tester Contract

`AI/tester_contract_setup_user_phase.md`. New file `tests/invariants/src/setup_user_phase_contract.rs` with its own harness, so `setup_contract.rs` is not touched beyond the migrated SET3: fake root, fake HOME, an `XDG_RUNTIME_DIR` owned by the test uid, stub `systemctl` with per-unit `--user` state, stub `install_remote.sh` with the real contract (refuses root, copies from `$CARGO_TARGET_DIR/release`, writes the template only when absent), fake `soos-remote` enforcing the prerequisites with exit 78. 22 behavioral and static tests (`test_set_u1` … `test_set_u22`), plus `crates/remote/tests/setup_render_tests.rs` parsing the render fixture `tests/fixtures/setup/remote.toml` with the real `parse_config` (every cascade variant accepted, the variants setup never writes refused). First Red run: 0 passed, 18 failed. Migrated test: only SET3 (owner-approved, 40b0fb1).

## 5. Auditor Constraints

`AI/auditor_constraints_setup_user_phase.md` U-1 … U-15, CLEARED: never root for the companion; only `--camera-view on` crosses `sudo`; only `tailscale version` runs; no login, code or key printed; 0600 / 0700 atomic writes with symbolic links refused; user files never edited; strict login and node-name character sets before TOML; no offline alerts without push; explicit `false` and disabled units kept; `daemon.toml` created only when absent via a hard link and rolled back; every call bounded; journaled companion rollback with exit 5; no `eval`, package word or `$UID`; `install_remote.sh` only changes its copy source; group and linger only printed.

## 6. Implementation

- `scripts/setup.sh`: options `--no-remote`, `--tailscale-login`, `--rp-id`, `--funnel`, `--remote-unlock`, `--password-alerts`, `--push`, `--offline-alerts`, `--camera-view`, `--camera-view-funnel`; constants (`EXIT_COMPANION`, `MIN_TAILSCALE_VERSION`, bounds, `COMPANION_SETTLE_S`); `resolve_features` (usage time without the file, preflight time with it), `classify_remote_config`, `resolve_companion` (read-only, prompts only when interactive), plan lines, `phase_preview` in the root half, `run_companion_phase` with its journal and traps, `print_companion_guide`. The PR 1 texts `[DETECT] tailscale … (read only; setup never runs serve or funnel)` and `[PLAN] later … PAM activation` were updated.
- `scripts/install_remote.sh`: `TARGET_DIR` from `CARGO_TARGET_DIR`.
- Docs: `Docs/PACKAGING_AND_PROVISIONING.md` §3.0 ("Phone companion"), `Docs/REMOTE_COMPANION.md` §4 and §2d, `README.md` §1.
- A bug found by the contract during the implementation: the validators run their own `=~`, which reset `BASH_REMATCH` under `set -u`; the parsed value is now copied before validation.

## 7. Candid Review

Two earlier fresh reviews returned APPROVED with MINOR findings, all fixed before the final review: the companion skip reason is repeated after the run, the exit-5 message names a kept `daemon.toml`, other TOML headers end the `[preview]` scan, a unit the run started is stopped by the rollback (`test_set_u22_rollback_stops_a_unit_started_by_the_run`), exit 78 (configuration) and 1 (passkey store) are reported apart, `~/.config` must be owned by the user, an idle sender left enabled after push was switched off is reported with its disable command, a unit the run restarted is restarted again on the restored file, a configuration directory the failed run created is removed, and an unreadable `remote.toml` is refused before any change. The final review is run by a fresh sub-agent on the final diff (`./scripts/candid_subagent.sh --prepare`); the fingerprint-bound verdict is `AI/candid_review_report.md`, and the branch is pushed only with an APPROVED report that matches the pushed diff (pre-push hook and CI re-check it).

## 8. Verification Results

```bash
cargo test --locked -p soos-invariants --all-features setup_ -- --test-threads=3   # 50 passed (28 PR 1 + 22 user phase)
cargo test --locked -p soos-remote --all-features --test setup_render_tests       # 3 passed
```

The full gates (fmt, clippy `--all-targets --all-features`, workspace tests, cargo-deny, candid layers 1 and 2) run through `./save.sh --push-pr`; CI results are recorded on the PR.

## 9. Known Limitations / Follow-ups

- Owner to confirm: AS-U1 a root start skips the companion; AS-U2 an existing `daemon.toml` is never edited (until the TOML editor and `soos-daemon --check-config`); AS-U3 a user-written `remote.toml` is never edited; AS-U4 exit 5; AS-U5 a unit that is not enabled on a re-run stays disabled (also after `install_remote.sh --uninstall`); the journal group for password alerts is reported, never added; `loginctl enable-linger` is printed, never run.
- A key forced off by a prerequisite is written `false` and then kept like a GUI choice until its own `on`.
- A setup-created `daemon.toml` survives `--camera-view off` and `uninstall.sh`.
- On hosts whose `/etc/resolv.conf` points into `/run`, push needs the resolver drop-in (printed, never added).
- The first companion run compiles a second variant of the companion crates (memory peak).
- SET39: the owner's real run on the Arch host (Funnel step, passkey, test push, second run without restart).
- Still deferred in #362: PAM activation, TOML editor and `--check-config`, `--uninstall`, `--profile`, Docker matrix, `--from-release`.
