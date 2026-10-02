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
#
# Ownership (GitHub #301): the .deb and the Arch package are built by the
# unprivileged user soosbuild; every archive entry must be root:root and every
# installed path uid 0. The Arch branch also builds packaging/arch/PKGBUILD with
# makepkg and requires the same layout as scripts/build_arch.sh (GitHub #316).
#
# Upgrade path (GitHub #327): every branch upgrades or reinstalls over the installed
# package (dpkg -i of a version-bumped copy, rpm -U --replacepkgs, pacman -U) and requires
# master.key, an enrolled template, /etc/soos/daemon.toml and the PAM activation state to
# stay byte-identical; the Debian branch also keeps an administrator-disabled profile.
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

# Always invoke cargo (a no-op when up to date) so a stale release artifact built
# by the host into the bind-mounted target/ is never packaged (GitHub #244).
info "Building release binaries (no-op when up to date)..."
cargo build --locked --release --workspace

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

# GitHub #301: native packages are built by an unprivileged user (like a developer
# running scripts/build_*.sh in a checkout), never only by root as in CI. Package
# outputs go to a directory that user owns; nothing is written into /workspace.
BUILD_USER="soosbuild"
PKG_OUT="/tmp/soos-packages"
ensure_build_user() {
    if ! id -u "${BUILD_USER}" >/dev/null 2>&1; then
        useradd -m "${BUILD_USER}"
    fi
    rm -rf "${PKG_OUT}"
    install -d -m 0755 -o "${BUILD_USER}" "${PKG_OUT}"
}

# Every archive entry must be owned by root:root: package managers extract the
# recorded owners, and a builder-owned soos-daemon or pam_soos.so would let that
# user replace the root daemon or the PAM module (GitHub #301).
# $1 = package path, $2 = family (debian|arch|fedora).
verify_archive_root_owned() {
    local pkg="$1" family="$2" foreign=""
    info "Verifying that every entry of ${pkg} is owned by root:root..."
    case "${family}" in
        debian)
            foreign="$(dpkg-deb -c "${pkg}" | awk '$2 != "root/root" {print $2, $NF}')"
            ;;
        arch)
            foreign="$(bsdtar -tvf "${pkg}" | awk '$3 != "root" || $4 != "root" {print $3 ":" $4, $NF}')"
            foreign+="$(bsdtar --numeric-owner -tvf "${pkg}" | awk '$3 != "0" || $4 != "0" {print $3 ":" $4, $NF}')"
            ;;
        fedora)
            foreign="$(rpm -qlvp --noghost "${pkg}" | awk '$3 != "root" || $4 != "root" {print $3 ":" $4, $NF}')"
            ;;
        *)
            error "verify_archive_root_owned: unknown family '${family}'"
            return 1
            ;;
    esac
    if [[ -n "${foreign}" ]]; then
        error "${pkg} contains entries not owned by root:root:"
        echo "${foreign}" >&2
        return 1
    fi
    success "Every entry of ${pkg} is owned by root:root."
}

# After installation every file and directory owned by the package is uid 0 and
# gid 0 (the RPM %ghost /run/soos is root:soos by design and is not a payload).
# Remaining args = command listing the installed paths of the package.
verify_installed_files_root_owned() {
    local path owner bad=""
    info "Verifying that every installed soos path is owned by uid 0..."
    while IFS= read -r path; do
        [[ -z "${path}" || ! -e "${path}" ]] && continue
        owner="$(stat -c '%u:%g' "${path}")"
        if [[ "${path}" == /run/soos ]]; then
            [[ "${owner%%:*}" == "0" ]] || bad+="${owner} ${path}"$'\n'
        elif [[ "${owner}" != "0:0" ]]; then
            bad+="${owner} ${path}"$'\n'
        fi
    done < <("$@")
    if [[ -n "${bad}" ]]; then
        error "Installed paths not owned by root:"
        printf '%s' "${bad}" >&2
        return 1
    fi
    success "Every installed soos path is owned by uid 0 (gid 0)."
}

