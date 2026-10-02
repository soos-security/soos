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
#   --dry-run                Print each distribution's plan; executes nothing privileged
#   --skip-docker            Run the native distro script on THIS host (requires
#                            --allow-host-changes)
#   --allow-host-changes     Explicit consent for --skip-docker live runs, which install
#                            packages and rewrite PAM files on the current host
#   --skip-build             Skip binary compilation step
#   -h, --help               Display this help message and exit
#
# Live validation runs only inside disposable Docker containers. Without Docker
# the runner fails: it never falls back to a live run on the host (GitHub #163).
# Each container overlays /workspace/target with a per-distribution Docker volume
# (soos-distro-target-<distro>), so artifacts never leak between distributions
# and no root-owned files are written into the host's target/ directory.
# Environment: SOOS_DOCKER overrides the docker binary (default: docker).
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
ALLOW_HOST_CHANGES=false
DOCKER_BIN="${SOOS_DOCKER:-docker}"

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
  --dry-run                Print each distribution's plan; executes nothing privileged
  --skip-docker            Execute the native distro script on THIS host (no container);
                           requires --allow-host-changes
  --allow-host-changes     Consent to install packages and rewrite PAM files on this host
  --skip-build             Skip cargo build and use the existing release binaries
  -h, --help               Display this help message and exit

Without --dry-run and --skip-docker, every distribution runs in a disposable
Docker container. If Docker is unavailable the runner fails instead of falling
back to a live run on the host.
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
        --allow-host-changes)
            ALLOW_HOST_CHANGES=true
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

# Explicit distro -> script mapping (GitHub #163: never interpolate script names).
distro_script() {
    case "$1" in
        ubuntu) echo "debian_ubuntu_test.sh" ;;
        fedora) echo "fedora_rhel_test.sh" ;;
        arch)   echo "arch_linux_test.sh" ;;
        *)
            error "No validation script for distribution '$1'."
            return 1
            ;;
    esac
}

# Dry runs and consented --skip-docker runs execute the native script directly.
run_local_test() {
    local distro="$1"
    local script
    script="$(distro_script "${distro}")"
    local flags=()
    if [[ "${DRY_RUN}" = true ]]; then
        flags+=(--dry-run)
    elif [[ "${ALLOW_HOST_CHANGES}" = true ]]; then
        flags+=(--allow-host-changes)
    else
        error "Refusing to run the live ${distro} validation on this host without --allow-host-changes."
        error "It installs packages and rewrites PAM files. Use Docker (default) or --dry-run."
        exit 2
    fi
    if [[ "${SKIP_BUILD}" = true ]]; then
        flags+=(--skip-build)
    fi

    case "${script}" in
        debian_ubuntu_test.sh)
            bash "${SCRIPT_DIR}/debian_ubuntu_test.sh" "${flags[@]}"
            ;;
        fedora_rhel_test.sh)
            bash "${SCRIPT_DIR}/fedora_rhel_test.sh" "${flags[@]}"
            ;;
        arch_linux_test.sh)
            bash "${SCRIPT_DIR}/arch_linux_test.sh" "${flags[@]}"
            ;;
    esac
}

run_docker_distro() {
    local distro="$1"
    local dockerfile="tests/docker/Dockerfile.${distro}"
    local tag="soos-distro-val-${distro}"
    local script
    script="$(distro_script "${distro}")"

    if ! command -v "${DOCKER_BIN}" >/dev/null 2>&1; then
        error "Docker ('${DOCKER_BIN}') is required for live distribution validation."
        error "Nothing was executed. Use --dry-run to print the plan, or"
        error "--skip-docker --allow-host-changes to run the live suite on this host deliberately."
        exit 1
    fi

    if [[ ! -f "${WORKSPACE_ROOT}/${dockerfile}" ]]; then
        error "Dockerfile not found: ${dockerfile}"
        return 1
    fi

    info "Building container test image '${tag}'..."
    "${DOCKER_BIN}" build -f "${WORKSPACE_ROOT}/${dockerfile}" -t "${tag}" "${WORKSPACE_ROOT}"

    local flags=()
    if [[ "${SKIP_BUILD}" = true ]]; then
        flags+=(--skip-build)
    fi

    # Optional pass-throughs (GitHub #318): the build parallelism of a shared
    # machine, and the host ONNX Runtime download cache that CI restores and saves
    # (SOOS_ORT_CACHE_DIR, an existing absolute directory). ort-sys verifies the
    # SHA-256 of every download before it extracts it into ORT_CACHE_DIR.
    local extra_args=()
    if [[ -n "${CARGO_BUILD_JOBS:-}" ]]; then
        extra_args+=(-e "CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}")
    fi
    if [[ -n "${SOOS_ORT_CACHE_DIR:-}" ]]; then
        if [[ "${SOOS_ORT_CACHE_DIR}" != /* || ! -d "${SOOS_ORT_CACHE_DIR}" ]]; then
            error "SOOS_ORT_CACHE_DIR must be an existing absolute directory: '${SOOS_ORT_CACHE_DIR}'"
            return 1
        fi
        extra_args+=(-v "${SOOS_ORT_CACHE_DIR}:/ort-cache" -e "ORT_CACHE_DIR=/ort-cache")
    fi

    # Consent is granted only to the disposable container, never to the host.
    info "Executing ${script} inside '${tag}'..."
    "${DOCKER_BIN}" run --rm \
        -v "${WORKSPACE_ROOT}":/workspace \
        -v "soos-distro-target-${distro}":/workspace/target \
        "${extra_args[@]}" \
        "${tag}" \
        bash "/workspace/tests/distro/${script}" --allow-host-changes "${flags[@]}"
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
