#!/usr/bin/env bash
# =============================================================================
# scripts/build_packages.sh — Master Unified Package Builder for soos
# =============================================================================
# Builds distribution packages (.deb, .rpm, .pkg.tar.zst) for Linux distributions.
#
# Usage:
#   ./scripts/build_packages.sh [TARGET] [OPTIONS]
#
# Targets:
#   all                      Build all supported packages (deb, rpm, arch)
#   deb                      Build Debian / Ubuntu (.deb) package
#   rpm                      Build Fedora / RHEL (.rpm) package
#   arch                     Build Arch Linux (.pkg.tar.zst) package
#
# Default target: deb
#
# Options:
#   -o, --output-dir <DIR>   Destination directory for generated packages (default: target/packages)
#   --skip-build             Skip cargo release build (use existing release binaries)
#   --dry-run                Print execution plan without building packages
#   -h, --help               Display this help message
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

TARGET=""
OUTPUT_DIR="${WORKSPACE_ROOT}/target/packages"
SKIP_BUILD=false
DRY_RUN=false
PASSTHROUGH_ARGS=()

usage() {
    cat <<EOF
Usage: $(basename "$0") [TARGET] [OPTIONS]

Master distribution package builder for soos (deb, rpm, PKGBUILD).

Targets:
  all                      Build all packages (deb, rpm, arch)
  deb                      Build Debian / Ubuntu (.deb) package
  rpm                      Build Fedora / RHEL (.rpm) package
  arch                     Build Arch Linux (.pkg.tar.zst) package

Options:
  -o, --output-dir <DIR>   Output directory (default: target/packages)
  --skip-build             Skip cargo build step
  --dry-run                Display plan without building packages
  -h, --help               Display this help message
EOF
}

# Parse TARGET if specified as first argument
if [[ $# -gt 0 && "$1" != -* ]]; then
    TARGET="$1"
    shift
fi

while [[ $# -gt 0 ]]; do
    case "$1" in
        -o|--output-dir)
            OUTPUT_DIR="$2"
            PASSTHROUGH_ARGS+=("-o" "$2")
            shift 2
            ;;
        --skip-build)
            SKIP_BUILD=true
            PASSTHROUGH_ARGS+=("--skip-build")
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            PASSTHROUGH_ARGS+=("--dry-run")
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

if [[ -z "${TARGET}" ]]; then
    TARGET="deb"
fi

echo "==================================================================="
echo "  soos Master Distribution Package Builder"
echo "==================================================================="
echo "Target:     ${TARGET}"
echo "Output:     ${OUTPUT_DIR}"
echo "Skip Build: ${SKIP_BUILD}"
echo "Dry Run:    ${DRY_RUN}"
echo "==================================================================="

if [[ "${DRY_RUN}" = true ]]; then
    echo "Dry run complete. No packages built."
    exit 0
fi

# 1. Compile workspace crates once upfront if not skipped
if [[ "${SKIP_BUILD}" = false ]]; then
    echo "[Build] Compiling workspace crates in release mode..."
    cargo build --release --workspace
    # Subsequent target builders can reuse the compiled artifacts
    PASSTHROUGH_ARGS+=("--skip-build")
fi

mkdir -p "${OUTPUT_DIR}"

run_deb() {
    echo ""
    echo "--- Building Debian / Ubuntu (.deb) Package ---"
    bash "${WORKSPACE_ROOT}/scripts/build_deb.sh" "${PASSTHROUGH_ARGS[@]}"
}

run_rpm() {
    echo ""
    echo "--- Building Fedora / RHEL (.rpm) Package ---"
    if command -v rpmbuild >/dev/null 2>&1; then
        bash "${WORKSPACE_ROOT}/scripts/build_rpm.sh" "${PASSTHROUGH_ARGS[@]}"
    else
        echo "Notice: rpmbuild not installed on this host. Skipping RPM build."
    fi
}

run_arch() {
    echo ""
    echo "--- Building Arch Linux Package ---"
    bash "${WORKSPACE_ROOT}/scripts/build_arch.sh" "${PASSTHROUGH_ARGS[@]}"
}

case "${TARGET}" in
    deb)
        run_deb
        ;;
    rpm)
        run_rpm
        ;;
    arch)
        run_arch
        ;;
    all)
        run_deb
        run_rpm
        run_arch
        ;;
    *)
        echo "Error: Unknown target '${TARGET}'. Supported: all, deb, rpm, arch." >&2
        exit 1
        ;;
esac

echo ""
echo "==================================================================="
echo "  Packaging process complete."
echo "  Artifacts located in: ${OUTPUT_DIR}"
echo "==================================================================="
ls -la "${OUTPUT_DIR}" 2>/dev/null || true
exit 0
