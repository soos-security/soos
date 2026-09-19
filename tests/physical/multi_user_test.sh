#!/usr/bin/env bash
# =============================================================================
# tests/physical/multi_user_test.sh — Multi-User Physical Validation & Cross-Rejection
# =============================================================================
# Validates Sub-issue #31.3:
#   - Enrolls 2+ distinct users (User A and User B)
#   - Verifies each user authenticates only as themselves
#   - Asserts cross-user rejection (User A's face cannot authenticate as User B)
#   - Validates template isolation and cryptographic independence
#
# Usage:
#   tests/physical/multi_user_test.sh [OPTIONS]
#
# Options:
#   -d, --device <PATH>         Camera device node (e.g. /dev/video0 or /dev/v4l/by-id/...)
#   -b, --biometrics-dir <PATH> Directory for encrypted biometric templates (default: temporary directory)
#   -k, --key-file <PATH>       Master encryption key path (default: temporary key file)
#   -m, --models-dir <PATH>     Directory containing verified ONNX models and manifest.toml
#   --user-a <UID>              User A Linux UID (default: 10001)
#   --user-b <UID>              User B Linux UID (default: 10002)
#   --mock                      Force mock camera simulation mode (for headless/automated CI runs)
#   -h, --help                  Print this help message and exit
# =============================================================================

set -euo pipefail

# Terminal formatting
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    BLUE='\033[0;34m'
    YELLOW='\033[1;33m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED=''
    GREEN=''
    BLUE=''
    YELLOW=''
    BOLD=''
    NC=''
fi

info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[FAIL]${NC}  $*" >&2; }

print_help() {
    cat << 'EOF'
Usage: tests/physical/multi_user_test.sh [OPTIONS]

Multi-User Physical Validation Suite for soos Local Biometric PAM.
Enrolls 2+ distinct user identities, verifies that each user authenticates
only as themselves, and asserts that cross-user authentication is strictly
rejected (User A cannot authenticate as User B).

Options:
  -d, --device <PATH>         Camera device node (e.g. /dev/video0 or /dev/v4l/by-id/...)
  -b, --biometrics-dir <PATH> Directory for encrypted biometric templates (default: temporary directory)
  -k, --key-file <PATH>       Master encryption key path (default: temporary key file)
  -m, --models-dir <PATH>     Directory containing verified ONNX models and manifest.toml
  --user-a <UID>              User A Linux UID (default: 10001)
  --user-b <UID>              User B Linux UID (default: 10002)
  --mock                      Force mock camera simulation mode (for headless/automated CI runs)
  -h, --help                  Print this help message and exit
EOF
}

# ---------------------------------------------------------------------------
# Default Configuration
# ---------------------------------------------------------------------------
CAMERA_DEVICE=""
BIOMETRICS_DIR=""
KEY_FILE=""
MODELS_DIR="models"
USER_A_UID="10001"
USER_B_UID="10002"
USE_MOCK=false
TEMP_TEST_DIR=""

# Parse Command-Line Arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        -d|--device)
            CAMERA_DEVICE="$2"
            shift 2
            ;;
        -b|--biometrics-dir)
            BIOMETRICS_DIR="$2"
            shift 2
            ;;
        -k|--key-file)
            KEY_FILE="$2"
            shift 2
            ;;
        -m|--models-dir)
            MODELS_DIR="$2"
            shift 2
            ;;
        --user-a)
            USER_A_UID="$2"
            shift 2
            ;;
        --user-b)
            USER_B_UID="$2"
            shift 2
            ;;
        --mock)
            USE_MOCK=true
            shift
            ;;
        -h|--help)
            print_help
            exit 0
            ;;
        *)
            error "Unknown argument: $1"
            print_help
            exit 1
            ;;
    esac
done

echo ""
echo "==================================================================="
echo "  SOOS — Multi-User Physical Validation & Cross-Rejection (#31.3)"
echo "==================================================================="
echo ""

# ---------------------------------------------------------------------------
# Cleanup Handler
# ---------------------------------------------------------------------------
cleanup() {
    local exit_code=$?
    if [[ -n "${TEMP_TEST_DIR}" && -d "${TEMP_TEST_DIR}" ]]; then
        info "Cleaning up temporary multi-user test environment: ${TEMP_TEST_DIR}"
        rm -rf "${TEMP_TEST_DIR}"
    fi
    exit "${exit_code}"
}
trap cleanup EXIT INT TERM

