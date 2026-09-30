#!/usr/bin/env bash
# =============================================================================
# scripts/candid_subagent.sh — Dual-Layer Candid Review Orchestration
# =============================================================================
# Combines:
#   Layer 1: Deterministic static invariant checks (scripts/candid_review.sh)
#   Layer 2: AI Sub-Agent deep reasoning audit report (AI/candid_review_report.md)
#
# Layer 2 is bound to the exact diff under review: the report MUST contain
#   - **Reviewed-Diff-Fingerprint**: `<sha256>`
# where <sha256> is the fingerprint of the diff between the merge-base with the
# base branch and the reviewed tree. The review-report singletons are excluded:
# the report file itself and AI/plan_evaluator_report.md (a per-issue process
# artifact that is rewritten for every issue, GitHub #267).
# Any code change after the review changes the fingerprint and fails the gate,
# so a stale, copied, or template report can never satisfy it.
#
# Usage:
#   ./scripts/candid_subagent.sh                 Layer 1 + Layer 2 on the working tree
#   ./scripts/candid_subagent.sh --prepare       Write target/candid_diff.patch, print fingerprint
#   ./scripts/candid_subagent.sh --fingerprint   Print the working-tree fingerprint
#   ./scripts/candid_subagent.sh --rev <rev> [--skip-layer1]
#                                                Verify the report committed in <rev>
#                                                (used by the pre-push hook and CI)
#
# Environment:
#   CANDID_BASE_REF   Base branch reference (default: origin/main, fallback: main)
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
error()   { echo -e "${RED}[ERROR]${NC} $*" >&2; }
step()    { echo -e "\n${BOLD}── $* ──${NC}"; }

# Always operate from the repository root (hooks and CI may call from elsewhere).
REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

readonly REPORT_FILE="AI/candid_review_report.md"
readonly PLAN_REPORT_FILE="AI/plan_evaluator_report.md"
# Pathspecs excluded from every review diff (fingerprint, file list, zero-change check).
readonly REVIEW_EXCLUDES=(":(exclude)${REPORT_FILE}" ":(exclude)${PLAN_REPORT_FILE}")
readonly PATCH_FILE="target/candid_diff.patch"

MODE="verify-worktree"
TARGET_REV=""
SKIP_LAYER1=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --prepare)     MODE="prepare"; shift ;;
        --fingerprint) MODE="fingerprint"; shift ;;
        --rev)
            if [[ $# -lt 2 ]]; then
                error "--rev requires a revision argument."
                exit 1
            fi
            MODE="verify-rev"; TARGET_REV="$2"; shift 2 ;;
        --skip-layer1) SKIP_LAYER1=true; shift ;;
        -h|--help)
            sed -n '2,29p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            error "Unknown option: $1"
            exit 1
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Base reference resolution
# ---------------------------------------------------------------------------
BASE_REF="${CANDID_BASE_REF:-origin/main}"
if ! git rev-parse --verify --quiet "${BASE_REF}^{commit}" >/dev/null; then
    BASE_REF="main"
fi
if ! git rev-parse --verify --quiet "${BASE_REF}^{commit}" >/dev/null; then
    error "Base reference not found (tried \${CANDID_BASE_REF:-origin/main} and main)."
    exit 1
fi

# ---------------------------------------------------------------------------
# Tree helpers
# ---------------------------------------------------------------------------
# Snapshot of the full working tree (tracked + untracked, honoring .gitignore)
# as a git tree object, built in a temporary index so the real index is untouched.
# Any git failure aborts (fail-closed) instead of silently falling back to HEAD.
worktree_tree() {
    local tmp_index tree
    tmp_index="$(mktemp)"
    rm -f "$tmp_index"
    if ! GIT_INDEX_FILE="$tmp_index" git read-tree HEAD \
        || ! GIT_INDEX_FILE="$tmp_index" git add -A -- . \
        || ! tree="$(GIT_INDEX_FILE="$tmp_index" git write-tree)"; then
        rm -f "$tmp_index"
        error "Failed to snapshot the working tree."
        exit 1
    fi
    rm -f "$tmp_index"
    echo "$tree"
}

