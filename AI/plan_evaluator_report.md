# Plan Evaluation Report — Re-evaluation (Revision 2)
- **Date**: 2026-10-23
- **Issue**: GitHub #362 owner decisions 2026-10-23 — one command, no manual step (spec `AI/architect_spec_one_command_setup.md`, §18 "Revision 2" authoritative over §1–§17; no `AI/BACKLOG.md` id)
- **Branch**: `chore/one-command-setup-spec`
- **Base commit**: `e7826de`

Re-read: §18 (R2-1..R2-12, Q5), the updated §5.2 and §8.1, and every section §18 overrides,
against the code already verified in the two previous rounds (`classify_remote_config`
`scripts/setup.sh:1386-1460`, the setup harnesses, `android_build.sh`, ANA-S10,
`crates/remote/src/config.rs`). Nothing executed besides read-only `grep`/`sed`.

### Resolution of every earlier finding

| Id | Finding | Resolution | Status |
|---|---|---|---|
| F1 (CRIT) | user-file explicit `false` overridden | R1-1 + R2-1, M1/M2 vectors, SDP-B11, STX-B13 | Resolved |
| F2 | SET20 bans `tailscale up` even quoted | R1-2 `setup_exposure.sh hint` | Resolved |
| F3 | harnesses, `--skip-packages`, unconditional exit 6 | R1-3, M12, M13, §7.7 | Resolved |
| F4 | unscoped `REQUIRED_TOOLS` | R1-4 + R2-5 | Resolved |
| F5 | SDP-S2 unsatisfiable | R1-5 | Resolved |
| F6 | probe as root | R1-6 | Resolved |
| F7 | exposure before config, S-4 | R1-7, S-4 | Resolved (residual recorded) |
| F8 | detection over managed file | R1-8, STX-B12 | Resolved |
| F9 | `[android]` dropped / `--funnel off` | R1-9, R2-11 | Resolved |
| F10 | unverified Tailscale/DNS assumptions | R1-10, R2-6, R2-7 | Resolved (spike B0 before PR B) |
| F11 | release path untested | R1-11 | Resolved |
| m1–m10, m12, m13 | (first round) | §17 minor items | Resolved |
| m11 | SDK components unpinned | R2-2 + Q5 | Resolved (fixed revisions pinned; latest-only ids are an owner question) |
| m14 | false `tls_eof` | R2-3 | Resolved |
| N1 | strict grammar on hand-written files | R2-1 tolerant, top-level, unknown value → `false` | Resolved |
| N2 | pins of always-latest components | R2-2 | Resolved |
| n3 | span rule contradiction | R2-3 `STUCK_SPAN_S = 60` | Resolved (stale constant text, r1) |
| n4 | stale §5.2/§8.1 | R2-4 | Resolved for those two; other stale spots (r1–r3) |
| n5 | SDP-S3 exemptions | R2-5 | Resolved |
| n6 | `dig` allowlist for referrals | R2-6 | Resolved |
| n7 | spike authority | R2-7 | Resolved |
| n8 | SDP-B11 power | R2-8 STX-B13 | Resolved |
| n9 | user-file identity order | R2-9 | Resolved (§6.1 text stale, r2) |
| n10 | absent keys turned on silently | R2-10 | Resolved |
| n11 | non-strict user `[android]` | R2-11 | Resolved |
| n12 | non-x86_64 | R2-12 | Resolved |

## 1. Coverage Matrix

| Owner decision / pain point (2026-10-23) | Spec element | Status |
|---|---|---|
| 1. setup runs `serve`/`funnel` (sudo once) | §7.2, §7.3, §4 | Covered (two prompts possible on a first run, stated) |
| 1. verify from outside | §7.4, §7.6, R1-10, R2-6 | Covered after spike B0 |
| 1. stuck Funnel handled automatically | §7.5, R2-3, R1-10 (b) | Covered where the hairpin works; else "unverified" exit 6, recorded |
| 2. `--android`: JDK 17, SDK in home, licenses, components | §5.1, §8.0–§8.3, R2-12 | Covered |
| 2. pinned, checksum-verified SDK | §8.1, R2-2, Q5 | Covered for fixed revisions; owner decides the two always-latest ids |
| 2. own key, one prompt; build, sign, fingerprint | §8.4, §8.5, R1-11 | Covered |
| 2. `[android]`, restart, assetlinks, `adb install` | §6.3, §8.6, §8.7, R1-9, R2-11 | Covered |
| 3. back up user `remote.toml`, continue | §6.1, R1-1, R2-1, R2-9, R2-10 | Covered |
| 4. push sender system unit | other PR | Out of scope |
| Pain points (packages, SDK, keystore, `SHA 256:`, `[android]`, `adb`, Chrome cache) | §5, §8, §10 | Covered |
| Every changed existing assertion listed | M1–M14 | Covered; owner approval (Q4) still pending |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| Managed grammar stays strict; R2-1 applies only to `user` files | `setup.sh:1409` | strict regex | Yes (no change to managed classification) |
| `soos-remote` rejects `[android]` without `allow_funnel` | `config.rs:518` | exit 78 | Yes; R2-1 (unknown → `false`) + R1-9 cannot produce a file that `soos-remote` accepts today with a binding and Funnel off |
| ANA-S10 needles, no `signingConfigs` | `android_native_contract.rs:249,770-797` | unchanged by R1-11/R2-2 | Yes |
| `build-tools;35.0.0` is a fixed revision; `platforms;android-36`, `platform-tools` follow the latest revision | §8.1 ids, `android.yml:60` | as stated | R2-2 correct |
| `STUCK_CONSECUTIVE` still in the §3 table and §7.4 | spec lines 135, 436 | stale | r1 |
| §6.1 item 1 still says "flags → auto-detection → user file" | spec line 260 | stale vs R2-9 | r2 |
| §8.1 pin file example has no build-tools file-hash keys | spec lines 519-528 | absent | r3 |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: hand-written `allow_funnel = false # tailnet only`, `allow_funnel=false`, `allow_funnel = "false"`. R2-1 reads the first two as `false` and treats the third (unrecognized) as `false`: Funnel never turns on against a written choice, and setup never refuses the file. PASS.
- Failure scenario considered: the same key below `[android]` is ignored, so a nested key cannot switch a top-level feature on. PASS; how a table header line is recognized is not spelled out (r4).
- Failure scenario considered: a companion failure after exposure; residual accepted and recorded (R1-7). PASS.

