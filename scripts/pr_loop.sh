#!/usr/bin/env bash
# =============================================================================
# scripts/pr_loop.sh — Autonomous PR loop, Copilot ping-pong review & auto-merge
# =============================================================================
# Orchestrates branch finalization with strict review gating:
#   1. Verifies dedicated topic branch (rejects 'main' and 'detached')
#   2. Executes ./save.sh --push-pr to validate, commit, and push
#   3. Opens Pull Request if not already created
#   4. Monitors CI checks (Quality, Security, PAM Docker)
#   5. Actively awaits GitHub Copilot code review for target commit SHA
#   6. Evaluates Copilot feedback:
#      - If review comments exist OR changes are recommended:
#        outputs targeted details and exits (code 2) for the AI agent
#        to apply fixes, commit, and re-run (ping-pong cycle).
#      - Only if ZERO comments, NO changes recommended, and CI 100% green:
#        squash-merges into 'main' and synchronizes local 'main' branch.
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

step "5/6: Actively Awaiting GitHub Copilot Code Review for $TARGET_HEAD_SHA"
info "Copilot is analyzing commit $TARGET_HEAD_SHA..."

MAX_WAIT_SECONDS=600 # 10 minutes maximum
WAITED=0
INTERVAL=10
COPILOT_FINISHED=false
MINIMUM_WAIT_SECONDS=40 # Allow at least 40s for Copilot run to trigger
TARGET_SHORT_SHA=$(echo "$TARGET_HEAD_SHA" | cut -c1-7)
COPILOT_RUN_NAME_REGEX='Copilot|Addressing comment on PR'

while [[ $WAITED -lt $MAX_WAIT_SECONDS ]]; do
    # Check all Copilot workflow runs on the repository for this branch
    if ! RUNS_JSON=$(gh run list --branch "$CURRENT_BRANCH" --json name,status,conclusion,headSha 2>/dev/null); then
        error "Failed to query workflow runs while waiting for Copilot review."
        exit 1
    fi
    
    # Are any Copilot runs currently queued or in_progress?
    ACTIVE_COPILOT_RUNS=$(echo "$RUNS_JSON" | jq -r ".[] | select(.name | test(\"$COPILOT_RUN_NAME_REGEX\"; \"i\")) | select(.headSha == \"$TARGET_HEAD_SHA\") | select(.status != \"completed\")" 2>/dev/null || true)
    
    # Check if a completed Copilot workflow exists specifically for TARGET_HEAD_SHA
    COMPLETED_TARGET_RUN=$(echo "$RUNS_JSON" | jq -r ".[] | select(.name | test(\"$COPILOT_RUN_NAME_REGEX\"; \"i\")) | select(.headSha == \"$TARGET_HEAD_SHA\") | select(.status == \"completed\") | select(.conclusion == \"success\")" 2>/dev/null || true)
    
    # Check if a review object exists specifically for TARGET_HEAD_SHA
    if ! REVIEWS_JSON=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER/reviews" 2>/dev/null); then
        error "Failed to query PR reviews for PR #$PR_NUMBER."
        exit 1
    fi
    MATCHING_REVIEW=$(echo "$REVIEWS_JSON" | jq -c "[.[] | select(.user.login | test(\"Copilot\"; \"i\")) | select(.commit_id == \"$TARGET_HEAD_SHA\")] | last // empty" 2>/dev/null || true)
    
    # Check if Copilot commented directly in PR conversation (issue comments)
    if ! ISSUE_COMMENTS_JSON=$(gh api "repos/:owner/:repo/issues/$PR_NUMBER/comments" 2>/dev/null); then
        error "Failed to query PR issue comments for PR #$PR_NUMBER."
        exit 1
    fi
    MATCHING_ISSUE_COMMENT=$(echo "$ISSUE_COMMENTS_JSON" | jq -r ".[] | select(.user.login | test(\"Copilot\"; \"i\")) | select((.body | test(\"$TARGET_SHORT_SHA|$TARGET_HEAD_SHA\"; \"i\")) or (.body | test(\"Reviewed the latest commit\"; \"i\"))) | .body" 2>/dev/null | tail -1 || true)

    if [[ -n "$MATCHING_REVIEW" ]]; then
        info "Formal review for commit $TARGET_HEAD_SHA submitted by GitHub Copilot!"
        COPILOT_FINISHED=true
        break
    elif [[ -n "$MATCHING_ISSUE_COMMENT" && -n "$COMPLETED_TARGET_RUN" && $WAITED -ge $MINIMUM_WAIT_SECONDS ]]; then
        info "GitHub Copilot posted a review response in PR comments!"
        COPILOT_FINISHED=true
        break
    elif [[ -n "$ACTIVE_COPILOT_RUNS" ]]; then
        # Copilot is currently active, keep waiting
        echo -ne "  ⏳ Copilot analysis actively running (${WAITED}s / ${MAX_WAIT_SECONDS}s)...\r"
    elif [[ -n "$COMPLETED_TARGET_RUN" && $WAITED -ge $MINIMUM_WAIT_SECONDS ]]; then
        info "Copilot workflow run for commit $TARGET_HEAD_SHA completed!"
    else
        echo -ne "  ⏳ Awaiting Copilot review trigger/completion (${WAITED}s / ${MAX_WAIT_SECONDS}s)...\r"
    fi

    sleep $INTERVAL
    WAITED=$((WAITED + INTERVAL))
