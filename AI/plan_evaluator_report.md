# Plan Evaluation Report — Round 3
- **Date**: 2026-10-13
- **Issue**: GitHub-only (no `AI/BACKLOG.md` id) — macOS port foundation, GitHub #367 (first PR of #360: planned split items 1–3)
- **Branch**: `chore/macos-port-foundation`
- **Base commit**: `1b6d211`
- **Plan evaluated**: `AI/architect_spec_macos_port_foundation.md` (round 3: §4.1 rewritten around the new helper `scripts/pr_checks_gate.sh`; §11 twin fixed)
- **Owner decisions applied**: separate private `soos-macos` repository; no Mac available; `sudo` through `sudo_local` first; RGB-only convenience factor; ad-hoc signing, no notarization; a non-required macOS CI job outside `CI Success`; crates.io publication, soos-remote Mac support and the reference Mac model are deferred; `runs-on: macos-latest` (orchestrator).

## 1. Coverage Matrix

| Acceptance line (#360 / #367) / TDD test | Spec element | Status |
|---|---|---|
| ADR "[2026-10-xx] macOS Port in a Separate Repository" | D-1, D-2, D-3, D-13, §8; A1 | Mapped |
| `soos-frame` pure, forbid unsafe, holds the three types | D-6, §2; A2 | Mapped |
| camera-v4l re-exports; vision drops camera-v4l | D-7, D-8; A3 | Mapped |
| `Frame` wiped on drop, proven by a test | §11 probe + non-vacuity twin (now fully initialised), `test_frame_zeroize_empties_data_and_keeps_capacity` | Mapped |
| `frame` in the business-crate list | AM-1, `candid_review.sh`; A5 | Mapped |
| Existing tests unchanged except import paths | §12 AM-1..AM-3 (enumerated) | Mapped |
| `test_portable_crates_have_no_linux_only_dependencies` + broken-manifest proof | §3.2 (root excluded), hermetic stub `v4l`, per-group witnesses; A7 | Mapped |
| biometric-store classified with evidence | D-4, §8 items 7–8 | Mapped (documented deviation) |
| macOS CI job (check/clippy/test), non-required, Linux jobs unchanged | D-11, §4, §4.1 helper + `pr_loop.sh`; A9 (`test_mac_macos_job_is_pinned_secret_free_and_non_required`, `test_mac_pr_loop_treats_only_the_macos_job_as_advisory`, `test_mac_pr_checks_gate_decisions`) | Mapped |
| `Docs/MACOS_PORT.md` + owner checklist | D-12, §10 M-6; A10 | Mapped |
| `AI/BACKLOG.md` entry | §8 item 3 (GitHub-only precedent) | Justified deviation |
| Matrix rows + walkthrough | `MAC1`–`MAC9`, walkthrough 198 | Mapped |

## 2. Facts Verified Against Code

Facts from rounds 1 and 2 are unchanged on `1b6d211`. New facts for round 3:

| Fact cited by plan | Code / tool | Actual value | Match |
|---|---|---|---|
| `gh pr checks --json` exits 0 when checks failed | `gh pr checks 342 --json name,bucket` (gh 2.102.0) | exit 0; plain form exits 1 | ✅ |
| Check `name` is the bare job name (what the allowlist matches) | `gh pr checks 342 --json name,bucket,workflow` | e.g. `PR Title (Conventional Commits)` (workflow `PR Title`), `Clippy (-D warnings)` (workflow `CI`); the workflow is a separate field | ✅ (exact match on `name:` of the job is correct) |
| Check names contain shell/glob-relevant characters | same output | `(`, `)`, `,`, `$`, `{`, `}` (e.g. `PAM Matrix (${{ matrix.distro }})` for skipped matrix jobs) | ⚠️ the comparison must be a quoted literal (R3-2) |
| `@tsv` cannot emit a raw tab/newline inside a field | jq `@tsv` | escapes `\t`, `\n`, `\r`, `\\`; verified that a backslash is escaped as `\\` | ✅ (one record is one line with exactly two fields) |
| Bash `while IFS=$'\t' read -r name bucket` on input whose last line has no trailing newline | local reproduction (2 lines, no final `\n`) | **only the first line is read** | ⚠️ a dropped final line can drop a blocking check (R3-1) |
| Bash `read` with `IFS=$'\t'` on `\tfail` (empty name) | local reproduction | the tab is IFS whitespace, so the leading tab is stripped: `name=fail`, `bucket=` (empty) | ✅ fail-closed via "unknown bucket" (blocks) |
| `pr_loop.sh` runs under `set -euo pipefail` | `scripts/pr_loop.sh:17` | yes | ⚠️ the helper's exit codes 1/3 must be captured, not left to `set -e` (R3-3) |

## 3. Pillar Analysis

### Pillar 1 — Architecture & threat model
- Failure scenario considered: as in rounds 1–2, the port erodes Linux-only invariants.
- Result: PASS (unchanged).

### Pillar 2 — PAM deadline & concurrency
- Failure scenario considered: `soos-pam` reaches `soos-frame` or Tokio.
- Result: PASS (no change to `crates/pam`).

### Pillar 3 — Panic safety & fail-closed (merge gate)
Each case below was attacked against `scripts/pr_checks_gate.sh` as specified, with the outcome.
- **R2-1 scenario** (macOS job fails early, PR Title still pending): helper returns `3`, and `pr_loop.sh` waits within the `WATCH_START` budget, then exits 1. PASS.
- **Tab inside a name**: `@tsv` escapes it, so the record stays two fields. PASS.
- **Extra field**: `bucket` would hold `x\ty`, which is not an allowed bucket, so the helper blocks. PASS.
- **Empty name**: the leading tab is stripped, so the bucket is empty or unknown and the helper blocks. PASS, but by accident: the spec's "non-empty name" rule is unreachable through `read`.
- **Empty line or empty output**: blocks. PASS.
- **Null bucket**: renders as an empty bucket, so the helper blocks. PASS.
- **Advisory substring or near-miss name**: the exact-match test covers it. PASS.
- **Name match done as a glob or regex instead of a literal comparison**: open. The names contain `(`, `)`, `,`, `$`, `{`, `}`, and with `=~` the parentheses become a regex group. MINOR, R3-2.
- **Final line without a trailing newline**: real `gh --jq` output ends with `\n`, but an implementation that captures with `$(...)` and re-feeds through `printf '%s'` or a pipe silently loses the last record. If that record is the only non-advisory `fail`/`pending` (e.g. PR Title), the helper returns `0`. That is fail-open. MINOR, R3-1: implementation-dependent, so it is closed by a required test and a count cross-check.
- **Another check named exactly like the advisory job**: a job in another workflow, now or later, with the same name would have its failure silently downgraded. The textual pin checks only `ci.yml`. MINOR, R3-2.
- **Snapshot race**: a check created after the helper's `0` is a ci.yml job gated by `needs:`, and the following `CI Success` poll covers it. A late check from another workflow, or a re-run of PR Title, after `CI Success` is not seen. MINOR hardening, R3-4.
- **Shell plumbing**: `pr_loop.sh` calls the helper under `set -e` without capturing its status. That fails closed (any non-zero exit kills the loop), but the advisory and pending paths would never work, and the textual pin would not notice. MINOR, R3-3.
- **Gate inputs**: a missing or non-numeric argument blocks, a non-zero `gh` exit blocks, and an unknown bucket blocks. PASS.

### Pillar 4 — Dependencies
- Failure scenario considered: same as round 2 (misspelled forbidden name, `--target all` false positive, networked stub manifest).
- Result: PASS (unchanged since round 2).

### Pillar 5 — Data confidentiality
- Failure scenario considered: R2-2, the twin reading uninitialised bytes.
- Result: PASS. The twin is now `vec![0xA5; 4096]` with `len == capacity`. The wipe test reads only bytes that `zeroize` has written, and the probe stays address-keyed, uses atomics only and records `SEEN`.

### Pillar 6 — Test integrity
- Failure scenario A: the helper's behavioural test misses the R3-1 case. Its fixture list has no "blocking record is the last line, printed without a trailing newline".
  Result: MINOR (R3-1).
- Failure scenario B (process observation): the worktree already contains Phase 2 material while the plan is still under evaluation: `crates/frame/`, `crates/camera-v4l/tests/frame_drop_wipe_tests.rs`, `tests/invariants/src/portable_crates_contract.rs`, and the AM-1..AM-3 edits to 13 vision tests, the fixture test and `tests/invariants/src/lib.rs`.
  Result: not evaluated here. Those files must be reconciled with this round-3 spec (and the R3 items below) before the auditor reviews them, and the Red-phase evidence must be re-run against the final contract.

## 4. Round-2 Findings — Resolution Check

| Round-2 finding | Round-3 resolution | Status |
|---|---|---|
| R2-1 (MAJOR) merge while a non-advisory check is pending | Helper: every non-advisory check must be `pass`/`skipping`, at least one exists, `pending` → `3`. `pr_loop.sh` loops on `3` within the `WATCH_START` budget. Every other outcome blocks. `CI Success` stays the final step | ✅ Resolved |
| R2-2 twin reads uninitialised memory | §11 twin `vec![0xA5; 4096]`, `len == capacity` | ✅ Resolved |
| R2-3 textual-only pin | `test_mac_pr_checks_gate_decisions` with a `gh` shim, 14 cases including near-miss names and shim arguments | ✅ Resolved (one case missing, R3-1) |

Round-1 findings remain resolved (see round 2).

## 5. Findings (Round 3)

- **[MINOR] R3-1 — A final record without a trailing newline can be dropped (fail-open).** **Required change (Phase 2/3):**
  - **Parsing:** the helper must read every record, e.g. `while IFS= read -r line || [[ -n "$line" ]]`, splitting the line itself. Do not use the `IFS=$'\t'` field split, which also strips an empty name and makes the "non-empty name" rule unreachable.
  - **Count cross-check:** fetch the record count in the same `gh` call, e.g. `--jq '(length|tostring), (.[] | [.name,.bucket] | @tsv)'`, and block unless the parsed record count equals it.
  - **New test case:** add to `test_mac_pr_checks_gate_decisions` a shim that prints `advisory<TAB>fail\nother<TAB>pass\nPR Title<TAB>pending` **without** a final newline, expecting `3`. Add the same with `fail` as the last record, expecting `1`.

- **[MINOR] R3-2 — Advisory matching must be literal and unambiguous across workflows.** **Required change:**
  - **Literal comparison:** compare with a quoted literal (`[[ "$name" == "$advisory" ]]`), never with `=~`, `case` patterns or an unquoted right-hand side. The names contain `(`, `)`, `,`, `$`, `{`, `}`.
  - **Uniqueness pin:** extend `test_mac_pr_loop_treats_only_the_macos_job_as_advisory` to assert that the advisory name appears as a job `name:` exactly once across all `.github/workflows/*.yml`. A same-named job elsewhere would otherwise have its failure silently downgraded.
  - **Shim test case:** a fixture where the advisory name is followed by a trailing space must be treated as non-advisory.

- **[MINOR] R3-3 — Capture the helper's exit status explicitly under `set -e`.** **Required change:** specify `rc=0; ./scripts/pr_checks_gate.sh "$PR_NUMBER" || rc=$?` followed by a `case "$rc"` with `0`/`3`/`*` arms. Extend the textual pin to require that capture form, so the advisory and pending paths are reachable and not killed by `set -e`.

- **[MINOR] R3-4 — The helper's `0` is a snapshot taken before `CI Success`.** **Recommended change:** after `CI Success` reports `success`, call the helper once more and require exit `0` before `gh pr merge`; `3` or `1` means `exit 1`. This catches checks of other workflows that appeared or were re-run in between. It is cheap, and it closes the last window in which a non-advisory check outside `CI Success` could be unfinished at merge time.

## 6. Verdict

Every CRITICAL/MAJOR finding from rounds 1 and 2 is resolved:
- the guard's root exclusion;
- the advisory macOS job without the pending-check fail-open;
- a hermetic and witnessed guard;
- a sound drop-wipe probe.

The round-3 helper design is fail-closed for malformed lines, unknown or null buckets, tabs and newlines inside names, empty output, `gh` errors and bad arguments. The four remaining items are MINOR: implementation constraints and extra test cases that the tester and auditor must carry as binding constraints (R3-1 to R3-3 required, R3-4 recommended). The Phase 2 files already in the worktree must be reconciled with this final spec.

VALIDATION_VERDICT: APPROVED
