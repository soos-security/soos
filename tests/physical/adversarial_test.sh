#!/usr/bin/env bash
# =============================================================================
# tests/physical/adversarial_test.sh — Presentation Attack Detection (PAD) Suite
# =============================================================================
# Validates Sub-issue #31.5:
#   - Adversarial presentation attack validation against physical hardware
#   - Attack Categories (NIST SP 800-63B / ISO/IEC 30107-3):
#       1. Printed photograph (matte/glossy paper presentation)
#       2. Smartphone digital screen (OLED/LCD display & moiré pattern)
#       3. Digital video replay attack (recorded motion on screen)
#   - Asserts that the PAD anti-spoofing model rejects presentation attacks
#   - Computes Attack Presentation Classification Error Rate (APCER / FAR)
#   - Computes Bona Fide Presentation Classification Error Rate (BPCER / FRR)
#   - Outputs structured adversarial security audit report
#
# Usage:
#   tests/physical/adversarial_test.sh [OPTIONS]
#
# Options:
#   -d, --device <PATH>     Camera device node (e.g. /dev/video0 or /dev/v4l/by-id/...)
#   -m, --models-dir <PATH> Directory containing verified ONNX models and manifest.toml
#   -u, --uid <UID>         Target enrolled user UID to verify against (default: 10001)
#   --mock                  Simulation only: PAD plumbing + real-model tests, NO security metrics
#   -h, --help              Print this help message and exit
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
Usage: tests/physical/adversarial_test.sh [OPTIONS]

Presentation Attack Detection (PAD) Adversarial Validation Suite for soos.
Validates the anti-spoofing machine learning model on physical hardware
against printed photos, smartphone OLED/LCD screens, and video replays.

Options:
  -d, --device <PATH>     Camera device node (e.g. /dev/video0 or /dev/v4l/by-id/...)
  -m, --models-dir <PATH> Directory containing verified ONNX models and manifest.toml
  -u, --uid <UID>         Target enrolled user UID to verify against (default: 10001)
  --mock                  Simulation only: runs PAD plumbing and real-model tests,
                          reports NO security metrics (no APCER / BPCER)
  -h, --help              Print this help message and exit
EOF
}

# ---------------------------------------------------------------------------
# Default Configuration
# ---------------------------------------------------------------------------
CAMERA_DEVICE=""
MODELS_DIR="models"
TARGET_UID="${UID:-10001}"
USE_MOCK=false
TEMP_TEST_DIR=""

# Parse Command-Line Arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        -d|--device)
            CAMERA_DEVICE="$2"
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
echo "  SOOS — Presentation Attack Detection (PAD) Adversarial Suite (#31.5)"
echo "==================================================================="
echo ""

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
# 1. Hardware Detection
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
# 2. Environment Setup
# ---------------------------------------------------------------------------
TEMP_TEST_DIR="$(mktemp -d /tmp/soos-phys-adv-XXXXXX)"
chmod 700 "${TEMP_TEST_DIR}"
BIOMETRICS_DIR="${TEMP_TEST_DIR}/biometrics"
mkdir -p "${BIOMETRICS_DIR}"
chmod 700 "${BIOMETRICS_DIR}"
KEY_FILE="${TEMP_TEST_DIR}/master.key"
head -c 32 /dev/urandom > "${KEY_FILE}"
chmod 600 "${KEY_FILE}"

# Test Counters for Metrics Computation
ATTACKS_TESTED=0
ATTACKS_REJECTED=0
ATTACKS_ACCEPTED=0

BONA_FIDE_TESTED=0
BONA_FIDE_ACCEPTED=0
BONA_FIDE_REJECTED=0

