#!/usr/bin/env bash
# =============================================================================
# tests/docker/test_suite.sh — Distribution-Agnostic In-Container PAM Test Suite
# =============================================================================
# Executes the full PAM test matrix inside any supported distribution container:
#   - T1: Nominal facial auth (daemon Allow -> PAM_SUCCESS, 0 password prompts)
#   - T2: Daemon slower than the stack's timeout_ms (PAM_IGNORE -> password
#         fallback succeeds within timeout_ms + tolerance; the elapsed time is asserted)
#   - T2b: Same slow daemon, no password: the late Allow never authenticates
#   - T3: Daemon slower than timeout_ms (wrong password rejected)
#   - T4: Daemon crash mid-request (PAM_IGNORE -> password fallback succeeds)
#   - T5: Daemon crash mid-request (wrong password rejected)
#   - T6: Distribution stack integration (common-auth or system-auth): facial
#         Allow with 0 prompts, password fallback, wrong password rejected
#   - T7: Offline daemon (PAM_IGNORE -> password fallback)
#   - T8: Absent module resilience (password accepted, wrong password and
#         password-less runs rejected)
#   - T9: Model deployment script integrity (manifest dry-run)
#   - T10: Panic inside the RELEASE-built .so returns PAM_IGNORE (never aborts
#          the PAM host process) — review finding PAM-01 / TCI-01 (GitHub #148)
#   - T11: pam-auth-update generated common-auth emits one PasswordFailed event
#          on a wrong password and none on success — review finding ONB-03 (GitHub #161)
#   - T12: /etc/soos/gdm.disable disables a `gdm-password` line without a
#          service= argument (PAM_SERVICE item) — review PAM-05 (GitHub #176)
#   - T13: Verdict::Deny -> PAM_IGNORE -> password fallback (ARCHITECTURE §3)
#   - T14: Truncated response body -> PAM_IGNORE -> password fallback
#   - T15: Malformed responses (undecodable verdict, mismatched request_id,
#          unsupported version, oversized and empty frames; three carry Allow)
#          -> never an authorization, password fallback intact
# T2, T6, T8 and T13-T15 are review finding TCI-06 (GitHub #189): every case
# exits non-zero on a failed expectation, none downgrades it to a warning.
# =============================================================================

set -euo pipefail

# Shared assertions (deadline bound, no-facial-authorization, password fallback).
# shellcheck source=tests/docker/pam_case_lib.sh
source "tests/docker/pam_case_lib.sh"

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
# 1b. Validate the Image's PAM Stack Files (GitHub #162)
# ---------------------------------------------------------------------------
# A stack written with `echo "...\n..."` under bash (the /bin/sh of fedora and
# arch) is a single comment line: every case below would then test nothing.
assert_pam_stack_file() {
    local file="$1"
    if [[ ! -f "${file}" ]]; then
        error "PAM stack file ${file} is missing."
        exit 1
    fi
    if grep -qF '\n' "${file}"; then
        error "PAM stack file ${file} contains a literal backslash-n (single-line file):"
        cat "${file}" >&2
        exit 1
    fi
    local modules
    modules="$(grep -cE '^[[:space:]]*-?(auth|account|password|session)[[:space:]]+' "${file}" || true)"
    if [[ "${modules}" -lt 2 ]]; then
        error "PAM stack file ${file} has ${modules} module line(s); expected at least 2."
        exit 1
    fi
    if ! grep -qE '^[[:space:]]*auth[[:space:]].*pam_soos\.so' "${file}" \
        || ! grep -qE '^[[:space:]]*auth[[:space:]].*pam_unix\.so' "${file}"; then
        error "PAM stack file ${file} must contain separate pam_soos.so and pam_unix.so auth lines."
        exit 1
    fi
    success "PAM stack file ${file} is valid (${modules} module lines)."
}

assert_pam_stack_file /etc/pam.d/test-soos
for stack in /etc/pam.d/common-auth /etc/pam.d/system-auth; do
    if [[ -f "${stack}" ]] && grep -q 'pam_soos.so' "${stack}"; then
        assert_pam_stack_file "${stack}"
    fi
done

# ---------------------------------------------------------------------------
# 1c. Synchronize the Rust Toolchain With rust-toolchain.toml (GitHub #244)
# ---------------------------------------------------------------------------
# The image installs `stable` when it is built, and the CI layer cache can keep
# that image (and its toolchain) for a long time. `rustup toolchain install`
# without arguments reads /workspace/rust-toolchain.toml and brings the channel
# up to the release the other CI jobs install, so this suite never builds with a
# stale compiler. SOOS_REQUIRE_TOOLCHAIN_SYNC=1 (set by CI) makes a failed sync
# fatal; a local offline run warns and continues with the image toolchain.
info "Synchronizing the Rust toolchain with rust-toolchain.toml..."
if rustup toolchain install --profile minimal; then
    success "Rust toolchain synchronized with rust-toolchain.toml."
