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
#   -d, --device <PATH>         Camera device node (e.g. /dev/video0 or /dev/v4l/by-id/...)
#   -b, --biometrics-dir <PATH> Provisioned template store (default: the soos-enroll default,
#                               /var/lib/soos/biometrics)
#   -k, --key-file <PATH>       Master key of that store (default: /var/lib/soos/master.key)
#   -m, --models-dir <PATH>     Directory containing verified ONNX models and manifest.toml
#                               (default: the soos-enroll default, /var/lib/soos/models)
#   -u, --uid <UID>             Enrolled user UID to verify against (default: SUDO_UID, else current UID)
#   --mock                      Simulation only: PAD plumbing + real-model tests, NO security metrics
#   -h, --help                  Print this help message and exit
#
# The physical session verifies against an already enrolled template (enroll it first with
# `sudo soos-enroll enroll --uid <UID>`). soos-enroll requires root (EUID 0), so run the
# physical session with sudo after building the CLI as your user:
#   cargo build --release -p soos-enrollment-cli
#   sudo tests/physical/adversarial_test.sh [OPTIONS]
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
  -d, --device <PATH>         Camera device node (e.g. /dev/video0 or /dev/v4l/by-id/...)
  -b, --biometrics-dir <PATH> Provisioned template store (default: the soos-enroll default,
                              /var/lib/soos/biometrics)
  -k, --key-file <PATH>       Master key of that store (default: /var/lib/soos/master.key)
  -m, --models-dir <PATH>     Directory containing verified ONNX models and manifest.toml
                              (default: the soos-enroll default, /var/lib/soos/models)
  -u, --uid <UID>             Enrolled user UID to verify against (default: SUDO_UID, else current UID)
  --mock                      Simulation only: runs PAD plumbing and real-model tests,
                              reports NO security metrics (no APCER / BPCER)
  -h, --help                  Print this help message and exit

The physical session verifies against an already enrolled template: enroll the subject
first (sudo soos-enroll enroll --uid <UID>). soos-enroll requires root (EUID 0): build it
as your user, then run the physical session with sudo:
  cargo build --release -p soos-enrollment-cli
  sudo tests/physical/adversarial_test.sh [OPTIONS]
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
echo "  SOOS — Presentation Attack Detection (PAD) Adversarial Suite (#31.5)"
echo "==================================================================="
echo ""

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
# 2. Test Counters for Metrics Computation
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
    # soos-enroll checks for EUID 0 before it parses any argument (there is no bypass flag).
    if [[ "${EUID}" -ne 0 ]]; then
        error "soos-enroll requires root privileges (EUID 0)."
        error "Build it as your user, then re-run: sudo tests/physical/adversarial_test.sh [OPTIONS]"
        exit 1
    fi

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

    # Verify against the provisioned store; soos-enroll accepts only absolute paths.
    CLI_COMMON_FLAGS=()
    if [[ -n "${BIOMETRICS_DIR}" ]]; then
        BIOMETRICS_DIR="$(realpath -m -- "${BIOMETRICS_DIR}")"
        CLI_COMMON_FLAGS+=("--biometrics-dir" "${BIOMETRICS_DIR}")
    fi
    if [[ -n "${KEY_FILE}" ]]; then
        KEY_FILE="$(realpath -m -- "${KEY_FILE}")"
        CLI_COMMON_FLAGS+=("--key-file" "${KEY_FILE}")
    fi
    if [[ -n "${MODELS_DIR}" ]]; then
        MODELS_DIR="$(realpath -m -- "${MODELS_DIR}")"
        CLI_COMMON_FLAGS+=("--models-dir" "${MODELS_DIR}")
    fi
    if [[ -n "${CAMERA_DEVICE}" ]]; then
        CLI_COMMON_FLAGS+=("--camera-device" "${CAMERA_DEVICE}")
    fi

    # An unenrolled UID would make every presentation fail as "not enrolled", which would
    # read as a perfect APCER and a 100% BPCER. Refuse to measure anything in that case.
    ENROLLED="$("${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" list --format json)"
    if ! grep -q "\"uid\": ${TARGET_UID}," <<< "${ENROLLED}"; then
        error "UID ${TARGET_UID} has no enrolled template in the selected store."
        error "Enroll the subject first: sudo soos-enroll enroll --uid ${TARGET_UID}"
        exit 1
    fi

    # Returns 0 for Verdict::Allow and 1 for any other verdict. A CLI failure that prints no
    # verdict (camera, model or store error) aborts the session, so an operational failure is
    # never counted as a rejected attack.
    verify_presentation() {
        local output
        local status=0
        output="$("${SOOS_ENROLL_BIN}" "${CLI_COMMON_FLAGS[@]}" verify --uid "${TARGET_UID}" 2>&1)" || status=$?
        if [[ "${status}" -eq 0 ]]; then
            return 0
        fi
        if grep -q '^Verdict:' <<< "${output}"; then
            return 1
        fi
        error "soos-enroll verify failed without a verdict (exit ${status}); aborting the session:"
        echo "${output}" >&2
        exit 2
    }

    info "Beginning physical presentation attack testing protocol."
    info "Verifying against the enrolled template of UID ${TARGET_UID}."

    # Baseline: Bona Fide (Genuine) Presentation
    echo ""
    info "-------------------------------------------------------------------"
    info "Phase 1: Bona Fide (Genuine Live Subject) Evaluation"
    info "-------------------------------------------------------------------"
    echo -n "Please present the genuine enrolled live subject to the camera and press Enter... "
    read -r
    BONA_FIDE_TESTED=$(( BONA_FIDE_TESTED + 1 ))
    if verify_presentation; then
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
    if verify_presentation; then
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
    if verify_presentation; then
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
    if verify_presentation; then
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