done
echo ""

if [[ "$COPILOT_FINISHED" == "true" ]]; then
    success "GitHub Copilot analysis completed for $TARGET_HEAD_SHA."
else
    error "Copilot analysis for $TARGET_HEAD_SHA did not complete within ${MAX_WAIT_SECONDS}s!"
    error "MERGE BLOCKED: Strict ping-pong policy requires a completed Copilot review on target commit."
    exit 1
fi

step "6/6: Evaluating Copilot Feedback & Auto-Merge Decision"
REPO_FULL_NAME=$(gh repo view --json nameWithOwner --jq '.nameWithOwner' 2>/dev/null)
if [[ -z "$REPO_FULL_NAME" || "$REPO_FULL_NAME" != */* ]]; then
    error "Failed to resolve repository owner/name via gh repo view."
    exit 1
fi
REPO_OWNER="${REPO_FULL_NAME%/*}"
REPO_NAME="${REPO_FULL_NAME#*/}"

# 1. Fetch unresolved review threads via GraphQL (paginated)
UNRESOLVED_THREADS=0
THREADS_HAS_NEXT_PAGE=true
THREADS_CURSOR=""

while [[ "$THREADS_HAS_NEXT_PAGE" == "true" ]]; do
    if [[ -n "$THREADS_CURSOR" ]]; then
        if ! THREADS_PAGE_JSON=$(gh api graphql \
            -f query='query($owner: String!, $repo: String!, $prNumber: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $prNumber) {
      reviewThreads(first: 100, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes { isResolved }
      }
    }
  }
}' \
            -F owner="$REPO_OWNER" \
            -F repo="$REPO_NAME" \
            -F prNumber="$PR_NUMBER" \
            -F after="$THREADS_CURSOR" 2>/dev/null); then
            error "Failed to query review threads page for PR #$PR_NUMBER."
            exit 1
        fi
    else
        if ! THREADS_PAGE_JSON=$(gh api graphql \
            -f query='query($owner: String!, $repo: String!, $prNumber: Int!, $after: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $prNumber) {
      reviewThreads(first: 100, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes { isResolved }
      }
    }
  }
}' \
            -F owner="$REPO_OWNER" \
            -F repo="$REPO_NAME" \
            -F prNumber="$PR_NUMBER" 2>/dev/null); then
            error "Failed to query review threads for PR #$PR_NUMBER."
            exit 1
        fi
    fi

    PAGE_UNRESOLVED=$(echo "$THREADS_PAGE_JSON" | jq -r '[.data.repository.pullRequest.reviewThreads.nodes[] | select(.isResolved == false)] | length')
    UNRESOLVED_THREADS=$((UNRESOLVED_THREADS + PAGE_UNRESOLVED))
    THREADS_HAS_NEXT_PAGE=$(echo "$THREADS_PAGE_JSON" | jq -r '.data.repository.pullRequest.reviewThreads.pageInfo.hasNextPage')
    THREADS_CURSOR=$(echo "$THREADS_PAGE_JSON" | jq -r '.data.repository.pullRequest.reviewThreads.pageInfo.endCursor // empty')
done

# 2. Fetch line comments on TARGET_HEAD_SHA
if ! COMMENTS=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER/comments" 2>/dev/null); then
    error "Failed to query line-level review comments for PR #$PR_NUMBER."
    exit 1
fi
TARGET_COMMENTS=$(echo "$COMMENTS" | jq -r "[.[] | select(.commit_id == \"$TARGET_HEAD_SHA\")] | length" 2>/dev/null || echo "0")

# 3. Fetch Copilot review specifically for TARGET_HEAD_SHA
if ! REVIEWS_JSON=$(gh api "repos/:owner/:repo/pulls/$PR_NUMBER/reviews" 2>/dev/null); then
    error "Failed to query review state for PR #$PR_NUMBER."
    exit 1
fi
MATCHING_REVIEW=$(echo "$REVIEWS_JSON" | jq -r "[.[] | select(.user.login | test(\"Copilot\"; \"i\")) | select(.commit_id == \"$TARGET_HEAD_SHA\")] | last // empty" 2>/dev/null || true)

if [[ -z "$MATCHING_REVIEW" ]]; then
    error "No GitHub Copilot review found for target commit $TARGET_HEAD_SHA."
    error "MERGE BLOCKED: target commit must have explicit Copilot feedback."
    exit 1
