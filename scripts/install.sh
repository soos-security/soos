#!/usr/bin/env bash
# =============================================================================
# scripts/install.sh — System Provisioning and Installation for soos
# =============================================================================
# Provisions system groups, directory hierarchies with strict permissions,
# installs release binaries and the PAM shared library, installs the systemd
# unit, generates the master encryption key on a live install if absent, and
# verifies neural models.
#
# Fail-closed and transactional (GitHub #164 / ONB-06):
#   - A read-only preflight runs BEFORE any change: root check for a live
#     install, every artifact present in the artifact directory (release
#     profile only, never target/debug implicitly), model manifest and tools.
#   - Every file, directory and backup written is journaled; if any later step
#     fails, the journal is replayed backwards (created files and directories
#     removed, overwritten files restored) and the script exits non-zero.
#   - Models are deployed and verified BEFORE the systemd unit is enabled.
#
# Staging mode (--destdir): the tree is package content, so NO key material is
# ever generated inside it (GitHub #144). The key is produced on the target host
# at first install by /usr/libexec/soos/provision-master-key (called from the
# package post-install scriptlets), which this script ships in libexec.
#
# Supported Options:
#   -d, --destdir <DIR>      Target staging root directory (default: /)
#   --prefix <DIR>           Installation prefix (default: /usr)
#   --sysconfdir <DIR>       Configuration directory (default: /etc)
#   --localstatedir <DIR>    State directory (default: /var)
#   --runstatedir <DIR>      Runtime directory (default: /run)
#   --pam-dir <DIR>          Explicit PAM module directory (overrides auto-detection)
#   --artifact-dir <DIR>     Directory holding the built artifacts
#                            (default: ${CARGO_TARGET_DIR:-target}/release)
#   --build                  Run the dependency preflight and
#                            'cargo build --release --locked --workspace' first
#   --allow-missing          Install a partial artifact set (developer use only)
#   --allow-debug-artifacts  Accept artifacts from a cargo 'debug' profile directory
#   --manifest <PATH>        Model manifest (default: models/manifest.toml)
#   --skip-models            Skip machine learning model downloading/verification
#   --skip-systemd           Skip systemctl reload/enable operations
#   --dry-run                Run the read-only preflight and print the plan
#   -h, --help               Display this help message
#
# Security Invariants:
#   - /var/lib/soos/{biometrics,evidence} created mode 0700 (root:root)
#   - /var/lib/soos/master.key created mode 0600 (root:root, 32 bytes) on the
#     target host only (never under --destdir)
#   - /run/soos created mode 0750 (root:soos)
#   - Binaries installed mode 0755
#   - PAM module installed mode 0644
#   - Pre-existing system directories (bin, PAM module dir, ...) are never re-moded
# =============================================================================

set -euo pipefail
umask 022

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
ARTIFACT_DIR=""
MANIFEST_PATH="${WORKSPACE_ROOT}/models/manifest.toml"
DO_BUILD=false
ALLOW_MISSING=false
ALLOW_DEBUG=false
SKIP_MODELS=false
SKIP_SYSTEMD=false
DRY_RUN=false

# Artifacts produced by 'cargo build --release --workspace'.
REQUIRED_ARTIFACTS=(soos-daemon soos-admin soos-enroll soos-gui libpam_soos.so)

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Installs and provisions soos Local Facial Biometric PAM module and daemon.
Fails closed: every artifact must exist, and any failure rolls back all changes.

Options:
  -d, --destdir <DIR>      Staging destination directory (default: /)
  --prefix <DIR>           Installation prefix (default: /usr)
  --sysconfdir <DIR>       Configuration directory (default: /etc)
  --localstatedir <DIR>    State directory (default: /var)
  --runstatedir <DIR>      Runtime directory (default: /run)
  --pam-dir <DIR>          Explicit PAM module directory (auto-detected if omitted)
  --artifact-dir <DIR>     Built artifacts directory (default: target/release)
  --build                  Check build dependencies, then run
                           'cargo build --release --locked --workspace'
  --allow-missing          Install a partial artifact set (developer use only)
  --allow-debug-artifacts  Accept artifacts from a cargo 'debug' profile directory
  --manifest <PATH>        Model manifest (default: models/manifest.toml)
  --skip-models            Skip model download and verification
  --skip-systemd           Skip systemctl reload and enable invocations
  --dry-run                Run the read-only preflight and print the plan
  -h, --help               Display this help message and exit

