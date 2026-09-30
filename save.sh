#!/usr/bin/env bash
# =============================================================================
# save.sh — Quality pipeline and automated commit script for soos
# =============================================================================
# Executes sequentially:
#   1. cargo fmt       — Formats Rust code in place
#   2. cargo clippy     — Static analysis, fails on any warning (-D warnings)
#   3. cargo test       — Runs unit and architectural invariant tests
#   4. cargo deny check — Audits licenses, security advisories, and bans
#   5. candid review    — Layer 1 invariants + fingerprint-bound Layer 2 report
#   6. sync_issue.py    — Offline mapping self-check + backlog report (no writes)
#   7. git add -u       — Stages tracked modifications, plus new files under
#                         crates/ tests/ Docs/ AI/ scripts/ packaging/ only
#                         (other untracked files are listed, never staged)
#   8. git commit       — Commits with Conventional Commits 1.0.0 message
#                         (-m, or a subject inferred from the branch prefix by
#                         scripts/commit_message.sh; no prefix match = refusal)
#   (--push-pr: after the push, committed checkboxes are mirrored to GitHub)
#
# Clippy/test/deny flags are identical to .github/workflows/ci.yml so that a
# green local run predicts a green CI run.
#
# If any step fails, the script halts immediately (set -euo pipefail).
# By default, this script only performs local commits and NEVER pushes.
#
# Flags:
#   --push-pr          — Pushes the branch to GitHub and opens a Pull Request
#   --auto-merge       — Runs the full autonomous PR, review, and merge loop
# =============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# Terminal Colors
# ---------------------------------------------------------------------------
if [[ -t 1 ]]; then
    readonly GREEN='\033[0;32m'
    readonly RED='\033[0;31m'
    readonly YELLOW='\033[1;33m'
    readonly BLUE='\033[0;34m'
    readonly BOLD='\033[1m'
    readonly NC='\033[0m'
else
    readonly GREEN=''
    readonly RED=''
    readonly YELLOW=''
    readonly BLUE=''
    readonly BOLD=''
    readonly NC=''
fi

# ---------------------------------------------------------------------------
# Logging Functions
# ---------------------------------------------------------------------------
info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[FAIL]${NC}  $*" >&2; }
step()    { echo -e "\n${BOLD}── Step $1 ──${NC}"; }

# ---------------------------------------------------------------------------
# Pre-flight Checks
# ---------------------------------------------------------------------------
if [[ ! -f "Cargo.toml" ]]; then
    error "This script must be executed from the root of the soos workspace."
    error "Cargo.toml not found in current working directory: $(pwd)"
    exit 1
fi

if ! git rev-parse --is-inside-work-tree &> /dev/null; then
    error "Current directory is not a Git repository."
    error "Initialize it with: git init"
    exit 1
fi

# Quick help check before branch validation
for arg in "$@"; do
    if [[ "$arg" == "-h" || "$arg" == "--help" ]]; then
        cat << 'EOF'
Usage: ./save.sh [OPTIONS] [COMMIT_MESSAGE]

Quality pipeline and automated commit/release script for soos.

Without a message, the subject is inferred from the branch: the type is the
feat/, fix/, test/ or chore/ prefix, the scope is the single crate touched (if
any), the description is the branch suffix. Any other branch requires -m.
Staging: tracked modifications (git add -u) plus new files under crates/,
tests/, Docs/, AI/, scripts/ and packaging/; git add other new files yourself.

Options:
  --auto-merge, --loop   Run full autonomous PR, review, and merge loop
  --push-pr, --pr        Run quality pipeline, commit, and push PR
  -m, --message <MSG>    Specify Conventional Commit message
  -h, --help             Display this help message and exit

Examples:
  ./save.sh "feat(daemon): add model verification"
  ./save.sh --auto-merge -m "fix(pam): handle timeout gracefully"
EOF
        exit 0
    fi
done

# Prevent direct commits on main branch
CURRENT_BRANCH=$(git symbolic-ref --short HEAD 2>/dev/null || echo "detached")
if [[ "$CURRENT_BRANCH" == "main" && "${ALLOW_MAIN_COMMIT:-0}" != "1" ]]; then
    error "Direct commits to the 'main' branch are strictly prohibited!"
    error "Create a dedicated topic branch before committing changes:"
    error "  git checkout -b feat/<name>   # for new features"
    error "  git checkout -b fix/<name>    # for bug fixes"
    error "  git checkout -b chore/<name>  # for tooling, CI, or docs"
    exit 1
fi

