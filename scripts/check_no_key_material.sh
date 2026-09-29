#!/usr/bin/env bash
# =============================================================================
# scripts/check_no_key_material.sh — Packaging guard against shipped keys
# =============================================================================
# Fails closed when a staged package tree contains any key file. Package
# builders (scripts/build_deb.sh, scripts/build_arch.sh, packaging/debian/rules)
# run it on the staging root before archiving so that a master key can never be
# baked into a distributable artifact (GitHub #144 / ONB-01).
#
# Usage:
#   scripts/check_no_key_material.sh <STAGED_TREE>
#
# Exit codes:
#   0  no key material found
#   1  key material found, or the tree is missing / unreadable
# =============================================================================

set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $(basename "$0") <STAGED_TREE>" >&2
    exit 1
fi

STAGE_DIR="${1%/}"

if [[ -z "${STAGE_DIR}" || ! -d "${STAGE_DIR}" ]]; then
    echo "check_no_key_material: staged tree '${1}' does not exist or is not a directory" >&2
    exit 1
fi

# Any regular file (or symlink) whose name ends in .key, anywhere in the tree.
# The result is captured in a variable on purpose: a `find | grep -q` pipeline
# would be short-circuited by SIGPIPE under `set -o pipefail` and could report
# a false negative.
STAGED_KEYS=$(find "${STAGE_DIR}" \( -type f -o -type l \) -name '*.key' -print)

if [[ -n "${STAGED_KEYS}" ]]; then
    echo "check_no_key_material: refusing to package '${STAGE_DIR}': key material found:" >&2
    echo "${STAGED_KEYS}" | sed 's/^/  - /' >&2
    echo "Keys are generated on the target host by /usr/libexec/soos/provision-master-key." >&2
    exit 1
fi

echo "check_no_key_material: no key material in '${STAGE_DIR}'."
exit 0