A live install (no --destdir) must run as root.
EOF
}

require_value() {
    if [[ $# -lt 2 || -z "$2" ]]; then
        error "Option $1 requires a value"
        usage >&2
        exit 1
    fi
}

# Parse command line options
while [[ $# -gt 0 ]]; do
    case "$1" in
        -d|--destdir)
            require_value "$@"
            DESTDIR="$2"
            shift 2
            ;;
        --prefix)
            require_value "$@"
            PREFIX="$2"
            shift 2
            ;;
        --sysconfdir)
            require_value "$@"
            SYSCONFDIR="$2"
            shift 2
            ;;
        --localstatedir)
            require_value "$@"
            LOCALSTATEDIR="$2"
            shift 2
            ;;
        --runstatedir)
            require_value "$@"
            RUNSTATEDIR="$2"
            shift 2
            ;;
        --pam-dir)
            require_value "$@"
            PAM_DIR="$2"
            shift 2
            ;;
        --artifact-dir)
            require_value "$@"
            ARTIFACT_DIR="$2"
            shift 2
            ;;
        --manifest)
            require_value "$@"
            MANIFEST_PATH="$2"
            shift 2
            ;;
        --build)
            DO_BUILD=true
            shift
            ;;
        --allow-missing)
            ALLOW_MISSING=true
            shift
            ;;
        --allow-debug-artifacts)
            ALLOW_DEBUG=true
            shift
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

DEFAULT_ARTIFACT_DIR="${CARGO_TARGET_DIR:-${WORKSPACE_ROOT}/target}/release"
if [[ "${DO_BUILD}" = true && -n "${ARTIFACT_DIR}" ]]; then
    error "--build always produces ${DEFAULT_ARTIFACT_DIR}; do not combine it with --artifact-dir"
    exit 1
fi
ARTIFACT_DIR="${ARTIFACT_DIR:-${DEFAULT_ARTIFACT_DIR}}"

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

LIVE_INSTALL=false
[[ -z "${DESTDIR}" ]] && LIVE_INSTALL=true

info "=== soos Installation Plan ==="
info "Destination root:   ${DESTDIR:-/}"
info "Artifacts:          ${ARTIFACT_DIR}"
info "Binaries:           ${TARGET_BIN_DIR}"
info "Daemon libexec:     ${TARGET_LIBEXEC_DIR}"
info "PAM Module:         ${TARGET_PAM_DIR}/pam_soos.so"
info "Systemd Unit:       ${TARGET_SYSTEMD_DIR}/soos-daemon.service"
info "State Directory:    ${TARGET_STATE_DIR}"
info "Runtime Directory:  ${TARGET_RUN_DIR}"
info "==============================="

# =============================================================================
# Phase A — Read-only preflight (no filesystem mutation before it passes)
# =============================================================================
PREFLIGHT_ERRORS=0
preflight_fail() {
    error "$*"
    PREFLIGHT_ERRORS=$((PREFLIGHT_ERRORS + 1))
}

# A.1 Root is required for a live install.
if [[ "${LIVE_INSTALL}" = true && "${DRY_RUN}" = false && "$(id -u)" -ne 0 ]]; then
    preflight_fail "A live installation (no --destdir) must run as root: re-run with sudo, or stage with --destdir <DIR>."
fi

# A.2 Required tools.
for tool in install mv cp rm rmdir mkdir chmod; do
    command -v "${tool}" >/dev/null 2>&1 || preflight_fail "Required tool not found: ${tool}"
done