# Ensure .gitignore exists
if [[ ! -f ".gitignore" ]]; then
    warn "No .gitignore detected. Creating minimal .gitignore..."
    cat > .gitignore << 'GITIGNORE'
# Rust build artifacts
/target/

# IDE / Editors
.vscode/
.idea/
*.swp
*.swo
*~

# OS
.DS_Store
Thumbs.db

# Debug files
*.pdb
GITIGNORE
    success ".gitignore created."
fi

# Ensure git hooks path is configured
if [[ -d ".githooks" ]]; then
    git config core.hooksPath .githooks 2>/dev/null || true
fi

# ---------------------------------------------------------------------------
# Argument Parsing (--push-pr, --auto-merge, custom commit message)
# ---------------------------------------------------------------------------
PUSH_PR=false
AUTO_MERGE=false
CUSTOM_MSG=""
FORWARD_ARGS=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --auto-merge|--loop)
            AUTO_MERGE=true
            shift
            ;;
        --push-pr|--pr)
            PUSH_PR=true
            shift
            ;;
        -h|--help)
            cat << 'EOF'
Usage: ./save.sh [OPTIONS] [COMMIT_MESSAGE]

Quality pipeline and automated commit/release script for soos.

Without a message, the subject is inferred from the branch: the type is the
feat/, fix/, test/ or chore/ prefix, the scope is the single crate touched (if
any), the description is the branch suffix. Any other branch requires -m.
Staging: tracked modifications (git add -u) plus new files under crates/,
tests/, Docs/, AI/, scripts/ and packaging/; git add other new files yourself.

Options:
  --auto-merge, --loop   Run full autonomous PR, review, and merge loop
  --push-pr, --pr        Run quality pipeline, commit, and push PR
  -m, --message <MSG>    Specify Conventional Commit message
  -h, --help             Display this help message and exit

Examples:
  ./save.sh "feat(daemon): add model verification"
  ./save.sh --auto-merge -m "fix(pam): handle timeout gracefully"
