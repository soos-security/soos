# Walkthrough 248 — Playwright Cross-Engine Bench for the soos-remote Web App

- **Date**: 2026-10-21
- **Issue**: GitHub #464, PR 2 of epic #355 (GitHub-only, no backlog id) — **Branch**: `test/remote-playwright`
- **Decision**: owner decision of 2026-10-21 on #355; amendment of 2026-10-21 to ADR 2026-10-13
  "Browser Support Policy for the `soos-remote` Web App"
- **Matrix criteria**: RPW1–RPW9 (new, verified). RXB1–RXB20 unchanged (RXB12–RXB20 stay owner hardware rows).

## 1. Context & Objectives

PR 1 of #355 (GitHub #366, walkthrough 197) added one capability probe and per-feature degradation to
`crates/remote/assets`, proven by static Rust invariants only: the project had no JavaScript runtime in CI.
On 2026-10-21 the owner approved PR 2: an automated Playwright bench on Chromium, Firefox and WebKit, in
CI, with a pinned Node/Playwright toolchain, no secrets, advisory at first. The behaviours to cover: the
sign-in screen; the status, alerts, offline history and battery cards; the phone tour shown once and its
storage block; reduced motion; and the per-feature messages when WebAuthn, `EventSource` or push is
missing.

## 2. Architect (Phase 1) and plan self-check (Phase 1.5)

- **Target**: the unchanged assets served by the loopback-only README fixture server
  (`tests/fixtures/examples/readme_web_fixture_server.rs`, scenes `unlocked` and `locked-alerts`,
  `--clock fixed`), one server per test, started from the host binary inside the container. The real
  `soos-remote` binary behind a test TLS proxy (epic design) needs its own proxy crate under `tests/`; it
  stays for a later PR together with the CSP header check, the CDP virtual authenticator and mobile
  emulation.
- **States the scenes lack**: a Funnel sign-in (`/api/auth/state` unauthenticated) and an active push view.
  Adding a third `WebScene` would break existing immutable fixture tests that iterate `WebScene::ALL`
  (for example `test_rac_web_fixture_keys_match_the_remote_types` asserts a disabled push view for every
  scene). The two synthetic bodies are therefore fulfilled in the browser with `page.route` (shapes of
  `AuthStateView` and `PushView`, no identity); the fixture crate is unchanged.
- **Missing features** are removed by an init script before any page script: `delete
  window.PublicKeyCredential`, `window.EventSource`, `window.PushManager`,
  `Navigator.prototype.serviceWorker`. A blocked storage is an accessor on `window.localStorage` that
  throws `SecurityError`.
- **Toolchain**: Playwright 1.64.0 (2026-10-07, two weeks old; 1.64.1 is a week old), the official image
  `mcr.microsoft.com/playwright:v1.64.0-noble` by index digest (Node 24.21.0, npm 11.19.0, chromium-1248,
  firefox-1555, webkit-2370), `@playwright/test` 1.64.0 exact, lockfile v3 generated with the image's npm.
- **Advisory without an assertion change**: `portable_crates_contract::test_mac_pr_loop_treats_only_the_macos_job_as_advisory`
  pins `ADVISORY_CHECKS` to two entries, and `scripts/pr_checks_gate.sh` decides over every check of a PR,
  other workflows included. Design: a separate workflow (like `android.yml`), so no `CI Success`
  dependency, with step-level `continue-on-error: true` on the bench step only: the job concludes green,
  the gate never blocks on it, and the outcome is written to the job summary with a `::warning`
  annotation. Promotion removes `continue-on-error` only. No existing assertion changes.
- **Plan self-check**: no production code, route, header, CSP or asset change (RXB-S8 stays green); no Rust
  dependency (the fixture example is std-only, `soos-test-fixtures` keeps its single dependency); nothing
  under `packaging/`; the CI doc and §2i of `Docs/REMOTE_COMPANION.md` describe the job; existing tests
  untouched.

## 3. Tester (Phase 2, red first)

- Static contract `tests/invariants/src/remote_playwright_contract.rs` (RPW-S1–RPW-S5), written before the
  workflow and the documentation: red on the first run (RPW-S5 without ADR amendment, docs, matrix rows and
  this walkthrough; RPW-S1 while the order check pointed at the wrong token), green after Phase 4.
- Bench `tests/browser/specs` (18 tests × 3 engines = 54):

| Spec | Tests | Matrix |
|---|---|---|
| `signin.spec.mjs` | sign-in screen renders with an enabled passkey button | RPW1 |
| `cards.spec.mjs` | locked status and idle minutes; battery from the event stream; alerts card; offline history (newest first); unlocked scene with all four cards | RPW2 |
| `tour.spec.mjs` | shows once, then remembered (Feature tour reopens it); blocked storage shows it again | RPW3 |
| `motion.spec.mjs` | reduce: no animation or transition anywhere; no-preference: state numeral animates, buttons transition | RPW4 |
| `degradation.spec.mjs` | without WebAuthn: sign-in, unlock and camera start disabled with `PASSKEY_UNSUPPORTED_TEXT`, lock works; control with WebAuthn; without `EventSource`: one more status fetch per 15 s of page time (`page.clock`), no stream; without push: `PUSH_UNSUPPORTED_TEXT`; without a service worker: `PUSH_NO_WORKER_TEXT`; control with push | RPW5 |

Every expected sentence is read from `app.js` (`appText`), and an auto fixture fails any test on an
uncaught page error, so an unguarded browser API call is caught even where no text changes.

