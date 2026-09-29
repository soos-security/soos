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
5. **CI Monitoring**: Waits until the PR head is the pushed commit and CI has started on it, then watches all GitHub Actions jobs (`lint`, `clippy`, `test`, `security`, `pam-integration`, `authselect-profile`, `PR Title`) with fail-fast and a 45-minute ceiling, and finally polls the `CI Success` aggregate of that commit (created only once its dependencies finish) until it completes.
6. **Streamlined Auto-Merge**: Only if `CI Success` concluded `success` **on that exact commit**, the PR is squash-merged with `gh pr merge --squash --match-head-commit <validated sha>` (GitHub refuses the merge if the head moved; `--admin` is never used), and local `main` is fast-forwarded. The loop never stashes: if the working tree is dirty it stays on the topic branch.
