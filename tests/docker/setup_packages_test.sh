#!/usr/bin/env bash
# =============================================================================
# tests/docker/setup_packages_test.sh — every package name of scripts/setup.sh
# exists, and every tool of a group resolves once the group is installed
# (GitHub #362, owner decisions of 2026-10-23, matrix SDP22)
# =============================================================================
# scripts/check_build_deps.sh is the only source of package names (groups
# build gui models companion android). The owner's from-scratch run failed on
# packages discovered missing in the middle of the run; setup.sh now installs
# every requested group in its package step and checks the REQUIRED_TOOLS of
# that group before the build. This test proves the names and the tool table
# against the real repositories, in digest-pinned images:
#   ubuntu:24.04, fedora:40 (the images of the other Docker jobs), fedora:44
#   (current Fedora, m7 of the spec) and archlinux.
#
# Per image, with the workspace mounted read-only and nothing written on the
# host:
#   1. every package of every group resolves in the distribution's own
#      repositories: apt-cache show (debian), dnf repoquery --available
#      (fedora), pacman -Si (arch);
#   2. the companion and android groups are installed with the package manager
#      setup.sh uses, then every REQUIRED_TOOLS command of that group (parsed
#      from scripts/setup.sh) must resolve with command -v.
#
# Known gap (ADR 2026-10-23 "One Command Without Manual Steps", part 1): no JDK
# package resolves on both fedora:40 and fedora:44, so the Fedora android group
# has no JDK and `java` is expected to be missing there until PR C decides how
# Fedora gets its JDK. The gap is asserted, so it cannot be closed silently.
#
# Usage:
#   bash tests/docker/setup_packages_test.sh                      # host: runs Docker
#   bash tests/docker/setup_packages_test.sh --in-container <family>
#   bash tests/docker/setup_packages_test.sh --help
# Environment: SOOS_DOCKER (docker binary).
# =============================================================================

set -euo pipefail
export LC_ALL=C

readonly IMAGES=(
    "debian ubuntu:24.04@sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3"
    "fedora fedora:40@sha256:3c86d25fef9d2001712bc3d9b091fc40cf04be4767e48f1aa3b785bf58d300ed"
    "fedora fedora:44@sha256:43b29f65a41eb9c35e1cd5323e3bdf3b655c2357a9f4f1ff2f9c2798e5045d80"
    "arch archlinux:latest@sha256:b21322c663be387c0ed9cbc7bbbfe18e41633ad4e7b7c77cfad45f128be20040"
)
readonly ALL_GROUPS="build gui models companion android"
readonly INSTALLED_GROUPS="companion android"
# <family>:<group>:<tool> expected to stay missing (asserted).
readonly KNOWN_GAPS=("fedora:android:java")
readonly RUN_TIMEOUT_S=2400

info() { echo "[INFO]  $*"; }
ok()   { echo "[OK]    $*"; }
fail() { echo "[FAIL]  $*" >&2; exit 1; }

usage() {
    sed -n '2,36p' "$0"
}

# -----------------------------------------------------------------------------
# Host mode
# -----------------------------------------------------------------------------
host_main() {
    local docker repo entry family image
    docker="${SOOS_DOCKER:-docker}"
    command -v "${docker}" >/dev/null 2>&1 || fail "docker is required"
    "${docker}" info >/dev/null 2>&1 || fail "the Docker daemon is not accessible"
    repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
    for entry in "${IMAGES[@]}"; do
        family="${entry%% *}"
        image="${entry#* }"
        info "${image} (${family}): package names and REQUIRED_TOOLS"
        timeout "${RUN_TIMEOUT_S}" "${docker}" run --rm \
            -v "${repo}:/workspace:ro" \
            "${image}" bash /workspace/tests/docker/setup_packages_test.sh --in-container "${family}" \
            || fail "${image}: see the lines above"
        ok "${image}"
    done
    ok "every package name resolves and every group tool is found"
}

# -----------------------------------------------------------------------------
# Container mode
# -----------------------------------------------------------------------------
refresh_index() {
    case "$1" in
        debian) DEBIAN_FRONTEND=noninteractive apt-get update -qq >/dev/null ;;
        fedora) dnf -q makecache >/dev/null ;;
        arch) pacman -Sy --noconfirm >/dev/null ;;
    esac
}

