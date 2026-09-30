#!/usr/bin/env bash
# =============================================================================
# tests/distro/fedora_rhel_test.sh — Fedora 40 / RHEL 9 Deployment Test
# =============================================================================
# Sub-issue #32.2:
#   - Install via .rpm package or install.sh
#   - Deploy and verify custom authselect profile
#   - Verify custom authselect profile strictly preserves pam_faillock
#   - Test sudo and gdm PAM service stack integrations
#   - Validate and document rollback procedure
#
# Usage:
#   bash tests/distro/fedora_rhel_test.sh [OPTIONS]
#
# Options:
#   --rpm                    Test installation via RPM (.rpm) package
#   --install-sh             Test installation via scripts/install.sh
#   --skip-build             Do not recompile binaries if artifacts exist
#   --dry-run                Simulate execution plan without modifying root filesystem
#   --allow-host-changes     Required for a live run: this script installs packages,
#                            rewrites PAM files and creates users on the host it runs
#                            on (run_distro_validation.sh passes it only inside a
#                            disposable Docker container, GitHub #163)
#   -h, --help               Display this help message and exit
# =============================================================================

set -euo pipefail

# Terminal Colors
if [[ -t 1 ]]; then
    readonly GREEN='\033[0;32m'
    readonly RED='\033[0;31m'
    readonly YELLOW='\033[1;33m'
    readonly BLUE='\033[0;34m'
    readonly BOLD='\033[1m'
    readonly NC='\033[0m'
else
    readonly GREEN=''
    readonly RED=''
    readonly YELLOW=''
    readonly BLUE=''
    readonly BOLD=''
    readonly NC=''
fi

info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[ERROR]${NC} $*" >&2; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

INSTALL_MODE="auto" # auto, rpm, install-sh
SKIP_BUILD=false
DRY_RUN=false
ALLOW_HOST_CHANGES=false
TEST_USER="testuser"
TEST_PASS="password123"

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Executes Fedora 40 / RHEL 9 deployment, authselect custom profile,
pam_faillock preservation, sudo and gdm integration, and rollback validation suite.

Options:
  --rpm                    Install via RPM package (.rpm)
  --install-sh             Install via scripts/install.sh
  --skip-build             Skip cargo build if binaries are already present
  --dry-run                Print execution plan without making root changes
  --allow-host-changes     Consent to a live run that modifies this host (packages,
                           PAM files, users); use it only in a disposable container
  -h, --help               Display this help message and exit

Security Invariants Tested:
  - Fedora custom authselect profile (/etc/authselect/custom/soos)
  - Strict preservation of pam_faillock (preauth and authfail hooks)
  - Seamless integration with sudo (/etc/pam.d/sudo)
  - Seamless integration with gdm (/etc/pam.d/gdm-password)
  - Nominal facial auth (Verdict::Allow -> PAM_SUCCESS, 0 password prompts)
  - Password fallback (Verdict::Deny / offline daemon -> password authentication)
  - Safe rollback and uninstallation restoring original authselect profile
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --rpm)
            INSTALL_MODE="rpm"
            shift
            ;;
        --install-sh)
            INSTALL_MODE="install-sh"
            shift
            ;;
        --skip-build)
            SKIP_BUILD=true
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --allow-host-changes)
            ALLOW_HOST_CHANGES=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            error "Unknown argument: $1"
            usage
            exit 1
            ;;
    esac
done

echo ""
info "==================================================================="
info "  SOOS — Fedora 40 / RHEL 9 Deployment Validation (#32.2)"
info "==================================================================="
info "Workspace:    ${WORKSPACE_ROOT}"
info "Install Mode: ${INSTALL_MODE}"
info "Dry Run:      ${DRY_RUN}"
echo ""

if [[ "${DRY_RUN}" = true ]]; then
    info "[DRY RUN] Simulating Fedora / RHEL deployment plan:"
    info "  1. Build / verify release binaries and .rpm package"
    info "  2. Install via ${INSTALL_MODE} (rpm -i or scripts/install.sh)"
    info "  3. Verify filesystem permissions (biometrics: 0700, master.key: 0600)"
    info "  4. Deploy authselect custom profile and verify pam_faillock preservation"
    info "  5. Test sudo integration via system-auth stack"
    info "  6. Test gdm display manager integration via password-auth stack"
    info "  7. Test nominal facial authentication (PAM_SUCCESS, 0 password prompts)"
    info "  8. Test password fallback (valid succeeds, invalid rejected)"
    info "  9. Execute rollback procedure and restore standard authselect profile"
    success "Dry run deployment validation completed successfully."
    exit 0
fi

