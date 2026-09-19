#!/usr/bin/env bash
# =============================================================================
# tests/docker/test_packages.sh — In-Container Distribution Package Test Harness
# =============================================================================
# Tests building and installing native distribution packages (.deb, .rpm, .pkg.tar.zst)
# within isolated distribution test containers.
#
# Usage:
#   bash tests/docker/test_packages.sh
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
echo "  SOOS — Distribution Package Docker Verification Suite"
echo "==================================================================="
echo ""

# Detect Distribution Family
DISTRO=""
if [[ -f /etc/os-release ]]; then
    # shellcheck disable=SC1091
    source /etc/os-release
    DISTRO="${ID}"
fi

info "Detected container distribution: ${DISTRO}"

# Ensure release binaries exist
if [[ ! -f "target/release/soos-daemon" || ! -f "target/release/libpam_soos.so" ]]; then
    info "Compiling release binaries..."
    cargo build --release --workspace
fi

verify_installation() {
    local pam_dir="$1"
    info "Verifying installed package filesystem hierarchy and permissions..."

    # Check binaries
    test -f "/usr/libexec/soos/soos-daemon" || { error "/usr/libexec/soos/soos-daemon missing"; return 1; }
    test -f "/usr/bin/soos-admin" || { error "/usr/bin/soos-admin missing"; return 1; }
    test -f "/usr/bin/soos-enroll" || { error "/usr/bin/soos-enroll missing"; return 1; }
    test -f "${pam_dir}/pam_soos.so" || { error "${pam_dir}/pam_soos.so missing"; return 1; }

    # Check execution
    /usr/bin/soos-admin --help >/dev/null 2>&1 || { error "soos-admin --help failed"; return 1; }
    /usr/bin/soos-enroll --help >/dev/null 2>&1 || { error "soos-enroll --help failed"; return 1; }

    # Check system group
    getent group soos >/dev/null 2>&1 || { error "System group 'soos' not found"; return 1; }

    # Check state directories and permissions
    test -d "/var/lib/soos" || { error "/var/lib/soos missing"; return 1; }
    test -d "/var/lib/soos/biometrics" || { error "/var/lib/soos/biometrics missing"; return 1; }
    test -d "/var/lib/soos/evidence" || { error "/var/lib/soos/evidence missing"; return 1; }
    test -f "/var/lib/soos/master.key" || { error "/var/lib/soos/master.key missing"; return 1; }

    local bio_mode
    bio_mode=$(stat -c "%a" "/var/lib/soos/biometrics" 2>/dev/null || stat -f "%OLp" "/var/lib/soos/biometrics")
    if [[ "${bio_mode}" != "700" ]]; then
        error "Incorrect permissions on /var/lib/soos/biometrics: ${bio_mode} (expected 700)"
        return 1
    fi

    local key_mode
    key_mode=$(stat -c "%a" "/var/lib/soos/master.key" 2>/dev/null || stat -f "%OLp" "/var/lib/soos/master.key")
    if [[ "${key_mode}" != "600" ]]; then
        error "Incorrect permissions on /var/lib/soos/master.key: ${key_mode} (expected 600)"
        return 1
    fi

    success "Package installation verified with complete filesystem invariants."
}

case "${DISTRO}" in
    ubuntu|debian)
        info "Running Debian (.deb) package verification..."
        bash scripts/build_deb.sh --skip-build

        DEB_FILE=$(ls -t target/packages/soos_*.deb | head -n 1)
        info "Installing ${DEB_FILE} via dpkg -i..."
        dpkg -i "${DEB_FILE}"

        PAM_DIR="/lib/x86_64-linux-gnu/security"
        if [[ ! -d "${PAM_DIR}" ]]; then
            PAM_DIR="/lib/security"
        fi

        verify_installation "${PAM_DIR}"

        info "Testing package removal via dpkg -r soos..."
        dpkg -r soos
        test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after dpkg -r"; exit 1; }
        success "Debian (.deb) package test passed cleanly."
        ;;

    fedora|rhel|centos)
        info "Running Fedora / RHEL (.rpm) package verification..."
        if ! command -v rpmbuild >/dev/null 2>&1; then
            warn "rpmbuild not available, skipping RPM test in this container."
            exit 0
        fi
        bash scripts/build_rpm.sh --skip-build

        RPM_FILE=$(ls -t target/packages/soos-*.rpm | head -n 1)
        info "Installing ${RPM_FILE} via rpm -i..."
        rpm -i "${RPM_FILE}"

        PAM_DIR="/usr/lib64/security"
        if [[ ! -d "${PAM_DIR}" ]]; then
            PAM_DIR="/usr/lib/security"
        fi

        verify_installation "${PAM_DIR}"

        info "Testing package removal via rpm -e soos..."
        rpm -e soos
        test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after rpm -e"; exit 1; }
        success "RPM package test passed cleanly."
        ;;

    arch)
        info "Running Arch Linux package verification..."
        bash scripts/build_arch.sh --skip-build

        PKG_FILE=$(ls -t target/packages/soos-*.pkg.tar.* | head -n 1)
        info "Installing ${PKG_FILE} via pacman -U..."
        pacman -U --noconfirm "${PKG_FILE}"

        verify_installation "/usr/lib/security"

        info "Testing package removal via pacman -R soos..."
        pacman -R --noconfirm soos
        test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after pacman -R"; exit 1; }
        success "Arch Linux package test passed cleanly."
        ;;

    *)
        warn "Unknown distribution '${DISTRO}'. Running generic build_packages dry-run..."
        bash scripts/build_packages.sh --dry-run
        ;;
esac

echo ""
success "==================================================================="
success "  All distribution package verification tests succeeded!"
success "==================================================================="
exit 0