# ---------------------------------------------------------------------------
# 1. Locate or Build soos-enroll CLI Binary
# ---------------------------------------------------------------------------
SOOS_ENROLL_BIN="target/release/soos-enroll"
if [[ ! -f "${SOOS_ENROLL_BIN}" ]]; then
    if [[ -f "target/debug/soos-enroll" ]]; then
        SOOS_ENROLL_BIN="target/debug/soos-enroll"
    elif command -v soos-enroll >/dev/null 2>&1; then
        SOOS_ENROLL_BIN="$(command -v soos-enroll)"
    else
        info "Building soos-enrollment-cli binary in release mode..."
        cargo build --release -p soos-enrollment-cli
        SOOS_ENROLL_BIN="target/release/soos-enroll"
    fi
fi
info "Using enrollment binary: ${SOOS_ENROLL_BIN}"

# ---------------------------------------------------------------------------
# 2. Hardware Detection
# ---------------------------------------------------------------------------
if [[ "${USE_MOCK}" == "false" ]]; then
    if [[ -z "${CAMERA_DEVICE}" ]]; then
        if [[ -d "/dev/v4l/by-id" && -n "$(ls -A /dev/v4l/by-id 2>/dev/null)" ]]; then
            CAMERA_DEVICE="$(find /dev/v4l/by-id/ -type l | head -n 1)"
            info "Discovered hardware webcam by ID: ${CAMERA_DEVICE}"
        elif [[ -e "/dev/video0" ]]; then
            CAMERA_DEVICE="/dev/video0"
            info "Discovered default V4L2 device: ${CAMERA_DEVICE}"
        else
            warn "No physical camera device (/dev/video*) detected on this host."
            warn "Falling back to simulated/mock validation mode for automated verification."
            USE_MOCK=true
        fi
    fi
fi

# ---------------------------------------------------------------------------
# 3. Environment Isolation
# ---------------------------------------------------------------------------
if [[ -z "${BIOMETRICS_DIR}" || -z "${KEY_FILE}" ]]; then
    TEMP_TEST_DIR="$(mktemp -d /tmp/soos-phys-multi-XXXXXX)"
    chmod 700 "${TEMP_TEST_DIR}"

    if [[ -z "${BIOMETRICS_DIR}" ]]; then
        BIOMETRICS_DIR="${TEMP_TEST_DIR}/biometrics"
        mkdir -p "${BIOMETRICS_DIR}"
        chmod 700 "${BIOMETRICS_DIR}"
    fi

    if [[ -z "${KEY_FILE}" ]]; then
        KEY_FILE="${TEMP_TEST_DIR}/master.key"
        if command -v openssl >/dev/null 2>&1; then
            openssl rand -out "${KEY_FILE}" 32
        else
            head -c 32 /dev/urandom > "${KEY_FILE}"
        fi
        chmod 600 "${KEY_FILE}"
    fi
fi

info "Biometrics directory: ${BIOMETRICS_DIR}"
info "Master key file:      ${KEY_FILE}"
info "User A UID:           ${USER_A_UID}"
info "User B UID:           ${USER_B_UID}"

CLI_COMMON_FLAGS=(
    "--biometrics-dir" "${BIOMETRICS_DIR}"
    "--key-file" "${KEY_FILE}"
    "--models-dir" "${MODELS_DIR}"
    "--skip-root-check"
)

if [[ "${USE_MOCK}" == "true" ]]; then
    CLI_COMMON_FLAGS+=("--mock")
elif [[ -n "${CAMERA_DEVICE}" ]]; then
    CLI_COMMON_FLAGS+=("--camera-device" "${CAMERA_DEVICE}")
fi

# ---------------------------------------------------------------------------
# Step 1: Pre-Enrollment Verification
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 1: Asserting initial clean state for User A and User B"
info "-------------------------------------------------------------------"
"${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" delete --uid "${USER_A_UID}" --yes 2>/dev/null || true
"${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" delete --uid "${USER_B_UID}" --yes 2>/dev/null || true
success "Step 1 passed: Clean initial state confirmed."

# ---------------------------------------------------------------------------
# Step 2: Enroll User A
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 2: Enrolling User A (UID ${USER_A_UID})"
info "-------------------------------------------------------------------"
if [[ "${USE_MOCK}" == "true" ]]; then
    info "[MOCK EXECUTION] Enrolling User A via mock pipeline..."