# Diff between merge-base and a tree, excluding the review-report singletons.
# Every option that user/system git config could alter is pinned so that the
# fingerprint is identical on every machine and in CI.
review_diff() {
    local tree="$1" merge_base
    merge_base="$(git merge-base "$BASE_REF" "$2")"
    git -c core.quotePath=false -c diff.noprefix=false -c diff.mnemonicPrefix=false \
        -c diff.relative=false -c diff.suppressBlankEmpty=false \
        diff --binary --no-color --no-ext-diff --no-textconv --full-index --no-renames \
        --diff-algorithm=myers --indent-heuristic --src-prefix=a/ --dst-prefix=b/ \
        -O/dev/null --unified=3 --inter-hunk-context=0 "$merge_base" "$tree" -- . "${REVIEW_EXCLUDES[@]}"
}

fingerprint_of() {
    local tmp_patch fp
    tmp_patch="$(mktemp)"
    if ! review_diff "$1" "$2" > "$tmp_patch"; then
        rm -f "$tmp_patch"
        error "Failed to compute the review diff."
        exit 1
    fi
    fp="$(sha256sum < "$tmp_patch" | cut -d' ' -f1)"
    rm -f "$tmp_patch"
    echo "$fp"
}

# ---------------------------------------------------------------------------
# Modes: --fingerprint / --prepare
# ---------------------------------------------------------------------------
if [[ "$MODE" == "fingerprint" ]]; then
    TREE="$(worktree_tree)"
    fingerprint_of "$TREE" HEAD
    exit 0
fi

if [[ "$MODE" == "prepare" ]]; then
    mkdir -p target
    TREE="$(worktree_tree)"
    if ! review_diff "$TREE" HEAD > "$PATCH_FILE"; then
        error "Failed to compute the review diff."
        exit 1
    fi
    FP="$(sha256sum < "$PATCH_FILE" | cut -d' ' -f1)"
    MERGE_BASE="$(git merge-base "$BASE_REF" HEAD)"
    info "Base reference:  $BASE_REF (merge-base $(git rev-parse --short "$MERGE_BASE"))"
    info "Review patch:    $PATCH_FILE ($(wc -l < "$PATCH_FILE" | tr -d ' ') lines)"
    info "Files in diff:"
    git diff --name-only "$MERGE_BASE" "$TREE" -- . "${REVIEW_EXCLUDES[@]}" | sed 's/^/  • /'
    echo ""
    echo "Reviewed-Diff-Fingerprint: $FP"
    echo ""
    info "Record this fingerprint in $REPORT_FILE as:"
    echo "  - **Reviewed-Diff-Fingerprint**: \`$FP\`"
    exit 0
fi

# ---------------------------------------------------------------------------
# Layer 1: deterministic invariant checks
# ---------------------------------------------------------------------------
if [[ "$SKIP_LAYER1" != "true" ]]; then
    step "Candid Review Layer 1: Deterministic Invariant Checks"
    if [[ -x "./scripts/candid_review.sh" ]]; then
        ./scripts/candid_review.sh
    else
        error "Deterministic review script './scripts/candid_review.sh' not found!"
        exit 1
    fi
fi

# ---------------------------------------------------------------------------
# Layer 2: fingerprint-bound AI sub-agent report
# ---------------------------------------------------------------------------
step "Candid Review Layer 2: AI Sub-Agent Reasoning Gate"