package_exists() {
    local family="$1" pkg="$2"
    case "${family}" in
        debian) apt-cache show -- "${pkg}" >/dev/null 2>&1 ;;
        fedora) [[ -n "$(dnf -q repoquery --available -- "${pkg}" 2>/dev/null)" ]] ;;
        arch) pacman -Si -- "${pkg}" >/dev/null 2>&1 ;;
    esac
}

install_packages() {
    local family="$1"
    shift
    case "${family}" in
        debian) DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends "$@" >/dev/null ;;
        fedora) dnf install -y -q "$@" >/dev/null ;;
        arch) pacman -S --needed --noconfirm "$@" >/dev/null ;;
    esac
}

# `tool scope` lines of the REQUIRED_TOOLS table of setup.sh.
required_tools() {
    sed -n '/^declare -rA REQUIRED_TOOLS=($/,/^)$/p' /workspace/scripts/setup.sh \
        | tr ' ' '\n' \
        | sed -n 's/^\[\([A-Za-z0-9_.-]*\)\]=\([a-z:-]*\)$/\1 \2/p'
}

is_known_gap() {
    local key="$1" gap
    for gap in "${KNOWN_GAPS[@]}"; do
        [[ "${gap}" == "${key}" ]] && return 0
    done
    return 1
}

container_main() {
    local family="$1" group list pkg tool scope missing=0 found_tools=0
    local -a pkgs=()
    case "${family}" in
        debian|fedora|arch) ;;
        *) fail "unknown family: ${family}" ;;
    esac
    refresh_index "${family}"
    for group in ${ALL_GROUPS}; do
        list="$(bash /workspace/scripts/check_build_deps.sh --distro "${family}" --print-packages "${group}")" \
            || fail "check_build_deps.sh has no ${group} group for ${family}"
        read -r -a pkgs <<< "${list}"
        [[ ${#pkgs[@]} -gt 0 ]] || fail "${family}/${group}: empty package list"
        for pkg in "${pkgs[@]}"; do
            if package_exists "${family}" "${pkg}"; then
                echo "[OK]    ${family}/${group}: ${pkg}"
            else
                echo "[FAIL]  ${family}/${group}: ${pkg} does not resolve" >&2
                missing=$((missing + 1))
            fi
        done
    done
    [[ "${missing}" -eq 0 ]] || fail "${missing} package name(s) do not resolve for ${family}"

    local -a table=()
    mapfile -t table < <(required_tools)
    [[ ${#table[@]} -gt 20 ]] || fail "could not parse REQUIRED_TOOLS from scripts/setup.sh"
    for group in ${INSTALLED_GROUPS}; do
        list="$(bash /workspace/scripts/check_build_deps.sh --distro "${family}" --print-packages "${group}")"
        read -r -a pkgs <<< "${list}"
        install_packages "${family}" "${pkgs[@]}" || fail "${family}/${group}: install failed"
        for entry in "${table[@]}"; do
            tool="${entry% *}"
            scope="${entry#* }"
            [[ "${scope}" == "${group}" ]] || continue
            found_tools=$((found_tools + 1))
            if is_known_gap "${family}:${group}:${tool}"; then
                if command -v "${tool}" >/dev/null 2>&1; then
                    fail "${family}/${group}: ${tool} now resolves; remove the known gap and record the JDK decision"
                fi
                echo "[GAP]   ${family}/${group}: ${tool} (known gap, ADR 2026-10-23 part 1)"
            elif command -v "${tool}" >/dev/null 2>&1; then
                echo "[OK]    ${family}/${group}: ${tool} -> $(command -v "${tool}")"
            else
                fail "${family}/${group}: ${tool} not found after installing ${list}"
            fi
        done
    done
    [[ "${found_tools}" -ge 5 ]] || fail "too few group tools checked (${found_tools})"
}

case "${1:-}" in
    --in-container)
        [[ $# -eq 2 ]] || fail "--in-container takes one family"
        container_main "$2"
        ;;
    -h|--help) usage ;;
    "") host_main ;;
    *) fail "unknown option: $1 (use --help)" ;;
esac