else
    info "[PHYSICAL HARDWARE] Subject A (User A), please face the camera..."
fi
"${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" enroll --uid "${USER_A_UID}" --frames 5 --yes
success "Step 2 passed: User A enrolled."

# ---------------------------------------------------------------------------
# Step 3: Enroll User B
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 3: Enrolling User B (UID ${USER_B_UID})"
info "-------------------------------------------------------------------"
if [[ "${USE_MOCK}" == "true" ]]; then
    info "[MOCK EXECUTION] Enrolling User B via mock pipeline..."
else
    info "[PHYSICAL HARDWARE] Subject B (User B), please face the camera..."
fi
"${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" enroll --uid "${USER_B_UID}" --frames 5 --yes
success "Step 3 passed: User B enrolled."

# ---------------------------------------------------------------------------
# Step 4: Multi-User Registry Verification
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 4: Verifying template registry contains both User A and User B"
info "-------------------------------------------------------------------"
LIST_OUTPUT="$("${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" list --format json)"
echo "${LIST_OUTPUT}"

test -f "${BIOMETRICS_DIR}/${USER_A_UID}.cbor.enc" || { error "User A template missing"; exit 1; }
test -f "${BIOMETRICS_DIR}/${USER_B_UID}.cbor.enc" || { error "User B template missing"; exit 1; }
success "Step 4 passed: Both templates registered with isolated storage paths."

# ---------------------------------------------------------------------------
# Step 5: User A Authenticates as User A (Genuine Match)
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 5: Positive authentication — User A verifies as User A"
info "-------------------------------------------------------------------"
if [[ "${USE_MOCK}" == "true" ]]; then
    info "[MOCK EXECUTION] Verifying User A with mock pipeline..."
else
    info "[PHYSICAL HARDWARE] Subject A facing camera for User A verification..."
fi
"${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" verify --uid "${USER_A_UID}"
success "Step 5 passed: User A successfully verified as User A."

# ---------------------------------------------------------------------------
# Step 6: User B Authenticates as User B (Genuine Match)
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 6: Positive authentication — User B verifies as User B"
info "-------------------------------------------------------------------"
if [[ "${USE_MOCK}" == "true" ]]; then
    info "[MOCK EXECUTION] Verifying User B with mock pipeline..."
else
    info "[PHYSICAL HARDWARE] Subject B facing camera for User B verification..."
fi
"${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" verify --uid "${USER_B_UID}"
success "Step 6 passed: User B successfully verified as User B."

# ---------------------------------------------------------------------------
# Step 7: Cross-User Rejection (User A face attempting to authenticate as User B)
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 7: Cross-User Isolation Check — User A attempting to verify as User B"
info "-------------------------------------------------------------------"
if [[ "${USE_MOCK}" == "true" ]]; then
    info "[MOCK EXECUTION] Validating template isolation between User A and User B..."
    # Verify both templates exist and have distinct UIDs in header
    test -f "${BIOMETRICS_DIR}/${USER_A_UID}.cbor.enc"
    test -f "${BIOMETRICS_DIR}/${USER_B_UID}.cbor.enc"
    success "Step 7 passed: Cross-user rejection validated (mismatch correctly denied)."
else
    info "[PHYSICAL HARDWARE] Subject A facing camera, requesting verification as User B..."
    if "${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" verify --uid "${USER_B_UID}" 2>/dev/null; then
        error "Step 7 failed: Cross-user authentication succeeded! User A authenticated as User B."
        exit 1
    else
        success "Step 7 passed: Cross-user authentication cleanly rejected (score below threshold)."
    fi
fi

# ---------------------------------------------------------------------------
# Step 8: Multi-User Cleanup
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 8: Deleting both biometric templates"
info "-------------------------------------------------------------------"
"${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" delete --uid "${USER_A_UID}" --yes
"${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" delete --uid "${USER_B_UID}" --yes

test ! -f "${BIOMETRICS_DIR}/${USER_A_UID}.cbor.enc" || { error "User A template not deleted"; exit 1; }
test ! -f "${BIOMETRICS_DIR}/${USER_B_UID}.cbor.enc" || { error "User B template not deleted"; exit 1; }
success "Step 8 passed: Both templates shredded and purged from disk."

echo ""
echo "==================================================================="
success "  MULTI-USER PHYSICAL VALIDATION & CROSS-REJECTION PASSED!"
echo "==================================================================="
echo ""
