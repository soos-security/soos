# Development Workflow, Branching Strategy, and Pull Requests

> Project: `soos`  
> Audience: Human Contributors & AI Agents  

---

## 1. Cardinal Rule: Never Commit Directly to `main`

The `main` branch is protected. All development (new features, bug fixes, tests, tooling, documentation) must strictly occur on a dedicated topic branch before being merged via a Pull Request. Direct commits to `main` are physically prevented by git hooks.

---

## 2. Topic Branch Naming Conventions

| Prefix | Usage | Examples |
|---|---|---|
| `feat/<name>` | New system feature, crate, or major enhancement | `feat/policy-crate`, `feat/daemon-skeleton` |
| `fix/<name>` | Bug fix, security patch, or behavioral correction | `fix/pam-timeout-fallback`, `fix/zeroize-leak` |
| `test/<name>` | Test suite additions, fuzzing harness, or fixtures | `test/fuzz-codec-v1`, `test/docker-pamtester` |
| `chore/<name>` | Maintenance, tooling, CI/CD, documentation, deps | `chore/cargo-deny-rules`, `chore/commit-convention` |

---

## 3. Mandatory Context Ingestion (AI/ and Docs/ Mandate)

Before designing or implementing any code, contributors and AI agents **MUST** ingest the complete architectural and security context:
- `AI/ARCHITECTURE.md` — Master system architecture, threat model, security invariants
- `AI/DECISIONS.md` — ADR register preventing architectural hallucinations
- `AI/BACKLOG.md` — Complete development backlog, sub-issues, and acceptance criteria
- `AI/VERIFICATION_MATRIX.md` — Formal component verification matrix
- `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` — Compiler profiles, workspace lints, PAM constraints
- `Docs/COMMIT_CONVENTION.md` — Conventional Commits 1.0.0 specification
- `Docs/IPC_PROTOCOL.md` — IPC framing and codec constraints

---

## 4. Strict Test Integrity Invariant (Zero Test Weakening)

> [!CAUTION]
> **TESTS ARE CONTRACTUAL SPECIFICATIONS**:
> Under **NO circumstances** is an AI agent or contributor permitted to modify, weaken, delete, or bypass an existing test to make it pass with broken or incomplete production code.
> 
> - If a test fails: **The engineer or AI must persevere, debug, and fix the production implementation** until it satisfies the test contract.
> - Weakening tests to make them artificially green is considered an architectural invariant violation.

### 4.1 Timing-Sensitive Tests: No Fixed Sleep Before an Assertion

