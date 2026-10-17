# Plan Evaluation Report
- **Date**: 2026-10-15
- **Issue**: GitHub-only #400 — `scripts/setup.sh` user phase (phone companion profile, PR 2 of #362); no `AI/BACKLOG.md` id
- **Branch**: `feat/setup-user-phase`
- **Base commit**: `40b0fb1` (= `origin/main` 2025738 + the owner-approved SET3 contract migration)
- **Plan evaluated**: `AI/architect_spec_setup_user_phase.md`

## 1. Coverage Matrix

Acceptance lines of GitHub #400, as amended by the owner comment of 2026-10-15.

| Acceptance line / owner decision | Spec element | Status |
|---|---|---|
| SET3 migration recorded in `AI/DECISIONS.md` (amendment of the #372 ADR) | §9 new ADR + pointer; OD-U1 | Covered |
| Default run with `--tailscale-login` + `--rp-id` installs and enables the companion with the default-on keys | §4.0 steps 7–9, §4.4, §5 render, §6 | Covered, but see F1 (install decision) and F2 (re-enabling a disabled unit) |
| `offline_alerts = true` only with `push_notifications = true` | §2 prerequisites, §4.0 step 8, §7 | Covered |
| All seven keys default `true` (owner 2026-10-15, overrides the issue body's "allow_unlock / camera_view_funnel off") | OD-U2, §5, §6, §9 ADR item (7) amendment | Covered |
| Explicit `false` kept unless `--<feature> on` | §4.0 step 8, §7 | Covered for keys; **not** for the unit switch of the GUI (F2) |
| Login / node name from flags or prompt, never `tailscale status` | OD-U3, §4.0 step 7 | Covered |
| Without login or node name nothing is enabled, exact steps printed, exit 0 | §4.0 step 7 (skip), §4.5 | Covered (the companion is not even installed; stricter than the issue body, acceptable) |
| User phase never as root; root half never writes under the user's home | AS-U1, §4.0 step 2, §7 | Covered (AS-U1 deviates from the issue body's `runuser` proposal: owner to confirm, listed) |
| `remote.toml` 0600, atomic, symlinks refused, user-managed untouched | §4.0 steps 5–6, §4.4 step 2, §7 | Covered (directory-mode claim inaccurate, F-m10) |
| Only `tailscale version` executed; serve/funnel printed only | §4.1 `tailscale` line, §4.5, §7 | Covered behaviourally; the static SET3 rule is easy to break with the spec's own wording (F3) |
| Inconsistent flags exit 2 before change | §2 usage / preflight errors | Covered |
| `--dry-run` writes nothing | §4.0 (read-only, no prompt in dry run), §4.1 | Covered |
| A second run changes nothing | §4.0 step 9 (`already done`), step 11 (`cmp`) | **At risk on real hardware** (F1: the `-p` vs `--workspace` build artifacts differ, so the companion is reinstalled and restarted on every run) |
| Journal group reported, never added; `enable-linger` printed, never run (OD-U4) | §4.5 | Covered |
| `install_remote.sh` `CARGO_TARGET_DIR` fix (OD-U5) | §1 table | Covered |
| Docs (§3.0, REMOTE_COMPANION §4), matrix rows, walkthrough 213 | §1 table, §9 | Covered (213 is the next free number; verified) |
| `--help` documents every new option and exit 5 (SET5 scanner) | §2 lists the options, §3 `EXIT_COMPANION` | Implicit only: the spec never says `usage()` gets them (F-m5) |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `MAX_REMOTE_CONFIG_BYTES` = `soos_remote::MAX_CONFIG_BYTES` | `crates/remote/src/lib.rs:77`; GUI `feature_control.rs:49` | 16 384 / 16 384 | Yes |
| `MAX_LOGIN_LEN` 254 | `crates/remote/src/lib.rs:81` | 254 | Yes |
| Login rule `^[A-Za-z0-9._%+@-]{1,254}$` | `config.rs:43-47` (printable ASCII 0x21–0x7E, no `,` `;` `"`) | the spec rule is a strict subset (no TOML quoting risk) | Yes (stricter) |
| `rp_id` rule: lowercase, `<node>.<tailnet>.ts.net`, ≥ 2 labels | `config.rs:829-845` (`validate_rp_id`, `labels < 2` refused, `is_valid_host_name`) | same | Yes |
| Prerequisites push ⇐ alerts, offline ⇐ push, camera_view_funnel ⇐ camera_view ∧ allow_funnel, everything needs rp_id | `config.rs:298-358, 614-620, 776-779, 950, 977` | same | Yes |
| `soos-remote --config X passkeys list` is a full configuration check, exit 78 on any refusal | `crates/remote/src/main.rs:184-215, 308-345` | `load_config` + `load_config_with_android` + `resolve_credentials_path` run before the subcommand, each refusal → 78; **but** a refused existing credential store (mode, owner, size, symlink) → `EXIT_RUNTIME` = 1, and an absent store prints "No passkey registered." to stdout and exits 0, writing nothing | Partially (F-m2) |
| Config errors never echo the login | `config.rs` (`InvalidLogin { index }`, "Never echoed in the error") | true | Yes |
| GUI rewrites `key = true` → `key = false`, keeps every other line, refuses tables | `crates/gui/src/feature_control.rs:548-611` | same; the GUI also disables the unit (`systemctl --user disable --now soos-remote.service`, ADR #379 item (2)) | Keys: yes; unit switch: not handled by the plan (F2) |
| `install_remote.sh` copies from `${REPO_ROOT}/target/release` | `scripts/install_remote.sh` `install_companion` | hard-coded `target/release` (bug confirmed) | Yes |
| `install_remote.sh` builds `-p soos-remote -p soos-push-sender` itself | same | `cargo build --release --locked -p soos-remote -p soos-push-sender` on every run | Not considered by the plan (F1) |
| `setup.sh resolve_target_dir` rule (absolute, or relative to the checkout) | `scripts/setup.sh:612-622` | same | Yes |
| Exit code 5 free | `scripts/setup.sh:49-54` | 1, 2, 3, 4, 40, 70 used | Yes |
| `QUERY_TIMEOUT_S` 10 s, `run_bounded` | `scripts/setup.sh:47, 291-299` | same | Yes |
| `MIN_TAILSCALE_VERSION` 1.102.4 | `install_remote.sh` step 3 text, ADR 2026-10-12 | 1.102.4 | Yes |
| Unit type for the verify step | `packaging/soos-remote.service`, `soos-push-sender.service` | `Type=exec`, `Restart=on-failure`, `RestartSec=5`, `RestartPreventExitStatus=78` | Relevant to F-m1 |
| A `daemon.toml` holding only `[preview]` equals runtime defaults + preview | `Docs/DAEMON.md` §1 ("every load path starts from `runtime_default()`", no packaged `daemon.toml`) | true | Yes |
| `install.sh` never creates `daemon.toml` (so "absent" is the common case) | `grep daemon.toml scripts/install.sh` (only `--config` reads) | true | Yes |
| `uninstall.sh` removes a setup-created `daemon.toml` | `scripts/uninstall.sh` | never removes it | Not addressed (F-m8) |
| Existing PR 1 harness never reaches the user phase | `setup_contract.rs` `Harness::run` (`env_clear`, no `XDG_RUNTIME_DIR`, stdin null, no login flags) | companion resolves `skip` (step 4 / step 7), no new calls, no new files | Yes: SET7/SET10/SET11/SET12 snapshots and call lists stay valid |
| Feature sets of a `-p soos-remote -p soos-push-sender` build equal those of `--workspace` | `cargo tree --offline` (metadata only, no build) | **differ**: `nix` (+dir,process,signal), `rustix` (+event,fs,pipe,shm,system,time), `tracing` (+log), `aes-gcm` (+zeroize), `smallvec`, `hashbrown`, `memchr`, `log`, … | No (F1) |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: the root half is handed user-controlled data through `--system-phase`. The plan only forwards `--camera-view on|off`, refuses every other companion option with `--system-phase` (exit 2), and the login never crosses the `sudo` boundary. The `[preview]` uid must come from the root half's own `TARGET_UID` resolution (getent), never from the user half — the spec writes `<uid>` without saying where it comes from (F-m7).
- Failure scenario considered: an owner turns the companion off in the GUI (unit disabled) while a `tailscale funnel --bg` publication persists, then re-runs `setup.sh` to update soos. §4.4 step 5 ("not enabled → `enable --now`") re-enables an internet-reachable service the owner switched off. The plan keeps explicit `false` keys but has no equivalent for the unit switch (F2).
- Failure scenario considered: `daemon.toml` symlink or present-but-foreign → refused / never edited (AS-U2); `ln` no-clobber; 0644 root; `/etc/soos` symlink refused by `ensure_soos_conf_dir`. PASS.
- Result: FINDING (MAJOR, F2; MINOR, F-m7, F-m8)

### Pillar 2 — PAM deadline & concurrency
- Not applicable to PAM: no Rust production change, no PAM change (SET3 still bans `/pam.d`, `pam-auth-update`, `authselect`).
- Setup-side bounding: `systemctl --user` through `run_bounded`; but `soos-remote passkeys list` and `install_remote.sh` (and its internal `systemctl --user daemon-reload`) are unbounded. `passkeys list` only reads bounded files, so this is a hygiene item (F-m2). A `Type=exec` unit reports `active` before it can exit 78, so the immediate `is-active` verify is not a real check (F-m1).
- Result: FINDING (MINOR)

### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered: `enable --now` enables the unit and then fails to start it. If `unit_enabled` is journaled only on success, the rollback removes or restores `remote.toml` but leaves an enabled, crash-looping unit (F-m3).
- Failure scenario considered: daemon restart after creating `daemon.toml` fails, removal + second restart also fails → spec exits 1 with the daemon down, while PR 1 maps "daemon not ready" to 70 (F-m14).
- Failure scenario considered: root half returns 3 (enrollment failed) and the companion then fails or succeeds; root half returns 70/1 — the final exit code and the user-phase message are not specified (F-m4).
- Result: FINDING (MINOR)

### Pillar 4 — Dependencies
- No crate, no new tool beyond `SAFE_TOOLS`-class utilities (`cmp`, `cksum`, `stat`, `sort -V`). The real issue is build reproducibility across two cargo invocations (F1): setup builds `--workspace`, `install_remote.sh` builds `-p soos-remote -p soos-push-sender`; with resolver 2 the dependency feature sets differ (verified with `cargo tree --offline`), so `target/release/soos-remote` is re-uplifted to a different binary by each build. Scenario: run 1 installs B (from the `-p` build); run 2's workspace build puts A back in `target/release`; step 11 `cmp A B` → "will run" → reinstall + restart on every run; the acceptance line "a second run changes nothing" fails on real hardware while the stubbed harness passes. Independently, step 11 is evaluated before the build: after `git pull`, the plan says "already done" against the old artifact and the new companion is never installed (stale binary next to an updated daemon).
- Result: FINDING (MAJOR, F1)

### Pillar 5 — Data confidentiality
- Login never printed (plan says `login: given`; config errors carry only an index); no enroll code, VAPID key or passkey data; `passkeys list` stdout discarded (it prints only fingerprints anyway). Interactive prompt echo is the owner's own terminal. Backup `remote.toml.soos-setup.bak` 0600. No password capture. PASS.
- Minor: §7 claims "0600 in a 0700 directory", but an existing `~/.config/soos` is never re-moded (`mkdir -p` under `umask 077` only affects a new directory); the credential store refuses a group/world-writable parent (`credentials.rs:432`), so a 0775 directory (umask 002 distributions) passes setup's checks and then breaks passkey registration later (F-m10).
- Result: PASS with MINOR (F-m10)

### Pillar 6 — Test integrity
- Existing contracts: the PR 1 harness never reaches the user phase (no `XDG_RUNTIME_DIR`, no TTY, no login flags), so SET7 (`is_read_only_call` rejects `systemctl --user …`), SET10 (exactly two `sudo`), SET11 (snapshot + "already done" list) and SET12 stay valid. `install_remote.sh` contracts (RMC-S6 word rules, RSH13 symlink/`set -C`, RLC-S9 `/etc` detector, RMC-S40 needles, RAN-S10 text) are unaffected by a target-directory variable change if the fix stays in `install_companion`'s two `install -Dm755` lines.
- Static traps the spec walks into (F3): SET3 flags any unquoted `tailscale ` that is not `tailscale version`, on every non-comment line including heredocs and trailing `# …` comments; §4.1 names a plan line `tailscale`, and `plan tailscale "printed only" …` — the natural spelling given the existing `plan packages …` calls — fails SET3. §4.5 "install it from the official instructions" invites `curl -fsSL … | sh`, banned by SET3 (`curl `) and SET2 (`curl` is a package word of `check_build_deps.sh`). SET2 also bans, as whole shell words anywhere in code lines (messages included), `make`, `pam`, `clang`, `openssl`, `curl`, `coreutils`, `wayland`, `gcc`, … — ordinary guidance prose ("make sure …") breaks an immutable test. SET3 bans `$UID` / `${UID` substrings.
- Test power: the planned fake `soos-remote` validates only a few prerequisites; a render bug (unquoted `rp_id`, duplicate key, `offline_alerts` without push in a cascade path the fake does not model) would pass the contract and fail with 78 on hardware (F-m11).
- Process: `tests/invariants/src/setup_user_phase_contract.rs` (1484 lines) and its `lib.rs` registration already exist, uncommitted, before this gate. They are not yet immutable contracts and must follow the revised spec (F-m12).
- Result: FINDING (MAJOR, F3; MINOR, F-m11, F-m12)

## 4. Findings

- **[MAJOR] F1 — Companion install decision is computed before the build and cannot be idempotent with two different cargo invocations.** §4.0 step 11 compares `<target>/release/soos-remote|soos-push-sender` with `~/.local/bin` before setup's `--workspace` build, and §4.4 step 3 skips `install_remote.sh` when "already done". (a) After an update the decision is stale and the new companion is never installed. (b) `install_remote.sh` rebuilds with `-p soos-remote -p soos-push-sender`, whose resolved features differ from `--workspace` (`cargo tree --offline`: `nix`, `rustix`, `tracing`, `aes-gcm`, … differ), so the two builds produce different binaries. Every run then sees a difference, reinstalls and restarts the companion, and "a second run changes nothing" fails on real hardware while the stubbed harness passes. **Required plan change:** decide "installed/changed" after the build from what actually lands in `~/.local/bin`. For example: always run `install_remote.sh` in the user phase (its build is a no-op when fresh), checksum the four installed files before and after, and restart only on a real change or a config change. The plan line then reads "checked after the build". Add a contract test with a changed artifact between two runs (installed and restarted) and an unchanged one (no restart), plus an owner hardware row checking a real second run reports the companion "already done" without a restart. Do not change `install_remote.sh` beyond the `CARGO_TARGET_DIR` fix (OD-U5).
- **[MAJOR] F2 — A re-run re-enables a companion unit the owner disabled in the GUI.** ADR #379 item (2) gives the GUI a companion switch (`systemctl --user disable --now soos-remote.service`). §4.4 step 5 turns "not enabled" into `enable --now` whenever the config is `create/replace/rewrite/already done`. With a persistent `tailscale funnel --bg` publication, re-running setup to update soos silently puts an internet-reachable service back online. This contradicts the spirit of OD-U2 ("each can be turned off in the GUI"; explicit off kept). **Required plan change:** when the managed configuration already existed before this run (`already done`/`rewrite`) and `soos-remote.service` is disabled, keep it off. Report "companion kept off (disabled, for example in the GUI)" and print the command. Re-enable only with an explicit flag (for example `--companion on`), and apply the same rule to a disabled `soos-push-sender` while `push_notifications = true`. Mark it "Owner to confirm" and add a contract test. First-time `create`/`replace` keeps enabling as specified.
- **[MAJOR] F3 — The spec does not constrain the user-phase text against the immutable PR 1 lexical contracts, and its own wording leads into them.** SET3 rejects any unquoted `tailscale ` other than `tailscale version` on any non-comment line, including heredoc bodies and trailing `# …` comments. §4.1's `tailscale` plan line written as `plan tailscale "…"` fails it. §4.5's "install Tailscale from the official instructions" invites `curl … | sh`, which breaks SET3 (`curl `) and SET2 (`curl` word). SET2 bans every `check_build_deps.sh` package name as a whole shell word in code lines, messages included (`make`, `pam`, `clang`, `openssl`, `curl`, `coreutils`, `wayland`, `gcc`, …). SET3 bans `$UID`/`${UID`. **Required plan change:** add a "text constraints" subsection. Plan keys and messages naming Tailscale must quote the word (`plan "tailscale" …`) or hold it in a quoted string. No heredoc for guidance and no inline comments containing `tailscale `. Tailscale install guidance points to the download page URL only, never `curl`/`wget`. No package word of `check_build_deps.sh` appears in any new message. Use `id -u`/`TARGET_UID`, never `$UID`. Require the developer to run `setup_contract.rs` unchanged as the first gate.
- **[MINOR] F-m1 — The verify step cannot detect a failed start.** Both units are `Type=exec`, so `is-active` right after `enable --now`/`restart` is `active` even when the process exits 78 milliseconds later (for example the push sender refusing without its sandbox). **Change:** add a bounded settle: poll `systemctl --user show -p ActiveState,SubState,NRestarts` (or `is-active` plus `is-failed`) for about 2–3 s within `QUERY_TIMEOUT_S`, and fail on `failed`, `auto-restart` or `NRestarts > 0`.
- **[MINOR] F-m2 — `passkeys list` facts and bounds.** It exits 1 (`EXIT_RUNTIME`), not 78, when an existing `remote-passkeys.json` is refused (mode, owner, size, symlink). **Change:** state "must exit 0; 78 = configuration refused, 1 = passkey store refused (message names the store)" and run it through `run_bounded`. Note that it writes nothing even when the store is absent (verified).
- **[MINOR] F-m3 — Journal before a partially failing `enable --now`.** **Change:** journal `unit_enabled` when `is-enabled` flips (or split `enable` and `start`), so a start failure after enabling is still rolled back with `disable --now`.
- **[MINOR] F-m4 — Exit precedence unspecified.** **Change:** specify root 0 + companion fail → 5; root 3 + companion OK → 3; root 3 + companion fail → 5 (or 3, but decide); root 70/1/4 → user phase not run, with a `[SKIP] companion: …` line and the re-run command.
- **[MINOR] F-m5 — `usage()` and stale PR 1 texts.** **Change:** state that `usage()` documents every new option and exit code `5` (the SET5 scanner checks only parsed options). Update the existing `plan later "not in this version" "… phone companion, push alerts, Funnel and camera view …"` line and the `[DETECT] tailscale: … (detected only; not configured by this version)` suffix. Keep the `[DETECT] tailscale: <version>` prefix that SET7 checks.
- **[MINOR] F-m6 — A cascade-forced `false` becomes sticky.** After `--password-alerts off`, the render writes `push_notifications = false` and `offline_alerts = false`. A later `--password-alerts on` keeps both "kept off", because a forced false cannot be told apart from an owner false. **Change:** document it in the ADR, Docs and plan output ("off: needs …" then "kept off"), or record forced keys differently. An owner choice is fine; it must be explicit.
- **[MINOR] F-m7 — `[preview]` uid source.** **Change:** state that `allowed_uids = [<uid>]` is the root half's own `TARGET_UID` (getent), never a value forwarded from the user half.
- **[MINOR] F-m8 — Lifecycle of the setup-created `daemon.toml`.** `--camera-view off` on a later run and `scripts/uninstall.sh` both leave it in place (`[preview] enabled = true` stays for the uid). **Change:** document it in the plan output, the ADR and Docs §3.0, or add a marker-based removal path in a later PR.
- **[MINOR] F-m9 — Default-on push is non-functional on systemd-resolved stub hosts** (default on Ubuntu and Fedora): the packaged sandbox hides `/run`. **Change:** besides the warning, print the documented opt-in drop-in steps (`Docs/REMOTE_COMPANION.md` §2d, never written by setup). The summary must not present push as working in that case.
- **[MINOR] F-m10 — Directory mode claim.** §7 says "0600 in a 0700 directory", but an existing `~/.config/soos` is never re-moded, and a group/world-writable one breaks the passkey store later (`credentials.rs:432`). **Change:** refuse the companion options (preflight, exit 2) or report it when `CONFIG_DIR` is group/world-writable, and correct the invariant text.
- **[MINOR] F-m11 — Test power for the render.** The fake `soos-remote` cannot catch a malformed render. **Change:** add a test that extracts the render from `scripts/setup.sh` (or uses the file written by the harness) and parses it with the real `soos_remote::config::parse_config` for the all-on and cascade variants. A crate-side test in `crates/remote/tests/` avoids adding a `soos-remote` dependency to `soos-invariants`.
- **[MINOR] F-m12 — Phase ordering.** `tests/invariants/src/setup_user_phase_contract.rs` and its `lib.rs` registration already exist, uncommitted, before this gate. **Change:** treat them as drafts and realign them with the revised spec (F1, F2, F3, F-m1, F-m3, F-m4) before the Red commit. They become immutable only once committed after APPROVED.
- **[MINOR] F-m13 — AS-U1 deviates from the issue body** (`runuser` for root starts → skip). It is listed under Owner to confirm. Keep it there and in the ADR.
- **[MINOR] F-m14 — Daemon-side failure exit code.** If the daemon cannot be restarted after `daemon.toml` was removed again, the daemon is down. **Change:** map that path to `EXIT_NOT_READY` (70, the PR 1 convention), not 1.

## 5. Verdict
VALIDATION_VERDICT: REVISION_REQUIRED

---

## Round 2 (2026-10-15) — evaluation of "## 10. Revision 1" (R-U1 … R-U16)

I read the implementation draft (`scripts/setup.sh`, the `scripts/install_remote.sh` diff, `tests/invariants/src/setup_user_phase_contract.rs`, `crates/remote/tests/setup_render_tests.rs`, `tests/fixtures/setup/remote.toml`) only to confirm the revision is real. The verdict is on the plan.

### R2.1 Coverage of the round-1 findings

| Round-1 finding | Revision | Status |
|---|---|---|
| F1 install decision | R-U1: `install_remote.sh` runs on every companion run; the decision comes from `cksum` of the four installed files before and after; restart only on a change; SET24/SET25 + owner row SET39 | Resolved. The draft's `installed_fingerprint` and before/after comparison match. The workspace artifacts are no longer compared, so the `-p` vs `--workspace` feature difference cannot cause flapping. |
| F2 disabled unit | R-U2: units enabled only when setup writes the config for the first time (absent/template); an existing managed config with a disabled unit stays disabled (`kept off`, command printed), AS-U5 | Resolved for `soos-remote.service`. **New gap for the sender** (R2-F1). |
| F3 text constraints | R-U3 (SET20) | Resolved. A replay of the SET2 word/substring rules and the SET3 `tailscale ` / `$UID` / forbidden-string rules on the draft `setup.sh` gives zero hits; `bash -n` passes. |
| F-m1 settle | R-U4 `COMPANION_SETTLE_S` = 2 s, then `is-active` per managed unit | Resolved (the settle runs only when a start happened in this run, which is correct). |
| F-m2 validation | R-U5 (0 / 78 / 1, bounded, stdout discarded) | Resolved |
| F-m3 journal order | R-U6 | Resolved |
| F-m4 exit precedence | R-U7 | Resolved |
| F-m5 usage / stale texts | R-U8 | Resolved. The new `[DETECT] tailscale: <version> (…)` keeps the SET7 prefix and contains no `tailscale ` token. |
| F-m6 sticky cascade | R-U9 (documented) | Resolved (owner-visible) |
| F-m7 uid source | R-U10 | Resolved |
| F-m8 daemon.toml lifecycle | R-U11 (documented) | Resolved |
| F-m9 resolver stub | R-U12 | Resolved |
| F-m10 directory mode | R-U13 (SET38) | Resolved |
| F-m11 render test power | R-U14: fixture pinned byte-equal by SET38, parsed by the real `parse_config` in `crates/remote/tests/setup_render_tests.rs`, including variants that must be refused | Resolved |
| F-m12 phase ordering | Drafts realigned before the Red evidence | Accepted. The drafts must be committed only after this gate. |
| F-m13 AS-U1 | kept under Owner to confirm | Resolved |
| F-m14 daemon-side exit | R-U15 (ready again → rollback, exit 1; still not ready → 70) | Resolved |
| — | R-U16 uniform skip / preflight rule; `daemon.toml` symlink never followed | Consistent with §2 and the PR 1 preflight model |

The `install_remote.sh` diff stays inside the OD-U5 scope: a `TARGET_DIR` variable and the two `install -Dm755` sources. It does not affect RMC-S6, RSH13, RLC-S9, RMC-S40 or RAN-S10 (no `sudo`, `tailscale`, `/etc` or config-write change).

### R2.2 Findings (round 2)

- **[MAJOR] R2-F1 — R-U2 keeps the push sender off even when the owner turns push on with `--push on`.** R-U2 (and the draft, `setup.sh` around lines 1271–1277) marks `soos-push-sender.service` as "kept off" whenever the existing config is `managed`, the resolved `push_notifications` is `true` and the sender is not enabled. It never checks whether push was already `true` in the existing file. The sender is never enabled while push is `false` (§4.4 step 5; SET27 asserts it after `--push off`), so this case is normal, not an owner action. Concrete failure:
  1. First run with `--push off`: the file has `push_notifications = false` and the sender stays disabled, as specified.
  2. Later run with `--push on`: the file is rewritten with `push_notifications = true` and `offline_alerts` still `false`.
  3. Because the sender "is not enabled", setup prints `[SKIP] … stays disabled (turned off later)` and exits 0.

  The explicit opt-in is written but never takes effect, and the message wrongly says the owner turned the sender off. The same happens after a GUI "Push off" (key `false`; the unit may still be enabled) followed by `--push on` if the owner also disabled the unit. That case is legitimate, but the first one is not. **Required plan change:** "kept off" applies to `soos-push-sender.service` only when the existing managed file already had `push_notifications = true` (`FILE_BOOL[push_notifications]`). When push goes from `false` in the file to `true` in this run, the sender is enabled and started (journaled `unit_enabled:soos-push-sender.service`, R-U6) like a first write. Add a contract test: create with `--push off`, then run with `--push on` → sender enabled and active, no `[SKIP] … sender`. Keep the existing kept-off test, with push already `true` and the sender disabled.
- **[MINOR] R2-m1 — Cost of R-U1 on the first run.** The first companion run compiles a second feature variant of the `soos-remote` / `soos-push-sender` dependency graph (about 200 crates, per `cargo tree`) after the workspace build. On RAM-limited hosts this is a second peak. **Change:** say so in the plan output or Docs §3.0 (for example "builds the companion separately, first run only"), so that a slow first run is expected. No behaviour change.
- **[MINOR] R2-m2 — R-U2 and a prior `install_remote.sh --uninstall`.** If the owner ran `scripts/install_remote.sh --uninstall` (units removed, config kept), the next setup run reinstalls the unit files through R-U1 but leaves them disabled, reported as "turned off later". That is defensible: the owner removed it. **Change:** make the `[SKIP]` wording neutral ("is not enabled; setup only enables it for a configuration it writes for the first time") and document the case under AS-U5.

### R2.3 Pillar re-check (round 2)
- Pillar 1 (architecture / threat model): R-U2 closes the Funnel re-exposure path for `soos-remote`. R2-F1 is a functional defect, not a security one: it fails toward "off".
- Pillars 2, 3 and 5: resolved by R-U4–R-U7, R-U13 and R-U15; nothing new found.
- Pillar 4 (dependencies / build): R-U1 removes the cross-invocation artifact comparison (R2-m1 is cost only).
- Pillar 6 (test integrity): the PR 1 contracts stay unedited and untriggered. The static SET2/SET3 rules hold on the draft. The render is now checked by the real parser. The missing test is the one in R2-F1.

### R2.4 Verdict (round 2)
One MAJOR finding (R2-F1) remains.

VALIDATION_VERDICT: REVISION_REQUIRED

---

## Round 3 (2026-10-15) — evaluation of "## 11. Revision 2"

| Round-2 finding | Revision 2 | Verified in draft | Status |
|---|---|---|---|
| R2-F1 sender kept off after `--push on` | Kept off only when the existing managed file already had `push_notifications = true` and the sender is not enabled; `false` → `true` enables and starts it (journaled, R-U6) | `setup.sh` ~1273–1279: `SENDER_UNIT_KEPT_OFF` is set only when `FILE_BOOL[push_notifications] == true`. `test_set_u21_push_switched_on_later_enables_the_sender` covers both directions: `--push off` then `--push on` → enabled + active, no `[SKIP]`; push already on and the sender disabled → stays disabled, `[SKIP]` printed. | Resolved |
| R2-m1 second compile | Stated in the `companion` plan line and in Docs §3.0 | Plan line at `setup.sh:1292` mentions "a second compile on the first run". **The `Docs/PACKAGING_AND_PROVISIONING.md` draft does not say it yet** (§3.0 lines 62/104). | Resolved in the plan; docs pending |
| R2-m2 neutral wording | "`<unit>` is not enabled; setup only enables it for a configuration it writes for the first time (or, for the sender, when push is switched on)"; AS-U5 / ADR item (9) | `setup.sh` 1312, 2177, 2184 use the neutral wording | Resolved |

Re-check on the draft: the SET2 package-word rules and the SET3 `tailscale ` / `$UID` rules give zero hits on `scripts/setup.sh`; `bash -n` passes.

### Findings (round 3)
- **[MINOR] R3-m1 — Docs drift to close during implementation.** `Docs/PACKAGING_AND_PROVISIONING.md` §3.0 (line 104) must say that the first run compiles a second variant (R2-m1). It must also say the sender is enabled when push is switched on, not only for a first write (R2-F1). The text currently says "Units are enabled only for a configuration setup writes for the first time". This is a documentation task for the developer/traceability phase; no plan change is needed.

No CRITICAL or MAJOR finding remains.

VALIDATION_VERDICT: APPROVED
