#!/usr/bin/env bash
# =============================================================================
# scripts/secret_scan.sh — Secret & Sensitive Artifact Scanner for soos
# =============================================================================
# Scans ADDED lines and ADDED/MODIFIED file names for:
#   - Private keys and certificates (PEM blocks, key/cert file extensions)
#   - Access tokens (GitHub, AWS, Slack, OpenAI/Anthropic-style API keys)
#   - Biometric and evidence artifacts (*.enc, master.key, evidence.key)
#   - ONNX model weights (must be downloaded and attested, never committed)
#
# Usage:
#   ./scripts/secret_scan.sh --staged              Scan the staged index (pre-commit)
#   ./scripts/secret_scan.sh --range <A>..<B>      Scan EVERY commit in A..B (CI)
#   ./scripts/secret_scan.sh --unpushed <rev>      Scan every commit reachable from
#                                                  <rev> but not from any remote (pre-push)
#   ./scripts/secret_scan.sh --history <rev>       Scan every commit reachable from <rev>
#
# Range modes inspect each commit individually, so a secret added in one commit
# and deleted in a later one is still detected (it would remain in history).
# Merge commits are scanned against their first parent (conflict resolutions).
# File names are always treated as literal paths, never as pathspec magic.
# Binary files are content-scanned too (--text).
#
# Exit status: 0 when clean, 1 when a finding is detected, 2 on usage or git error
# (fail-closed: an unresolvable range never passes).
# Findings print the file and pattern name only — never the matched secret.
# =============================================================================

set -euo pipefail

export GIT_LITERAL_PATHSPECS=1
# Byte-oriented matching: in UTF-8 locales GNU grep silently skips lines with
# invalid UTF-8 (binary data, Latin-1 text), which would let secrets through.
export LC_ALL=C

usage() {
    echo "Usage: $0 --staged | --range <A>..<B> | --unpushed <rev> | --history <rev>" >&2
    exit 2
}

MODE=""
COMMITS=()
case "${1:-}" in
    --staged) MODE="staged" ;;
    --range|--unpushed|--history)
        [[ -n "${2:-}" ]] || usage
        MODE="commits"
        if [[ "$1" == "--range" ]]; then
            [[ "$2" == *..* ]] || usage
            from="${2%%..*}"
            to="${2##*..}"
            for rev in "$from" "$to"; do
                if ! git rev-parse --verify --quiet "${rev}^{commit}" >/dev/null; then
                    echo "secret_scan: cannot resolve revision '$rev' (fail-closed)" >&2
                    exit 2
                fi
            done
            rev_list_args=("${from}..${to}")
        else
            if ! git rev-parse --verify --quiet "${2}^{commit}" >/dev/null; then
                echo "secret_scan: cannot resolve revision '$2' (fail-closed)" >&2
                exit 2
            fi
            if [[ "$1" == "--unpushed" ]]; then
                rev_list_args=("$2" --not --remotes)
            else
                rev_list_args=("$2")
            fi
        fi
        if ! commit_list="$(git rev-list "${rev_list_args[@]}")"; then
            echo "secret_scan: git rev-list failed (fail-closed)" >&2
            exit 2
        fi
        if [[ -n "$commit_list" ]]; then
            mapfile -t COMMITS <<< "$commit_list"
        fi
        ;;
    *) usage ;;
esac

# name|extended-regex
readonly CONTENT_PATTERNS=(
    "PEM private key|-----BEGIN ([A-Z]+ )*PRIVATE KEY( BLOCK)?-----"
    "GitHub token|(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36}"
    "GitHub fine-grained token|github_pat_[A-Za-z0-9_]{82}"
    "AWS access key id|(AKIA|ASIA)[0-9A-Z]{16}"
    "Slack token|xox[abprs]-[A-Za-z0-9-]{10,}"
    "Generic API secret key|(^|[^A-Za-z0-9_-])sk-(ant-|proj-)?[A-Za-z0-9_-]{32,}"
)

