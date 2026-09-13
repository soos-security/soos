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
1. **Local Quality Gates**: Executes `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets` (including architectural invariants), and `cargo deny check`.
2. **Candid Pre-Push Code Review**: Runs `./scripts/candid_review.sh` to perform an impartial, context-free audit of the raw diff for panic safety, `#![forbid(unsafe_code)]`, forbidden dependencies (`opencv`, `nokhwa`), shell script syntax, output isolation, and strict English policy.
3. **Pre-Commit Guardrails**: Verifies active topic branch (refuses `main`), validates Conventional Commit message format, and scans for secret leaks.
4. **Push & Pull Request**: Pushes branch to GitHub and opens a Pull Request if not already created.
5. **CI Monitoring**: Monitors all 3 GitHub Actions jobs (`Quality`, `Security`, `PAM Integration Docker`).
6. **Streamlined Auto-Merge**: As soon as all CI checks are 100% green, the PR is automatically squash-merged into `main` (`gh pr merge --squash --delete-branch`), and local `main` is synchronized.
