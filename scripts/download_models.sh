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
#   --preflight               Validate the manifest and required tools, then exit (no disk writes)
#   -h, --help                Display this help message
#
# Requirements: bash >= 4, coreutils (sha256sum), and curl + ca-certificates for
# https:// sources. No Python: the manifest is parsed in bash (GitHub #167).
#
# Invariants:
#   - The manifest is validated and the tool preflight passes BEFORE any write.
#   - Model filenames must be bare names (no '/', no leading '.'), SHA-256
#     digests exactly 64 hex characters, sources https:// or file:// only.
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
PREFLIGHT=false

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Downloads, verifies (SHA-256), and deploys attested ONNX models to the system.

Options:
  -t, --target-dir <DIR>   Target destination directory (default: ${DEFAULT_TARGET_DIR})
  -m, --manifest <PATH>    Path to models manifest.toml (default: ${DEFAULT_MANIFEST})
  --check-only             Verify integrity of existing deployed models
  --dry-run                Show download plan and checksums without downloading
  --preflight              Validate manifest and required tools only (no writes)
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
        --preflight)
            PREFLIGHT=true
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

# -----------------------------------------------------------------------------
# Tool preflight and SHA-256 helper (no Python dependency, GitHub #167)
# -----------------------------------------------------------------------------
SHA256_TOOL=""
if command -v sha256sum >/dev/null 2>&1; then
    SHA256_TOOL="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
    SHA256_TOOL="shasum"
fi

compute_sha256() {
    local file_path="$1" digest _rest
    if [[ "${SHA256_TOOL}" == "sha256sum" ]]; then
        read -r digest _rest < <(sha256sum -- "${file_path}")
    else
        read -r digest _rest < <(shasum -a 256 -- "${file_path}")
    fi
    printf '%s\n' "${digest}"
}

# -----------------------------------------------------------------------------
# Manifest parser for the fixed models/manifest.toml schema (pure bash)
# -----------------------------------------------------------------------------
# Supported subset: '[models.<id>]' tables whose 'filename', 'sha256',
# 'source_url' and 'license' keys hold single-line, double-quoted strings
# without escapes. Every other table, key and multi-line array is skipped.
# Anything ambiguous for the four keys above fails closed.
M_IDS=()
M_FILES=()
M_SHAS=()
M_URLS=()
M_LICENSES=()

