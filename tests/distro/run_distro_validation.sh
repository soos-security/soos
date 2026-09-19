#!/usr/bin/env bash
# =============================================================================
# tests/distro/run_distro_validation.sh — Multi-Distro Deployment Validation Driver
# =============================================================================
# Unified execution harness for Issue #32:
#   - #32.1: Debian 12 / Ubuntu 24.04 Deployment Validation
#   - #32.2: Fedora 40 / RHEL 9 Deployment Validation
#   - #32.3: Arch Linux Deployment Validation
#
# Usage:
#   bash tests/distro/run_distro_validation.sh [OPTIONS] [DISTRO]
#
# Arguments:
#   DISTRO                   ubuntu, fedora, arch, all, or auto (default: auto)
#
# Options:
#   --dry-run                Simulate execution plan without modifying filesystem
#   --skip-docker            Run local native script directly without Docker container
#   --skip-build             Skip binary compilation step
#   -h, --help               Display this help message and exit
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

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

TARGET_DISTRO="auto"
DRY_RUN=false
SKIP_DOCKER=false
SKIP_BUILD=false

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS] [DISTRO]

Orchestrates distribution-specific deployment validation across Debian/Ubuntu,
Fedora/RHEL, and Arch Linux environments.

Distro Targets:
  auto                     Auto-detect current host distribution (default)
  ubuntu                   Run Debian 12 / Ubuntu 24.04 deployment test (#32.1)
  fedora                   Run Fedora 40 / RHEL 9 deployment test (#32.2)
  arch                     Run Arch Linux deployment test (#32.3)
  all                      Execute all distribution validation test suites

Options:
  --dry-run                Simulate test execution plan without system changes
  --skip-docker            Execute native distro scripts directly (no Docker container)
  --skip-build             Skip cargo build if release binaries are present
  -h, --help               Display this help message and exit
EOF
}

# Parse options and arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --skip-docker)
            SKIP_DOCKER=true
            shift
            ;;
        --skip-build)
            SKIP_BUILD=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        ubuntu|debian)
            TARGET_DISTRO="ubuntu"
            shift
            ;;
        fedora|rhel)
            TARGET_DISTRO="fedora"
            shift
            ;;
        arch)
            TARGET_DISTRO="arch"
            shift
            ;;
        all)
            TARGET_DISTRO="all"
            shift
            ;;
        auto)
            TARGET_DISTRO="auto"
            shift
            ;;
        *)
            error "Unknown argument: $1"
            usage
            exit 1
            ;;
    esac
done

# Resolve auto-detected distro
if [[ "${TARGET_DISTRO}" = "auto" ]]; then
    if [[ -f "/etc/os-release" ]]; then
        # shellcheck disable=SC1091
        source /etc/os-release
        case "${ID:-}" in
            ubuntu|debian)
                TARGET_DISTRO="ubuntu"
                ;;
            fedora|rhel|centos)
                TARGET_DISTRO="fedora"
                ;;
            arch)
                TARGET_DISTRO="arch"
                ;;
            *)
                info "Unknown distribution '${ID:-}'. Defaulting to 'all'."
                TARGET_DISTRO="all"
                ;;
        esac
    else
        TARGET_DISTRO="all"
    fi
fi

echo ""
info "==================================================================="
info "  SOOS — Distribution Deployment Validation Orchestrator"
info "==================================================================="
info "Target Distribution: ${TARGET_DISTRO}"
info "Dry Run:             ${DRY_RUN}"
info "Skip Docker:         ${SKIP_DOCKER}"
echo ""

run_local_test() {
    local distro="$1"
    local flags=()
    if [[ "${DRY_RUN}" = true ]]; then
        flags+=(--dry-run)
    fi
    if [[ "${SKIP_BUILD}" = true ]]; then
        flags+=(--skip-build)
    fi

    case "${distro}" in
        ubuntu)
            bash "${SCRIPT_DIR}/debian_ubuntu_test.sh" "${flags[@]}"
            ;;
        fedora)
            bash "${SCRIPT_DIR}/fedora_rhel_test.sh" "${flags[@]}"
            ;;
        arch)
            bash "${SCRIPT_DIR}/arch_linux_test.sh" "${flags[@]}"
            ;;
    esac
}

run_docker_distro() {
    local distro="$1"
    local dockerfile="tests/docker/Dockerfile.${distro}"
    local tag="soos-distro-val-${distro}"

    if ! command -v docker >/dev/null 2>&1; then
        warn "Docker not available; falling back to dry-run verification..."
        run_local_test "${distro}"
        return 0
    fi

    if [[ ! -f "${WORKSPACE_ROOT}/${dockerfile}" ]]; then
        error "Dockerfile not found: ${dockerfile}"
        return 1
    fi

    info "Building container test image '${tag}'..."
    docker build -f "${WORKSPACE_ROOT}/${dockerfile}" -t "${tag}" "${WORKSPACE_ROOT}"

    info "Executing distribution deployment validation inside '${tag}'..."
    docker run --rm \
        -v "${WORKSPACE_ROOT}":/workspace \
        "${tag}" \
        bash "/workspace/tests/distro/${distro}_test.sh"
}

case "${TARGET_DISTRO}" in
    ubuntu)
        if [[ "${SKIP_DOCKER}" = true || "${DRY_RUN}" = true ]]; then
            run_local_test "ubuntu"
        else
            run_docker_distro "ubuntu"
        fi
        ;;
    fedora)
        if [[ "${SKIP_DOCKER}" = true || "${DRY_RUN}" = true ]]; then
            run_local_test "fedora"
        else
            run_docker_distro "fedora"
        fi
        ;;
    arch)
        if [[ "${SKIP_DOCKER}" = true || "${DRY_RUN}" = true ]]; then
            run_local_test "arch"
        else
            run_docker_distro "arch"
        fi
        ;;
    all)
        info "Executing full multi-distribution validation suite..."
        for d in ubuntu fedora arch; do
            if [[ "${SKIP_DOCKER}" = true || "${DRY_RUN}" = true ]]; then
                run_local_test "${d}"
            else
                run_docker_distro "${d}"
            fi
        done
        ;;
esac

echo ""
success "==================================================================="
success "  DISTRIBUTION DEPLOYMENT VALIDATION COMPLETED SUCCESSFULLY!      "
success "==================================================================="
exit 0
