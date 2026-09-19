#!/usr/bin/env bash
# =============================================================================
# scripts/install.sh — System Provisioning and Installation for soos
# =============================================================================
# Provisions system groups, directory hierarchies with strict permissions,
# compiles/installs binaries, installs PAM shared library, installs systemd unit,
# generates master encryption key if absent, and verifies neural models.
#
# Supported Options:
#   -d, --destdir <DIR>      Target staging root directory (default: /)
#   --prefix <DIR>           Installation prefix (default: /usr)
#   --sysconfdir <DIR>       Configuration directory (default: /etc)
#   --localstatedir <DIR>    State directory (default: /var)
#   --runstatedir <DIR>      Runtime directory (default: /run)
#   --pam-dir <DIR>          Explicit PAM module directory (overrides auto-detection)
#   --skip-models            Skip machine learning model downloading/verification
#   --skip-systemd           Skip systemctl reload/enable operations
#   --dry-run                Display installation plan without modifying filesystem
#   -h, --help               Display this help message
#
# Security Invariants:
#   - /var/lib/soos/{biometrics,evidence} created mode 0700 (root:root)
#   - /var/lib/soos/master.key created mode 0600 (root:root, 32 bytes)
#   - /run/soos created mode 0750 (root:soos)
#   - Binaries installed mode 0755
#   - PAM module installed mode 0644
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

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

DESTDIR=""
PREFIX="/usr"
SYSCONFDIR="/etc"
LOCALSTATEDIR="/var"
RUNSTATEDIR="/run"
PAM_DIR=""
SKIP_MODELS=false
SKIP_SYSTEMD=false
DRY_RUN=false

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Installs and provisions soos Local Facial Biometric PAM module and daemon.

Options:
  -d, --destdir <DIR>      Staging destination directory (default: /)
  --prefix <DIR>           Installation prefix (default: /usr)
  --sysconfdir <DIR>       Configuration directory (default: /etc)
  --localstatedir <DIR>    State directory (default: /var)
  --runstatedir <DIR>      Runtime directory (default: /run)
  --pam-dir <DIR>          Explicit PAM module directory (auto-detected if omitted)
  --skip-models            Skip model download and verification
  --skip-systemd           Skip systemctl reload and enable invocations
  --dry-run                Print plan without modifying filesystem
  -h, --help               Display this help message and exit
EOF
}

# Parse command line options
while [[ $# -gt 0 ]]; do
    case "$1" in
        -d|--destdir)
            DESTDIR="$2"
            shift 2
            ;;
        --prefix)
            PREFIX="$2"
            shift 2
            ;;
        --sysconfdir)
            SYSCONFDIR="$2"
            shift 2
            ;;
        --localstatedir)
            LOCALSTATEDIR="$2"
            shift 2
            ;;
        --runstatedir)
            RUNSTATEDIR="$2"
            shift 2
            ;;
        --pam-dir)
            PAM_DIR="$2"
            shift 2
            ;;
        --skip-models)
            SKIP_MODELS=true
            shift
            ;;
        --skip-systemd)
            SKIP_SYSTEMD=true
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

# Clean DESTDIR trailing slash
DESTDIR="${DESTDIR%/}"

# Auto-detect PAM security directory if not specified
if [[ -z "${PAM_DIR}" ]]; then
    for candidate in \
        "/lib/x86_64-linux-gnu/security" \
        "/lib/aarch64-linux-gnu/security" \
        "/usr/lib64/security" \
        "/lib64/security" \
        "/usr/lib/security" \
        "/lib/security"; do
        if [[ -d "${DESTDIR}${candidate}" || ( -z "${DESTDIR}" && -d "${candidate}" ) ]]; then
            PAM_DIR="${candidate}"
            break
        fi
    done
    if [[ -z "${PAM_DIR}" ]]; then
        # Default fallback
        if [[ -d "/usr/lib64" ]]; then
            PAM_DIR="/usr/lib64/security"
        else
            PAM_DIR="/lib/security"
        fi
    fi
fi

TARGET_BIN_DIR="${DESTDIR}${PREFIX}/bin"
TARGET_LIBEXEC_DIR="${DESTDIR}${PREFIX}/libexec/soos"
TARGET_PAM_DIR="${DESTDIR}${PAM_DIR}"
TARGET_SYSTEMD_DIR="${DESTDIR}${SYSCONFDIR}/systemd/system"
TARGET_STATE_DIR="${DESTDIR}${LOCALSTATEDIR}/lib/soos"
TARGET_RUN_DIR="${DESTDIR}${RUNSTATEDIR}/soos"