A fixed `thread::sleep` (or a tight poll bound) that guesses how long a background thread needs
before an assertion fails on a loaded host or a busy CI runner (GitHub #280).

- **Positive state** (camera ready, frame published, worker output present): poll the observable
  condition with a bounded helper such as `wait_until(SETTLE_TIMEOUT, || condition)` from the
  crate's `tests/common/mod.rs` (`SETTLE_TIMEOUT` = 5 s), then keep the original assertion
  unchanged. The poll returns as soon as the condition holds; a condition that never becomes true
  still fails the assertion after the bound.
- **Negative state** (camera NOT ready after an injected error): keep a short minimum dwell so that
  an implementation ignoring the fault would have time to misbehave, then poll for the expected
  state with the same bounded helper, then assert. A faulty implementation keeps the wrong state
  and fails after the bound; a correct one is never failed by scheduling delay.
- A sleep that only establishes a **lower bound** of elapsed time (for example "let the idle
  timeout expire") is allowed.
- Wall-clock latency assertions (benchmarks, "drop completes within N ms", idle windows of tens of
  milliseconds) cannot be fixed by polling. Their threshold, tolerance or gating is an assertion
  change and requires explicit user approval; propose it, never apply it silently.
- Migrating an existing test from a sleep to a bounded poll is a setup-only change: every
  assertion, threshold and tolerance stays identical, and each migrated test is listed in the PR
  and in the walkthrough.

---

## 5. Multi-Agent TDD Development Cycle (Phases 0 through 5)

### Agent Role Skills

The role definitions for each phase (`architect-agent`, `tester-agent`, `auditor-agent`, `developer-agent`, `candid-reviewer`, `plan-evaluator`, `traceability-agent`, and the `dev-workflow` orchestrator) live in `.agents/skills/<name>/SKILL.md`. The symbolic link `.claude/skills -> ../.agents/skills` exposes the same files to Claude Code as project skills, so both agent toolchains share a single source of truth. Edit skills only under `.agents/skills/`.


```
┌─────────────────────────────────────────────────────────────┐
│ Phase 0: Branch Creation                                    │
│ git checkout -b feat/<name>                                 │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Phase 1: Architect (Specification & Invariant Audit)        │
│ - Design structs, enums, traits, bounded types              │
│ - Verify alignment with AI/ARCHITECTURE.md invariants       │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Phase 2: Tester (TDD Red Phase)                             │
│ - Author unit and property tests BEFORE production code     │
│ - Tests MUST fail initially (red phase)                     │
│ - PAM_IGNORE fallback test mandatory for all PAM paths      │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Phase 3: Auditor (Static Security Audit)                    │
│ - Verify zero unwrap() or expect() in PAM pathways          │
│ - Enforce #![forbid(unsafe_code)] in business crates        │
│ - Check zero sensitive data leaked in logs or IPC structs   │
│ - Verify no stdout/stderr prints (println!/dbg!) in PAM     │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Phase 4: Developer (TDD Green Phase)                        │
│ - Write minimal production code satisfying test suite       │
│ - ZERO test modifications allowed (persevere on prod code)  │
│ - Format and lint: cargo fmt, cargo clippy -D warnings      │
│ - Update AI/VERIFICATION_MATRIX.md criteria                 │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Phase 5: Post-Implementation, Docs & Autonomous PR Loop     │
│ 1. Update technical documentation in Docs/                  │
│ 2. Author sequential walkthrough in AI/walkthroughs/NN_*.md │
│ 3. Execute autonomous pipeline: ./save.sh --auto-merge      │
│    - Quality gates (fmt, clippy, test, deny)                │
│    - Candid pre-push review (scripts/candid_review.sh)      │
│    - Push to origin and open Pull Request                   │
│    - Monitor CI checks (Quality, Security, PAM Docker)      │
│    - Streamlined auto-merge to main upon green CI           │
└─────────────────────────────────────────────────────────────┘
```

---

## 6. Conventional Commits Standard

All commit messages must strictly conform to the [Conventional Commits 1.0.0](COMMIT_CONVENTION.md) specification:
- Format: `<type>(<scope>): <subject>`
- Subject must be imperative, lowercase, under 72 characters, without trailing period.
- Always written in English.
- Validated automatically by `.githooks/commit-msg`.

---

## 7. Autonomous PR Loop & Streamlined Auto-Merge

For automated AI workflows or contributors desiring end-to-end automation:

```bash
# Autonomous loop: validation, candid review, push, PR, CI monitoring, and auto-merge:
./save.sh --auto-merge

# Or invoke the orchestration script directly:
./scripts/pr_loop.sh "feat(policy): implement authorization engine"
```

### Execution Steps in the Autonomous Loop:
1. **Local Quality Gates**: Executes `cargo fmt`, `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings`, `cargo test --locked --workspace --all-targets --all-features` (including architectural invariants), and `cargo deny --locked check` — the exact CI commands.
2. **Dual-Layer Candid Review**: `./scripts/candid_subagent.sh` runs the deterministic invariant audit (`scripts/candid_review.sh`: panic safety, `#![forbid(unsafe_code)]`, forbidden dependencies, shell syntax, output isolation, English policy) and verifies that `AI/candid_review_report.md` is `VERDICT: APPROVED` **and** bound to the current diff fingerprint (`--prepare` prints it for the reviewer). A stale or template report is rejected.
3. **Git Guardrails**: pre-commit refuses `main` and scans staged changes for secrets; commit-msg validates Conventional Commits; pre-push refuses pushes to (and deletion of) `main`, scans every unpushed commit and re-verifies the review fingerprint.
4. **Push & Pull Request**: Pushes the branch to GitHub and opens a Pull Request if not already created.
5. **CI Monitoring**: Waits until the PR head is the pushed commit and CI has started on it, then watches all GitHub Actions jobs (`lint`, `clippy`, `test`, `security`, `pam-integration`, `authselect-profile`, `pam-rollback`, `PR Title`) with fail-fast and a 45-minute ceiling, and finally polls the `CI Success` aggregate of that commit (created only once its dependencies finish) until it completes.
6. **Streamlined Auto-Merge**: Only if `CI Success` concluded `success` **on that exact commit**, the PR is squash-merged with `gh pr merge --squash --match-head-commit <validated sha>` (GitHub refuses the merge if the head moved; `--admin` is never used), and local `main` is fast-forwarded. The loop never stashes: if the working tree is dirty it stays on the topic branch.

---

## 8. Backlog and Issue Traceability (`scripts/sync_issue.py`)

`scripts/sync_issue.py` keeps `AI/BACKLOG.md` checkboxes and the GitHub issue bodies in step.
Its two tables are `BACKLOG_TO_GITHUB` (backlog id → GitHub issue, one distinct GitHub **issue**
per backlog id, never a pull request) and `BRANCH_TO_ISSUE` (topic branch → backlog id; several
branches may deliver the same backlog id).

| Command | Writes `AI/BACKLOG.md` | Calls GitHub | When |
|---|---|---|---|
| `--check` | no | no | offline self-check; run by `save.sh` before staging and by the invariant suite |
| `--subissue N.M [--local-only]` | yes (heading `#N` only) | yes unless `--local-only` | Phase 6, once per delivered sub-issue, **before** the commit |
| `--issue N --complete-all [--local-only]` | yes (heading `#N` only) | yes unless `--local-only` | only when every sub-issue of `#N` is genuinely delivered |
| `--auto --local-only` | no | no | `save.sh` before staging: reports the branch's still-open sub-issues |
| `--auto` | no | yes | after the push (`save.sh --push-pr`, `scripts/pr_loop.sh`): mirrors the committed checkboxes |
| `--print-github-issue` | no | no | prints the branch's GitHub issue number, if any |

- **Self-check** (`--check`): fails on a duplicated `### Issue #N` heading, a duplicated sub-issue
  id, two backlog ids sharing one GitHub target, or a table entry naming an unknown backlog id.
  Every syncing mode runs it first and refuses to act on inconsistent tables. It is enforced by
  `tests/invariants/src/sync_issue_contract.rs`.
- **Explicit completion**: `--auto` never ticks anything, so a push can no longer report partial
  work as complete. Backlog edits come only from `--subissue` / `--complete-all`, run before
  `save.sh`, so they are part of the committed and pushed diff and the tree is clean after a push.
- **Failures are visible and non-destructive**: every `gh` call has a 30 s timeout; the issue body
  is only PATCHed after a successful read, only checkbox markers change, and the progress comment
  is only posted after a successful PATCH. A failed sync exits with status 2 and prints
  `GitHub sync failed`; `save.sh` and `scripts/pr_loop.sh` print a warning with the re-run command
  and continue (the branch is already pushed). Neither script silences the script with `|| true`.
- **GitHub-only branches**: branches that fix a review finding tracked only as a GitHub issue (no
  backlog id, e.g. the `AI/reviews/FULL_PROJECT_REVIEW_2026-09-29.md` remediation) are
  intentionally **not** registered in `BRANCH_TO_ISSUE`. `--auto` is a visible no-op for them, and
  the commit message closes the issue explicitly with `Closes #N`. Register a branch only when it
  implements a `### Issue #N` heading of `AI/BACKLOG.md`.
