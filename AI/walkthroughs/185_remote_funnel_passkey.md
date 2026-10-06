# Walkthrough 185 — Tailscale Funnel Access and In-House Passkey Authentication for `soos-remote`

- **Date**: 2026-10-06
- **Issue**: GitHub-only follow-up of #339 (no `AI/BACKLOG.md` entry; the branch is not registered
  in `scripts/sync_issue.py`). **Branch**: `feat/remote-funnel-passkey`, from
  `feat/remote-companion` at `b5d593a` (draft PR #340 belongs to `feat/remote-companion`).
  Nothing is committed, pushed or deployed by the agents (owner decision D-I).
- **ADR**: "[2026-10-06] Tailscale Funnel Access and In-House Passkey Authentication for
  `soos-remote`" (supersedes, for `soos-remote` only, item 8 of the 2026-10-05 ADR as far as it
  forbids `tailscale funnel`, and item 2 of the 2026-10-06 "Remote Unlock" ADR).
- **Matrix criteria**: RMC26–RMC44 (new; RMC40 and RMC42–RMC44 are owner hardware checks),
  RMC23–RMC25 (amended), RMC19 (annotated).

## 1. Context & Objectives

Walkthroughs 183 and 184 delivered `soos-remote`: a user-level service on a `0600` Unix socket,
proxied by `tailscale serve`, that shows the lock status, locks the session and, when
`allow_unlock = true`, unlocks it on the Tailscale identity alone. The owner then asked for two
things (decisions D-A to D-I, 2026-10-06):

- reach the PC from the iPhone without a permanent VPN and without paying for anything, through
  Tailscale Funnel on the node's own `*.ts.net` name (no Cloudflare, no domain), while the
  tailnet path stays supported;
- a secure authentication coded in-house: WebAuthn passkeys with Face ID / Touch ID and user
  verification, required for **every** unlock on every path; over Funnel, a passkey login is
  needed for status, events and lock; passkey registration must be impossible from the internet.

The owner's chat request also moves any Cloudflare Tunnel / Cloudflare Access work to a separate
branch and keeps "Tailscale with the VPN" here. Funnel is therefore opt-in (`allow_funnel`
defaults to `false`); the recommended deployment documented in `Docs/REMOTE_COMPANION.md` §2b
stays tailnet-only, and the passkeys protect every unlock in both deployments.

## 2. Architect Design

Spec: `AI/architect_spec_remote_passkey_funnel.md`, round 2. It was backed by two research notes,
`AI/research_funnel.md` (what `tailscaled` 1.102.4 sets and strips on the Serve and Funnel paths)
and `AI/research_webauthn.md` (the WebAuthn L3 subset).

- **Request classification** (`identity.rs::classify_request`, pure): one allowed
  `Tailscale-User-Login` and no Funnel marker means a tailnet caller. One
  `Tailscale-Funnel-Request: ?1`, no identity and `allow_funnel = true` means a Funnel caller.
  Anything else is `403`. The classification never reads `Host`, `X-Forwarded-Host` or
  `X-Forwarded-For`. `tailscaled` deletes client copies of both headers before it sets them.
- **Pure WebAuthn verifier** (`webauthn.rs`). It accepts ES256 only and attestation `none` only.
  UV is required. Credentials are discoverable and `userHandle` is required. The verifier checks
  the strict COSE key, the 37-byte assertion authenticator data and the BE/BS rules. `signCount`
  `0/0` is accepted. It is built on RustCrypto `p256` 0.13.2 (`ecdsa` only), `ciborium`, `sha2`,
  `base64ct` and `subtle`.
- **State** (`challenge.rs`, `websession.rs`, `auth.rs`):
  - Challenges are single use and scoped by purpose, class and binding. They live 120 s.
  - Web sessions use the `__Host-soos_session` cookie. The server keeps only the SHA-256 of the
    token, in memory. Sessions expire after 15 min idle or 8 h absolute, with at most 4.
  - Limiters are keyed per `LimitKey`.
  - Class capacity is reserved per caller class (`Capacity`).
  - `getrandom` is the only CSPRNG.
- **Credential store** (`credentials.rs`): `remote-passkeys.json`, `0600`, owner-checked,
  opened with `O_NOFOLLOW`, at most 16 KiB and 4 passkeys. Writes run under `flock` and go
  through temp file + `fsync` + `rename`.
- **Enrollment** (`enroll.rs`): `soos-remote enroll-code` produces a 10-symbol Crockford base32
  code (50 bits) that lasts 5 min. It is single use and allows 3 attempts. Only its hash is stored,
  in a `0600` file in the `0700` socket directory. Registration is tailnet-only.
