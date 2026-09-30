# Walkthrough 129 — Shared Fixtures Crate and Hardened Invariant Lexing

**Issues**: GitHub #241 (review finding TCI-10), GitHub #242 (review finding TCI-11)
**Branch**: `test/p2-fixtures-invariant-lexing`
**ADR**: `AI/DECISIONS.md` — "Shared Test Fixtures Crate and Hardened Invariant Lexing"
**Matrix**: `AI/VERIFICATION_MATRIX.md` component `fixtures-and-invariant-lexing` (FIL1–FIL9)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`) found two gaps in
the test tooling:

- **TCI-10**: `tests/fixtures/mod.rs` was not a crate. It was reachable only through
  `#[path = "../../../tests/fixtures/mod.rs"]` includes (three test files today: vision
  `pad_tests.rs`, daemon and enrollment-cli `pad_wiring_tests.rs`), carried a blanket
  `dead_code` allowance and an unused `embeddings` module that built **128D** vectors, while
  the embedding contract is 512D. `AI/MOCK_STRATEGY.md` described JPEG / tensor fixtures that
  never existed.
- **TCI-11**: the deterministic gates had lexing holes:
  - `extract_production_code` (`tests/invariants/src/lib.rs`) treats any line containing
    `#[cfg(test)]` as the start of a braced block, so `#[cfg(test)] mod tests;` or a
    `#[cfg(test)] use` line swallows the production code that follows;
  - the PAM unwrap invariant only matches `.unwrap()` / `.expect(`;
  - the Tokio invariant only reads the text of `crates/pam/Cargo.toml` (a transitive Tokio
    would pass);
  - `scripts/candid_review.sh` pipes the PAM audits through `grep -v 'tests'` (a production
    line such as `let attests = x.unwrap();` is dropped), lists the English words `pour` and
    `attention` as French markers, and uses a two-dot `git diff origin/main` in most audits,
    so once `main` moves past the branch point, reversed upstream hunks are audited.

## 2. Architect Design

### 2.1 Fixtures crate (TCI-10)

- `tests/fixtures/Cargo.toml` declares the workspace member `soos-test-fixtures` with
  `[lib] path = "mod.rs"`, `publish.workspace = true` and workspace lints; its only
  dependency is `soos-camera-v4l` (for `Frame`). It is dev-only: no crate may list it under
  `[dependencies]`.
- `tests/fixtures/mod.rs` keeps `synthetic`, `pad` and `onnx`; the unused 128D `embeddings`
  module is deleted (no test referenced it: `grep -rn 'embeddings::' crates tests` is empty).
- The three legacy `#[path]` includes live in pre-existing test files, which this change may
  not edit (test integrity rule). They are frozen: a new include fails
  `test_fil_fixture_path_includes_are_limited_to_legacy_files`. The file-level `dead_code`
  allowance is kept only for those includes, and its reason says so. Converting them is a
  pending proposal (section 6).

### 2.2 Hardened lexing (TCI-11)

`tests/invariants/src/lexing_contract.rs::production_code`:

1. `sanitize` blanks line and (nested) block comments and the contents of string, byte
   string, raw string (`r#"..."#`) and char literals, keeping lifetimes and every newline.
2. At the start of a line, an attribute whose compact text is `#[cfg(test)]` or starts with
   `#[cfg(all(test,` gates the next item. The item is removed up to its first `;` at bracket
   and parenthesis depth 0 (`mod tests;`, `use ...;`, `const X: [u8; 2] = ...;`) or, when a
   `{` comes first, up to the matching `}` (`mod tests { ... }`, `fn`, `impl`). Further
   attributes (single or multi-line) between the gate and the item are skipped with it.
3. Line numbers are preserved, so reports cite real lines.

`scripts/candid_review.sh`:

- `MERGE_BASE=$(git merge-base "$BASE_REF" HEAD)` is computed once, before any audit, and
  every `git diff` uses it (the working tree is still included).
- The PAM panic and print audits call `pam_production_additions <ERE>`: for each changed
  `crates/pam/src/*.rs` file, an embedded awk program (`RUST_PRODUCTION_FILTER`, the same
  item rule as the Rust extractor, single-line literals only, POSIX awk) extracts the
  production code at the merge base and in the working tree, and only the added production
  lines are matched. `grep -v 'tests'` is gone. Audit 2 also matches `panic_any(`.
- `pour` and `attention` are removed from the French marker list.

New static checks:

