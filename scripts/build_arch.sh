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
#   --tar <bsdtar|tar>       Archiver (default: bsdtar when installed, else GNU tar)
#   --compress <auto|zstd|gzip>
#                            Compression (default auto: zstd when installed, else gzip)
#   --dry-run                Print build plan without generating package
#   -h, --help               Display help message
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

OUTPUT_DIR="${WORKSPACE_ROOT}/target/packages"
SKIP_BUILD=false
DRY_RUN=false
TAR_BIN=""
COMPRESS="auto"

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Builds the Arch Linux (.pkg.tar.zst) distribution package for soos.

Options:
  -o, --output-dir <DIR>   Output directory (default: target/packages)
  --skip-build             Skip cargo compilation (use existing release binaries)
  --tar <bsdtar|tar>       Archiver (default: bsdtar when installed, else GNU tar)
  --compress <auto|zstd|gzip>
                           Compression (default auto: zstd when installed, else gzip)
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
        --tar)
            TAR_BIN="${2:-}"
            shift 2
            ;;
        --compress)
            COMPRESS="${2:-}"
            shift 2
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
RELEASE="1"
ARCH=$(uname -m)

PKG_FILENAME="${PKG_NAME}-${VERSION}-${RELEASE}-${ARCH}.pkg.tar.zst"

echo "=== soos Arch Linux Package Builder ==="
echo "Package:    ${PKG_NAME}"
echo "Version:    ${VERSION}-${RELEASE}"
echo "License:    ${PKG_LICENSE}"
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
    cargo build --locked --release --workspace
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
    --pam-dir "/usr/lib/security" \
    --unitdir /usr/lib/systemd/system \
    --distro arch \
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
license = ${PKG_LICENSE}
depend = pam
depend = systemd
EOF

# Copy install scriptlet as .INSTALL
cp "${WORKSPACE_ROOT}/packaging/arch/soos.install" "${ARCH_STAGE}/.INSTALL"

echo "[4/4] Creating package archive..."
if [[ -z "${TAR_BIN}" ]]; then
    if command -v bsdtar >/dev/null 2>&1; then
        TAR_BIN="bsdtar"
    else
        TAR_BIN="tar"
    fi
fi
case "${TAR_BIN}" in
    bsdtar|tar) ;;
    *)
        echo "Error: --tar must be bsdtar or tar (got '${TAR_BIN}')." >&2
        exit 1
        ;;
esac
# GitHub #301: pacman extracts the owners recorded in the archive, so every entry
# must be root:root whoever runs this script (a builder-owned soos-daemon or
# pam_soos.so would let that user replace the root daemon or the PAM module).
if "${TAR_BIN}" --version 2>/dev/null | grep -q 'GNU tar'; then
    TAR_OWNER_FLAGS=(--owner=root:0 --group=root:0)
else
    TAR_OWNER_FLAGS=(--uid 0 --gid 0 --uname root --gname root)
fi
if [[ "${COMPRESS}" == "auto" ]]; then
    if command -v zstd >/dev/null 2>&1; then
        COMPRESS="zstd"
    else
        COMPRESS="gzip"
    fi
fi

if [[ "${COMPRESS}" == "zstd" ]]; then
    (cd "${ARCH_STAGE}" && ${TAR_BIN} "${TAR_OWNER_FLAGS[@]}" -cf - .PKGINFO .INSTALL * | zstd -q -c -T0 -19 > "${OUTPUT_DIR}/${PKG_FILENAME}")
elif [[ "${COMPRESS}" == "gzip" ]]; then
    # Fallback to standard gzip if zstd is absent
    PKG_FILENAME="${PKG_NAME}-${VERSION}-${RELEASE}-${ARCH}.pkg.tar.gz"
    (cd "${ARCH_STAGE}" && ${TAR_BIN} "${TAR_OWNER_FLAGS[@]}" -czf "${OUTPUT_DIR}/${PKG_FILENAME}" .PKGINFO .INSTALL *)
else
    echo "Error: --compress must be auto, zstd or gzip (got '${COMPRESS}')." >&2
    exit 1
fi

echo "Successfully built Arch package: ${OUTPUT_DIR}/${PKG_FILENAME}"
exit 0
