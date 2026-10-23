# Architect Spec — GitHub #362 (owner decisions of 2026-10-23): one command, no manual step

- **Date**: 2026-10-23
- **Issue**: GitHub #362 (epic "one-command install"), owner comment of 2026-10-23; no `AI/BACKLOG.md` id.
  The implementation PRs (§13) each get a GitHub sub-issue of #362 when they start.
- **Branch (this spec)**: `chore/one-command-setup-spec` (specification only: phases 1 and 1.5).
- **Base commit**: `e7826de` (`origin/main`).
- **Composes with**: `fix/push-sender-system-service` (another agent, in flight): `soos-push-sender`
  becomes a sandboxed **system** unit with `DynamicUser=`, installed and enabled by the root half
  (`install.sh` / packaging). This spec assumes that PR merges first and never touches the
  push-sender lines of `setup.sh`, `install_remote.sh` or `packaging/**`; §13 rebases on it.

## 0. Owner request and decisions (2026-10-23)

> "The installation is far too long; too many commands to type and errors because packages are
> missing. I never want to hit these problems again. One command installs everything, and with an
> opt-in the user chooses whether to get the Android APK."

| Id | Decision (verbatim intent, #362 comment of 2026-10-23) |
|---|---|
| OD-T1 | `scripts/setup.sh` **may run** `tailscale serve` and `tailscale funnel` (sudo once), verifies **from outside** that the Funnel URL answers and handles the stuck-Funnel case automatically (`tailscale down` / `tailscale up`). Supersedes "setup runs no tailscale command except `tailscale version`". |
| OD-T2 | `--android` opt-in, local build: JDK 17 (distro package), pinned SHA-256-verified Android command-line tools in the user's home, `platforms;android-36`, `build-tools;35.0.0`, `platform-tools` through `sdkmanager` (licenses accepted explicitly: prompt or flag), the user's own EC signing key (one password prompt, never logged, never in the repo), build through `scripts/android_build.sh`, `apksigner` signing, locale-independent fingerprint, `[android]` at the **end** of `remote.toml`, restart, public `/.well-known/assetlinks.json` check, `adb install` when exactly one device is connected, else the APK path. Idempotent. |
| OD-T3 | An existing `remote.toml` not written by setup is **backed up** to `remote.toml.bak-<UTC timestamp>` (0600, no symlink following) and setup **continues**, saying so. |
| OD-T4 | Every missing build/runtime dependency is detected in **preflight** and installed with the distro package manager; nothing is discovered mid-run. |
| OD-T5 | (#362 2026-10-23 item 4, other PR) `soos-push-sender` = sandboxed system unit. Relied on, not specified here. |

Earlier owner decisions that still hold: default-on companion profile (2026-10-12, ADR 2026-10-15
item (3)); enrollment from a local seat only (2026-10-13); journal group and linger are **print
only** (2026-10-21); no prebuilt APK, #361 stays on hold.

### 0.1 Pain points of the owner's from-scratch Ubuntu 24.04 run → where each disappears

| Pain point | Removed by |
|---|---|
| setup refused a pre-existing `remote.toml` | §6 (backup and continue) |
| `soos-push-sender` 226/NAMESPACE | OD-T5 (other PR) |
| manual `tailscale serve --bg unix:…` / `tailscale funnel --bg …` | §7.3 (root half exposes the socket) |
| Funnel `ERR_CONNECTION_CLOSED` until `tailscale down && tailscale up` | §7.4–§7.5 (outside probe, bounded recovery) |
| Android: SDK location not found, manual cmdline-tools, env vars, licenses, components | §8.2–§8.3 |
| manual `keytool`, French `SHA 256:` breaking `grep SHA256`, manual `[android]`, restart, assetlinks check, `adb install` | §8.4–§8.7 |
| Chrome keeps the tailnet `100.x` address after the phone's VPN is turned off | §10 summary text (cannot be fixed from the PC) |
| passkey enrollment needs the phone on the tailnet once | kept; §10 summary step 1 states it first |
| packages missing mid-run (`unzip` for Gradle, JDK, …) | §5 (preflight dependency matrix and the tool-coverage invariant) |

## 1. Scope & Blast Radius

| Area | Change | PR |
|---|---|---|
| `scripts/check_build_deps.sh` | two new `--print-packages` groups, `companion` and `android` (§5); existing groups byte-identical | A |
| `scripts/setup.sh` | preflight of the new groups; `REQUIRED_TOOLS` table; `remote.toml` backup (§6); managed grammar accepts a trailing `[android]` table (§6.3); identity auto-detection through the exposure helper (§7.1); new flags (§2); `--tailscale-expose` passed to the root half; root-half exposure step; end-to-end probe; Android step; exit codes 6 and 7; summary (§10) | A, B, C |
| `scripts/setup_exposure.sh` | **new**, mode 0755: the only file of the setup chain that runs `tailscale` (other than the existing bounded `tailscale version` in `setup.sh`), `dig` and the probe `curl` (§7) | B |
| `scripts/setup_android.sh` | **new**, mode 0755: SDK, licenses, key, build, sign, fingerprint, `adb` (§8); refuses root | C |
| `android/sdk-pins.properties` | **new**: single pin file for the command-line tools archive and the SDK component ids (§8.1) | C |
| `scripts/android_build.sh` | **additive** mode `release` (`assembleRelease`, unsigned, strict dependency verification, permission check on the release APK); `verify|bootstrap|ci` byte-identical in behavior | C |
| `android/gradle/verification-metadata.xml` | release-variant artifacts added by a reviewed `bootstrap` run (diff reviewed in the candid review) | C |
| `crates/remote` | **none** in production code. `crates/remote/tests/setup_render_tests.rs` gains a test for a new fixture `tests/fixtures/setup/remote_android.toml` (additive) | C |
| `tests/invariants/src/` | new `setup_deps_contract.rs` (A), `setup_exposure_contract.rs` (B), `setup_android_contract.rs` (C) + `lib.rs` registration; owner-approved migrations of existing files (§12) | A, B, C |
| `tests/docker/setup_packages_test.sh` + CI job | **new** (A): every package name of every group exists in `ubuntu:24.04`, `fedora:40`, `archlinux` (pinned digests of the existing Dockerfiles) and every `REQUIRED_TOOLS` command resolves after installing its group | A |
| Docs | `README.md` (install = one command), `Docs/PACKAGING_AND_PROVISIONING.md` §3.0, `Docs/REMOTE_COMPANION.md` §4/§5/§2j + new troubleshooting §2m "Funnel does not answer", `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` (download pin rule extended to the SDK) | A, B, C |
| `AI/DECISIONS.md`, `AI/VERIFICATION_MATRIX.md`, walkthroughs | §11 | A, B, C |

**Untouched**: every crate's production code, `pam_soos.so`, the daemon, IPC, `scripts/install.sh`,
`scripts/install_remote.sh` (its "never touches Tailscale" rule of ADR 2026-10-05 / 2026-10-12
item (2) **stays**), `scripts/uninstall.sh`, `packaging/**` (owned by the push-sender PR), the
Gradle files (no `signingConfigs`; ANA contract forbids them), the app manifest.

No new crate, no new Rust dependency, no network code in any crate. `soos-remote` stays
`AF_UNIX`-only; the push sender stays the only network-capable soos binary.

## 2. Command Line (additions; every existing option unchanged)

```
scripts/setup.sh [existing options] [--android] [--accept-android-licenses]
                 [--no-tailscale] [--no-tailscale-recover]
```

| Option | Default | Meaning |
|---|---|---|
| `--android` | off | Opt-in: JDK 17 + Android SDK + signing key + local release build + `[android]` + `adb install` (§8). Needs the companion to run with Funnel on (`soos-remote` refuses `[android]` without `allow_funnel`): `--android` with `--no-remote` or `--funnel off` → **exit 2** before any change. Needs a TTY on stdin for the one key-password prompt, also with `--yes` (no secret is ever taken from a flag, a file or the environment); `--android` without a TTY → exit 2. |
| `--accept-android-licenses` | off | Non-interactive acceptance of the Android SDK licenses. Without it, an interactive run shows `sdkmanager --licenses` to the user, who answers each license; `--yes --android` without this flag → **exit 2** (licenses are never accepted silently). Without `--android` → exit 2 (option without effect). |
| `--no-tailscale` | off | Setup never runs a Tailscale command other than the read-only `tailscale version` and prints the `serve` / `funnel` commands exactly as today (escape hatch, and the behavior on any host where exposure is not possible, §7.2). Incompatible with `--android` (the assetlinks check needs the public URL): exit 2. |
| `--no-tailscale-recover` | off | Never run `tailscale down` / `tailscale up`; a stuck Funnel is reported with the commands (§7.5). |
| `--tailscale-expose funnel\|serve` | — | **Internal**, root half only (with `--system-phase`), decided by the user half (§7.2). Refused (exit 2) outside `--system-phase`. |
| `--tailscale-recover yes\|no` | — | **Internal**, root half only: the consent collected at plan time (§7.5). |

`--tailscale-login` and `--rp-id` stay and **override** auto-detection (§7.1). `--yes` keeps its
meaning: no plan confirmation. Validation stays complete before any change (exit 2 + usage).

**Interactive questions** (never with `--yes`, never with `--dry-run`), all asked at plan time,
before the first escalation, so the long run that follows is unattended:

1. Plan confirmation (`y`), as today.
2. Identity confirmation when it was auto-detected (§7.1): "Use this PC's Tailscale name `<host>`
   and the Tailscale account that owns it for the phone companion? [Y/n]" (the login is never
   printed, ADR 2026-10-15 item (4)).
3. First Funnel exposure only (Funnel not already active for this socket): the typed phrase
   `yes, expose to the internet` (#362 acceptance line kept for interactive runs; see §9 S-12
   for the `--yes` reconciliation). Anything else → Funnel off for this run (serve only), the
   rest continues.
4. Recovery consent (only when Funnel will be exposed and recovery is allowed by §7.5 guards):
   "If the public address does not answer, may setup restart the Tailscale connection
   (`tailscale down`/`up`, a few seconds offline for every tailnet connection of this PC)? [Y/n]".
5. With `--android`: the license prompts of `sdkmanager` (unless `--accept-android-licenses`) and
   the key password (§8.4). These are the only prompts that happen **after** plan time, inside the
   Android step, which is the last step.

## 3. Constants (all `readonly`; one definition each, in the file that uses it)

`scripts/setup.sh`:

| Name | Value | Meaning |
|---|---|---|
| `EXIT_FUNNEL` | `6` | face + companion installed; Funnel requested but not verified from outside (§7.6) |
| `EXIT_ANDROID` | `7` | face + companion installed; the Android step failed (§8.8) |
| `REMOTE_BACKUP_PREFIX` | `remote.toml.bak-` | §6 |
| `MAX_BACKUP_SUFFIX` | `9` | collision suffixes `-1`..`-9` within one second, then preflight error |
| `ANDROID_PACKAGE` | `org.soos.remote` | the `[android] package` value written (equals `applicationId`; consistency test) |
| `REQUIRED_TOOLS` | associative array `command → group` | §5.2 |

`scripts/setup_exposure.sh`:

| Name | Value | Meaning / bound |
|---|---|---|
| `MAX_TS_JSON_BYTES` | `262144` | bounded read of `tailscale status --json --peers=false`, `serve status --json`; larger → treated as unavailable |
| `TS_QUERY_TIMEOUT_S` | `10` | every read-only `tailscale` call |
| `TS_CHANGE_TIMEOUT_S` | `30` | `serve`, `funnel`, `down` |
| `TS_UP_TIMEOUT_S` | `60` | `tailscale up` (stdin `/dev/null`; an interactive login URL is a failure) |
| `TS_RUNNING_WAIT_S` | `60` | wait for `BackendState == "Running"` after `up`, polled every 2 s |
| `DNS_TIMEOUT_S` / `DNS_TRIES` | `3` / `2` | each `dig` |
| `MAX_INGRESS_IPS` | `4` | IPv4 addresses kept from the authoritative answer |
| `PROBE_CONNECT_TIMEOUT_S` / `PROBE_MAX_TIME_S` | `5` / `10` | each probe request |
| `PROBE_MAX_BYTES` | `65536` | `--max-filesize`; the body is read only for the assetlinks check |
| `FUNNEL_PROBE_WINDOW_S` / `FUNNEL_PROBE_INTERVAL_S` | `90` / `5` | ingress probe window (≤ 18 probes; first certificate issuance takes up to about a minute) |
| `E2E_PROBE_WINDOW_S` | `30` | end-to-end probe after the companion units are active |
| `STUCK_SPAN_S` | `60` | stuck ⇔ every probe result of the last ≥ 60 s (≥ 12 probes) is `tls_eof` and none in the window is `answered` (R2-3; first-certificate issuance can also look like `tls_eof`) |
| `MAX_TAILSCALE_RECOVERIES` | `1` | down/up cycles per run, never more |

`scripts/setup_android.sh`:

| Name | Value | Meaning / bound |
|---|---|---|
| `ANDROID_KEY_ALIAS` | `soos` | |
| `ANDROID_KEY_VALIDITY_DAYS` | `10000` | |
| `ANDROID_KEY_DNAME` | `CN=soos companion` | no personal data in the certificate |
| `MIN_KS_PASS_LEN` / `MAX_KS_PASS_LEN` | `8` / `128` | password bounds (keytool needs ≥ 6); newline or NUL refused |
| `MIN_ANDROID_FREE_KIB` | `4194304` | 4 GiB free under the SDK and cache directories, checked in preflight |
| `SDK_DOWNLOAD_MAX_TIME_S` | `900` | curl `--max-time` for the archive |
| `MAX_SDK_ARCHIVE_BYTES` | `268435456` | `--max-filesize` |
| `SDKMANAGER_TIMEOUT_S` | `1800` | one bounded `sdkmanager` install call |
| `ADB_QUERY_TIMEOUT_S` / `ADB_INSTALL_TIMEOUT_S` | `10` / `120` | |
| `MAX_ADB_OUTPUT_BYTES` | `16384` | bounded `adb devices` parse |

Pins (`android/sdk-pins.properties`, §8.1): `cmdlineToolsUrl`, `cmdlineToolsSha256`,
`cmdlineToolsRevision`, `platform`, `buildTools`, `platformTools`.

**Sentinel semantics**: no `0` timeout exists (every value above is a positive literal; a static
test pins them). An empty or unparsable `tailscale` JSON means "unavailable", never "running". An
empty `dig` answer means "pending" (retried inside the window), never "reachable". An HTTP status
other than the expected one means "answered" for the ingress probe and "failed" for the
end-to-end probe (§7.4/§7.6).

## 4. Phases and Ordering (privileges: escalations unchanged in number)

```
user half (session owner, never root)
 P0 parse + validate flags                                  (no effect; exit 2 on error)
 P1 detect (read-only): os-release, family, packages, seat, tailscale version,
    setup_exposure.sh detect (tailscale status --json --peers=false, serve status --json),
    remote.toml classification, Android state (§8.0)
 P2 plan printed; --dry-run stops here (exit 0, no write, no network)
 P3 interactive questions (§2)                               (no effect)
 E1 sudo #1 — packages: build+models+gui (+companion when the companion runs)
    (+android with --android)                                [only when something is missing]
 P4 toolchain (install_rustup.sh), build (cargo, as the build user)
 E2 sudo #2 — setup.sh --system-phase … [--tailscale-expose funnel|serve]
    [--tailscale-recover yes|no]
      install, access, presence, lock widget, preview, verify, enrollment (unchanged order)
      LAST: exposure step (§7.3–§7.5)  — never changes the root half's exit code
      except through its own report line; returns 0/3 as today
 P5 lock-screen step (unchanged)
 P6 companion phase (remote.toml incl. backup §6, install_remote.sh, units, validation)
 P7 end-to-end probe through the public URL (§7.6)           [as the user, no root]
 P8 Android step (§8) when --android                         [as the user, no root]
 P9 summary (§10); exit code (§4.1)
```

- **Why the exposure runs at the end of the root half, before the companion units exist**: it
  keeps the number of escalations at two (owner: "sudo once"; the package escalation is skipped
  when nothing is missing, so a re-run prompts once). The stuck-Funnel signature (TLS EOF at the
  ingress) does not depend on the backend: with no socket behind it, a healthy ingress answers
  with an HTTP status (502); only a stuck one closes the TLS connection. The backend is verified
  later, as the user (P7).
- **The root half never receives the companion identity** (ADR 2026-10-15 item (2), test SET24
  `!contains(LOGIN) && !contains("--rp-id")` keeps holding): it derives the host from its own
  `tailscale status --json` and the socket path from the uid of `--user` (§7.3).
- **Sudo timestamp**: E2 follows the build, which can outlast the sudo timestamp, so two password
  prompts remain possible on a first run that installs packages — exactly as today. No
  `sudo -v` keep-alive loop and no operator grant are added (§9 S-3).

### 4.1 Exit codes (new ones bold)

Order of precedence = order of the steps: a non-zero root-half code other than 3 stops the run as
today; then companion `5`; then **`6`** (Funnel not verified, P7 or root exposure); then **`7`**
(Android). The first failing step's code is the exit code; later independent steps still run
when their prerequisites hold (the Android step needs P6 = 0; it runs with `6`, skipping only the
assetlinks check and saying so). `3` (enrollment failed) is kept as today when everything after
it succeeds.

## 5. Preflight Dependency Matrix (OD-T4)

### 5.1 New groups in `scripts/check_build_deps.sh` (the only source of package names, SET2)

| Group | debian (Ubuntu 24.04, Debian 12) | fedora (40+) | arch | Needed by |
|---|---|---|---|---|
| `companion` | `jq` `bind9-dnsutils` `curl` `ca-certificates` | `jq` `bind-utils` `curl` `ca-certificates` | `jq` `bind` `curl` `ca-certificates` | `setup_exposure.sh` (JSON, `dig`, probe) |
| `android` | `openjdk-17-jdk-headless` `unzip` `android-sdk-platform-tools-common` | `java-17-openjdk-devel` `unzip` `android-tools` | `jdk17-openjdk` `unzip` `android-udev` | JDK, `unzip` (Gradle and cmdline-tools archives; today undeclared), udev rules so `adb` sees the phone without root |

Array names `DEBIAN_COMPANION`, `FEDORA_COMPANION`, `ARCH_COMPANION`, `DEBIAN_ANDROID`, … in the
existing style. The `udev` package names and the Fedora JDK name are **verified by the Docker job
DEP6 before merge** (the job fails if a name does not resolve in the pinned image); when a family
no longer ships JDK 17 (future Fedora), the implementer records the replacement (JDK 21 runs
Gradle 8.14.5 and AGP with `jvmTarget` 17 unchanged) in the ADR rather than silently changing it.
`adb` itself comes from the SDK `platform-tools` (pinned SDK, §8), not from the distro.

`setup.sh` deduplicates the union of the requested groups, queries each with the existing
`package_installed` (bounded), and installs the missing ones in E1. `--skip-packages` keeps
printing the command and then **fails preflight (exit 2)** when a package of the `android` group
is missing with `--android` (a build that would fail mid-run is refused up front); for the
`companion` group, missing tools turn exposure into "printed only" (§7.2) instead of failing.

**Tailscale itself is not a package dependency** (`#362` out-of-scope list: no third-party
repository is ever added; Ubuntu and Fedora ship it only from Tailscale's repository). Missing or
logged-out Tailscale → exposure "printed only" with the official install link (§7.2, Q1).

### 5.2 Tool coverage table (the guard against "discovered mid-run")

`setup.sh` declares `REQUIRED_TOOLS` (`[jq]=companion [dig]=companion [curl]=companion
[java]=android [unzip]=android …`). Preflight checks `command -v` for every tool of every
requested group **after** E1 (the package may provide it) and **before** P4; a missing tool →
exit 1 `[FAIL] preflight: <tool> (package group <g>) not found after installing <pkgs>`, nothing
built — **except** the `companion` group, whose missing tools only turn exposure into
`printed:<tool> missing` (§7.2), and only tools whose scope is active are checked (R1-4). Scopes of the table are defined in R1-4/R2-5 (only active scopes are checked).

Static invariant SDP-S3 (§12): every external command word that `setup.sh`, `setup_exposure.sh` and
`setup_android.sh` run (found with the SCN lexer of `setup_scanner_contract.rs`, functions and
builtins excluded) is a key of `REQUIRED_TOOLS`, a script of the repository, or an SDK tool called
by absolute path under `ANDROID_HOME`/`JAVA_HOME`. A new external command without an entry fails CI.

## 6. Existing `remote.toml` Not Written by Setup (OD-T3)

### 6.1 Behavior

Classification (`classify_remote_config`) is unchanged: `absent`, `template`, `managed`, `user`.
Symbolic links, a non-regular file, a foreign owner, a group/world-writable `~/.config/soos`
keep being **refused** (exit 2, nothing changed): "no symlink following" is the owner's rule and
the refusal is not the reported pain.

For `CONFIG_STATE == user` (replaces the ADR 2026-10-15 item (5) / AS-U3 refusal):

1. **Identity source**, first that yields both values (R2-9): flags → the user file's own `allowed_logins` (exactly one entry passing `valid_login`) and `rp_id` (passing
   `valid_rp_id`), read with the existing bounded line parser (no TOML library, no `source`)
   → auto-detection (§7.1), with the R1-8 `[WARN]` when detection disagrees.
   None → companion skipped exactly as today without identity, the file **untouched**.
2. Plan line: `[PLAN] companion config: back up <path> to remote.toml.bak-<UTC> and write it -
   the file was not written by setup`.
3. The run (P6) copies the file to `${CONFIG_DIR}/remote.toml.bak-$(date -u +%Y%m%dT%H%M%SZ)`:
   bounded read (`MAX_REMOTE_CONFIG_BYTES` + 1; larger → exit 5 rollback, nothing written), written
   with `set -C` (noclobber, `O_EXCL`) under `umask 077`, mode 0600, never through a link
   (`[[ -L ]]` re-checked immediately before; the directory is the already-validated 0700 one).
   A name that exists → suffix `-1`..`-9` (`MAX_BACKUP_SUFFIX`), then exit 5. The backup is
   compared byte-for-byte with the source (`cmp`) before the managed file replaces it.
4. The managed file is then written exactly as for a template (temporary file + rename, 0600),
   then validated (`soos-remote --config … passkeys list`), and the units are enabled (a file
   written by setup for the first time, ADR 2026-10-15 item (6)).
5. Output: `[DONE] companion config: your previous remote.toml was saved as <backup>; setup now
   manages remote.toml`.
6. **Rollback** (companion failure, exit 5): `remote.toml` is restored from the timestamped
   backup (rename of a copy, 0600); the backup file is kept (never deleted by setup).
7. Keys of the user file that setup does not manage (`socket_path`, `poll_interval_ms`,
   `allowed_hosts`, `credentials_path`, `lock_screen_programs`, `vapid_subject`,
   `push_socket_path`, `push_previews`, `camera_*` tuning, `battery_status`) are **not** carried
   over (setup manages a fixed key set); the summary lists the names (never the values) of the
   keys that were in the backup and are no longer set, so the user can re-apply them in soos-gui
   or by hand. Values are never printed (a key such as `allowed_logins` holds personal data).

### 6.2 Template case

Unchanged (`remote.toml.soos-setup.bak`, SET30 first half).

### 6.3 Managed grammar accepts a trailing `[android]` table

Today any `[android]` line makes a managed file `user`, so the first re-run after `--android`
would back the file up and **drop** the binding. New grammar: after the managed key lines, an
optional final block exactly

```toml

[android]
package = "org.soos.remote"
sha256_cert_fingerprints = ["AA:..:FF"]            # or two entries, ", " separated
```

(each fingerprint `^([0-9A-F]{2}:){31}[0-9A-F]{2}$`, at most `MAX_ANDROID_CERT_FINGERPRINTS` = 2,
no duplicate; nothing after it). Parsed into `FILE_ANDROID_FPS`. A managed file with this block
stays `managed`; re-running setup **without** `--android` re-renders the same block (binding
kept). `render_remote_config` appends the block last (TOML: a key after a table header belongs to
the table — the reason it must be last). Fixture `tests/fixtures/setup/remote_android.toml` is
parsed by `soos-remote` itself in `crates/remote/tests/setup_render_tests.rs` (prerequisites
`rp_id` + `allow_funnel = true` included).

## 7. Tailscale Exposure (OD-T1) — `scripts/setup_exposure.sh`

One helper owns every Tailscale write and every network probe of the setup chain. Rationale: one
small auditable file with an exact command allowlist (static contract EXP1, §12), the same strict
bash rules as `setup.sh` (`set -euo pipefail`, no `eval`, `LC_ALL=C`, no `source`), and
`setup.sh` keeps its current scanners meaningful (it still runs only `tailscale version` itself).
This is **not** a way around the old rule: the rule is superseded by OD-T1, the ADR says so, and
every executed-call assertion that sees the new calls is migrated with the owner's approval (§12).

Modes (positional, validated; unknown → exit 2):

| Mode | Runs as | Effect |
|---|---|---|
| `detect` | user or root | read-only: prints bounded `key=value` lines (`state`, `host`, `login_ok`, `caps_https`, `caps_funnel`, `handler`) |
| `expose --uid <N> --mode funnel\|serve --recover yes\|no` | root only (`id -u` = 0) | §7.3–§7.5 |
| `probe --host <h> --path <p> --expect <code> [--body-has <s>]...` | user | §7.6 (network read-only) |

### 7.1 Detection and identity auto-detection (user half, P1; read-only, also in `--dry-run`)

- `tailscale status --json --peers=false` (bounded `MAX_TS_JSON_BYTES`, `TS_QUERY_TIMEOUT_S`,
  stdin `/dev/null`), parsed with `jq` (fixed filter strings, `--` before input, no `--arg`
  interpolation of untrusted data into filters).
- `state`: `BackendState` (`Running` required for exposure; anything else → printed only).
- `host`: `Self.DNSName` without the trailing dot, lowercased, must pass `valid_rp_id`.
- `login`: `User[Self.UserID].LoginName`, must pass `valid_login` and must **not** be a tagged
  identity (`Self.Tags` non-empty, or a login of `tagged-devices`): a tagged node has no human
  owner, so `--tailscale-login` is then required (the companion is skipped and the flag printed,
  as today). The login is kept in memory only, never printed, never logged, never passed to root.
- `caps_https` / `caps_funnel`: presence of the HTTPS-certificates and Funnel node capabilities in
  `Self.CapMap` (exact key names pinned by the implementer from a **redacted** `status --json`
  fixture captured with Tailscale ≥ 1.102.4 on the owner host; fixture committed under
  `tests/fixtures/setup/`, no login, no IP, no key).
- `handler`: from `tailscale serve status --json`: `none` (no handler on HTTPS 443), `ours`
  (443 `/` → `unix:/run/user/<uid>/soos-remote/remote.sock`, with `funnel` on/off), or `foreign`
  (anything else on 443).
- Identity precedence (R1-8): `--tailscale-login`/`--rp-id` → **managed file** → detection
  (confirmed interactively, §2 Q2; accepted with `--yes`) → user file (§6.1). A value that
  disagrees with the detected one is used as given, with a `[WARN]` naming the host (never the
  login: "the configured login differs from this PC's Tailscale account").
- `detect` prints `login=<value>` on its stdout, which the user half captures with `$(…)` into a
  shell variable only (never echoed, never logged, never passed to root).
- `detect` runs only when the companion runs (not with `--no-remote`, not as root start), and
  reads `serve status --json` only when `state == Running`.

This replaces "never with `--yes`, which then skips the companion" (ADR 2026-10-15 item (4)):
with Tailscale running, `./scripts/setup.sh --yes` now sets up the companion with no flag.

### 7.2 Exposure decision (user half, P1/P2)

`EXPOSE` = `funnel`, `serve` or `printed:<reason>`:

| Condition (first match) | `EXPOSE` |
|---|---|
| companion does not run | none (nothing printed about Tailscale beyond today's lines) |
| `--no-tailscale` | `printed:--no-tailscale` |
| `tailscale` absent / `jq`, `dig` or `curl` absent with `--skip-packages` | `printed:<tool> missing` + install link for Tailscale |
| `state != Running` | `printed:Tailscale is not connected (run: sudo tailscale up)` |
| `XDG_RUNTIME_DIR` ≠ `/run/user/<uid>` or `--runstatedir` ≠ `/run` | `printed:socket outside /run/user/<uid>` (same read-path rule as PFP/SRP: tailscaled connects to the real path) |
| `handler == foreign` | `printed:HTTPS port 443 of this node already serves something else; setup never replaces it` |
| `allow_funnel` resolves `false` (or the interactive phrase was declined) | `serve` (and, if `handler == ours` with Funnel on, the Funnel is turned off for our handler only, §7.3) |
| `caps_https` false | `printed:HTTPS certificates are off in the tailnet (admin console → DNS → HTTPS Certificates)` |
| `caps_funnel` false | `serve` + `[WARN]` with the admin-console steps to grant the Funnel node attribute; exit code 6 at the end because Funnel was requested and is not available |
| otherwise | `funnel` |

Plan lines: `[PLAN] tailscale: expose <host> to the Internet (Funnel) - <socket>` /
`[PLAN] tailscale: expose on the tailnet only (serve) - …` / `[PLAN] tailscale: printed only -
<reason>` (the last keeps the exact prefix of SET23 so the no-Tailscale harness stays green).
`[PLAN] tailscale recovery: allowed|not allowed (<reason>)`.

### 7.3 Expose (root half, last step of `--system-phase`)

1. Re-validate everything from scratch as root (never trust the user half): `--uid` equals the
   uid of `--user`; socket path `/run/user/<uid>/soos-remote/remote.sock` (constructed, never
   received); `detect` again; `state == Running`; host passes `valid_rp_id`; `handler` not
   `foreign`. Any mismatch → `[SKIP] tailscale: <reason>` + printed commands; root exit code
   unchanged.
2. `handler == ours` with the requested Funnel state → `[OK] tailscale: already exposed` (no
   command run besides reads).
3. Otherwise exactly one of (bounded `TS_CHANGE_TIMEOUT_S`, stdin `/dev/null`):
   - `tailscale serve --bg --yes unix:/run/user/<uid>/soos-remote/remote.sock` (serve), or
   - `tailscale funnel --bg --yes unix:/run/user/<uid>/soos-remote/remote.sock` (funnel), or
   - `tailscale funnel --https=443 off` (Funnel turned off for our handler; serve kept).
   The exact flag spelling (`--yes` existence, `--https=443 off`) is pinned by the implementer
   against Tailscale 1.102.4 `--help` output (captured fixture) — if `--yes` does not exist the
   call simply omits it; an interactive "enable Funnel at <URL>" wait is cut by the timeout and
   reported as `caps_funnel` false.
4. Re-read `serve status --json`: `handler` must now be `ours` with the requested Funnel state,
   else `[FAIL] tailscale: configuration not applied` (exit-code effect: 6, reported through the
   user half; the root half itself still returns 0/3).
5. Funnel mode only: ingress probe (§7.4), then recovery (§7.5) when stuck.

Never run (static allowlist EXP1): `serve reset`, `funnel reset`, `--http`, `--tcp`,
`--tls-terminated-tcp`, a TCP or `localhost:` target, `set`, `up` with any argument, `--reset`,
`--operator`, `logout`, `login`, `cert`, `file`, `ssh`. Setup never grants operator rights
(§9 S-3).

### 7.4 Outside reachability check without a third-party service

Problem: on the PC, MagicDNS resolves `<host>` to the tailnet address (`100.x`), so a plain
`curl https://<host>/` tests the tailnet path, not Funnel. Public resolvers (1.1.1.1, 8.8.8.8)
and "is my site up" services are third parties.

Method (each step bounded):

1. **Find the zone's own servers**: `dig +short +time=3 +tries=2 NS ts.net` through the system
   resolver (this query reveals nothing about the node). Keep names matching
   `^[a-z0-9.-]+\.$`, at most 4.
2. **Ask Tailscale's authoritative servers directly**: `dig +short +norecurse +time=3 +tries=2
   @<ns> <host> A` until one answers. These servers are operated by Tailscale, which is already in
   the trust path of every Funnel request, so no new party learns the host. Keep at most
   `MAX_INGRESS_IPS` IPv4 addresses that are **public** (not 0/8, 10/8, 100.64/10, 127/8,
   169.254/16, 172.16/12, 192.168/16, 224/4, 240/4). No address → result `pending`.
3. **Connect to the ingress with the right SNI and full certificate validation**:
   `curl --silent --show-error --output /dev/null --write-out '%{http_code}' --resolve
   "<host>:443:<ip>" --noproxy '*' --proto '=https' --tlsv1.2 --max-redirs 0 --connect-timeout 5
   --max-time 10 --max-filesize 65536 --user-agent soos-setup-probe "https://<host>/"`.
   `--resolve` pins the TCP destination to the public ingress while SNI, `Host` and certificate
   verification use `<host>`; no `-k`, no cookies, no credentials, no proxy.
4. **Classify** (`curl` exit code → result): `0` with any HTTP status → `answered:<code>`
   (ingress healthy; 502 just means no backend yet); `35`, `52`, `56` → `tls_eof` (the owner's
   `ERR_CONNECTION_CLOSED` signature); `60` (certificate not yet valid for the host) → `pending`
   (first certificate issuance); `6`, `7`, `28` → `unreachable` (path or firewall problem, not
   the ingress); anything else → `error:<n>`.
5. **Window**: probe every `FUNNEL_PROBE_INTERVAL_S` for at most `FUNNEL_PROBE_WINDOW_S`, rotating
   over the ingress IPs. Stop early on `answered`. At the end: `answered` → `[OK] funnel: reachable
   from the Internet`; every result of the last `STUCK_SPAN_S` seconds `tls_eof` and none `answered` → **stuck**; `unreachable` /
   `pending` / `error` → `unverified` with the reason and "check from the phone with Wi-Fi off and
   the Tailscale VPN off" (never a recovery: down/up does not fix a local firewall).

Assumption to verify on the owner host (matrix row OCS-H1): a request from the node to its own
Funnel ingress is relayed back to the node (hairpin through the Funnel relay). If it is not, the
probe yields `unreachable`, never `answered`, so the failure mode is "unverified" (exit 6 with
guidance), never a false "reachable" and never a needless down/up.

### 7.5 Stuck-Funnel recovery (root half; bounded; at most once per run)

Preconditions (all, else `[FAIL] funnel: stuck` + the manual commands, no recovery):

- `--tailscale-recover yes` (plan-time consent: interactive Q4 answered yes, or `--yes`;
  `--no-tailscale-recover` → `no`).
- Not reached over the tailnet: the user half sets recover `no` when `SSH_CONNECTION`'s client
  address is in `100.64.0.0/10` or `fd7a:115c:a1e0::/48`, or when the logind session is remote and
  `SSH_CONNECTION` is unparsable (`tailscale down` would cut the user's own session).
- No recovery already done in this run (`MAX_TAILSCALE_RECOVERIES` = 1).

Sequence:

1. `tailscale down` (bounded `TS_CHANGE_TIMEOUT_S`).
2. `tailscale up` with **no argument** (bounded `TS_UP_TIMEOUT_S`, stdin `/dev/null`). Plain `up`
   re-uses the stored preferences; it is the owner-verified fix. Never `--reset` (it would drop
   the user's settings).
3. Failure of step 2 (non-zero, timeout, login URL): the node is now **down**. Setup prints at
   once and again as the **first line of the summary**: `[FAIL] tailscale: the connection is DOWN
   — run: sudo tailscale up` plus the first 2 KiB of `tailscale up`'s stderr with control
   characters removed (it names the flags Tailscale wants when preferences are non-default).
   Exit code 6. Setup does not retry and never guesses flags.
4. Wait `BackendState == Running` (≤ `TS_RUNNING_WAIT_S`). Re-read `serve status --json`;
   re-apply §7.3 step 3 only if our handler disappeared.
5. Probe window again (§7.4). `answered` → `[OK] funnel: reachable after reconnecting Tailscale`;
   otherwise `[FAIL] funnel: still not reachable` + support text (§10), exit 6.

Worst-case added time: 30 + 60 + 60 + 30 + 100 s ≈ 5 min, printed in the plan line.

### 7.6 End-to-end probe (user half, P7, after the units are active)

`setup_exposure.sh probe --host <rp_id> --path / --expect 200` (same DNS + `--resolve` method,
window `E2E_PROBE_WINDOW_S`). `GET /` is the web app shell (`Route::Asset(Index)`, Funnel-public
per `is_funnel_public`), so a healthy chain answers 200 anonymously. Runs when the user half's
own `detect` (§7.7) shows our handler with Funnel on. Result `answered:200` → `[OK] companion: https://<host>/
answers from the Internet`; any other → `[WARN]` with `journalctl --user -u soos-remote -n 50`,
exit 6.

### 7.7 No channel from the root half to the user half

The user half never needs the root half's exposure result: after the companion units are
active (P7) it runs `setup_exposure.sh detect` again as itself (read-only) and derives every
outcome from the live state — **only when `EXPOSE ∈ {funnel, serve}`** (a `printed:` run never
changes the exit code): `state != Running` → the summary's first line is `[FAIL] tailscale:
the connection is DOWN - run: sudo tailscale up` (exit 6); requested Funnel but `handler` is not
`ours` with Funnel on → `[FAIL] funnel: not configured` (exit 6); otherwise the end-to-end probe of
§7.6 decides. The root half's own `[OK]/[FAIL] tailscale|funnel` lines are for the human only.
Rejected alternatives: a key in `setup.json` (root 0600; reading it from the user half would
weaken that rule), a file written by root into a user-chosen path (symbolic-link attack surface),
piping the root half's stdout through `tee` (would turn the enrollment's terminal into a pipe).
The state file format stays 1.

## 8. Android Opt-In (OD-T2) — `scripts/setup_android.sh`

Runs as the session owner only (`id -u` ≠ 0 else exit 2), after a successful companion phase,
inside `setup.sh` P8. Inputs passed as arguments: `--rp-id`, `--remote-config <path>`,
`--funnel-verified yes|no`, `--accept-licenses yes|no`, `--dry-run`. No environment variable is
trusted for paths except `HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` (absolute, owned by the user,
not inside the checkout — else exit 2).

### 8.0 Locations (never inside the repository; a resolved path under `WORKSPACE_ROOT` → exit 2)

| What | Path | Mode |
|---|---|---|
| SDK (`ANDROID_HOME`) | `${XDG_DATA_HOME:-$HOME/.local/share}/soos/android-sdk` | dir 0700 |
| archive cache | `${XDG_CACHE_HOME:-$HOME/.cache}/soos-android/<sha256>/` (passed to `android_build.sh` as `SOOS_ANDROID_CACHE=…`, whose own default ignores `XDG_CACHE_HOME`) | 0700 |
| signing key | `${XDG_DATA_HOME:-$HOME/.local/share}/soos/android-signing/soos-android.p12` | dir 0700, file 0600 |
| signed APK | `${XDG_DATA_HOME:-$HOME/.local/share}/soos/android/soos-remote.apk` | 0644 |

Each directory: created with `umask 077`, refused when a symbolic link, not owned by the user or
group/world-writable.

### 8.1 Pins (`android/sdk-pins.properties`)

```properties
# Single pin of the Android command-line tools (values from
# https://developer.android.com/studio#command-line-tools-only, "SHA-256 checksum" column).
cmdlineToolsUrl=https://dl.google.com/android/repository/commandlinetools-linux-<BUILD>_latest.zip
cmdlineToolsSha256=<64 lower-case hex, filled by the implementer from Google's published value>
cmdlineToolsRevision=<Pkg.Revision of the archive's source.properties>
platform=platforms;android-36
buildTools=build-tools;35.0.0
platformTools=platform-tools
# R2-2: SHA-256 of the fixed-revision build-tools files, checked after install and before use.
buildToolsSha256.aapt2=<64 lower-case hex>
buildToolsSha256.zipalign=<64 lower-case hex>
buildToolsSha256.lib/apksigner.jar=<64 lower-case hex>
```

Rules (static contract SAN-S1, same supply-chain style as the Gradle pin): URL must match
`^https://dl\.google\.com/android/repository/commandlinetools-linux-[0-9]+_latest\.zip$`; SHA-256
`^[0-9a-f]{64}$`; download with `curl --fail --proto '=https' --proto-redir '=https' --max-redirs
5 --tlsv1.2 --connect-timeout 20 --max-time 900 --max-filesize <MAX_SDK_ARCHIVE_BYTES>` into
`<file>.part`, then `sha256sum --check --status`; mismatch → file deleted, exit 7; extraction only
after the check, with `unzip -q` into a fresh temporary directory, then moved to
`cmdline-tools/latest` (the `sdkmanager` layout). `platform`/`buildTools` must equal
`compileSdk = 36` / `buildToolsVersion = "35.0.0"` of `android/app/build.gradle.kts` and
`BUILD_TOOLS_VERSION` of `android_build.sh` (consistency test SAN-S2 — three literals exist
already and ANA-S10 pins one of them, so a consistency test is used instead of a single source).

**Residual (documented, R2-2)**: `sdkmanager` downloads the components from Google's repository
over HTTPS and checks them against the checksums of Google's repository manifest. The fixed
revision `build-tools;35.0.0` is additionally pinned file by file (R2-2); `platforms;android-36`
and `platform-tools` follow Google's latest revision and are not pinned by this repository. Same trust level as the Android Gradle Plugin itself; the Gradle
dependencies stay covered by strict dependency verification.

### 8.2 JDK

From the `android` package group (E1). `JAVA_HOME` is resolved, never taken from the environment:
first `/usr/lib/jvm/*` directory whose `bin/java -XshowSettings:properties -version` prints
`java.specification.version = 17` (bounded 10 s); none → exit 7 (cannot happen after a
successful E1 unless the package layout changed — reported with the directory list).

### 8.3 SDK components and licenses

1. Archive and `cmdline-tools/latest` present with matching `source.properties` revision →
   `[OK] android sdk: command-line tools <rev> already installed` (no network).
2. Licenses: `$ANDROID_HOME/licenses/` already holds the files → skip. Else with
   `--accept-licenses yes`: `(yes || true) | head -n 64 | sdkmanager --sdk_root=… --licenses`
   (bounded input; `|| true` because `yes` dies of SIGPIPE under `pipefail`, as in `android.yml`); else (interactive): `sdkmanager --sdk_root=… --licenses` on the
   user's TTY, the user answers; a refused license → exit 7 `[FAIL] android: the SDK licenses were
   not accepted; nothing was built`.
3. Components whose `source.properties` exists are skipped; the others in one call
   `sdkmanager --sdk_root=… --install "<platform>" "<buildTools>" "<platformTools>"` (bounded
   `SDKMANAGER_TIMEOUT_S`), then re-checked.

All SDK tools are called by absolute path (`$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager`,
`…/build-tools/35.0.0/apksigner`, `…/zipalign`, `…/platform-tools/adb`) with `JAVA_HOME` and
`ANDROID_HOME` set by prefix assignment for that command only.

### 8.4 Signing key (security-critical)

- **Reuse**: the key file exists → never recreated, never overwritten. The password prompt is
  still needed to sign (one prompt per run).
- **Create** (first run): `read -r -s -p "Password for your new Android signing key (min 8
  characters, it protects the key that lets your phone sign in): "` then one confirmation entry
  (`read -r -s`); mismatch → asked again, at most 3 times, then exit 7. Bounds `MIN_KS_PASS_LEN`
  ..`MAX_KS_PASS_LEN`, no newline/NUL.
- `keytool -J-Duser.language=en -J-Duser.country=US -genkeypair -storetype PKCS12 -keystore
  <path> -alias soos -keyalg EC -groupname secp256r1 -sigalg SHA256withECDSA -validity 10000
  -dname "CN=soos companion" -storepass:env SOOS_KS_PASS -keypass:env SOOS_KS_PASS`, with
  `SOOS_KS_PASS` set by **bash prefix assignment** (`SOOS_KS_PASS="${pw}" "${KEYTOOL}" …`) — never
  `export`, never `env VAR=…` (which would put it in `env`'s argv), never on any command line,
  never in a file, never echoed; `set +x` is asserted at the top (the script never enables
  tracing); `unset pw` right after the last use; the variable is not inherited by any other child.
  Created under `umask 077` into a temporary name in the 0700 directory, then `mv -n` into place
  (no overwrite race).
- Never `~/.android/debug.keystore`, never a CI key; the path is checked to be outside the
  checkout. `*.p12` stays git-ignored and rejected by `scripts/secret_scan.sh` (already the case).
- Summary reminder: "Back up `<key path>` and its password: losing it means reinstalling the app
  signed with a new key".

### 8.5 Build, sign, fingerprint

1. `ANDROID_HOME=… JAVA_HOME=… scripts/android_build.sh release` (new additive mode:
   `assembleRelease` with `--dependency-verification strict`, then the existing permission
   allowlist check on `app/build/outputs/apk/release/app-release-unsigned.apk`; bounded by the
   existing Gradle/curl bounds). The **release** variant is used, not the debug APK, so the
   installed app is not `debuggable` (closes the residual stated in `Docs/REMOTE_COMPANION.md`
   §2j). Gradle never sees the key (no `signingConfigs`, ANA contract unchanged).
2. `zipalign -c -P 16 4 <unsigned>`; not aligned → `zipalign -P 16 -f 4 <unsigned> <aligned>`.
3. `SOOS_KS_PASS="${pw}" apksigner sign --ks <p12> --ks-key-alias soos --ks-pass
   env:SOOS_KS_PASS --v1-signing-enabled false --v2-signing-enabled true --v3-signing-enabled true
   --out <tmp>.apk <aligned>` (minSdk 34: no v1), then `mv` to the final APK path.
4. **Fingerprint (locale-independent)**: `apksigner verify --print-certs <apk>` (fixed English
   messages), exactly one line matching `^Signer #1 certificate SHA-256 digest: ([0-9a-f]{64})$`
   and no `Signer #2`; cross-check with `keytool -J-Duser.language=en -exportcert -keystore <p12>
   -alias soos -storepass:env SOOS_KS_PASS` (DER bytes on stdout) piped to `sha256sum` — both
   digests must be equal, else exit 7. Converted to upper case with `:` between pairs. `keytool
   -list` text output is **never parsed** (French prints `SHA 256:`).

### 8.6 `[android]` table, restart, assetlinks

1. Desired `FILE_ANDROID_FPS` = `[current]` (a different previous fingerprint is replaced and the
   summary says the previously installed app must be reinstalled; setup never keeps a key it no
   longer holds as a trust anchor).
2. Unchanged → `[OK] android binding: already in remote.toml`, no write, no restart.
3. Changed → render the managed file with the block last (§6.3) to a temporary file, validate it
   with `soos-remote --config <tmp> passkeys list` (exit 78 → nothing written), keep the previous
   file as `remote.toml.soos-setup.bak` (existing rule), rename, then
   `systemctl --user restart soos-remote.service` (bounded `QUERY_TIMEOUT_S`, unit must be active
   after `COMPANION_SETTLE_S`). Restart failure → previous file restored, restarted again, exit 7.
4. `--funnel-verified yes` → `setup_exposure.sh probe --host <rp_id> --path
   /.well-known/assetlinks.json --expect 200 --body-has '"org.soos.remote"' --body-has
   '<fingerprint>'` → `[OK] android binding: assetlinks.json is published at https://<host>/…`;
   else `[WARN]` + the URL to open on the phone later (exit 7 only when the probe ran and failed).

### 8.7 `adb install`

1. `adb devices` (bounded, output ≤ `MAX_ADB_OUTPUT_BYTES`); if its output says the server was
   started by this call, `adb kill-server` runs at the end of the step (setup leaves no adb
   daemon it started).
2. Lines `^<serial>\t(device|unauthorized|no permissions.*)$`:
   - exactly one `device` and nothing else → `adb -s <serial> install -r <apk>` (bounded
     `ADB_INSTALL_TIMEOUT_S`). `INSTALL_FAILED_UPDATE_INCOMPATIBLE` (app signed with another key)
     → never uninstalls automatically (app data would be lost); prints the uninstall command;
     exit 7.
   - `unauthorized` → "Unlock the phone and accept the USB debugging prompt, then re-run setup".
   - `no permissions` → the udev hint (the `android` group installs the rules; re-plug the phone).
   - zero or several devices → APK path + "copy it to the phone and open it, or connect exactly one
     phone with USB debugging and re-run `./scripts/setup.sh --android`". Not an error (exit 0 for
     this step): installing on a phone is the user's choice.
3. The serial is never printed in full (last 4 characters only).

### 8.8 Idempotence and failure of the Android step

Re-run with nothing changed: no download (checksums match), no `sdkmanager` install, key reused
(one password prompt), Gradle incremental build, signature identical → `[android]` unchanged → no
restart; `adb install -r` repeated when one phone is connected. Failures never roll back the SDK,
the cache or the key (kept like the toolchain); only `remote.toml` is restored (§8.6). Exit 7.

## 9. Security Invariants

| Id | Invariant |
|---|---|
| S-1 | `setup.sh` itself still runs no Tailscale command other than the bounded `tailscale version`, holds no `curl`/`wget` and downloads nothing (SET3, SET41, SCN2/3/6 stay byte-identical and green). Every Tailscale call, DNS query and probe lives in `setup_exposure.sh`; every SDK download in `setup_android.sh` / `android_build.sh` — each with its own allowlist contract. |
| S-2 | Exposure target is only `unix:/run/user/<uid>/soos-remote/remote.sock` at `/` on HTTPS 443; never a TCP port, never `--http`, never `reset`; a foreign 443 handler is never replaced. |
| S-3 | No persistent privilege grant: no `tailscale set --operator`, no sudoers drop-in, no `sudo -v` keep-alive, no polkit rule. Tailscale changes run as root inside the existing system-phase escalation. |
| S-4 | Setup turns Funnel on only when the `allow_funnel` it resolved for this run is `true` (flags, kept explicit `false` of a managed **or user** file, prerequisites); `--funnel off` turns our Funnel off; the interactive first exposure needs the typed phrase. Exposure happens in the root half **before** the companion phase: if the companion phase then fails (exit 5), the Funnel stays on but only reaches the companion socket — absent socket ⇒ public 502, and a restored older `remote.toml` with `allow_funnel = false` ⇒ `soos-remote` refuses Funnel requests itself; the summary prints `sudo tailscale funnel --https=443 off` (through `setup_exposure.sh hint`) and the next run reconciles (R1-7). |
| S-5 | `tailscale down`/`up`: at most once per run, never over a tailnet SSH session, only with plan-time consent, never `up` with flags, failure loudly reported as "connection is DOWN" with the exact command. |
| S-6 | The Tailscale login is never printed, logged, passed to root or written outside `remote.toml`; the status JSON is never stored; JSON reads are bounded; `jq` filters are constants. |
| S-7 | Probes: HTTPS only, full certificate validation, no redirects, bounded time and size, no cookies/credentials/proxy, fixed User-Agent; no third-party service (only the system resolver for `NS ts.net` and Tailscale's authoritative servers). |
| S-8 | Android pin: exact URL + SHA-256 checked before extraction; no `-k`/`--insecure`; downloads only over `https` with `--proto-redir '=https'`. |
| S-9 | Key: outside the checkout, 0600 in a 0700 directory, PKCS12 EC P-256, never overwritten; password read with `read -s`, passed only by prefix-assigned environment to `keytool`/`apksigner`, never in argv, logs, files, the summary or an error message; no `set -x`. |
| S-10 | Fingerprint derived from the signed APK and cross-checked with the keystore certificate bytes; text output of `keytool -list` never parsed. |
| S-11 | `remote.toml` backup: no link followed, `O_EXCL`, 0600, byte-verified, never deleted; values of the user's file never printed. |
| S-12 | **Typed confirmation vs one command**: #362's line "`--yes` alone never enables Funnel/remote unlock/camera over Funnel" was superseded by the owner's default-on decision of 2026-10-12 (ADR 2026-10-13 item (7), ADR 2026-10-15 item (3)). This spec keeps the typed phrase for the **first** interactive exposure (the one moment the PC becomes reachable from the Internet) and lets `--yes` expose, with the plan line and the summary stating it in plain words. Owner to confirm (Q2). |
| S-13 | Every external command has a `REQUIRED_TOOLS` entry; every package name lives only in `check_build_deps.sh`. |
| S-14 | Nothing of PAM, the daemon config rules, the kill switch, the enrollment rules, journal group or linger changes (print-only decisions of 2026-10-21 kept). |

## 10. Final Summary Output (P9)

Order: (0) any `connection is DOWN` line first; (1) what was done; (2) what the user still has to
do; (3) rescue/undo. Example (Funnel verified, `--android`, one phone connected):

```
== soos is installed ==
[DONE] face unlock: enrolled alice, presence auto-unlock on
[DONE] phone companion: https://pc.tail1234.ts.net/ answers from the Internet (Funnel)
[DONE] Android app: built, signed with your key, installed on the phone (…9F3A)

On your phone, once:
  1. Turn the Tailscale VPN ON, open https://pc.tail1234.ts.net/ in Chrome.
     At this PC run:  soos-remote enroll-code     (the code is shown only here, valid 5 minutes)
     Type the code on the phone and create the passkey. Passkey registration only works over
     the tailnet; this is the only time the phone needs Tailscale.
  2. Turn the Tailscale VPN OFF. Open the soos app (or the page) and sign in with the passkey.
     If Chrome still tries the old 100.x address: Chrome → chrome://net-internals/#dns →
     "Clear host cache", then #sockets → "Flush socket pools" (or toggle airplane mode).
  3. Tap "Enable notifications" when asked.
Back up your Android signing key: ~/.local/share/soos/android-signing/soos-android.p12

Optional: loginctl enable-linger alice …   (printed, never run — unchanged)
Undo: …
```

Variants: no phone connected → step "Install the app: copy <apk> to the phone…"; Funnel
unverified → "Check from the phone with Wi-Fi and the Tailscale VPN off: open https://…; if it
does not load, see Docs/REMOTE_COMPANION.md §2m" (exit 6); exposure printed only → today's
`tailscale funnel --bg unix:…` lines with the reason. The enroll code is never generated by setup
(it would be logged into a scrollback the user may share; the 300 s TTL also outlasts nothing
useful at the end of a long run) — Q3 asks whether the owner wants setup to run it at the very
end of an interactive local-seat run.

## 11. Documentation, ADR, Matrix, Walkthroughs

- **ADR** (one, appended; drafted text for the implementer):
  *[2026-10-2x] One Command Without Manual Steps: Tailscale Exposure, Android Opt-In, Preflight
  Dependencies (GitHub #362, owner decisions of 2026-10-23).* Amends ADR 2026-10-13 "One-Command
  Setup" item (8) (Tailscale carve-out → exposure helper with allowlist, items S-1..S-5) and item
  (9) (Tailscale serve, Docker package matrix delivered); ADR 2026-10-15 "Setup User Phase" items
  (4) (identity auto-detected from `tailscale status --json --peers=false`; `--yes` no longer
  skips the companion when Tailscale is running), (5) and (9) AS-U3 (user file backed up and
  replaced), (8) (setup runs serve/funnel; the printed commands remain the fallback); ADR
  2026-10-13 "Native Android App" (release signing: the app is built as the unsigned release
  variant and signed by `apksigner` with the user's own key created by setup; debug re-signing
  stays documented for developers). Records: hairpin probe assumption, Fedora JDK rule,
  sdkmanager component residual, exit codes 6 and 7, no root-to-user result channel (§7.7). Unchanged and
  restated: ADR 2026-10-05 item (8) / 2026-10-12 item (2) (`install_remote.sh` never runs
  Tailscale), ADR 2026-10-06 agent rule (agents never run `tailscale serve`/`funnel`; all tests
  use stubs).
- **Matrix**: new component sections **SDP1–SDPn** (dependencies + backup, PR A), **STX1–STXn**
  (exposure, PR B), **SAN1–SANn** (Android, PR C); SET3, SET23, SET24, SET29, SET30 row texts
  updated to name the migrations (§12); owner hardware rows **OCS-H1** (from-scratch Ubuntu 24.04:
  `./scripts/setup.sh --android` → phone signs in over Funnel; includes the hairpin check and,
  when it happens, a stuck-Funnel recovery) and **OCS-H2** (Arch re-run idempotence).
- **Walkthroughs**: one per PR, next free numbers at implementation time (254+ today).
- **Docs**: README install section = `git clone … && ./scripts/setup.sh` (+ `--android`);
  `Docs/PACKAGING_AND_PROVISIONING.md` §3.0 flags/exit codes/phases; `Docs/REMOTE_COMPANION.md`
  §4/§5 (manual path kept, one-command path first), §2j (release build via setup, key location,
  backup), new §2m "Funnel does not answer" (signature, recovery, Chrome cache, hairpin note);
  `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` (pinned downloads list + the SDK pin).

## 12. Tests

### 12.1 New contracts (tester-agent writes them first; they must fail before the code)

**PR A — `setup_deps_contract.rs`** (own harness, cloned from `setup_user_phase_contract.rs`):
- SDP-S1 `check_build_deps.sh --print-packages companion|android` for debian/fedora/arch print the
  §5.1 lists; unknown group still exits non-zero.
- SDP-S2 SET2 extended: no name of the `_COMPANION`/`_ANDROID` arrays appears in `setup.sh`,
  `setup_exposure.sh`, `setup_android.sh`.
- SDP-S3 tool coverage (§5.2) with the SCN lexer: synthetic bad snippet (`foo --x` unknown tool)
  rejected, good accepted, then the three real scripts.
- SDP-B1 missing `jq` + companion → E1 installs it **before** the build (log order: `sudo pacman
  -S … jq` < `cargo build`); `--android` adds the android group; without it the android group is
  never queried.
- SDP-B2 tool still missing after E1 (stub package that installs nothing) → exit 1 before
  `cargo`, message names tool and package.
- SDP-B3 `--skip-packages --android` with JDK missing → exit 2, nothing changed.
- SDP-B4..B9 backup (§6): user file + flags → exit 0, exactly one `remote.toml.bak-<YYYYmmddTHHMMSSZ>`
  (regex) 0600 equal to the original bytes, managed file written, units enabled, `[DONE]` line
  names the backup, no value of the user file in the output; identity taken from the user file
  when no flag; no identity anywhere → file untouched; collision suffix; symlinked file still
  refused (exit 2, target untouched); companion failure → original restored, backup kept, exit 5;
  removed-key names listed.
- SDP-B10 managed file with a trailing `[android]` block stays managed and is re-rendered
  byte-identically by a run without `--android`; a malformed block (lower-case hex, 3 entries,
  a key after it) → `user` → backup path.
- SDP-D1 Docker `tests/docker/setup_packages_test.sh` (pinned images): every name of every group
  resolves (`apt-cache show`, `dnf repoquery --available`, `pacman -Si`), and after installing a
  group every `REQUIRED_TOOLS` command of that group resolves with `command -v`. CI job
  "setup.sh package matrix (Docker)" added to `ci-success` needs.

**PR B — `setup_exposure_contract.rs`**:
- STX-S1 static allowlist of `setup_exposure.sh` with the SCN lexer: every `tailscale` command
  word sequence is one of `version`, `status --json --peers=false`, `serve status --json`,
  `serve --bg [--yes] unix:${SOCKET}`, `funnel --bg [--yes] unix:${SOCKET}`, `funnel --https=443
  off`, `down`, `up` (no further word); synthetic bad inputs (`serve reset`, `up --reset`,
  `set --operator=x`, `serve --http 80`, `funnel --bg 8080`) rejected first.
- STX-S2 `curl` only in the probe form (all §7.4 flags present, no `-k`/`--insecure`/`-L`);
  `dig` only `NS ts.net` and `@<ns> <host> A` forms; `jq` filters are single-quoted literals.
- STX-S3 constants (§3) pinned; `set -euo pipefail`, no `eval`, `LC_ALL=C`; `expose` refuses a
  non-root caller.
- STX-B1 dry-run with a running-Tailscale stub: plan shows host and `Funnel`, the only logged
  Tailscale calls are `version`, `status --json --peers=false`, `serve status --json`; no
  `dig`/`curl`; nothing written.
- STX-B2 `--yes` with no identity flag and a running stub → companion set up with the detected
  host/login; the login never in stdout/stderr/log arguments other than the `remote.toml` write;
  the escalation line never contains host or login.
- STX-B3 tagged node → companion skipped, `--tailscale-login` printed.
- STX-B4 handler `ours` → no change command; `none` → exactly one `funnel --bg … unix:/run/user/
  <uid>/soos-remote/remote.sock` as uid 0; `foreign` → no change, printed; `--funnel off` with
  ours+funnel → `funnel --https=443 off`.
- STX-B5 probe classification table driven by a `curl` stub returning each exit code; DNS stubs
  returning a 100.x address are ignored (never probed); empty answer → pending.
- STX-B6 stuck (three `tls_eof`) + consent → `down` then `up` exactly once, then `answered` →
  `[OK] … after reconnecting`; stuck again → exit 6, no second `down`.
- STX-B7 `up` fails → summary's first line `connection is DOWN`, exit 6, no retry.
- STX-B8 `SSH_CONNECTION="100.101.102.103 …"` → recovery never runs (`[PLAN] tailscale recovery:
  not allowed (SSH over the tailnet)`); `--no-tailscale-recover` same.
- STX-B9 `--no-tailscale` → only `tailscale version` runs (the old behavior, byte-identical
  printed lines).
- STX-B10 end-to-end probe 200 → `[OK]`; 502 → `[WARN]`, exit 6; precedence 5 > 6.
- STX-B11 every bound: a `tailscale` stub that sleeps forever is killed at the timeout and the
  run continues (`timeout` in `run_bounded`).

**PR C — `setup_android_contract.rs`** + `crates/remote/tests/setup_render_tests.rs` (new test):
- SAN-S1 pins file format and URL/SHA regex (every `cmdlineToolsSha256` and
  `buildToolsSha256.*` value `^[0-9a-f]{64}$`; exactly the three R2-2 build-tools keys); SAN-S2 consistency of SDK ids with the Gradle file
  and `android_build.sh`; SAN-S3 `setup_android.sh` static: `read -r -s`, no `set -x`, no
  `export SOOS_KS_PASS`, no `env SOOS_KS_PASS`, `--ks-pass env:SOOS_KS_PASS`, `-storepass:env`,
  no `keytool -list` parsing, `-J-Duser.language=en`, no `-k`, `--proto-redir '=https'`,
  `sha256sum --check --status` before `unzip`.
- SAN-B1 `--android` with `--no-remote`, `--funnel off`, `--no-tailscale`, no TTY, `--yes`
  without `--accept-android-licenses` → exit 2, nothing changed.
- SAN-B2 full run with stubs (`curl` serving a fixture zip whose SHA-256 is written into the
  harness copy of the pins file — no production override hook exists; `sdkmanager`, `java`,
  `keytool`, `apksigner`, `zipalign`, `adb` stubs, password fed through a pty helper): sentinel
  password never appears in the stub log (argv), stdout, stderr or any file under the fake HOME
  except the stub keystore; key 0600 in 0700; `[android]` last in `remote.toml` with the
  upper-case colon fingerprint; restart logged; assetlinks probe logged; `adb install -r` once.
- SAN-B3 checksum mismatch → archive deleted, nothing extracted, exit 7; `remote.toml` untouched.
- SAN-B4 re-run: no `curl`, no `sdkmanager --install`, no `keytool -genkeypair`, no restart.
- SAN-B5 fingerprint: `apksigner` stub with two signers → exit 7; digest mismatch with the
  keystore bytes → exit 7; a `keytool` stub printing French `SHA 256:` text changes nothing.
- SAN-B6 adb cases: 0, 2 devices, `unauthorized`, `no permissions`,
  `INSTALL_FAILED_UPDATE_INCOMPATIBLE` (no `uninstall` ever logged).
- SAN-B7 restart failure → previous `remote.toml` restored, exit 7.
- SAN-B8 root caller → exit 2; key path inside the checkout → exit 2.
- SAN-R1 (Rust) the `[android]` fixture rendered by setup parses in `soos-remote` and its
  assetlinks body contains the fingerprint.
- ANA unchanged: `android_build.sh` keeps every ANA-S10 needle; the new `release` mode adds
  `readonly RELEASE_TASKS=(assembleRelease)` and runs `check_permissions` on the release APK.

Every behavioral test runs in the fake root with stubs only (ADR 2026-10-06 agent rule; OD-6 of
#372): no real `tailscale`, no network, no real SDK download, no real `sudo`.

### 12.2 Existing tests that must change — **owner approval required** (immutable contracts)

Category **A** = assertion change (needs explicit approval). Category **S** = harness setup only
(fixture wiring, no assertion touched; accepted practice per the 2026-09-30 precedents, still
listed). Category **I** = assertion text unchanged but the rule it documents is amended (owner
acknowledgment, no code change).

| # | PR | File : test / helper | Current assertion | Proposed change | Cat. |
|---|---|---|---|---|---|
| M1 | A | `setup_user_phase_contract.rs` : `test_set_u12_template_replaced_user_file_untouched` (SET30), second half | user file + `run_companion` → `code == 2` and `before == snapshot` | `code == 0`; exactly one `remote.toml.bak-<UTC>` (0600) byte-equal to `mine`; `remote.toml == managed_remote([false, true, true, true, true, true, false])` (the explicit `allow_funnel = false` of `mine` is kept and forces `camera_view_funnel` off, R1-1); no `funnel --bg` ever logged | A |
| M2 | A | same test, `--yes` run | `code == 0`; file `== mine`; output contains `[PLAN] companion config: kept`; no `systemctl --user enable`; no `--camera-view on` | identity is now read from `mine` (`me@example.com`, `pc.tail1234.ts.net`): file backed up and managed with `allow_funnel = false` kept (same vector as M1); output contains `[PLAN] companion config: back up`; units enabled; no Tailscale change command. The "kept" assertions move to a new case whose user file has no valid identity (file untouched, `[PLAN] companion config: kept`) | A |
| M3 | A | `setup_user_phase_contract.rs` `Harness::new` and `setup_contract.rs` `Harness::new` | `for group in ["build", "models", "gui"]` marks packages installed; fixed copy list of repo files; stub list | add `"companion"` (and `"android"` where `--android` is used) to the group loop; copy `scripts/setup_exposure.sh`, `scripts/setup_android.sh`, `android/sdk-pins.properties`; add a logging `dig` stub (the **real** `jq` joins `SAFE_TOOLS`, never a logging stub, so dry-run read-only checks are unaffected); keep `STUB_TAILSCALE` (version only) as the default so the old tests keep their meaning | S |
| M4 | B | `setup_contract.rs` : `is_read_only_call` | `"tailscale" => arg == "version"` | `arg == "version" \|\| (arg == "status" && rest == "--json --peers=false") \|\| (arg == "serve" && rest == "status --json")` | A |
| M5 | B | `setup_contract.rs` : `assert_no_forbidden_calls` | any logged `tailscale` call must be `version` | face-profile harness: `version`, `status --json --peers=false`, `serve status --json` only (the face harness never reaches exposure); `curl` ban unchanged | A |
| M6 | B | `setup_user_phase_contract.rs` : `is_read_only_call` | `"tailscale" => arg == "version"` | same as M4 | A |
| M7 | B | `setup_user_phase_contract.rs` : `assert_no_forbidden_calls` | any logged `tailscale` call must be `version`; `curl` never runs | `tailscale` calls must belong to the STX-S1 allowlist (shared helper `tailscale_call_allowed`); `curl` allowed only as a **uid-1000** probe line containing `--resolve` and `https://<RP_ID>/` (the root half runs its probe through `runuser -u <user>`, R1-6); `usermod`, `pam-auth-update`, `authselect`, linger bans unchanged | A |
| M8 | B | `setup_contract.rs` : `test_set_s3_…` (SET3) | `tailscale ` in `setup.sh` only as `tailscale version` or quoted; no `curl `/`wget ` | **unchanged code**; matrix/ADR wording becomes "setup.sh itself" (the tool now changes Tailscale through `setup_exposure.sh`) | I |
| M9 | B | `setup_contract.rs` : `test_set_s7_…` (SET41); `setup_scanner_contract.rs` SCN2, SCN3, SCN6 | no Tailscale command other than `version` in a substitution / variable of `setup.sh` | **unchanged code**; same wording note; the helper gets its own allowlist (STX-S1) instead | I |
| M10 | B | `setup_user_phase_contract.rs` : `test_set_u1_static_rules_of_the_user_phase` (SET20) | `tailscale serve`/`funnel` only quoted in `setup.sh`; `tailscale status`, `tailscale up`, `tailscale set`, `--http` never in `setup.sh` | **unchanged code**, which requires that `setup.sh` never contains the substrings `tailscale status`, `tailscale up`, `tailscale set`, `--http` even in a printed message: every message naming them ("run: sudo tailscale up", …) is printed by `setup_exposure.sh hint <reason>` (R1-2). The documented rule "`tailscale status` is never read" (ADR 2026-10-15 item (4)) is amended: `setup.sh` never reads it, `setup_exposure.sh` does | I |
| M11 | B | `setup_user_phase_contract.rs` : `test_set_u10_without_identity_the_companion_is_skipped` (SET28) | `--yes` without identity → companion skipped, re-run flags printed | **unchanged code** (the default stub has no `status`, so detection fails and the skip holds); the ADR sentence "never with `--yes`" is amended | I |

| M12 | A | `tests/docker/setup_root_preview_test.sh` (SET34 Docker job) | copies a fixed set of scripts into its checkout and runs `--yes --skip-packages` with identity flags | also copy `scripts/setup_exposure.sh` (and, from PR C, `setup_android.sh`, `android/sdk-pins.properties`); the image has no `jq`/`dig`/`tailscale`, so exposure is `printed:` and every expected exit code stays as is | S |
| M13 | A | `lockscreen_installer_contract.rs` setup harness (`Harness`, line ~355) | fixed copy list and package groups | same wiring as M3 | S |
| M14 | B | `setup_scanner_contract.rs` lexer | lexer private to the file | moved verbatim into a shared test module (`tests/invariants/src/shell_lexer.rs`) so STX-S1 and the SCN6/SCN7 checks can run on the helpers; every SCN test body byte-identical | S |

No test of `installer_contract.rs`, `android_native_contract.rs`, `remote_*_contract.rs`,
`setup_scanner_contract.rs` (assertions) or the Docker scripts needs an assertion change. The
SET24 escalation assertions (`one escalation`, no login, no `--rp-id`) stay as they are: the new
`--tailscale-expose`/`--tailscale-recover` arguments carry neither.

If the owner refuses M1/M2, OD-T3 cannot be implemented (the test encodes the old refusal). If
the owner refuses M4–M7, OD-T1 cannot be implemented in a run that the harness observes.

## 13. PR Split (each PR: full 4-phase cycle, candid review, CI green, squash merge)

| PR | Branch | Content | Depends on |
|---|---|---|---|
| 0 | `fix/push-sender-system-service` (other agent) | sandboxed system unit | — |
| A | `feat/setup-preflight-deps` | §5 groups + `REQUIRED_TOOLS` + preflight + Docker package matrix; §6 backup + `[android]` grammar; migrations M1–M3; ADR part 1; SDP rows; walkthrough | 0 (rebase only) |
| B | `feat/setup-tailscale-expose` (after spike B0, R1-10) | `setup_exposure.sh`, auto-detection, root exposure, probe, recovery, end-to-end probe, exit 6, summary text, `REMOTE_COMPANION.md` §2m; migrations M4–M11; ADR part 2; STX rows | A |
| C | `feat/setup-android-opt-in` | pins file, `setup_android.sh`, `android_build.sh release`, verification metadata, `[android]` write, assetlinks, adb, exit 7, §2j; SAN rows; owner hardware rows OCS-H1/H2 | B |

Each PR is independently useful: A removes the missing-package and `remote.toml` pain points; B
removes every manual Tailscale step; C the Android ones.

## 14. Test Hooks for the Tester

- Stubs on `PATH` for `tailscale` (scripted by `STUB_TS_*` variables: backend state, caps, handler,
  `up` result, sleep), `dig`, `curl` (exit-code script per call), `jq` (**real** `jq`, added to
  `SAFE_TOOLS`), `sdkmanager`, `keytool`, `apksigner`, `zipalign`, `adb`, `java`; every stub logs
  `uid=<n> <name> <args>` (existing prologue).
- `SSH_CONNECTION`, `XDG_*` env per test; a pty helper (`script -qec` or a tiny Rust pty in the
  test) to feed the key password and the typed phrase.
- The root half in tests is the existing `env -i` sudo stub (real root is covered by the Docker
  job only for the package matrix; exposure never runs for real in CI).
- No production test hook: SDK pins are overridden by writing the **harness copy** of
  `android/sdk-pins.properties`; there is no environment override in `setup_android.sh`.

## 15. Documentation Drift / ADR

- ADR 2026-10-13 item (8) and ADR 2026-10-15 items (4), (5), (8), (9) AS-U3 → amended (§11).
- #362 acceptance "Interactive typed confirmation; `--yes` alone never enables Funnel" →
  superseded since 2026-10-12 for `--yes`, kept for interactive first exposure (S-12, Q2).
- #362 out-of-scope "Installing or logging in to Tailscale itself" → kept (Q1).
- `Docs/REMOTE_COMPANION.md` §2j "Release signing is a later PR" and "The debug APK is
  debuggable" → updated by PR C (release variant through setup).
- `scripts/android_build.sh` header "never installs either" (JDK/SDK) → still true:
  `setup_android.sh` installs them; `android_build.sh` stays a pure builder.
- `unzip` is used by `android_build.sh` today but declared in no package list (drift fixed by the
  `android` group).

## 16. Open Questions for the Owner (real decisions only)

- **Q1 Tailscale install/login**: when Tailscale is missing or logged out, setup prints the
  official steps and continues without exposure (#362 out of scope; Ubuntu/Fedora need
  Tailscale's own repository, which setup never adds). Should setup instead (a) install the Arch
  `tailscale` package and/or (b) run the first interactive `sudo tailscale up` login itself?
- **Q2 Typed phrase**: keep `yes, expose to the internet` for the first interactive Funnel
  exposure (spec default), or drop it so an interactive run needs only the plan `y`?
- **Q3 Enroll code**: should setup run `soos-remote enroll-code` itself at the very end of an
  interactive local-seat run and show the code (valid 5 minutes), or keep printing the command
  (spec default)?
- **Q4 Migrations**: approve M1–M7 (assertion changes) and acknowledge M8–M11 (§12.2).

## 17. Revision 1 (answers the plan evaluation of 2026-10-23; authoritative over §1–§16 where they differ)

- **R1-1 (F1, CRITICAL) User-file choices are kept.** The user-file parser (§6.1) reads the seven
  managed booleans (`COMPANION_KEYS`) with the same bounded grammar as the managed file into
  `FILE_BOOL`; every explicit `false` and the keys its prerequisites force off are kept exactly as
  for a managed file (ADR 2026-10-15 item (3)), unless the matching `--<feature> on` flag is given.
  A user file with `allow_funnel = false` therefore never leads to `funnel --bg` (new test
  SDP-B11, plus M1/M2 expectations). A boolean key with an unparsable value is treated as absent
  (default-on) only when the run is interactive and the plan line names it; with `--yes` an
  unparsable boolean → preflight exit 2 (never guess a security switch).
- **R1-2 (F2) Messages naming forbidden Tailscale words.** `setup_exposure.sh hint <reason>`
  prints every message containing `tailscale up`, `tailscale status`, `tailscale set`, `--http`
  or `funnel --https=443 off`; `setup.sh` calls it and never contains those substrings (SET20
  stays green unchanged). Reasons are an enum (`down`, `logged_out`, `missing`, `funnel_off`,
  `no_https`, `no_funnel_cap`, `foreign_handler`, `stuck`, `unverified`).
- **R1-3 (F3) Printed-only never fails; harnesses listed.** Missing companion tools (also with
  `--skip-packages`) ⇒ `printed:` (§5.1 wins over §5.2); `printed:` runs keep today's exit codes;
  M12 (Docker SET34 job) and M13 (lock-screen harness) added as category S.
- **R1-4 (F4) Scoped tool table.** Declaration form (keeps SET2 green: keys are command names
  inside `[...]=`, the value is a scope string, no bare package word list):
  `declare -rA REQUIRED_TOOLS=([jq]=companion [dig]=companion [curl]=companion [unzip]=android
  [java]=android [pacman]=family:arch [apt-get]=family:debian [dpkg-query]=family:debian
  [dnf]=family:fedora [rpm]=family:fedora [sudo]=escalate [doas]=escalate [runuser]=root-start
  [gnome-extensions]=optional [systemctl]=base …)`. Scopes: `base` (always checked),
  `family:<f>` (checked for the detected family), `escalate` (at least one of the alternatives),
  `root-start`, `companion`, `android` (checked when that group is requested), `optional` (never
  checked; the call site is guarded by `command -v`). SDP-S3 checks that every external command
  word has an entry; preflight checks only active scopes.
- **R1-5 (F5) SDP-S2 precise form.** No package token of `_COMPANION`/`_ANDROID` appears in the
  three scripts **except** as a `REQUIRED_TOOLS` key or as a command word that is itself a
  `REQUIRED_TOOLS` key (`jq`, `curl`, `unzip`, `dig`).
- **R1-6 (F6) Probes run as the user, always.** The root half runs `runuser -u <user> -- bash
  setup_exposure.sh probe …` (environment cleared to the existing allow-list); no `curl`/`dig`
  ever runs as uid 0. STX-B6 and M7 agree (uid 1000).
- **R1-7 (F7) Exposure before the companion phase.** S-4 rewritten in place (§9): what is
  guaranteed is that only the companion socket is reachable and `soos-remote` enforces its own
  `allow_funnel`; no root rollback exists after exit 5, the off command is printed and the next
  run reconciles. Ordering the exposure after P6 would need a third escalation (owner: "sudo
  once") — rejected.
- **R1-8 (F8) Identity precedence** fixed in place (§7.1): flags → managed file → detection →
  user file; `login=` output captured in memory. Test STX-B12: managed file with login A,
  detection login B, `--yes` → A kept, `[WARN]` printed, B never printed.
- **R1-9 (F9) `[android]` tables.**
  - A user file whose trailing `[android]` table passes the strict grammar (§6.3) **is carried
    over** into the managed file (the owner's current hand-written file keeps its binding).
  - `[android]` present (file or `--android`) while the resolved `allow_funnel` is `false`
    (`--funnel off` or a kept explicit `false`) → preflight **exit 2**, nothing changed, message:
    "the Android app needs Funnel: remove the [android] table or keep Funnel on" (same rule as
    soos-gui GAK3; `soos-remote` would exit 78 otherwise).
  - Tests SDP-B12 (carry-over, byte-identical block), SDP-B13 (`--funnel off` + block → exit 2).
- **R1-10 (F10) Spike before PR B (new PR B0, read-only, owner host, no repository code).** The
  owner (or an agent with the owner's explicit go-ahead for each command; agents never run
  `tailscale serve`/`funnel`, ADR 2026-10-06) records in `AI/research_funnel.md`:
  (a) `dig +norecurse @<each ts.net NS> <host> A` — answer or referral; the helper follows **at
  most one** referral (NS records of a delegated child zone, then the same query there);
  (b) hairpin: `curl --resolve <host>:443:<ingress> https://<host>/` from the node itself with
  Funnel on — status line or not; (c) `tailscale serve status --json` and `status --json` as the
  non-root user; (d) the exact 1.102.4 spellings of `serve|funnel --bg [--yes]` and of turning
  Funnel off **without** removing the serve handler (if `funnel --https=443 off` removes the
  handler, the helper re-runs `serve --bg …` and re-reads; m12); (e) the `Self.CapMap` key names
  (redacted fixture). Fallbacks fixed now: (c) denied for the user ⇒ the user half treats
  `handler` as `unknown`, the root half (which can read it) decides, and P7 relies on the probe
  only; (b) no hairpin ⇒ results are `unreachable` ⇒ `unverified` (exit 6 with guidance, never
  a down/up) and the ADR records that automatic recovery is unavailable on such hosts; (a)
  unanswerable ⇒ `pending` ⇒ `unverified`. PR B starts only after B0 is recorded.
- **R1-11 (F11) Release build covered by CI.** `android_build.sh`: new
  `readonly RELEASE_TASKS=(assembleRelease)`; `run_strict` and `run_bootstrap` run
  `"${TASKS[@]}" "${RELEASE_TASKS[@]}"` (so `bootstrap` records the release artifacts and `ci`
  builds release on every Android-touching PR); `check_permissions` runs on both APKs; the new
  `release` mode runs `RELEASE_TASKS` only with strict verification. ANA-S10 needles
  (`readonly TASKS=(assembleDebug testDebugUnitTest lintDebug)`, the `ci)` bootstrap-exits-1
  structure, no `signingConfigs`) stay byte-identical; the verification-metadata diff is
  reviewed in the candid review.
- **Minor items.** m1 fixed in §8.3. m2/m3 fixed in M3/M5. m4: the troubleshooting section is
  `§2m`, after the existing `## 2k. Feature tour`; PR C keeps every ANA §2j needle (debuggable,
  debug.keystore, keytool, apksigner, rotation, accepted risk, …). m5: ids are SDP/STX/SAN only
  (DEP3 = SDP-S3, EXP1 = STX-S1). m6: M14 (shared lexer). m7: DEP6 runs on the pinned
  `fedora:40` image **and** a newly pinned current Fedora image; a name that does not resolve
  fails the job; Arch uses `bind` (dig; no service is enabled) unless `ldns`/`drill` is chosen in
  PR A with the probe adapted; the Fedora udev package is dropped if `android-tools` would put a
  second `adb` first on `PATH` (setup calls the SDK `adb` by absolute path either way) and the
  `no permissions` hint covers it; Debian/Ubuntu `android-sdk-platform-tools-common` needs
  `universe` — when it is not available, preflight reports it and continues without udev rules
  (hint), never adds a repository. m8 fixed in §8.0. m9: every wait loop is bounded by an
  iteration count derived from the constants (window / interval), and the tests replace `sleep`
  with a no-op stub so they stay fast; the bounds themselves are pinned statically. m10: stated in
  §4 (two prompts remain possible on a first run that installs packages). m11: the owner asked
  for a checksum-verified SDK, so `android/sdk-pins.properties` also pins the SHA-256 of the
  installed files that the build and signing trust (`platforms/android-36/android.jar`,
  `build-tools/35.0.0/aapt2`, `…/zipalign`, `…/lib/apksigner.jar`, `platform-tools/adb`), checked
  after every `sdkmanager` install and on every run before use; mismatch → exit 7, component
  directory left for inspection, message names the file (values filled from a clean install by
  the implementer, re-pinned only with the pin file). m12: in R1-10 (d). m13: a re-run whose
  handler is ours stops the probe window at the first `answered` (normally the first probe); a
  stuck state first seen at P7 is reported with "re-run ./scripts/setup.sh" (the next root half
  recovers). m14: stuck requires the span rule of `STUCK_SPAN_S` (§3, R2-3); a lying resolver can
  at worst cause one bounded down/up per run (residual recorded in the ADR).
- **PR split update**: 0 → A → B0 (spike, documentation only) → B → C.

## 18. Revision 2 (answers the re-evaluation; authoritative over §1–§17 where they differ)

- **R2-1 (N1, MAJOR) Tolerant, fail-safe boolean parsing of a user file.** For a user file only,
  the seven `COMPANION_KEYS` are read at top level (lines before the first `[table]` header)
  with `^[[:space:]]*<key>[[:space:]]*=[[:space:]]*(true|false)[[:space:]]*(#.*)?$`. A key that
  is present but whose value this rule does not recognize is treated as **`false`** (kept off;
  the plan line names the key, never the value) — never default-on, never exit 2. Keys below a
  table header (any line matching `^[[:space:]]*\[`) are ignored; SDP-B11 also covers a spaced
  header such as `[ android ]`. The R1-1 sentence about "unparsable → default-on / exit 2" is
  withdrawn. SDP-B11 cases: `allow_funnel=false`, `allow_funnel = false # c`, an unrecognized
  value → off, the same key below `[android]` ignored.
- **R2-2 (N2, MAJOR) File pins only for fixed revisions.** `android/sdk-pins.properties` pins
  the SHA-256 of `build-tools/35.0.0/aapt2`, `…/zipalign` and `…/lib/apksigner.jar` (fixed
  revision), plus the cmdline-tools archive. `platforms;android-36` and `platform-tools` (always
  the latest revision in `sdkmanager`) are **not** file-pinned: documented residual (§8.1),
  owner question Q5. The m11 sentence of §17 that pins `android.jar` and `adb` is withdrawn.
- **R2-3 (n3) Stuck rule.** Stuck ⇔ every probe result of the last ≥ 60 s (≥ 12 probes at the
  5 s interval) is `tls_eof` and no result in the window is `answered`. `STUCK_CONSECUTIVE` is
  replaced by `STUCK_SPAN_S = 60`.
- **R2-4 (n4)** Stale text aligned (§5.2, §8.1).
- **R2-5 (n5) SDP-S3 exemptions and scopes.** Exempt: installed soos binaries through
  script-assigned absolute paths (`ADMIN_BIN`, `ENROLL_BIN`, `${BIN_DIR}/soos-remote`, …);
  variable command words are resolved through their SCN7 assignment values (`ESCALATE` → `sudo`
  / `doas`, `KEYTOOL`, `APKSIGNER`, `SDKMANAGER`, `ADB` → absolute SDK/JDK paths). New scope
  `toolchain` for `cargo`/`rustc`, checked after P4 (rustup installs them).
- **R2-6 (n6)** STX-S2 `dig` allowlist also contains exactly `+norecurse +noall +authority
  @<ns> <host> A` (to read one delegation) and `+short @<ns> <child-ns> A` / the same `<host> A`
  query against the child servers; nothing else.
- **R2-7 (n7) Spike B0 split by authority.** Owner-only (Tailscale writes, ADR 2026-10-06):
  (b) Funnel on + hairpin `curl`, (d) turning Funnel off and the `--yes` spelling when it must be
  tried. An agent may run only the read-only items: `tailscale serve --help`/`funnel --help`
  output, (a) `dig` queries, (c) `status --json` / `serve status --json` as the user, (e) the
  redacted `CapMap` key names.
- **R2-8 (n8)** New STX-B13 (PR B, running stub): user file with `allow_funnel = false` → only
  `serve --bg` logged, never `funnel --bg`.
- **R2-9 (n9) User-file identity before detection.** Precedence: flags → managed file → user
  file → detection; any disagreement with detection prints the R1-8 `[WARN]` (login never
  printed).
- **R2-10 (n10)** Every key absent from a user file and written `true` by the default-on profile
  is named in the plan line and in the `[DONE] companion config` line ("turned on by setup's
  default profile: allow_unlock, push_notifications, …"; switchable in soos-gui).
- **R2-11 (n11)** A user `[android]` block that fails the strict grammar is not carried over;
  the summary says "your [android] table was not carried over (see the backup); the Android app
  cannot sign in until you re-run with --android". Such a block does **not** trigger the R1-9
  exit-2 rule (only a carried-over or newly requested binding does).
- **R2-12 (n12)** `--android` with `uname -m` ≠ `x86_64` → preflight exit 2 ("the Android SDK
  command-line build tools are published for x86_64 Linux only").
- **Open question added**: **Q5** — pin `platform-tools` and `platforms;android-36` too
  (download exact revisions from Google's repository manifest by URL + SHA-256, re-pinned on each
  update), or accept them as a residual (spec default)?
