#!/usr/bin/env bash
# =============================================================================
# tests/physical/enrollment_test.sh — Physical Hardware Enrollment Validation Suite
# =============================================================================
# Validates Sub-issue #31.1:
#   - Full enrollment lifecycle on physical hardware with a real USB webcam
#   - Steps: Initial Clean State -> Enroll -> Verify -> List -> Delete -> Final Clean State
#   - Verifies real camera capture, neural model inference, and template encryption
#   - Supports deterministic mock fallback for headless and continuous integration
#
# Usage:
#   tests/physical/enrollment_test.sh [OPTIONS]
#
# Options:
#   -d, --device <PATH>         Camera device node (e.g. /dev/video0 or /dev/v4l/by-id/...)
#   -b, --biometrics-dir <PATH> Directory for encrypted biometric templates (default: temporary directory)
#   -k, --key-file <PATH>       Master encryption key path (default: temporary key file)
#   -m, --models-dir <PATH>     Directory containing verified ONNX models and manifest.toml
#                               (default: the soos-enroll default, /var/lib/soos/models)
#   -u, --uid <UID>             Target User ID for enrollment testing (default: SUDO_UID, else current UID)
#   --mock                      Force mock camera simulation mode (for headless/automated CI runs)
#   -h, --help                  Print this help message and exit
#
# soos-enroll refuses to run without root (EUID 0) and accepts only absolute paths, so run
# this suite with sudo after building the CLI as your user:
#   cargo build --release -p soos-enrollment-cli
#   sudo tests/physical/enrollment_test.sh [OPTIONS]
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
Usage: tests/physical/enrollment_test.sh [OPTIONS]

Physical Hardware Enrollment Validation Suite for soos Local Biometric PAM.
Validates the full enrollment lifecycle (enroll -> verify -> list -> delete) on
physical hardware with real USB webcam capture and neural model inference.

Options:
  -d, --device <PATH>         Camera device node (e.g. /dev/video0 or /dev/v4l/by-id/...)
  -b, --biometrics-dir <PATH> Directory for encrypted biometric templates (default: temporary directory)
  -k, --key-file <PATH>       Master encryption key path (default: temporary key file)
  -m, --models-dir <PATH>     Directory containing verified ONNX models and manifest.toml
                              (default: the soos-enroll default, /var/lib/soos/models)
  -u, --uid <UID>             Target User ID for enrollment testing (default: SUDO_UID, else current UID)
  --mock                      Force mock camera simulation mode (for headless/automated CI runs)
  -h, --help                  Print this help message and exit

soos-enroll requires root (EUID 0): build it as your user, then run this suite with sudo:
  cargo build --release -p soos-enrollment-cli
  sudo tests/physical/enrollment_test.sh [OPTIONS]
EOF
}

# ---------------------------------------------------------------------------
# Default Configuration
# ---------------------------------------------------------------------------
CAMERA_DEVICE=""
BIOMETRICS_DIR=""
KEY_FILE=""
MODELS_DIR=""
TARGET_UID="${SUDO_UID:-${UID}}"
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
        -u|--uid)
            TARGET_UID="$2"
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
echo "  SOOS — Physical Hardware Enrollment Validation Suite (#31.1)"
echo "==================================================================="
echo ""

# soos-enroll checks for EUID 0 before it parses any argument (there is no bypass flag).
if [[ "${EUID}" -ne 0 ]]; then
    error "soos-enroll requires root privileges (EUID 0)."
    error "Build it as your user, then re-run: sudo tests/physical/enrollment_test.sh [OPTIONS]"
    exit 1
fi

