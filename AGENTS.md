# Project Rules — soos (Linux Biometric PAM)

## Project Identity
- **Name**: soos (formerly "Zero-Trust Linux Hello" / "ZTLH")
- **Mission**: Local facial verification PAM module for Linux
- **Language**: Rust, Cargo workspace monorepo

---

## Strict Project-Wide Language Policy: English Only
- **All Deliverables in English**:
  - Source code, function names, docstrings, and inline comments MUST be in English.
  - All documentation in `Docs/` and `AI/` MUST be written in English.
  - All walkthroughs in `AI/walkthroughs/` MUST be numbered and written in English.
  - All Git commit messages, branch names, and Pull Request titles/descriptions MUST be in English.
  - All script outputs, logs, diagnostics, and errors MUST be in English.
  - All directory and file names MUST be in English (e.g. no French words like `ET`, `CYCLE`, etc.).
- **User Language Decoupling**:
  - Even if the user submits tasks, questions, or prompts in French (or any other language), the AI MUST generate ALL project code, documentation, commits, PRs, and walkthroughs exclusively in professional English. (Conversational replies in chat may respond in the user's language, but zero non-English content may enter git).

---

## Mandatory Reference Documents (Full Context Mandate)
Before any implementation, the agent MUST read and ingest the full context from `AI/` and `Docs/`:
1. `AI/ARCHITECTURE.md` — Master architecture, threat model, invariants, latency budget
2. `AI/DECISIONS.md` — ADR register (anti-hallucination)
3. `AI/BACKLOG.md` — Complete development backlog and issue specifications
4. `AI/VERIFICATION_MATRIX.md` — Component acceptance criteria matrix
5. `AI/MOCK_STRATEGY.md` — Hardware-free simulation strategy
6. `AI/ROLES_AND_WORKFLOW.md` — Multi-agent roles and quality guidelines
7. `Docs/SECURITY_AND_QUALITY_GUIDELINES.md` — Security guidelines, compiler profiles, workspace lints
8. `Docs/DEVELOPMENT_WORKFLOW.md` — Complete TDD lifecycle and testing guidelines
9. `Docs/COMMIT_CONVENTION.md` — Conventional Commits 1.0.0 specification
10. `Docs/IPC_PROTOCOL.md` — IPC framing and codec constraints

---

## Strict Test Integrity Invariant (Zero Test Weakening)
- **Tests Are Immutable Contracts**: Tests authored during Phase 2 (Tester Agent) represent the contractual acceptance criteria derived from `AI/BACKLOG.md` and `AI/VERIFICATION_MATRIX.md`.
- **Zero Weakening / Bypassing**: Under **NO circumstances** is an AI agent permitted to modify, weaken, delete, or bypass an existing test to make it pass with broken or incomplete production code.
- **Perseverance Required**: If a test fails, the AI MUST persevere, analyze the root cause, and fix the production implementation until all tests pass cleanly. Artificially weakening tests is considered an architectural invariant violation.

---

## Multi-Agent TDD Workflow (Strict 4-Phase Cycle)
For every functional feature or modification:

### Phase 1 — Architect Agent
- Specify structs, enums, traits, and error types.
- Check alignment with `AI/ARCHITECTURE.md`.
- Identify affected security invariants and bounds.

### Phase 2 — Tester Agent
- Write unit tests BEFORE production code (TDD Red Phase).
- Tests MUST fail initially.
- Every PAM pathway must include an automated test asserting `PAM_IGNORE` fallback.

### Phase 3 — Auditor Agent
- Hunt down panics (`unwrap`, `expect`) in PAM and FFI pathways.
- Ensure bounded memory allocations and zero leaks.
- Validate `#![forbid(unsafe_code)]` in business crates.
- Ensure zero sensitive data (passwords, embeddings, raw frames) is logged or exposed.

### Phase 4 — Developer Agent
- Implement minimal code to turn all tests green.
- Comply with all constraints defined in preceding phases.

---

## Immutable Architectural Decisions
- **IPC**: Unix Domain Socket only (`SOCK_SEQPACKET` / framed `SOCK_STREAM`).
- **PAM Module**: NEVER start an asynchronous runtime (Tokio). Blocking `std::os::unix::net::UnixStream` only. Every blocking PAM operation has an explicit deadline derived from the clamped `timeout_ms` (default 1000 ms, range 10–5000 ms, `crates/pam/src/config.rs`); nothing is ever unbounded (ADR 2026-09-30 "PAM Deadline Derived From Clamped `timeout_ms`").
- **Panic Safety**: Mandatory `catch_unwind` wrapping all FFI boundaries, systematically returning `PAM_IGNORE`.
- **Camera**: Root daemon is the exclusive owner of `/dev/video*`. `v4l` crate in production, not `nokhwa`.
- **AI**: `ort` (ONNX Runtime) CPU only. Absolute prohibition against OpenCV.
- **Naming**: Shared object is `pam_soos.so`, background daemon is `soos-daemon`, project is `soos`.
- **Commits**: Strict adherence to Conventional Commits 1.0.0 (enforced via `.githooks/commit-msg`).

---

## Strict Prohibitions
- ❌ NEVER use OpenCV
- ❌ NEVER start Tokio inside the PAM `.so`
- ❌ NEVER use `unwrap()` or `expect()` in PAM production code
- ❌ NEVER accept, store, or transmit passwords over the IPC socket
- ❌ NEVER make the IPC socket world-writable (`0666`)
- ❌ NEVER log frames, biometric embeddings, or credentials
- ❌ NEVER convert an error into `PAM_SUCCESS`

---

## Workspace Monorepo Structure
```
soos/
├── Cargo.toml              # workspace resolver="2"
├── crates/
│   ├── protocol/           # bounded schemas, codec v1, fuzz
│   ├── policy/             # authorization logic, rate limit, zero I/O
│   ├── pam/                # cdylib pam_soos.so
│   ├── daemon/             # root binary, Tokio
│   ├── camera-v4l/         # V4L2, mock-camera feature
│   ├── vision/             # preprocessing, alignment
│   ├── inference-ort/      # isolated ONNX Runtime CPU
│   ├── biometric-store/    # encrypted embeddings at rest
│   ├── evidence-store/     # opt-in intrusion snapshots
│   ├── enrollment-cli/     # root enrollment CLI
│   ├── admin-cli/          # non-biometric diagnostic CLI
│   └── gui/                # soos-gui diagnostic and enrollment GUI
├── models/                 # manifest.toml + SHA-256 checksums
├── tests/
│   ├── invariants/         # soos-invariants static repository checks
│   ├── fixtures/           # soos-test-fixtures: synthetic frames, PAD presentations, ONNX graphs
│   ├── docker/             # Dockerized PAM matrix (pam_test_runner)
│   ├── distro/             # per-distribution package validation
│   └── physical/           # real-hardware validation suite
└── AI/                     # AI documentation and walkthroughs
```

---

## Branching & Pull Request Policy (MANDATORY)
- **Never commit directly to `main`**: All work must be conducted on a dedicated topic branch.
- **Branch Naming**:
  - `feat/<name>`: New feature (e.g. `feat/ipc-client`)
  - `fix/<name>`: Bug fix or security patch (e.g. `fix/pam-timeout`)
  - `test/<name>`: Tests, benchmarks, or fixtures (e.g. `test/fuzz-codec`)
  - `chore/<name>`: Tooling, CI, dependencies, docs (e.g. `chore/commit-convention`)
  - No other prefix is allowed (commit types such as `refactor` or `docs` are not branch prefixes). The merged historical branches `refactor/remove-ort-landmark-detector`, `refactor/vision-pipeline-3-model`, `refactor/model-ids-nextgen`, `refactor/mock-backends-nextgen` and `docs/nextgen-model-documentation` stay registered in `scripts/sync_issue.py` as frozen `LEGACY_BRANCHES`; `--check` rejects any new one.
- **Autonomous Loop to Merge (MANDATORY)**:
  1. **Phase 0 — Topic Branch**: `git checkout -b <type>/<name>`. Pre-commit hook prevents direct commits to `main`.
  2. **Phases 1 to 4 — TDD Cycle**: Architect → Tester → Auditor → Developer.
  3. **Documentation & Walkthrough**: Update technical docs in `Docs/` and author sequential walkthrough in `AI/walkthroughs/NN_<name>.md`.
  4. **Validation & Candid Pre-Push Review**: Execute local quality pipeline and `./scripts/candid_review.sh` (context-free, impartial audit of the raw diff for logic, security invariants, panic safety, and English policy).
  5. **Autonomous Push & PR**: `./save.sh --auto-merge` (or `scripts/pr_loop.sh`) opens PR and monitors CI checks (Quality, Security, PAM Docker).
  6. **Streamlined Auto-Merge**: When all GitHub Actions CI checks are green, the PR is automatically squash-merged into `main` without requiring external review, and local `main` is updated.
  7. **Zero Human Friction**: The AI agent completes the cycle autonomously through merge.

---

## Rust Coding Conventions
- `#![forbid(unsafe_code)]` in every business crate (at least `protocol`, `policy`, `vision`; the authoritative list is `test_business_crates_forbid_unsafe_code` in `tests/invariants/src/lib.rs`).
- `unsafe` isolated, documented with safety invariants, and confined to adapter crates (`pam`, `camera-v4l`).
- `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` required before any commit.
- Use `./save.sh` for quality-checked commits.
- Use `./run_tests.sh` for Dockerized PAM integration tests.

---

## Compilation Error Handling
If the user provides raw `cargo check` or compiler output, analyze silently and output the corrected code directly without verbose commentary.
