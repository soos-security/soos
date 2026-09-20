#!/usr/bin/env bash
# =============================================================================
# scripts/download_models.sh — Machine Learning Model Download and Verification
# =============================================================================
# Downloads, cryptographically verifies (SHA-256), and deploys the attested
# ONNX models cataloged in models/manifest.toml to /var/lib/soos/models/.
#
# Supported Options:
#   -t, --target-dir <DIR>    Target models directory (default: /var/lib/soos/models)
#   -m, --manifest <PATH>     Path to manifest.toml (default: models/manifest.toml)
#   --check-only              Verify integrity of existing deployed models without downloading
#   --dry-run                 Parse manifest and display download actions without modifying disk
#   -h, --help                Display this help message
#
# Invariants:
#   - Every model file must strictly match its attested SHA-256 checksum.
#   - Files are installed with permissions mode 0644 (root:root if executed as root).
#   - Directory is created with mode 0755 (root:root if executed as root).
#   - manifest.toml is copied to <target-dir>/manifest.toml upon completion.
#   - Mismatched or corrupted downloads fail loudly and are removed immediately.
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

# Determine default paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

DEFAULT_TARGET_DIR="${SOOS_MODELS_DIR:-/var/lib/soos/models}"
DEFAULT_MANIFEST="${SOOS_MANIFEST_PATH:-${WORKSPACE_ROOT}/models/manifest.toml}"

TARGET_DIR="${DEFAULT_TARGET_DIR}"
MANIFEST_PATH="${DEFAULT_MANIFEST}"
CHECK_ONLY=false
DRY_RUN=false

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Downloads, verifies (SHA-256), and deploys attested ONNX models to the system.

Options:
  -t, --target-dir <DIR>   Target destination directory (default: ${DEFAULT_TARGET_DIR})
  -m, --manifest <PATH>    Path to models manifest.toml (default: ${DEFAULT_MANIFEST})
  --check-only             Verify integrity of existing deployed models
  --dry-run                Show download plan and checksums without downloading
  -h, --help               Show this help message and exit

Environment Variables:
  SOOS_MODELS_DIR          Override default target directory
  SOOS_MANIFEST_PATH       Override default manifest.toml path
EOF
}

# Parse command line options
while [[ $# -gt 0 ]]; do
    case "$1" in
        -t|--target-dir)
            TARGET_DIR="$2"
            shift 2
            ;;
        -m|--manifest)
            MANIFEST_PATH="$2"
            shift 2
            ;;
        --check-only)
            CHECK_ONLY=true
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
            error "Unknown option: $1"
            usage >&2
            exit 1
            ;;
    esac
done

if [[ ! -f "${MANIFEST_PATH}" ]]; then
    error "Manifest file not found at: ${MANIFEST_PATH}"
    exit 1
fi

info "Using manifest: ${MANIFEST_PATH}"
info "Target directory: ${TARGET_DIR}"

# Compute SHA-256 helper
compute_sha256() {
    local file_path="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "${file_path}" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "${file_path}" | awk '{print $1}'
    else
        python3 -c "import hashlib, sys; print(hashlib.sha256(open(sys.argv[1], 'rb').read()).hexdigest())" "${file_path}"
    fi
}

# Python helper to parse TOML manifest into tab-delimited records:
# <id>\t<filename>\t<sha256>\t<source_url>\t<license>
parse_manifest() {
    python3 -c "
import sys
try:
    import tomllib
except ImportError:
    try:
        import tomli as tomllib
    except ImportError:
        import tomllib_fallback
with open(sys.argv[1], 'rb') as f:
    data = tomllib.load(f)
models = data.get('models', {})
for model_id, meta in models.items():
    filename = meta.get('filename', '')
    sha256 = meta.get('sha256', '')
    source_url = meta.get('source_url', '')
    license_ = meta.get('license', '')
    print(f'{model_id}\t{filename}\t{sha256}\t{source_url}\t{license_}')
" "${MANIFEST_PATH}"
}

# Fallback direct download URLs for standard model repositories
resolve_download_url() {
    local model_id="$1"
    local source_url="$2"
    local filename="$3"

    # If source_url already points directly to a file (.onnx or file://)
    if [[ "${source_url}" =~ \.onnx$ || "${source_url}" =~ ^file:// ]]; then
        echo "${source_url}"
        return
    fi

    # Known upstream direct download locations
    case "${model_id}" in
        scrfd_500m_kps)
            echo "https://huggingface.co/ykk648/face_lib/resolve/main/face_detect/scrfd_onnx/scrfd_500m_bnkps.onnx"
            ;;
        arcface_w600k_mbf)
            echo "https://huggingface.co/garavv/arcface-onnx/resolve/main/arc.onnx"
            ;;
        minifasnet_v2_pad)
            echo "https://github.com/QingHeYang/Silent-Face-Anti-Spoofing-onnx/raw/main/onnx/2.7_80x80_MiniFASNetV2.onnx"
            ;;
        ultraface_slim_320)
            echo "https://raw.githubusercontent.com/Linzaer/Ultra-Light-Fast-Generic-Face-Detector-1MB/master/models/onnx/version-slim-320.onnx"
            ;;
        landmark_5point)
            echo "https://raw.githubusercontent.com/deepinsight/insightface/master/alignment/coordinate_regress/model/landmark_5point.onnx"
            ;;
        mobilefacenet_arcface)
            echo "https://raw.githubusercontent.com/sirius-ai/MobileFaceNet_TF/master/arch/mobilefacenet_arcface.onnx"
            ;;
        minifasnet_pad)
            echo "https://raw.githubusercontent.com/minivision-ai/Silent-Face-Anti-Spoofing/master/resources/anti_spoof_models/minifasnet_pad.onnx"
            ;;
        *)
            # Fallback to source_url joined with filename if applicable
            echo "${source_url}/${filename}"
            ;;
    esac
}

