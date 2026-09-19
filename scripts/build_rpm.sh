#!/usr/bin/env bash
# =============================================================================
# scripts/build_rpm.sh — RPM Package Builder for soos
# =============================================================================
# Builds release binaries and packages them into an RPM package (.rpm)
# for Fedora, RHEL, CentOS, and compatible Linux distributions.
#
# Options:
#   -o, --output-dir <DIR>   Output directory for built .rpm (default: target/packages)
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

Builds the RPM (.rpm) distribution package for soos.

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
ARCH=$(uname -m)

echo "=== soos RPM Package Builder ==="
echo "Package:    ${PKG_NAME}"
echo "Version:    ${VERSION}"
echo "Arch:       ${ARCH}"
echo "Output:     ${OUTPUT_DIR}"
echo "================================"

if [[ "${DRY_RUN}" = true ]]; then
    echo "Dry run complete. Exiting."
    exit 0
fi

if ! command -v rpmbuild >/dev/null 2>&1; then
    echo "Error: 'rpmbuild' is required but not installed." >&2
    echo "Install rpm-build package (e.g. dnf install rpm-build or apt install rpm)." >&2
    exit 1
fi

mkdir -p "${OUTPUT_DIR}"

if [[ "${SKIP_BUILD}" = false ]]; then
    echo "[1/4] Compiling workspace crates in release mode..."
    cargo build --release --workspace
fi

RPM_ROOT=$(mktemp -d "/tmp/soos_rpmbuild.XXXXXX")
cleanup() {
    rm -rf "${RPM_ROOT}"
}
trap cleanup EXIT INT TERM

mkdir -p "${RPM_ROOT}"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

echo "[2/4] Archiving source tree..."
tar --exclude="./target" --exclude="./.git" \
    --transform="s,^\.,${PKG_NAME}-${VERSION}," \
    -czf "${RPM_ROOT}/SOURCES/${PKG_NAME}-${VERSION}.tar.gz" -C "${WORKSPACE_ROOT}" .

echo "[3/4] Preparing spec file..."
cp "${WORKSPACE_ROOT}/packaging/rpm/soos.spec" "${RPM_ROOT}/SPECS/soos.spec"

echo "[4/4] Executing rpmbuild..."
rpmbuild -bb \
    --define "_topdir ${RPM_ROOT}" \
    --define "_tmppath ${RPM_ROOT}/tmp" \
    "${RPM_ROOT}/SPECS/soos.spec"

echo "Copying built RPMs to ${OUTPUT_DIR}..."
find "${RPM_ROOT}/RPMS" -type f -name "*.rpm" -exec cp {} "${OUTPUT_DIR}/" \;

echo "Successfully built RPM packages in ${OUTPUT_DIR}:"
ls -la "${OUTPUT_DIR}"/*.rpm 2>/dev/null || true
exit 0
