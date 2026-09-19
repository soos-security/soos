#!/usr/bin/env bash
# =============================================================================
# tests/distro/debian_ubuntu_test.sh — Debian 12 / Ubuntu 24.04 Deployment Test
# =============================================================================
# Sub-issue #32.1:
#   - Install via .deb package or install.sh
#   - Enroll user, verify facial auth, test password fallback
#   - Validate and document rollback procedure
#
# Usage:
#   bash tests/distro/debian_ubuntu_test.sh [OPTIONS]
#
# Options:
#   --deb                    Test installation via Debian (.deb) package (default if .deb built)
#   --install-sh             Test installation via scripts/install.sh
#   --skip-build             Do not recompile binaries if artifacts exist
#   --dry-run                Simulate execution plan without modifying root filesystem
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

INSTALL_MODE="auto" # auto, deb, install-sh
SKIP_BUILD=false
DRY_RUN=false
TEST_USER="testuser"
TEST_PASS="password123"

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Executes Debian 12 / Ubuntu 24.04 deployment, enrollment, facial auth,
password fallback, and rollback validation suite.

Options:
  --deb                    Install via Debian package (.deb)
  --install-sh             Install via scripts/install.sh
  --skip-build             Skip cargo build if binaries are already present
  --dry-run                Print execution plan without making root changes
  -h, --help               Display this help message and exit

Security Invariants Tested:
  - debian pam-auth-update integration (/etc/pam.d/common-auth)
  - /var/lib/soos/{biometrics,evidence} mode 0700 (root:root)
  - /var/lib/soos/master.key mode 0600 (root:root, 32 bytes)
  - Nominal facial auth (Verdict::Allow -> PAM_SUCCESS, 0 password prompts)
  - Password fallback (Verdict::Deny / offline daemon -> password authentication)
  - Safe rollback and uninstallation preserving user recovery
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --deb)
            INSTALL_MODE="deb"
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
info "  SOOS — Debian 12 / Ubuntu 24.04 Deployment Validation (#32.1)"
info "==================================================================="
info "Workspace:    ${WORKSPACE_ROOT}"
info "Install Mode: ${INSTALL_MODE}"
info "Dry Run:      ${DRY_RUN}"
echo ""

if [[ "${DRY_RUN}" = true ]]; then
    info "[DRY RUN] Simulating Debian / Ubuntu deployment plan:"
    info "  1. Build / verify release binaries and .deb package"
    info "  2. Install via ${INSTALL_MODE} (dpkg -i or scripts/install.sh)"
    info "  3. Verify filesystem permissions (biometrics: 0700, master.key: 0600)"
    info "  4. Configure Debian PAM profile in /etc/pam.d/common-auth"
    info "  5. Enroll user '${TEST_USER}' into /var/lib/soos/biometrics/"
    info "  6. Test nominal facial authentication (PAM_SUCCESS, 0 password prompts)"
    info "  7. Test fallback to password authentication (valid succeeds, invalid rejected)"
    info "  8. Execute rollback procedure and verify clean system state"
    success "Dry run deployment validation completed successfully."
    exit 0
fi

# Root check for live execution
if [[ "$(id -u)" -ne 0 ]]; then
    error "Live deployment testing requires root privileges (or use --dry-run)."
    exit 1
fi