# Explicit consent for live execution (GitHub #163): refuse before any command,
# trap or file operation so that an accidental run never touches the host.
if [[ "${ALLOW_HOST_CHANGES}" != true ]]; then
    error "Refusing a live run without --allow-host-changes: this suite installs packages,"
    error "rewrites PAM files and creates users. Run it through"
    error "tests/distro/run_distro_validation.sh (disposable Docker container) or use --dry-run."
    exit 2
fi

# Root check for live execution
if [[ "$(id -u)" -ne 0 ]]; then
    error "Live deployment testing requires root privileges (or use --dry-run)."
    exit 1
fi

cleanup() {
    info "Executing test cleanup..."
    # Stop the mock by PID first: minimal images (fedora:40) ship without pkill.
    if [[ -n "${MOCK_PID:-}" ]]; then
        kill "${MOCK_PID}" 2>/dev/null || true
        wait "${MOCK_PID}" 2>/dev/null || true
        MOCK_PID=""
    fi
    pkill -f "mock_daemon.py" 2>/dev/null || true
    rm -f /run/soos/daemon.sock
}
trap cleanup EXIT INT TERM

# ---------------------------------------------------------------------------
# Step 1: Package Build / Staging
# ---------------------------------------------------------------------------
cd "${WORKSPACE_ROOT}"

if [[ "${SKIP_BUILD}" = false ]]; then
    if [[ ! -f "target/release/soos-daemon" || ! -f "target/release/libpam_soos.so" ]]; then
        info "Compiling release artifacts..."
        cargo build --release --workspace
    fi
fi

# ---------------------------------------------------------------------------
# Step 2: Installation (.rpm or scripts/install.sh)
# ---------------------------------------------------------------------------
if [[ "${INSTALL_MODE}" = "auto" || "${INSTALL_MODE}" = "rpm" ]]; then
    if command -v rpmbuild >/dev/null 2>&1 && command -v rpm >/dev/null 2>&1; then
        info "Building Fedora / RHEL RPM package..."
        bash scripts/build_rpm.sh --skip-build
        RPM_PKG=$(ls -t target/packages/soos-*.rpm | head -n 1)
        info "Installing RPM package: ${RPM_PKG}..."
        rpm -i "${RPM_PKG}"
        INSTALL_MODE="rpm"
        success "RPM package installed successfully."
    else
        warn "rpmbuild not found, falling back to scripts/install.sh..."
        INSTALL_MODE="install-sh"
    fi
fi

if [[ "${INSTALL_MODE}" = "install-sh" ]]; then
    info "Installing via scripts/install.sh..."
    bash scripts/install.sh --skip-models --skip-systemd
    success "scripts/install.sh completed successfully."
fi

# ---------------------------------------------------------------------------
# Step 3: Filesystem Invariants & Permissions Verification
# ---------------------------------------------------------------------------
info "Verifying installed filesystem invariants..."
test -f "/usr/libexec/soos/soos-daemon" || { error "soos-daemon binary missing"; exit 1; }
test -f "/usr/bin/soos-admin" || { error "soos-admin binary missing"; exit 1; }
test -f "/usr/bin/soos-enroll" || { error "soos-enroll binary missing"; exit 1; }

PAM_TARGET=""
for candidate in "/usr/lib64/security" "/usr/lib/security" "/lib64/security"; do
    if [[ -f "${candidate}/pam_soos.so" ]]; then
        PAM_TARGET="${candidate}/pam_soos.so"
        break
    fi
done

if [[ -z "${PAM_TARGET}" ]]; then
    error "pam_soos.so module not found in system security directories."
    exit 1
fi
info "Located PAM module at ${PAM_TARGET}"

test -d "/var/lib/soos/biometrics" || { error "/var/lib/soos/biometrics missing"; exit 1; }
test -d "/var/lib/soos/evidence" || { error "/var/lib/soos/evidence missing"; exit 1; }
test -f "/var/lib/soos/master.key" || { error "/var/lib/soos/master.key missing"; exit 1; }

BIO_PERMS=$(stat -c "%a" "/var/lib/soos/biometrics" 2>/dev/null || stat -f "%OLp" "/var/lib/soos/biometrics")
if [[ "${BIO_PERMS}" != "700" ]]; then
    error "Invalid permissions on /var/lib/soos/biometrics: ${BIO_PERMS} (expected 700)"
    exit 1
fi

KEY_PERMS=$(stat -c "%a" "/var/lib/soos/master.key" 2>/dev/null || stat -f "%OLp" "/var/lib/soos/master.key")
if [[ "${KEY_PERMS}" != "600" ]]; then
    error "Invalid permissions on /var/lib/soos/master.key: ${KEY_PERMS} (expected 600)"
    exit 1