# A.3 Optional release build (explicit, locked, never a debug build).
BUILD_CMD=(cargo build --release --locked --workspace)
run_release_build() {
    info "Checking build dependencies (scripts/check_build_deps.sh)..."
    info "Building release artifacts: cargo build --release --locked --workspace"
    if [[ "$(id -u)" -eq 0 && -n "${SUDO_USER:-}" && "${SUDO_USER}" != "root" ]]; then
        # Never compile as root inside a user's checkout: build as the invoking user.
        # shellcheck disable=SC2016 # "$1" is expanded by the inner login shell.
        runuser -u "${SUDO_USER}" -- bash -lc \
            'cd "$1" && ./scripts/check_build_deps.sh && cargo build --release --locked --workspace' \
            _ "${WORKSPACE_ROOT}"
    else
        (cd "${WORKSPACE_ROOT}" && ./scripts/check_build_deps.sh && "${BUILD_CMD[@]}")
    fi
}

if [[ "${DO_BUILD}" = true ]]; then
    if [[ "${DRY_RUN}" = true ]]; then
        info "[DRY-RUN] Would run: ${BUILD_CMD[*]} (after scripts/check_build_deps.sh)"
        "${WORKSPACE_ROOT}/scripts/check_build_deps.sh" || preflight_fail "Build dependencies are missing (see above)."
    elif [[ "${PREFLIGHT_ERRORS}" -eq 0 ]]; then
        if ! run_release_build; then
            error "Release build failed; nothing was installed."
            exit 40
        fi
    fi
fi

# A.4 Artifacts: release profile only, every artifact present unless --allow-missing.
declare -A ARTIFACT_PATHS=()
MISSING_ARTIFACTS=()
if [[ "${DO_BUILD}" = true && "${DRY_RUN}" = true ]]; then
    info "[DRY-RUN] Artifacts will be produced by the build; artifact check skipped."
elif [[ ! -d "${ARTIFACT_DIR}" ]]; then
    MISSING_ARTIFACTS=("${REQUIRED_ARTIFACTS[@]}")
    preflight_fail "Artifact directory not found: ${ARTIFACT_DIR} (build with 'cargo build --release --locked --workspace' or pass --build)"
else
    RESOLVED_ARTIFACT_DIR="$(cd "${ARTIFACT_DIR}" && pwd -P)"
    if [[ "$(basename "${RESOLVED_ARTIFACT_DIR}")" == "debug" && "${ALLOW_DEBUG}" = false ]]; then
        preflight_fail "Refusing artifacts from a cargo debug profile directory (${RESOLVED_ARTIFACT_DIR}): install release builds, or pass --allow-debug-artifacts explicitly."
    fi
    for name in "${REQUIRED_ARTIFACTS[@]}"; do
        candidate="${RESOLVED_ARTIFACT_DIR}/${name}"
        if [[ -f "${candidate}" && -s "${candidate}" ]]; then
            ARTIFACT_PATHS[${name}]="${candidate}"
        else
            MISSING_ARTIFACTS+=("${name}")
        fi
    done
