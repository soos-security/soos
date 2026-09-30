# Walkthrough 131 — Matrix Evidence Hardening and Packaging Follow-ups

- **Date**: 2026-09-30
- **Issue**: GitHub #281 ([TCI-FU] matrix citation invariant hardening, `rpm -U` key guard test
  and packaging follow-ups), including the checklist item of its first comment (`cmp` /
  `diffutils` in `%posttrans`)
- **Branch**: `test/p2-quality-followups`
- **Matrix criteria**: QFU1–QFU8 (new); P1–P4, PA6–PA8, GEPU4, TFL1, TFL2 and three global
  invariants now cite real evidence; DV1, DV2, DV3, PK5–PK7, DVM9 record the first CI runs
- **ADRs**: "GDM `timeout_ms=2500` Versus Daemon `connection_timeout_ms`", "Package Scriptlets
  Surface, Never Silence, Skipped Integration", "Verification-Matrix Rows Must Cite Checkable
  Evidence" (all 2026-09-30)

---

## 1. Checklist

| Item | Result |
|---|---|
| Matrix invariant passes silently on rows without citations, `proptest!` files count every `fn`, `#[cfg(all(test, ...))]` counts as a test | Fixed (QFU7, QFU8) |
| Docker `rpm -U` case for the `%pre`/`%posttrans` key guard | Added to `tests/docker/test_packages.sh` (QFU3); Docker run still pending (§5) |
| `postinst` runs `pam-auth-update ... \|\| true` without `--force` | Surfaced with a warning; `--force` deliberately not added (QFU4) |
| GDM `timeout_ms=2500` versus `connection_timeout_ms` | Documented; changing a value is left to the owner (QFU5, §6) |
| CI run URLs for DV1, PK5–PK7, DV2, DV3, DVM9 | Ubuntu and Fedora recorded; Arch has no green run yet (§4) |
| `%posttrans` `cmp` without `diffutils` | `Requires(posttrans): diffutils` and exit-status handling (QFU1, QFU2) |

## 2. Matrix Citation Invariant (`tests/invariants/src/matrix_citations.rs`)

- New `rows_without_resolvable_evidence`: a claimed row (or checked global invariant) must have
  at least one citation in its evidence columns that `resolve` accepts. Enforced by
  `test_matrix_claimed_rows_cite_at_least_one_resolvable_evidence`; the helper is self-tested by
  `test_matrix_rows_without_resolvable_evidence_are_reported` (a row with no code span, a row
  whose only path sits in the criterion column, a checked invariant without citation).
- `is_test_attribute`: only an attribute whose path is `test` or ends in `::test`
  (`#[test]`, `#[tokio::test]`, `#[tokio::test(flavor = ...)]`). `#[cfg(all(test, unix))]`,
  `#[cfg_attr(test, ...)]` and `#[path = "../tests/fixtures/mod.rs"]` no longer mark the next
  function as a test.
- The "every `fn` of a file containing `proptest!` is a test" shortcut is gone: every
  `proptest!` function in the workspace carries `#[test]`, so it is detected like any other.
- `prop_*` names are now classified as test citations, so rows citing only property tests
  (P5, MJB6) are really checked instead of passing with zero citations.
- Existing tests of the file are unchanged (only the helpers they call changed); the
  repository-wide check `test_matrix_claimed_rows_cite_only_existing_evidence` still passes with
  the tightened detection.

Rows fixed with real evidence (none downgraded): P1–P4 (`crates/protocol/src/codec.rs` unit
tests and `property_tests::prop_*`), PA6 (`test_protocol_request_and_response_have_no_sensitive_fields`),
PA7, PA8 (`tests/docker/test_suite.sh`, `distro_matrix::test_pam_matrix_t6_and_t8_can_fail`),
GEPU4 (walkthrough 67 plus the import tests), TFL1, TFL2 (helpers, walkthrough 107 and three of
the migrated tests), and the global invariants "no camera device opened by the PAM module"
(new `quality_followups::test_pam_crate_never_opens_camera_devices`), "daemon unavailable =
password fallback" and "the `.so` never panics across FFI".

## 3. Packaging

- `packaging/rpm/soos.spec`: `Requires(posttrans): diffutils`. `%posttrans` stores the `cmp`
  status: 0 discards the equal copy, 1 keeps it with the existing "differs" warning, anything
  else keeps both files with "could not compare ... (cmp exit status N)". Only a proven-equal
  copy is ever deleted. `tests/docker/Dockerfile.fedora` installs `diffutils` so `rpm -U`
  resolves the new scriptlet dependency.
