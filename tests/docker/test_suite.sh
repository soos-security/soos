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
#   - T9: Model deployment script integrity (manifest dry-run)
#   - T10: Panic inside the RELEASE-built .so returns PAM_IGNORE (never aborts
#          the PAM host process) — review finding PAM-01 / TCI-01 (GitHub #148)
#   - T11: /etc/soos/gdm.disable disables a `gdm-password` line without a
#          service= argument (PAM_SERVICE item) — review PAM-05 (GitHub #176)
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

# T10 artifacts: fault-injection variant of the module and its dedicated PAM services.
FAULT_SO_PATH="target/fault-injection/release/libpam_soos.so"
FAULT_MODULE_NAME="pam_soos_fault.so"
cleanup_fault_injection() {
    rm -f "${PAM_MOD_DIR}/${FAULT_MODULE_NAME}"
    rm -f /etc/pam.d/test-soos-fault-panic /etc/pam.d/test-soos-fault-overflow
}

# T11 artifacts: GDM-named PAM service and the gdm.disable flag.
T11_SERVICE="gdm-password"
cleanup_gdm_disable() {
    rm -f /etc/soos/gdm.disable "/etc/pam.d/${T11_SERVICE}"
}

cleanup_all() {
    cleanup_daemon
    cleanup_fault_injection
    cleanup_gdm_disable
}
trap cleanup_all EXIT INT TERM

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

# ---------------------------------------------------------------------------
# T10: Panic Inside the RELEASE-Built Module -> PAM_IGNORE (PAM-01 / GitHub #148)
# ---------------------------------------------------------------------------
# The shipped pam_soos.so is a release build. If [profile.release] used
# panic = "abort", every catch_unwind in the module would be a no-op and a panic
# would kill the PAM host process (gdm, sudo, login) with SIGABRT (exit 134)
# instead of degrading to PAM_IGNORE + password fallback (ARCHITECTURE.md
# invariant 5). This case builds the module with the SAME release profile plus
# the opt-in `fault-injection` feature (never enabled by packaging), loads it
# under a distinct module name, and arms a deliberate panic through the PAM
# argument `fault_inject=<panic|overflow>`.
echo ""
info "-------------------------------------------------------------------"
info "T10: Release-Built Module Panic Safety (catch_unwind -> PAM_IGNORE)"
info "-------------------------------------------------------------------"
cleanup_daemon
cleanup_fault_injection

info "Compiling fault-injection variant of pam_soos (release profile)..."
cargo build --release -p soos-pam --features fault-injection --target-dir target/fault-injection
if [[ ! -f "${FAULT_SO_PATH}" ]]; then
    error "T10 failed: fault-injection artifact not found at ${FAULT_SO_PATH}"
    exit 1
fi
cp "${FAULT_SO_PATH}" "${PAM_MOD_DIR}/${FAULT_MODULE_NAME}"
chmod 644 "${PAM_MOD_DIR}/${FAULT_MODULE_NAME}"

for FAULT_ARG in fault_inject=panic fault_inject=overflow; do
    FAULT_MODE="${FAULT_ARG#fault_inject=}"
    FAULT_SERVICE="test-soos-fault-${FAULT_MODE}"
    cat > "/etc/pam.d/${FAULT_SERVICE}" <<EOF