fi
if [[ ${#MISSING_ARTIFACTS[@]} -gt 0 ]]; then
    if [[ "${ALLOW_MISSING}" = true ]]; then
        warn "--allow-missing: the following artifacts will NOT be installed: ${MISSING_ARTIFACTS[*]}"
    else
        preflight_fail "Missing build artifacts in ${ARTIFACT_DIR}: ${MISSING_ARTIFACTS[*]}"
    fi
fi

# A.5 Model manifest and model tools (validated read-only by download_models.sh).
if [[ "${SKIP_MODELS}" = false ]]; then
    if ! bash "${WORKSPACE_ROOT}/scripts/download_models.sh" --preflight \
            --manifest "${MANIFEST_PATH}" --target-dir "${TARGET_STATE_DIR}/models"; then
        preflight_fail "Model preflight failed (manifest or required tools); fix it or pass --skip-models."
    fi
fi

if [[ "${PREFLIGHT_ERRORS}" -gt 0 ]]; then
    error "Preflight failed with ${PREFLIGHT_ERRORS} error(s); nothing was modified."
    exit 2
fi
success "Preflight passed."

if [[ "${DRY_RUN}" = true ]]; then
    success "Dry run complete. No modifications made to system."
    exit 0
fi

# =============================================================================
# Phase B — Transaction journal and rollback
# =============================================================================
JOURNAL_FILES=()          # files created by this run (removed on rollback)
JOURNAL_DIRS=()           # directories created by this run (rmdir on rollback)
JOURNAL_BACKUP_ORIG=()    # files overwritten by this run ...
JOURNAL_BACKUP_COPY=()    # ... and their saved previous version
GROUP_CREATED=false
UNIT_INSTALLED=false
UNIT_ENABLED=false
COMMITTED=false
KEEP_BACKUP=false
BACKUP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/soos-install-backup.XXXXXX")"

rollback() {
    set +e
    local i
    warn "Rolling back all changes made by this installation run..."
    if [[ "${UNIT_ENABLED}" = true ]]; then
        systemctl disable soos-daemon.service >/dev/null 2>&1
    fi
    for ((i = ${#JOURNAL_BACKUP_ORIG[@]} - 1; i >= 0; i--)); do
        if ! { cp -a -- "${JOURNAL_BACKUP_COPY[i]}" "${JOURNAL_BACKUP_ORIG[i]}.soos-restore.$$" \
                && mv -f -- "${JOURNAL_BACKUP_ORIG[i]}.soos-restore.$$" "${JOURNAL_BACKUP_ORIG[i]}"; }; then
            error "Could not restore ${JOURNAL_BACKUP_ORIG[i]} (saved copy kept in ${BACKUP_DIR})"
            KEEP_BACKUP=true
        fi
    done
    for ((i = ${#JOURNAL_FILES[@]} - 1; i >= 0; i--)); do
        rm -f -- "${JOURNAL_FILES[i]}"
    done
    for ((i = ${#JOURNAL_DIRS[@]} - 1; i >= 0; i--)); do
        rmdir -- "${JOURNAL_DIRS[i]}" 2>/dev/null
    done
    if [[ "${UNIT_INSTALLED}" = true && "${LIVE_INSTALL}" = true && -d /run/systemd/system ]] \
            && command -v systemctl >/dev/null 2>&1; then
        systemctl daemon-reload >/dev/null 2>&1
    fi
    if [[ "${GROUP_CREATED}" = true ]] && command -v groupdel >/dev/null 2>&1; then
        groupdel soos >/dev/null 2>&1
    fi
}

on_exit() {
    local rc=$?
    trap - EXIT
    if [[ "${COMMITTED}" != true ]]; then
        [[ "${rc}" -eq 0 ]] && rc=1
        rollback
        # Keep the saved copies if a restore failed, otherwise discard them.
        if [[ "${KEEP_BACKUP}" = true ]]; then
            error "Installation FAILED (exit ${rc}); rollback was incomplete, see the errors above."
        else
            error "Installation FAILED (exit ${rc}); all changes were rolled back."
            rm -rf -- "${BACKUP_DIR}"
        fi
    else
        rm -rf -- "${BACKUP_DIR}"
    fi
    exit "${rc}"
}
trap on_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Creates a directory (and any missing parent), journaling every created level.
# MODE is applied to the leaf when it is created, or always when ENFORCE=owned
# (soos-owned directories). Pre-existing system directories are never re-moded.
ensure_dir() {
    local dir="$1" mode="$2" enforce="${3:-system}"
    local missing=() probe="${dir}" i created_leaf=false
    while [[ -n "${probe}" && "${probe}" != "/" && ! -e "${probe}" ]]; do
        missing=("${probe}" "${missing[@]}")
        probe="$(dirname "${probe}")"
    done
    for ((i = 0; i < ${#missing[@]}; i++)); do
        mkdir -- "${missing[i]}"
        JOURNAL_DIRS+=("${missing[i]}")
        created_leaf=true
    done
    if [[ ! -d "${dir}" ]]; then
        error "Not a directory: ${dir}"
        return 1
    fi
    if [[ "${created_leaf}" = true || "${enforce}" == "owned" ]]; then
        chmod "${mode}" "${dir}"
    fi
}

# Installs SRC to DST with MODE atomically (temp file + rename in the same
# directory), saving any previous DST so that rollback can restore it.
tracked_install() {
    local mode="$1" src="$2" dst="$3" tmp backup
    if [[ -d "${dst}" && ! -L "${dst}" ]]; then
        error "Refusing to replace directory ${dst} with a file"
        return 1
    fi
    if [[ -e "${dst}" || -L "${dst}" ]]; then
        backup="${BACKUP_DIR}/${#JOURNAL_BACKUP_ORIG[@]}"
        cp -a -- "${dst}" "${backup}"
        JOURNAL_BACKUP_ORIG+=("${dst}")
        JOURNAL_BACKUP_COPY+=("${backup}")
    else
        JOURNAL_FILES+=("${dst}")
    fi
    tmp="$(dirname "${dst}")/.soos-install.$$.$(basename "${dst}")"
    if ! install -m "${mode}" -- "${src}" "${tmp}"; then
        rm -f -- "${tmp}"
        return 1
    fi
    mv -f -- "${tmp}" "${dst}"
}

# Records the files a sub-process added to DIR (compared to a previous listing).
journal_new_files() {
    local dir="$1" before="$2" entry
    [[ -d "${dir}" ]] || return 0
    while IFS= read -r entry; do
        [[ -z "${entry}" ]] && continue
        if ! grep -Fxq -- "${entry}" <<< "${before}"; then
            JOURNAL_FILES+=("${entry}")
        fi
    done < <(find "${dir}" -mindepth 1 -maxdepth 1 -type f)
}

# =============================================================================
# Phase C — Installation steps (journaled)
# =============================================================================

# 1. Create System Group 'soos'
if [[ "${LIVE_INSTALL}" = true ]]; then
    if getent group soos >/dev/null 2>&1; then
        info "System group 'soos' already exists."
    else
        info "Creating system group 'soos'..."
        if command -v groupadd >/dev/null 2>&1; then
            groupadd -r soos
            GROUP_CREATED=true
            success "Created system group 'soos'."
        elif command -v addgroup >/dev/null 2>&1; then
            addgroup --system soos
            GROUP_CREATED=true
            success "Created system group 'soos'."
        else
            warn "Neither groupadd nor addgroup found. Please ensure 'soos' group is created."
        fi
    fi
fi

# 2. Provision Directory Hierarchy with Invariant Permissions
info "Provisioning directories..."
ensure_dir "${TARGET_STATE_DIR}" 0755 owned
ensure_dir "${TARGET_STATE_DIR}/biometrics" 0700 owned
ensure_dir "${TARGET_STATE_DIR}/evidence" 0700 owned
ensure_dir "${TARGET_STATE_DIR}/models" 0755 owned
ensure_dir "${TARGET_RUN_DIR}" 0750 owned
ensure_dir "${TARGET_LIBEXEC_DIR}" 0755 owned
ensure_dir "${TARGET_BIN_DIR}" 0755
ensure_dir "${TARGET_PAM_DIR}" 0755
ensure_dir "${TARGET_SYSTEMD_DIR}" 0755

if [[ "$(id -u)" -eq 0 && "${LIVE_INSTALL}" = true ]]; then
    chown root:root "${TARGET_STATE_DIR}"
    chown root:root "${TARGET_STATE_DIR}/biometrics"
    chown root:root "${TARGET_STATE_DIR}/evidence"
    chown root:root "${TARGET_STATE_DIR}/models"
    chown root:soos "${TARGET_RUN_DIR}" || chown root:root "${TARGET_RUN_DIR}"
fi
success "Directories provisioned with verified permissions."

# 3. Provision Master Key (live install only — never inside a staging tree)
KEY_HELPER_SRC="${SCRIPT_DIR}/provision_master_key.sh"
if [[ "${LIVE_INSTALL}" = true ]]; then
    info "Provisioning cryptographic master key if absent..."
    KEY_EXISTED=false
    [[ -e "${TARGET_STATE_DIR}/master.key" ]] && KEY_EXISTED=true
    # A key created by a failed run protects nothing yet: rollback removes it.
    [[ "${KEY_EXISTED}" = false ]] && JOURNAL_FILES+=("${TARGET_STATE_DIR}/master.key")
    sh "${KEY_HELPER_SRC}" --state-dir "${TARGET_STATE_DIR}"
    success "Master key provisioned at ${TARGET_STATE_DIR}/master.key (mode 0600)."
else
    info "Staging mode (--destdir): master key is NOT generated; it will be created on the target host at first install."
fi

# 4. Install Binaries and Shared Library
info "Installing executable binaries..."
# Key provisioning helper (shared by install.sh and all package scriptlets)
tracked_install 0755 "${KEY_HELPER_SRC}" "${TARGET_LIBEXEC_DIR}/provision-master-key"
success "Installed ${TARGET_LIBEXEC_DIR}/provision-master-key"

install_artifact() {
    local name="$1" mode="$2" dst="$3"
    if [[ -n "${ARTIFACT_PATHS[${name}]:-}" ]]; then
        tracked_install "${mode}" "${ARTIFACT_PATHS[${name}]}" "${dst}"
        success "Installed ${dst}"
    else
        warn "${name} not installed (--allow-missing)."
    fi
}
install_artifact soos-daemon 0755 "${TARGET_LIBEXEC_DIR}/soos-daemon"
install_artifact soos-admin 0755 "${TARGET_BIN_DIR}/soos-admin"
install_artifact soos-enroll 0755 "${TARGET_BIN_DIR}/soos-enroll"
install_artifact soos-gui 0755 "${TARGET_BIN_DIR}/soos-gui"
install_artifact libpam_soos.so 0644 "${TARGET_PAM_DIR}/pam_soos.so"

# 5. Install Systemd Service Unit (enabled only at the very end, step 8)
SERVICE_SRC="${WORKSPACE_ROOT}/packaging/soos-daemon.service"
if [[ -f "${SERVICE_SRC}" ]]; then
    tracked_install 0644 "${SERVICE_SRC}" "${TARGET_SYSTEMD_DIR}/soos-daemon.service"
    UNIT_INSTALLED=true
    success "Installed ${TARGET_SYSTEMD_DIR}/soos-daemon.service"
fi

# 6. Install Distribution PAM Config Templates
info "Installing distribution PAM configuration templates..."
PAM_PKG_DIR="${WORKSPACE_ROOT}/packaging/pam"

# Debian pam-auth-update profiles
if [[ -f "${PAM_PKG_DIR}/debian/soos" ]]; then
    DEBIAN_PAM_DIR="${DESTDIR}/usr/share/pam-configs"
    ensure_dir "${DEBIAN_PAM_DIR}" 0755
    tracked_install 0644 "${PAM_PKG_DIR}/debian/soos" "${DEBIAN_PAM_DIR}/soos"
    tracked_install 0644 "${PAM_PKG_DIR}/debian/soos-notify" "${DEBIAN_PAM_DIR}/soos-notify"
    success "Installed Debian pam-auth-update profiles."
fi

# Fedora authselect profile
if [[ -d "${PAM_PKG_DIR}/fedora/soos" ]]; then
    FEDORA_AUTH_DIR="${DESTDIR}${SYSCONFDIR}/authselect/custom/soos"
    ensure_dir "${FEDORA_AUTH_DIR}" 0755
    for profile_file in "${PAM_PKG_DIR}/fedora/soos/"*; do
        [[ -f "${profile_file}" ]] || continue
        tracked_install 0644 "${profile_file}" "${FEDORA_AUTH_DIR}/$(basename "${profile_file}")"
    done
    success "Installed Fedora custom authselect profile template."

    # Record the currently selected authselect profile (id + features) so that
    # scripts/uninstall.sh can restore it once custom/soos has been activated.
    # The profile is never activated automatically (GitHub #145).
    if [[ "${LIVE_INSTALL}" = true ]] && command -v authselect >/dev/null 2>&1; then
        AUTHSELECT_CURRENT="$(authselect current --raw 2>/dev/null || true)"
        AUTHSELECT_PREVIOUS_FILE="${SYSCONFDIR}/soos/authselect.previous"
        if [[ -n "${AUTHSELECT_CURRENT}" && "${AUTHSELECT_CURRENT}" != custom/soos* ]]; then
            ensure_dir "${SYSCONFDIR}/soos" 0755
            AUTHSELECT_RECORD="${BACKUP_DIR}/authselect.previous.new"
            printf '%s\n' "${AUTHSELECT_CURRENT}" > "${AUTHSELECT_RECORD}"
            tracked_install 0644 "${AUTHSELECT_RECORD}" "${AUTHSELECT_PREVIOUS_FILE}"
            success "Recorded current authselect profile for rollback: ${AUTHSELECT_CURRENT}"
        fi
        info "Activate with: authselect select custom/soos with-faillock --force && authselect check"
    fi
fi

# Arch snippet
if [[ -f "${PAM_PKG_DIR}/arch/system-auth.snippet" ]]; then
    ARCH_PAM_DIR="${DESTDIR}${SYSCONFDIR}/pam.d"
    ensure_dir "${ARCH_PAM_DIR}" 0755
    tracked_install 0644 "${PAM_PKG_DIR}/arch/system-auth.snippet" "${ARCH_PAM_DIR}/soos.snippet"
    success "Installed Arch Linux PAM snippet."
fi

# 7. Download and Verify Neural Models (BEFORE the unit is enabled)
if [[ "${SKIP_MODELS}" = false ]]; then
    info "Deploying and verifying machine learning models..."
    MODELS_DIR="${TARGET_STATE_DIR}/models"
    MODELS_BEFORE="$(find "${MODELS_DIR}" -mindepth 1 -maxdepth 1 -type f 2>/dev/null || true)"
    MODELS_RC=0
    bash "${WORKSPACE_ROOT}/scripts/download_models.sh" \
        --target-dir "${MODELS_DIR}" \
        --manifest "${MANIFEST_PATH}" || MODELS_RC=$?
    journal_new_files "${MODELS_DIR}" "${MODELS_BEFORE}"
    if [[ "${MODELS_RC}" -ne 0 ]]; then
        error "Model deployment or SHA-256 verification failed (exit ${MODELS_RC}); the unit is NOT enabled."
        exit 60
    fi
fi

# 8. Enable the Systemd Unit (live install only, last mutating step)
if [[ "${UNIT_INSTALLED}" = true && "${SKIP_SYSTEMD}" = false && "${LIVE_INSTALL}" = true && -d "/run/systemd/system" ]] \
        && command -v systemctl >/dev/null 2>&1; then
    info "Reloading systemd manager configuration..."
    systemctl daemon-reload
    UNIT_ENABLED=true
    systemctl enable soos-daemon.service
    success "Enabled soos-daemon.service in systemd."
fi

COMMITTED=true

echo ""
if [[ ${#MISSING_ARTIFACTS[@]} -gt 0 ]]; then
    warn "==================================================================="
    warn "  soos installation INCOMPLETE (--allow-missing): not installed:"
    warn "    ${MISSING_ARTIFACTS[*]}"
    warn "==================================================================="
    exit 0
fi
success "==================================================================="
success "  soos installation and provisioning completed successfully!      "
success "==================================================================="
info "Next steps:"
info "  1. Add authorized users: soos-admin add-user <username>"
info "  2. Enroll facial vectors: sudo soos-enroll <username>"
info "  3. Start the daemon:     sudo systemctl start soos-daemon"
info "  4. Test authentication:  soos-admin test-pam"
exit 0