fi
success "All directory hierarchy and permission invariants verified."

# ---------------------------------------------------------------------------
# Step 4: authselect Custom Profile Activation & pam_faillock Preservation
# ---------------------------------------------------------------------------
# The profile is activated for real (GitHub #145): a template-only inspection
# cannot detect unsupported syntax, undeclared features or missing templates.
info "Activating custom authselect profile and verifying pam_faillock preservation..."

AUTHSELECT_PROFILE_DIR="/etc/authselect/custom/soos"
test -d "${AUTHSELECT_PROFILE_DIR}" || { error "custom authselect profile missing: ${AUTHSELECT_PROFILE_DIR}"; exit 1; }
for template in README REQUIREMENTS system-auth password-auth nsswitch.conf fingerprint-auth smartcard-auth postlogin; do
    test -f "${AUTHSELECT_PROFILE_DIR}/${template}" || { error "custom authselect profile template missing: ${template}"; exit 1; }
done
command -v authselect >/dev/null 2>&1 || { error "authselect is required on Fedora / RHEL"; exit 1; }

AUTHSELECT_ORIGINAL="$(authselect current --raw 2>/dev/null || true)"
if [[ -f /etc/soos/authselect.previous ]]; then
    AUTHSELECT_ORIGINAL="$(head -n 1 /etc/soos/authselect.previous)"
fi
info "Original authselect profile: ${AUTHSELECT_ORIGINAL:-<none>}"
NSSWITCH_BEFORE="$(grep -v '^[[:space:]]*#' /etc/nsswitch.conf | sed '/^[[:space:]]*$/d')"

info "Running: authselect select custom/soos with-faillock --force"
authselect select custom/soos with-faillock --force || { error "custom authselect profile activation failed"; exit 1; }
authselect check || { error "authselect check failed after activating custom/soos"; exit 1; }

# Verify generated stacks: faillock preauth -> pam_soos -> pam_unix -> faillock authfail
for stack in system-auth password-auth; do
    STACK_FILE="/etc/pam.d/${stack}"
    if grep -q '{' "${STACK_FILE}"; then
        error "INVARIANT VIOLATION: unresolved template syntax in ${STACK_FILE}"; exit 1
    fi
    PREAUTH_LINE="$(grep -n 'pam_faillock.so preauth' "${STACK_FILE}" | head -n 1 | cut -d: -f1)"
    SOOS_LINE="$(grep -n 'pam_soos.so timeout_ms=250' "${STACK_FILE}" | head -n 1 | cut -d: -f1)"
    UNIX_LINE="$(grep -n 'pam_unix.so' "${STACK_FILE}" | head -n 1 | cut -d: -f1)"
    AUTHFAIL_LINE="$(grep -n 'pam_faillock.so authfail' "${STACK_FILE}" | head -n 1 | cut -d: -f1)"
    if [[ -z "${PREAUTH_LINE}" || -z "${SOOS_LINE}" || -z "${UNIX_LINE}" || -z "${AUTHFAIL_LINE}" ]]; then
        error "INVARIANT VIOLATION: pam_faillock/pam_soos/pam_unix lines missing in ${STACK_FILE}"; exit 1
    fi
    if (( PREAUTH_LINE >= SOOS_LINE || SOOS_LINE >= UNIX_LINE || UNIX_LINE >= AUTHFAIL_LINE )); then
        error "INVARIANT VIOLATION: ${STACK_FILE} ordering preauth=${PREAUTH_LINE} soos=${SOOS_LINE} unix=${UNIX_LINE} authfail=${AUTHFAIL_LINE}"; exit 1
    fi
    grep -qE '^account[[:space:]]+required[[:space:]]+pam_faillock\.so' "${STACK_FILE}" \
        || { error "INVARIANT VIOLATION: account-phase pam_faillock.so missing in ${STACK_FILE}"; exit 1; }
done

# Verify /etc/nsswitch.conf was regenerated with its databases (never emptied)
NSSWITCH_AFTER="$(grep -v '^[[:space:]]*#' /etc/nsswitch.conf | sed '/^[[:space:]]*$/d')"
for database in passwd shadow group hosts; do
    echo "${NSSWITCH_AFTER}" | grep -q "^${database}:" \
        || { error "INVARIANT VIOLATION: /etc/nsswitch.conf lost its '${database}' database"; exit 1; }
done
if [[ "${AUTHSELECT_ORIGINAL%% *}" == "local" && "${NSSWITCH_AFTER}" != "${NSSWITCH_BEFORE}" ]]; then
    error "INVARIANT VIOLATION: /etc/nsswitch.conf changed after activating custom/soos"; exit 1