- `test_fil_pam_production_code_has_no_panicking_constructs`: `.unwrap()`, `.expect(`,
  `panic!(`, `unreachable!(`, `todo!(`, `unimplemented!(`, `panic_any(` and
  `#[allow(clippy::unwrap_used | expect_used | panic | unreachable | todo | unimplemented |
  indexing_slicing)]` are forbidden in PAM production code. Only `fault_injection.rs` is
  exempt (compiled solely with `--features fault-injection`, GitHub #148).
- `test_fil_pam_dependency_graph_has_no_async_runtime`: `cargo tree --locked -p soos-pam -e
  normal --target all --all-features --prefix none` must not contain `tokio`, `async-std`,
  `smol`, `async-io` or `async-executor`; the same query on `soos-daemon` must see `tokio`
  (non-vacuity). A failing `cargo tree` fails the test (fail-closed).

## 3. Tester Contract (Red Evidence)

All tests are new; no pre-existing test line was changed. The extractor tests were first
run against a temporary copy of the legacy algorithm (switch removed afterwards), the other
tests against the unmodified script and fixtures:

```
FIL_RED=1 cargo test -p soos-invariants fil_
test fixtures_contract::test_fil_fixtures_carry_no_stale_128d_embeddings ... FAILED
test fixtures_contract::test_fil_fixtures_is_a_workspace_member_crate ... FAILED
test lexing_contract::test_fil_extractor_strips_cfg_test_items_but_not_cfg_not_test ... FAILED
test lexing_contract::test_fil_extractor_ignores_braces_in_literals_and_comments ... FAILED
test lexing_contract::test_fil_extractor_keeps_code_after_cfg_test_use ... FAILED
test lexing_contract::test_fil_extractor_strips_test_module_with_multiline_attributes ... FAILED
test lexing_contract::test_fil_extractor_keeps_code_after_cfg_test_mod_declaration ... FAILED
test candid_review_contract::test_fil_candid_flags_pam_unwrap_on_line_containing_tests_substring ... FAILED
test candid_review_contract::test_fil_candid_allows_unwrap_inside_cfg_test_module ... FAILED
test candid_review_contract::test_fil_candid_accepts_english_attention_and_pour ... FAILED
test candid_review_contract::test_fil_candid_ignores_upstream_commits_after_branch_point ... FAILED
test result: FAILED. 6 passed; 11 failed
```

The six tests that passed on the old code are guards (runtime-dependency and `#[path]`
ratchets, the PAM panic and async-runtime checks, the French non-regression, and the
`mod tests;` candid case, which the old script caught only because it had no cfg filter).

The candid tests build a scratch git repository under `target/candid_review_contract/`,
copy the real script, fake `origin/main` with `git update-ref`, strip every `GIT_*`
variable (the tests may run inside the pre-commit hook) and use a private `HOME` and
`GIT_CONFIG_GLOBAL=/dev/null`. The French sample is assembled at run time so the test
source itself carries no marker.

## 4. Audit

- No production crate changes; no `unsafe`; no Tokio or new dependency in `crates/pam`.
- `soos-test-fixtures` is dev-only (enforced) and generates data only: no image, frame
  capture, embedding or credential.
- The scratch repositories live under the workspace `target/` directory and are removed on
  drop; no network access, no write outside `target/`.
- `cargo tree` runs with `--locked`; a failure is an assertion failure, never a pass.

## 5. Implementation

| File | Change |
|---|---|
| `Cargo.toml` | `tests/fixtures` added to the workspace members |
| `tests/fixtures/Cargo.toml` | new crate `soos-test-fixtures` |
| `tests/fixtures/mod.rs` | header rewritten, 128D `embeddings` module removed, allowance reason narrowed |
| `tests/fixtures/tests/fixture_generators_tests.rs` | crate self-tests (consumed as a normal crate) |
| `scripts/candid_review.sh` | merge-base everywhere, awk production filter, French list |
| `tests/invariants/src/{fixtures_contract,lexing_contract,candid_review_contract}.rs` | new contract modules, registered in `lib.rs` |
| `AI/MOCK_STRATEGY.md` §3, `Docs/CI_CD_AND_SECURITY.md` §5, `project-facts.md` | documentation |

## 6. Test Changes Awaiting Approval

The binding test-integrity rule forbids editing pre-existing test lines, so these parts are
left as proposals:

1. Replace `#[path = "../../../tests/fixtures/mod.rs"]` + `mod fixtures;` by
   `use soos_test_fixtures as fixtures;` (and add `soos-test-fixtures` to
   `[dev-dependencies]`) in `crates/vision/tests/pad_tests.rs`,
   `crates/daemon/tests/pad_wiring_tests.rs` and
   `crates/enrollment-cli/tests/pad_wiring_tests.rs`; then empty `LEGACY_PATH_INCLUDES` and
   drop the `dead_code` allowance in `tests/fixtures/mod.rs`. No assertion changes.
2. Make the legacy `extract_production_code` in `tests/invariants/src/lib.rs` delegate to
   `lexing_contract::production_code`, so the existing unwrap and evidence-store network
   invariants use the hardened lexer (stricter, never weaker).

## 7. Verification

`cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features
-- -D warnings`, `cargo test --locked --workspace --all-targets --all-features --no-fail-fast`,
`./scripts/candid_review.sh` and `bash -n scripts/candid_review.sh` pass.
