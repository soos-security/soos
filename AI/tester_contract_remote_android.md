# Tester Contract — GitHub #349: Android Support for `soos-remote` and Web Push

- **Date**: 2026-10-09
- **Branch**: `feat/remote-android`
- **Inputs**: `AI/architect_spec_remote_android.md` (round 1, amended by PE-1..PE-6), `AI/plan_evaluator_report.md`
  (APPROVED), owner amendments AM-1..AM-7 (already applied in commit `a5e6d8c`).
- **AM-7 branch in force**: `png = "=0.18.1"` added under `[dev-dependencies]` of `crates/remote/Cargo.toml`.
  `Cargo.lock` gains only `"png"` in the `soos-remote` dependency list (no new package);
  `cargo tree -p soos-remote -e dev --locked` resolves `png v0.18.1`. The three pixel tests therefore live in
  `crates/remote/tests/android_assets_tests.rs` (spec §12, "AM-7 approved").
- **CI commands used**: `cargo test --locked --all-features -p soos-invariants`, `-p soos-remote`,
  `-p soos-push-protocol`, `-p soos-push-sender`; `cargo fmt --all -- --check`; `cargo clippy --locked
  --all-features --all-targets -- -D warnings`.

## New files

| File | Tests |
|---|---|
| `tests/invariants/src/remote_android_contract.rs` (+ `mod` line in `tests/invariants/src/lib.rs`) | `test_ran_s1_*` … `test_ran_s10_*`, `test_ran_helper_js_lexer_self_test` |
| `crates/push-protocol/tests/android_endpoint_tests.rs` | `test_ran_endpoint_shapes_are_accepted`, `test_ran_push_host_is_the_single_source_of_the_allowlist` |
| `crates/push-sender/tests/android_sender_tests.rs` | `test_ran_sender_accepts_android_endpoints` |
| `crates/remote/tests/android_push_tests.rs` | `test_ran_push_service_maps_each_host`, `test_ran_endpoints_subscribe_store_and_dispatch`, `test_ran_chrome_subscription_json_parses`, `test_ran_worker_resubscribe_shape_is_accepted`, `test_ran_worker_resubscribe_without_funnel_session_is_refused` |
| `crates/remote/tests/android_webauthn_tests.rs` | `test_ran_chrome_client_data_with_extra_members_verifies`, `test_ran_google_password_manager_registration_verifies`, `test_ran_google_password_manager_assertion_with_zero_counter_verifies` |
| `crates/remote/tests/android_assets_tests.rs` | `test_ran_assets_are_routed_as_png`, `test_ran_assets_are_served_with_the_unchanged_csp`, `test_ran_maskable_icon_keeps_the_mark_in_the_safe_zone`, `test_ran_badge_is_white_on_transparent`, `test_ran_any_icons_are_full_bleed_brand_renders` |

27 new tests: 11 static (10 contracts + 1 helper self-test), 16 behaviour. No existing test, helper or assertion was
modified; `tests/common/*` is unchanged (the AAGUID registration helper lives in `android_webauthn_tests.rs`, the push
harness is a reduced copy of `push_server_tests.rs` inside `android_push_tests.rs`).

## Tester Contract

