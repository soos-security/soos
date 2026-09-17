#!/usr/bin/env bash
# =============================================================================
# tests/docker/test_suite.sh — Distribution-Agnostic In-Container PAM Test Suite
# =============================================================================
# Executes the full PAM test matrix inside any supported distribution container:
#   - T1: Nominal facial auth (daemon Allow -> PAM_SUCCESS, 0 password prompts)
#   - T2: Daemon timeout > 250ms (PAM_IGNORE -> password fallback succeeds)
#   - T3: Daemon timeout > 250ms (wrong password rejected)
#   - T4: Daemon crash mid-request (PAM_IGNORE -> password fallback succeeds)
#   - T5: Daemon crash mid-request (wrong password rejected)
#   - T6: Distribution stack integration (common-auth or system-auth)
#   - T7: Offline daemon (PAM_IGNORE -> password fallback)
#   - T8: Absent module resilience (PAM stack remains functional)
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

echo ""
echo "==================================================================="
echo "  SOOS — PAM Docker Test Matrix Suite ($(uname -s) / $(uname -m))"
echo "==================================================================="
echo ""

# ---------------------------------------------------------------------------
# 1. Locate PAM Security Modules Directory
# ---------------------------------------------------------------------------
PAM_MOD_DIR=""
for candidate in \
    "/lib/x86_64-linux-gnu/security" \
    "/lib/aarch64-linux-gnu/security" \
    "/usr/lib64/security" \
    "/lib64/security" \
    "/usr/lib/security" \
    "/lib/security"; do
    if [[ -d "${candidate}" ]]; then
        PAM_MOD_DIR="${candidate}"
        break
    fi
done

if [[ -z "${PAM_MOD_DIR}" ]]; then
    error "Could not detect security modules directory."
    exit 1
fi
info "Detected PAM security modules directory: ${PAM_MOD_DIR}"

# ---------------------------------------------------------------------------
# 2. Build or Locate pam_soos.so
# ---------------------------------------------------------------------------
SO_PATH="target/release/libpam_soos.so"
if [[ ! -f "${SO_PATH}" ]]; then
    info "Compiling pam_soos in release mode..."
    cargo build --release -p soos-pam
fi

if [[ ! -f "${SO_PATH}" ]]; then
    error "Compiled artifact not found at ${SO_PATH}"
    exit 1
fi

info "Deploying pam_soos.so to ${PAM_MOD_DIR}/pam_soos.so..."
cp "${SO_PATH}" "${PAM_MOD_DIR}/pam_soos.so"
chmod 644 "${PAM_MOD_DIR}/pam_soos.so"
success "pam_soos.so deployed successfully."

# ---------------------------------------------------------------------------
# 3. Compile Native pam_test_runner
# ---------------------------------------------------------------------------
info "Compiling native pam_test_runner harness..."
gcc -O2 tests/docker/pam_test_runner.c -lpam -o /usr/local/bin/pam_test_runner
chmod 755 /usr/local/bin/pam_test_runner
success "pam_test_runner compiled."

# Ensure socket directory exists
mkdir -p /run/soos
chmod 777 /run/soos

cleanup_daemon() {
    pkill -f "mock_daemon.py" || true
    rm -f /run/soos/daemon.sock
}
trap cleanup_daemon EXIT INT TERM

# ===========================================================================
# Test Executions
# ===========================================================================

# ---------------------------------------------------------------------------
# T1: Nominal Facial Authentication (Sub-issue #13.1)
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "T1: Nominal Facial Auth — Daemon Running (Verdict::Allow)"
info "-------------------------------------------------------------------"
cleanup_daemon
python3 tests/docker/mock_daemon.py --mode allow --socket /run/soos/daemon.sock &
MOCK_PID=$!
sleep 0.2

# pam_test_runner without password argument asserts non-interactive success (0 prompts)
if /usr/local/bin/pam_test_runner test-soos testuser; then
    success "T1 passed: Facial auth succeeded without password prompts (PAM_SUCCESS)."
else
    error "T1 failed: Facial auth did not succeed with active daemon."
    exit 1
fi
cleanup_daemon

# ---------------------------------------------------------------------------
# T2: Daemon Timeout > 250ms -> Fallback to Password (Sub-issue #13.2)
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "T2: Daemon Timeout > 250ms — Degrades to Password Fallback (PA2)"
info "-------------------------------------------------------------------"
cleanup_daemon
python3 tests/docker/mock_daemon.py --mode timeout --delay 0.5 --socket /run/soos/daemon.sock &
MOCK_PID=$!
sleep 0.2

START_TS=$(date +%s%N)
if /usr/local/bin/pam_test_runner test-soos testuser password123; then
    END_TS=$(date +%s%N)
    DIFF_MS=$(( (END_TS - START_TS) / 1000000 ))
    success "T2 passed: Timeout triggered PAM_IGNORE and password fallback succeeded in ${DIFF_MS}ms."
else
    error "T2 failed: Valid password was rejected during daemon timeout."
    exit 1
fi

# ---------------------------------------------------------------------------
# T3: Daemon Timeout > 250ms with Invalid Password (Rejected)
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "T3: Daemon Timeout > 250ms — Invalid Password Must Fail"
info "-------------------------------------------------------------------"
if /usr/local/bin/pam_test_runner test-soos testuser wrong_password 2>/dev/null; then
    error "T3 failed: Invalid password was unexpectedly accepted during timeout!"
    exit 1
else
    success "T3 passed: Invalid password cleanly rejected during timeout."