if [[ "$MODE" == "verify-rev" ]]; then
    if ! git rev-parse --verify --quiet "${TARGET_REV}^{commit}" >/dev/null; then
        error "Revision '$TARGET_REV' not found."
        exit 1
    fi
    TREE="$(git rev-parse "${TARGET_REV}^{tree}")"
    ANCHOR="$TARGET_REV"
    REPORT_CONTENT="$(git show "${TARGET_REV}:${REPORT_FILE}" 2>/dev/null || true)"
    TARGET_LABEL="revision $(git rev-parse --short "$TARGET_REV")"
else
    TREE="$(worktree_tree)"
    ANCHOR="HEAD"
    REPORT_CONTENT="$(cat "$REPORT_FILE" 2>/dev/null || true)"
    TARGET_LABEL="working tree"
fi

MERGE_BASE="$(git merge-base "$BASE_REF" "$ANCHOR")"
if git diff --quiet "$MERGE_BASE" "$TREE" -- . "${REVIEW_EXCLUDES[@]}"; then
    success "Zero changes relative to $BASE_REF ($TARGET_LABEL). AI Sub-Agent review not required."
    exit 0
fi

EXPECTED_FP="$(fingerprint_of "$TREE" "$ANCHOR")"
info "Reviewed target:  $TARGET_LABEL"
info "Expected fingerprint: $EXPECTED_FP"

fail_gate() {
    error "═════════════════════════════════════════════════════════════"
    error "  AI Sub-Agent Candid Review gate FAILED: $1"
    error "  Run: ./scripts/candid_subagent.sh --prepare"
    error "  then perform the review per .agents/skills/candid-reviewer/SKILL.md"
    error "  and record the printed fingerprint in $REPORT_FILE."
    error "═════════════════════════════════════════════════════════════"
    exit 1
}

if [[ -z "$REPORT_CONTENT" ]]; then
    fail_gate "no report found at $REPORT_FILE"
fi

# Only verdict lines count (e.g. "**VERDICT: APPROVED**" at line start), not
# quotations of the words inside the report body. Any CHANGES_REQUESTED verdict
# line fails the gate, even if an APPROVED line appears elsewhere.
VERDICTS="$(grep -oE '^[[:space:]]*\*{0,2}VERDICT: (APPROVED|CHANGES_REQUESTED)' <<< "$REPORT_CONTENT" \
    | grep -oE '(APPROVED|CHANGES_REQUESTED)' || true)"
VERDICT="APPROVED"
if [[ -z "$VERDICTS" ]]; then
    VERDICT=""
elif grep -qx 'CHANGES_REQUESTED' <<< "$VERDICTS"; then
    VERDICT="CHANGES_REQUESTED"
fi

if [[ "$VERDICT" == "CHANGES_REQUESTED" ]]; then
    fail_gate "the report requests changes (VERDICT: CHANGES_REQUESTED)"
fi

if [[ "$VERDICT" != "APPROVED" ]]; then
    fail_gate "the report has no 'VERDICT: APPROVED' line"
fi

REPORT_FP="$(grep -oE 'Reviewed-Diff-Fingerprint\*{0,2}:\*{0,2}[[:space:]]*`?[0-9a-f]{64}' <<< "$REPORT_CONTENT" \
    | grep -oE '[0-9a-f]{64}' | head -n 1 || true)"

if [[ -z "$REPORT_FP" ]]; then
    fail_gate "the report has no 'Reviewed-Diff-Fingerprint' (stale or template report)"
fi

if [[ "$REPORT_FP" != "$EXPECTED_FP" ]]; then
    error "Report fingerprint:   $REPORT_FP"
    fail_gate "the report was written for a different diff (code changed after review)"
fi

success "═════════════════════════════════════════════════════════════"
success "  Dual-Layer Candid Review PASSED!"
if [[ "$SKIP_LAYER1" != "true" ]]; then
    success "  ✓ Layer 1: All deterministic architectural invariants verified"
fi
success "  ✓ Layer 2: APPROVED report bound to fingerprint ${EXPECTED_FP:0:12}…"
success "═════════════════════════════════════════════════════════════"
exit 0
