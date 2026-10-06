# Candid Review Report

- **Date**: 2026-10-06
- **Target Branch**: `feat/remote-funnel-passkey`
- **Base (merge-base)**: `222665f0`
- **Reviewed-Diff-Fingerprint**: `9c761e601ede2bbd19e066c47f688d75bfd19e5b4278f336556b7f050fe33ca4`
- **Audited Files**: `.agents/skills/dev-workflow/references/project-facts.md`, `AGENTS.md`, `AI/ARCHITECTURE.md`,
  `AI/DECISIONS.md`, `AI/MOCK_STRATEGY.md`, `AI/VERIFICATION_MATRIX.md`, `AI/architect_spec_remote_companion.md`,
  `AI/architect_spec_remote_passkey_funnel.md`, `AI/auditor_constraints_remote_companion.md`,
  `AI/auditor_constraints_remote_passkey_funnel.md`, `AI/research_funnel.md`, `AI/research_webauthn.md`,
  `AI/tester_contract_remote_companion.md`, `AI/tester_contract_remote_passkey_funnel.md`,
  `AI/walkthroughs/183_remote_companion.md`, `AI/walkthroughs/184_remote_unlock.md`,
  `AI/walkthroughs/185_remote_funnel_passkey.md`, `Cargo.lock`, `Cargo.toml`, `Docs/README.md`,
  `Docs/REMOTE_COMPANION.md`, `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`,
  `crates/remote/**` (Cargo.toml, assets, `src/{assets,audit,auth,challenge,config,credentials,enroll,http,identity,lib,logind,main,routes,server,session,socket,status,webauthn,websession}.rs`,
  every file under `crates/remote/tests/`), `deny.toml`, `packaging/soos-remote.service`, `scripts/candid_review.sh`,
  `scripts/install_remote.sh`, `tests/invariants/src/{lib,presence_unlock_contract,remote_companion_contract,remote_passkey_contract}.rs`

## 1. Executive Summary

Re-review after the previous APPROVED report (fingerprint `5fa4bbba…`). The author states that the
only changes since are two error texts in `crates/remote` reworded from "...permissions are
unsafe" to "...permissions are insecure" (source, the tests asserting them, the architect spec)
plus one sentence in `AI/auditor_constraints_remote_passkey_funnel.md` line 20.

Verified mechanically, not by trust: the current index (staged content, no untracked files)
reproduces the previous fingerprint `5fa4bbba…` exactly when diffed with the gate's pinned options,
and the unstaged work-tree delta is exactly 7 changed lines in 6 files
(`crates/remote/src/credentials.rs:501`, `crates/remote/src/enroll.rs:229`,
`crates/remote/tests/credentials_tests.rs:226`, `crates/remote/tests/enroll_tests.rs:243`,
`AI/architect_spec_remote_passkey_funnel.md:741,804`,
`AI/auditor_constraints_remote_passkey_funnel.md:20`). A line diff of the old and new frozen
patches shows no other content change. No occurrence of the old text remains outside the
historical sentence in the auditor constraints, which deliberately quotes it.

Both tests compare `err.to_string()` with `assert_eq!` against the exact new strings, so the
production texts and the contract moved together; this is a wording change of an English
`Display` text, not a weakening (the variant mapping, the checks that produce `Insecure` at
`credentials.rs:227,234,237,264,404,409` and `enroll.rs:207`, and their tests are unchanged).
No HTTP body, log line or other test depends on the old text (grep).

The full review was still performed on the code: the WebAuthn verifier, the assertion check, the
four passkey routes, the unlock route order, the Funnel classification and pre-routing gate, the
web-session store, the credential-store update path, the dependency changes and `deny.toml`.
Results: `cargo fmt --check` clean; `cargo clippy -p soos-remote -p soos-invariants --all-targets
--all-features -- -D warnings` clean; `cargo test --locked -p soos-remote -p soos-invariants
--all-features` passes (661 tests: 480 invariants, 181 `soos-remote`; re-run on this fingerprint); `cargo deny check` reports advisories,
bans, licenses and sources ok.