# GitHub #316 ONB-NEW-3: packages own the unit under /usr/lib/systemd/system
# (/etc/systemd/system belongs to the administrator) and never ship /run (a tmpfs
# recreated by the scriptlets and the unit's RuntimeDirectory=).
# Remaining args = command printing the archive listing.
verify_unit_and_run_layout() {
    local listing
    listing=$("$@" | awk '{print $NF}' | sed -e 's|^\./||' -e 's|^/||')
    grep -Fxq "usr/lib/systemd/system/soos-daemon.service" <<< "${listing}" \
        || { error "Package lacks usr/lib/systemd/system/soos-daemon.service"; return 1; }
    if grep -Eq '^(etc/systemd/|run/|var/run/)' <<< "${listing}"; then
        error "Package ships /etc/systemd or /run entries: $(grep -E '^(etc/systemd/|run/|var/run/)' <<< "${listing}" | tr '\n' ' ')"
        return 1
    fi
    success "Unit under /usr/lib/systemd/system; no /etc/systemd or /run entry."
}

# GitHub #316 ONB-NEW-3: packaging/arch/PKGBUILD (makepkg) and scripts/build_arch.sh
# (install.sh staging) must produce the same package layout: same paths, modes and
# owners. makepkg runs as the unprivileged builder under fakeroot; --repackage
# packages the release artifacts already built in target/ (no second build).
# $1 = package built by scripts/build_arch.sh.
verify_pkgbuild_parity() {
    local reference="$1" work src
    src="$(pwd)"
    work="$(mktemp -d /tmp/soos-makepkg.XXXXXX)"
    cp packaging/arch/PKGBUILD packaging/arch/soos.install "${work}/"
    # No stripping and no debug split package: the parity is about the layout.
    sed -e 's/^OPTIONS=.*/OPTIONS=(!strip docs !libtool !staticlibs emptydirs zipman purge !debug !lto)/' \
        /etc/makepkg.conf > "${work}/makepkg.conf"
    chown -R "${BUILD_USER}" "${work}"
    info "Building packaging/arch/PKGBUILD with makepkg as '${BUILD_USER}'..."
    (cd "${work}" && runuser -u "${BUILD_USER}" -- env SOOS_SRC_DIR="${src}" \
        PKGDEST="${work}/out" BUILDDIR="${work}/build" SRCDEST="${work}/src" \
        makepkg --config "${work}/makepkg.conf" --noconfirm --nodeps --repackage --force) \
        || { error "makepkg failed on packaging/arch/PKGBUILD"; return 1; }
    local made
    made="$(find "${work}/out" -name 'soos-[0-9]*.pkg.tar.*' | head -n 1)"
    [[ -n "${made}" ]] || { error "makepkg produced no soos package"; return 1; }
    verify_archive_root_owned "${made}" arch
    layout() {
        bsdtar --numeric-owner -tvf "$1" | awk '{print $1, $3, $4, $NF}' \
            | sed -e 's| \./| |' | grep -Ev ' \.(PKGINFO|BUILDINFO|MTREE|INSTALL)$' \
            | sed -e 's|/$||' | sort
    }
    if ! diff -u <(layout "${reference}") <(layout "${made}"); then
        error "PKGBUILD (makepkg) and scripts/build_arch.sh package layouts differ (diff above: - build_arch.sh, + PKGBUILD)."
        return 1
    fi
    rm -rf "${work}"
    success "PKGBUILD (makepkg) and scripts/build_arch.sh produce the same paths, modes and owners."
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

# GitHub #327 (UPG4, UPG6): an upgrade or a reinstall over the installed package keeps the
# master key, an enrolled template (synthetic ciphertext: no camera), /etc/soos/daemon.toml
# and the PAM activation state byte-identical. Digests are compared, never printed.
readonly UPGRADE_TEMPLATE="/var/lib/soos/biometrics/4242.cbor.enc"
UPGRADE_CREATED_CONFIG=false

prepare_upgrade_state() {
    if [[ ! -f /etc/soos/daemon.toml ]]; then
        install -d -m 0755 /etc/soos
        printf '%s\n' '# package upgrade test' '[pipeline]' 'use_mock_camera = true' > /etc/soos/daemon.toml
        chmod 0644 /etc/soos/daemon.toml
        UPGRADE_CREATED_CONFIG=true
    fi
    (umask 077 && head -c 512 /dev/urandom > "${UPGRADE_TEMPLATE}")
}

cleanup_upgrade_state() {
    rm -f "${UPGRADE_TEMPLATE}"
    if [[ "${UPGRADE_CREATED_CONFIG}" = true ]]; then
        rm -f /etc/soos/daemon.toml
        rmdir /etc/soos 2>/dev/null || true
        UPGRADE_CREATED_CONFIG=false
    fi
}

# "<path> <sha256>" lines of every preserved file, then modes and the authselect profile.
upgrade_state_digest() {
    local f
    for f in /var/lib/soos/master.key "${UPGRADE_TEMPLATE}" /etc/soos/daemon.toml; do
        if [[ -f "${f}" ]]; then
            printf '%s %s\n' "${f}" "$(sha256sum < "${f}" | cut -d' ' -f1)"
        else
            printf '%s missing\n' "${f}"
        fi
    done
    while IFS= read -r -d '' f; do
        printf '%s %s\n' "${f}" "$(sha256sum < "${f}" | cut -d' ' -f1)"
    done < <(find /etc/pam.d /var/lib/pam -type f -print0 2>/dev/null | sort -z)
    stat -c '%n %a %U:%G' /var/lib/soos/master.key /var/lib/soos/biometrics 2>/dev/null || true
    if command -v authselect >/dev/null 2>&1; then
        printf 'authselect %s\n' "$(authselect current --raw 2>/dev/null || echo none)"
    fi
}

# $1 = label, then the upgrade command.
verify_upgrade_preserves_state() {
    local label="$1" before after changed
    shift
    before="$(upgrade_state_digest)"
    info "${label}: $*"
    "$@"
    after="$(upgrade_state_digest)"
    if [[ "${after}" != "${before}" ]]; then
        changed="$({ diff <(echo "${before}") <(echo "${after}") || true; } | sed -nE '/^[<>] /{s/[0-9a-f]{64}/<sha256>/;p}')"
        error "${label} changed preserved state (master.key, template, /etc/soos/daemon.toml or /etc/pam.d):"
        sed 's/^/  /' <<< "${changed}" >&2
        return 1
    fi
    grep -qx '/var/lib/soos/master.key 600 root:root' <<< "${after}" \
        || { error "${label}: master.key is not 600 root:root"; return 1; }
    success "${label} kept master.key, ${UPGRADE_TEMPLATE}, /etc/soos/daemon.toml and the PAM activation state byte-identical."
}

# A copy of the .deb with a higher version (same payload and maintainer scripts), so that
# `dpkg -i` runs a real upgrade (prerm upgrade, postinst configure <old-version>) without a
# second package build. Prints the path of the copy.
make_newer_deb() {
    local deb="$1" work version
    work="$(mktemp -d /tmp/soos_upgrade_deb.XXXXXX)"
    dpkg-deb -R "${deb}" "${work}/root"
    version="$(dpkg-deb -f "${deb}" Version)"
    sed -i "s/^Version: .*/Version: ${version}+upgradetest1/" "${work}/root/DEBIAN/control"
    dpkg-deb -b --root-owner-group "${work}/root" "${work}/soos_upgrade.deb" >/dev/null
    printf '%s\n' "${work}/soos_upgrade.deb"
}

case "${DISTRO}" in
    ubuntu|debian)
        info "Running Debian (.deb) package verification..."
        ensure_build_user
        info "Building the .deb as the unprivileged user '${BUILD_USER}'..."
        runuser -u "${BUILD_USER}" -- bash scripts/build_deb.sh --skip-build -o "${PKG_OUT}"

        DEB_FILE=$(ls -t "${PKG_OUT}"/soos_*.deb | head -n 1)
        verify_package_has_no_key_material "${DEB_FILE}" dpkg-deb -c "${DEB_FILE}"
        verify_distro_templates debian dpkg-deb -c "${DEB_FILE}"
        verify_archive_root_owned "${DEB_FILE}" debian
        verify_unit_and_run_layout dpkg-deb -c "${DEB_FILE}"

        info "Installing ${DEB_FILE} via dpkg -i..."
        dpkg -i "${DEB_FILE}"

        PAM_DIR="/lib/x86_64-linux-gnu/security"
        if [[ ! -d "${PAM_DIR}" ]]; then
            PAM_DIR="/lib/security"
        fi

        verify_installation "${PAM_DIR}"
        verify_installed_files_root_owned dpkg -L soos
        FIRST_KEY_FP=$(key_fingerprint)

        info "Upgrade path (GitHub #327): dpkg -i of a newer .deb over the installed one..."
        # The image ships a hand-written common-auth that pam-auth-update treats as locally
        # modified (never rewritten): hand the stack to pam-auth-update for this case, so the
        # profile selection is really exercised, and restore the image's files afterwards.
        PAM_BACKUP=$(mktemp -d /tmp/soos_pam_backup.XXXXXX)
        cp -a /etc/pam.d "${PAM_BACKUP}/pam.d"
        cp -a /var/lib/pam "${PAM_BACKUP}/var-lib-pam"
        pam-auth-update --package --force --enable soos soos-notify
        grep -q 'pam_soos\.so' /etc/pam.d/common-auth \
            || { error "precondition: pam-auth-update did not enable soos in /etc/pam.d/common-auth"; exit 1; }
        prepare_upgrade_state
        UPGRADE_DEB=$(make_newer_deb "${DEB_FILE}")
        verify_upgrade_preserves_state "dpkg -i newer .deb (soos enabled)" dpkg -i "${UPGRADE_DEB}"
        # An administrator's choice survives the upgrade: soos disabled stays disabled.
        pam-auth-update --package --disable soos soos-notify
        if grep -q 'pam_soos\.so' /etc/pam.d/common-auth; then
            error "precondition: pam-auth-update --disable left pam_soos.so in common-auth"; exit 1
        fi
        verify_upgrade_preserves_state "dpkg -i .deb again (soos disabled by the administrator)" dpkg -i "${UPGRADE_DEB}"
        cleanup_upgrade_state
        rm -rf "$(dirname "${UPGRADE_DEB}")"
        rm -rf /etc/pam.d /var/lib/pam
        cp -a "${PAM_BACKUP}/pam.d" /etc/pam.d
        cp -a "${PAM_BACKUP}/var-lib-pam" /var/lib/pam
        rm -rf "${PAM_BACKUP}"

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
        # rpmbuild takes owners from %files (%defattr / %attr), never from the builder.
        verify_archive_root_owned "${RPM_FILE}" fedora
        verify_unit_and_run_layout rpm -qlp --noghost "${RPM_FILE}"

        info "Installing ${RPM_FILE} via rpm -i..."
        rpm -i "${RPM_FILE}"

        PAM_DIR="/usr/lib64/security"
        if [[ ! -d "${PAM_DIR}" ]]; then
            PAM_DIR="/usr/lib/security"
        fi

        verify_installation "${PAM_DIR}"
        verify_installed_files_root_owned rpm -ql soos
        FIRST_KEY_FP=$(key_fingerprint)

        prepare_upgrade_state
        verify_upgrade_preserves_state "rpm -U --replacepkgs (reinstall over the installed package)" \
            rpm -U --replacepkgs "${RPM_FILE}"
        cleanup_upgrade_state

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
        ensure_build_user
        info "Building the Arch package as the unprivileged user '${BUILD_USER}'..."
        runuser -u "${BUILD_USER}" -- bash scripts/build_arch.sh --skip-build -o "${PKG_OUT}"

        PKG_FILE=$(ls -t "${PKG_OUT}"/soos-*.pkg.tar.* | head -n 1)
        verify_package_has_no_key_material "${PKG_FILE}" bsdtar -tf "${PKG_FILE}"
        verify_distro_templates arch bsdtar -tf "${PKG_FILE}"
        verify_archive_root_owned "${PKG_FILE}" arch
        verify_unit_and_run_layout bsdtar -tf "${PKG_FILE}"
        verify_pkgbuild_parity "${PKG_FILE}"

        info "Installing ${PKG_FILE} via pacman -U..."
        pacman -U --noconfirm "${PKG_FILE}"

        verify_installation "/usr/lib/security"
        verify_installed_files_root_owned pacman -Qlq soos
        FIRST_KEY_FP=$(key_fingerprint)

        prepare_upgrade_state
        verify_upgrade_preserves_state "pacman -U (reinstall over the installed package)" \
            pacman -U --noconfirm "${PKG_FILE}"
        cleanup_upgrade_state

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