- **HTTP** (`http.rs::parse_request`, `read_body`): the four passkey body routes accept a body
  of at most 8 KiB. The framing is `Content-Length` or strict bounded chunked. Every other route
  keeps the `parse_request_head` semantics.
- **Constants**: all live in `crates/remote/src/lib.rs`. The list is in
  `.agents/skills/dev-workflow/references/project-facts.md` §2.
- **Invariants touched**:
  - RMC-S3 (unchanged).
  - New RMC-S13–RMC-S23 in `tests/invariants/src/remote_passkey_contract.rs`.
  - RC-1/RC-2 restated in `AI/ARCHITECTURE.md` §13.

The PAM module, the daemon and the IPC protocol are untouched.

## 3. Plan Evaluation

`AI/plan_evaluator_report.md` returned `VALIDATION_VERDICT: APPROVED` on round 2. Round 1 had
returned REVISION_REQUIRED with F-1 to F-10, and every one of them was resolved:

- the pending user handle;
- availability per path class;
- the dependency check by exact crate names;
- SSE touch;
- the UV-clear negative test;
- revocation and rotation;
- the store error after a valid assertion;
- the invariant needles;
- chunked framing;
- the local same-uid threat.

Round 2 left seven MINOR findings:

- **G-1**: refusals lingering on a global permit.
- **G-2**: the logout and register order on Funnel.
- **G-3**: assets never refresh the idle time.
- **G-4**: IPv4 hints are distinct.
- **G-5**: hint-bucket eviction is a residual.
- **G-6**: `rp_id` breadth when `allowed_hosts` is set.
- **G-7**: matrix cells swapped in spec §12. This was fixed when the rows were copied into the
  matrix.

## 4. Tester Contract

Contract: `AI/tester_contract_remote_passkey_funnel.md`. It added 69 tests (60 in `soos-remote`,
9 in `soos-invariants`) and encoded G-1 to G-6. The Red phase compiled stubs with the exact spec
signatures, and the tests failed on their assertions. Eight guard tests passed at red by design.
Three red runs gave identical results: 119 ok / 59 failed in `soos-remote` and 471 / 7 in
`soos-invariants`.

The tests are in these files:

- `config_tests.rs` (tests 1–7)
- `identity_tests.rs::funnel_classification` (8–10, 58)
- `http_tests.rs::body_framing` (11–13)
- `routes_tests.rs::passkey_routes` (14, 15)
- `webauthn_tests.rs` (16–22)
- `auth_store_tests.rs` (23–26)
- `credentials_tests.rs` (27–29)
- `enroll_tests.rs` (30, 31)
- `auth_server_tests.rs` (32–45)
- `auth_capacity_tests.rs` (54–59)
- `remote_passkey_contract.rs` (46–53a)

The shared harness moved to `crates/remote/tests/common/` (`harness.rs`, `passkey.rs`).

The auditor required four more tests, which were added before the candid review:

- `auth_store_tests::test_rmc_secret_types_debug_is_redacted` (A-T1)
- `credentials_tests::test_rmc_credential_store_lock_file_is_never_followed` (A-T2)
- `auth_capacity_tests::test_rmc_refused_anonymous_funnel_requests_never_hold_funnel_slots` (A-T3)
- `remote_passkey_contract::test_rmc_s22_production_wiring_has_no_test_hooks` (A-T4)

The developer also added `remote_passkey_contract::test_rmc_s23_secret_types_without_redaction_have_no_debug`
(A-T1, invariant variant).

**Migrated tests.** Each migration below is mandated by ADR item 10 and stated there:

- In `server_tests.rs`, `test_rmc_unlock_flow_and_rate_limit`,
  `test_rmc_unlock_and_lock_rate_limits_are_independent`, `test_rmc_unlock_flow_deadline` and
  `test_rmc_unlock_is_audited_without_identity` now send a valid assertion with every unlock that
  used to succeed on the identity alone. Their status, result, logind-call and log assertions are
  unchanged. The audit test gained needles: the credential id, the public key, the user handle and
  every challenge.
- In `test_rmc_unlock_requires_identity_host_and_csrf`, "a 2-byte body is `413`" became
  "`{}` is `400` and a body over 8 KiB is `413`".
- The harness gained the passkey configuration. This is a setup-only change.
- One `advance_ms(OPTIONS_WINDOW_MS)` keeps a 13-unlock test inside the new options limiter.
- `presence_unlock_contract::test_pau_zbus_is_used_only_by_the_daemon` was widened to exactly two
  manifests (ADR 2026-10-05 item 7). It now also asserts `allowed_seen == 2`.
- No assertion was weakened, and no `#[ignore]` or tolerance was added.

## 5. Auditor Constraints

`AI/auditor_constraints_remote_passkey_funnel.md`: **CLEARED** under C1–C26. Here is how they are met:

