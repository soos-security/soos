#!/usr/bin/env bash
# =============================================================================
# scripts/pr_checks_gate.sh — Merge decision over the checks of a pull request
# =============================================================================
# Used by scripts/pr_loop.sh (GitHub #367, ADR 2026-10-13 "macOS Port in a
# Separate Repository"). `main` has no branch protection, so pr_loop.sh is the
# only merge gate, and 'CI Success' alone is NOT sufficient: checks of other
# workflows (for example "PR Title (Conventional Commits)" of pr-title.yml) are
# outside it. This helper decides over every check of the PR, except the
# advisory (non-required) checks listed below, whose state is only reported.
#
# Usage: scripts/pr_checks_gate.sh <pr-number>
#
# Exit codes (fail-closed: every unknown state blocks):
#   0  proceed: at least one non-advisory check, all of them pass or skipping
#   1  block:   a non-advisory check failed or was cancelled, no non-advisory
#               check exists, gh failed, the output is malformed, or the
#               argument is invalid
#   3  pending: no failure, but a non-advisory check is still pending
#
# `gh pr checks --json` exits 0 even when checks failed (gh 2.102.0; the plain
# form exits 1), so its output is parsed rather than its status inferred.
# =============================================================================

set -euo pipefail

# Exactly one advisory (non-required) check, the macOS job of ci.yml.
readonly ADVISORY_CHECKS=("Portable crates (macOS, non-required)")

readonly EXIT_PROCEED=0
readonly EXIT_BLOCK=1
readonly EXIT_PENDING=3

block() {
    echo "[FAIL]  pr_checks_gate: $*" >&2
    exit "$EXIT_BLOCK"
}

PR="${1:-}"
[[ "$PR" =~ ^[0-9]+$ ]] || block "expected a pull request number, got '${PR}'"

# One gh call: the record count, then one "name<TAB>bucket" record per check
# (@tsv escapes tabs and newlines inside names, so a record is one line).
if ! OUTPUT="$(gh pr checks "$PR" --json name,bucket --jq '(length|tostring), (.[] | [.name, .bucket] | @tsv)')"; then
    block "gh pr checks failed for PR #${PR}"
fi

is_advisory() {
    local name="$1" advisory
    for advisory in "${ADVISORY_CHECKS[@]}"; do
        # Quoted literal comparison: check names contain ( ) , $ { }.
        if [[ "$name" == "$advisory" ]]; then
            return 0
        fi
    done
    return 1
}

EXPECTED=""
RECORDS=0
NON_ADVISORY=0
FAILED=0
PENDING=0
TAB=$'\t'

# `|| [[ -n "$line" ]]` keeps a final record that has no trailing newline.
while IFS= read -r line || [[ -n "$line" ]]; do
    if [[ -z "$EXPECTED" ]]; then
        [[ "$line" =~ ^[0-9]+$ ]] || block "malformed check count '${line}'"
        EXPECTED="$line"
        continue
    fi
    RECORDS=$((RECORDS + 1))
    # Exactly one tab: a non-empty name, then the bucket.
    [[ "$line" == *"$TAB"* ]] || block "malformed check record"
    name="${line%%"$TAB"*}"
    bucket="${line#*"$TAB"}"
    [[ -n "$name" ]] || block "check record without a name"
    [[ "$bucket" != *"$TAB"* ]] || block "malformed check record (extra field)"
    case "$bucket" in
        pass | fail | pending | skipping | cancel) ;;
        *) block "unknown state '${bucket}' for check '${name}'" ;;
    esac
    if is_advisory "$name"; then
        echo "[WARN]  advisory check '${name}' is '${bucket}' (non-required, never blocks)" >&2
        continue
    fi
    NON_ADVISORY=$((NON_ADVISORY + 1))
    case "$bucket" in
        fail | cancel) FAILED=$((FAILED + 1)) ;;
        pending) PENDING=$((PENDING + 1)) ;;
        *) ;;
    esac
done <<< "$OUTPUT"

[[ -n "$EXPECTED" ]] || block "empty gh output"
(( RECORDS == EXPECTED )) || block "parsed ${RECORDS} check(s), gh reported ${EXPECTED}"
(( NON_ADVISORY > 0 )) || block "no non-advisory check found"
(( FAILED == 0 )) || block "${FAILED} non-advisory check(s) failed or were cancelled"
if (( PENDING > 0 )); then
    echo "[INFO]  ${PENDING} non-advisory check(s) still pending" >&2
    exit "$EXIT_PENDING"
fi
exit "$EXIT_PROCEED"