fi

REVIEW_STATE=""
REVIEW_BODY=""
if [[ -n "$MATCHING_REVIEW" ]]; then
    REVIEW_STATE=$(echo "$MATCHING_REVIEW" | jq -r '.state // empty')
    REVIEW_BODY=$(echo "$MATCHING_REVIEW" | jq -r '.body // empty')
fi

# 4. Check Copilot issue comment in PR conversation
if ! ISSUE_COMMENTS_JSON=$(gh api "repos/:owner/:repo/issues/$PR_NUMBER/comments" 2>/dev/null); then
    error "Failed to query PR conversation comments for PR #$PR_NUMBER."
    exit 1
fi
TARGET_ISSUE_COMMENTS=$(echo "$ISSUE_COMMENTS_JSON" | jq -r "[.[] | select(.user.login | test(\"Copilot\"; \"i\")) | select((.body | test(\"$TARGET_SHORT_SHA|$TARGET_HEAD_SHA\"; \"i\")) or (.body | test(\"Reviewed the latest commit\"; \"i\")))] | length" 2>/dev/null || echo "0")
MATCHING_ISSUE_COMMENT=$(echo "$ISSUE_COMMENTS_JSON" | jq -r ".[] | select(.user.login | test(\"Copilot\"; \"i\")) | select((.body | test(\"$TARGET_SHORT_SHA|$TARGET_HEAD_SHA\"; \"i\")) or (.body | test(\"Reviewed the latest commit\"; \"i\"))) | .body" 2>/dev/null | tail -1 || true)

# Check for recommendations or changes
CHANGES_REQUESTED=false

if [[ "$UNRESOLVED_THREADS" -gt 0 ]]; then
    info "Found $UNRESOLVED_THREADS unresolved review thread(s)."
    CHANGES_REQUESTED=true
fi

if [[ "$TARGET_COMMENTS" -gt 0 ]]; then
    info "Found $TARGET_COMMENTS comment(s) on target commit $TARGET_HEAD_SHA."
    CHANGES_REQUESTED=true
fi

if [[ "$TARGET_ISSUE_COMMENTS" -gt 0 ]]; then
    info "Found $TARGET_ISSUE_COMMENTS Copilot issue comment(s) for target commit context."
fi

if [[ "$REVIEW_STATE" == "CHANGES_REQUESTED" ]]; then
    info "Review state is CHANGES_REQUESTED."
    CHANGES_REQUESTED=true
fi

if echo "$REVIEW_BODY" | grep -qiE "(### 🟡|Changes recommended|Critical issues|Moderate issues|Suppressed comments|Changes requested)"; then
    info "Review body indicates changes are recommended or issues found."
    CHANGES_REQUESTED=true
fi

if [[ -n "$MATCHING_ISSUE_COMMENT" ]]; then
    info "Copilot conversation comment: $(echo "$MATCHING_ISSUE_COMMENT" | head -2)"
    if echo "$MATCHING_ISSUE_COMMENT" | grep -qiE "(didn't find|didn’t find|no additional blocking|looks good|no issues found)"; then
        info "Copilot explicitly confirmed no blocking issues in conversation comment!"
    elif echo "$MATCHING_ISSUE_COMMENT" | grep -qiE "(blocking issue|changes recommended|critical issue|moderate issue|changes requested|please fix)"; then
        info "Copilot conversation comment reported issues to resolve."
        CHANGES_REQUESTED=true
        REVIEW_BODY="${REVIEW_BODY}
${MATCHING_ISSUE_COMMENT}"
    fi
fi

if [[ "$CHANGES_REQUESTED" == "true" ]]; then
    echo ""
    warn "═════════════════════════════════════════════════════════════"
    warn "  GitHub Copilot has requested changes on PR #$PR_NUMBER!"
    warn "═════════════════════════════════════════════════════════════"
    echo ""
    if [[ "$TARGET_COMMENTS" -gt 0 ]]; then
        info "Specific line comments on $TARGET_HEAD_SHA ($TARGET_COMMENTS):"
        echo "$COMMENTS" | jq -r ".[] | select(.commit_id == \"$TARGET_HEAD_SHA\") | \"[\(.path):\(.line // .original_line // \"?\")] \(.body)\"" | head -30
        echo ""
    fi
    if [[ -n "$REVIEW_BODY" ]]; then
        info "Copilot review summary:"
        echo "$REVIEW_BODY" | head -60
        echo ""
    fi
    warn "MERGE BLOCKED: The PR will NOT be merged until all feedback is addressed."
    info "Ping-pong required: analyze points above, apply corrections, commit, and re-run."
    exit 2
fi

success "Zero blocking comments and zero changes recommended. Copilot approved!"
info "Auto-merging PR #$PR_NUMBER into 'main'..."

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
