#!/usr/bin/env bash
# =============================================================================
# scripts/build_arch.sh — Arch Linux Package Builder for soos
# =============================================================================
# Builds release binaries and packages them into an Arch Linux package (.pkg.tar.zst)
# using makepkg or direct tar.zst archive packaging.
#
# Options:
#   -o, --output-dir <DIR>   Output directory for built package (default: target/packages)
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

Builds the Arch Linux (.pkg.tar.zst) distribution package for soos.

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
VERSION="0.1.0"
RELEASE="1"
ARCH=$(uname -m)

PKG_FILENAME="${PKG_NAME}-${VERSION}-${RELEASE}-${ARCH}.pkg.tar.zst"

echo "=== soos Arch Linux Package Builder ==="
echo "Package:    ${PKG_NAME}"
echo "Version:    ${VERSION}-${RELEASE}"
echo "Arch:       ${ARCH}"
echo "Output:     ${OUTPUT_DIR}/${PKG_FILENAME}"
echo "======================================="

if [[ "${DRY_RUN}" = true ]]; then
    echo "Dry run complete. Exiting."
    exit 0
fi

mkdir -p "${OUTPUT_DIR}"

if [[ "${SKIP_BUILD}" = false ]]; then
    echo "[1/4] Compiling workspace crates in release mode..."
    cargo build --release --workspace
fi

ARCH_STAGE=$(mktemp -d "/tmp/soos_arch_stage.XXXXXX")
cleanup() {
    rm -rf "${ARCH_STAGE}"
}
trap cleanup EXIT INT TERM

echo "[2/4] Staging package filesystem..."
bash "${WORKSPACE_ROOT}/scripts/install.sh" \
    --destdir "${ARCH_STAGE}" \
    --prefix "/usr" \
    --skip-models \
    --skip-systemd

# Hard guard (GitHub #144): a package must never contain key material. The
# master key is generated on the target host by the post-install scriptlet.
bash "${WORKSPACE_ROOT}/scripts/check_no_key_material.sh" "${ARCH_STAGE}"

echo "[3/4] Generating Arch Linux package metadata..."
# Create .PKGINFO
BUILD_DATE=$(date +%s)
SIZE=$(du -sb "${ARCH_STAGE}" | cut -f1)

cat <<EOF > "${ARCH_STAGE}/.PKGINFO"
pkgname = ${PKG_NAME}
pkgver = ${VERSION}-${RELEASE}
pkgdesc = Local facial biometric authentication PAM module and daemon for Linux
url = https://github.com/Mysticaly622/soos
builddate = ${BUILD_DATE}
packager = soos developers <dev@soos.local>
size = ${SIZE}
arch = ${ARCH}
license = Apache-2.0
license = MIT
depend = pam
depend = systemd
EOF

# Copy install scriptlet as .INSTALL
cp "${WORKSPACE_ROOT}/packaging/arch/soos.install" "${ARCH_STAGE}/.INSTALL"

echo "[4/4] Creating package archive..."
if command -v bsdtar >/dev/null 2>&1; then
    TAR_BIN="bsdtar"
else
    TAR_BIN="tar"
fi

if command -v zstd >/dev/null 2>&1; then
    (cd "${ARCH_STAGE}" && ${TAR_BIN} -cf - .PKGINFO .INSTALL * | zstd -c -T0 -19 > "${OUTPUT_DIR}/${PKG_FILENAME}")
else
    # Fallback to standard gzip if zstd is absent
    PKG_FILENAME="${PKG_NAME}-${VERSION}-${RELEASE}-${ARCH}.pkg.tar.gz"
    (cd "${ARCH_STAGE}" && ${TAR_BIN} -czf "${OUTPUT_DIR}/${PKG_FILENAME}" .PKGINFO .INSTALL *)
fi

echo "Successfully built Arch package: ${OUTPUT_DIR}/${PKG_FILENAME}"
exit 0
