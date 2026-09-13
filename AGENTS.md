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

## Mandatory Reference Documents
Before any implementation, the agent MUST read:
1. `AI/ARCHITECTURE.md` — Master architecture, threat model, invariants, latency budget
2. `AI/DECISIONS.md` — ADR register (anti-hallucination)
3. `AI/MOCK_STRATEGY.md` — Hardware-free simulation strategy
4. `AI/VERIFICATION_MATRIX.md` — Component acceptance criteria matrix
5. `Docs/COMMIT_CONVENTION.md` — Conventional Commits 1.0.0 specification

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
- **PAM Module**: NEVER start an asynchronous runtime (Tokio). Blocking `std::os::unix::net::UnixStream` only, strict 200–250ms deadline.
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
│   └── admin-cli/          # non-biometric diagnostic CLI
├── models/                 # manifest.toml + SHA-256 checksums
├── tests/                  # invariants, integration, fixtures
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
- **Autonomous Loop to Merge (MANDATORY)**:
  1. **Phase 0 — Topic Branch**: `git checkout -b <type>/<name>`. Pre-commit hook prevents direct commits to `main`.
  2. **Phases 1 to 4 — TDD Cycle**: Architect → Tester → Auditor → Developer.
  3. **Documentation & Walkthrough**: Update technical docs in `Docs/` and author sequential walkthrough in `AI/walkthroughs/NN_<name>.md`.
  4. **Validation & Autonomous Push**: Run `./save.sh --auto-merge` (or `scripts/pr_loop.sh`).
  5. **Copilot Review Loop**: Autonomous loop requests review, polls CI and Copilot, surfaces review comments, applies corrections, and pushes updates.
  6. **Auto-Merge**: When all 3 CI checks are green and zero unresolved review comments remain, the PR is automatically squash-merged into `main` and local `main` is updated.
  7. **Zero Human Friction**: The AI agent completes the cycle autonomously through merge.

---

## Rust Coding Conventions
- `#![forbid(unsafe_code)]` in business crates (`protocol`, `policy`, `vision`).
- `unsafe` isolated, documented with safety invariants, and confined to adapter crates (`pam`, `camera-v4l`).
- `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test` required before any commit.
- Use `./save.sh` for quality-checked commits.
- Use `./run_tests.sh` for Dockerized PAM integration tests.

---

## Compilation Error Handling
If the user provides raw `cargo check` or compiler output, analyze silently and output the corrected code directly without verbose commentary.
