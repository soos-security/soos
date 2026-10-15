# Plan Evaluation Report — Round 2
- **Date**: 2026-10-13
- **Issue**: GitHub #382 — native Android app foundation (PR 1 of #357)
- **Branch**: `feat/android-native-app`
- **Base commit**: `6f32ef8`
- **Spec evaluated**: `AI/architect_spec_android_native_app.md` (untracked, 339 lines: §0–§8, plus "## 9. Revision 1" and "## 10. Test plan", A-5 revised)
- **Also glanced at**: the stub-phase drafts in `crates/remote/src/*.rs`, `crates/remote/tests/android_app_tests.rs` and `android/` (for consistency only; the plan is what is evaluated)

GitHub-only issue (no `AI/BACKLOG.md` id, no existing matrix rows). The coverage matrix uses the
scope lines of #382 and the owner decisions on #357 / #354 as acceptance lines.

## 0. Round-1 resolution

| Round-1 finding | Revision 1 resolution | Status |
|---|---|---|
| M1 bootstrap passes green | Bootstrap run exits 1 with `::error::`; PR/push runs strict only; owner commits the artifact of the PR's own run before merge; invariant `test_ana_verification_metadata_covers_the_catalog` | **Resolved** (procedure gaps n1–n4, MINOR) |
| M2 §9 needle | §9 keeps "native Android app"; no existing test modified | **Resolved** (other §9 pins not listed, n5) |
| M3 signing key as trust anchor | Accepted risk in ADR + §2j; dedicated key; fingerprint reading; rotation; off switch; ANA14 | **Resolved** |
| M4 owner merge approval | Run rule: agents never commit/push/merge; A-1..A-5 listed for the owner. Also enforced structurally: the PR cannot go green before the owner commits the metadata (M1) | **Resolved** |
| m1 `handle_all_urls` | A-5 single relation; ANA14 decides any extension | Resolved |
| m2 inert table without Funnel | `AndroidRequiresFunnel` after `AndroidRequiresRpId` | Resolved (precedence checked, §2) |
| m3 curl redirect | `--location --proto-redir '=https' --max-redirs 5` | Resolved |
| m4 conditional `aapt2` | Mandatory in `ci` mode | Resolved |
| m5 job name / gate | Renamed, concurrency group, never advisory | Resolved |
| m6 `org.json` coercion | Explicit type checks | Resolved |
| m7 test assertions | §10 list | Mostly resolved (n6) |
| m8 TOML placement | Documented | Resolved |
| m9 keystore names | `secret_scan.sh` + `.gitignore` | Resolved (no existing test pins the scan list, §2) |
| m10 Gradle licences | ADR list | Resolved (fact to correct, n7) |
| m11 logging / debuggable | Forbidden list + docs | Resolved |
| m12 third-party providers | Google Password Manager first, ANA14 records provider | Resolved |
| m13 CI toolchain trust | ADR + `sdkmanager --licenses` | Resolved |

## 1. Coverage Matrix

