#!/usr/bin/env bash
# =============================================================================
# run_tests.sh — PAM compilation & integration test in an isolated container
# =============================================================================
# Executes:
#   1. Builds sandbox Docker image (if not already cached)
#   2. Runs ephemeral container with workspace bind-mounted
#   3. Compiles PAM module in release mode
#   4. Deploys .so into container PAM directory
#   5. Runs the PAM matrix T1–T10 (ABI loading, refusal, password fallback,
#      release-build panic safety)
#   6. Automatically removes ephemeral container
#
# Usage:
#   ./run_tests.sh [--matrix|--all|ubuntu|fedora|arch|authselect]
#
#   authselect: activates, validates and rolls back the Fedora authselect
#               profile in a fedora:40 container (tests/docker/authselect_profile_test.sh)
# =============================================================================

set -euo pipefail

MODE="${1:-default}"

if [[ "${MODE}" == "--matrix" || "${MODE}" == "--all" ]]; then
    exec ./tests/docker/run_matrix.sh all
elif [[ "${MODE}" == "ubuntu" || "${MODE}" == "fedora" || "${MODE}" == "arch" ]]; then
    exec ./tests/docker/run_matrix.sh "${MODE}"
elif [[ "${MODE}" == "authselect" ]]; then
    exec ./tests/docker/authselect_profile_test.sh
fi

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
readonly IMAGE_NAME="soos-sandbox"
readonly CONTAINER_NAME="soos-test-run"
readonly PAM_MODULE_NAME="libpam_soos.so"
readonly PAM_MODULES_DIR="/lib/x86_64-linux-gnu/security"

# ---------------------------------------------------------------------------
# Terminal Colors
# ---------------------------------------------------------------------------
if [[ -t 1 ]]; then
    readonly GREEN='\033[0;32m'
    readonly RED='\033[0;31m'
    readonly YELLOW='\033[1;33m'
    readonly BLUE='\033[0;34m'
    readonly NC='\033[0m'
else
    readonly GREEN=''
    readonly RED=''
    readonly YELLOW=''
    readonly BLUE=''
    readonly NC=''
fi

info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[FAIL]${NC}  $*" >&2; }

# ---------------------------------------------------------------------------
# Pre-flight Checks
# ---------------------------------------------------------------------------
if ! command -v docker &> /dev/null; then
    error "Docker is not installed or not in PATH."
    error "Install Docker: https://docs.docker.com/engine/install/"
    exit 1
fi

if ! docker info &> /dev/null; then
    error "Docker daemon is not accessible."
    error "Ensure Docker is running and current user belongs to the 'docker' group."
    exit 1
fi

if [[ ! -f "Dockerfile" ]]; then
    error "This script must be executed from the root of the soos workspace."
    error "File 'Dockerfile' not found in current directory: $(pwd)"
    exit 1
fi

# ---------------------------------------------------------------------------
# Step 1: Docker Image Build
# ---------------------------------------------------------------------------
info "Building Docker sandbox image '${IMAGE_NAME}'..."
if docker build -t "${IMAGE_NAME}" .; then
    success "Image '${IMAGE_NAME}' built successfully."
else
    error "Failed to build Docker sandbox image."
    exit 1
fi

# ---------------------------------------------------------------------------
# Step 2: Preventative Cleanup
# ---------------------------------------------------------------------------
if docker container inspect "${CONTAINER_NAME}" &> /dev/null; then
    warn "Residual container '${CONTAINER_NAME}' detected, removing..."
    docker rm -f "${CONTAINER_NAME}" > /dev/null
fi

# ---------------------------------------------------------------------------
# Step 3: Ephemeral Container Execution & PAM Testing
# ---------------------------------------------------------------------------
info "Launching ephemeral sandbox container '${CONTAINER_NAME}'..."
info "  → Mount: $(pwd) → /workspace"
info "  → Running full PAM test matrix suite..."

docker run --rm \
    --name "${CONTAINER_NAME}" \
    -v "$(pwd)":/workspace \
    "${IMAGE_NAME}" \
    bash /workspace/tests/docker/test_suite.sh


echo ""
success "═══════════════════════════════════════════"
success "  Sandbox validated — container removed"
success "═══════════════════════════════════════════"
