# Plan Evaluation Report
- **Date**: 2026-10-06
- **Issue**: GitHub-only issue — Remote companion for real-time lock status and remote lock, `soos-remote` (GitHub #339; no `AI/BACKLOG.md` entry; draft PR #340, not merged)
- **Branch**: `feat/remote-companion`
- **Base commit**: `222665f` (`origin/main`); branch head `0b0b50c` holds the committed revision-3 crate
- **Plan evaluated**: `AI/architect_spec_remote_companion.md` **section 13 only — Revision 4, D5a′, as amended by the Revision 5 change log (§13.4)** (round 2 for this revision)
- **Scope of this evaluation**: the D5a′ host rule (`X-Forwarded-Host` / `X-Forwarded-Proto`), the `scripts/install_remote.sh` git mode and its static contract RMC-S12, the documentation contract RMC-S7b, and the "iPhone home screen and Shortcuts" documentation section drafted in the working tree. Sections 0–12 were evaluated in the earlier rounds and are not re-opened here.
- **Round history**:
  - Rounds 1–3 (Revisions 1–3): Revision 3 `APPROVED`.
  - Round 1 of Revision 4: `REVISION_REQUIRED`, findings R4-1 … R4-9.
  - Round 2 of Revision 4 (this report): every R4 finding has a resolution in §13.2 / §13.3 / §13.4 and §3 / §8; two new MINOR observations (R4b-1, R4b-2), none blocking.

## 1. Coverage Matrix

The issue has no backlog entry; the lines below are the round-1 findings, the §13.3 change items and the #339 acceptance items that §13 touches.

| Acceptance line / finding / change item | Spec element | Status |
|---|---|---|
| R4-1 (MAJOR) proto optional with XFH | §13.2 "Transport": with XFH present, `X-Forwarded-Proto` MUST be present exactly once, OWS-trimmed, ASCII-case-insensitive `https`; absent / repeated / other → `NotAllowed`; without XFH the proto is optional-but-`https`. Rows "XFH + proto absent → `NotAllowed`", "`Host` only + proto absent → OK"; e2e `421` without proto | **Resolved** |
| R4-2 (MAJOR) `Host` with XFH under-specified | §13.2 "Effective host": `Host` not inspected when XFH present; `Missing` = neither header; fixed evaluation order (1)–(4); `HostError` doc comments rewritten, `Display` texts and `421` mapping unchanged. Rows: XFH + `Host` absent → OK; XFH + `Host` repeated → OK; neither → `Missing`; XFH twice (+ bad proto) → `Repeated` | **Resolved** |
| R4-3 test power | §13.2 "Normalisation" explicit; rows `https` twice (identical), `HTTPS`, `" https "`, `" PC.Tail1234.TS.NET:443 "`, `:8443`, `100.64.0.1`, comma list, empty, trailing dot, allowlist cross rows; e2e verbatim head, `202` with `Origin: https://<xfh>`, `403` with `Origin: https://localhost`, no `LockSession` call on `403` | **Resolved** (one optional strengthening, R4b-2) |
| R4-4 docs / matrix drift | §13.3 rows for `Docs/REMOTE_COMPANION.md` §2 (lines 47–52), §4 (rename, redacted evidence), §5 (`allowed_hosts` row), §7 (`421` row), §8 (bullet replaced); RMC4 text; RMC20 → ✅ with §13.1 evidence; RMC21 stays ⬜; walkthrough lines 8, 73, 153; RMC-S7b as a **new** test (RMC-S7 untouched) | **Resolved** (line numbers verified, see §2) |
| R4-5 script mode contract | §8 RMC-S12 `test_rmc_s12_installer_scripts_are_executable`, `std::fs::metadata` + `PermissionsExt`, no `git` subprocess; staged mode change noted | **Resolved** with a fact correction (R4b-1) |
| R4-6 Shortcuts factual nit | §13.3 row: `Self.DNSName` trailing dot dropped / `tailscale serve status`; `misdirected_request` listed with a pointer to section 7 | **Resolved** |
| R4-7 unverifiable iOS claims | §13.3 row: text hedged (*Request Body: File*, error sentence), owner procedure named (who, how, recorded as RMC21 evidence) | **Resolved** |
| R4-8 redaction | §13.3: redaction rule for every repository file; e2e fixtures `pc.tail1234.ts.net` / `owner@example.com` / `100.64.0.1` | **Resolved** |
| R4-9 trust wording | §13.2 "Trust argument": exactly three overwritten headers named; `X-Forwarded-For` ignored by the service (verified: no source, test or contract mentions `forwarded`) | **Resolved** |
| Phase 4 owner check of the `Host` forwarding (D5a, matrix RMC20) | §13.1 evidence; consistent with the code path (`Host: localhost` → `NotAllowed` → `421` on the revision-3 build) | Covered |
| F5 Host / DNS rebinding defence stays in force | §13.2 effective host rule, same `is_valid_host_name` / `*.ts.net` / `allowed_hosts` pipeline, `421` otherwise | Covered |
| `Origin` comparison uses the effective host | §13.2 last bullet; `server.rs:496` binds the returned value and `server.rs:552` passes it to `check_lock_csrf` — no code change needed outside `check_host` | Covered |
| Existing tests untouched | §13.3 "new test functions only"; the three `check_host` tests and every `server_tests` test send no `X-Forwarded-*` header (`with_identity` sends `Host` + `Tailscale-User-Login` only) | Verified |
| Confidentiality of the recorded evidence | §13.1 redacted; §13.3 redaction rule | Covered |

## 2. Facts Verified Against Code

| Fact cited by plan | Code location | Actual value | Match |
|---|---|---|---|
| `check_host` signature `(&[(&str, &[u8])], &[String]) -> Result<String, HostError>` | `crates/remote/src/identity.rs:99-102` | identical | ✅ |
| `HostError` variants `Missing` / `Repeated` / `NotAllowed`, `Display` "host header missing" / "host header repeated" / "host not allowed", all → `421` | `identity.rs:29-40`, `server.rs:496-507`, `identity_tests.rs:263-275` | identical; the echo test pins the `Display` texts, which §13.3 keeps | ✅ |
| Header names lowercased, values OWS-trimmed by the parser; repeated headers kept as separate entries | `http.rs:140-141` (`to_ascii_lowercase`, `trim_ows`), `headers.push(...)` per header | yes; `single_header` additionally compares ASCII-case-insensitively | ✅ |
| `MAX_HEADERS` 32, `MAX_HOST_LEN` 253 | `lib.rs:48`, `lib.rs:82` | 32 / 253 | ✅ |
| `IDENTITY_HEADER` / `ACTION_HEADER` / `ACTION_LOCK` lowercase constants exist; `FORWARDED_*` do not yet | `lib.rs:90-94`; no `FORWARDED` in `lib.rs` | §3 row and §13.3 add the three constants | ✅ |
| `check_lock_csrf(head, normalized_host)` accepts `https://<host>` / `https://<host>:443`, ASCII-lowercased | `routes.rs:108-133` | yes | ✅ |
| Lock success `202 {"result":"lock_requested"}`; `421` body `misdirected_request`; `403` body `forbidden` | `server.rs:500-512`, `server.rs:603` | yes | ✅ |
| `scripts/install_remote.sh` mode `100644` in `HEAD`, `100755` staged, file executable on disk | `git diff --cached --summary`, `git ls-files -s`, `ls -la` | `mode change 100644 => 100755`; index `100755`; `-rwxr-xr-x` | ✅ |
| "no prior contract checks any script's mode" (§8 RMC-S12, §13.3) | `tests/invariants/src/lib.rs:990-1000` asserts `scripts/install.sh` is executable; `installer_contract.rs:925-935` checks `check_build_deps.sh`; `lib.rs:1160-1170` checks `provision_master_key.sh` | **inaccurate**: `scripts/install.sh` is already covered; only `scripts/install_remote.sh` has no mode contract | ❌ (R4b-1, MINOR) |
| Invariants resolve paths through `workspace_root()` (`CARGO_MANIFEST_DIR`) | `remote_companion_contract.rs:30-31` | yes | ✅ |
| RMC-S7 needle list does not include `X-Forwarded-Host` etc.; adding RMC-S7b leaves it untouched | `remote_companion_contract.rs:742-783` | needles: `tailscale serve --bg unix:`, `allowed_logins`, `allowed_hosts`, `LockedHint`, … | ✅ |
| `Docs/REMOTE_COMPANION.md` strings to be removed by RMC-S7b occur only in the section to rewrite | lines 131 (`(pending)`), 144 (`pending owner verification`), 235 (`pending owner verification`) | §13.3 rewrites §4 and replaces the §8 bullet, so both strings disappear | ✅ |
| Docs line references: §2 trust bullets 47–52; §5 `allowed_hosts` row; §7 `421` row | `Docs/REMOTE_COMPANION.md:47-52`, `:181`, `:218` | match | ✅ |
| Walkthrough lines 8, 73, 153 | `AI/walkthroughs/183_remote_companion.md:7-8`, `:72-74`, `:153` | "RMC20 and RMC21 pending owner checks", "The evaluator also required the Phase 4 verification", "the `tailscale` host check is recorded as pending (constraint 35)" | ✅ |
| Matrix rows RMC4, RMC16, RMC19, RMC20, RMC21 | `AI/VERIFICATION_MATRIX.md:2034, 2046, 2049, 2050, 2051` | RMC20 / RMC21 ⬜ Pending; RMC4 text is D5a | ✅ |
| ADR 2026-10-05 item (8) wording "requires the `Host` to be an allowed `*.ts.net` name … pending owner verification (matrix RMC20)" | `AI/DECISIONS.md:121` | present, to be amended per §13.3 | ✅ |
| Shortcuts draft says `Self.DNSName`, "Request Body: leave it empty", lists six `result` values without `misdirected_request` | `Docs/REMOTE_COMPANION.md:150, 161, 165-166` (working tree) | as described; §13.3 rows fix all three | ✅ |
| `tailscale serve` observed head (`Host: localhost`, XFH, XFP, `Tailscale-User-Login`, `X-Forwarded-For`) | §13.1 — cannot be re-run here (agents may not run `tailscale`, constraint 35) | consistent with Go's `httputil.ReverseProxy` behaviour for Unix backends and with the recorded `421` | accepted as owner evidence |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: a tailnet peer forges `X-Forwarded-Host: evil.com` / `X-Forwarded-Proto: http` to bypass the DNS-rebinding defence or to make the service accept plain HTTP. §13.1 records that Serve overwrites exactly those two headers (and the identity header) with the real values; the only other path to the socket is the `0600` file reachable by `tailscaled` and the owner. A local forger is the owner. A Serve release that stops sending XFH falls back to `Host: localhost` → `421` (fail-closed, documented in the new §8 bullet). `X-Forwarded-For` is never read.
- Second scenario: an effective host accepted from XFH that is not a `*.ts.net` name (`evil.com`, IP literal, comma list, `:8443`, trailing dot) — all refused by the unchanged `is_valid_host_name` + suffix / allowlist pipeline (§13.2 "Normalisation").
- Result: PASS.

### Pillar 2 — PAM deadline & concurrency
- Not applicable: `crates/pam` and the daemon are untouched; `check_host` stays pure and synchronous, bounded by `MAX_HEADERS` (32) iterations and one `MAX_HOST_LEN`-bounded `String`.
- Result: PASS (out of scope by construction).

### Pillar 3 — Panic safety & fail-closed
- Failure scenario considered: a non-UTF-8 `X-Forwarded-Host` or `X-Forwarded-Proto` value. The D5a pipeline maps `from_utf8` failure to `NotAllowed`; the spec's "any invalid DNS name → `NotAllowed`" covers the host, and "any other value → `NotAllowed`" covers the proto. No `unwrap`/`expect`/indexing is required by the evaluation order. The invariant `test_rmc_production_code_never_panics_or_prints` stays in force over `crates/remote/src`.
- Second scenario: an implementation that treats a missing proto as acceptable when XFH is present (the revision-4 round-1 defect) now fails the row "XFH + proto absent → `NotAllowed`" and the e2e "captured head without `X-Forwarded-Proto` → `421`".
- Result: PASS.

### Pillar 4 — Dependencies
- No new crate, feature or dependency; three string constants in `lib.rs`; `#![forbid(unsafe_code)]` unchanged (RMC-S1). `cargo deny` surface unchanged.
- Result: PASS.

### Pillar 5 — Data confidentiality
- Failure scenario considered: the §13.1 capture leaking the owner's login, tailnet name, 100.x address or profile URL into the walkthrough, docs, matrix, fixtures or commit messages. §13.3 states the redaction rule for every repository file and pins the synthetic names for fixtures; RMC-S7b does not require any real value. `debug!(%err, "misdirected request")` logs the fixed `Display` text only; `Display` texts are unchanged and pinned by `test_rmc_identity_errors_never_echo_values`.
- Result: PASS.

### Pillar 6 — Test integrity
- Failure scenario considered: an existing test is edited to accommodate D5a′. Verified: the three existing `check_host` tests send only `Host` lines (`identity_tests.rs:152-260`), `with_identity` sends `Host` + `Tailscale-User-Login` (`server_tests.rs:431-435`), and no test or contract contains `forwarded`; the spec adds RMC-S7b and RMC-S12 as new tests instead of editing RMC-S7. The §13.3 row "XFH + `X-Forwarded-Proto: https` **twice**" kills a first-header-wins implementation; "XFH + `Host` repeated → OK" kills an implementation that still counts `Host`; the allowlist cross row "XFH `other.tail1234.ts.net` + `Host: mypc.tail1234.ts.net` → `NotAllowed`" kills a fall-back-to-`Host` implementation.
- Result: PASS (R4b-2 proposes one extra discriminating row; non-blocking).

## 4. Findings

- **[MINOR] R4b-1 — RMC-S12 fact correction.** §8 RMC-S12 and the `scripts/install_remote.sh` row of §13.3 state "no prior contract checks any script's mode". `tests/invariants/src/lib.rs:990-1000` already asserts `scripts/install.sh` is executable, and `installer_contract.rs:925-935` / `lib.rs:1160-1170` check two other helpers. Required spec change: reword to "no contract checks the mode of `scripts/install_remote.sh`; `scripts/install.sh` is already covered by `tests/invariants/src/lib.rs`", and let `test_rmc_s12_installer_scripts_are_executable` assert `scripts/install_remote.sh` only (asserting `install.sh` a second time is harmless but redundant). Also replace `std::fs::metadata(path)?` with `.expect(...)` in the contract text: a `#[test]` without a `Result` return cannot use `?`, and the neighbouring invariants use `expect`.
- **[MINOR] R4b-2 — One more discriminating row.** In the no-allowlist table, pair the invalid-XFH rows with a **valid** `Host` (`X-Forwarded-Host: evil.com` + `Host: pc.tail1234.ts.net` → `NotAllowed`; `X-Forwarded-Host: 100.64.0.1` + `Host: pc.tail1234.ts.net` → `NotAllowed`) so that a fall-back-to-`Host` implementation also fails without an allowlist, and add `X-Forwarded-Host: \xff.ts.net` → `NotAllowed` (non-UTF-8) mirroring the existing `Host` row. The tester may add these without a spec revision; recorded here so the Phase 2 contract carries them.

No CRITICAL or MAJOR finding. Both minor items can be folded into the tester contract and the RMC-S12 text without re-opening the design.

## 5. Verdict

Revision 4 (D5a′) as amended by the Revision 5 change log closes both MAJOR findings of round 1: the transport header is mandatory and exactly one `https` whenever `X-Forwarded-Host` is present, and the `Host` handling with `X-Forwarded-Host` present is fully specified with a fixed evaluation order and discriminating rows. The documentation, matrix, walkthrough and ADR drift is enumerated line by line and verified against the current files; the installer mode has a static contract; the Shortcuts section is corrected on its one verifiable fact and hedged on the two iOS behaviours no agent can verify, with the owner's verification procedure named. The remaining items are a fact correction in the RMC-S12 text and one extra test row, both MINOR.

VALIDATION_VERDICT: APPROVED
