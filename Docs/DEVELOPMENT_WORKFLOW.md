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
| `feat/<name>` | New system feature, crate, or major enhancement | `feat/ipc-client-pam`, `feat/rate-limit-policy` |
| `fix/<name>` | Bug fix, security patch, or behavioral correction | `fix/pam-timeout-fallback`, `fix/zeroize-leak` |
| `test/<name>` | Test suite additions, fuzzing harness, or fixtures | `test/fuzz-codec-v1`, `test/docker-pamtester` |
| `chore/<name>` | Maintenance, tooling, CI/CD, documentation, deps | `chore/cargo-deny-rules`, `chore/commit-convention` |

---

## 3. Multi-Agent TDD Development Cycle (Phases 0 through 5)

For any code addition or modification, human developers and AI agents strictly follow this 6-phase cycle:

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
│ - Verify zero unwrap() or expect() in PAM paths             │
│ - Enforce #![forbid(unsafe_code)] in business crates        │
│ - Check zero sensitive data leaked in logs or IPC structs   │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Phase 4: Developer (TDD Green Phase)                        │
│ - Write minimal production code satisfying test suite       │
│ - Format and lint: cargo fmt, cargo clippy -D warnings      │
│ - Update AI/VERIFICATION_MATRIX.md criteria                 │
└──────────────────────────────┬──────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Phase 5: Post-Implementation, Docs & PR Loop                │
│ 1. Update technical documentation in Docs/                  │
│ 2. Author sequential walkthrough in AI/walkthroughs/NN_*.md │
│ 3. Execute autonomous pipeline: ./save.sh --auto-merge      │
│    - Quality gates (fmt, clippy, test, deny)                │
│    - Local hooks (Conventional Commits, secret scanner)     │
│    - Push to origin and open Pull Request                   │
│    - Solicit GitHub Copilot review and monitor CI           │
│    - Resolve review comments and auto-merge to main         │
└─────────────────────────────────────────────────────────────┘
```

---

## 4. Conventional Commits Standard

All commit messages must strictly conform to the [Conventional Commits 1.0.0](COMMIT_CONVENTION.md) specification:
- Format: `<type>(<scope>): <subject>`
- Subject must be imperative, lowercase, under 72 characters, without trailing period.
- Always written in English.
- Validated automatically by `.githooks/commit-msg`.

---

## 5. Autonomous PR Loop, Copilot Review, and Auto-Merge

For automated AI workflows or contributors desiring end-to-end automation:

```bash
# Autonomous loop: validation, push, PR, Copilot review, and auto-merge to main:
./save.sh --auto-merge

# Or invoke the orchestration script directly:
./scripts/pr_loop.sh "feat(protocol): implement bounded payload codec"
```

### Execution Steps in the Autonomous Loop:
1. **Local Quality Gates**: Executes `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets` (including architectural invariants), and `cargo deny check`.
2. **Pre-Commit Guardrails**: Verifies active topic branch (refuses `main`), validates Conventional Commit message format, and scans for secret leaks.
3. **Push & Pull Request**: Pushes branch to GitHub and opens a Pull Request if not already created.
4. **Copilot Review Solicitation (Dual-Trigger)**:
   - Formally requests review from GitHub Copilot via GraphQL API (`requestReviews(botIds: ["BOT_kgDOCnlnWA"])`).
   - Dispatches a prompt via PR comment: `@copilot review`.
5. **CI Monitoring**: Monitors all 3 GitHub Actions jobs (`Quality`, `Security`, `PAM Integration Docker`).
6. **Active Copilot Analysis Wait**: Waits for Copilot to publish its complete code review (typically 30 seconds to 6 minutes).
7. **Strict Review Gate**: If Copilot emits review comments or requests changes, the PR is **NOT** merged. The script returns an error with targeted file and line details, allowing the AI to apply fixes and re-submit.
8. **Auto-Merge**: Once CI is 100% green and zero unresolved review comments remain, the PR is squash-merged into `main` (`gh pr merge --squash --delete-branch`), and local `main` is synchronized.
