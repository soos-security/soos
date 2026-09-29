#!/usr/bin/env bash
# =============================================================================
# scripts/uninstall.sh — Safe Uninstallation and Rollback for soos
# =============================================================================
# Safely rolls back PAM configuration, stops and disables systemd services,
# removes installed binaries and PAM shared libraries, and optionally preserves
# or purges biometric templates and cryptographic master keys.
#
# Supported Options:
#   -d, --destdir <DIR>      Staging destination root directory (default: /)
#   --prefix <DIR>           Installation prefix (default: /usr)
#   --sysconfdir <DIR>       Configuration directory (default: /etc)
#   --localstatedir <DIR>    State directory (default: /var)
#   --runstatedir <DIR>      Runtime directory (default: /run)
#   --pam-dir <DIR>          Explicit PAM module directory
#   --keep-data              Preserve biometric database, master key, and evidence (DEFAULT)
#   --purge-data             Permanently purge all biometric templates and cryptographic keys
#   --skip-systemd           Skip systemctl reload/disable operations
#   --dry-run                Display uninstallation plan without modifying filesystem
#   -h, --help               Display this help message
#
# Invariants:
#   - PAM stack is restored to a safe state without breaking authentication.
#   - Biometric data and master key are preserved unless --purge-data is explicitly specified.
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

DESTDIR=""
PREFIX="/usr"
SYSCONFDIR="/etc"
LOCALSTATEDIR="/var"
RUNSTATEDIR="/run"
PAM_DIR=""
KEEP_DATA=true
PURGE_DATA=false
SKIP_SYSTEMD=false
DRY_RUN=false

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Safely uninstalls soos and rolls back PAM configurations.

Options:
  -d, --destdir <DIR>      Staging destination directory (default: /)
  --prefix <DIR>           Installation prefix (default: /usr)
  --sysconfdir <DIR>       Configuration directory (default: /etc)
  --localstatedir <DIR>    State directory (default: /var)
  --runstatedir <DIR>      Runtime directory (default: /run)
  --pam-dir <DIR>          Explicit PAM module directory (auto-detected if omitted)
  --keep-data              Preserve biometric templates, evidence, and master key (DEFAULT)
  --purge-data             Permanently remove /var/lib/soos and cryptographic keys
  --skip-systemd           Skip systemctl stop, disable, and reload operations
  --dry-run                Print actions without modifying filesystem
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
        --keep-data)
            KEEP_DATA=true
            PURGE_DATA=false
            shift
            ;;
        --purge-data)
            PURGE_DATA=true
            KEEP_DATA=false
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

DESTDIR="${DESTDIR%/}"

TARGET_BIN_DIR="${DESTDIR}${PREFIX}/bin"
TARGET_LIBEXEC_DIR="${DESTDIR}${PREFIX}/libexec/soos"
TARGET_SYSTEMD_DIR="${DESTDIR}${SYSCONFDIR}/systemd/system"
TARGET_STATE_DIR="${DESTDIR}${LOCALSTATEDIR}/lib/soos"
TARGET_RUN_DIR="${DESTDIR}${RUNSTATEDIR}/soos"
TARGET_PAM_D="${DESTDIR}${SYSCONFDIR}/pam.d"

# Auto-detect PAM candidate directories
PAM_CANDIDATES=(
    "${DESTDIR}/lib/x86_64-linux-gnu/security"
    "${DESTDIR}/lib/aarch64-linux-gnu/security"
    "${DESTDIR}/usr/lib64/security"
    "${DESTDIR}/lib64/security"
    "${DESTDIR}/usr/lib/security"
    "${DESTDIR}/lib/security"
)
if [[ -n "${PAM_DIR}" ]]; then
    PAM_CANDIDATES=("${DESTDIR}${PAM_DIR}" "${PAM_CANDIDATES[@]}")
fi

info "=== soos Uninstallation Plan ==="
info "Destination root:   ${DESTDIR:-/}"
info "Purge Biometrics:   ${PURGE_DATA}"
info "Keep Biometrics:    ${KEEP_DATA}"
info "================================="

if [[ "${DRY_RUN}" = true ]]; then
    success "Dry run complete. No modifications made to system."
    exit 0
fi

# 1. Teardown Systemd Service
if [[ -f "${TARGET_SYSTEMD_DIR}/soos-daemon.service" || -z "${DESTDIR}" ]]; then
    if [[ "${SKIP_SYSTEMD}" = false && -z "${DESTDIR}" && -d "/run/systemd/system" ]] && command -v systemctl >/dev/null 2>&1; then
        info "Stopping and disabling soos-daemon.service..."
        systemctl stop soos-daemon.service 2>/dev/null || true
        systemctl disable soos-daemon.service 2>/dev/null || true
    fi
    if [[ -f "${TARGET_SYSTEMD_DIR}/soos-daemon.service" ]]; then
        rm -f "${TARGET_SYSTEMD_DIR}/soos-daemon.service"
        success "Removed ${TARGET_SYSTEMD_DIR}/soos-daemon.service"
    fi
    if [[ "${SKIP_SYSTEMD}" = false && -z "${DESTDIR}" && -d "/run/systemd/system" ]] && command -v systemctl >/dev/null 2>&1; then
        systemctl daemon-reload 2>/dev/null || true
    fi
fi

# 2. Roll Back PAM Configurations
info "Rolling back PAM configurations..."

# Rollback backups if present
for backup in "${TARGET_PAM_D}/"*.soos-backup; do
    if [[ -f "${backup}" ]]; then
        orig="${backup%.soos-backup}"
        info "Restoring PAM backup: ${backup} -> ${orig}"
        cp -f "${backup}" "${orig}"
        rm -f "${backup}"
        success "Restored ${orig}"
    fi
