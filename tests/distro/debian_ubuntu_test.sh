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
#   --skip-build             Do not recompile; package the existing release artifacts
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

INSTALL_MODE="auto" # auto, deb, install-sh
SKIP_BUILD=false
DRY_RUN=false
ALLOW_HOST_CHANGES=false
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
  --skip-build             Skip cargo build and use the existing release binaries
  --dry-run                Print execution plan without making root changes
  --allow-host-changes     Consent to a live run that modifies this host (packages,
                           PAM files, users); use it only in a disposable container
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

# Without --skip-build, always invoke cargo (a no-op when up to date) so a stale
# release artifact from the bind-mounted target/ is never packaged (GitHub #244).
if [[ "${SKIP_BUILD}" = false ]]; then
    info "Building release artifacts (no-op when up to date)..."
    cargo build --locked --release --workspace
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
        INSTALL_MODE="deb"
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

# pam-auth-update is part of libpam-runtime on every Debian/Ubuntu system: a
# failure to enable the profiles is a deployment failure (GitHub #274), never
# ignored. --force: the sandbox image ships a hand-written common-auth, which
# pam-auth-update otherwise refuses to regenerate. Stack order and rollback of
# the generated file are asserted by tests/docker/pam_rollback_test.sh (D2/D3).
if ! command -v pam-auth-update >/dev/null 2>&1; then
    error "pam-auth-update is missing on a Debian-based system."
    exit 1
fi
info "Applying pam-auth-update configuration..."
if ! DEBIAN_FRONTEND=noninteractive pam-auth-update --package --force --enable soos soos-notify; then
    error "pam-auth-update --package --force --enable soos soos-notify failed."
    exit 1
fi
if ! grep -q 'pam_soos.so event=password-failed' /etc/pam.d/common-auth; then
    error "Generated /etc/pam.d/common-auth lacks the soos-notify password-failed hook."
    exit 1
fi
success "pam-auth-update enabled the soos and soos-notify profiles in /etc/pam.d/common-auth."

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

# ---------------------------------------------------------------------------
# Step 6: PAM Facial Auth & Password Fallback Verification
# ---------------------------------------------------------------------------
info "Testing PAM authentication pathways on Debian stack..."

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

# 6a. Nominal Facial Auth (PAM_SUCCESS, 0 prompts)
info "Running nominal facial authentication test..."
start_mock_daemon --mode allow

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
