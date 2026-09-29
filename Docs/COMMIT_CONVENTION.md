# Conventional Commits Specification — soos Project

This document outlines the strict commit message standard for the **soos** repository.
All contributors, CI workflows, and AI assistants MUST strictly adhere to this specification. Compliance is automatically enforced by git hooks (`.githooks/commit-msg`).

---

## 1. Commit Message Structure

Every commit message must follow the [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/) format:

```text
<type>(<scope>): <subject>

[optional body]

[optional footer(s)]
```

### Core Rules
1. **Header format**: `<type>(<scope>): <subject>` or `<type>: <subject>` (scope is highly recommended).
2. **Subject line length**: Maximum **72 characters** (hard ceiling of 80 enforced by hook).
3. **Subject line case**: Starts with a lowercase letter (unless referencing a proper noun, e.g. `PAM` or `Linux`).
4. **Imperative mood**: Use imperative verbs ("add", "fix", "update", "refactor"), not past tense ("added", "fixed") or present continuous ("adding", "fixing").
5. **No trailing punctuation**: Do NOT end the subject line with a period (`.`).
6. **Separating blank line**: If a body or footer is provided, there MUST be an empty line between the subject and the body.
7. **Language**: Commit messages MUST ALWAYS be written in **English**.

---

## 2. Commit Types

| Type | Description | Example |
|---|---|---|
| `feat` | A new user- or system-facing feature | `feat(protocol): add validation for deadline timestamp` |
| `fix` | A bug fix or security remediation | `fix(pam): ensure PAM_IGNORE is returned on socket failure` |
| `docs` | Documentation-only changes | `docs(readme): update system architecture overview` |
| `style` | Formatting, whitespace, or style changes (zero logic change) | `style(daemon): format import blocks according to rustfmt` |
| `refactor` | Code refactoring without bug fixes or new features | `refactor(codec): simplify buffer length validation logic` |
| `perf` | Code changes that improve performance / latency | `perf(camera): optimize mmap buffer rotation loop` |
| `test` | Adding missing tests or correcting existing tests | `test(invariants): add assertion for zeroize on credentials` |
| `build` | Build system changes, Cargo workspace dependencies | `build(deps): update zeroize dependency to 1.9.0` |
| `ci` | CI workflows, automated scripts, GitHub Actions | `ci(github): add matrix check for Ubuntu 24.04 runner` |
| `chore` | Routine repository maintenance, tooling, configs | `chore(hooks): install conventional commit git hook` |
| `revert` | Reverts a previous commit | `revert: revert feat(policy): experimental rate-limiter` |

---

## 3. Canonical Scopes

Scopes define the subsystem or component touched by the change:

- `pam`: PAM module (`crates/pam`, `pam_soos.so`)
- `daemon`: Background privileged daemon (`crates/daemon`, `soos-daemon`)
- `protocol`: IPC protocol schemas, codec, and serialization (`crates/protocol`)
- `policy`: Authorization, rate-limiting, and verdict logic (`crates/policy`)
- `vision`: Preprocessing, face alignment, and crop pipelines (`crates/vision`)
- `camera`: V4L2 driver and camera capture (`crates/camera-v4l`)
- `biometrics`: Biometric store, template encryption (`crates/biometric-store`)
- `infra`: Scripts, Dockerfiles, dev containers, and local tooling
- `ci`: GitHub Actions workflows and CI configurations
- `security`: Security policies, threat model guardrails, and invariant rules
- `deps`: Dependency updates or `deny.toml` rules
- `docs`: Technical documentation (`Docs/`, `AI/`)

---

## 4. Breaking Changes

Breaking API or protocol changes are indicated by an exclamation mark `!` before the colon, or by a `BREAKING CHANGE:` footer:

```text
feat(protocol)!: transition payload encoding from postcard to bincode

BREAKING CHANGE: The IPC serialization format has changed from Postcard to Bincode.
Previous PAM modules will receive a protocol version mismatch error.
```

---

## 5. Body and Footer Guidelines

### Body
- Use the body to explain the **why** and **what**, rather than the *how* (the code diff explains the *how*).
- Wrap lines at roughly 72 characters.
- Bullet points can be used to list non-trivial side effects or secondary details.

### Footers
- References to issues or pull requests:
  - `Fixes #42`
  - `Closes #15`
  - `Refs #108`
- Co-authors or reviewers:
  - `Co-authored-by: Name <email>`

---

## 6. Good vs. Bad Examples

### Good Commit Messages
```text
feat(protocol): implement bounded request validation

Ensure request size does not exceed 4096 bytes before attempting deserialization.
This prevents memory exhaustion from untrusted IPC inputs.
```

```text
fix(pam): fallback gracefully to PAM_IGNORE on daemon disconnect

When the Unix domain socket disconnects unexpectedly during authentication,
catch the error and immediately return PAM_IGNORE to preserve password fallback.
```

```text
docs(ipc): document peer credentials verification flow
```

### Bad Commit Messages
- `fix bug` *(missing type, scope, context)*
- `FEAT: ADD NEW CAMERA CODE.` *(uppercase type, uppercase subject, ends with period)*
- `wip` *(vague, no conventional prefix)*
- `updated documentation and fixed some bugs` *(missing type, non-imperative, no scope)*
- `refactor(protocol): Refactored the thing and made it work better because I changed some stuff in the files and it was necessary.` *(exceeds character limit, conversational, non-imperative)*

---

## 7. Automated Enforcement

The commit convention is validated at multiple stages:
1. **Pre-commit**: Local hook `.githooks/commit-msg` validates every commit made with `git commit`.
2. **Quality Script (`save.sh`)**: Automatically generates conventional commit headers in English based on changed files when no custom message is provided.
3. **CI Pipeline**: The `PR Title` workflow (`.github/workflows/pr-title.yml`) runs `.githooks/commit-msg` against the pull request title (maximum 72 characters, since GitHub appends ` (#NNN)`), which becomes the squash-merge commit subject on `main`; a non-compliant title fails CI.