No CRITICAL or MAJOR finding. The two carried-over MINOR code findings remain documented
residual limitations, and the MINOR walkthrough row-count inaccuracy from the previous review is
still present (unchanged file).

## 2. Test Changes (mechanical listing from step 3, with justification per change)

- Test files touched: all of `crates/remote/tests/*` (new crate relative to `origin/main`),
  `crates/remote/tests/common/`, `http_tests.proptest-regressions`,
  `tests/invariants/src/{lib,presence_unlock_contract,remote_companion_contract,remote_passkey_contract}.rs`.
- Removed/changed assertions in the frozen patch: exactly one (`patch:28128`),
  `presence_unlock_contract::test_pau_zbus_is_used_only_by_the_daemon`: the daemon-only zbus check
  becomes exactly two allowed manifests (`crates/daemon`, `crates/remote`) each declaring
  `zbus = { workspace = true }` once, plus `allowed_seen == 2`; every other manifest still must
  not mention zbus. Justified by ADR 2026-10-05 item (7) and matrix PAU17; stricter for every
  other crate.
- `tests/invariants/src/lib.rs`: `"remote"` added to the `forbid(unsafe_code)` business-crate list
  (strengthening) and the `remote_passkey_contract` module registered.
- New escape hatches (`#[ignore]`, `#[cfg(any())]`, `should_panic`, tolerance/epsilon): none (the
  only grep hit, `patch:4801`, is walkthrough prose).
- Inline `mod tests` changes: none.
- Delta since the previous approved review: two lines in new (added) test files,
  `credentials_tests.rs:226` and `enroll_tests.rs:243`, change the expected `Insecure` text to
  "...permissions are insecure", matching the production `#[error]` change. The assertions are
  still exact `assert_eq!(err.to_string(), text)`; no check was removed or loosened. These files are
  wholly added relative to `origin/main`, so the step-3 removed-line grep is unchanged (one hit,
  `patch:28128`).

## 3. Deep Reasoning Audit

### Logic & Architecture
- Delta claim: verified by rebuilding the previous tree from the index and reproducing fingerprint
  `5fa4bbba…`, then diffing the two frozen patches: only the seven reworded lines differ. Scenario
  "a caller matches on the old text" → none in `crates/`, `tests/`, `scripts/` or `Docs/`; errors are
  mapped by variant (`auth.rs:325`), not by string. PASS.
- Troubleshooting count: `Docs/REMOTE_COMPANION.md` §7 vs `HEAD` gains twelve rows
  (`passkey_required`, `passkeys_not_configured`, `login_required`, `no_passkey`,
  `enroll_code_rejected`, `store_unavailable`, `503 busy`, `421` over Funnel, address does not
  resolve, `tailscale funnel` refuses, Face ID prompt never appears, Safari vs home-screen app)
  and amends two (`403` on every page, `/api/events 503`). The walkthrough still says eleven.
  FINDING (MINOR, carried over, documentation accuracy).
- WebAuthn (`webauthn.rs`): scenario "assertion with UV clear but valid signature" → refused by
  `check_rp_and_user` before the signature; "attestation with trailing CBOR byte / sixth COSE
  label / duplicate label / off-curve point" → refused; "origin with port or other host" → byte
  comparison with `https://<rp_id>`; "`crossOrigin: true` or `topOrigin`" → refused; "BS without
  BE / BE changed since registration" → refused; "counter 5 after stored 5" → refused (non-zero
  rule). PASS.
- Assertion check (`auth.rs:355`): the challenge is consumed before the store read, so a replay
  or a challenge of another purpose/class/binding (login vs unlock, Funnel session hash) fails;
  unknown credential id, absent user handle → `Rejected` (counted). PASS.
- Funnel gate (`server.rs` `handle_connection`): a Funnel request whose host is not `rp_id` →
  `421`; register routes → `403`; any non-public route (status, events, lock, unlock, unlock
  options) without a valid session → `403 login_required`; the marker plus a
  `Tailscale-User-Login` header is `Ambiguous` → `403`. Scenario "anonymous Funnel caller calls
  `/api/unlock` with a valid assertion" → refused before routing. PASS.