elif [[ "${SOOS_REQUIRE_TOOLCHAIN_SYNC:-0}" == "1" ]]; then
    error "Could not synchronize the Rust toolchain with rust-toolchain.toml."
    exit 1
else
    warn "Could not synchronize the Rust toolchain (offline?); using the image toolchain."
fi
info "Toolchain in use: $(rustc --version)"

# ---------------------------------------------------------------------------
# 2. Build pam_soos.so
# ---------------------------------------------------------------------------
# Always invoke cargo (a no-op when up to date) so a stale artifact built by
# another distribution image is never deployed. run_matrix.sh overlays
# /workspace/target with a per-distribution Docker volume.
SO_PATH="target/release/libpam_soos.so"
info "Compiling pam_soos in release mode..."
cargo build --locked --release -p soos-pam

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

# Socket directory and group mirror the production invariant (GitHub #168):
# /run/soos is 0750 root:soos and the daemon socket is 0660 root:soos. The PAM
# host (pam_test_runner) runs as root, like sudo/login/gdm in production.
getent group soos >/dev/null 2>&1 || groupadd -r soos
install -d -m 0750 -o root -g soos /run/soos

# Asserts the socket directory and socket modes after the mock daemon started.
assert_socket_modes() {
    local dir_modes sock_modes
    dir_modes="$(stat -c '%a %U:%G' /run/soos)"
    if [[ "${dir_modes}" != "750 root:soos" ]]; then
        error "/run/soos is '${dir_modes}', expected '750 root:soos'."
        exit 1
    fi
    if [[ ! -S /run/soos/daemon.sock ]]; then
        error "/run/soos/daemon.sock is not a socket."
        exit 1
    fi
    sock_modes="$(stat -c '%a %U:%G' /run/soos/daemon.sock)"
    if [[ "${sock_modes}" != "660 root:soos" ]]; then
        error "/run/soos/daemon.sock is '${sock_modes}', expected '660 root:soos'."
        exit 1
    fi
}

# Starts the mock daemon in the given mode and waits (bounded) for its socket.
start_mock_daemon() {
    python3 tests/docker/mock_daemon.py --socket /run/soos/daemon.sock "$@" &
    MOCK_PID=$!
    for _ in $(seq 1 50); do
        [[ -S /run/soos/daemon.sock ]] && break
        sleep 0.1
    done
    assert_socket_modes
}

cleanup_daemon() {
    # Stop the mock by PID first: minimal images (fedora:40) ship without pkill.
    if [[ -n "${MOCK_PID:-}" ]]; then
        kill "${MOCK_PID}" 2>/dev/null || true
        wait "${MOCK_PID}" 2>/dev/null || true
        MOCK_PID=""
    fi
    pkill -f "mock_daemon.py" 2>/dev/null || true
    rm -f /run/soos/daemon.sock
}

# T10 artifacts: fault-injection variant of the module and its dedicated PAM services.
FAULT_SO_PATH="target/fault-injection/release/libpam_soos.so"
FAULT_MODULE_NAME="pam_soos_fault.so"
cleanup_fault_injection() {
    rm -f "${PAM_MOD_DIR}/${FAULT_MODULE_NAME}"
    rm -f /etc/pam.d/test-soos-fault-panic /etc/pam.d/test-soos-fault-overflow
}