# ---------------------------------------------------------------------------
# Automated / Simulated Execution (CI & Regression Test Harness)
# ---------------------------------------------------------------------------
if [[ "${USE_MOCK}" == "true" ]]; then
    # Mock mode is a SIMULATION (review finding PAD-06 / GitHub #172). It exercises the PAD
    # plumbing and the real-model evidence target, but it NEVER computes, fabricates or reports
    # APCER / BPCER: the only valid security metrics come from a physical session (below) or
    # from the corpus-driven test `pad_real_model_tests` run with SOOS_PAD_CORPUS_DIR set.
    warn "SIMULATION – no security metrics"
    info "Running PAD pipeline plumbing tests (MockPadDetector, scripted verdicts)..."
    cargo test -p soos-vision --test pad_tests -- --nocapture

    info "Running real-model PAD evidence target (skips cleanly when models are absent)..."
    SOOS_MODELS_DIR="${SOOS_MODELS_DIR:-/var/lib/soos/models}" \
        cargo test -p soos-inference-ort --test pad_real_model_tests -- --nocapture

    echo ""
    echo "==================================================================="
    echo "   SIMULATION – no security metrics"
    echo "==================================================================="
    echo " Mock mode verified PAD plumbing only. No presentation attack was"
    echo " presented to a camera, so no APCER / BPCER is reported."
    echo " Real measurement: run this script on hardware without --mock, or run"
    echo "   SOOS_PAD_CORPUS_DIR=<corpus> cargo test -p soos-inference-ort \\"
    echo "     --test pad_real_model_tests -- --nocapture"
    echo " (see Docs/INFERENCE_ORT_CRATE.md, 'PAD real-model evidence')."
    echo "==================================================================="
    exit 0
else
    # -----------------------------------------------------------------------
    # Interactive Physical Hardware Presentation Attack Protocol
    # -----------------------------------------------------------------------
    info "Beginning physical presentation attack testing protocol."
    info "Ensure the enrolled subject's template is provisioned."

    # Baseline: Bona Fide (Genuine) Presentation
    echo ""
    info "-------------------------------------------------------------------"
    info "Phase 1: Bona Fide (Genuine Live Subject) Evaluation"
    info "-------------------------------------------------------------------"
    echo -n "Please present the genuine enrolled live subject to the camera and press Enter... "
    read -r
    BONA_FIDE_TESTED=$(( BONA_FIDE_TESTED + 1 ))
    # Verification call
    if target/release/soos-enroll \
        --biometrics-dir "${BIOMETRICS_DIR}" \
        --key-file "${KEY_FILE}" \
        --models-dir "${MODELS_DIR}" \
        --camera-device "${CAMERA_DEVICE}" \
        --skip-root-check \
        verify --uid "${TARGET_UID}" 2>/dev/null; then
        success "Bona fide presentation recognized as genuine (Verdict::Allow)."
        BONA_FIDE_ACCEPTED=$(( BONA_FIDE_ACCEPTED + 1 ))
    else
        warn "Bona fide presentation was rejected (FRR occurrence)."
        BONA_FIDE_REJECTED=$(( BONA_FIDE_REJECTED + 1 ))
    fi

    # Attack Type 1: High-Resolution Printed Photograph
    echo ""
    info "-------------------------------------------------------------------"
    info "Phase 2: Presentation Attack 1 — Printed Photograph (Matte/Glossy)"
    info "-------------------------------------------------------------------"
    echo -n "Hold a printed high-resolution photo of the subject before the lens and press Enter... "
    read -r
    ATTACKS_TESTED=$(( ATTACKS_TESTED + 1 ))
    if target/release/soos-enroll \
        --biometrics-dir "${BIOMETRICS_DIR}" \
        --key-file "${KEY_FILE}" \
        --models-dir "${MODELS_DIR}" \
        --camera-device "${CAMERA_DEVICE}" \
        --skip-root-check \
        verify --uid "${TARGET_UID}" 2>/dev/null; then
        error "VULNERABILITY: Printed photo spoof was accepted as genuine!"
        ATTACKS_ACCEPTED=$(( ATTACKS_ACCEPTED + 1 ))
    else
        success "Attack 1 passed: Printed photo correctly detected as spoof and rejected."
        ATTACKS_REJECTED=$(( ATTACKS_REJECTED + 1 ))
    fi

    # Attack Type 2: Smartphone Digital Screen
    echo ""
    info "-------------------------------------------------------------------"
    info "Phase 3: Presentation Attack 2 — Smartphone Screen (OLED / LCD)"
    info "-------------------------------------------------------------------"
    echo -n "Display a full-screen still photo on a smartphone before the lens and press Enter... "
    read -r
    ATTACKS_TESTED=$(( ATTACKS_TESTED + 1 ))
    if target/release/soos-enroll \
        --biometrics-dir "${BIOMETRICS_DIR}" \
        --key-file "${KEY_FILE}" \
        --models-dir "${MODELS_DIR}" \
        --camera-device "${CAMERA_DEVICE}" \
        --skip-root-check \
        verify --uid "${TARGET_UID}" 2>/dev/null; then
        error "VULNERABILITY: Smartphone screen spoof was accepted as genuine!"
        ATTACKS_ACCEPTED=$(( ATTACKS_ACCEPTED + 1 ))
    else
        success "Attack 2 passed: Smartphone screen correctly detected as spoof and rejected."
        ATTACKS_REJECTED=$(( ATTACKS_REJECTED + 1 ))
    fi

    # Attack Type 3: Digital Video Replay Attack
    echo ""
    info "-------------------------------------------------------------------"
    info "Phase 4: Presentation Attack 3 — Video Replay (Motion Display)"
    info "-------------------------------------------------------------------"
    echo -n "Play a recorded video clip of the subject's face on a screen before the lens and press Enter... "
    read -r
    ATTACKS_TESTED=$(( ATTACKS_TESTED + 1 ))
    if target/release/soos-enroll \
        --biometrics-dir "${BIOMETRICS_DIR}" \
        --key-file "${KEY_FILE}" \
        --models-dir "${MODELS_DIR}" \
        --camera-device "${CAMERA_DEVICE}" \
        --skip-root-check \
        verify --uid "${TARGET_UID}" 2>/dev/null; then
        error "VULNERABILITY: Video replay spoof was accepted as genuine!"
        ATTACKS_ACCEPTED=$(( ATTACKS_ACCEPTED + 1 ))
    else
        success "Attack 3 passed: Video replay attack correctly detected as spoof and rejected."
        ATTACKS_REJECTED=$(( ATTACKS_REJECTED + 1 ))
    fi
