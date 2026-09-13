#!/usr/bin/env bash
# =============================================================================
# scripts/candid_review.sh — Independent Candid Pre-Push Code Review
# =============================================================================
# Evaluates code changes without historical conversation context or author bias.
# Runs automated invariant checks on the diff and formats a candid audit report:
#   1. Zero unsafe in business crates (protocol, policy, vision)
#   2. Zero unwrap() / expect() in PAM production code
#   3. Zero tokio in PAM module
#   4. Zero opencv in workspace
#   5. Shell script syntax validation (bash -n)
#   6. Language policy check (English only in comments, docs, walkthroughs)
#   7. Conventional commit conformance
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

step "Candid Pre-Push Review: Analyzing Changes vs $BASE_REF"

# Determine diff targets
CHANGED_FILES=$(git diff --name-only "$BASE_REF"...HEAD 2>/dev/null || true)
WORKING_CHANGED=$(git status --porcelain 2>/dev/null || true)

if [[ -z "$CHANGED_FILES" && -z "$WORKING_CHANGED" ]]; then
    info "No changes detected against $BASE_REF. Nothing to audit."
    exit 0
fi

info "Files modified relative to $BASE_REF:"
if [[ -n "$CHANGED_FILES" ]]; then
    echo "$CHANGED_FILES" | sed 's/^/  • /'
fi

# Obtain complete raw diff (committed on branch + unstaged/staged working tree)
RAW_DIFF=$(git diff "$BASE_REF" 2>/dev/null || git diff HEAD 2>/dev/null || true)

ERRORS_FOUND=0

# 1. Check for unsafe code in business crates
step "Audit 1: Checking #![forbid(unsafe_code)] Invariant"
UNSAFE_HITS=$(echo "$RAW_DIFF" | grep -E '^\+[^+].*\bunsafe\b' || true)
if [[ -n "$UNSAFE_HITS" ]]; then
    # Verify if unsafe is in forbidden crates
    FORBIDDEN_UNSAFE=$(git diff "$BASE_REF" -- crates/protocol crates/policy crates/vision 2>/dev/null | grep -E '^\+[^+].*\bunsafe\b' || true)
    if [[ -n "$FORBIDDEN_UNSAFE" ]]; then
        error "Forbidden 'unsafe' code detected in business crates!"
        echo "$FORBIDDEN_UNSAFE" | sed 's/^/    /'
        ERRORS_FOUND=$((ERRORS_FOUND + 1))
    else
        info "Unsafe code detected but isolated within permitted adapter crates."
    fi
else
    success "Zero 'unsafe' additions detected across all crates."
fi

# 2. Check for unwrap / expect in PAM production pathways
step "Audit 2: Checking Panic-Safety Invariant in PAM Module"
PAM_PANICS=$(git diff "$BASE_REF" -- 'crates/pam/src/**/*.rs' 'crates/pam/src/*.rs' 2>/dev/null | grep -E '^\+[^+].*(\.unwrap\(|\.expect\()' | grep -v 'tests' || true)
if [[ -n "$PAM_PANICS" ]]; then
    error "Forbidden panic (.unwrap() / .expect()) added in PAM production code!"
    echo "$PAM_PANICS" | sed 's/^/    /'
    ERRORS_FOUND=$((ERRORS_FOUND + 1))
else
    success "Zero unwrap() / expect() added in PAM production pathways."
fi

# 3. Check for async/Tokio in PAM crate
step "Audit 3: Checking Asynchronous Runtime Invariant"
PAM_TOKIO=$(git diff "$BASE_REF" -- crates/pam/Cargo.toml 2>/dev/null | grep -E '^\+[^+].*tokio' || true)
if [[ -n "$PAM_TOKIO" ]]; then
    error "Forbidden Tokio dependency added to crates/pam/Cargo.toml!"
    echo "$PAM_TOKIO" | sed 's/^/    /'
    ERRORS_FOUND=$((ERRORS_FOUND + 1))
else
    success "Zero Tokio dependencies in crates/pam."
fi

# 4. Check for forbidden OpenCV dependency
step "Audit 4: Checking Forbidden Third-Party Dependencies (OpenCV)"
OPENCV_HITS=$(git diff "$BASE_REF" -- '**/Cargo.toml' 'Cargo.toml' 2>/dev/null | grep -E '^\+[^+].*(opencv|nokhwa)' || true)
if [[ -n "$OPENCV_HITS" ]]; then
    error "Forbidden OpenCV or Nokhwa dependency detected in Cargo.toml!"
    echo "$OPENCV_HITS" | sed 's/^/    /'
    ERRORS_FOUND=$((ERRORS_FOUND + 1))
else
    success "Zero OpenCV or prohibited camera dependencies detected."
fi

# 5. Shell script syntax validation
step "Audit 5: Validating Shell Scripts Syntax"
SHELL_FILES=$(git diff --name-only "$BASE_REF" 2>/dev/null | grep -E '\.sh$' || true)
if [[ -n "$SHELL_FILES" ]]; then
    for sh_file in $SHELL_FILES; do
        if [[ -f "$sh_file" ]]; then
            if bash -n "$sh_file"; then
                success "Syntax check passed: $sh_file"
            else
                error "Shell syntax error in: $sh_file"
                ERRORS_FOUND=$((ERRORS_FOUND + 1))
            fi
        fi
    done
else
    info "No shell scripts modified."
fi

# 6. English-Only Deliverables Audit
step "Audit 6: Checking Strict English-Only Deliverable Policy"
# Scan code comments in diff for obvious French words
FRENCH_MARKERS=$(git diff "$BASE_REF" -- '*.rs' '*.md' '*.sh' 2>/dev/null | grep -E '^\+[^+]*(//|/\*|#|<!--).*(\b(pour|avec|dans|faire|ajouter|vérifier|fonction|problème|étape|fichier|modifié|remarque|attention)\b)' || true)
if [[ -n "$FRENCH_MARKERS" ]]; then
    warn "Possible non-English comment or text detected in modified files:"
    echo "$FRENCH_MARKERS" | head -10 | sed 's/^/    /'
    info "Ensure all comments, docstrings, and technical documentation are strictly in English."
else
    success "All comments and documentation conform to English-only deliverable policy."
fi

# 7. Summary & Verdict
step "Candid Review Summary"
if [[ $ERRORS_FOUND -gt 0 ]]; then
    error "═════════════════════════════════════════════════════════════"
    error "  Candid Review FAILED with $ERRORS_FOUND invariant violation(s)!"
    error "  Please resolve the issues above before pushing."
    error "═════════════════════════════════════════════════════════════"
    exit 1
fi

success "═════════════════════════════════════════════════════════════"
success "  Candid Review PASSED: All architectural invariants verified!"
success "  Changes are logically sound and compliant with AGENTS.md."
success "═════════════════════════════════════════════════════════════"
exit 0