FINDINGS=0
NAMES_FILE="$(mktemp)"
trap 'rm -f "$NAMES_FILE"' EXIT

report() {
    echo -e "\033[0;31m[BLOCKED]\033[0m $1: $2" >&2
    FINDINGS=$((FINDINGS + 1))
}

is_merge() {
    git rev-parse --verify --quiet "${1}^2" >/dev/null
}

# $1 = "staged" or a commit sha. Writes NUL-separated names to NAMES_FILE.
list_names() {
    if [[ "$1" == "staged" ]]; then
        git diff --cached --name-only --diff-filter=ACMRT -z > "$NAMES_FILE"
    elif is_merge "$1"; then
        git diff --name-only --diff-filter=ACMRT -z "${1}^1" "$1" > "$NAMES_FILE"
    else
        git diff-tree --root --no-commit-id -r --name-only --diff-filter=ACMRT -z "$1" > "$NAMES_FILE"
    fi
}

# Prints the added lines of one file; any git failure aborts (fail-closed).
added_lines() {
    local target="$1" file="$2" patch
    if [[ "$target" == "staged" ]]; then
        patch="$(git diff --cached --text --no-color --no-ext-diff --no-textconv -U0 -- "$file")" || return 1
    elif is_merge "$target"; then
        patch="$(git diff --text --no-color --no-ext-diff --no-textconv -U0 "${target}^1" "$target" -- "$file")" || return 1
    else
        patch="$(git diff-tree --root --no-commit-id -p --text --no-color --no-ext-diff --no-textconv -U0 "$target" -- "$file")" || return 1
    fi
    # Skip the per-file header ("+++ b/path") that precedes the first hunk; inside
    # hunks every '+' line is content, even one that starts with "++ ".
    awk '/^@@/ { in_hunk = 1; next } in_hunk && /^\+/ { print }' <<< "$patch"
}

scan_target() {
    local target="$1" label file base lower added entry name regex
    label="$target"
    [[ "$target" != "staged" ]] && label="commit ${target:0:12}"
    if ! list_names "$target"; then
        echo "secret_scan: git failed while listing changes of $label (fail-closed)" >&2
        exit 2
    fi
    while IFS= read -r -d '' file; do
        base="$(basename "$file")"
        lower="${base,,}"
        case "$lower" in
            *.enc|master.key|evidence.key)
                report "Encrypted biometric/evidence artifact or master key (must never be committed)" "$file ($label)" ;;
            *.pem|*.key|*.p12|*.pfx|*.jks|*.secret|id_rsa*|id_ecdsa*|id_ed25519*)
                report "Sensitive key or certificate file" "$file ($label)" ;;
            *.onnx|*.ort)
                report "Model weights (download and attest via scripts/download_models.sh instead)" "$file ($label)" ;;
            .env|.env.*)
                report "Environment file" "$file ($label)" ;;
        esac

        if ! added="$(added_lines "$target" "$file")"; then
            echo "secret_scan: git failed while reading $file ($label) (fail-closed)" >&2
            exit 2
        fi
        [[ -z "$added" ]] && continue
        for entry in "${CONTENT_PATTERNS[@]}"; do
            name="${entry%%|*}"
            regex="${entry#*|}"
            if grep -qE -- "$regex" <<< "$added"; then
                report "$name pattern detected in added lines" "$file ($label)"
            fi
        done
    done < "$NAMES_FILE"
}

if [[ "$MODE" == "staged" ]]; then
    scan_target staged
else
    for commit in "${COMMITS[@]}"; do
        scan_target "$commit"
    done
fi

if [[ $FINDINGS -gt 0 ]]; then
    echo "secret_scan: $FINDINGS finding(s). Remove the secret, rotate it if it was ever pushed," >&2
    echo "             and keep keys/biometric data out of the repository (rewrite history" >&2
    echo "             if the secret exists in an intermediate commit)." >&2
    exit 1
fi

exit 0