### Pillar 2 — PAM deadline & concurrency
- Not touched (S-14); every wait is count-bounded. PASS.

### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered: first-certificate issuance producing `tls_eof` for 30 s → R2-3 needs ≥ 60 s of uninterrupted `tls_eof` with no `answered` in the window, so no down/up. A lying resolver can still cause one bounded down/up (recorded residual). PASS.
- Failure scenario considered: unrecognized user-file boolean → `false` (fail-safe), never default-on. PASS.

### Pillar 4 — Dependencies
- Failure scenario considered: Google publishes a new `platform-tools` → no file pin on it any more (R2-2), so fresh installs keep working; tamper protection for it rests on Google's manifest (residual, Q5). PASS.
- `aarch64` host with `--android` → exit 2 up front (R2-12). PASS.

### Pillar 5 — Data confidentiality
- Plan lines name keys, never values (R2-1, R2-10); the login is never printed in the R2-9 `[WARN]`. PASS.

### Pillar 6 — Test integrity
- Failure scenario considered: an implementation that ignores the user file's `allow_funnel = false` → M1 vector and STX-B13 (running stub, only `serve --bg`) both fail. Tests have power. PASS.
- The migrations M1–M7 and M12–M13 still need the owner's explicit approval (Q4) before the tester phase starts; that is a process gate, not a plan defect.

## 4. Findings

- **r1 [MINOR]** `STUCK_CONSECUTIVE` is still in the §3 constants table and in §7.4 step 5, although R2-3 replaces it with `STUCK_SPAN_S = 60`; STX-S3 pins constants. — Replace both in place so the tester pins `readonly STUCK_SPAN_S=60`.
- **r2 [MINOR]** §6.1 item 1 still gives the identity order "flags → auto-detection → user file", contradicting R2-9 (flags → managed → user file → detection). — Edit in place.
- **r3 [MINOR]** The §8.1 pin-file example and SAN-S1 do not show the R2-2 keys for the build-tools file hashes (`aapt2`, `zipalign`, `lib/apksigner.jar`). — Add the key names and the `^[0-9a-f]{64}$` rule so SAN-S1 can pin them.
- **r4 [MINOR]** R2-1 says "lines before the first `[table]` header" without the exact rule. — Define a header as any line matching `^[[:space:]]*\[` (covers `[ android ]` and `[[x]]`) and add one SDP-B11 case with a spaced header.