EOF
            exit 0
            ;;
        -m|--message)
            shift
            if [[ $# -gt 0 ]]; then
                CUSTOM_MSG="$1"
                FORWARD_ARGS+=("$1")
                shift
            fi
            ;;
        *)
            FORWARD_ARGS+=("$1")
            if [[ -z "$CUSTOM_MSG" ]]; then
                CUSTOM_MSG="$1"
            fi
            shift
            ;;
    esac
done


if [[ "$AUTO_MERGE" == "true" ]]; then
    # Delegate full autonomous lifecycle to scripts/pr_loop.sh without recursive flag
    exec ./scripts/pr_loop.sh "${FORWARD_ARGS[@]}"
fi

if [[ "${PUSH_PR_ENV:-0}" == "1" ]]; then
    PUSH_PR=true
fi

# Without -m, refuse early (before the long pipeline) when no subject can be
# inferred from the branch name (GitHub #245).
# shellcheck source=scripts/commit_message.sh
source ./scripts/commit_message.sh
if [[ -z "$CUSTOM_MSG" ]] && ! soos_infer_commit_subject "$CURRENT_BRANCH" < /dev/null > /dev/null; then
    error "No commit message given and none can be inferred from branch '$CURRENT_BRANCH'."
    exit 1
fi

# ---------------------------------------------------------------------------
# Step 1: Code Formatting
# ---------------------------------------------------------------------------
step "1/5: cargo fmt"
info "Formatting Rust code with rustfmt..."
if cargo fmt; then
    success "Code formatted successfully."
else
    error "cargo fmt encountered an error."
    exit 1
fi

# ---------------------------------------------------------------------------
# Step 2: Static Analysis (Clippy)
# ---------------------------------------------------------------------------
step "2/5: cargo clippy"
info "Running Clippy linting (--locked --workspace --all-targets --all-features -- -D warnings)..."
if cargo clippy --locked --workspace --all-targets --all-features -- -D warnings; then
    success "Zero Clippy warnings detected."
else
    error "Clippy detected warnings or lint errors."
    error "Resolve all lint issues above before committing."
    exit 1
fi

# ---------------------------------------------------------------------------
# Step 3: Automated Tests (Unit & Invariant Suites)
# ---------------------------------------------------------------------------
step "3/5: cargo test"
info "Running test suite (unit tests and security invariants)..."
if cargo test --locked --workspace --all-targets --all-features; then
    success "All automated tests passed successfully."
else
    error "Test failures detected."
    error "Resolve failing tests before committing."
    exit 1
fi

# ---------------------------------------------------------------------------
# Step 4: Dependency & Security Auditing (cargo-deny)
# ---------------------------------------------------------------------------
step "4/5: cargo deny check"
if command -v cargo-deny &> /dev/null; then
    info "Auditing third-party supply chain (licenses, advisories, sources, bans)..."
    if cargo deny --locked check; then
        success "cargo-deny audit passed."
    else
        error "cargo-deny detected security advisory, license, or ban violations."
        error "Review deny.toml and workspace dependencies before proceeding."
        exit 1
    fi
else
    warn "cargo-deny is not installed locally — skipping check."
    warn "Install with: cargo install cargo-deny"
fi

# ---------------------------------------------------------------------------
# Step 5: Independent Candid Pre-Push Code Review
# ---------------------------------------------------------------------------
step "5/5: candid review"
if [[ -f "./scripts/candid_subagent.sh" ]]; then
    if ./scripts/candid_subagent.sh; then
        success "Candid pre-push review passed."
    else
        error "Candid pre-push review failed invariant checks or AI sub-agent review."
        exit 1
    fi
elif [[ -f "./scripts/candid_review.sh" ]]; then
    if ./scripts/candid_review.sh; then
        success "Candid pre-push review passed."
    else
        error "Candid pre-push review failed invariant checks."
        exit 1
    fi
fi

# ---------------------------------------------------------------------------
# Backlog / Issue Traceability (before staging, GitHub #188)
# ---------------------------------------------------------------------------
# The offline self-check rejects duplicate GitHub targets and unknown backlog ids. The
# local-only report never writes AI/BACKLOG.md: sub-issues are ticked explicitly with
# `sync_issue.py --subissue` beforehand, so the backlog edit is staged with this commit.
if [[ -f "./scripts/sync_issue.py" ]]; then
    if ! python3 ./scripts/sync_issue.py --check; then
        error "scripts/sync_issue.py --check failed: fix BACKLOG_TO_GITHUB / BRANCH_TO_ISSUE or AI/BACKLOG.md."
        exit 1
    fi
    if ! python3 ./scripts/sync_issue.py --auto --local-only --branch "$CURRENT_BRANCH"; then
        error "scripts/sync_issue.py --auto --local-only failed."
        exit 1
    fi
fi

# ---------------------------------------------------------------------------
# Stage Changes
# ---------------------------------------------------------------------------
echo ""
# Never `git add .`: it staged every untracked scratch file (GitHub #245). Stage
# tracked modifications, then new files under the source directories only.
info "Staging tracked modifications and new files under the source directories..."
git add -u
git add ./crates ./tests ./Docs ./AI ./scripts ./packaging
NOT_STAGED=$(git ls-files --others --exclude-standard)
if [[ -n "$NOT_STAGED" ]]; then
    warn "Untracked files outside the source directories were NOT staged:"
    while IFS= read -r untracked; do
        warn "  ${untracked}"
    done <<< "$NOT_STAGED"
    warn "Stage them explicitly with 'git add <path>' before ./save.sh if they belong in the commit."
fi

# Check if there is anything to commit
if git diff --cached --quiet; then
    echo ""
    warn "No modified files staged for commit. Repository is clean."
    if [[ "$PUSH_PR" == "true" ]]; then
        HEAD_SUBJECT=$(git log -1 --pretty=%s)
        if [[ ${#HEAD_SUBJECT} -gt 72 ]]; then
            error "HEAD subject is ${#HEAD_SUBJECT} characters; subjects pushed as PRs are limited to 72."
            exit 1
        fi
        info "Push requested for current branch..."
        git push -u origin "$CURRENT_BRANCH"
    fi
    exit 0
fi

# ---------------------------------------------------------------------------
# Conventional Commit Message Generation
# ---------------------------------------------------------------------------
if [[ -n "$CUSTOM_MSG" ]]; then
    COMMIT_MSG="$CUSTOM_MSG"
else
    # One subject line from the branch prefix and the staged crate (GitHub #245):
    # no generic feat(...) type, no pseudo-subject body bullets, no file counter.
    if ! COMMIT_MSG=$(git diff --cached --name-only | soos_infer_commit_subject "$CURRENT_BRANCH"); then
        error "Could not infer a commit subject; re-run with -m \"<type>(<scope>): <description>\"."
        exit 1
    fi
    warn "Inferred subject from branch '$CURRENT_BRANCH': $COMMIT_MSG"
    warn "Pass -m for a precise description (it becomes the squash-merge subject)."
fi

# The subject becomes the PR title / squash subject when pushed: GitHub appends
# " (#NNN)", so CI (pr-title.yml) caps it at 72 characters. Fail before committing.
SUBJECT_LINE=$(echo "$COMMIT_MSG" | head -n 1)
if [[ "$PUSH_PR" == "true" && ${#SUBJECT_LINE} -gt 72 ]]; then
    error "Commit subject is ${#SUBJECT_LINE} characters; subjects pushed as PRs are limited to 72."
    exit 1
fi

# ---------------------------------------------------------------------------
# Commit Execution
# ---------------------------------------------------------------------------
info "Committing changes..."
echo -e "${BLUE}  Subject:${NC} $(echo "$COMMIT_MSG" | head -n 1)"

git commit -m "$COMMIT_MSG"

# ---------------------------------------------------------------------------
# Summary and Push / PR
# ---------------------------------------------------------------------------
echo ""
success "═══════════════════════════════════════════════"
success "  Local save and commit successful!"
success "═══════════════════════════════════════════════"
echo ""
info "Latest commit:"
git log --oneline -1

if [[ "$PUSH_PR" == "true" ]]; then
    echo ""
    info "Option --push-pr detected. Pushing branch to GitHub..."
    git push -u origin "$CURRENT_BRANCH"
    success "Branch '$CURRENT_BRANCH' successfully pushed to origin."

    echo ""
    info "Preparing Pull Request..."

    # Mirror the committed AI/BACKLOG.md checkboxes to the GitHub issue (never writes the
    # backlog, so the tree stays clean after the push). A gh/network failure is reported and
    # does not abort: the branch is already pushed and the sync can be re-run by hand.
    CLOSES_KEYWORD=""
    if [[ -f "./scripts/sync_issue.py" ]]; then
        info "Mirroring committed AI/BACKLOG.md checkboxes to the GitHub issue..."
        if ! python3 ./scripts/sync_issue.py --auto --branch "$CURRENT_BRANCH"; then
            warn "GitHub issue sync FAILED (see above); nothing local was changed."
            warn "Re-run: python3 scripts/sync_issue.py --auto --branch $CURRENT_BRANCH"
        fi
        GITHUB_ISSUE_ID=$(python3 -c "
import os
from scripts.sync_issue import BRANCH_TO_ISSUE, BACKLOG_TO_GITHUB, get_current_branch
b = os.environ.get('CURRENT_BRANCH') or get_current_branch() or ''
bi = BRANCH_TO_ISSUE.get(b)
if bi and bi in BACKLOG_TO_GITHUB:
    print(BACKLOG_TO_GITHUB[bi])
" 2>/dev/null || true)
        if [[ -n "$GITHUB_ISSUE_ID" ]]; then
            CLOSES_KEYWORD="

Closes #$GITHUB_ISSUE_ID"
            info "Associated with GitHub Issue #$GITHUB_ISSUE_ID (will auto-close on merge)."
        fi
    fi

    FIRST_LINE=$(echo "$COMMIT_MSG" | head -n 1)
    PR_BODY="## Summary
$COMMIT_MSG
$CLOSES_KEYWORD

## Automated Quality & Security Checks
- [x] cargo fmt (official formatting)
- [x] cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
- [x] cargo test --locked --workspace --all-targets --all-features (unit tests + architectural invariants)
- [x] cargo deny --locked check (licenses, advisories, source integrity, bans)
- [x] Dual-layer candid review (fingerprint-bound AI/candid_review_report.md)
- [x] Pre-commit hook validation (anti-commit main + secret scanner)"

    PR_CREATED=false
    if command -v gh &> /dev/null; then
        if gh auth status &> /dev/null; then
            if gh pr create --title "$FIRST_LINE" --body "$PR_BODY" --base main --head "$CURRENT_BRANCH"; then
                success "Pull Request created successfully via GitHub CLI!"
                PR_CREATED=true
            fi
        fi
    fi

    if [[ "$PR_CREATED" != "true" ]]; then
        REPO_URL="https://github.com/Mysticaly622/soos"
        PR_URL="${REPO_URL}/pull/new/${CURRENT_BRANCH}"
        echo ""
        info "Direct URL to open your Pull Request in 1 click:"
        echo -e "${BOLD}${BLUE}  👉 ${PR_URL}${NC}"
        echo ""
        warn "Tip: To allow gh to create PRs automatically without browser interaction,"
        warn "run 'gh auth login' once in your terminal."
    fi
else
    echo ""
    warn "Notice: Branch has NOT been pushed to GitHub."
    info "To push and automatically open a Pull Request, use:"
    echo -e "  ./save.sh --push-pr"
fi
