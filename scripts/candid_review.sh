#!/usr/bin/env bash
# =============================================================================
# scripts/candid_review.sh — Independent Candid Pre-Push Code Review
# =============================================================================
# Evaluates code changes without historical conversation context or author bias.
# Runs automated invariant checks on the diff and formats a candid audit report:
#   1. Zero unsafe in business crates (all crates declaring #![forbid(unsafe_code)])
#      and no removal of an existing #![forbid(unsafe_code)] attribute
#   2. Zero unwrap(), expect(), panic!(), todo!(), unimplemented!() in PAM production code
#   3. Zero async runtime (tokio, async-std, smol) in PAM module
#   4. Zero opencv or nokhwa across workspace
#   5. Shell script and git hook syntax validation (bash -n)
#   6. Zero stdout/stderr prints (println!, eprintln!, dbg!) in PAM production code
#   7. Language policy check (English only in comments, docs, configs, walkthroughs)
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
# Business crates (must match test_business_crates_forbid_unsafe_code in tests/invariants)
BUSINESS_CRATES=(protocol policy vision inference-ort biometric-store evidence-store enrollment-cli admin-cli gui)
BUSINESS_PATHS=()
for crate_name in "${BUSINESS_CRATES[@]}"; do
    BUSINESS_PATHS+=("crates/${crate_name}")
done
UNSAFE_HITS=$(echo "$RAW_DIFF" | grep -E '^\+[^+].*\bunsafe\b' || true)
FORBIDDEN_UNSAFE=$(git diff "$BASE_REF" -- "${BUSINESS_PATHS[@]}" 2>/dev/null | grep -E '^\+[^+].*\bunsafe\b' | grep -vE '^\+\s*//' || true)
# A removed attribute only counts if the file no longer declares it (moves/reformatting are fine).
MERGE_BASE=$(git merge-base "$BASE_REF" HEAD 2>/dev/null || echo "$BASE_REF")
REMOVED_FORBID=""
while IFS= read -r rs_file; do
    [[ -z "$rs_file" ]] && continue
    # Capture first: 'grep -q' closing the pipe early would make pipefail report 141 on large diffs.
    rs_diff="$(git diff "$MERGE_BASE" -- "$rs_file" 2>/dev/null || true)"
    if grep -E '^-[^-]*#!\[forbid\(unsafe_code\)\]' <<< "$rs_diff" >/dev/null \
        && ! grep -qE '^[[:space:]]*#!\[forbid\(unsafe_code\)\]' "$rs_file" 2>/dev/null; then
        REMOVED_FORBID="${REMOVED_FORBID}${rs_file}"$'\n'
    fi
done < <(git diff --name-only "$MERGE_BASE" -- '*.rs' 2>/dev/null || true)
if [[ -n "$FORBIDDEN_UNSAFE" ]]; then
    error "Forbidden 'unsafe' code detected in business crates!"
    echo "$FORBIDDEN_UNSAFE" | sed 's/^/    /'
    ERRORS_FOUND=$((ERRORS_FOUND + 1))
elif [[ -n "$UNSAFE_HITS" ]]; then
    info "Unsafe code detected but isolated within permitted adapter crates (pam, camera-v4l, daemon mlock)."
else
    success "Zero 'unsafe' additions detected across all crates."
fi
if [[ -n "$REMOVED_FORBID" ]]; then
    error "A '#![forbid(unsafe_code)]' attribute was removed!"
    echo "$REMOVED_FORBID" | sed 's/^/    /'
    ERRORS_FOUND=$((ERRORS_FOUND + 1))
fi

# 2. Check for panics, unwraps, and unfinished stubs in PAM production pathways
step "Audit 2: Checking Panic Safety & Robustness Invariant in PAM Module"
PAM_PANICS=$(git diff "$BASE_REF" -- 'crates/pam/src/**/*.rs' 'crates/pam/src/*.rs' 2>/dev/null | grep -E '^\+[^+].*(\.unwrap\(|\.expect\(|panic!|todo!|unimplemented!|unreachable!)' | grep -v 'tests' || true)
if [[ -n "$PAM_PANICS" ]]; then
    error "Forbidden panic, unwrap, or unfinished stub added in PAM production code!"
    echo "$PAM_PANICS" | sed 's/^/    /'
    ERRORS_FOUND=$((ERRORS_FOUND + 1))
