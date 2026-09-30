# Walkthrough 103 — PAM Deadline Contract and Runnable Physical Scripts

- **Date**: 2026-09-30
- **Issues**: Review findings TCI-02 (GitHub #185) and TCI-03 (GitHub #186)
- **Branch**: `fix/packaged-timeout-and-physical-scripts`
- **Matrix criteria**: PSC1–PSC4, PDC1–PDC2 (new)

---

## 1. Context & Objectives

The full project review (`AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md`) confirmed two defects in
tests and documentation:

1. **TCI-02**: every packaged PAM profile passes `pam_soos.so timeout_ms=250`, and two invariant
   tests assert that literal. The module default is 1000 ms (`crates/pam/src/config.rs`). The
   normative documents (`AGENTS.md`, `AI/ARCHITECTURE.md`, `AI/DECISIONS.md`, security
   guidelines) still describe a fixed "200 to 250 ms" deadline.
2. **TCI-03**: `enrollment_test.sh`, `multi_user_test.sh` and `adversarial_test.sh` pass
   `--skip-root-check`, which the CLI removed (EN11). Clap rejects it, so every script fails at
   its first `soos-enroll` call. The invariant `test_physical_hardware_validation_suite_spec`
   only grepped for subcommand words, so it could not see the break.

## 2. Architect Design

### 2.1 TCI-02: the deadline rule (ADR 2026-09-30 "PAM Deadline Derived From Clamped `timeout_ms`")

Facts checked in the code:

| Fact | Source | Value |
|---|---|---|
| PAM default / clamp | `crates/pam/src/config.rs` | `DEFAULT_TIMEOUT_MS` 1000, clamp 10–5000 ms |
| Daemon request deadline | `crates/daemon/src/dispatcher.rs` step 8, `inference.rs` | min(client deadline, start + `connection_timeout`) − `RESPONSE_WRITE_MARGIN_MS` (50 ms) |
| Daemon `connection_timeout` default | `crates/daemon/src/config.rs` | 1000 ms (`[dispatcher] connection_timeout_ms`) |
| Consensus | `crates/policy/src/pad_consensus.rs` | 3 captures for `Allow` |
| Embedding latency | walkthrough 102 | p50 127.5 ms, p95 170.9 ms (one embedding) |
| GDM | `crates/admin-cli/src/gdm.rs` | `timeout_ms=2500` |

With `timeout_ms=250` the daemon has about 200 ms, which is less than two embeddings. The
3-capture consensus can never finish, so every attempt falls back to the password. With the
1000 ms default the daemon has about 950 ms, which fits three captures at the measured p95 and
matches its own `connection_timeout`. When no face is present, the password prompt still appears
within about 1 s. Decision: the packaged console/sudo/login/polkit/lock-screen rules rely on the
module default, so they carry no `timeout_ms=`. GDM keeps 2500 ms. The enforced invariant becomes
"every blocking PAM operation has an explicit deadline derived from the clamped `timeout_ms`".

**Packaging not changed (blocked).** Removing `timeout_ms=250` from `packaging/pam/**` would break
assertions in two existing invariants. The project rule is to never edit an existing test
assertion without approval, so this change was stopped and reported instead (section 6).

### 2.2 TCI-03: a CLI contract for the physical scripts

`soos-enroll` calls `check_privileges(true)` before `Cli::parse()`. This means `--help` cannot run
without root, so the invariant has to read the clap definition itself. The new module
`tests/invariants/src/physical_contract.rs` parses `crates/enrollment-cli/src/args.rs` and collects:

- global flags (`global = true`), `-h`/`--help` everywhere, and `-V`/`--version` at the top level;
- the flags of each subcommand, taken from its `Args` struct. `Commands` variants map to kebab-case
  subcommands (`DebugVision` → `debug-vision`).

For each script it then joins `\` continuations, skips comments and heredocs, and resolves the
bash flag arrays (`CLI_COMMON_FLAGS=(...)`, `+=(...)`). It checks every `"${SOOS_ENROLL_BIN}"`
command-position invocation: the invocation must name a known subcommand, and each flag must be
accepted at its position. The module also rejects a literal `target/*/soos-enroll` command, which
would bypass the check.

## 3. Tester Contract (Red Evidence)

New tests (`cargo test --locked -p soos-invariants --all-features`), before the fix:

| Test | Red result |
|---|---|
| `physical_contract::test_enroll_cli_spec_parser_matches_clap_definition` | green (parser sanity; guards against a vacuous scan) |
| `physical_contract::test_physical_scripts_only_use_accepted_soos_enroll_flags` | FAILED: `--skip-root-check` rejected on every invocation of the three scripts; four literal `target/release/soos-enroll` calls in `adversarial_test.sh`; that script had no checked invocation |
| `physical_contract::test_physical_scripts_require_root_and_absolute_paths` | FAILED: no EUID gate in the three scripts; `MODELS_DIR="models"` is relative, and the CLI rejects relative paths |
| `physical_contract::test_adversarial_physical_session_uses_an_enrolled_store` | FAILED: the physical session created an empty `mktemp` store |
| `pam_deadline_contract::test_normative_docs_do_not_restate_a_fixed_pam_deadline` | FAILED: 14 findings (AGENTS.md:72, AI/ARCHITECTURE.md:16/51/120, AI/DECISIONS.md:8, Docs/SECURITY_AND_QUALITY_GUIDELINES.md:35, Docs/IPC_PROTOCOL.md:17, tests/physical/screensaver_test.md:25/144, project-facts.md:59) |
| `pam_deadline_contract::test_normative_docs_state_the_enforced_pam_deadline_invariant` | FAILED: AGENTS.md lacks the invariant wording |

During the red phase the first draft also counted `[[ ! -f "${SOOS_ENROLL_BIN}" ]]` as an
invocation. The new test now only counts a command-position occurrence. This fixed the new test
itself; no existing test was touched.

## 4. Audit

- No existing test was modified, weakened or deleted. The two new modules are additive.
- The scripts no longer run `cargo build` when the binary is missing, because under `sudo` that
  would leave root-owned outputs in `target/`. They print the build command instead.
- The adversarial session no longer counts an operational failure as a rejected attack: `verify`
  output without a `Verdict:` line aborts the session with exit 2. The script prints the CLI
  error on stderr. That text has no embedding, frame or score.
- The mock branch of `adversarial_test.sh` is unchanged, and
  `test_adversarial_mock_mode_never_fabricates_pad_metrics` still passes.

## 5. Implementation

- `tests/physical/enrollment_test.sh`, `multi_user_test.sh`:
  - dropped `--skip-root-check`;
  - added an EUID 0 gate with a `sudo` hint, placed after argument parsing so `--help` still works
    unprivileged;
  - made paths absolute (`realpath -m`);
  - `--models-dir` is passed only when given, so the CLI default `/var/lib/soos/models` applies;
  - the default UID is `SUDO_UID`, else the current UID;
  - the JSON UID match is exact (`"uid": N,`).
- `tests/physical/adversarial_test.sh`:
  - the physical session uses the provisioned store, with new `-b`/`-k` overrides;
  - it checks that the UID is enrolled (`list --format json`) before measuring;
  - the root gate is in the physical branch only, because `--mock` runs cargo tests and no
    `soos-enroll`;
  - the binary is discovered like in the other scripts, and one `verify_presentation` helper
    replaces the four literal invocations.
- Deadline wording updated: `AGENTS.md`, `AI/ARCHITECTURE.md` (§1 table, §3 diagram, async
  boundaries), `Docs/SECURITY_AND_QUALITY_GUIDELINES.md`, `Docs/IPC_PROTOCOL.md`,
  `tests/physical/screensaver_test.md`, `.agents/skills/dev-workflow/references/project-facts.md`.
  The 2026-09-12 ADR line keeps its text and gains a `*(Superseded in part ...)*` marker. The new
  ADR is appended to `AI/DECISIONS.md`.
- `Docs/INFERENCE_ORT_CRATE.md` now documents the physical PAD session requirements.

Runtime evidence (no camera on the host, `archlinux:latest` container as root, worktree mounted
read-only):

- `enrollment_test.sh --mock`: all six steps passed (enroll, list, mode 0600, verify `Allow`,
  delete, clean slate);
- `multi_user_test.sh --mock`: passed, including cross-user rejection;
- `adversarial_test.sh -d /dev/null -b /tmp/store -k /tmp/store.key -u 1000`: refused with exit 1
  (UID not enrolled);
- the same run after a mock enrollment of UID 1000: `verify` failed without a verdict (no models
  in the container), and the session aborted with exit 2.

Hardware re-runs of PH1/PH3/PH5 are still pending (no camera on this host).

## 6. Assertion Change Awaiting Approval (TCI-02 packaging)

To ship the ADR, the packaged primary rules become `pam_soos.so` with no `timeout_ms=`. The
existing tests below assert the literal `pam_soos.so timeout_ms=250`. Proposed replacement: assert
that the primary rule has the expected control and module, and has no `timeout_ms=` argument (or,
if one is present, that its value equals `DEFAULT_TIMEOUT_MS` = 1000).

- `tests/invariants/src/lib.rs` `test_pam_config_ordering_matches_spec`: lines 621 (Debian),
  641 (Fedora), 672 (Arch), 689 (Arch snippet).
- `tests/invariants/src/lib.rs` `test_fedora_authselect_profile_preserves_faillock_ordering`:
  lines 2381 and 2402.
- The Docker fixtures owned by the Docker-matrix work (`tests/docker/Dockerfile.{ubuntu,fedora,arch}`,
  `authselect_profile_test.sh:182,227`, `pam_rollback_test.sh:338,415`) grep the same literal.

## 7. Observation (out of scope)

The daemon's request deadline is also capped by its own `connection_timeout` (default 1000 ms). As
a result, the GDM `timeout_ms=2500` gives the daemon no extra time unless `[dispatcher]
connection_timeout_ms` is raised in `daemon.toml`. A follow-up issue should decide whether the
GDM budget or the daemon default must change.

## 8. Verification

`cargo fmt --all -- --check`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`,
`cargo test --locked --workspace --all-targets --all-features --no-fail-fast`,
`./scripts/candid_review.sh`, and `bash -n` on the four physical scripts.