No CRITICAL or MAJOR finding remains. Owner questions Q1–Q5 (and Q4's approval of the test migrations) must be answered before Phase 2; spike B0 must be recorded before PR B.

## 5. Verdict
VALIDATION_VERDICT: APPROVED

## 6. Post-verdict note

The architect applied the four MINOR text fixes r1 to r4 in place (`STUCK_SPAN_S` in §3 and §7.4, the §6.1 identity order, the build-tools hash keys in §8.1 and SAN-S1, the table-header rule in R2-1). None of them changes the verdict.

---

# Round 3 — §19 "Owner decisions of 2026-10-23" against `main` after #473 (2026-10-23)
- **Branch**: `feat/setup-preflight-deps` (PR A of §13)
- **Base commit**: `d5e3c09` (`origin/main`, push sender system service merged)
- **Scope**: §19 was not re-evaluated after the owner's answers; this round checks it, and the
  parts of §5/§6/§12 that PR A implements, against the code on `main` and against the package
  repositories of the pinned images (read-only `docker run` of `ubuntu:24.04`, `fedora:40`,
  `fedora:44` and `archlinux` with `apt-cache show`, `dnf repoquery --available`, `pacman -Si`).

## 1. Facts verified

| Fact cited by the spec | Where | Actual | Match |
|---|---|---|---|
| `UNIT_START_TIMEOUT_S` bounds `systemctl enable --now tailscaled.service` (§19 OD-Q1) | `scripts/setup.sh:68` | `readonly UNIT_START_TIMEOUT_S=90` | Yes |
| `setup.sh` holds no `curl`/`wget` and no `tailscale up`/`status` (S-1, SET3, SET20) | `scripts/setup.sh`; `setup_contract.rs`, `setup_user_phase_contract.rs` | true; §19 keeps every new call in `setup_exposure.sh` | Yes |
| A user `remote.toml` is refused with options and kept without (AS-U3) | `scripts/setup.sh:1671-1681` | exit 2 with options; `CONFIG_ACTION=kept` otherwise | Yes (M1/M2 encode the change) |
| `classify_remote_config` reads the size first | `scripts/setup.sh:1431-1435` | `> MAX_REMOTE_CONFIG_BYTES` → `user` without parsing | Yes (F4) |
| `[android]` needs `allow_funnel` and at most 2 upper-case fingerprints | `crates/remote/src/config.rs:509-540`, `crates/remote/src/lib.rs:291` | as stated | Yes |
| `applicationId = "org.soos.remote"` | `android/app/build.gradle.kts:17` | as stated | Yes |
| §5.1 companion names | images | `jq bind9-dnsutils curl ca-certificates` (ubuntu), `jq bind-utils curl ca-certificates` (fedora 40, 44), `jq bind curl ca-certificates` (arch) all resolve | Yes |
| §5.1 android names | images | ubuntu `openjdk-17-jdk-headless unzip android-sdk-platform-tools-common` resolve (universe is enabled in the image); arch `jdk17-openjdk unzip android-udev` resolve; fedora 40 `java-17-openjdk-devel` resolves, **fedora 44 has no `java-17`/`java-21` package** (only `java-25-openjdk`, `java-latest-openjdk` 26/27) | **No** (F1) |
| §19 `tailscale` group | arch image | `tailscale` resolves in `extra`; `gnupg` (ubuntu) and `gnupg2` (fedora) resolve | Yes (PR B) |
| The SCN lexer can be reused by SDP-S3 | `tests/invariants/src/setup_scanner_contract.rs` | private items; M14 is assigned to PR B | Partly (F6) |
| The default harness `tailscale` stub "fails" `status` (STX-B16) | `setup_user_phase_contract.rs:338-341` | exits 0 with no output for anything but `version` | **No** (F8, PR B) |
| `system_up_to_date` and the push sender drop-in (#473 review) | `scripts/setup.sh:1006-1015`; PSS16 | no drop-in compared; PSS16 models a fake-prefix install without the drop-in | F10 |

## 2. Findings

- **F1 [MAJOR]** No JDK package name resolves on both `fedora:40` and the current `fedora:44`
  (pinned by digest for the package matrix, m7), and Gradle 8.14.5 does not run on the only
  Fedora 44 JDK (25). §5.1 as written makes the required Docker job fail. — PR A: the Fedora
  `android` group is `unzip android-tools`; the Fedora JDK becomes an owner question for PR C;
  PR C refuses `--android` on fedora in preflight until decided (OD-T4 kept). ADR record.
- **F2 [MINOR]** The §19 `tailscale` group is PR B work; its debian/fedora package comes from
  Tailscale's repository, absent from the matrix images. — Group lists `gnupg`/`gnupg2` there;
  the repository package name is an exempted constant of `setup_exposure.sh`.
- **F3 [MINOR]** §6.1 item 1 ("companion skipped") contradicts the approved M2 ("kept"). — M2
  wins; spec text fixed.
- **F4 [MINOR]** An oversized user file would fail with exit 5 after a plan that promised a
  backup. — Preflight exit 2 when a backup would be needed; spec text fixed; SDP13.
- **F5 [MINOR]** SDP-B1 (android half) and SDP-B3 need `--android` (PR C). — Moved; SDP-B2's
  fatal path is tested on the `toolchain` scope in PR A.
- **F6 [MINOR]** SDP-S3 needs the lexer before M14. — Visibility only (`pub(crate)`), SCN bodies
  byte-identical; category S.
- **F7 [MINOR]** R2-11 names `--android` before it exists. — PR A wording points to §2j.
- **F8 [MINOR, PR B]** Empty `status --json` output with exit 0 must be "unavailable".
- **F9 [MINOR, PR B]** The post-E2 identity question is the one late prompt; acceptable.
- **F10 [MINOR, scope]** The #473 follow-up (2) needs a setup-only change of PSS16 (M17,
  category S): the lead decides; no assertion text changes.

No CRITICAL finding. All fixes are applied in the spec as §20 (and in place in §5.1 and §6.1).

## 3. Verdict
VALIDATION_VERDICT: APPROVED