- Registration: tailnet only, enrollment code checked on both options and verify, challenge bound
  to the code hash, user-handle conflict, duplicates and the passkey cap re-checked under the
  store lock. PASS.
- Carried over: counter equality under concurrent assertions (`server.rs:476`) and the anonymous
  `409 no_passkey` oracle (`server.rs:438`) remain; documented in `Docs/REMOTE_COMPANION.md` §8.

### PAM Concurrency & Deadlines
- `crates/pam` is not touched. The user-level service bounds the head read, the body read
  (`BODY_READ_TIMEOUT_MS`), the store lock wait (`STORE_LOCK_TIMEOUT_MS`, non-blocking retries) and
  the unlock flow; the auth mutex is never held across a body read or a logind call. PASS.

### Panic Safety & Fail-Closed
- No `unwrap`/`expect`/`panic!`/`todo!`/`unreachable!` in `crates/remote/src` (grep); slices use
  `get`, arithmetic is saturating/checked. Every error path of the unlock route (CSRF, disabled,
  no `rp_id`, no body, host, lockout, malformed body, rejected assertion, store failure, counter
  persistence failure) returns a refusal before `unlock_flow`; RNG failure → `503`. No path reaches
  `UnlockSession` without a verified UV assertion. PASS.

### Test Integrity & Anti-Weakening
- §2 listing reviewed; the only removed assertion is a justified, stricter contract migration.
  Suites pass with 181 `soos-remote` and 480 invariant tests. PASS.

### Memory, Bounds & Secrets
- Bodies capped at 8 KiB, client data / attestation / signature / credential id bounded before
  decoding, CBOR recursion limit 16, base64url length checked before decoding; challenges,
  session tokens, enrollment code and bodies held in `Zeroizing`; `ClientHint` and `LimitKey`
  `Debug` redacted; no secret in `debug!` fields; credential store `0600` and owner-checked;
  `#![forbid(unsafe_code)]` in `lib.rs` and `main.rs`; `app.js` uses no `innerHTML`/`eval`. PASS.

### Supply Chain & Automation
- New workspace dependencies `p256 0.13` (`ecdsa` only, no default features), `base64ct`,
  `subtle` are RustCrypto/dalek pure-Rust crates, no OpenSSL; the `der@0.7.10` duplicate skip in
  `deny.toml` carries a precise reason. `cargo deny check` clean. No `.github/` change. PASS.

### English-Only Policy
- All additions are English; the only accented characters in the patch are deliberate test inputs
  (`"é@example.com"`, `"é"`) for login validation. PASS.

## 4. Detailed Findings & Action Items

- **[MINOR]** `crates/remote/src/server.rs:476` — (carried over) `persist_counter` keeps the stored
  counter but still returns `Ok` when a concurrent assertion already stored an equal or higher
  counter, so two assertions of a cloned device-bound authenticator with the same counter can both
  succeed; documented in `Docs/REMOTE_COMPANION.md` §8. Follow-up: refuse with `403
  passkey_rejected` when `outcome.sign_count <= record.sign_count` (either non-zero) under the
  store lock.
- **[MINOR]** `crates/remote/src/server.rs:438` — (carried over) an anonymous Funnel
  `login/options` answers `409 no_passkey` vs `200`, disclosing whether a passkey is enrolled;
  documented in §8. Follow-up: answer the anonymous path uniformly.
- **[MINOR]** `AI/walkthroughs/185_remote_funnel_passkey.md:231` — (carried over) says "§7 has eleven new
  troubleshooting rows" while `Docs/REMOTE_COMPANION.md` §7 gains twelve new rows (and two amended);
  correct the count to twelve or drop the number.

## 5. Final Verdict

No CRITICAL or MAJOR finding. The delta is confined to the two reworded error texts, their exact
test expectations, the spec and one auditor sentence, as claimed; fmt, clippy (`-D warnings`) and
the `soos-remote` + `soos-invariants` suites are green on this fingerprint. Remaining findings are MINOR and
non-blocking.

**VERDICT: APPROVED**