- `packaging/debian/postinst`: `pam-auth-update --package --enable soos soos-notify` without
  `--force` (the administrator's local stack is never overwritten) and without `|| true`. A
  failure prints `soos: WARNING: pam-auth-update failed ...`; a `common-auth` without
  `pam_soos.so` afterwards prints a warning naming `pam-auth-update --enable soos soos-notify`.
  The installation continues in both cases.
- `tests/docker/test_packages.sh`: `verify_rpm_upgrade_keeps_ghost_owned_key` builds a minimal
  legacy fixture (`soos-0.1.0-0.legacy`, owns `/var/lib/soos/master.key` as
  `%ghost %attr(0600, root, root)`, generates it in `%post`), installs it, upgrades to the real
  RPM with `rpm -U`, and fails unless the key fingerprint is unchanged, no package owns the key
  and `.master.key.upgrade` is gone. Then the upgraded package is verified and removed.

Hermetic tests (`tests/invariants/src/quality_followups.rs`) execute the real `%posttrans` body
(macro redirected to a scratch directory, stub `cmp` exiting 0/1/2/127) and the real postinst
step 4 under `sh -e` (stub `pam-auth-update`: local-modification notice, failure, success).

## 4. CI Run URLs

Run 36745249037 (`main` push `e2f602c`, 2026-09-30) is the first with `distro-deploy`:

- `package-deploy` (ubuntu): green,
  https://github.com/Mysticaly622/soos/actions/runs/36745249037/job/109989794002 (DV1, PK5, DVM9).
- `distro-deploy` (fedora): green, "RPM package test passed cleanly",
  https://github.com/Mysticaly622/soos/actions/runs/36745249037/job/109989793726 (PK6, DV2).
- `distro-deploy` (arch): failed before packaging on a transient `ort-sys` prebuilt download
  error (`native-tls: unexpected EOF`),
  https://github.com/Mysticaly622/soos/actions/runs/36745249037/job/109989793805. PK7 and DV3
  keep "no green CI run URL yet"; a re-run (or the next `main` push) is needed.

## 5. Docker Evidence (`rpm -U`)

Scratch clone of this branch under the worktree parent directory, image built from
`tests/docker/Dockerfile.fedora` under a private tag, `bash tests/docker/test_packages.sh`
in it:

Not completed in this session: the first image build finished but the tagged image was gone before `docker run` (shared Docker host), and the rebuild was still running when the session ended. The `rpm -U` case is therefore only covered by the static contract `quality_followups::test_docker_package_test_covers_rpm_upgrade_from_ghost_owned_key` and the hermetic `%posttrans` test; the next `distro-deploy` (fedora) run on `main` or a manual Docker run must execute it.

## 6. GDM Timeout (decision left to the owner)

The daemon decides within min(client deadline, request start + `connection_timeout_ms`) minus
50 ms. With the default 1000 ms the GDM `timeout_ms=2500` buys no daemon time. This change only
documents it (`Docs/DISTRIBUTION_DEPLOYMENT.md` §2.1, ADR). Options for the owner: raise the
daemon default (daemon-wide; also lengthens how long a slow client holds a connection slot),
lower the GDM value (changes the `GDM_PAM_LINE` contract and its tests), or keep both and rely
on the documented `daemon.toml` tuning.

## 7. Red → Green

- Before the fix: `test_matrix_citation_parser_rejects_non_test_attributes_and_proptest_helpers`
  detected 7 "tests" instead of 2; `test_matrix_rows_without_resolvable_evidence_are_reported`
  returned `[]` instead of `["Y1", "Y2", "global-invariant@L7"]`; the repository check then
  listed 15 claimed rows without resolvable evidence. Five of the six `quality_followups` tests
  failed (`|| true` in postinst, missing `rpm -U` case, missing GDM note, missing
  `Requires(posttrans)`, `cmp` exit 2 reported as "differs"). The camera test passed from the
  start: it is new evidence for an existing claim.
- After: `cargo test -p soos-invariants --lib`: 124 passed.

## 8. Verification

`cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
`cargo test --locked --workspace --all-targets --all-features --no-fail-fast`,
`./scripts/candid_review.sh`, `bash -n tests/docker/test_packages.sh`, `sh -n packaging/debian/postinst`.
