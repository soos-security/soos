#!/usr/bin/env bash
# =============================================================================
# tests/docker/run_matrix.sh — Host Driver for Multi-Distribution PAM Matrix
# =============================================================================
# Usage:
#   ./tests/docker/run_matrix.sh [ubuntu|fedora|arch|all]
#
# Default: ubuntu
# =============================================================================

set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[FAIL]${NC}  $*" >&2; }

TARGET="${1:-ubuntu}"

if ! command -v docker &> /dev/null; then
    error "Docker is not installed or not in PATH."
    exit 1
fi

if ! docker info &> /dev/null; then
    error "Docker daemon is not accessible. Verify user permissions or group membership."
    exit 1
fi

run_distro() {
    local distro="$1"
    local dockerfile="tests/docker/Dockerfile.${distro}"
    local tag="soos-sandbox-${distro}"

    if [[ ! -f "${dockerfile}" ]]; then
        error "Dockerfile for '${distro}' not found: ${dockerfile}"
        return 1
    fi

    echo ""
    info "==================================================================="
    info "  Executing PAM Matrix for Distribution: ${distro^^}"
    info "==================================================================="
    info "Building container image '${tag}' from ${dockerfile}..."
    docker build -f "${dockerfile}" -t "${tag}" .

    info "Running container test suite..."
    docker run --rm \
        --name "soos-matrix-${distro}" \
        -v "$(pwd)":/workspace \
        "${tag}" \
        bash /workspace/tests/docker/test_suite.sh

    success "Distribution '${distro}' passed all PAM matrix tests."
}

case "${TARGET}" in
    ubuntu)
        run_distro "ubuntu"
        ;;
    fedora)
        run_distro "fedora"
        ;;
    arch)
        run_distro "arch"
        ;;
    all)
        info "Running Full Multi-Distribution PAM Test Matrix (Ubuntu, Fedora, Arch)..."
        run_distro "ubuntu"
        run_distro "fedora"
        run_distro "arch"
        echo ""
        success "==================================================================="
        success "  FULL MULTI-DISTRO MATRIX COMPLETED WITH 100% SUCCESS!"
        success "==================================================================="
        ;;
    *)
        error "Unknown target '${TARGET}'. Supported targets: ubuntu, fedora, arch, all."
        exit 1
        ;;
esac