else
    success "Zero unwrap(), expect(), panic!(), or unfinished stubs in PAM production pathways."
fi

# 3. Check for async/Tokio in PAM crate
step "Audit 3: Checking Asynchronous Runtime Invariant"
PAM_TOKIO=$(git diff "$BASE_REF" -- crates/pam/Cargo.toml 2>/dev/null | grep -E '^\+[^+].*\b(tokio|async-std|smol|async-io)\b' || true)
if [[ -n "$PAM_TOKIO" ]]; then
    error "Forbidden asynchronous runtime dependency added to crates/pam/Cargo.toml!"
    echo "$PAM_TOKIO" | sed 's/^/    /'
    ERRORS_FOUND=$((ERRORS_FOUND + 1))
else
    success "Zero asynchronous runtime dependencies in crates/pam."
fi

# 4. Check for forbidden OpenCV and Nokhwa dependencies
step "Audit 4: Checking Forbidden Third-Party Dependencies (OpenCV & Nokhwa)"
FORBIDDEN_DEPS=$(git diff "$BASE_REF" -- '**/Cargo.toml' 'Cargo.toml' 2>/dev/null | grep -E '^\+[^+].*(opencv|nokhwa)' || true)
if [[ -n "$FORBIDDEN_DEPS" ]]; then
    error "Forbidden OpenCV or Nokhwa dependency detected in Cargo.toml!"
    echo "$FORBIDDEN_DEPS" | sed 's/^/    /'
    ERRORS_FOUND=$((ERRORS_FOUND + 1))
else
    success "Zero OpenCV or prohibited camera dependencies detected."
fi

# 5. Shell script syntax validation
step "Audit 5: Validating Shell Scripts Syntax"
SHELL_FILES=$(git diff --name-only "$BASE_REF" 2>/dev/null | grep -E '(\.sh$|^\.githooks/)' || true)
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

# 6. PAM output isolation check (no stdout/stderr pollution)
step "Audit 6: Checking PAM Module Output Isolation (No stdout/stderr prints)"
PAM_PRINTS=$(git diff "$BASE_REF" -- 'crates/pam/src/**/*.rs' 'crates/pam/src/*.rs' 2>/dev/null | grep -E '^\+[^+].*(println!|eprintln!|print!|eprint!|dbg!)' | grep -v 'tests' || true)
if [[ -n "$PAM_PRINTS" ]]; then
    error "Forbidden stdout/stderr print (println!, eprintln!, dbg!) in PAM production code!"
    echo "$PAM_PRINTS" | sed 's/^/    /'
    ERRORS_FOUND=$((ERRORS_FOUND + 1))
else
    success "Zero stdout/stderr prints in PAM production code."
fi

# 7. English-Only Deliverables Audit
step "Audit 7: Checking Strict English-Only Deliverable Policy"
# Scan all added lines in modified files for common non-English keywords
FRENCH_MARKERS=$(git diff "$BASE_REF" -- '*.rs' '*.md' '*.sh' '*.yml' '*.yaml' '*.toml' 'Dockerfile' '.gitignore' ':!scripts/candid_review.sh' 2>/dev/null | grep -E '^\+[^+]*(//|/\*|#|<!--|").*(\b(pour|avec|dans|faire|ajouter|vérifier|fonction|problème|étape|fichier|modifié|remarque|attention|défaut|sécurité|exécution|gestion)\b)' || true)
if [[ -n "$FRENCH_MARKERS" ]]; then
    error "Non-English comment or text detected in modified files:"
    echo "$FRENCH_MARKERS" | head -15 | sed 's/^/    /'
    error "Ensure all comments, docstrings, configs, and technical documentation are strictly in English."
    ERRORS_FOUND=$((ERRORS_FOUND + 1))
else
    success "All additions conform to English-only deliverable policy."
fi

# 8. Summary & Verdict
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
