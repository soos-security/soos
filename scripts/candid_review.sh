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

# Every audit diffs against the merge base, never against the moving tip of BASE_REF: once
# main advances past the branch point, a two-dot diff would contain reversed upstream hunks.
MERGE_BASE=$(git merge-base "$BASE_REF" HEAD 2>/dev/null || echo "$BASE_REF")

# Determine diff targets
CHANGED_FILES=$(git diff --name-only "$MERGE_BASE" HEAD 2>/dev/null || true)
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
RAW_DIFF=$(git diff "$MERGE_BASE" 2>/dev/null || git diff HEAD 2>/dev/null || true)

# Rust production-code filter for the PAM audits (mirrors `production_code` in
# tests/invariants/src/lexing_contract.rs). A small character lexer, whose state carries
# across lines, removes line comments and nested, multi-line block comments and blanks the
# contents of string, byte-string, raw strings (`r"..."`, `r#"..."#`, `br#"..."#`) and
# char literals (lifetimes are kept); one output line is printed per input line. It then
# drops exactly the item gated by `#[cfg(test)]` / `#[cfg(all(test, ...))]`: up to its `;`
# (`mod tests;`, `use ...;`) or up to the `}` matching its body (`mod tests { ... }`, `fn`,
# `impl`). Braces inside comments and literals never shift the item boundary (GitHub #285).
RUST_PRODUCTION_FILTER=$(cat <<'AWK'
function is_ident(c) {
    return c ~ /^[A-Za-z0-9_]$/
}
# Lexer state persists across lines: blk (block comment depth), in_str (inside "..."),
# in_raw (inside a raw string closed by `"` followed by the hashes in rawh).
function sanitize(s,    out, i, n, c, nx, j, h, closer, before) {
    out = ""
    n = length(s)
    i = 1
    while (i <= n) {
        c = substr(s, i, 1)
        nx = substr(s, i + 1, 1)
        if (blk > 0) {
            if (c == "*" && nx == "/") { blk--; i += 2; continue }
            if (c == "/" && nx == "*") { blk++; i += 2; continue }
            i++
            continue
        }
        if (in_raw) {
            closer = "\"" rawh
            if (substr(s, i, length(closer)) == closer) {
                out = out closer
                in_raw = 0
                i += length(closer)
                continue
            }
            i++
            continue
        }
        if (in_str) {
            if (c == "\\") { i += 2; continue }
            if (c == "\"") { out = out c; in_str = 0 }
            i++
            continue
        }
        if (c == "/" && nx == "/") break
        if (c == "/" && nx == "*") { blk = 1; out = out " "; i += 2; continue }
        if (c == "\"") { out = out c; in_str = 1; i++; continue }
        if (c == "r") {
            # Raw string prefix: `r` (or `br`) not preceded by another identifier character.
            before = (i > 1) ? substr(s, i - 1, 1) : ""
            if (before == "b" && (i == 2 || !is_ident(substr(s, i - 2, 1)))) before = ""
            if (!is_ident(before)) {
                j = i + 1
                h = ""
                while (substr(s, j, 1) == "#") { h = h "#"; j++ }
                if (substr(s, j, 1) == "\"") {
                    out = out "r" h "\""
                    rawh = h
                    in_raw = 1
                    i = j + 1
                    continue
                }
            }
        }
        if (c == "'") {
            if (nx == "\\") {
                # Escaped char literal: skip the escaped character, then find the closing quote.
                j = index(substr(s, i + 3), "'")
                if (j > 0) { out = out "''"; i = i + 3 + j; continue }
            } else if (nx != "" && substr(s, i + 2, 1) == "'") {
                out = out "''"
                i += 3
                continue
            }
        }
        out = out c
        i++
    }
    return out
}
function skip_item(text,    i, n, ch) {
    n = length(text)
    for (i = 1; i <= n; i++) {
        ch = substr(text, i, 1)
        if (in_body) {
            if (ch == "{") depth++
            else if (ch == "}") { depth--; if (depth == 0) { state = 0; return } }
        } else if (ch == "[") square++
        else if (ch == "]") { if (square > 0) square-- }
        else if (ch == "(") paren++
        else if (ch == ")") { if (paren > 0) paren-- }
        else if (ch == "{" && square == 0 && paren == 0) { in_body = 1; depth = 1 }
        else if (ch == ";" && square == 0 && paren == 0) { state = 0; return }
    }
}
{
    code = sanitize($0)
    if (state == 0) {
        if (code ~ /^[ \t]*#\[[ \t]*cfg[ \t]*\([ \t]*(test[ \t]*\)|all[ \t]*\([ \t]*test[ \t]*,)/) {
            state = 1; in_body = 0; depth = 0; square = 0; paren = 0
            skip_item(code)
            next
        }
        print code
        next
    }
    skip_item(code)
}
AWK
)