# Verify target directory creation
if [[ "${DRY_RUN}" = false && "${CHECK_ONLY}" = false ]]; then
    mkdir -p "${TARGET_DIR}"
    chmod 755 "${TARGET_DIR}"
    if [[ "$(id -u)" -eq 0 ]]; then
        chown root:root "${TARGET_DIR}"
    fi
fi

PARSED_MODELS=$(parse_manifest)
TOTAL_MODELS=0
VERIFIED_MODELS=0

while IFS=$'\t' read -r model_id filename expected_sha source_url license_name; do
    [[ -z "${model_id}" ]] && continue
    TOTAL_MODELS=$((TOTAL_MODELS + 1))

    dest_path="${TARGET_DIR}/${filename}"
    download_url=$(resolve_download_url "${model_id}" "${source_url}" "${filename}")

    if [[ "${DRY_RUN}" = true ]]; then
        info "[DRY-RUN] Model '${model_id}':"
        info "          File:        ${filename}"
        info "          SHA-256:     ${expected_sha}"
        info "          License:     ${license_name}"
        info "          Source:      ${download_url}"
        continue
    fi

    if [[ "${CHECK_ONLY}" = true ]]; then
        if [[ ! -f "${dest_path}" ]]; then
            error "Model file not found: ${dest_path}"
            exit 1
        fi
        actual_sha=$(compute_sha256 "${dest_path}")
        if [[ "${actual_sha,,}" != "${expected_sha,,}" ]]; then
            error "SHA-256 checksum mismatch for '${model_id}' at ${dest_path}!"
            error "  Expected: ${expected_sha}"
            error "  Actual:   ${actual_sha}"
            exit 1
        fi
        success "Attestation verified: ${filename} (${expected_sha:0:16}...)"
        VERIFIED_MODELS=$((VERIFIED_MODELS + 1))
        continue
    fi

    # Check if target already exists and satisfies checksum
    if [[ -f "${dest_path}" ]]; then
        existing_sha=$(compute_sha256 "${dest_path}")
        if [[ "${existing_sha,,}" == "${expected_sha,,}" ]]; then
            success "Already deployed and verified: ${filename}"
            VERIFIED_MODELS=$((VERIFIED_MODELS + 1))
            continue
        else
            warn "Existing file ${filename} has mismatched checksum. Re-downloading..."
        fi
    fi

    info "Acquiring model '${model_id}' (${filename})..."
    tmp_dest="${dest_path}.tmp.$$"

    if [[ "${download_url}" =~ ^file:// ]]; then
        local_src_path="${download_url#file://}"
        if [[ ! -f "${local_src_path}" ]]; then
            error "Local source file not found: ${local_src_path}"
            exit 1
        fi
        cp "${local_src_path}" "${tmp_dest}"
    else
        info "Downloading from: ${download_url}"
        curl -fSL --retry 3 --connect-timeout 15 -o "${tmp_dest}" "${download_url}" || {
            error "Failed to download model from ${download_url}"
            rm -f "${tmp_dest}"
            exit 1
        }
    fi

    actual_sha=$(compute_sha256 "${tmp_dest}")
    if [[ "${actual_sha,,}" != "${expected_sha,,}" ]]; then
        rm -f "${tmp_dest}"
        error "═════════════════════════════════════════════════════════════"
        error "SECURITY VIOLATION: SHA-256 CHECKSUM MISMATCH!"
        error "Model:    ${model_id} (${filename})"
        error "Expected: ${expected_sha}"
        error "Actual:   ${actual_sha}"
        error "The downloaded file was discarded fail-closed."
        error "═════════════════════════════════════════════════════════════"
        exit 1
    fi

    mv "${tmp_dest}" "${dest_path}"
    chmod 0644 "${dest_path}"
    if [[ "$(id -u)" -eq 0 ]]; then
        chown root:root "${dest_path}"
    fi

    success "Verified and deployed: ${filename} (SHA-256 match)"
    VERIFIED_MODELS=$((VERIFIED_MODELS + 1))
done <<< "${PARSED_MODELS}"

if [[ "${DRY_RUN}" = true ]]; then
    success "Dry run complete. ${TOTAL_MODELS} models cataloged."
    exit 0
fi

# Copy manifest.toml to destination directory
if [[ "${CHECK_ONLY}" = false ]]; then
    target_manifest="${TARGET_DIR}/manifest.toml"
    cp "${MANIFEST_PATH}" "${target_manifest}"
    chmod 0644 "${target_manifest}"
    if [[ "$(id -u)" -eq 0 ]]; then
        chown root:root "${target_manifest}"
    fi
    success "Attestation manifest deployed to: ${target_manifest}"
fi

echo ""
success "==================================================================="
success "  All ${VERIFIED_MODELS}/${TOTAL_MODELS} models verified and deployed to ${TARGET_DIR}"
success "==================================================================="
exit 0
