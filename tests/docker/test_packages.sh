#!/usr/bin/env bash
# =============================================================================
# tests/docker/test_packages.sh — In-Container Distribution Package Test Harness
# =============================================================================
# Tests building and installing native distribution packages (.deb, .rpm, .pkg.tar.zst)
# within isolated distribution test containers.
#
# Usage:
#   bash tests/docker/test_packages.sh
#
# CI: job `package-deploy` (ubuntu, every pull request) and job `distro-deploy`
# (fedora RPM and arch pacman branches, push to main and manual dispatch) run it
# in the image and target volume of tests/distro/run_distro_validation.sh.
# Fails closed: missing packaging tools or an unknown distribution is an error.
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
    cargo build --locked --release --workspace
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

    local key_size
    key_size=$(wc -c < "/var/lib/soos/master.key" | tr -d ' ')
    if [[ "${key_size}" != "32" ]]; then
        error "Incorrect size for /var/lib/soos/master.key: ${key_size} bytes (expected 32)"
        return 1
    fi

    test -x "/usr/libexec/soos/provision-master-key" || { error "/usr/libexec/soos/provision-master-key missing or not executable"; return 1; }

    success "Package installation verified with complete filesystem invariants."
}

# GitHub #144 (ONB-01): a distributable artifact must never contain key material.
# $1 = human-readable package path, remaining args = command printing the archive listing.
verify_package_has_no_key_material() {
    local pkg="$1"
    shift
    info "Verifying that ${pkg} ships no key material..."
    local listing
    listing=$("$@")
    local key_entries
    key_entries=$(echo "${listing}" | grep -E '\.key$' || true)
    if [[ -n "${key_entries}" ]]; then
        error "Package ${pkg} contains key material:"
        echo "${key_entries}" >&2
        return 1
    fi
    success "Package ${pkg} contains no *.key entry."
}

# GitHub #209 (ONB-13): a package ships the PAM template of its own distribution
# only, and never a file in /etc/pam.d (every file there is a PAM service).
# $1 = template family (debian|fedora|arch), remaining args = listing command.
verify_distro_templates() {
    local family="$1"
    shift
    info "Verifying that the package ships only the ${family} PAM template..."
    local listing
    # Last field of each entry, without a leading "./" or "/" (dpkg-deb -c, rpm -qlp, bsdtar -tf).
    listing=$("$@" | awk '{print $NF}' | sed -e 's|^\./||' -e 's|^/||')
    local -a required=() forbidden=()
    case "${family}" in
        debian)
            required=("usr/share/pam-configs/soos" "usr/share/pam-configs/soos-notify")
            forbidden=("etc/authselect/" "usr/share/soos/pam/" "etc/pam.d/")
            ;;
        fedora)
            required=("etc/authselect/custom/soos/system-auth")
            forbidden=("usr/share/pam-configs/" "usr/share/soos/pam/" "etc/pam.d/")
            ;;
        arch)
            required=("usr/share/soos/pam/system-auth.snippet")
            forbidden=("usr/share/pam-configs/" "etc/authselect/" "etc/pam.d/")
            ;;
        *)
            error "verify_distro_templates: unknown family '${family}'"
            return 1
            ;;
    esac
    local entry
    for entry in "${required[@]}"; do
        grep -Fxq -- "${entry}" <<< "${listing}" || { error "Package lacks ${entry}"; return 1; }
    done
    for entry in "${forbidden[@]}"; do
        if grep -Fq -- "${entry}" <<< "${listing}"; then
            error "Package ships a foreign PAM template or a /etc/pam.d entry: $(grep -F -- "${entry}" <<< "${listing}" | tr '\n' ' ')"
            return 1
        fi
    done
    success "Package ships only the ${family} PAM template."
}

# The key must be generated on the target host: a second fresh install must
# produce a different key, and package removal must leave the key untouched
# (it is not package-owned, so enrolled templates survive remove/upgrade).
key_fingerprint() {
    sha256sum "/var/lib/soos/master.key" | cut -d' ' -f1
}

verify_key_survives_removal() {
    test -f "/var/lib/soos/master.key" || { error "master.key must survive package removal (it is host state, not package content)"; return 1; }
    success "master.key preserved across package removal."
}