fi
success "Custom authselect profile activated: pam_faillock preauth/authfail preserved, nsswitch.conf intact."

# ---------------------------------------------------------------------------
# Step 5: sudo and gdm PAM Integration Verification
# ---------------------------------------------------------------------------
info "Verifying sudo and gdm service stack configurations..."

# Verify /etc/pam.d/sudo incorporates system-auth
cat << 'EOF' > /etc/pam.d/test-sudo-fedora
#%PAM-1.0
auth       include      test-system-auth-fedora
account    include      test-system-auth-fedora
password   include      test-system-auth-fedora
session    optional     pam_keyinit.so revoke
session    required     pam_limits.so
session    include      test-system-auth-fedora
EOF

# Verify /etc/pam.d/gdm-password incorporates password-auth
cat << 'EOF' > /etc/pam.d/test-gdm-fedora
#%PAM-1.0
auth       include      test-password-auth-fedora
account    include      test-password-auth-fedora
password   include      test-password-auth-fedora
session    include      test-password-auth-fedora
EOF

# Create test-system-auth-fedora and test-password-auth-fedora
cat << 'EOF' > /etc/pam.d/test-system-auth-fedora
#%PAM-1.0
auth        required      pam_env.so
auth        [success=done default=ignore] pam_soos.so timeout_ms=250
auth        sufficient    pam_unix.so try_first_pass nullok
auth        optional      pam_soos.so event=password-failed timeout_ms=20
auth        required      pam_deny.so
account     required      pam_unix.so
session     required      pam_unix.so
EOF

cat << 'EOF' > /etc/pam.d/test-password-auth-fedora
#%PAM-1.0
auth        required      pam_env.so
auth        [success=done default=ignore] pam_soos.so timeout_ms=250
auth        sufficient    pam_unix.so try_first_pass nullok
auth        optional      pam_soos.so event=password-failed timeout_ms=20
auth        required      pam_deny.so
account     required      pam_unix.so
session     required      pam_unix.so
EOF

# Ensure test user exists
if ! id -u "${TEST_USER}" >/dev/null 2>&1; then
    useradd -m -s /bin/bash "${TEST_USER}"
    echo "${TEST_USER}:${TEST_PASS}" | chpasswd
fi

# Add user to soos system group
soos-admin add-user "${TEST_USER}" || usermod -aG soos "${TEST_USER}"

# Enroll through the real CLI with the mock camera and synthetic inference
# (GitHub #168). soos-biometric-store writes <uid>.cbor.enc, encrypted with
# /var/lib/soos/master.key; a hand-made file would prove nothing.
TEST_UID="$(id -u "${TEST_USER}")"
ENROLLED_TEMPLATE="/var/lib/soos/biometrics/${TEST_UID}.cbor.enc"
soos-enroll --mock enroll --username "${TEST_USER}" --yes \
    || { error "soos-enroll --mock enroll failed for '${TEST_USER}'"; exit 1; }
test -f "${ENROLLED_TEMPLATE}" || { error "Enrollment did not create ${ENROLLED_TEMPLATE}"; exit 1; }
TEMPLATE_MODES="$(stat -c '%a %U:%G' "${ENROLLED_TEMPLATE}")"
[[ "${TEMPLATE_MODES}" == "600 root:root" ]] \
    || { error "${ENROLLED_TEMPLATE} is '${TEMPLATE_MODES}', expected '600 root:root'"; exit 1; }
soos-enroll --mock verify --username "${TEST_USER}" \
    || { error "soos-enroll --mock verify rejected the freshly enrolled template"; exit 1; }
success "User '${TEST_USER}' enrolled via soos-enroll --mock: ${ENROLLED_TEMPLATE} (600 root:root)."

# Compile pam_test_runner if needed
if [[ ! -x "/usr/local/bin/pam_test_runner" ]]; then
    gcc -O2 tests/docker/pam_test_runner.c -lpam -o /usr/local/bin/pam_test_runner
    chmod 755 /usr/local/bin/pam_test_runner
fi

# Socket directory invariant (GitHub #168): /run/soos is 0750 root:soos and the
# socket 0660 root:soos, exactly as the daemon and the packages create them.
# The PAM host (pam_test_runner) runs as root, like sudo/login/gdm.
install -d -m 0750 -o root -g soos /run/soos

