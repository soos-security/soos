# Audit Constraints — GitHub #423: companion off-switch and opt-in wording

Inputs: `AI/architect_spec_companion_wording.md`, the new contract
`tests/invariants/src/companion_wording_contract.rs` (CWD1–CWD5, red on `74782b8` for CWD1, CWD3
and CWD4), `scripts/setup.sh`, `README.md`, `Docs/REMOTE_COMPANION.md`, `AI/ARCHITECTURE.md`.

| # | Constraint | Applies to | Verified by |
|---|---|---|---|
| C1 | Text only: no shell logic, variable, option, exit code or control-flow change in `setup.sh`; only the `usage()` heredoc paragraph and the closing `info` string change | `scripts/setup.sh` | `git diff`, SET1–SET39 |
| C2 | Every option line of the help is unchanged (`--no-remote` … `--camera-view-funnel`, exit codes) | `usage()` | SET21 |
| C3 | No new command word in `setup.sh`: no lowercase `tailscale `, `usermod`, package name, `pam-auth-update`, `/pam.d`, `curl `, `wget `, `chmod`, `pkexec ` | `scripts/setup.sh` | SET2, SET3 |
| C4 | The `remote.toml` marker comment printed by setup is byte-identical | `setup.sh` ~971 | `setup_user_phase_contract` (line 811) |
| C5 | The new help text and summary use no `$`, backtick or `$(…)` inside the unquoted heredoc (no expansion at `--help` time) and the summary string stays inside double quotes with no unescaped `"` | `usage()`, closing `info` | review, SET21 |
| C6 | Every claim is true on `74782b8`: switches exist only for `allow_unlock`, `allow_funnel`, `push_notifications`, `camera_view`, `offline_alerts`; `--password-alerts off` cascades push and offline alerts off; the code default of `allow_funnel` is `false` (`403`) | help, docs | CWD2, `resolve_features`, `config.rs` |
| C7 | README: no unit claims a key can be turned off in `remote.toml` on its own; no "opt-in"/"off by default" for remote unlock or the camera view; the security needles stay | `README.md` | RDM10, RDM12 |
| C8 | `ARCHITECTURE.md` Unlock row keeps "default `false`"; no other cell text changes | `AI/ARCHITECTURE.md` | CWD3, `git diff` |
| C9 | No test assertion of `origin/main` is modified, weakened or deleted; only a new module and its `mod` line are added | `tests/invariants` | `git diff origin/main -- tests/` |
| C10 | No password, identity or secret appears in any new text (wording only) | all | review |

### Pre-existing violations found (not introduced by this change)

None. Note (out of scope by request): `AI/ARCHITECTURE.md` §13 prose still says "Funnel (opt-in …)"
and "Live camera view (opt-in …)" at lines 370 and 375 of the base; they describe the code default.
