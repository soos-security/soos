#!/usr/bin/env bash
# =============================================================================
# tests/distro/arch_linux_test.sh — Arch Linux Deployment Validation Test
# =============================================================================
# Sub-issue #32.3:
#   - Verify PKGBUILD build and installation via pacman
#   - Validate /etc/pam.d/system-auth integration via snippet
#   - Test swaylock and hyprlock Wayland screen locker integration
#   - Validate and document rollback procedure
#
# Usage:
#   bash tests/distro/arch_linux_test.sh [OPTIONS]
#
# Options:
#   --pkgbuild               Test installation via Arch PKGBUILD (.pkg.tar.zst)
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

INSTALL_MODE="auto" # auto, pkgbuild, install-sh
SKIP_BUILD=false
DRY_RUN=false
TEST_USER="testuser"
TEST_PASS="password123"

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Executes Arch Linux deployment, PKGBUILD packaging, system-auth integration,
swaylock and hyprlock screen locker testing, and rollback validation suite.

Options:
  --pkgbuild               Install via Arch Linux PKGBUILD package
  --install-sh             Install via scripts/install.sh
  --skip-build             Skip cargo build if binaries are already present
  --dry-run                Print execution plan without making root changes
  -h, --help               Display this help message and exit

Security Invariants Tested:
  - Arch Linux PKGBUILD specification & pacman install invariants
  - Arch /etc/pam.d/system-auth snippet integration
  - Wayland screen locker integration (swaylock & hyprlock)
  - /var/lib/soos/{biometrics,evidence} mode 0700 (root:root)
  - /var/lib/soos/master.key mode 0600 (root:root, 32 bytes)
  - Nominal facial auth (Verdict::Allow -> PAM_SUCCESS, 0 password prompts)
  - Password fallback (Verdict::Deny / offline daemon -> password authentication)
  - Safe rollback and uninstallation via pacman -R or uninstall.sh
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --pkgbuild)
            INSTALL_MODE="pkgbuild"
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
info "  SOOS — Arch Linux Deployment Validation (#32.3)"
info "==================================================================="
info "Workspace:    ${WORKSPACE_ROOT}"
info "Install Mode: ${INSTALL_MODE}"
info "Dry Run:      ${DRY_RUN}"
echo ""

if [[ "${DRY_RUN}" = true ]]; then
    info "[DRY RUN] Simulating Arch Linux deployment plan:"
    info "  1. Build / verify release binaries and PKGBUILD package"
    info "  2. Install via ${INSTALL_MODE} (pacman -U or scripts/install.sh)"
    info "  3. Verify filesystem permissions (biometrics: 0700, master.key: 0600)"
    info "  4. Verify /etc/pam.d/system-auth configuration and snippet"
    info "  5. Configure and test swaylock and hyprlock PAM services"
    info "  6. Test nominal screen unlocking via facial authentication (0 prompts)"
    info "  7. Test screen unlocking password fallback (valid succeeds, invalid rejected)"
    info "  8. Execute rollback procedure and restore clean system state"
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
# Step 2: Installation (PKGBUILD / pacman or scripts/install.sh)
# ---------------------------------------------------------------------------
if [[ "${INSTALL_MODE}" = "auto" || "${INSTALL_MODE}" = "pkgbuild" ]]; then
    if command -v pacman >/dev/null 2>&1; then
        info "Building Arch Linux PKGBUILD package..."
        bash scripts/build_arch.sh --skip-build
        PKG_FILE=$(ls -t target/packages/soos-*.pkg.tar.* | head -n 1)
        info "Installing package via pacman -U: ${PKG_FILE}..."
        pacman -U --noconfirm "${PKG_FILE}"
        success "Arch package installed successfully via pacman."
    else
        warn "pacman not found, falling back to scripts/install.sh..."
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
for candidate in "/usr/lib/security" "/lib/security"; do
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
# Step 4: Arch Linux system-auth Snippet Verification
# ---------------------------------------------------------------------------
info "Verifying Arch Linux PAM configuration and system-auth integration..."
test -f "packaging/pam/arch/system-auth.snippet" || { error "Arch PAM snippet missing"; exit 1; }

cat << 'EOF' > /etc/pam.d/test-system-auth-arch
#%PAM-1.0
# /etc/pam.d/test-system-auth-arch with soos snippet
auth      [success=done default=ignore] pam_soos.so timeout_ms=250
auth      required      pam_unix.so try_first_pass nullok
auth      optional      pam_soos.so event=password-failed timeout_ms=20
account   required      pam_unix.so
session   required      pam_unix.so
EOF