| Test (path::name) | Acceptance line / matrix | Expected red reason (observed) | Coverage or red |
|---|---|---|---|
| `soos-invariants::remote_android_contract::test_ran_helper_js_lexer_self_test` | helper | — (self-test of the JS lexer, brace matcher, section and CSP extractors) | green helper |
| `…::test_ran_s1_service_worker_notification_options` | A1 / RAN1 | `sw.js must contain const ICON_PATH = "/icon-192.png";` | red |
| `…::test_ran_s2_service_worker_resubscribes_once_with_the_page_request_shape` | A2 / RAN2 | `exactly one pushsubscriptionchange listener` (left 0, right 1) | red |
| `…::test_ran_s3_manifest_is_installable_on_android` | A3 / RAN3 | `manifest id "/"` | red |
| `…::test_ran_s4_android_icons_are_well_formed_pngs` | A3 / RAN4 | `crates/remote/assets/icon-192.png must exist (spec §6)` | red |
| `…::test_ran_s5_android_assets_are_embedded_and_routed` | A4 / RAN5 | `AssetId has the variant Icon192` | red |
| `…::test_ran_s6_user_text_is_platform_neutral` | A5 / RAN6 | `app.js: every Face ID is followed by ", fingerprint or screen lock": ["Face ID / Touch ID) …", …]` | red |
| `…::test_ran_s7_push_client_keeps_unknown_key_subscriptions_and_bounds_the_worker_wait` | A6 / RAN7 | `sameServerKey is replaced` | red |
| `…::test_ran_s8_push_service_classification_is_exhaustive` | A7 / RAN8 | `pub enum PushHost` | red |
| `…::test_ran_s9_android_is_documented` | A9, A10 / RAN11 | `## 2h. Android section` (not found) | red |
| `…::test_ran_s10_installer_text_is_platform_neutral` | A5, A9 / RAN6 | `scripts/install_remote.sh must not contain iPhone lock screen` | red |
| `soos-push-protocol::android_endpoint_tests::test_ran_endpoint_shapes_are_accepted` | A8 / RAN9 | compile: E0599 `push_host` (spec §2.1 API); host/origin/bounds part already holds | red (API) — coverage otherwise |
| `soos-push-protocol::android_endpoint_tests::test_ran_push_host_is_the_single_source_of_the_allowlist` | A7 / RAN8 | compile: E0432 `soos_push_protocol::PushHost` (spec §2.1 API) | red |
| `soos-push-sender::android_sender_tests::test_ran_sender_accepts_android_endpoints` | A8 / RAN9 | — passes today | coverage (green) |
| `soos-remote::android_push_tests::test_ran_push_service_maps_each_host` | A7 / RAN8 | compile: E0624 `associated function of is private` (spec §2.2 makes it `pub`) | red |
| `soos-remote::android_push_tests::test_ran_endpoints_subscribe_store_and_dispatch` | A8 / RAN9 | blocked only by the E0624 of the same file; passes once `of` is public | coverage |
| `soos-remote::android_push_tests::test_ran_chrome_subscription_json_parses` | A8 / RAN9 | same (file-level compile error only) | coverage |
| `soos-remote::android_push_tests::test_ran_worker_resubscribe_shape_is_accepted` | A2 / RAN2 | same (file-level compile error only) | coverage |
| `soos-remote::android_push_tests::test_ran_worker_resubscribe_without_funnel_session_is_refused` | A2 / RAN2 | same (file-level compile error only) | coverage (regression guard) |
| `soos-remote::android_webauthn_tests::test_ran_chrome_client_data_with_extra_members_verifies` | A8 / RAN10 | — passes today | coverage (green) |
| `soos-remote::android_webauthn_tests::test_ran_google_password_manager_registration_verifies` | A8 / RAN10 | — passes today | coverage (green) |
| `soos-remote::android_webauthn_tests::test_ran_google_password_manager_assertion_with_zero_counter_verifies` | A8 / RAN10 | — passes today | coverage (green) |
| `soos-remote::android_assets_tests::test_ran_assets_are_routed_as_png` | A4 / RAN5 | `GET /icon-192.png must route to an asset, got NotFound` | red |
| `soos-remote::android_assets_tests::test_ran_assets_are_served_with_the_unchanged_csp` | A4 / RAN5 | `…/assets/icon-192.png must exist (spec §6)` | red |
| `soos-remote::android_assets_tests::test_ran_maskable_icon_keeps_the_mark_in_the_safe_zone` | A3 / RAN4 | `…/assets/icon-maskable-512.png must exist (spec §6)` | red |
| `soos-remote::android_assets_tests::test_ran_badge_is_white_on_transparent` | A1 / RAN1 | `…/assets/badge-96.png must exist (spec §6)` | red |
| `soos-remote::android_assets_tests::test_ran_any_icons_are_full_bleed_brand_renders` | A3 / RAN4 | `…/assets/icon-192.png must exist (spec §6)` | red |