| Acceptance line (#382 / owner decision) | Spec element | Status |
|---|---|---|
| `android/` Gradle project, Kotlin + Compose, outside Cargo workspace | §1.2, §2.2 | Covered |
| Exact version catalog, Gradle distribution SHA-256 | §3.3, `gradle-wrapper.properties`, `scripts/android_build.sh`, §10 invariants | Covered |
| Gradle dependency verification metadata | §9 M1: committed and reviewed inside PR 1 before merge; catalog-coverage invariant | Covered (n1–n4) |
| No analytics / Firebase / Play services | A-2, §6, §10 invariants | Covered |
| Brand shell, repository palette, adaptive icon from `icon.svg` | §6, §10 (`colors.xml` = `theme.rs`, path data = `icon.svg`) | Covered |
| Settings screen, PC URL validated fail closed | `PcUrl`, `PcUrlTest` | Covered |
| Funnel sign-in via Credential Manager, remembered `allowCredentials`, no credential named to anonymous clients | `PasskeyJson`, `PasskeySignIn`, `SettingsStore`, OD-3 | Covered |
| Read-only status and lock | `SoosApi`, home screen, ANA16 | Covered |
| `[android]` serves `assetlinks.json` with one `get_login_creds` statement | A-5 revised, §3.1 `ASSET_LINKS_RELATION`, §4.2 | Covered |
| App origin accepted for login only | `OriginPolicy`, `check_assertion` purpose switch, §10 | Covered (CameraView negative missing, n6) |
| Without the table: `404`, web origin only | §4.1, §10 byte-identical 404 | Covered |
| Path-filtered CI job, SHA-pinned, pinned JDK/Gradle/SDK, no secrets, debug build + unit tests | §5 + §9 m3/m4/m5/m13 | Covered (n2) |
| Invariants for `android/` | §10 invariant list | Covered |
| ADR, matrix rows incl. owner hardware checks, docs section, walkthrough 208 | §8, §10 ANA1–ANA16 (ANA13–16 owner rows) | Covered |
| Owner #354: merge only after explicit owner approval | §9 M4 | Covered |
| Owner #357: web app stays, sideload only, publisher `soos-security` | OD-1, OD-2 | Covered |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `allow_funnel` already requires `rp_id` | `config.rs:281-282, 829-831` | `FunnelRequiresRpId` checked before any table | ✓ — `AndroidRequiresRpId` stays reachable (table, no `rp_id`, no Funnel); `AndroidRequiresFunnel` only fires with `rp_id` and Funnel off; no overlap with existing errors |
| `AndroidRequiresFunnel` interplay with existing config tests | grep `crates/remote/tests` | no existing test writes an `[android]` table; draft `test_ana_config_android_requires_funnel` covers implicit and explicit `false` | ✓ |
| Camera precedent for a "requires allow_funnel" error | `config.rs:326-328` `CameraFunnelRequiresFunnel` | same text shape | ✓ |
| `secret_scan.sh` filename list | `scripts/secret_scan.sh:144` | `*.pem\|*.key\|*.p12\|*.pfx\|*.jks\|...` | ✓ adding `*.keystore`, `keystore.properties`, `local.properties`, `google-services.json` breaks nothing |
| Tests pinning `secret_scan.sh` | grep `tests/`, `crates/`, `scripts/` | none (only `.githooks/*` and `ci.yml:124,127` call it) | ✓ |
| No tracked file would newly match | `git ls-files` for those names | none | ✓ |
| Content patterns vs a large `verification-metadata.xml` (hundreds of 64-hex digests) | `secret_scan.sh:82-89` | PEM, GitHub, AWS, Slack, `sk-` only | ✓ no false positive |
| `Docs/REMOTE_COMPANION.md` §9 pins | `remote_android_contract.rs:1240` (`native Android app`, `notify.windows.com`, `Tasker`); `remote_camera_contract.rs:902-908` (a line with `live camera` and `recording`); `remote_cross_browser_contract.rs:1413-1414` (`section 2i`); `remote_push_contract.rs:788-800` (no `- **push notification` line) | five pins | Spec names only the first (n5) |
| Section order for a new `## 2j.` | `REMOTE_COMPANION.md:889` `## 2i.`, `:966` `## 3.` | `2j` between them keeps RAN/RXB order checks | ✓ |
| Pre-push hook binds a candid review to the pushed diff | `.githooks/pre-push:9-10, 38-39` | push blocked without a fresh APPROVED report for `local_sha` | Owner's metadata commit needs a new review (n1) |
| Gate on failing non-advisory checks | `scripts/pr_checks_gate.sh:15-19, 28` | one advisory check (macOS) | ✓ red bootstrap run blocks `pr_loop.sh` |
| Catalog coordinates the coverage invariant must find | `android/gradle/libs.versions.toml` (draft) | `androidx-compose-ui` and `material3` have **no version** (BOM-managed); 3 plugin ids | Invariant must match BOM-managed entries by `group:name` (n4) |
| Draft metadata header text | `android/gradle/verification-metadata.xml` comment | still says "flags the run with a warning" | Stale vs Revision 1 (n8) |
| Walkthrough number 208 | `AI/walkthroughs` max = 199; #382 body says 208 | no contiguity invariant found | ✓ (owner's numbering) |
| `org.json` licence | ADR plan says "JSON License" | JSON-java has been released as Public Domain since 2023 (verify in the 20250517 POM) | Likely wrong (n7) |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Scenario (M3 revisited): an APK signed with a listed key on the owner's phone obtains a Funnel session. With that session it can request unlock/camera **options** (`server.rs:1439, 1485, 1508` gate on Funnel + session), but the verify step goes through `check_assertion` with `WebOnly` for those purposes, and a non-privileged app cannot make Credential Manager emit an `https://` origin. Risk bounded as the revision records. PASS.
- Scenario: `[android]` with `rp_id`, Funnel off → now refused at start-up (`AndroidRequiresFunnel`). `ServerState::with_android_app` used directly with a Funnel-off config still serves the route on the tailnet; harmless (public data), config is the only production path. PASS.
- RC-1 unchanged (no new socket, no new crate). PASS.

### Pillar 2 — Deadlines & concurrency
- No `crates/pam` / `crates/daemon` change. The asset-links route uses the existing anonymous Funnel permit; body prebuilt. PASS (capacity test not listed, n6).

### Pillar 3 — Fail closed
- Scenario: the first strict run after the owner's commit fails because an artifact resolved only in strict mode (or a platform-classified `aapt2` for another OS) is missing from the TOFU file. The job is red (fail closed), but the plan has no documented recovery: `ci` mode bootstraps only when the file has **zero** components, so a partial file can only be repaired by hand or by emptying the list again. Safe, but undocumented (n3, MINOR).
- Scenario: someone adds a `<trusted-artifacts>` regex such as `.*`, or `org.gradle.dependency.verification=lenient` in `gradle.properties`. The CLI flag `--dependency-verification strict` overrides the property, but a wildcard trusted-artifact silently disables checks while the catalog-coverage invariant still passes (n4, MINOR).
- Config errors: value-free texts, run after existing checks; precedence verified in §2. PASS.

### Pillar 4 — Dependencies / supply chain
- Bootstrap feasibility: (1) the owner pushes the branch (pre-push needs a candid review, normal flow); (2) the PR run fails by design and uploads the artifact; (3) the owner reviews and commits the file and pushes again — **the pre-push hook then demands a fresh APPROVED candid review bound to the new diff**, which now contains a large generated XML; the plan does not say who runs it or that `--no-verify` is not the answer (n1, MINOR); (4) a strict run must go green before merge. Feasible with that addition.
- Upload ordering: the script exits 1, so a following `actions/upload-artifact` step only runs with an explicit `if: failure() && steps.<build>.outputs.bootstrap == 'true'` (and `bootstrap=true` written to `$GITHUB_OUTPUT` before the exit). The revision wording ("uploads ..., prints `::error::` and exits 1") reads as if the script uploads; not specified (n2, MINOR).
- No new Rust crate; `cargo deny` untouched. Licence statement needs a correction (n7).

### Pillar 5 — Data confidentiality / logging
- `Debug` redacted, value-free errors, Kotlin logging bans extended (`printStackTrace`, `System.err/out`), cookie memory-only, secret names now scanned. PASS.

### Pillar 6 — Test integrity / test power
- No existing test or invariant is modified: §9 keeps all five pinned needles **only if** the editor knows them (n5); `secret_scan.sh` has no pinning test; no existing test writes `[android]`.
- Scenario (wrong purpose switch): an implementation using `if purpose != ChallengePurpose::Unlock` would accept the app origin for `CameraView`; §10 tests unlock and registration only, so it passes (n6, MINOR — round 1 m7 listed CameraView).
- Scenario (wrong touch): mapping `AssetLinks` to `Touch::Refresh` refreshes a valid Funnel session's `last_used` without a `Set-Cookie`; §10 only asserts "no `Set-Cookie`" for anonymous and bogus-cookie requests (n6).
- Metadata invariant: if it requires a versioned component for every catalog entry, it cannot match the BOM-managed `ui`/`material3`; if it skips version-less entries, those two are unchecked (n4).

## 4. Findings

No CRITICAL or MAJOR finding remains.

- **[MINOR] n1 — Owner metadata commit vs pre-push candid review.** `.githooks/pre-push` blocks a push without a fresh APPROVED candid review bound to the pushed diff. — State in the ADR/walkthrough procedure that, after committing the reviewed `verification-metadata.xml`, a candid review is re-run for the new head (the generated file reviewed for origin: only `google()`, `mavenCentral()`, plugin portal coordinates; component count recorded) before the push; never `--no-verify`.
- **[MINOR] n2 — Bootstrap upload step condition.** — Specify: the script writes `bootstrap=true` to `$GITHUB_OUTPUT` before `exit 1`; the upload step has `if: failure() && steps.build.outputs.bootstrap == 'true'`; the invariant pins that condition (otherwise the artifact the procedure depends on is never produced).
- **[MINOR] n3 — No recovery path for an incomplete TOFU file.** — Document: if the first strict run reports missing entries, the owner empties `<components>` to re-run the bootstrap (never hand-edits digests, never adds `trusted-artifacts`); note that the file is generated on Linux, so a local strict build on another OS may need its own platform `aapt2` entry (later PR).
- **[MINOR] n4 — Metadata invariant precision.** — Match catalog entries by `group:name` (any version) for BOM-managed libraries and by `group:name:version` otherwise, and also require the BOM itself and the three plugin markers; forbid `<trusted-artifacts>`, `<trust ` and `<ignored-keys>` in the file and `org.gradle.dependency.verification` in `gradle.properties`.
- **[MINOR] n5 — §9 pins not all listed.** The M2 resolution says the needles are listed in §10; they are not. — List every pin the docs step must keep: §9 contains `native Android app`, `notify.windows.com`, `Tasker` (RAN-S9), a single line with `live camera` and `recording` (camera S-4), `section 2i` (RXB), and no line starting `- **push notification` (push contract); `## 2j.` placed between `## 2i.` and `## 3.`.
- **[MINOR] n6 — Test plan residuals.** — Add to §10: app origin refused for `ChallengePurpose::CameraView`; `AssetLinks` with a valid Funnel session leaves `last_used` unchanged (`Touch::Keep`) and sets no cookie; anonymous Funnel capacity exhaustion answers `503` on the route.
- **[MINOR] n7 — Licence facts.** — Verify the `org.json:json:20250517` POM licence (JSON-java is Public Domain since 2023, not the JSON License) and list JUnit's transitive `hamcrest-core` 1.3 (BSD-3-Clause, test-only).
- **[MINOR] n8 — Spec self-consistency.** §0 A-4 and §5 still describe a warning-only green bootstrap and the old job name; §1.1 says "four" `ConfigError` variants (now five); the draft `verification-metadata.xml` header says "flags the run with a warning". — Fold Revision 1 into §0/§1.1/§5 (or mark them superseded) so the tester and developer read one normative text, and fix the draft header.

## 5. Verdict
VALIDATION_VERDICT: APPROVED