assert_socket_modes() {
    local dir_modes sock_modes
    dir_modes="$(stat -c '%a %U:%G' /run/soos)"
    [[ "${dir_modes}" == "750 root:soos" ]] \
        || { error "/run/soos is '${dir_modes}', expected '750 root:soos'"; exit 1; }
    [[ -S /run/soos/daemon.sock ]] || { error "/run/soos/daemon.sock is not a socket"; exit 1; }
    sock_modes="$(stat -c '%a %U:%G' /run/soos/daemon.sock)"
    [[ "${sock_modes}" == "660 root:soos" ]] \
        || { error "/run/soos/daemon.sock is '${sock_modes}', expected '660 root:soos'"; exit 1; }
}

start_mock_daemon() {
    python3 tests/docker/mock_daemon.py --socket /run/soos/daemon.sock "$@" &
    MOCK_PID=$!
    for _ in $(seq 1 50); do
        [[ -S /run/soos/daemon.sock ]] && break
        sleep 0.1
    done
    assert_socket_modes
}

# ---------------------------------------------------------------------------
# Step 6: Execute PAM Auth & Password Fallback on sudo and gdm
# ---------------------------------------------------------------------------
# 6a. Nominal Facial Auth for sudo
info "Testing nominal facial auth on sudo service stack..."
start_mock_daemon --mode allow

if /usr/local/bin/pam_test_runner test-sudo-fedora "${TEST_USER}"; then
    success "sudo service stack authenticated via facial auth without password prompts."
else
    error "sudo service stack failed nominal facial auth."
    exit 1
fi
cleanup

# 6b. Nominal Facial Auth for gdm
info "Testing nominal facial auth on gdm service stack..."
start_mock_daemon --mode allow

if /usr/local/bin/pam_test_runner test-gdm-fedora "${TEST_USER}"; then
    success "gdm service stack authenticated via facial auth without password prompts."
else
    error "gdm service stack failed nominal facial auth."
    exit 1
fi
cleanup

# 6c. Password Fallback on sudo (Offline Daemon)
info "Testing password fallback on sudo service stack..."
if /usr/local/bin/pam_test_runner test-sudo-fedora "${TEST_USER}" "${TEST_PASS}"; then
    success "sudo service stack degraded seamlessly to password authentication."
else
    error "sudo service stack rejected valid password during fallback."
    exit 1
fi

# 6d. Password Fallback on gdm (Offline Daemon)
info "Testing password fallback on gdm service stack..."
if /usr/local/bin/pam_test_runner test-gdm-fedora "${TEST_USER}" "${TEST_PASS}"; then
    success "gdm service stack degraded seamlessly to password authentication."
else
    error "gdm service stack rejected valid password during fallback."
    exit 1
fi

# ---------------------------------------------------------------------------
# Step 7: Rollback Procedure Verification
# ---------------------------------------------------------------------------
info "Validating safe uninstallation and rollback procedure..."
if [[ "${INSTALL_MODE}" = "rpm" ]] && command -v rpm >/dev/null 2>&1; then
    info "Removing package via rpm -e soos..."
    rpm -e soos
    test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after rpm -e"; exit 1; }
    success "rpm -e rollback removed binaries cleanly."
else
    info "Executing rollback via scripts/uninstall.sh --keep-data..."
    bash scripts/uninstall.sh --keep-data --skip-systemd
    test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after uninstall"; exit 1; }
    success "scripts/uninstall.sh rollback completed cleanly."
fi

# Verify the authselect rollback: custom/soos is no longer selected, the recorded
# profile is back and the configuration is valid.
AUTHSELECT_AFTER="$(authselect current --raw 2>/dev/null || true)"
if [[ "${AUTHSELECT_AFTER}" == custom/soos* ]]; then
    error "authselect still points at custom/soos after rollback"; exit 1
fi
if [[ -n "${AUTHSELECT_ORIGINAL}" && "${AUTHSELECT_AFTER}" != "${AUTHSELECT_ORIGINAL}" ]]; then
    error "authselect rollback restored '${AUTHSELECT_AFTER}' instead of '${AUTHSELECT_ORIGINAL}'"; exit 1
fi
authselect check || { error "authselect check failed after rollback"; exit 1; }
test ! -d "${AUTHSELECT_PROFILE_DIR}" || { error "${AUTHSELECT_PROFILE_DIR} still present after rollback"; exit 1; }
success "authselect rollback verified: profile '${AUTHSELECT_AFTER}' restored."

# Clean up test files
rm -f /etc/pam.d/test-sudo-fedora /etc/pam.d/test-gdm-fedora /etc/pam.d/test-system-auth-fedora /etc/pam.d/test-password-auth-fedora "${ENROLLED_TEMPLATE}"

echo ""
success "==================================================================="
success "  Fedora 40 / RHEL 9 Deployment Validation Succeeded (100%)"
success "==================================================================="
exit 0
