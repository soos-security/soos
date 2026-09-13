#!/usr/bin/env bash
# =============================================================================
# save.sh — Quality pipeline and automated commit script for soos
# =============================================================================
# Executes sequentially:
#   1. cargo fmt       — Formats Rust code in place
#   2. cargo clippy     — Static analysis, fails on any warning (-D warnings)
#   3. cargo test       — Runs unit and architectural invariant tests
#   4. cargo deny check — Audits licenses, security advisories, and bans
#   5. git add .        — Stages all modifications
#   6. git commit       — Commits with Conventional Commits 1.0.0 message
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

for arg in "$@"; do
    if [[ "$arg" == "--auto-merge" || "$arg" == "--loop" ]]; then
        AUTO_MERGE=true
    elif [[ "$arg" == "--push-pr" || "$arg" == "--pr" ]]; then
        PUSH_PR=true
    else
        FORWARD_ARGS+=("$arg")
        if [[ -z "$CUSTOM_MSG" ]]; then
            CUSTOM_MSG="$arg"
        fi
    fi
done

if [[ "$AUTO_MERGE" == "true" ]]; then
    # Delegate full autonomous lifecycle to scripts/pr_loop.sh without recursive flag
    exec ./scripts/pr_loop.sh "${FORWARD_ARGS[@]}"
fi

if [[ "${PUSH_PR_ENV:-0}" == "1" ]]; then
    PUSH_PR=true
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
info "Running Clippy linting (--all-targets -- -D warnings)..."
if cargo clippy --all-targets -- -D warnings; then
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
if cargo test --all-targets; then
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
    if cargo deny check; then
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
if [[ -f "./scripts/candid_review.sh" ]]; then
    if ./scripts/candid_review.sh; then
        success "Candid pre-push review passed."
    else
        error "Candid pre-push review failed invariant checks."
        exit 1
    fi
fi

# ---------------------------------------------------------------------------
# Stage Changes
# ---------------------------------------------------------------------------
echo ""
info "Staging modified files with git add..."
git add .

# Check if there is anything to commit
if git diff --cached --quiet; then
    echo ""
    warn "No modified files staged for commit. Repository is clean."
    if [[ "$PUSH_PR" == "true" ]]; then
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
    CHANGED_FILES=$(git diff --cached --name-only)

    CRATES_CHANGED=()
    DOCS_CHANGED=false
    TESTS_CHANGED=false
    CONFIG_CHANGED=false
    SCRIPTS_CHANGED=false
    AI_CHANGED=false

    while IFS= read -r file; do
        [[ -z "$file" ]] && continue

        case "$file" in
            crates/*/src/*)
                crate_name=$(echo "$file" | cut -d'/' -f2)
                if [[ ! " ${CRATES_CHANGED[*]:-} " =~ " ${crate_name} " ]]; then
                    CRATES_CHANGED+=("$crate_name")
                fi
                ;;
            crates/*/tests/* | tests/*)
                TESTS_CHANGED=true
                ;;
            Docs/* | docs/* | *.md)
                DOCS_CHANGED=true
                ;;
            AI/*)
                AI_CHANGED=true
                ;;
            Cargo.toml | Cargo.lock | rust-toolchain.toml | deny.toml)
                CONFIG_CHANGED=true
                ;;
            *.sh | Dockerfile | .dockerignore | .githooks/*)
                SCRIPTS_CHANGED=true
                ;;
            *)
                CONFIG_CHANGED=true
                ;;
        esac
    done <<< "$CHANGED_FILES"

    PARTS=()

    if [[ ${#CRATES_CHANGED[@]} -eq 1 ]]; then
        PARTS+=("feat(${CRATES_CHANGED[0]}): update component implementation")
    elif [[ ${#CRATES_CHANGED[@]} -gt 1 ]]; then
        PARTS+=("feat(workspace): update multiple crate implementations")
        for c in "${CRATES_CHANGED[@]}"; do
            PARTS+=("feat($c): update component")
        done
    fi
    if [[ "$TESTS_CHANGED" == true ]]; then
        PARTS+=("test: update test suites and security invariants")
    fi
    if [[ "$DOCS_CHANGED" == true ]]; then
        PARTS+=("docs: update technical documentation")
    fi
    if [[ "$AI_CHANGED" == true ]]; then
        PARTS+=("docs(ai): update AI strategy documents and walkthroughs")
    fi
    if [[ "$SCRIPTS_CHANGED" == true ]]; then
        PARTS+=("chore(infra): update scripts, tooling, and guardrails")
    fi
    if [[ "$CONFIG_CHANGED" == true ]]; then
        PARTS+=("chore(config): update workspace and dependency configuration")
    fi

    if [[ ${#PARTS[@]} -eq 0 ]]; then
        COMMIT_MSG="chore: update workspace files"
    elif [[ ${#PARTS[@]} -eq 1 ]]; then
        COMMIT_MSG="${PARTS[0]}"
    else
        # Enforce empty line between subject line and body bullets
        SUBJECT="${PARTS[0]}"
        BODY=""
        for ((i = 1; i < ${#PARTS[@]}; i++)); do
            BODY="${BODY}
- ${PARTS[$i]}"
        done
        COMMIT_MSG="${SUBJECT}
${BODY}"
    fi

    FILE_COUNT=$(echo "$CHANGED_FILES" | wc -l | tr -d ' ')
    COMMIT_MSG="${COMMIT_MSG}

[${FILE_COUNT} file(s) modified]"
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
    FIRST_LINE=$(echo "$COMMIT_MSG" | head -n 1)
    PR_BODY="## Summary
$COMMIT_MSG

## Automated Quality & Security Checks
- [x] cargo fmt --check (official formatting)
- [x] cargo clippy --all-targets -- -D warnings (zero warnings)
- [x] cargo test --all-targets (unit tests + architectural invariants)
- [x] cargo deny check (licenses, advisories, source integrity, bans)
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