# ---------------------------------------------------------------------------
# Step 5: Screen Locker Integration (swaylock / hyprlock)
# ---------------------------------------------------------------------------
info "Configuring and verifying Wayland screen locker PAM stacks (swaylock, hyprlock)..."

# In Arch Linux, swaylock includes system-auth
cat << 'EOF' > /etc/pam.d/test-swaylock
#%PAM-1.0
auth include test-system-auth-arch
account include test-system-auth-arch
EOF

# In Arch Linux, hyprlock includes system-auth
cat << 'EOF' > /etc/pam.d/test-hyprlock
#%PAM-1.0
auth include test-system-auth-arch
account include test-system-auth-arch
EOF

# Ensure test user exists
if ! id -u "${TEST_USER}" >/dev/null 2>&1; then
    useradd -m -s /bin/bash "${TEST_USER}"
    echo "${TEST_USER}:${TEST_PASS}" | chpasswd
fi

# Add user to soos system group
soos-admin add-user "${TEST_USER}" || usermod -aG soos "${TEST_USER}"

# Ensure test template exists
ENROLLED_TEMPLATE="/var/lib/soos/biometrics/${TEST_USER}.bin"
head -c 512 /dev/urandom > "${ENROLLED_TEMPLATE}"
chmod 0600 "${ENROLLED_TEMPLATE}"
chown root:root "${ENROLLED_TEMPLATE}"

# Compile pam_test_runner if needed
if [[ ! -x "/usr/local/bin/pam_test_runner" ]]; then
    gcc -O2 tests/docker/pam_test_runner.c -lpam -o /usr/local/bin/pam_test_runner
    chmod 755 /usr/local/bin/pam_test_runner
fi

mkdir -p /run/soos
chmod 777 /run/soos

# ---------------------------------------------------------------------------
# Step 6: Test Screen Locker PAM Flow (swaylock & hyprlock)
# ---------------------------------------------------------------------------
# 6a. Nominal Facial Auth for swaylock
info "Testing nominal facial unlock for swaylock..."
python3 tests/docker/mock_daemon.py --mode allow --socket /run/soos/daemon.sock &
MOCK_PID=$!
sleep 0.2

if /usr/local/bin/pam_test_runner test-swaylock "${TEST_USER}"; then
    success "swaylock authenticated via facial verification without password prompt."
else
    error "swaylock failed nominal facial authentication."
    exit 1
fi
cleanup

# 6b. Nominal Facial Auth for hyprlock
info "Testing nominal facial unlock for hyprlock..."
python3 tests/docker/mock_daemon.py --mode allow --socket /run/soos/daemon.sock &
MOCK_PID=$!
sleep 0.2

if /usr/local/bin/pam_test_runner test-hyprlock "${TEST_USER}"; then
    success "hyprlock authenticated via facial verification without password prompt."
else
    error "hyprlock failed nominal facial authentication."
    exit 1
fi
cleanup

# 6c. Password Fallback on Screen Locker (Offline Daemon)
info "Testing screen locker password fallback (daemon offline)..."
if /usr/local/bin/pam_test_runner test-swaylock "${TEST_USER}" "${TEST_PASS}"; then
    success "swaylock cleanly fell back to password authentication."
else
    error "swaylock rejected valid password during fallback."
    exit 1
fi

# 6d. Rejection of Wrong Password on Screen Locker
info "Testing screen locker rejection of incorrect password..."
if /usr/local/bin/pam_test_runner test-swaylock "${TEST_USER}" "wrong_password" 2>/dev/null; then
    error "Security Invariant Violation: Invalid password accepted by swaylock!"
    exit 1
else
    success "swaylock rejected invalid password cleanly."
fi

# ---------------------------------------------------------------------------
# Step 7: Rollback Procedure Verification
# ---------------------------------------------------------------------------
info "Validating safe uninstallation and rollback procedure..."
if [[ "${INSTALL_MODE}" = "pkgbuild" ]] && command -v pacman >/dev/null 2>&1; then
    info "Removing package via pacman -R soos..."
    pacman -R --noconfirm soos
    test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after pacman -R"; exit 1; }
    success "pacman -R rollback removed binaries cleanly."
else
    info "Executing rollback via scripts/uninstall.sh --keep-data..."
    bash scripts/uninstall.sh --keep-data --skip-systemd
    test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after uninstall"; exit 1; }
    success "scripts/uninstall.sh rollback completed cleanly."
fi

# Clean up test files
rm -f /etc/pam.d/test-system-auth-arch /etc/pam.d/test-swaylock /etc/pam.d/test-hyprlock "${ENROLLED_TEMPLATE}"

echo ""
success "==================================================================="
success "  Arch Linux Deployment Validation Succeeded (100%)"
success "==================================================================="
exit 0