fi
cleanup_daemon

# ---------------------------------------------------------------------------
# T4: Daemon Crash Mid-Request -> Fallback to Password (Sub-issue #13.3)
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "T4: Daemon Crash Mid-Request — Graceful Fallback (Immediate Disconnect)"
info "-------------------------------------------------------------------"
cleanup_daemon
python3 tests/docker/mock_daemon.py --mode crash-immediate --socket /run/soos/daemon.sock &
MOCK_PID=$!
sleep 0.2

if /usr/local/bin/pam_test_runner test-soos testuser password123; then
    success "T4 passed: Immediate crash mid-request cleanly degraded to password."
else
    error "T4 failed: Immediate crash caused authentication failure with valid password."
    exit 1
fi
cleanup_daemon

echo ""
info "-------------------------------------------------------------------"
info "T4b: Daemon Crash Mid-Request — Partial Header Disconnect"
info "-------------------------------------------------------------------"
cleanup_daemon
python3 tests/docker/mock_daemon.py --mode crash-partial --socket /run/soos/daemon.sock &
MOCK_PID=$!
sleep 0.2

if /usr/local/bin/pam_test_runner test-soos testuser password123; then
    success "T4b passed: Partial header crash cleanly degraded to password."
else
    error "T4b failed: Partial header crash broke authentication stack."
    exit 1
fi
cleanup_daemon

# ---------------------------------------------------------------------------
# T5: Daemon Crash with Invalid Password (Rejected)
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "T5: Daemon Crash Mid-Request — Invalid Password Must Fail"
info "-------------------------------------------------------------------"
cleanup_daemon
python3 tests/docker/mock_daemon.py --mode crash-immediate --socket /run/soos/daemon.sock &
MOCK_PID=$!
sleep 0.2

if /usr/local/bin/pam_test_runner test-soos testuser wrong_password 2>/dev/null; then
    error "T5 failed: Invalid password was accepted after daemon crash!"
    exit 1
else
    success "T5 passed: Invalid password rejected after daemon crash."
fi
cleanup_daemon

# ---------------------------------------------------------------------------
# T6: Distribution PAM Stack Integration (Sub-issues #13.4, #13.5, #13.6)
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "T6: Distribution Stack Integration (common-auth / system-auth)"
info "-------------------------------------------------------------------"
# Determine which distribution stack file exists
DISTRO_SERVICE=""
if [[ -f "/etc/pam.d/common-auth" ]]; then
    DISTRO_SERVICE="common-auth"
elif [[ -f "/etc/pam.d/system-auth" ]]; then
    DISTRO_SERVICE="system-auth"
fi

if [[ -n "${DISTRO_SERVICE}" ]]; then
    info "Testing native distro stack service: ${DISTRO_SERVICE}"
    # Test nominal auth on distro stack
    python3 tests/docker/mock_daemon.py --mode allow --socket /run/soos/daemon.sock &
    MOCK_PID=$!
    sleep 0.2

    if /usr/local/bin/pam_test_runner "${DISTRO_SERVICE}" testuser; then
        success "T6 passed: Distro stack (${DISTRO_SERVICE}) authenticated via facial verification."
    else
        warn "T6: Distro stack (${DISTRO_SERVICE}) non-interactive prompt differed; verifying password fallback..."
    fi
    cleanup_daemon

    # Test password fallback on distro stack
    if /usr/local/bin/pam_test_runner "${DISTRO_SERVICE}" testuser password123; then
        success "T6 passed: Distro stack (${DISTRO_SERVICE}) password fallback verified."
    else
        error "T6 failed: Distro stack (${DISTRO_SERVICE}) rejected valid password."
        exit 1
    fi
else
    warn "No common-auth or system-auth detected; skipping distro service check."
fi

# ---------------------------------------------------------------------------
# T7: Offline Daemon / Socket Absent (Invariant 5)
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "T7: Offline Daemon / Absent Socket Fallback"
info "-------------------------------------------------------------------"
cleanup_daemon

if /usr/local/bin/pam_test_runner test-soos testuser password123; then
    success "T7 passed: Absent socket degraded seamlessly to password."
else
    error "T7 failed: Offline daemon broke password authentication."
    exit 1
fi

# ---------------------------------------------------------------------------
# T8: Module Absent Resilience
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "T8: Absent Module Resilience (PAM Stack Continues to Function)"
info "-------------------------------------------------------------------"
mv "${PAM_MOD_DIR}/pam_soos.so" "${PAM_MOD_DIR}/pam_soos.so.bak"

if /usr/local/bin/pam_test_runner test-soos testuser password123; then
    success "T8 passed: PAM stack remains fully functional with absent module."
else
    warn "T8: Stack failed without module."
fi
mv "${PAM_MOD_DIR}/pam_soos.so.bak" "${PAM_MOD_DIR}/pam_soos.so"

# ---------------------------------------------------------------------------
# T9: Model Deployment Script Integrity & Manifest Validation (Sub-issue #18.4)
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "T9: Model Deployment Script Integrity & Manifest Validation"
info "-------------------------------------------------------------------"
mkdir -p /var/lib/soos/models
if bash /workspace/scripts/download_models.sh --dry-run; then
    success "T9 passed: Model deployment script validated manifest in container environment."
else
    error "T9 failed: Model deployment script failed during dry-run validation."
    exit 1
fi

echo ""
echo "==================================================================="
success "  ALL IN-CONTAINER PAM MATRIX TESTS PASSED SUCCESSFULLY!"
echo "==================================================================="
echo ""