cleanup() {
    info "Executing test cleanup..."
    pkill -f "mock_daemon.py" || true
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
# Step 2: Installation (.deb or scripts/install.sh)
# ---------------------------------------------------------------------------
if [[ "${INSTALL_MODE}" = "auto" || "${INSTALL_MODE}" = "deb" ]]; then
    if command -v dpkg >/dev/null 2>&1; then
        info "Building Debian (.deb) package..."
        bash scripts/build_deb.sh --skip-build
        DEB_PKG=$(ls -t target/packages/soos_*.deb | head -n 1)
        info "Installing package: ${DEB_PKG}..."
        dpkg -i "${DEB_PKG}"
        success "Debian package installed successfully."
    else
        warn "dpkg not found, falling back to scripts/install.sh..."
        INSTALL_MODE="install-sh"
    fi
fi

if [[ "${INSTALL_MODE}" = "install-sh" ]]; then
    info "Installing via scripts/install.sh..."
    bash scripts/install.sh --skip-models --skip-systemd
    success "scripts/install.sh completed successfully."
fi

# ---------------------------------------------------------------------------
# Step 3: Filesystem Invariants & Permission Verification
# ---------------------------------------------------------------------------
info "Verifying installed filesystem invariants..."
test -f "/usr/libexec/soos/soos-daemon" || { error "soos-daemon binary missing"; exit 1; }
test -f "/usr/bin/soos-admin" || { error "soos-admin binary missing"; exit 1; }
test -f "/usr/bin/soos-enroll" || { error "soos-enroll binary missing"; exit 1; }

PAM_TARGET=""
for candidate in "/lib/x86_64-linux-gnu/security" "/lib/security" "/usr/lib/security"; do
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
# Step 4: Debian PAM Stack Configuration (/etc/pam.d/common-auth)
# ---------------------------------------------------------------------------
info "Validating Debian PAM configuration templates..."
test -f "/usr/share/pam-configs/soos" || { error "/usr/share/pam-configs/soos missing"; exit 1; }
test -f "/usr/share/pam-configs/soos-notify" || { error "/usr/share/pam-configs/soos-notify missing"; exit 1; }

# If pam-auth-update exists, test enabling profile; otherwise configure test service
if command -v pam-auth-update >/dev/null 2>&1; then
    info "Applying pam-auth-update configuration..."
    pam-auth-update --package --enable soos soos-notify || true
fi

# Ensure test service definition exists
cat << 'EOF' > /etc/pam.d/test-soos-debian
# /etc/pam.d/test-soos-debian test harness stack
auth  [success=done default=ignore]  pam_soos.so timeout_ms=250
auth  required                       pam_unix.so nullok
auth  optional                       pam_soos.so event=password-failed timeout_ms=20
account required pam_unix.so
session required pam_unix.so
EOF

# ---------------------------------------------------------------------------
# Step 5: User Enrollment Verification
# ---------------------------------------------------------------------------
info "Validating user enrollment workflow for '${TEST_USER}'..."
if ! id -u "${TEST_USER}" >/dev/null 2>&1; then
    useradd -m -s /bin/bash "${TEST_USER}"
    echo "${TEST_USER}:${TEST_PASS}" | chpasswd
fi

# Add user to soos system group
soos-admin add-user "${TEST_USER}" || usermod -aG soos "${TEST_USER}"

# Generate mock enrolled template for testuser in /var/lib/soos/biometrics/
ENROLLED_TEMPLATE="/var/lib/soos/biometrics/${TEST_USER}.bin"
head -c 512 /dev/urandom > "${ENROLLED_TEMPLATE}"
chmod 0600 "${ENROLLED_TEMPLATE}"
chown root:root "${ENROLLED_TEMPLATE}"
success "User '${TEST_USER}' enrolled with template ${ENROLLED_TEMPLATE} (mode 0600)."

# ---------------------------------------------------------------------------
# Step 6: PAM Facial Auth & Password Fallback Verification
# ---------------------------------------------------------------------------
info "Testing PAM authentication pathways on Debian stack..."

# Compile pam_test_runner if needed
if [[ ! -x "/usr/local/bin/pam_test_runner" ]]; then
    gcc -O2 tests/docker/pam_test_runner.c -lpam -o /usr/local/bin/pam_test_runner
    chmod 755 /usr/local/bin/pam_test_runner
fi

mkdir -p /run/soos
chmod 777 /run/soos

# 6a. Nominal Facial Auth (PAM_SUCCESS, 0 prompts)
info "Running nominal facial authentication test..."
python3 tests/docker/mock_daemon.py --mode allow --socket /run/soos/daemon.sock &
MOCK_PID=$!
sleep 0.2

if /usr/local/bin/pam_test_runner test-soos-debian "${TEST_USER}"; then
    success "Nominal facial authentication succeeded (PAM_SUCCESS, 0 password prompts)."
else
    error "Nominal facial authentication failed."
    exit 1
fi
cleanup

# 6b. Password Fallback on Daemon Timeout / Offline
info "Running password fallback verification on offline daemon..."
if /usr/local/bin/pam_test_runner test-soos-debian "${TEST_USER}" "${TEST_PASS}"; then
    success "Password fallback succeeded with valid credentials (PAM_IGNORE -> pam_unix)."
else
    error "Password fallback failed with valid credentials."
    exit 1
fi

# 6c. Password Fallback with Wrong Password (Rejected)
info "Verifying rejection of invalid password during fallback..."
if /usr/local/bin/pam_test_runner test-soos-debian "${TEST_USER}" "wrong_password" 2>/dev/null; then
    error "Security Invariant Violation: Invalid password was accepted!"
    exit 1
else
    success "Invalid password was cleanly rejected (fails closed)."
fi

# ---------------------------------------------------------------------------
# Step 7: Rollback Procedure Verification
# ---------------------------------------------------------------------------
info "Validating safe uninstallation and rollback procedure..."
if [[ "${INSTALL_MODE}" = "deb" ]] && command -v dpkg >/dev/null 2>&1; then
    info "Removing package via dpkg -r soos..."
    dpkg -r soos
    test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after dpkg -r"; exit 1; }
    success "dpkg rollback removed binaries cleanly."
else
    info "Executing rollback via scripts/uninstall.sh --keep-data..."
    bash scripts/uninstall.sh --keep-data --skip-systemd
    test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after uninstall"; exit 1; }
    success "scripts/uninstall.sh rollback completed cleanly."
fi

# Verify biometric data preserved under --keep-data
test -f "${ENROLLED_TEMPLATE}" || { error "Biometric data was not preserved during rollback"; exit 1; }
success "Biometric template preserved at ${ENROLLED_TEMPLATE}."

# Clean up test artifacts
rm -f /etc/pam.d/test-soos-debian "${ENROLLED_TEMPLATE}"

echo ""
success "==================================================================="
success "  Debian 12 / Ubuntu 24.04 Deployment Validation Succeeded (100%)"
success "==================================================================="
exit 0