done

# Clean up Debian pam-auth-update profiles
DEBIAN_PAM_DIR="${DESTDIR}/usr/share/pam-configs"
if [[ -f "${DEBIAN_PAM_DIR}/soos" || -f "${DEBIAN_PAM_DIR}/soos-notify" ]]; then
    if [[ -z "${DESTDIR}" ]] && command -v pam-auth-update >/dev/null 2>&1; then
        info "Deregistering soos from pam-auth-update..."
        pam-auth-update --package --remove soos soos-notify 2>/dev/null || true
    fi
    rm -f "${DEBIAN_PAM_DIR}/soos" "${DEBIAN_PAM_DIR}/soos-notify"
    success "Removed Debian pam-auth-update profiles."
fi

# Clean up Fedora authselect custom profile. When custom/soos is the selected
# profile, the previously recorded profile is restored first: deleting the
# selected profile would leave authselect without a valid configuration
# (GitHub #145). If no restoration is possible, the profile is kept in place.
FEDORA_AUTH_DIR="${DESTDIR}${SYSCONFDIR}/authselect/custom/soos"
AUTHSELECT_PREVIOUS_FILE="${DESTDIR}${SYSCONFDIR}/soos/authselect.previous"
AUTHSELECT_RESTORED=true
if [[ -z "${DESTDIR}" ]] && command -v authselect >/dev/null 2>&1; then
    AUTHSELECT_CURRENT="$(authselect current --raw 2>/dev/null || true)"
    if [[ "${AUTHSELECT_CURRENT}" == custom/soos* ]]; then
        AUTHSELECT_PREVIOUS=""
        if [[ -f "${AUTHSELECT_PREVIOUS_FILE}" ]]; then
            # Profile id and feature names only (defensive filtering of the recorded line).
            AUTHSELECT_PREVIOUS="$(head -n 1 "${AUTHSELECT_PREVIOUS_FILE}" | tr -cd 'A-Za-z0-9/_. -')"
        fi
        if [[ -z "${AUTHSELECT_PREVIOUS}" || "${AUTHSELECT_PREVIOUS}" == custom/soos* ]]; then
            AUTHSELECT_PREVIOUS=""
            for fallback in local minimal sssd; do
                if authselect list 2>/dev/null | grep -q "^- ${fallback}[[:space:]]"; then
                    AUTHSELECT_PREVIOUS="${fallback}"
                    break
                fi
            done
        fi
        if [[ -z "${AUTHSELECT_PREVIOUS}" ]]; then
            error "No authselect profile available to replace custom/soos."
            AUTHSELECT_RESTORED=false
        else
            info "Restoring authselect profile: ${AUTHSELECT_PREVIOUS}"
            # shellcheck disable=SC2086 # word splitting intended: profile id followed by features
            if authselect select ${AUTHSELECT_PREVIOUS} --force >/dev/null; then
                success "Restored authselect profile '${AUTHSELECT_PREVIOUS}'."
            else
                error "Failed to restore authselect profile '${AUTHSELECT_PREVIOUS}'."
                AUTHSELECT_RESTORED=false
            fi
        fi
    fi
fi
if [[ "${AUTHSELECT_RESTORED}" = true ]]; then
    rm -f "${AUTHSELECT_PREVIOUS_FILE}"
    if [[ -d "${FEDORA_AUTH_DIR}" ]]; then
        rm -rf "${FEDORA_AUTH_DIR}"
        success "Removed Fedora authselect custom profile."
    fi
else
    warn "Keeping ${FEDORA_AUTH_DIR}: it is still the selected authselect profile."
    warn "Run 'authselect select <profile> [features] --force' and remove it manually."
fi

# Clean up Arch snippet
if [[ -f "${TARGET_PAM_D}/soos.snippet" ]]; then
    rm -f "${TARGET_PAM_D}/soos.snippet"
    success "Removed Arch PAM snippet."
fi

# 3. Remove Binaries and Shared Libraries
info "Removing binaries and shared libraries..."
rm -f "${TARGET_LIBEXEC_DIR}/soos-daemon"
rm -f "${TARGET_LIBEXEC_DIR}/provision-master-key"
rmdir "${TARGET_LIBEXEC_DIR}" 2>/dev/null || true
rm -f "${TARGET_BIN_DIR}/soos-admin"
rm -f "${TARGET_BIN_DIR}/soos-enroll"
rm -f "${TARGET_BIN_DIR}/soos-gui"

for cand in "${PAM_CANDIDATES[@]}"; do
    if [[ -f "${cand}/pam_soos.so" ]]; then
        rm -f "${cand}/pam_soos.so"
        success "Removed ${cand}/pam_soos.so"
    fi
done

# 4. Clean Runtime Directory
if [[ -d "${TARGET_RUN_DIR}" ]]; then
    rm -rf "${TARGET_RUN_DIR}"
    success "Removed runtime directory ${TARGET_RUN_DIR}"
fi

# 5. Handle State and Biometric Data
if [[ "${PURGE_DATA}" = true ]]; then
    if [[ -d "${TARGET_STATE_DIR}" ]]; then
        warn "Purging all biometric templates, evidence, and master keys at ${TARGET_STATE_DIR}..."
        rm -rf "${TARGET_STATE_DIR}"
        success "Purged ${TARGET_STATE_DIR}."
    fi
else
    info "Preserving biometric templates, evidence, and master key at ${TARGET_STATE_DIR}."
fi

echo ""
success "==================================================================="
success "  soos has been cleanly uninstalled and PAM stack restored.       "
success "==================================================================="
exit 0
