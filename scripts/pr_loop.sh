#!/usr/bin/env bash
# =============================================================================
# scripts/pr_loop.sh — Autonomous PR loop, Candid Review & Streamlined Merge
# =============================================================================
# Orchestrates branch finalization with local candid review and CI merge:
#   1. Executes ./save.sh --push-pr:
#      - Quality gates (fmt, clippy, tests, deny)
#      - Dual-layer candid review (scripts/candid_subagent.sh, fingerprint-bound)
#      - Conventional commit & push
#   2. Opens Pull Request if not already created
#   3. Monitors CI checks of the pushed SHA (watch + fail-fast, 45 min ceiling)
#   4. Once all CI checks are 100% green, squash-merges into 'main' with
#      --match-head-commit (atomic: refuses if the PR head moved after CI) and
#      fast-forwards local 'main' without requiring external review.
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

step "1/4: Local Quality Gates, Conventional Commit, and Push"
info "Running quality pipeline and pushing branch '$CURRENT_BRANCH'..."
PASSED_ARGS=()
for arg in "$@"; do
    if [[ "$arg" != "--auto-merge" && "$arg" != "--loop" ]]; then
        PASSED_ARGS+=("$arg")
    fi
done
./save.sh --push-pr "${PASSED_ARGS[@]}"

TARGET_HEAD_SHA=$(git rev-parse "$CURRENT_BRANCH")
info "Target HEAD commit SHA: $TARGET_HEAD_SHA"

step "2/4: Pull Request Verification or Creation"
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

PR_NUMBER=$(gh pr list --head "$CURRENT_BRANCH" --state open --json number --jq '.[0].number // empty' 2>/dev/null || true)

