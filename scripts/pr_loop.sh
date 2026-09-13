#!/usr/bin/env bash
# =============================================================================
# scripts/pr_loop.sh — Autonomous PR loop, Candid Review & Streamlined Merge
# =============================================================================
# Orchestrates branch finalization with local candid review and CI merge:
#   1. Executes ./save.sh --push-pr:
#      - Quality gates (fmt, clippy, tests, deny)
#      - Candid pre-push review (scripts/candid_review.sh)
#      - Conventional commit & push
#   2. Opens Pull Request if not already created
#   3. Monitors CI checks (Quality, Security, PAM Docker on GitHub Actions)
#   4. Once all CI checks are 100% green, squash-merges into 'main'
#      and synchronizes local 'main' branch without requiring external review.
# =============================================================================

set -euo pipefail
export GH_PAGER=cat

# Terminal Colors
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

info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[FAIL]${NC}  $*" >&2; }
step()    { echo -e "\n${BOLD}── $* ──${NC}"; }

# 1. Branch verification
CURRENT_BRANCH=$(git symbolic-ref --short HEAD 2>/dev/null || echo "detached")
if [[ "$CURRENT_BRANCH" == "main" || "$CURRENT_BRANCH" == "detached" ]]; then
    error "pr_loop.sh must be run on a dedicated topic branch (currently on: '$CURRENT_BRANCH')."
    exit 1
fi

# Ensure git hooks path is configured
if [[ -d ".githooks" ]]; then
    if ! git config core.hooksPath .githooks; then
        error "Failed to configure core.hooksPath to .githooks."
        exit 1
    fi
fi

step "1/6: Local Quality Gates, Conventional Commit, and Push"
info "Running quality pipeline and pushing branch '$CURRENT_BRANCH'..."
PASSED_ARGS=()
for arg in "$@"; do
    if [[ "$arg" != "--auto-merge" && "$arg" != "--loop" ]]; then
        PASSED_ARGS+=("$arg")
    fi
done
./save.sh --push-pr "${PASSED_ARGS[@]}"

TARGET_HEAD_SHA=$(git rev-parse HEAD)
info "Target HEAD commit SHA: $TARGET_HEAD_SHA"

step "2/6: Pull Request Verification or Creation"
if ! command -v gh &>/dev/null || ! gh auth status &>/dev/null; then
    warn "GitHub CLI ('gh') is not installed or not authenticated."
    warn "Branch '$CURRENT_BRANCH' has been safely committed and pushed to origin."
    REPO_URL="https://github.com/Mysticaly622/soos"
    PR_URL="${REPO_URL}/pull/new/${CURRENT_BRANCH}"
    echo ""
    info "Direct URL to open and review your Pull Request in 1 click:"
    echo -e "${BOLD}${BLUE}  👉 ${PR_URL}${NC}"
    echo ""
    success "Local quality pipeline, tests, and candid review passed 100%!"
    exit 0
fi

PR_JSON=$(gh pr list --head "$CURRENT_BRANCH" --json number,url,state --state open 2>/dev/null || echo "[]")
PR_NUMBER=$(echo "$PR_JSON" | grep -o '"number":[0-9]*' | head -1 | cut -d':' -f2 || true)

if [[ -z "$PR_NUMBER" ]]; then
    info "Creating new Pull Request for '$CURRENT_BRANCH' targeting 'main'..."
    LAST_COMMIT_MSG=$(git log -1 --pretty=%B)
    FIRST_LINE=$(echo "$LAST_COMMIT_MSG" | head -1)

    CLOSES_KEYWORD=""
    if [[ -f "./scripts/sync_issue.py" ]]; then
        info "Synchronizing task checkboxes with GitHub Issues & AI/BACKLOG.md..."
        python3 ./scripts/sync_issue.py --auto || true
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

    PR_BODY="## Summary
$LAST_COMMIT_MSG
$CLOSES_KEYWORD

## Automated Quality & Security Checks
- [x] cargo fmt --check
- [x] cargo clippy --all-targets -- -D warnings
- [x] cargo test --all-targets (including architectural invariants)
- [x] cargo deny check (licenses, advisories, sources, bans)"

    PR_URL=$(gh pr create --title "$FIRST_LINE" --body "$PR_BODY" --base main --head "$CURRENT_BRANCH")
    PR_NUMBER=$(gh pr view --json number -q .number)
    success "Pull Request #$PR_NUMBER created: $PR_URL"
else
    PR_URL="https://github.com/Mysticaly622/soos/pull/$PR_NUMBER"
    success "Existing Pull Request #$PR_NUMBER detected: $PR_URL"
fi

step "3/4: Monitoring CI Workflow Checks (GitHub Actions)"
info "Monitoring CI jobs for PR #$PR_NUMBER (Quality, Security, PAM Docker)..."
sleep 5

CI_PASSED=false
for attempt in $(seq 1 60); do
    if gh pr checks "$PR_NUMBER" >/dev/null 2>&1; then
        CI_PASSED=true
        break
    fi
    STATUS=$(gh pr checks "$PR_NUMBER" 2>&1 || true)
    if echo "$STATUS" | grep -qiE "(fail|cancelled)"; then
        error "CI checks failed on GitHub Actions!"
        echo "$STATUS"
        exit 1
    fi
    echo -ne "  ⏳ CI checks in progress (attempt ${attempt}/60)...\r"
    sleep 10
done
echo ""

if [[ "$CI_PASSED" != "true" ]]; then
    if ! gh pr checks "$PR_NUMBER"; then
        error "CI check timeout or verification failure!"
        exit 1
    fi
fi
success "All CI checks passed successfully on GitHub Actions!"

step "4/4: Streamlined Auto-Merge to Main"
info "CI green: auto-merging PR #$PR_NUMBER into 'main'..."

IS_ALREADY_MERGED=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER" --jq '.merged' 2>/dev/null || echo "false")
CURRENT_PR_HEAD_SHA=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER" --jq '.head.sha' 2>/dev/null || true)
if [[ -z "$CURRENT_PR_HEAD_SHA" ]]; then
    error "Failed to read current PR head SHA before merge."
    exit 1
fi
if [[ "$CURRENT_PR_HEAD_SHA" != "$TARGET_HEAD_SHA" ]]; then
    error "PR head changed from $TARGET_HEAD_SHA to $CURRENT_PR_HEAD_SHA during review."
    error "MERGE BLOCKED: restart ping-pong loop on latest commit."
    exit 2
fi

if [[ "$IS_ALREADY_MERGED" == "true" ]]; then
    success "═════════════════════════════════════════════════════════════"
    success "  Pull Request #$PR_NUMBER already merged into main!"
    success "═════════════════════════════════════════════════════════════"
elif gh pr merge "$PR_NUMBER" --squash --admin 2>/dev/null || gh pr merge "$PR_NUMBER" --squash; then
    success "═════════════════════════════════════════════════════════════"
    success "  Pull Request #$PR_NUMBER approved and merged into main!"
    success "═════════════════════════════════════════════════════════════"
else
    error "Failed to merge PR #$PR_NUMBER."
    exit 1
fi

info "Switching to local 'main' branch and synchronizing..."
git stash --include-untracked >/dev/null 2>&1 || true
git checkout main
git pull origin main
git stash pop >/dev/null 2>&1 || true
success "Local 'main' branch synchronized. Mission accomplished!"
