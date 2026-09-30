#!/usr/bin/env bash
# =============================================================================
# scripts/commit_message.sh — Conventional Commit subject inference (GitHub #245)
# =============================================================================
# Sourced by save.sh when no -m message is given. Defines:
#
#   soos_infer_commit_subject BRANCH < staged-file-list
#
# It prints exactly one subject line (no body, no file counter) or fails with a
# message asking for an explicit -m:
#   - type        : the branch prefix, only feat/, fix/, test/ or chore/ (the
#                   prefixes allowed by AGENTS.md); anything else is refused
#                   instead of guessing a misleading type;
#   - scope       : the crate directory when every staged crate file belongs to
#                   one crate (crates/<dir>/...), otherwise no scope;
#   - description : the branch suffix, lowercased, every character outside
#                   [a-z0-9] turned into a single space;
#   - length      : a subject longer than 72 characters (the PR title limit) is
#                   refused, never truncated.
# =============================================================================

soos_infer_commit_subject() {
    local branch="${1:-}"
    local hint="pass an explicit Conventional Commit message with -m \"<type>(<scope>): <description>\""

    if [[ "$branch" != */* ]]; then
        echo "Cannot infer a commit type from branch '${branch}': ${hint}." >&2
        return 1
    fi

    local prefix="${branch%%/*}"
    local suffix="${branch#*/}"
    case "$prefix" in
        feat | fix | test | chore) ;;
        *)
            echo "Branch prefix '${prefix}/' is not feat/, fix/, test/ or chore/: ${hint}." >&2
            return 1
            ;;
    esac

    local description
    description=$(printf '%s' "$suffix" \
        | LC_ALL=C tr '[:upper:]' '[:lower:]' \
        | LC_ALL=C tr -c 'a-z0-9' ' ' \
        | LC_ALL=C tr -s ' ' \
        | sed -e 's/^ //' -e 's/ $//')
    if [[ -z "$description" ]]; then
        echo "Branch '${branch}' has no usable description: ${hint}." >&2
        return 1
    fi

    local scope="" crate="" file
    local multiple=false
    while IFS= read -r file; do
        case "$file" in
            crates/*/*)
                crate="${file#crates/}"
                crate="${crate%%/*}"
                if [[ -z "$scope" ]]; then
                    scope="$crate"
                elif [[ "$scope" != "$crate" ]]; then
                    multiple=true
                fi
                ;;
        esac
    done
    if [[ "$multiple" == true || ! "$scope" =~ ^[a-z0-9-]+$ ]]; then
        scope=""
    fi

    local subject
    if [[ -n "$scope" ]]; then
        subject="${prefix}(${scope}): ${description}"
    else
        subject="${prefix}: ${description}"
    fi

    if (( ${#subject} > 72 )); then
        echo "Inferred subject is ${#subject} characters (limit 72): ${hint}." >&2
        return 1
    fi

    printf '%s\n' "$subject"
}