Coverage contracts (acceptance line A8 and the §3.3 "no check loosened" guard) are green by design: the wire,
the subscribe parser and the WebAuthn verifier already accept Android (spec §0.1, plan evaluator Pillar 6). They pin
behaviour the issue keeps; this is not a weakening.

`android_assets_tests.rs` never names the new `AssetId` variants (it reaches them through `route()` and pins their
names through `Debug`), so it compiles today and fails on assertions.

### Amended existing invariants (AM-1..AM-7, committed in `a5e6d8c`, not touched here)

| Test | Current state |
|---|---|
| `remote_brand_contract::test_rmc_s50_ui_contract_ids_labels_and_attributes_are_kept` (AM-5) | red: `label of #login-button` left `"Sign in with Face ID"`, right `"Sign in with your passkey"` |
| `remote_brand_contract::test_rmc_s53_manifest_and_meta_colors` (AM-4) | **does not compile** (see blocker); with a local type-only fix it is red: `manifest must contain "id":"/"` |
| `remote_brand_contract::test_rmc_s54_system_fonts_and_fixed_asset_set` (AM-1..AM-3) | red: `the asset directory holds exactly eleven files` (seven listed) |
| `remote_push_contract::test_rmc_s38_page_and_service_worker` (AM-6) | red: `app.js must contain On iPhone, notifications need the home-screen app …` |
| `remote_battery_contract::test_rbs_s6_no_new_dependency` (AM-7) | green (the `png` dev-dependency is in the amended allowlist) |

Every other existing invariant stays green.

### Blocker found (existing committed test, not modified)

`tests/invariants/src/remote_brand_contract.rs` L2340–L2370 (`test_rmc_s53_manifest_and_meta_colors`, AM-4) does not
compile: `error[E0308]: mismatched types … expected an array with a size of 3, found one with a size of 4`. The
`for entry in [[…3 needles], [… 3], [… 4], [… 4], [… 4]]` literal mixes array lengths. The whole `soos-invariants`
test crate therefore fails to build at `a5e6d8c`, independent of this phase. A type-only fix that keeps every needle
and assertion (make the elements slices: `&[…][..]` for the first element and `&[…]` for the others) was applied
locally for verification only and reverted; it needs owner/orchestrator approval before it is committed.

## Power check (reference implementation, scratch copy, not committed)

A throwaway copy of the tree with a spec-conformant implementation (§2.1 `PushHost`, §2.2 `PushService::of`, §2.3
assets and routes, §3 `sw.js` verbatim, §4 `app.js`/`index.html`, §5 manifest verbatim, §6 icons generated with the
exact commands — byte sizes 2885 / 8121 / 6926 / 1098 as in the spec — §7 docs and installer, the ADR title, and the
AM-4 type-only fix) gives: `soos-invariants` 550 passed / 0 failed; `soos-remote`, `soos-push-protocol`,
`soos-push-sender` all green; `cargo clippy --all-features --all-targets -D warnings` clean on the four packages;
`cargo fmt --check` clean. So every contract is satisfiable by the spec as written.

## Flakiness check

`android_push_tests` and `android_assets_tests` run 10 times in a row on the reference copy: 10/10 green. All server
tests use the paused clock; the only real-time wait is the bounded `wait_view` poll (at most 2 s) after a `410`.

## Notes for the developer

- `test_ran_s6`/`test_ran_s10` follow D-6 literally: every `Face ID` (comments included) must be immediately followed
  by `, fingerprint or screen lock` on the same line.
- `test_ran_s2` requires the `headers` object of `postSubscription` to hold exactly the two page headers, no `.then(`
  or response read in `postSubscription`, no `.unsubscribe(` and no `"/api/push"` literal in `sw.js`.
- `test_ran_s7` requires the `=== "different"` branch to hold the only `.unsubscribe(` of `syncSubscription`, and in
  `enableNotifications` the order `await readyRegistration()` → `=== null` → `PUSH_WORKER_FAILED_TEXT` →
  `pushManager.subscribe(`.
- Docs needles are matched with whitespace runs collapsed, so Markdown line wrapping is free.
