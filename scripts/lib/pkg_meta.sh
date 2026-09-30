#!/usr/bin/env bash
# =============================================================================
# scripts/lib/pkg_meta.sh — Package metadata derived from the Cargo workspace
# =============================================================================
# Single source of truth for the package version and license (GitHub #210):
# the [workspace.package] table of the root Cargo.toml. Read statically (no
# cargo invocation, no network, no Python), so it works in --dry-run and on
# hosts that only package prebuilt artifacts.
#
# Sourced:   source scripts/lib/pkg_meta.sh; soos_pkg_version; soos_pkg_license
# Executed:  bash scripts/lib/pkg_meta.sh [--manifest <Cargo.toml>] <version|license>
#
# Every function fails (non-zero, message on stderr) when the manifest, the
# [workspace.package] table or the key is missing: a package is never built
# with a guessed version or license.
# =============================================================================

SOOS_PKG_META_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOOS_PKG_MANIFEST_DEFAULT="$(cd "${SOOS_PKG_META_DIR}/../.." && pwd)/Cargo.toml"

# soos_workspace_package_value <key> [manifest]
# Prints the string value of <key> in [workspace.package]; other tables are ignored.
soos_workspace_package_value() {
    local key="$1"
    local manifest="${2:-${SOOS_PKG_MANIFEST_DEFAULT}}"
    local value
    if [[ ! -f "${manifest}" ]]; then
        echo "pkg_meta: manifest not found: ${manifest}" >&2
        return 1
    fi
    value="$(awk -v key="${key}" '
        /^[[:space:]]*\[/ {
            line = $0
            gsub(/[[:space:]]/, "", line)
            in_table = (line == "[workspace.package]")
            next
        }
        in_table {
            split_at = index($0, "=")
            if (split_at == 0) next
            k = substr($0, 1, split_at - 1)
            gsub(/[[:space:]]/, "", k)
            if (k != key) next
            v = substr($0, split_at + 1)
            sub(/^[[:space:]]*"/, "", v)
            sub(/"[[:space:]]*(#.*)?$/, "", v)
            print v
            exit
        }
    ' "${manifest}")"
    if [[ -z "${value}" ]]; then
        echo "pkg_meta: [workspace.package] ${key} not found in ${manifest}" >&2
        return 1
    fi
    # Package fields accept a conservative character set only.
    if [[ ! "${value}" =~ ^[A-Za-z0-9.+_\ -]+$ ]]; then
        echo "pkg_meta: unexpected characters in ${key}: ${value}" >&2
        return 1
    fi
    printf '%s\n' "${value}"
}

# soos_pkg_version [manifest]
soos_pkg_version() {
    soos_workspace_package_value version "${1:-}"
}

# soos_pkg_license [manifest]  (SPDX expression, e.g. AGPL-3.0-or-later)
soos_pkg_license() {
    soos_workspace_package_value license "${1:-}"
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    set -euo pipefail
    manifest=""
    if [[ "${1:-}" == "--manifest" ]]; then
        manifest="${2:?--manifest needs a path}"
        shift 2
    fi
    case "${1:-}" in
        version) soos_pkg_version "${manifest}" ;;
        license) soos_pkg_license "${manifest}" ;;
        *)
            echo "Usage: $(basename "$0") [--manifest <Cargo.toml>] <version|license>" >&2
            exit 2
            ;;
    esac
fi
