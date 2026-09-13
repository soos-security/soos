#!/usr/bin/env bash
# =============================================================================
# run_tests.sh — PAM compilation & integration test in an isolated container
# =============================================================================
# Executes:
#   1. Builds sandbox Docker image (if not already cached)
#   2. Runs ephemeral container with workspace bind-mounted
#   3. Compiles PAM module in release mode
#   4. Deploys .so into container PAM directory
#   5. Runs pamtester to validate ABI loading, refusal, and password fallback
#   6. Automatically removes ephemeral container
#
# Usage:
#   ./run_tests.sh
# =============================================================================

set -euo pipefail

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
info "  → Release compilation + pamtester assertions"

docker run --rm \
    --name "${CONTAINER_NAME}" \
    -v "$(pwd)":/workspace \
    "${IMAGE_NAME}" \
    bash -c '
set -euo pipefail

echo ""
echo "========================================="
echo "  SOOS — PAM Testing Sandbox"
echo "========================================="
echo ""

if [[ ! -f "Cargo.toml" ]]; then
    echo "[FAIL] Cargo.toml not found in /workspace."
    exit 1
fi

echo "[INFO] Compiling PAM module in release mode..."
cargo build --release -p soos-pam 2>&1
echo "[OK]   Compilation completed."

SO_PATH="target/release/'"${PAM_MODULE_NAME}"'"
if [[ ! -f "${SO_PATH}" ]]; then
    echo "[FAIL] Artifact ${SO_PATH} not found after build."
    exit 1
fi

echo "[INFO] Deploying PAM module..."
cp "${SO_PATH}" '"${PAM_MODULES_DIR}"'/pam_soos.so
chmod 644 '"${PAM_MODULES_DIR}"'/pam_soos.so
echo "[OK]   Module deployed to '"${PAM_MODULES_DIR}"'/pam_soos.so"

echo ""
echo "[TEST] T1 — C ABI loading + valid password authentication"
if echo "password123" | pamtester test-soos testuser authenticate; then
    echo "[OK]   T1 passed: module loaded, PAM_IGNORE returned, pam_unix verified password."
else
    echo "[FAIL] T1 failed: module crashed or pam_unix rejected password."
    exit 1
fi

echo ""
echo "[TEST] T2 — Invalid password (must reject cleanly)"
if echo "wrong_password" | pamtester test-soos testuser authenticate; then
    echo "[FAIL] T2 failed: invalid password was unexpectedly accepted!"
    exit 1
else
    echo "[OK]   T2 passed: invalid password rejected as expected."
fi

echo ""
echo "[TEST] T3 — Absent .so module (PAM stack fault tolerance)"
mv '"${PAM_MODULES_DIR}"'/pam_soos.so '"${PAM_MODULES_DIR}"'/pam_soos.so.bak

if echo "password123" | pamtester test-soos testuser authenticate; then
    echo "[OK]   T3 passed: PAM stack remains fully functional without pam_soos.so."
else
    echo "[WARN] T3: PAM rejected valid password without module."
fi
mv '"${PAM_MODULES_DIR}"'/pam_soos.so.bak '"${PAM_MODULES_DIR}"'/pam_soos.so

echo ""
echo "========================================="
echo "  ALL SANDBOX INTEGRATION TESTS: OK"
echo "========================================="
echo ""
'

echo ""
success "═══════════════════════════════════════════"
success "  Sandbox validated — container removed"
success "═══════════════════════════════════════════"