**Red proof (deliberate breaks, never committed)**: one break of the assets per spec file, applied to the
working tree, that spec run on the three engines through `scripts/remote_browser_tests.sh`, then the asset
restored with `git checkout` (the working tree was checked clean after each revert).

| Spec | Deliberate break of the assets | Result (chromium, firefox, webkit) |
|---|---|---|
| `signin.spec.mjs` | `index.html`: sign-in heading `Sign in` → `Log in` | 3 failed: sign-in screen renders (`Expected "Sign in"`, `Received "Log in"`) |
| `cards.spec.mjs` | `app.js`: battery line `"Battery " + p + "%"` → `" %"` | 6 failed, 9 passed: battery line from the event stream; unlocked scene with all four cards |
| `tour.spec.mjs` | `app.js`: the tour flag is written as `"no"` instead of `TOUR_STORAGE_SEEN` | 3 failed, 3 passed: the tour shows once, then is remembered |
| `motion.spec.mjs` | `style.css`: the `prefers-reduced-motion: reduce` block → `max-width: 1px` (never matches) | 3 failed, 3 passed: no element animates or transitions (34 moving elements) |
| `degradation.spec.mjs` | `app.js`: the probe reports `eventSource: true` whatever the browser | 3 failed, 21 passed: the status is polled every `STATUS_POLL_MS` instead (state `Unreachable`, not `Locked`) |

## 4. Auditor (Phase 3)

Clearance: CLEARED with these constraints, all met and pinned by RPW-S1–RPW-S4:

1. No secret, token or credential anywhere: the workflow has `contents: read`, no `secrets.` reference, no
   `pull_request_target`, checkout without persisted credentials, no artifact upload.
2. Supply chain: exact `@playwright/test`; lockfile v3 with exactly `@playwright/test`, `playwright` and
   `playwright-core` from `registry.npmjs.org` with `sha512` integrity; no `hasInstallScript`;
   `.npmrc` `ignore-scripts=true`; `npm ci --ignore-scripts` with `PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1`
   (the browsers come from the digest-pinned image, so no postinstall script and no browser download
   runs at all); actions pinned to full commit SHAs; Dependabot (npm, grouped `test(deps)` titles within
   72 characters, 7-day cooldown).
3. Isolation: the test container runs with `--network none`, as the calling user, `--cap-drop ALL`,
   `no-new-privileges`, the fixture binary and the assets mounted read-only; the fixture server binds
   `127.0.0.1` only and serves synthetic, identity-free bodies.
4. No sensitive data: trace, screenshot and video are off; no `console.log`; push bodies carry a synthetic
   key and no endpoint; no passkey material exists (the WebKit stub rejects every ceremony).
5. Bounded: one fixture server per test (60 s lifetime, 5 s start-up wait, killed at teardown), test
   timeout 30 s, job timeout 20 min, `retries: 0`, `forbidOnly`.
6. Dev-only: `tests/browser` is not a workspace member and nothing under `packaging/` names it.

## 5. Developer (Phase 4)

- `tests/browser/`: `package.json`, `package-lock.json`, `.npmrc`, `playwright.config.mjs` (three
  projects, `en-US`, `UTC`, no artefacts), `fixtures.mjs` (fixture server per test, page-error guard,
  feature removal, WebAuthn surface stub for engines without it, storage block, frozen clock), five specs.
- `scripts/remote_browser_tests.sh`: builds the fixture example (or takes `--fixture-server`), `npm ci` in
  the image, then the offline test run; extra arguments go to `playwright test`. RAC15
  (`readme_assets_contract::test_rac_capture_tooling_is_dev_only`) reserves the fixture example's name, in
  `scripts/` and `.github/`, to the two README capture scripts; the bench script therefore takes the
  executable from cargo's own JSON build report (`cargo build --locked -p soos-test-fixtures --examples`)
  and stops with a setup error unless the crate reports exactly one example executable. No feature is
  enabled and the fixture crate is unchanged.
- `AGENTS.md` and `AI/ARCHITECTURE.md`: the workspace trees list `tests/browser/`
  (`maintainer_hygiene_contract::test_workspace_maps_list_every_member_and_real_tests_dirs`).
- `.github/workflows/browser.yml`: three advisory jobs; `.github/dependabot.yml`: npm ecosystem;
  `.gitignore`: install tree and outputs.
- Finding: the Linux WebKit build of Playwright has no `navigator.credentials`, so the page shows
  `PASSKEY_UNSUPPORTED_TEXT` there by itself (the degradation works on a real engine). The positive
  sign-in test gives such an engine a stub surface whose `get()`/`create()` reject.
- The `EventSource` test settles the load-time fetches first (`resume()` runs on load and on `pageshow`),
  then checks exactly one more `/api/status` fetch per `page.clock.runFor(15000)`.

## 6. Verification

- `scripts/remote_browser_tests.sh`: 54 passed (18 tests on Chromium, Firefox and WebKit); one run per
  engine as in CI (`-- --project=<engine>`): 18 passed each; `--repeat-each=4`: 216 passed.
- Pinning checked: the image index digest is the registry digest of `v1.64.0-noble` (Node 24.21.0, npm
  11.19.0 inside); the three lockfile integrity hashes equal the registry `dist.integrity`; the lockfile
  regenerated with the image's npm (`npm install --package-lock-only --ignore-scripts`) is byte-identical.
- `cargo test --locked -p soos-invariants -p soos-test-fixtures -p soos-remote`: green, no existing test
  changed.
- Not verified here: the real `soos-remote` binary, real devices (owner rows RXB12–RXB20), and the
  workflow on GitHub runners (it runs on the PR).
