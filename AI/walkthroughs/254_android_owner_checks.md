# Walkthrough 254 — Android Owner Checks of 2026-10-23 and Troubleshooting

- **Date**: 2026-10-23
- **Issue**: owner hardware validation of #357 / #354 / #359 / #363 (GitHub #296 checklist); no code
  change
- **Branch**: `chore/android-owner-checks`
- **Decision**: none new; the owner decisions taken during the test (one-command setup, push sender
  as a system service) are recorded on GitHub #362 and handled by their own pull requests.
- **Matrix criteria**: ANA14, ANA15, AAF4, ATR9 verified by the owner; ANA18, ANA19, ANA22,
  ANA23, ANA24, AAF1, AHB4, AHB5, AHB6, ATR1 verified by CI; RAN12, RDB10, ANA26 partially
  verified (still pending); ANA16, ANA21, AHB8 not tested yet.

## 1. Context

The owner installed soos from scratch on an Ubuntu 24.04 laptop (GNOME, Tailscale 1.104.1) with
`scripts/setup.sh`, then tested the phone web app and the native Android app on an Android phone,
mostly over Funnel with the Tailscale app turned off (the owner does not want to need it).

## 2. Results

| Row | Result |
|---|---|
| RAN12 | Parts 1–2 OK: Chrome *Install app*, passkey enrolment with `soos-remote enroll-code`, sign-in over Funnel. Push parts not tested (push off, see §3). |
| RDB10 | Android OK: unlock and camera start went straight to the biometric prompt over Funnel. iPhone part not tested. |
| ATR9, ANA14 | OK: the tour opened once and stayed closed; the second sign-in went straight to the fingerprint prompt. |
| ANA15 | OK: icon, themed icon and blue header in light and dark mode. |
| AAF4 | OK: "Acknowledged." disappears on refresh. |
| ANA26 | Battery card OK (follows the charger). Offline history card shows "off" because `offline_alerts = false`. |
| ANA16 | Not applicable to this owner setup (no Tailscale app on the phone); still pending. |
| ANA21, AHB8 | Not tested yet (AHB8 needs a 30 min outage). |

The web app's *Acknowledge* first looked broken; it worked once Funnel was repaired (§3).

The CI-only rows (JVM unit tests of the Android app) cite the green `Android (debug build)` run
39622619945 on `main` (`fb2a0bf`), whose log shows `:app:testDebugUnitTest`.

## 3. Problems Met and Where They Are Handled

- **Push sender `226/NAMESPACE` on Ubuntu 24.04** (unprivileged user namespaces restricted by
  AppArmor): workaround `--push off --offline-alerts off`; fix in progress (system service).
- **Funnel unreachable from outside** (TLS EOF at the ingress) although configured and badged:
  fixed by `tailscale down` / `tailscale up`.
- **Chrome kept the tailnet address** after the VPN was turned off: clear the host cache.
- **Android build**: no SDK / `ANDROID_HOME`; French `keytool` prints `SHA 256:`.
- **Pre-existing `remote.toml`**: setup refused it; the owner now wants a backup and continue.

All are added to `Docs/REMOTE_COMPANION.md` §7; the one-command setup (GitHub #362, decisions
of 2026-10-23) will remove the manual steps.

## 4. Verification

`cargo test --locked -p soos-invariants` (matrix citations and structure) passes.