# Prints the PAM production lines (comment- and test-free, see RUST_PRODUCTION_FILTER)
# added since the merge base that match the ERE "$1", as "<file>: <code>".
pam_production_additions() {
    local pattern="$1" file base_code head_code
    while IFS= read -r file; do
        [[ "$file" == *.rs ]] || continue
        base_code="$(git show "${MERGE_BASE}:${file}" 2>/dev/null | awk "$RUST_PRODUCTION_FILTER" || true)"
        head_code=""
        if [[ -f "$file" ]]; then
            head_code="$(awk "$RUST_PRODUCTION_FILTER" "$file" || true)"
        fi
        diff <(printf '%s\n' "$base_code") <(printf '%s\n' "$head_code") 2>/dev/null \
            | grep -E '^>' | grep -E "$pattern" | sed "s|^> |${file}: |" || true
    done < <(git diff --name-only "$MERGE_BASE" -- 'crates/pam/src' 2>/dev/null || true)
}

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
FORBIDDEN_UNSAFE=$(git diff "$MERGE_BASE" -- "${BUSINESS_PATHS[@]}" 2>/dev/null | grep -E '^\+[^+].*\bunsafe\b' | grep -vE '^\+\s*//' || true)
# A removed attribute only counts if the file no longer declares it (moves/reformatting are fine).
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
# Production lines only: #[cfg(test)] items are filtered out lexically (no `grep -v tests`).
PAM_PANICS=$(pam_production_additions '(\.unwrap\(|\.expect\(|panic!|panic_any\(|todo!|unimplemented!|unreachable!)')
if [[ -n "$PAM_PANICS" ]]; then
    error "Forbidden panic, unwrap, or unfinished stub added in PAM production code!"
    echo "$PAM_PANICS" | sed 's/^/    /'
    ERRORS_FOUND=$((ERRORS_FOUND + 1))
else
    success "Zero unwrap(), expect(), panic!(), or unfinished stubs in PAM production pathways."
fi

# 3. Check for async/Tokio in PAM crate
step "Audit 3: Checking Asynchronous Runtime Invariant"
PAM_TOKIO=$(git diff "$MERGE_BASE" -- crates/pam/Cargo.toml 2>/dev/null | grep -E '^\+[^+].*\b(tokio|async-std|smol|async-io)\b' || true)
if [[ -n "$PAM_TOKIO" ]]; then
    error "Forbidden asynchronous runtime dependency added to crates/pam/Cargo.toml!"
    echo "$PAM_TOKIO" | sed 's/^/    /'
    ERRORS_FOUND=$((ERRORS_FOUND + 1))
else
    success "Zero asynchronous runtime dependencies in crates/pam."
fi

# 4. Check for forbidden OpenCV and Nokhwa dependencies
step "Audit 4: Checking Forbidden Third-Party Dependencies (OpenCV & Nokhwa)"
FORBIDDEN_DEPS=$(git diff "$MERGE_BASE" -- '**/Cargo.toml' 'Cargo.toml' 2>/dev/null | grep -E '^\+[^+].*(opencv|nokhwa)' || true)
if [[ -n "$FORBIDDEN_DEPS" ]]; then
    error "Forbidden OpenCV or Nokhwa dependency detected in Cargo.toml!"
    echo "$FORBIDDEN_DEPS" | sed 's/^/    /'
    ERRORS_FOUND=$((ERRORS_FOUND + 1))
else
    success "Zero OpenCV or prohibited camera dependencies detected."
fi

# 5. Shell script syntax validation
step "Audit 5: Validating Shell Scripts Syntax"
SHELL_FILES=$(git diff --name-only "$MERGE_BASE" 2>/dev/null | grep -E '(\.sh$|^\.githooks/)' || true)
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
PAM_PRINTS=$(pam_production_additions '(println!|eprintln!|print!|eprint!|dbg!)')
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
# 'pour' and 'attention' are also English words ("pour the buffer", "Attention: ...") and are
# deliberately not markers.
FRENCH_MARKERS=$(git diff "$MERGE_BASE" -- '*.rs' '*.md' '*.sh' '*.yml' '*.yaml' '*.toml' 'Dockerfile' '.gitignore' ':!scripts/candid_review.sh' 2>/dev/null | grep -E '^\+[^+]*(//|/\*|#|<!--|").*(\b(avec|dans|faire|ajouter|vérifier|fonction|problème|étape|fichier|modifié|remarque|défaut|sécurité|exécution|gestion)\b)' || true)
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
