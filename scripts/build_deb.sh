#!/usr/bin/env bash
# =============================================================================
# scripts/build_deb.sh — Debian (.deb) Package Builder for soos
# =============================================================================
# Builds release binaries and packages them into a Debian .deb package
# adhering to Debian standards, systemd integration, and security invariants.
#
# Options:
#   -o, --output-dir <DIR>   Output directory for built .deb (default: target/packages)
#   --skip-build             Skip cargo build step (use existing target/release artifacts)
#   --dry-run                Print build plan without generating package
#   -h, --help               Display help message
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

OUTPUT_DIR="${WORKSPACE_ROOT}/target/packages"
SKIP_BUILD=false
DRY_RUN=false

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Builds the Debian (.deb) distribution package for soos.

Options:
  -o, --output-dir <DIR>   Output directory (default: target/packages)
  --skip-build             Skip cargo compilation (use existing release binaries)
  --dry-run                Display plan without building package
  -h, --help               Display this help message
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -o|--output-dir)
            OUTPUT_DIR="$2"
            shift 2
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
            echo "Unknown option: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

PKG_NAME="soos"
# Version and license come from [workspace.package] in Cargo.toml (GitHub #210).
# shellcheck source=scripts/lib/pkg_meta.sh
source "${WORKSPACE_ROOT}/scripts/lib/pkg_meta.sh"
VERSION="$(soos_pkg_version)"
PKG_LICENSE="$(soos_pkg_license)"
ARCH=$(dpkg --print-architecture 2>/dev/null || uname -m)
case "${ARCH}" in
    x86_64) ARCH="amd64" ;;
    aarch64) ARCH="arm64" ;;
esac

DEB_FILENAME="${PKG_NAME}_${VERSION}_${ARCH}.deb"

echo "=== soos Debian Package Builder ==="
echo "Package:    ${PKG_NAME}"
echo "Version:    ${VERSION}"
echo "License:    ${PKG_LICENSE}"
echo "Arch:       ${ARCH}"
echo "Output:     ${OUTPUT_DIR}/${DEB_FILENAME}"
echo "==================================="

if [[ "${DRY_RUN}" = true ]]; then
    echo "Dry run complete. Exiting."
    exit 0
fi

mkdir -p "${OUTPUT_DIR}"

if [[ "${SKIP_BUILD}" = false ]]; then
    echo "[1/4] Compiling workspace crates in release mode..."
    cargo build --locked --release --workspace
fi

STAGE_DIR=$(mktemp -d "/tmp/soos_deb_stage.XXXXXX")
cleanup() {
    rm -rf "${STAGE_DIR}"
}
trap cleanup EXIT INT TERM

# Debian/Ubuntu PAM loads modules from the multiarch directory only; never let
# install.sh guess from the build host (it would pick /usr/lib64/security).
DEB_HOST_MULTIARCH="$(dpkg-architecture -qDEB_HOST_MULTIARCH 2>/dev/null || gcc -print-multiarch 2>/dev/null || true)"
if [[ -z "${DEB_HOST_MULTIARCH}" ]]; then
    echo "[ERROR] Cannot determine the Debian multiarch triplet (install dpkg-dev)." >&2
    exit 1
fi
DEB_PAM_DIR="/usr/lib/${DEB_HOST_MULTIARCH}/security"

echo "[2/4] Staging package filesystem..."
bash "${WORKSPACE_ROOT}/scripts/install.sh" \
    --destdir "${STAGE_DIR}" \
    --prefix "/usr" \
    --pam-dir "${DEB_PAM_DIR}" \
    --distro debian \
    --skip-models \
    --skip-systemd

# Hard guard (GitHub #144): a package must never contain key material. The
# master key is generated on the target host by the post-install scriptlet.
bash "${WORKSPACE_ROOT}/scripts/check_no_key_material.sh" "${STAGE_DIR}"

echo "[3/4] Configuring DEBIAN metadata and control files..."
mkdir -p "${STAGE_DIR}/DEBIAN"

# Compute installed size in KB
INSTALLED_SIZE=$(du -sk "${STAGE_DIR}" | cut -f1)

cat <<EOF > "${STAGE_DIR}/DEBIAN/control"
Package: ${PKG_NAME}
Version: ${VERSION}
Section: admin
Priority: optional
Architecture: ${ARCH}
Maintainer: soos developers <dev@soos.local>
Depends: libc6, libgcc-s1, libstdc++6, libpam0g, libpam-runtime (>= 1.1.8-1), adduser | passwd, systemd
Recommends: libxkbcommon0, libwayland-client0, libwayland-egl1, libegl1, libgl1, libx11-6, libxcursor1, libxi6, libxrandr2
Installed-Size: ${INSTALLED_SIZE}
Description: Local facial biometric authentication PAM module and daemon
 soos is a zero-trust local facial biometric PAM subsystem for Linux.
 It features deadline-bounded verification, warm camera streaming,
 presentation attack detection (PAD), and encrypted biometric vector storage.
 This package installs the privileged background daemon, user administration
 tools, enrollment CLI, systemd service, and Linux-PAM module.
EOF

cp "${WORKSPACE_ROOT}/packaging/debian/postinst" "${STAGE_DIR}/DEBIAN/postinst"
cp "${WORKSPACE_ROOT}/packaging/debian/prerm" "${STAGE_DIR}/DEBIAN/prerm"
if [[ -f "${WORKSPACE_ROOT}/packaging/debian/postrm" ]]; then
    cp "${WORKSPACE_ROOT}/packaging/debian/postrm" "${STAGE_DIR}/DEBIAN/postrm"
    chmod 0755 "${STAGE_DIR}/DEBIAN/postrm"
fi

chmod 0755 "${STAGE_DIR}/DEBIAN/postinst"
chmod 0755 "${STAGE_DIR}/DEBIAN/prerm"

echo "[4/4] Building .deb archive..."
dpkg-deb --build --root-owner-group "${STAGE_DIR}" "${OUTPUT_DIR}/${DEB_FILENAME}"

echo "Successfully built: ${OUTPUT_DIR}/${DEB_FILENAME}"
exit 0
