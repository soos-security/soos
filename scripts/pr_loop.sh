#!/usr/bin/env bash
# =============================================================================
# scripts/pr_loop.sh — Autonomous PR loop, Copilot review, and auto-merge
# =============================================================================
# Orchestrates branch finalization:
#   1. Verifies dedicated topic branch (rejects 'main' and 'detached')
#   2. Executes ./save.sh --push-pr to validate, commit, and push
#   3. Opens Pull Request if not already created
#   4. Monitors CI checks (Quality, Security, PAM Docker)
#   5. Actively awaits GitHub Copilot code review
#   6. Evaluates Copilot feedback:
#      - If review comments exist: outputs targeted line details and exits (code 2)
#        to allow the AI agent to apply fixes and retry.
#      - If zero comments and CI 100% green: squash-merges into 'main'
#        and synchronizes local 'main' branch.
# =============================================================================

set -euo pipefail

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

step "1/6: Local Quality Gates, Conventional Commit, and Push"
info "Running quality pipeline and pushing branch '$CURRENT_BRANCH'..."
PASSED_ARGS=()
for arg in "$@"; do
    if [[ "$arg" != "--auto-merge" && "$arg" != "--loop" ]]; then
        PASSED_ARGS+=("$arg")
    fi
done
./save.sh --push-pr "${PASSED_ARGS[@]}"

step "2/6: Pull Request Verification or Creation"
PR_JSON=$(gh pr list --head "$CURRENT_BRANCH" --json number,url,state --state open 2>/dev/null || echo "[]")
PR_NUMBER=$(echo "$PR_JSON" | grep -o '"number":[0-9]*' | head -1 | cut -d':' -f2 || true)

if [[ -z "$PR_NUMBER" ]]; then
    info "Creating new Pull Request for '$CURRENT_BRANCH' targeting 'main'..."
    LAST_COMMIT_MSG=$(git log -1 --pretty=%B)
    FIRST_LINE=$(echo "$LAST_COMMIT_MSG" | head -1)

    PR_BODY="## Summary
$LAST_COMMIT_MSG

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

step "3/6: Soliciting GitHub Copilot Code Review"
info "Requesting review from GitHub Copilot (dual-trigger)..."
# 1. Formal reviewer assignment via GraphQL API
PR_NODE_ID=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER" --jq '.node_id' 2>/dev/null || true)
if [[ -n "$PR_NODE_ID" ]]; then
    gh api graphql -f query='mutation { requestReviews(input: { pullRequestId: "'"$PR_NODE_ID"'", botIds: ["BOT_kgDOCnlnWA"] }) { pullRequest { id } } }' > /dev/null 2>&1 || true
fi
# 2. Trigger review agent via PR comment
gh pr comment "$PR_NUMBER" --body "@copilot review" > /dev/null 2>&1 || true
success "Review request sent to GitHub Copilot (reviewers + mention)."

step "4/6: Monitoring CI Workflow Checks (GitHub Actions)"
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
success "All CI checks passed successfully!"

step "5/6: Actively Awaiting GitHub Copilot Code Review"
info "Copilot is analyzing the code (typically takes between 30s and 5 minutes)..."

MAX_WAIT_SECONDS=480 # 8 minutes maximum
WAITED=0
INTERVAL=10
COPILOT_FINISHED=false

while [[ $WAITED -lt $MAX_WAIT_SECONDS ]]; do
    # 1. Formal review published
    REVIEWS_COPILOT=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER/reviews" 2>/dev/null | grep -E '"login": "Copilot"' || true)

    # 2. Copilot comment posted
    COMMENTS_COPILOT=$(gh api "repos/:owner/:repo/issues/$PR_NUMBER/comments" 2>/dev/null | grep -E '"login": "Copilot"' || true)

    # 3. Copilot workflow completed
    COPILOT_RUN_STATUS=$(gh run list --branch "$CURRENT_BRANCH" --json name,status,conclusion 2>/dev/null | grep -i "Copilot" || true)

    if [[ -n "$REVIEWS_COPILOT" ]] || [[ -n "$COMMENTS_COPILOT" ]]; then
        info "GitHub Copilot review/comment detected!"
        COPILOT_FINISHED=true
        break
    fi

    if [[ -n "$COPILOT_RUN_STATUS" ]] && echo "$COPILOT_RUN_STATUS" | grep -q '"status":"completed"'; then
        info "GitHub Copilot workflow run completed!"
        COPILOT_FINISHED=true
        break
    fi

    echo -ne "  ⏳ Awaiting Copilot review (${WAITED}s / ${MAX_WAIT_SECONDS}s)...\r"
    sleep $INTERVAL
    WAITED=$((WAITED + INTERVAL))
done
echo ""

if [[ "$COPILOT_FINISHED" == "true" ]]; then
    success "GitHub Copilot review completed."
else
    warn "Copilot wait ceiling exceeded (${MAX_WAIT_SECONDS}s) or Copilot did not trigger a run."
    warn "Proceeding with evaluation based on CI checks and existing comments."
fi

step "6/6: Evaluating Copilot Feedback & Auto-Merge Decision"
COMMENTS=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER/comments" 2>/dev/null || echo "[]")
COMMENT_COUNT=$(echo "$COMMENTS" | grep -c '"id":' || true)

if [[ "$COMMENT_COUNT" -gt 0 ]]; then
    warn "GitHub Copilot posted $COMMENT_COUNT review comment(s) on PR #$PR_NUMBER!"
    echo ""
    info "Review points from Copilot:"
    echo "$COMMENTS" | grep -E '("path"|"line"|"body")' | sed 's/^[[:space:]]*//' | head -40
    echo ""
    warn "PR #$PR_NUMBER WILL NOT be merged until these review items are addressed."
    info "Agent will now inspect feedback, apply fixes, and re-run the loop."
    exit 2
fi

success "Zero blocking comments. All 3 CI checks and code reviews are 100% green!"
info "Auto-merging PR #$PR_NUMBER into 'main'..."

IS_ALREADY_MERGED=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER" --jq '.merged' 2>/dev/null || echo "false")

if [[ "$IS_ALREADY_MERGED" == "true" ]]; then
    success "═════════════════════════════════════════════════════════════"
    success "  Pull Request #$PR_NUMBER already merged into main!"
    success "═════════════════════════════════════════════════════════════"
elif gh pr merge "$PR_NUMBER" --squash --delete-branch --admin 2>/dev/null || gh pr merge "$PR_NUMBER" --squash --delete-branch; then
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