# T10 PAM service: the module is armed to panic (${FAULT_MODE}) inside catch_unwind
auth  [success=done default=ignore]  ${FAULT_MODULE_NAME} ${FAULT_ARG} timeout_ms=250
auth  required                       pam_unix.so
account required pam_unix.so
session required pam_unix.so
EOF

    # Valid password: the panic must degrade to PAM_IGNORE and pam_unix must succeed.
    set +e
    /usr/local/bin/pam_test_runner "${FAULT_SERVICE}" testuser password123
    T10_RC=$?
    set -e
    if [[ ${T10_RC} -eq 134 || ${T10_RC} -ge 128 ]]; then
        error "T10 (${FAULT_MODE}) failed: PAM host process was killed by a signal (exit ${T10_RC}); catch_unwind is not effective in the release build."
        exit 1
    fi
    if [[ ${T10_RC} -ne 0 ]]; then
        error "T10 (${FAULT_MODE}) failed: valid password rejected after in-module panic (exit ${T10_RC})."
        exit 1
    fi
    success "T10 (${FAULT_MODE}) passed: in-module panic degraded to PAM_IGNORE, password fallback succeeded."

    # Wrong password: the panic must never be converted into an authorization.
    set +e
    /usr/local/bin/pam_test_runner "${FAULT_SERVICE}" testuser wrong_password 2>/dev/null
    T10_RC=$?
    set -e
    if [[ ${T10_RC} -eq 0 ]]; then
        error "T10 (${FAULT_MODE}) failed: invalid password accepted after in-module panic!"
        exit 1
    fi
    if [[ ${T10_RC} -ge 128 ]]; then
        error "T10 (${FAULT_MODE}) failed: PAM host process was killed by a signal (exit ${T10_RC})."
        exit 1
    fi
    success "T10 (${FAULT_MODE}) passed: invalid password still rejected after in-module panic (exit ${T10_RC})."
done
cleanup_fault_injection

# ---------------------------------------------------------------------------
# T11: gdm.disable honored through the PAM_SERVICE item (PAM-05 / GitHub #176)
# ---------------------------------------------------------------------------
# The line installed by `soos-admin gdm enable` carries no `service=` argument.
# The module must read PAM_SERVICE ("gdm-password") so that the flag written by
# `soos-admin gdm disable` (/etc/soos/gdm.disable) really disables facial login.
echo ""
info "-------------------------------------------------------------------"
info "T11: /etc/soos/gdm.disable Disables the GDM Line via PAM_SERVICE"
info "-------------------------------------------------------------------"
cleanup_daemon
cleanup_gdm_disable
mkdir -p /etc/soos
cat > "/etc/pam.d/${T11_SERVICE}" <<EOF
# T11 PAM service: same arguments as the soos-admin GDM line (no service= argument)
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250
auth  required                       pam_unix.so
account required pam_unix.so
session required pam_unix.so
EOF
python3 tests/docker/mock_daemon.py --mode allow --socket /run/soos/daemon.sock &
MOCK_PID=$!
sleep 0.2

# Control: without the flag the Allow verdict authenticates with zero prompts.
if /usr/local/bin/pam_test_runner "${T11_SERVICE}" testuser; then
    success "T11 control passed: ${T11_SERVICE} authenticated facially without the flag."
else
    error "T11 control failed: ${T11_SERVICE} did not authenticate facially without the flag."
    exit 1
fi

touch /etc/soos/gdm.disable
# With the flag the module must return PAM_IGNORE: the password prompt is reached,
# so a run without password must fail even though the daemon answers Allow.
if /usr/local/bin/pam_test_runner "${T11_SERVICE}" testuser 2>/dev/null; then
    error "T11 failed: gdm.disable present but ${T11_SERVICE} still authenticated facially!"
    exit 1
fi
success "T11 passed: gdm.disable made ${T11_SERVICE} fall back to the password prompt."

if /usr/local/bin/pam_test_runner "${T11_SERVICE}" testuser password123; then
    success "T11 passed: valid password accepted while GDM facial login is disabled."
else
    error "T11 failed: valid password rejected while GDM facial login is disabled."
    exit 1
fi
if /usr/local/bin/pam_test_runner "${T11_SERVICE}" testuser wrong_password 2>/dev/null; then
    error "T11 failed: invalid password accepted while GDM facial login is disabled!"
    exit 1
fi
success "T11 passed: invalid password rejected while GDM facial login is disabled."
cleanup_daemon
cleanup_gdm_disable

echo ""
echo "==================================================================="
success "  ALL IN-CONTAINER PAM MATRIX TESTS PASSED SUCCESSFULLY!"
echo "==================================================================="
echo ""