parse_manifest() {
    local manifest="$1"
    local re_model_table='^\[models\.([A-Za-z0-9_-]+)\][[:space:]]*(#.*)?$'
    local re_other_table='^\[\[?[A-Za-z_"]'
    local re_key='^(filename|sha256|source_url|license)[[:space:]]*='
    local re_string='^[a-z0-9_]+[[:space:]]*=[[:space:]]*"([^"\\]*)"[[:space:]]*(#.*)?$'
    local line trimmed key value lineno=0 cur=-1 in_model=false i
    declare -A seen_keys=()

    while IFS= read -r line || [[ -n "${line}" ]]; do
        lineno=$((lineno + 1))
        line="${line%$'\r'}"
        trimmed="${line#"${line%%[![:space:]]*}"}"
        [[ -z "${trimmed}" || "${trimmed}" == \#* ]] && continue

        if [[ "${trimmed}" =~ ${re_model_table} ]]; then
            local id="${BASH_REMATCH[1]}"
            for ((i = 0; i < ${#M_IDS[@]}; i++)); do
                if [[ "${M_IDS[i]}" == "${id}" ]]; then
                    error "Manifest line ${lineno}: duplicate model table [models.${id}]"
                    return 1
                fi
            done
            M_IDS+=("${id}")
            M_FILES+=("")
            M_SHAS+=("")
            M_URLS+=("")
            M_LICENSES+=("")
            cur=$((${#M_IDS[@]} - 1))
            in_model=true
            seen_keys=()
            continue
        fi
        if [[ "${trimmed}" =~ ${re_other_table} ]]; then
            in_model=false
            continue
        fi
        [[ "${in_model}" == true ]] || continue
        [[ "${trimmed}" =~ ${re_key} ]] || continue

        key="${BASH_REMATCH[1]}"
        if [[ ! "${trimmed}" =~ ${re_string} ]]; then
            error "Manifest line ${lineno}: '${key}' must be a single-line double-quoted string"
            return 1
        fi
        value="${BASH_REMATCH[1]}"
        if [[ -n "${seen_keys[${key}]:-}" ]]; then
            error "Manifest line ${lineno}: duplicate key '${key}' in [models.${M_IDS[cur]}]"
            return 1
        fi
        seen_keys[${key}]=1
        case "${key}" in
            filename)   M_FILES[cur]="${value}" ;;
            sha256)     M_SHAS[cur]="${value}" ;;
            source_url) M_URLS[cur]="${value}" ;;
            license)    M_LICENSES[cur]="${value}" ;;
        esac
    done < "${manifest}"

    if [[ ${#M_IDS[@]} -eq 0 ]]; then
        error "Manifest declares no [models.<id>] table: ${manifest}"
        return 1
    fi

    for ((i = 0; i < ${#M_IDS[@]}; i++)); do
        local mid="${M_IDS[i]}"
        # A bare file name only: no path separator, no leading dot (no traversal).
        if [[ ! "${M_FILES[i]}" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
            error "Model '${mid}': invalid or missing filename '${M_FILES[i]}' (bare file name required)"
            return 1
        fi
        if [[ ! "${M_SHAS[i]}" =~ ^[0-9A-Fa-f]{64}$ ]]; then
            error "Model '${mid}': invalid or missing sha256 (64 hexadecimal characters required)"
            return 1
        fi
        if [[ -z "${M_URLS[i]}" ]]; then
            error "Model '${mid}': missing source_url"
            return 1
        fi
    done
    return 0
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
        *)
            # Fallback to source_url joined with filename if applicable
            echo "${source_url}/${filename}"
            ;;
    esac
}


# -----------------------------------------------------------------------------
# 1. Validate the manifest and resolve every download URL (read-only)
# -----------------------------------------------------------------------------
if ! parse_manifest "${MANIFEST_PATH}"; then
    error "Manifest validation failed: ${MANIFEST_PATH}"
    exit 1
fi
TOTAL_MODELS=${#M_IDS[@]}

M_DOWNLOAD_URLS=()
NEEDS_CURL=false
for ((idx = 0; idx < TOTAL_MODELS; idx++)); do
    url="$(resolve_download_url "${M_IDS[idx]}" "${M_URLS[idx]}" "${M_FILES[idx]}")"
    case "${url}" in
        file://*) ;;
        https://*) NEEDS_CURL=true ;;
        *)
            error "Model '${M_IDS[idx]}': unsupported source URL scheme (https:// or file:// required): ${url}"
            exit 1
            ;;
    esac
    M_DOWNLOAD_URLS+=("${url}")
done

# -----------------------------------------------------------------------------
# 2. Tool preflight (before any filesystem mutation)
# -----------------------------------------------------------------------------
if [[ "${DRY_RUN}" = false ]]; then
    MISSING_TOOLS=()
    [[ -n "${SHA256_TOOL}" ]] || MISSING_TOOLS+=("sha256sum (coreutils)")
    if [[ "${CHECK_ONLY}" = false && "${NEEDS_CURL}" = true ]] && ! command -v curl >/dev/null 2>&1; then
        MISSING_TOOLS+=("curl")
    fi
    if [[ ${#MISSING_TOOLS[@]} -gt 0 ]]; then
        error "Missing required tools: ${MISSING_TOOLS[*]}"
        error "Install them first (see scripts/check_build_deps.sh --print-packages models)."
        exit 1
    fi
fi

if [[ "${PREFLIGHT}" = true ]]; then
    success "Preflight passed: ${TOTAL_MODELS} models attested, required tools present."
    exit 0
fi

# -----------------------------------------------------------------------------
# 3. Deploy / verify
# -----------------------------------------------------------------------------
if [[ "${DRY_RUN}" = false && "${CHECK_ONLY}" = false ]]; then
    mkdir -p "${TARGET_DIR}"
    chmod 755 "${TARGET_DIR}"
    if [[ "$(id -u)" -eq 0 ]]; then
        chown root:root "${TARGET_DIR}"
    fi
fi

VERIFIED_MODELS=0

for ((idx = 0; idx < TOTAL_MODELS; idx++)); do
    model_id="${M_IDS[idx]}"
    filename="${M_FILES[idx]}"
    expected_sha="${M_SHAS[idx]}"
    license_name="${M_LICENSES[idx]}"
    download_url="${M_DOWNLOAD_URLS[idx]}"
    dest_path="${TARGET_DIR}/${filename}"

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
        curl -fSL --proto '=https' --proto-redir '=https' --tlsv1.2 \
            --retry 3 --connect-timeout 15 --max-time 900 \
            -o "${tmp_dest}" "${download_url}" || {
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

    chmod 0644 "${tmp_dest}"
    if [[ "$(id -u)" -eq 0 ]]; then
        chown root:root "${tmp_dest}"
    fi
    mv -f "${tmp_dest}" "${dest_path}"

    success "Verified and deployed: ${filename} (SHA-256 match)"
    VERIFIED_MODELS=$((VERIFIED_MODELS + 1))
done

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
