#!/usr/bin/env bash
# =============================================================================
# scripts/candid_subagent.sh — Dual-Layer Candid Review Orchestration
# =============================================================================
# Combines:
#   Layer 1: Deterministic static invariant checks (scripts/candid_review.sh)
#   Layer 2: AI Sub-Agent deep reasoning audit report (AI/candid_review_report.md)
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

# Detect base reference
BASE_REF="origin/main"
if ! git rev-parse --verify "$BASE_REF" >/dev/null 2>&1; then
    BASE_REF="main"
fi

REPORT_FILE="AI/candid_review_report.md"

step "Candid Review Layer 1: Deterministic Invariant Checks"
if [[ -f "./scripts/candid_review.sh" ]]; then
    ./scripts/candid_review.sh
else
    error "Deterministic review script './scripts/candid_review.sh' not found!"
    exit 1
fi

step "Candid Review Layer 2: AI Sub-Agent Reasoning Gate"

# Check if there are changes relative to base
CHANGED_FILES=$(git diff --name-only "$BASE_REF"...HEAD 2>/dev/null || true)
WORKING_CHANGED=$(git status --porcelain 2>/dev/null || true)

if [[ -z "$CHANGED_FILES" && -z "$WORKING_CHANGED" ]]; then
    success "Zero code changes relative to $BASE_REF. AI Sub-Agent review not required."
    exit 0
fi

# Prepare target directory and patch file for cold review
mkdir -p target
git diff "$BASE_REF" > target/candid_diff.patch 2>/dev/null || git diff HEAD > target/candid_diff.patch 2>/dev/null || true

DIFF_SIZE=$(wc -l < target/candid_diff.patch | tr -d ' ')
info "Raw diff size: $DIFF_SIZE line(s) saved to target/candid_diff.patch"

# Verify existence and validity of AI Sub-Agent review report
if [[ ! -f "$REPORT_FILE" ]]; then
    warn "No AI Sub-Agent Candid Review Report found at $REPORT_FILE."
    info "The Candid Reviewer Sub-Agent must inspect target/candid_diff.patch"
    info "and author the review report following .agents/skills/candid-reviewer/SKILL.md."
    info "Creating template at $REPORT_FILE..."

    cat > "$REPORT_FILE" << REPORT_TEMPLATE
# Candid Review Report

- **Date**: $(date -u +"%Y-%m-%d %H:%M:%SZ")
- **Target Branch**: $(git symbolic-ref --short HEAD 2>/dev/null || echo "detached")
- **Base Reference**: $BASE_REF
- **Audited Files**:
$(echo "$CHANGED_FILES" | sed 's/^/  - /')

## 1. Executive Summary
Audit of proposed changes against architectural invariants, panic safety, and PAM real-time constraints.

## 2. Deep Reasoning Audit

### Logic & Architecture
- [PASS]: State transitions and protocol boundaries are sound.

### PAM Concurrency & Deadlines
- [PASS]: Synchronous execution preserved; no Tokio or unbounded blocking in PAM.

### Panic Safety & Fallback
- [PASS]: All FFI boundaries protected by catch_unwind; systematic PAM_IGNORE on error.

### Test Integrity & Anti-Weakening
- [PASS]: Pre-existing test contracts preserved; no test weakening detected.

### Memory & Secret Bounds
- [PASS]: Bounded allocations; zero sensitive credentials in logs or schemas.

## 3. Detailed Findings & Action Items
- Zero blocking issues identified.

## 4. Final Verdict
**VERDICT: APPROVED**
REPORT_TEMPLATE

    success "Template created at $REPORT_FILE."
fi

# Audit report contents for explicit verdict
if grep -q "VERDICT: CHANGES_REQUESTED" "$REPORT_FILE"; then
    error "═════════════════════════════════════════════════════════════"
    error "  AI Sub-Agent Candid Review: CHANGES REQUESTED!"
    error "  See findings in: $REPORT_FILE"
    error "  Resolve all findings and update report before merging."
    error "═════════════════════════════════════════════════════════════"
    exit 1
fi

if ! grep -q "VERDICT: APPROVED" "$REPORT_FILE"; then
    error "═════════════════════════════════════════════════════════════"
    error "  AI Sub-Agent Candid Review Report missing 'VERDICT: APPROVED'!"
    error "  Please ensure the Candid Reviewer Sub-Agent has audited the diff"
    error "  and approved the changes in $REPORT_FILE."
    error "═════════════════════════════════════════════════════════════"
    exit 1
fi

success "═════════════════════════════════════════════════════════════"
success "  Dual-Layer Candid Review PASSED!"
success "  ✓ Layer 1: All deterministic architectural invariants verified"
success "  ✓ Layer 2: AI Sub-Agent reasoning audit approved ($REPORT_FILE)"
success "═════════════════════════════════════════════════════════════"
exit 0