verify_fresh_install_generates_distinct_key() {
    local first_fp="$1"
    local second_fp="$2"
    if [[ "${first_fp}" == "${second_fp}" ]]; then
        error "Two fresh installs produced the same master key: the key is baked into the package"
        return 1
    fi
    success "Fresh install generated a distinct master key (not shipped in the package)."
}

# GitHub #281: upgrade with `rpm -U` from a legacy build that still owned master.key as
# %ghost (walkthrough 106). RPM erases the old package's %ghost file after the new
# package's %post; the %pre/%posttrans guard of packaging/rpm/soos.spec must restore the
# exact pre-upgrade key and discard its temporary copy.
# $1 = the current soos RPM (the upgrade target).
verify_rpm_upgrade_keeps_ghost_owned_key() {
    local new_rpm="$1"
    local topdir
    topdir=$(mktemp -d "/tmp/soos_legacy_rpm.XXXXXX")
    mkdir -p "${topdir}"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

    # Minimal legacy fixture: same name and version, lower release, owns the key as
    # %ghost and generates it in %post like the builds before walkthrough 106.
    cat > "${topdir}/SPECS/soos-legacy.spec" <<'SPEC'
Name: soos
Version: 0.1.0
Release: 0.legacy
Summary: Legacy soos fixture owning master.key as ghost (test only)
License: Apache-2.0 OR MIT
%global debug_package %{nil}

%description
Test fixture for the rpm -U master key upgrade guard (GitHub #281).

%install
install -d -m 0755 %{buildroot}/var/lib/soos

%post
if [ ! -e /var/lib/soos/master.key ]; then
    (umask 077 && head -c 32 /dev/urandom > /var/lib/soos/master.key)
fi

%files
%dir /var/lib/soos
%ghost %attr(0600, root, root) /var/lib/soos/master.key
SPEC

    info "Building the legacy %ghost-key fixture RPM..."
    rpmbuild -bb --define "_topdir ${topdir}" "${topdir}/SPECS/soos-legacy.spec" >/dev/null
    local legacy_rpm
    legacy_rpm=$(ls "${topdir}"/RPMS/*/soos-0.1.0-0.legacy.*.rpm | head -n 1)

    rm -f /var/lib/soos/master.key /var/lib/soos/.master.key.upgrade
    info "Installing the legacy fixture ${legacy_rpm} via rpm -i..."
    rpm -i "${legacy_rpm}"
    rpm -qf /var/lib/soos/master.key >/dev/null 2>&1 \
        || { error "legacy fixture does not own master.key"; return 1; }
    local legacy_fp
    legacy_fp=$(key_fingerprint)

    info "Upgrading the legacy fixture to ${new_rpm} via rpm -U..."
    rpm -U "${new_rpm}"
    rm -rf "${topdir}"

    test -f /var/lib/soos/master.key \
        || { error "master.key was erased by rpm -U from a %ghost-owning build"; return 1; }
    if [[ "$(key_fingerprint)" != "${legacy_fp}" ]]; then
        error "master.key changed across rpm -U: templates enrolled before the upgrade are lost"
        return 1
    fi
    if [[ -e /var/lib/soos/.master.key.upgrade ]]; then
        error "/var/lib/soos/.master.key.upgrade was left behind although the keys are equal"
        return 1
    fi
    if rpm -qf /var/lib/soos/master.key >/dev/null 2>&1; then
        error "master.key is still owned by a package after the upgrade"
        return 1
    fi
    success "rpm -U from a %ghost-owning build kept the exact master key (no leftover copy)."
}

case "${DISTRO}" in
    ubuntu|debian)
        info "Running Debian (.deb) package verification..."
        bash scripts/build_deb.sh --skip-build

        DEB_FILE=$(ls -t target/packages/soos_*.deb | head -n 1)
        verify_package_has_no_key_material "${DEB_FILE}" dpkg-deb -c "${DEB_FILE}"
        verify_distro_templates debian dpkg-deb -c "${DEB_FILE}"

        info "Installing ${DEB_FILE} via dpkg -i..."
        dpkg -i "${DEB_FILE}"

        PAM_DIR="/lib/x86_64-linux-gnu/security"
        if [[ ! -d "${PAM_DIR}" ]]; then
            PAM_DIR="/lib/security"
        fi

        verify_installation "${PAM_DIR}"
        FIRST_KEY_FP=$(key_fingerprint)

        info "Testing package removal via dpkg -r soos..."
        dpkg -r soos
        test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after dpkg -r"; exit 1; }
        verify_key_survives_removal

        info "Simulating a second fresh host: removing the key and reinstalling..."
        rm -f /var/lib/soos/master.key
        dpkg -i "${DEB_FILE}"
        verify_installation "${PAM_DIR}"
        verify_fresh_install_generates_distinct_key "${FIRST_KEY_FP}" "$(key_fingerprint)"
        dpkg -r soos
        success "Debian (.deb) package test passed cleanly."
        ;;

    fedora|rhel|centos)
        info "Running Fedora / RHEL (.rpm) package verification..."
        if ! command -v rpmbuild >/dev/null 2>&1; then
            error "rpmbuild is not available: the RPM branch cannot be verified in this container."
            exit 1
        fi
        bash scripts/build_rpm.sh --skip-build

        # soos-[0-9]*: the main package, never soos-debuginfo / soos-debugsource.
        RPM_FILE=$(ls -t target/packages/soos-[0-9]*.rpm | head -n 1)
        # --noghost: %ghost entries are metadata only and carry no payload.
        verify_package_has_no_key_material "${RPM_FILE}" rpm -qlp --noghost "${RPM_FILE}"
        verify_distro_templates fedora rpm -qlp --noghost "${RPM_FILE}"

        info "Installing ${RPM_FILE} via rpm -i..."
        rpm -i "${RPM_FILE}"

        PAM_DIR="/usr/lib64/security"
        if [[ ! -d "${PAM_DIR}" ]]; then
            PAM_DIR="/usr/lib/security"
        fi

        verify_installation "${PAM_DIR}"
        FIRST_KEY_FP=$(key_fingerprint)

        info "Testing package removal via rpm -e soos..."
        rpm -e soos
        test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after rpm -e"; exit 1; }
        verify_key_survives_removal

        info "Simulating a second fresh host: removing the key and reinstalling..."
        rm -f /var/lib/soos/master.key
        rpm -i "${RPM_FILE}"
        verify_installation "${PAM_DIR}"
        verify_fresh_install_generates_distinct_key "${FIRST_KEY_FP}" "$(key_fingerprint)"
        rpm -e soos

        verify_rpm_upgrade_keeps_ghost_owned_key "${RPM_FILE}"
        verify_installation "${PAM_DIR}"
        rpm -e soos
        verify_key_survives_removal
        success "RPM package test passed cleanly."
        ;;

    arch)
        info "Running Arch Linux package verification..."
        bash scripts/build_arch.sh --skip-build

        PKG_FILE=$(ls -t target/packages/soos-*.pkg.tar.* | head -n 1)
        verify_package_has_no_key_material "${PKG_FILE}" bsdtar -tf "${PKG_FILE}"
        verify_distro_templates arch bsdtar -tf "${PKG_FILE}"

        info "Installing ${PKG_FILE} via pacman -U..."
        pacman -U --noconfirm "${PKG_FILE}"

        verify_installation "/usr/lib/security"
        FIRST_KEY_FP=$(key_fingerprint)

        info "Testing package removal via pacman -R soos..."
        pacman -R --noconfirm soos
        test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after pacman -R"; exit 1; }
        verify_key_survives_removal

        info "Simulating a second fresh host: removing the key and reinstalling..."
        rm -f /var/lib/soos/master.key
        pacman -U --noconfirm "${PKG_FILE}"
        verify_installation "/usr/lib/security"
        verify_fresh_install_generates_distinct_key "${FIRST_KEY_FP}" "$(key_fingerprint)"
        pacman -R --noconfirm soos
        success "Arch Linux package test passed cleanly."
        ;;

    *)
        error "Unsupported distribution '${DISTRO}': no native package path to verify."
        exit 1
        ;;
esac

echo ""
success "==================================================================="
success "  All distribution package verification tests succeeded!"
success "==================================================================="
exit 0