# T12 artifacts: GDM-named PAM service and the gdm.disable flag.
T12_SERVICE="gdm-password"
cleanup_gdm_disable() {
    rm -f /etc/soos/gdm.disable "/etc/pam.d/${T12_SERVICE}"
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
start_mock_daemon --mode allow

# pam_test_runner without password argument asserts non-interactive success (0 prompts)
if /usr/local/bin/pam_test_runner test-soos testuser; then
    success "T1 passed: Facial auth succeeded without password prompts (PAM_SUCCESS)."
else
    error "T1 failed: Facial auth did not succeed with active daemon."
    exit 1
fi
cleanup_daemon

# ---------------------------------------------------------------------------
# T2: Daemon Slower Than timeout_ms -> Fallback to Password (Sub-issue #13.2, PA2)
# ---------------------------------------------------------------------------
# The budget is read from the stack under test, never hard-coded: the mock delay
# always exceeds timeout_ms + tolerance, so a module that waits for the (late)
# daemon Allow instead of its own deadline fails the elapsed-time bound.
echo ""
info "-------------------------------------------------------------------"
info "T2: Daemon Slower Than timeout_ms — Degrades to Password Within the Deadline (PA2)"
info "-------------------------------------------------------------------"
cleanup_daemon
T2_TIMEOUT_MS="$(pam_stack_timeout_ms /etc/pam.d/test-soos)"
T2_DELAY_MS="$(timeout_mock_delay_ms "${T2_TIMEOUT_MS}")"
T2_DELAY_S="$(printf '%d.%03d' $((T2_DELAY_MS / 1000)) $((T2_DELAY_MS % 1000)))"
info "T2: test-soos timeout_ms=${T2_TIMEOUT_MS}; mock daemon answers Allow after ${T2_DELAY_MS} ms."
start_mock_daemon --mode timeout --delay "${T2_DELAY_S}"

T2_START_MS="$(now_ms)"
if /usr/local/bin/pam_test_runner test-soos testuser password123; then
    T2_ELAPSED_MS=$(( $(now_ms) - T2_START_MS ))
else
    error "T2 failed: Valid password was rejected during daemon timeout."
    exit 1
fi
if ! assert_elapsed_within_deadline "T2" "${T2_ELAPSED_MS}" "${T2_TIMEOUT_MS}"; then
    error "T2 failed: the timeout did not bound the PAM run."
    exit 1
fi
success "T2 passed: Timeout triggered PAM_IGNORE and password fallback succeeded in ${T2_ELAPSED_MS}ms."

# ---------------------------------------------------------------------------
# T2b: Late Allow Never Authenticates (no password, same slow daemon)
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "T2b: Daemon Slower Than timeout_ms — Late Allow Is Never Honored"
info "-------------------------------------------------------------------"
# pam_unix applies a ~2 s failure delay to a refused prompt; T2b runs a copy of
# test-soos whose pam_unix line carries `nodelay`, so the elapsed time of the
# failed run measures the module's deadline alone.
T2B_SERVICE="test-soos-nodelay"
sed -E 's/^([[:space:]]*auth[[:space:]].*pam_unix\.so)(.*)$/\1\2 nodelay/' \
    /etc/pam.d/test-soos > "/etc/pam.d/${T2B_SERVICE}"
T2B_START_MS="$(now_ms)"
if ! assert_no_facial_authorization test-soos-nodelay "T2b"; then
    error "T2b failed: a verdict delivered after the deadline authenticated the user!"
    exit 1
fi
T2B_ELAPSED_MS=$(( $(now_ms) - T2B_START_MS ))
if ! assert_elapsed_within_deadline "T2b" "${T2B_ELAPSED_MS}" "${T2_TIMEOUT_MS}"; then
    error "T2b failed: the timeout did not bound the password-less PAM run."
    exit 1
fi
rm -f "/etc/pam.d/${T2B_SERVICE}"
success "T2b passed: the late Allow was ignored (run took ${T2B_ELAPSED_MS}ms)."

# ---------------------------------------------------------------------------
# T3: Daemon Slower Than timeout_ms with Invalid Password (Rejected)
# ---------------------------------------------------------------------------
echo ""
info "-------------------------------------------------------------------"
info "T3: Daemon Slower Than timeout_ms — Invalid Password Must Fail"
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
start_mock_daemon --mode crash-immediate

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
start_mock_daemon --mode crash-partial

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
start_mock_daemon --mode crash-immediate

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
# Every supported image ships common-auth (Debian/Ubuntu) or system-auth
# (Fedora, Arch) with the pam_soos.so line; each expectation is a hard failure
# (GitHub #189: this case used to end in a warning and could never fail).
echo ""
info "-------------------------------------------------------------------"
info "T6: Distribution Stack Integration (common-auth / system-auth)"
info "-------------------------------------------------------------------"
DISTRO_SERVICE=""
for candidate in common-auth system-auth; do
    if [[ -f "/etc/pam.d/${candidate}" ]] && grep -q 'pam_soos.so' "/etc/pam.d/${candidate}"; then
        DISTRO_SERVICE="${candidate}"
        break
    fi
done
if [[ -z "${DISTRO_SERVICE}" ]]; then
    error "T6 failed: no common-auth or system-auth stack with pam_soos.so in this image."
    exit 1
fi
info "Testing native distro stack service: ${DISTRO_SERVICE}"

# Facial Allow on the distribution stack: PAM_SUCCESS with 0 password prompts.
cleanup_daemon
start_mock_daemon --mode allow
if /usr/local/bin/pam_test_runner "${DISTRO_SERVICE}" testuser; then
    success "T6 passed: Distro stack (${DISTRO_SERVICE}) authenticated via facial verification (0 prompts)."
else
    error "T6 failed: Distro stack (${DISTRO_SERVICE}) did not honor the daemon Allow without a password prompt."
    exit 1
fi
cleanup_daemon

# Daemon offline: the valid password is accepted, a wrong password is rejected.
if /usr/local/bin/pam_test_runner "${DISTRO_SERVICE}" testuser password123; then
    success "T6 passed: Distro stack (${DISTRO_SERVICE}) password fallback verified."
else
    error "T6 failed: Distro stack (${DISTRO_SERVICE}) rejected valid password."
    exit 1
fi
if /usr/local/bin/pam_test_runner "${DISTRO_SERVICE}" testuser wrong_password 2>/dev/null; then
    error "T6 failed: Distro stack (${DISTRO_SERVICE}) accepted a wrong password!"
    exit 1
fi
success "T6 passed: Distro stack (${DISTRO_SERVICE}) rejected a wrong password."

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
# With pam_soos.so missing from disk, libpam reports the line as an unknown
# module; `default=ignore` must skip it so the password stack keeps working,
# and nothing may authenticate without a password. Hard failures (GitHub #189).
echo ""
info "-------------------------------------------------------------------"
info "T8: Absent Module Resilience (PAM Stack Continues to Function)"
info "-------------------------------------------------------------------"
cleanup_daemon
restore_pam_module() {
    if [[ -f "${PAM_MOD_DIR}/pam_soos.so.bak" ]]; then
        mv "${PAM_MOD_DIR}/pam_soos.so.bak" "${PAM_MOD_DIR}/pam_soos.so"
    fi
}
mv "${PAM_MOD_DIR}/pam_soos.so" "${PAM_MOD_DIR}/pam_soos.so.bak"
# A daemon answering Allow proves the missing module is not bypassed somehow.
start_mock_daemon --mode allow

if /usr/local/bin/pam_test_runner test-soos testuser password123; then
    success "T8 passed: PAM stack remains fully functional with absent module."
else
    error "T8 failed: valid password rejected while pam_soos.so is absent."
    restore_pam_module
    exit 1
fi
if /usr/local/bin/pam_test_runner test-soos testuser wrong_password 2>/dev/null; then
    error "T8 failed: wrong password accepted while pam_soos.so is absent!"
    restore_pam_module
    exit 1
fi
success "T8 passed: wrong password rejected with absent module."
if ! assert_no_facial_authorization test-soos "T8"; then
    error "T8 failed: authenticated without a password while pam_soos.so is absent!"
    restore_pam_module
    exit 1
fi
cleanup_daemon
restore_pam_module

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
cargo build --locked --release -p soos-pam --features fault-injection --target-dir target/fault-injection
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

# ---------------------------------------------------------------------------
# T12: gdm.disable honored through the PAM_SERVICE item (PAM-05 / GitHub #176)
# ---------------------------------------------------------------------------
# The line installed by `soos-admin gdm enable` carries no `service=` argument.
# The module must read PAM_SERVICE ("gdm-password") so that the flag written by
# `soos-admin gdm disable` (/etc/soos/gdm.disable) really disables facial login.
echo ""
info "-------------------------------------------------------------------"
info "T12: /etc/soos/gdm.disable Disables the GDM Line via PAM_SERVICE"
info "-------------------------------------------------------------------"
cleanup_daemon
cleanup_gdm_disable
mkdir -p /etc/soos
cat > "/etc/pam.d/${T12_SERVICE}" <<EOF
# T12 PAM service: same arguments as the soos-admin GDM line (no service= argument)
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250
auth  required                       pam_unix.so
account required pam_unix.so
session required pam_unix.so
EOF
python3 tests/docker/mock_daemon.py --mode allow --socket /run/soos/daemon.sock &
MOCK_PID=$!
sleep 0.2

# Control: without the flag the Allow verdict authenticates with zero prompts.
if /usr/local/bin/pam_test_runner "${T12_SERVICE}" testuser; then
    success "T12 control passed: ${T12_SERVICE} authenticated facially without the flag."
else
    error "T12 control failed: ${T12_SERVICE} did not authenticate facially without the flag."
    exit 1
fi

touch /etc/soos/gdm.disable
# With the flag the module must return PAM_IGNORE: the password prompt is reached,
# so a run without password must fail even though the daemon answers Allow.
if /usr/local/bin/pam_test_runner "${T12_SERVICE}" testuser 2>/dev/null; then
    error "T12 failed: gdm.disable present but ${T12_SERVICE} still authenticated facially!"
    exit 1
fi
success "T12 passed: gdm.disable made ${T12_SERVICE} fall back to the password prompt."

if /usr/local/bin/pam_test_runner "${T12_SERVICE}" testuser password123; then
    success "T12 passed: valid password accepted while GDM facial login is disabled."
else
    error "T12 failed: valid password rejected while GDM facial login is disabled."
    exit 1
fi
if /usr/local/bin/pam_test_runner "${T12_SERVICE}" testuser wrong_password 2>/dev/null; then
    error "T12 failed: invalid password accepted while GDM facial login is disabled!"
    exit 1
fi
success "T12 passed: invalid password rejected while GDM facial login is disabled."
cleanup_daemon
cleanup_gdm_disable

# ---------------------------------------------------------------------------
# T13: Verdict::Deny -> PAM_IGNORE -> Password Fallback (ARCHITECTURE §3, #189)
# ---------------------------------------------------------------------------
# A Deny is a non-match, not a hard failure of the stack: the module returns
# PAM_IGNORE so the user can still type a password, and Deny never authenticates.
echo ""
info "-------------------------------------------------------------------"
info "T13: Verdict::Deny — No Facial Authorization, Password Fallback"
info "-------------------------------------------------------------------"
cleanup_daemon
start_mock_daemon --mode deny
if ! assert_no_facial_authorization test-soos "T13"; then
    error "T13 failed: Verdict::Deny authenticated the user!"
    exit 1
fi
if ! assert_password_fallback test-soos "T13"; then
    error "T13 failed: password fallback broken after Verdict::Deny."
    exit 1
fi
success "T13 passed: Verdict::Deny degraded to PAM_IGNORE and the password stack."
cleanup_daemon

# ---------------------------------------------------------------------------
# T14: Truncated Response Body -> PAM_IGNORE -> Password Fallback (#189)
# ---------------------------------------------------------------------------
# The daemon declares a 37-byte response, sends 4 bytes and closes the stream.
echo ""
info "-------------------------------------------------------------------"
info "T14: Truncated Response — No Facial Authorization, Password Fallback"
info "-------------------------------------------------------------------"
cleanup_daemon
start_mock_daemon --mode crash-truncated
if ! assert_no_facial_authorization test-soos "T14"; then
    error "T14 failed: a truncated response authenticated the user!"
    exit 1
fi
if ! assert_password_fallback test-soos "T14"; then
    error "T14 failed: password fallback broken after a truncated response."
    exit 1
fi
success "T14 passed: truncated response degraded to PAM_IGNORE and the password stack."
cleanup_daemon

# ---------------------------------------------------------------------------
# T15: Malformed Responses Are Never an Authorization (#189)
# ---------------------------------------------------------------------------
# Each mock mode puts one defect on the wire (tests/docker/mock_daemon.py):
#   malformed         complete frame, undecodable verdict discriminant
#   wrong-request-id  complete Allow frame answering another request
#   bad-version       complete Allow frame with protocol version 2
#   oversized         length prefix above MAX_MESSAGE_SIZE
#   empty             zero-length frame
echo ""
info "-------------------------------------------------------------------"
info "T15: Malformed Responses — No Facial Authorization, Password Fallback"
info "-------------------------------------------------------------------"
for T15_MODE in malformed wrong-request-id bad-version oversized empty; do
    cleanup_daemon
    start_mock_daemon --mode "${T15_MODE}"
    if ! assert_no_facial_authorization test-soos "T15 (${T15_MODE})"; then
        error "T15 (${T15_MODE}) failed: a malformed response authenticated the user!"
        exit 1
    fi
    if ! /usr/local/bin/pam_test_runner test-soos testuser password123; then
        error "T15 (${T15_MODE}) failed: valid password rejected after a malformed response."
        exit 1
    fi
    success "T15 (${T15_MODE}) passed: rejected as an authorization, password fallback intact."
done
# One wrong-password run is enough to prove the fallback still fails closed after
# a malformed Allow (each such run costs the ~2 s pam_unix failure delay).
cleanup_daemon
start_mock_daemon --mode wrong-request-id
if ! assert_password_fallback test-soos "T15 (wrong-request-id)"; then
    error "T15 failed: password fallback broken after a mismatched request_id Allow."
    exit 1
fi
cleanup_daemon

echo ""
echo "==================================================================="
success "  ALL IN-CONTAINER PAM MATRIX TESTS PASSED SUCCESSFULLY!"
echo "==================================================================="
echo ""