info "=== soos Installation Plan ==="
info "Destination root:   ${DESTDIR:-/}"
info "Binaries:           ${TARGET_BIN_DIR}"
info "Daemon libexec:     ${TARGET_LIBEXEC_DIR}"
info "PAM Module:         ${TARGET_PAM_DIR}/pam_soos.so"
info "Systemd Unit:       ${TARGET_SYSTEMD_DIR}/soos-daemon.service"
info "State Directory:    ${TARGET_STATE_DIR}"
info "Runtime Directory:  ${TARGET_RUN_DIR}"
info "==============================="

if [[ "${DRY_RUN}" = true ]]; then
    success "Dry run complete. No modifications made to system."
    exit 0
fi

# 1. Create System Group 'soos'
if [[ -z "${DESTDIR}" ]]; then
    if getent group soos >/dev/null 2>&1; then
        info "System group 'soos' already exists."
    else
        info "Creating system group 'soos'..."
        if command -v groupadd >/dev/null 2>&1; then
            groupadd -r soos
            success "Created system group 'soos'."
        elif command -v addgroup >/dev/null 2>&1; then
            addgroup --system soos
            success "Created system group 'soos'."
        else
            warn "Neither groupadd nor addgroup found. Please ensure 'soos' group is created."
        fi
    fi
fi

# 2. Provision Directory Hierarchy with Invariant Permissions
info "Provisioning directories..."
mkdir -p "${TARGET_STATE_DIR}/biometrics"
mkdir -p "${TARGET_STATE_DIR}/evidence"
mkdir -p "${TARGET_STATE_DIR}/models"
mkdir -p "${TARGET_RUN_DIR}"
mkdir -p "${TARGET_LIBEXEC_DIR}"
mkdir -p "${TARGET_BIN_DIR}"
mkdir -p "${TARGET_PAM_DIR}"
mkdir -p "${TARGET_SYSTEMD_DIR}"

# Set permissions
chmod 0755 "${TARGET_STATE_DIR}"
chmod 0700 "${TARGET_STATE_DIR}/biometrics"
chmod 0700 "${TARGET_STATE_DIR}/evidence"
chmod 0755 "${TARGET_STATE_DIR}/models"
chmod 0750 "${TARGET_RUN_DIR}"
chmod 0755 "${TARGET_LIBEXEC_DIR}"
chmod 0755 "${TARGET_BIN_DIR}"
chmod 0755 "${TARGET_PAM_DIR}"

if [[ "$(id -u)" -eq 0 && -z "${DESTDIR}" ]]; then
    chown root:root "${TARGET_STATE_DIR}"
    chown root:root "${TARGET_STATE_DIR}/biometrics"
    chown root:root "${TARGET_STATE_DIR}/evidence"
    chown root:root "${TARGET_STATE_DIR}/models"
    chown root:soos "${TARGET_RUN_DIR}" || chown root:root "${TARGET_RUN_DIR}"
fi
success "Directories provisioned with verified permissions."

# 3. Generate Master Key if Absent
MASTER_KEY_FILE="${TARGET_STATE_DIR}/master.key"
if [[ ! -f "${MASTER_KEY_FILE}" ]]; then
    info "Generating cryptographic master key (32 bytes)..."
    if command -v openssl >/dev/null 2>&1; then
        openssl rand 32 > "${MASTER_KEY_FILE}"
    else
        head -c 32 /dev/urandom > "${MASTER_KEY_FILE}"
    fi
    chmod 0600 "${MASTER_KEY_FILE}"
    if [[ "$(id -u)" -eq 0 && -z "${DESTDIR}" ]]; then
        chown root:root "${MASTER_KEY_FILE}"
    fi
    success "Master key generated at ${MASTER_KEY_FILE} (mode 0600)."
else
    info "Master key already present at ${MASTER_KEY_FILE}."
fi

# 4. Install Binaries and Shared Library
# Locate artifacts in release or debug targets
find_artifact() {
    local name="$1"
    local candidates=(
        "${WORKSPACE_ROOT}/target/release/${name}"
        "${WORKSPACE_ROOT}/target/debug/${name}"
    )
    for c in "${candidates[@]}"; do
        if [[ -f "${c}" ]]; then
            echo "${c}"
            return 0
        fi
    done
    return 1
}

info "Installing executable binaries..."
DAEMON_BIN=$(find_artifact "soos-daemon" || true)
if [[ -n "${DAEMON_BIN}" ]]; then
    install -m 0755 "${DAEMON_BIN}" "${TARGET_LIBEXEC_DIR}/soos-daemon"
    success "Installed ${TARGET_LIBEXEC_DIR}/soos-daemon"
else
    warn "soos-daemon binary not built yet. Skipped."
