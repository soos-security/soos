# Tester Contract — GitHub #382: native Android app foundation (PR 1 of #357)

Branch `feat/android-native-app`; spec `AI/architect_spec_android_native_app.md` (revisions 1
and 2); plan evaluation round 2 APPROVED. Matrix prefix `ANA`.

## Rust contract tests — `crates/remote/tests/android_app_tests.rs`

| Test | Matrix | Red evidence (stub phase: API present, `[android]` ignored, no origin acceptance, no route) |
|---|---|---|
| `test_ana_config_constants` | ANA1 | passed (constants written with the stubs) |
| `test_ana_config_without_android_table_is_unchanged` | ANA1 | regression guard, passed |
| `test_ana_config_valid_android_table` | ANA1 | FAILED: `android table` (stub returns no binding values) |
| `test_ana_config_android_requires_rp_id` | ANA1 | FAILED: `Ok(..)` instead of `AndroidRequiresRpId` |
| `test_ana_config_android_requires_funnel` | ANA1 | FAILED (variant never returned) |
| `test_ana_config_accepts_valid_packages` | ANA1 | FAILED: package not kept |
| `test_ana_config_rejects_invalid_packages` | ANA1 | FAILED: accepted |
| `test_ana_config_fingerprint_count` | ANA1 | FAILED: accepted |
| `test_ana_config_rejects_invalid_fingerprints` | ANA1 | FAILED: accepted |
| `test_ana_config_unknown_or_malformed_android_table_is_syntax` | ANA1 | passed (serde `deny_unknown_fields` written with the stubs) |
| `test_ana_config_checks_run_after_existing_ones` | ANA1 | regression guard, passed |
| `test_ana_config_values_never_reach_debug_or_errors` | ANA1 | FAILED (no error for an invalid table) |
| `test_ana_load_config_with_android_reads_the_file` | ANA1 | FAILED: empty package |
| `test_ana_asset_links_json_is_exact_and_bounded` | ANA3 | FAILED: empty body |
| `test_ana_android_origin_accepted_only_with_policy_for_get` | ANA2 | FAILED: `OriginMismatch` for a configured app origin |
| `test_ana_android_origin_refused_without_app` | ANA2 | regression guard, passed |
| `test_ana_near_android_origins_are_refused` | ANA2 | regression guard, passed |
| `test_ana_android_package_name_member_must_match_when_present` | ANA2 | FAILED: matching package refused |
| `test_ana_android_origin_keeps_cross_origin_rules` | ANA2 | FAILED: `OriginMismatch` instead of `CrossOrigin` |
| `test_ana_arbitrary_origins_never_pass_as_android` (proptest) | ANA2 | passed (refusals) |
| `test_ana_check_assertion_accepts_the_app_origin_for_login_only` | ANA2 | added in revision 2 after the implementation (round-2 finding n6); fails against an implementation that widens Unlock or CameraView, or never accepts Login |
| `test_ana_route_asset_links` | ANA3 | FAILED: `NotFound` |
| `test_ana_asset_links_absent_is_byte_identical_404` | ANA3 | regression guard, passed |
| `test_ana_asset_links_served_on_tailnet_and_funnel` | ANA3 | FAILED: `404` |
| `test_ana_asset_links_other_methods_and_hosts` | ANA3 | FAILED: `404` instead of `405` |
| `test_ana_asset_links_need_rp_id_at_runtime` | ANA3 | passed (fail-closed guard) |
| `test_ana_login_accepts_the_configured_app_origin` | ANA4 | FAILED: `403 passkey_rejected` |
| `test_ana_login_refuses_app_origin_without_binding_or_other_digest` | ANA4 | regression guard, passed |
| `test_ana_unlock_and_registration_stay_web_origin_only` | ANA4 | regression guard, passed |
| `test_ana_fixture_digests_differ` | — | fixture sanity |

Stub run: 17 failed, 12 passed (the passing ones are the fail-closed guards that the base code
already satisfies). That run used the shared cargo target directory, which the coordinator later
declared unsafe; every Green result reported in the walkthrough was re-run with a private target.

## Invariants — `tests/invariants/src/android_native_contract.rs`

| Test | Matrix |
|---|---|
| `test_ana_s1_no_secret_files_under_android` | ANA7 |
| `test_ana_s2_versions_are_exact_pins`, `test_ana_s3_gradle_distribution_is_pinned`, `test_ana_s4_dependency_verification_metadata` | ANA8 |
| `test_ana_s5_no_analytics_firebase_play_services_or_webview`, `test_ana_s6_manifest_hardening` | ANA9 |
| `test_ana_s7_brand_colors_and_icon` | ANA10 |
| `test_ana_s8_kotlin_hygiene` | ANA11 |
| `test_ana_s9_ci_job`, `test_ana_s10_build_script` | ANA12 |
| `test_ana_s11_remote_binding_is_login_only_and_silent` | ANA6 |
| `test_ana_s12_docs_adr_walkthrough` | ANA5 |

Red: written before the docs; `test_ana_s12_docs_adr_walkthrough` failed with `section 2j`, and
`test_ana_s2_versions_are_exact_pins` caught a `"+"` inside a catalog comment (reworded).

## Kotlin JVM unit tests (CI only: no JDK/SDK on the development host)

`android/app/src/test/java/org/soos/remote/{PcUrlTest,SessionCookieTest,ApiJsonTest,PasskeyJsonTest,SoosApiTest}.kt`
(ANA11). They were never executed locally; their first run is the CI job after the verification
metadata commit (ANA13).

### Migrated existing tests

None. No existing test, fixture or harness file was changed.

### Flakiness check

The suite runs on the frozen paused clock of the shared harness (no wall-clock assertion); it was run repeatedly during the developer gate with the private target (walkthrough 208).