if [[ -z "$PR_NUMBER" ]]; then
    info "Creating new Pull Request for '$CURRENT_BRANCH' targeting 'main'..."
    LAST_COMMIT_MSG=$(git log -1 --pretty=%B)
    FIRST_LINE=$(echo "$LAST_COMMIT_MSG" | head -1)
    # The PR title becomes the squash subject; GitHub appends " (#NNN)" (pr-title.yml limit).
    if (( ${#FIRST_LINE} > 72 )); then
        error "Commit subject is ${#FIRST_LINE} characters; PR titles are limited to 72."
        error "Amend the commit subject, then re-run the loop."
        exit 1
    fi

    CLOSES_KEYWORD=""
    if [[ -f "./scripts/sync_issue.py" ]]; then
        # save.sh already mirrored the committed checkboxes; this never writes AI/BACKLOG.md.
        info "Mirroring committed AI/BACKLOG.md checkboxes to the GitHub issue..."
        if ! python3 ./scripts/sync_issue.py --auto --branch "$CURRENT_BRANCH"; then
            warn "GitHub issue sync FAILED (see above); nothing local was changed."
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

    PR_BODY="## Summary
$LAST_COMMIT_MSG
$CLOSES_KEYWORD

## Automated Quality & Security Checks
- [x] cargo fmt
- [x] cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
- [x] cargo test --locked --workspace --all-targets --all-features (including architectural invariants)
- [x] cargo deny --locked check (licenses, advisories, sources, bans)
- [x] Dual-layer candid review (fingerprint-bound AI/candid_review_report.md)"

    PR_URL=$(gh pr create --title "$FIRST_LINE" --body "$PR_BODY" --base main --head "$CURRENT_BRANCH")
    PR_NUMBER=$(gh pr view "$CURRENT_BRANCH" --json number -q .number)
    success "Pull Request #$PR_NUMBER created: $PR_URL"
else
    PR_URL="https://github.com/Mysticaly622/soos/pull/$PR_NUMBER"
    success "Existing Pull Request #$PR_NUMBER detected: $PR_URL"
fi

step "3/4: Monitoring CI Workflow Checks (GitHub Actions)"
info "Monitoring CI jobs for PR #$PR_NUMBER on commit ${TARGET_HEAD_SHA:0:12}..."

REPO_SLUG=$(gh repo view --json nameWithOwner --jq '.nameWithOwner')

# The PR head must be exactly the commit validated locally and CI must have
# started on it; otherwise 'gh pr checks' could report the previous head's
# green checks right after the push. Jobs without 'needs:' (lint, clippy, ...)
# get check runs immediately; 'CI Success' only appears once its dependencies
# finish, so it is awaited separately below.
SHA_READY=false
for _ in $(seq 1 36); do
    PR_HEAD=$(gh pr view "$PR_NUMBER" --json headRefOid --jq '.headRefOid' 2>/dev/null || true)
    RUNS=$(gh api "repos/${REPO_SLUG}/commits/${TARGET_HEAD_SHA}/check-runs" \
        --jq '.total_count' 2>/dev/null || echo 0)
    if [[ "$PR_HEAD" == "$TARGET_HEAD_SHA" && "$RUNS" =~ ^[1-9][0-9]*$ ]]; then
        SHA_READY=true
        break
    fi
    sleep 5
done
if [[ "$SHA_READY" != "true" ]]; then
    error "CI did not start on ${TARGET_HEAD_SHA:0:12} for PR #$PR_NUMBER after 180s."
    exit 1
fi

CI_WATCH_TIMEOUT_SECS="${CI_WATCH_TIMEOUT_SECS:-2700}"
WATCH_START=$(date +%s)

# Watch until completion, aborting on the first failing job (45 min ceiling,
# above the longest job timeout of 40 min).
if ! timeout "$CI_WATCH_TIMEOUT_SECS" gh pr checks "$PR_NUMBER" --watch --fail-fast --interval 15; then
    error "CI checks failed, were cancelled, or timed out on GitHub Actions!"
    gh pr checks "$PR_NUMBER" || true
    info "Inspect failures with: gh run view --log-failed"
    exit 1
fi

# Authoritative verdict: the aggregate check run of the exact validated commit.
# It may be created only after the other jobs complete, so poll until it exists
# and has completed.
CI_CONCLUSION="missing"
while true; do
    CI_CONCLUSION=$(gh api "repos/${REPO_SLUG}/commits/${TARGET_HEAD_SHA}/check-runs?check_name=CI%20Success" \
        --jq '[.check_runs[]] | if length == 0 then "missing"
              elif any(.status != "completed") then "pending"
              elif all(.conclusion == "success") then "success"
              else "failure" end' 2>/dev/null || echo "unknown")
    if [[ "$CI_CONCLUSION" == "success" || "$CI_CONCLUSION" == "failure" ]]; then
        break
    fi
    if (( $(date +%s) - WATCH_START > CI_WATCH_TIMEOUT_SECS )); then
        break
    fi
    sleep 10
done
if [[ "$CI_CONCLUSION" != "success" ]]; then
    error "'CI Success' for ${TARGET_HEAD_SHA:0:12} is '$CI_CONCLUSION' — refusing to merge."
    exit 1
fi
success "All CI checks passed on ${TARGET_HEAD_SHA:0:12}!"

step "4/4: Streamlined Auto-Merge to Main"
info "CI green: auto-merging PR #$PR_NUMBER into 'main'..."

IS_ALREADY_MERGED=$(gh pr view "$PR_NUMBER" --json state --jq '.state' 2>/dev/null || echo "UNKNOWN")
if [[ "$IS_ALREADY_MERGED" == "MERGED" ]]; then
    success "═════════════════════════════════════════════════════════════"
    success "  Pull Request #$PR_NUMBER already merged into main!"
    success "═════════════════════════════════════════════════════════════"
# --match-head-commit makes the merge atomic: GitHub refuses it if the PR head
# moved after CI validated TARGET_HEAD_SHA. No --admin: branch protection and
# required checks are never bypassed.
elif gh pr merge "$PR_NUMBER" --squash --match-head-commit "$TARGET_HEAD_SHA"; then
    success "═════════════════════════════════════════════════════════════"
    success "  Pull Request #$PR_NUMBER approved and merged into main!"
    success "═════════════════════════════════════════════════════════════"
else
    error "Failed to merge PR #$PR_NUMBER (head moved, protection rule, or conflict)."
    error "MERGE BLOCKED: re-run the review and release loop on the latest commit."
    exit 2
fi

info "Synchronizing local 'main' branch..."
if [[ -n "$(git status --porcelain)" ]]; then
    # Never stash: the stash stack is shared across worktrees and sessions.
    warn "Working tree has uncommitted changes; staying on '$CURRENT_BRANCH'."
    git fetch origin main
    warn "Run 'git switch main && git pull --ff-only origin main' once the tree is clean."
elif git worktree list --porcelain | grep -x 'branch refs/heads/main' >/dev/null; then
    # 'main' is checked out in another worktree; it cannot be switched to here.
    git fetch origin main
    warn "'main' is checked out in another worktree; update it there with 'git pull --ff-only'."
else
    git checkout main
    git pull --ff-only origin main
fi
success "Local 'main' branch synchronized. Mission accomplished!"