- **C1–C3** (no panics, `forbid(unsafe_code)`, no print macros). Clippy is clean with the
  workspace lints. The subcommands write through `writeln!` to a locked stdout. `passkeys list`
  prints an 8-hex fingerprint, never the credential id.
- **C4, C5** (bounded decoding, strict chunked decoder). Length checks run before decoding. The
  CBOR recursion limit is set. Tests 11–13, 16–18 and 21 pass.
- **C6, C7** (no body read before the gates, class capacity). Class permits are released before
  the lingering close. `FUNNEL_REFUSAL_LINGER_MS` = 100 is defined once in `lib.rs`. Tests 33, 40,
  54, 54a, 55 and A-T3 pass.
- **C8** (async discipline). The server takes the store lock with non-blocking `flock` attempts
  and `tokio::time::sleep`, bounded by `STORE_LOCK_TIMEOUT_MS` = 500. The synchronous 16 KiB
  read/write on the runtime thread is documented as an accepted bound in `server.rs` and
  `Docs/REMOTE_COMPANION.md` §2b.
- **C9** (assertion order, unconditional UV). `take` runs before the credential lookup, and
  there is a single `unlock_flow` call site. Tests 37, 39, 39a and 43 pass.
- **C10** (constant-time comparisons). `subtle::ConstantTimeEq` is used on challenges, token
  hashes, user handles and code hashes.
- **C11** (fail-closed mapping). A store or RNG failure answers `503` and drops the sessions.
  Test 43 passes.
- **C12** (filesystem safety). `O_NOFOLLOW`, an `fstat` on the opened descriptor, mode `0600`
  at creation and an atomic rename. Tests 28, 29, 31 and A-T2 pass.
- **C13** (production wiring). A-T4 and RMC-S20 pass.
- **C14, C15** (secrets and logging). Secret types have a redacted `Debug` or no `Debug`. The
  fixed audit lines go through `audit.rs`: a constant message with no field, dispatched without a
  macro callsite so that the per-thread subscriber always sees them. Tests 22, 45, A-T1 and S15
  pass.
- **C16–C21** (responses, classification, cookies, CSRF, limiters, enrollment). Tests 14, 15,
  23–26, 30–41b, 42 and 56–59 pass.
- **C22** (clocks). `SystemTime::now` is used only in `main.rs` and `server.rs`.
- **C23** (supply chain). `deny.toml` has one `der@0.7.10` skip, and `cargo deny --locked check`
  is clean.
- **C24** (existing invariants). Every `remote_companion_contract` test passes.
- **C25** (test integrity, ADR residual restatement). The ADR residual on the dropped tailnet
  connection now states the real bound, `FUNNEL_REFUSAL_LINGER_MS`, with class permits released
  before lingering. This change was made in the traceability phase. The fold of G-1 to G-7 into the
  architect spec as round 3 has not been done; it is listed in section 9.
- **C26** (the 10× timing loop). This is recorded by the developer gate and was not re-run here.

## 6. Implementation

**New modules** in `crates/remote/src/`:

- `webauthn.rs`
- `challenge.rs`
- `websession.rs`
- `credentials.rs`
- `enroll.rs`
- `auth.rs`
- `audit.rs`

**Changed modules**:

- `lib.rs`: constants and compile-time asserts.
- `config.rs`: `AuthConfig` with `rp_id`, `allow_funnel` and `credentials_path`.
- `identity.rs`: `classify_request` and `client_hint`.
- `http.rs`: `parse_request`, `read_body` and `413 body_too_large`.
- `routes.rs`: the passkey routes, `is_funnel_public` and `check_auth_csrf`.
- `server.rs`: the dispatch order of spec §6, class capacity, ceremonies and unlock with an
  assertion.
- `main.rs`: the `enroll-code` and `passkeys list|remove` subcommands, all of which refuse root.

**Other changed files**:

- The page (`assets/index.html`, `app.js`, `style.css`) gained a sign-in screen, *Add a passkey*,
  and Face ID before *Unlock now*. WebAuthn runs modal only, with no web storage.
- `Cargo.toml` and `crates/remote/Cargo.toml` gained the dependencies; `deny.toml` gained the
  `der` skip.
- `scripts/install_remote.sh` now prints Funnel guidance. It still never runs `tailscale`.

**Documentation updated in this phase**:

- `Docs/REMOTE_COMPANION.md`:
  - §2b states that Funnel is optional and that tailnet-only is the recommended deployment.
  - §2b also lists the Funnel prerequisites (MagicDNS, HTTPS certificates, the `funnel` node
    attribute, shields-up) and the owner's commands: `tailscale funnel --bg unix:...`,
    `tailscale funnel status`, and how to roll back.
  - §4 replaces the old blanket Funnel ban with "port 443 only, with `allow_funnel` and `rp_id`".
  - §7 has eleven new troubleshooting rows and two amended ones.
  - §8 lists the Funnel marker dependency and the two MINOR review findings as residual limits.
  - §9 adds any other public exposure (Cloudflare Tunnel or Access) to the out-of-scope list.