fi

# ---------------------------------------------------------------------------
# 3. Compute Metrics (APCER / BPCER)
# ---------------------------------------------------------------------------
# APCER = False Acceptances / Total Attacks Tested
# BPCER = False Rejections / Total Bona Fide Tested

APCER_PERCENT="0.00"
if [[ "${ATTACKS_TESTED}" -gt 0 ]]; then
    APCER_PERCENT="$(awk -v a="${ATTACKS_ACCEPTED}" -v t="${ATTACKS_TESTED}" 'BEGIN { printf "%.2f", (a/t)*100 }')"
fi

BPCER_PERCENT="0.00"
if [[ "${BONA_FIDE_TESTED}" -gt 0 ]]; then
    BPCER_PERCENT="$(awk -v r="${BONA_FIDE_REJECTED}" -v t="${BONA_FIDE_TESTED}" 'BEGIN { printf "%.2f", (r/t)*100 }')"
fi

echo ""
echo "==================================================================="
echo "             SOOS ADVERSARIAL PAD EVALUATION REPORT                "
echo "==================================================================="
echo " Standards Compliance:         NIST SP 800-63B / ISO/IEC 30107-3"
echo " Attack Presentations Tested:  ${ATTACKS_TESTED}"
echo " Attack Presentations Stopped: ${ATTACKS_REJECTED}"
echo " Attack Presentations Leaked:  ${ATTACKS_ACCEPTED}"
echo " APCER (Attack Presentation Error Rate / FAR): ${APCER_PERCENT}%"
echo "-------------------------------------------------------------------"
echo " Bona Fide (Live) Tested:      ${BONA_FIDE_TESTED}"
echo " Bona Fide Accepted:           ${BONA_FIDE_ACCEPTED}"
echo " Bona Fide Rejected:           ${BONA_FIDE_REJECTED}"
echo " BPCER (Bona Fide Error Rate / FRR):           ${BPCER_PERCENT}%"
echo "==================================================================="

if [[ "${ATTACKS_ACCEPTED}" -gt 0 ]]; then
    error "PAD SECURITY AUDIT FAILED: ${ATTACKS_ACCEPTED} presentation attacks breached anti-spoofing!"
    exit 1
else
    success "PAD physical session: 0 of ${ATTACKS_TESTED} presentation attacks accepted."
    warn "A single physical session is too small a sample for a certified APCER / BPCER;"
    warn "use the corpus-driven pad_real_model_tests target (>= 20 crops per class) for rates."
fi

echo ""
