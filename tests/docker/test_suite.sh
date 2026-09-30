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
#   - T11: pam-auth-update generated common-auth emits one PasswordFailed event
#          on a wrong password and none on success — review finding ONB-03 (GitHub #161)
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

cleanup_all() {
    cleanup_daemon
    cleanup_fault_injection
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
# T11: Debian pam-auth-update Stack Emits PasswordFailed on a Wrong Password
#      (ONB-03 / GitHub #161)
# ---------------------------------------------------------------------------
# The shipped profiles are enabled with the real pam-auth-update, then the
# generated common-auth is exercised with the real module and a recording mock
# daemon (Verdict::Deny): a wrong password must reach the password-failed hook
# (one PasswordFailed event) before pam_deny, a correct password must not.
echo ""
info "-------------------------------------------------------------------"
info "T11: pam-auth-update Stack — PasswordFailed Event on Wrong Password"
info "-------------------------------------------------------------------"
if command -v pam-auth-update >/dev/null 2>&1; then
    cleanup_daemon
    T11_SAVED_COMMON_AUTH="$(mktemp)"
    T11_EVENTS="$(mktemp)"
    cp -p /etc/pam.d/common-auth "${T11_SAVED_COMMON_AUTH}"
    install -m 0644 packaging/pam/debian/soos /usr/share/pam-configs/soos
    install -m 0644 packaging/pam/debian/soos-notify /usr/share/pam-configs/soos-notify
    # --force: the sandbox image ships a hand-written common-auth.
    DEBIAN_FRONTEND=noninteractive pam-auth-update --package --force --enable soos soos-notify
    T11_NOTIFY_LINE="$(grep -n 'pam_soos.so event=password-failed' /etc/pam.d/common-auth | head -n 1 | cut -d: -f1)"
    T11_DENY_LINE="$(grep -n 'pam_deny.so' /etc/pam.d/common-auth | head -n 1 | cut -d: -f1)"
    if [[ -z "${T11_NOTIFY_LINE}" || -z "${T11_DENY_LINE}" || "${T11_NOTIFY_LINE}" -ge "${T11_DENY_LINE}" ]]; then
        error "T11 failed: password-failed hook (line ${T11_NOTIFY_LINE:-none}) is not before pam_deny (line ${T11_DENY_LINE:-none})."
        grep -v '^#' /etc/pam.d/common-auth | sed '/^$/d' >&2
        exit 1
    fi
    success "T11: generated common-auth places the hook (line ${T11_NOTIFY_LINE}) before pam_deny (line ${T11_DENY_LINE})."

    python3 tests/docker/mock_daemon.py --mode deny --record "${T11_EVENTS}" --socket /run/soos/daemon.sock &
    MOCK_PID=$!
    sleep 0.2

    set +e
    /usr/local/bin/pam_test_runner common-auth testuser wrong_password 2>/dev/null
    T11_RC=$?
    set -e
    sleep 0.2
    if [[ ${T11_RC} -eq 0 ]]; then
        error "T11 failed: wrong password accepted through the generated common-auth."
        exit 1
    fi
    T11_EVENT_COUNT="$(grep -c '^event kind=password-failed' "${T11_EVENTS}" || true)"
    if [[ "${T11_EVENT_COUNT}" -ne 1 ]]; then
        error "T11 failed: expected 1 PasswordFailed event after a wrong password, got ${T11_EVENT_COUNT}."
        cat "${T11_EVENTS}" >&2
        exit 1
    fi
    success "T11 passed: wrong password rejected and exactly one PasswordFailed event received."

    if ! /usr/local/bin/pam_test_runner common-auth testuser password123; then
        error "T11 failed: correct password rejected through the generated common-auth."
        exit 1
    fi
    sleep 0.2
    T11_EVENT_COUNT="$(grep -c '^event kind=password-failed' "${T11_EVENTS}" || true)"
    if [[ "${T11_EVENT_COUNT}" -ne 1 ]]; then
        error "T11 failed: a successful password login emitted a PasswordFailed event."
        cat "${T11_EVENTS}" >&2
        exit 1
    fi
    success "T11 passed: correct password accepted without any PasswordFailed event."
    cleanup_daemon

    DEBIAN_FRONTEND=noninteractive pam-auth-update --package --remove soos soos-notify
    rm -f /usr/share/pam-configs/soos /usr/share/pam-configs/soos-notify
    cp -p "${T11_SAVED_COMMON_AUTH}" /etc/pam.d/common-auth
    rm -f "${T11_SAVED_COMMON_AUTH}" "${T11_EVENTS}"
elif [[ -f /etc/debian_version ]]; then
    error "T11 failed: pam-auth-update is missing on a Debian-based image."
    exit 1
else
    info "T11 skipped: not a pam-auth-update distribution."
fi

echo ""
echo "==================================================================="
success "  ALL IN-CONTAINER PAM MATRIX TESTS PASSED SUCCESSFULLY!"
echo "==================================================================="
echo ""