- `AI/ARCHITECTURE.md` §13: new rows for request classes, passkeys and Funnel access; the
  passkey-gated unlock row; updated bounds, privacy and invariant text.
- `AI/MOCK_STRATEGY.md`: a new "Funnel and passkey doubles" subsection.
- `AI/VERIFICATION_MATRIX.md`: a new component section and amended rows RMC23–RMC25.
- `AI/DECISIONS.md`: the matrix reference and the restated G-1 residual.
- The project facts (crate map row and a new Funnel/passkey constants row).

**Decision taken in traceability.** The spec has a single hardware row, RMC40. It was split into
four owner checks so that each one can be reported on its own:

- RMC40: Funnel reachability on 4G.
- RMC42: passkey registration.
- RMC43: Face ID login.
- RMC44: Face ID unlock.

The ADR reference was updated to RMC26–RMC44 to match.

## 7. Candid Review

`AI/candid_review_report.md` gave **VERDICT: APPROVED**. The review is bound to fingerprint
`6974247bf00222fff1897b6b39b1dc4c9a7e55b1c6c9a684e0b641e14104002c` against merge-base `222665f0`.
It found no CRITICAL or MAJOR issue.

- **MINOR** (`server.rs::persist_counter`): the counter is re-checked under the store lock, but
  an equal or lower counter is kept silently instead of being refused. Two concurrent assertions
  of a cloned device-bound passkey with the same counter can both pass. This affects clone
  detection only. Synced `0/0` passkeys are unaffected. Not fixed; documented in §8 of the
  operator page and in section 9.
- **MINOR** (`server.rs::issue_options`): an anonymous Funnel login/options answers
  `409 no_passkey` when the store is empty, which tells an internet client whether a passkey
  exists. Not fixed; documented the same way.
- **SUGGESTION**: state that Funnel is optional and that tailnet-only is the recommended
  deployment. Done in `Docs/REMOTE_COMPANION.md` §2b.

The documentation edits of this phase change the diff, so the fingerprint-bound gate
(`./scripts/candid_subagent.sh`) must be re-run before any push.

## 8. Verification Results

Run on 2026-10-06 in the worktree:

- `cargo test --locked -p soos-remote -p soos-invariants --all-features`: all pass.
  - `soos-invariants`: 480 passed. This includes `matrix_citations`, run after the matrix edit;
    every RMC26–RMC41 citation resolves.
  - `soos-remote`: 181 passed. `auth_capacity_tests` 8, `auth_server_tests` 17,
    `auth_store_tests` 6, `config_tests` 27, `credentials_tests` 4, `enroll_tests` 2,
    `http_tests` 20, `identity_tests` 18, `routes_tests` 11, `server_tests` 40,
    `session_tests` 10, `socket_tests` 9, `webauthn_tests` 9.
- `cargo deny --locked check`: `advisories ok, bans ok, licenses ok, sources ok`.
- `cargo test --locked --workspace --all-targets --all-features`: exit 0, 3017 passed, 0 failed,
  6 ignored, over 353 test binaries (no test of this change is ignored).
- `cargo fmt --all -- --check` and
  `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`: clean at the
  candid review. This phase changed no Rust file.

## 9. Known Limitations / Follow-ups

- **Hardware checks (owner)**. Funnel reachability from the iPhone on 4G (RMC40), passkey
  registration (RMC42), Face ID login (RMC43) and Face ID unlock on both paths (RMC44, which also
  closes the hardware part of RMC25) were verified by the owner on 2026-10-06; the Shortcuts
  steps of RMC21 are still pending. The agents never run
  `tailscale funnel` or `tailscale serve` (D-I).
- **Two MINOR review findings** (section 7): strict counter refusal under the store lock, and a
  uniform anonymous answer to login/options.
- **Spec fold**: the architect spec still reads round 2. Folding G-1 to G-7 into it as round 3
  (auditor C25) is pending. The ADR residual was restated in this phase.
- **Funnel marker**: Funnel depends on the undocumented `Tailscale-Funnel-Request: ?1` marker. If
  a Tailscale release drops it, Funnel requests get `403`. This is fail-closed.
- **Accepted residual risks** (ADR):
  - distributed denial of service against the anonymous Funnel login;
  - same-uid local processes;
  - compromise of the owner's Apple account (synced passkeys);
  - the public Certificate Transparency record of the node name.
- **Cloudflare Tunnel / Cloudflare Access**: out of scope here, and to be explored on a separate
  branch with its own ADR, as the owner requested.