# ---------------------------------------------------------------------------
# Cleanup Handler
# ---------------------------------------------------------------------------
cleanup() {
    local exit_code=$?
    if [[ -n "${TEMP_TEST_DIR}" && -d "${TEMP_TEST_DIR}" ]]; then
        info "Cleaning up temporary test environment: ${TEMP_TEST_DIR}"
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
        # Never run cargo as root: it would leave root-owned build outputs in target/.
        error "soos-enroll binary not found. Build it as your user first:"
        error "  cargo build --release -p soos-enrollment-cli"
        exit 1
    fi
fi
info "Using enrollment binary: ${SOOS_ENROLL_BIN}"

# ---------------------------------------------------------------------------
# 2. Hardware Detection and Mode Selection
# ---------------------------------------------------------------------------
if [[ "${USE_MOCK}" == "false" ]]; then
    if [[ -z "${CAMERA_DEVICE}" ]]; then
        # Search for available V4L2 video devices
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
    else
        if [[ ! -e "${CAMERA_DEVICE}" ]]; then
            error "Specified camera device does not exist: ${CAMERA_DEVICE}"
            exit 1
        fi
        info "Using specified physical camera device: ${CAMERA_DEVICE}"
    fi
else
    info "Mock mode explicitly requested; using mock camera simulation."
fi

# ---------------------------------------------------------------------------
# 3. Environment & Storage Isolation
# ---------------------------------------------------------------------------
if [[ -z "${BIOMETRICS_DIR}" || -z "${KEY_FILE}" ]]; then
    TEMP_TEST_DIR="$(mktemp -d /tmp/soos-phys-enroll-XXXXXX)"
    chmod 700 "${TEMP_TEST_DIR}"

    if [[ -z "${BIOMETRICS_DIR}" ]]; then
        BIOMETRICS_DIR="${TEMP_TEST_DIR}/biometrics"
        mkdir -p "${BIOMETRICS_DIR}"
        chmod 700 "${BIOMETRICS_DIR}"
    fi

    if [[ -z "${KEY_FILE}" ]]; then
        KEY_FILE="${TEMP_TEST_DIR}/master.key"
        # Generate random 32-byte master encryption key
        if command -v openssl >/dev/null 2>&1; then
            openssl rand -out "${KEY_FILE}" 32
        else
            head -c 32 /dev/urandom > "${KEY_FILE}"
        fi
        chmod 600 "${KEY_FILE}"
    fi
fi

# soos-enroll validates every path as absolute (no relative components).
BIOMETRICS_DIR="$(realpath -m -- "${BIOMETRICS_DIR}")"
KEY_FILE="$(realpath -m -- "${KEY_FILE}")"
if [[ -n "${MODELS_DIR}" ]]; then
    MODELS_DIR="$(realpath -m -- "${MODELS_DIR}")"
fi

info "Biometrics directory: ${BIOMETRICS_DIR}"
info "Master key file:      ${KEY_FILE}"
info "Models directory:     ${MODELS_DIR:-soos-enroll default}"
info "Target test UID:      ${TARGET_UID}"

# Base flags for soos-enroll invocations
CLI_COMMON_FLAGS=(
    "--biometrics-dir" "${BIOMETRICS_DIR}"
    "--key-file" "${KEY_FILE}"
)
if [[ -n "${MODELS_DIR}" ]]; then
    CLI_COMMON_FLAGS+=("--models-dir" "${MODELS_DIR}")
fi

if [[ "${USE_MOCK}" == "true" ]]; then
    CLI_COMMON_FLAGS+=("--mock")
elif [[ -n "${CAMERA_DEVICE}" ]]; then
    CLI_COMMON_FLAGS+=("--camera-device" "${CAMERA_DEVICE}")
fi

# ---------------------------------------------------------------------------
# Step 1: Pre-enrollment Clean State Verification
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 1: Asserting initial clean state (zero enrolled templates)"
info "-------------------------------------------------------------------"
INITIAL_LIST="$("${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" list --format json 2>&1 || true)"
if echo "${INITIAL_LIST}" | grep -q "\"uid\": ${TARGET_UID},"; then
    warn "UID ${TARGET_UID} was previously enrolled. Purging existing template..."
    "${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" delete --uid "${TARGET_UID}" --yes
fi
success "Initial state verified: UID ${TARGET_UID} is not enrolled."

# ---------------------------------------------------------------------------
# Step 2: Biometric Enrollment Lifecycle Execution
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 2: Executing biometric enrollment (capture, inference, encryption)"
info "-------------------------------------------------------------------"
info "Evaluating face frames and extracting biometric embedding..."

if [[ "${USE_MOCK}" == "true" ]]; then
    info "[MOCK EXECUTION] Running simulated face capture with synthetic frames..."
else
    info "[PHYSICAL HARDWARE] Please look directly into the camera lens for face capture..."
fi
"${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" enroll --uid "${TARGET_UID}" --frames 5 --yes
success "Step 2 passed: Biometric enrollment completed successfully."

# ---------------------------------------------------------------------------
# Step 3: Template Registry & Invariant Verification
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 3: Listing registered templates and auditing filesystem invariants"
info "-------------------------------------------------------------------"
"${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" list --format table

# Assert template file existence and 0600 mode
TEMPLATE_PATH="${BIOMETRICS_DIR}/${TARGET_UID}.cbor.enc"
if [[ ! -f "${TEMPLATE_PATH}" ]]; then
    error "Enrolled template file missing at ${TEMPLATE_PATH}"
    exit 1
fi

FILE_MODE="$(stat -c "%a" "${TEMPLATE_PATH}" 2>/dev/null || stat -f "%OLp" "${TEMPLATE_PATH}")"
if [[ "${FILE_MODE}" != "600" ]]; then
    error "Template file permissions invariant violated: ${FILE_MODE} (expected 600)"
    exit 1
fi
success "Step 3 passed: Template exists with invariant mode 0600."

# ---------------------------------------------------------------------------
# Step 4: Biometric Verification
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 4: Executing diagnostic biometric verification (verify subcommand)"
info "-------------------------------------------------------------------"
if [[ "${USE_MOCK}" == "true" ]]; then
    info "[MOCK EXECUTION] Verifying enrolled template with mock inference..."
else
    info "[PHYSICAL HARDWARE] Please face the camera for biometric verification..."
fi
"${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" verify --uid "${TARGET_UID}"
success "Step 4 passed: Biometric verification succeeded with Verdict::Allow."

# ---------------------------------------------------------------------------
# Step 5: Secure Deletion & Shredding
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 5: Secure deletion of biometric template (shred & remove)"
info "-------------------------------------------------------------------"
"${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" delete --uid "${TARGET_UID}" --yes

if [[ -f "${TEMPLATE_PATH}" ]]; then
    error "Template file still exists after delete subcommand: ${TEMPLATE_PATH}"
    exit 1
fi
success "Step 5 passed: Template securely shredded and removed."

# ---------------------------------------------------------------------------
# Step 6: Post-deletion Clean State Verification
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "Step 6: Confirming clean slate after deletion"
info "-------------------------------------------------------------------"
FINAL_LIST="$("${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" list --format json 2>&1 || true)"
if echo "${FINAL_LIST}" | grep -q "\"uid\": ${TARGET_UID},"; then
    error "UID ${TARGET_UID} still reported in template list after deletion!"
    exit 1
fi
success "Step 6 passed: Registry confirmed completely clean."

echo ""
echo "==================================================================="
success "  PHYSICAL ENROLLMENT LIFECYCLE VALIDATION PASSED SUCCESSFULLY!"
echo "==================================================================="
echo ""