fi

ADMIN_BIN=$(find_artifact "soos-admin" || true)
if [[ -n "${ADMIN_BIN}" ]]; then
    install -m 0755 "${ADMIN_BIN}" "${TARGET_BIN_DIR}/soos-admin"
    success "Installed ${TARGET_BIN_DIR}/soos-admin"
else
    warn "soos-admin binary not built yet. Skipped."
fi

ENROLL_BIN=$(find_artifact "soos-enroll" || true)
if [[ -n "${ENROLL_BIN}" ]]; then
    install -m 0755 "${ENROLL_BIN}" "${TARGET_BIN_DIR}/soos-enroll"
    success "Installed ${TARGET_BIN_DIR}/soos-enroll"
else
    warn "soos-enroll binary not built yet. Skipped."
fi

PAM_LIB=$(find_artifact "libpam_soos.so" || true)
if [[ -n "${PAM_LIB}" ]]; then
    install -m 0644 "${PAM_LIB}" "${TARGET_PAM_DIR}/pam_soos.so"
    success "Installed ${TARGET_PAM_DIR}/pam_soos.so"
else
    warn "libpam_soos.so artifact not built yet. Skipped."
fi

# 5. Install Systemd Service Unit
SERVICE_SRC="${WORKSPACE_ROOT}/packaging/soos-daemon.service"
if [[ -f "${SERVICE_SRC}" ]]; then
    install -m 0644 "${SERVICE_SRC}" "${TARGET_SYSTEMD_DIR}/soos-daemon.service"
    success "Installed ${TARGET_SYSTEMD_DIR}/soos-daemon.service"
    
    if [[ "${SKIP_SYSTEMD}" = false && -z "${DESTDIR}" && -d "/run/systemd/system" ]] && command -v systemctl >/dev/null 2>&1; then
        info "Reloading systemd manager configuration..."
        systemctl daemon-reload
        systemctl enable soos-daemon.service
        success "Enabled soos-daemon.service in systemd."
    fi
fi

# 6. Install Distribution PAM Config Templates
info "Installing distribution PAM configuration templates..."
PAM_PKG_DIR="${WORKSPACE_ROOT}/packaging/pam"

# Debian pam-auth-update profiles
if [[ -f "${PAM_PKG_DIR}/debian/soos" ]]; then
    DEBIAN_PAM_DIR="${DESTDIR}/usr/share/pam-configs"
    mkdir -p "${DEBIAN_PAM_DIR}"
    install -m 0644 "${PAM_PKG_DIR}/debian/soos" "${DEBIAN_PAM_DIR}/soos"
    install -m 0644 "${PAM_PKG_DIR}/debian/soos-notify" "${DEBIAN_PAM_DIR}/soos-notify"
    success "Installed Debian pam-auth-update profiles."
fi

# Fedora authselect profile
if [[ -d "${PAM_PKG_DIR}/fedora/soos" ]]; then
    FEDORA_AUTH_DIR="${DESTDIR}${SYSCONFDIR}/authselect/custom/soos"
    mkdir -p "${FEDORA_AUTH_DIR}"
    cp -r "${PAM_PKG_DIR}/fedora/soos/"* "${FEDORA_AUTH_DIR}/"
    success "Installed Fedora custom authselect profile template."
fi

# Arch snippet
if [[ -f "${PAM_PKG_DIR}/arch/system-auth.snippet" ]]; then
    ARCH_PAM_DIR="${DESTDIR}${SYSCONFDIR}/pam.d"
    mkdir -p "${ARCH_PAM_DIR}"
    install -m 0644 "${PAM_PKG_DIR}/arch/system-auth.snippet" "${ARCH_PAM_DIR}/soos.snippet"
    success "Installed Arch Linux PAM snippet."
fi

# 7. Download and Verify Neural Models
if [[ "${SKIP_MODELS}" = false && -f "${WORKSPACE_ROOT}/scripts/download_models.sh" ]]; then
    info "Deploying and verifying machine learning models..."
    bash "${WORKSPACE_ROOT}/scripts/download_models.sh" \
        --target-dir "${TARGET_STATE_DIR}/models" \
        --manifest "${WORKSPACE_ROOT}/models/manifest.toml"
fi

echo ""
success "==================================================================="
success "  soos installation and provisioning completed successfully!      "
success "==================================================================="
info "Next steps:"
info "  1. Add authorized users: soos-admin add-user <username>"
info "  2. Enroll facial vectors: sudo soos-enroll <username>"
info "  3. Start the daemon:     sudo systemctl start soos-daemon"
info "  4. Test authentication:  soos-admin test-pam"
exit 0
