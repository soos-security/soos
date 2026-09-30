#!/usr/bin/env bash
# =============================================================================
# scripts/pam_snapshot.sh — Pre-install PAM State Snapshot for soos Rollback
# =============================================================================
# Records the PAM state of the host BEFORE soos touches it, so that
# scripts/uninstall.sh can restore residual edits byte-for-byte and verify that
# the rollback returned every file to its original content (GitHub #166).
#
# Snapshot layout (root-only, mode 0700):
#   <localstatedir>/lib/soos/state/pam-backup/
#     pam.d/<name>        dereferenced copy of every /etc/pam.d entry
#     nsswitch.conf       copy of /etc/nsswitch.conf (authselect rewrites it)
#     SHA256SUMS          "<sha256>  pam.d/<name>" / "<sha256>  nsswitch.conf"
#     authselect.current  `authselect current --raw` (live hosts with authselect)
#
# Commands:
#   snapshot   Create the snapshot; an existing snapshot is NEVER overwritten
#              (it holds the pristine pre-soos state)
#   verify     Compare the current state with the snapshot:
#              exit 0 identical, 1 differences (listed), 2 no snapshot
#   discard    Remove the snapshot
#
# Options:
#   -d, --destdir <DIR>      Staging root (default: /; authselect is not queried)
#   --sysconfdir <DIR>       Configuration directory (default: /etc)
#   --localstatedir <DIR>    State directory (default: /var)
#   -h, --help               Display this help message
#
# The snapshot contains PAM/NSS configuration only: no credential, key or
# biometric material is ever copied.
# =============================================================================

set -euo pipefail

usage() {
    cat <<EOF
Usage: $(basename "$0") <snapshot|verify|discard> [OPTIONS]

Records, verifies or discards the pre-install PAM state used by the soos rollback.

Options:
  -d, --destdir <DIR>      Staging root directory (default: /)
  --sysconfdir <DIR>       Configuration directory (default: /etc)
  --localstatedir <DIR>    State directory (default: /var)
  -h, --help               Display this help message and exit
EOF
}

COMMAND=""
DESTDIR=""
SYSCONFDIR="/etc"
LOCALSTATEDIR="/var"

while [[ $# -gt 0 ]]; do
    case "$1" in
        snapshot|verify|discard)
            COMMAND="$1"
            shift
            ;;
        -d|--destdir)
            DESTDIR="${2:-}"
            shift 2
            ;;
        --sysconfdir)
            SYSCONFDIR="${2:-}"
            shift 2
            ;;
        --localstatedir)
            LOCALSTATEDIR="${2:-}"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "[ERROR] Unknown argument: $1" >&2
            usage >&2
            exit 64
            ;;
    esac
done

if [[ -z "${COMMAND}" ]]; then
    usage >&2
    exit 64
fi

DESTDIR="${DESTDIR%/}"
ETC_DIR="${DESTDIR}${SYSCONFDIR}"
PAM_D="${ETC_DIR}/pam.d"
STATE_DIR="${DESTDIR}${LOCALSTATEDIR}/lib/soos/state"
SNAPSHOT_DIR="${STATE_DIR}/pam-backup"

# Refuses to operate through symlinked state directories (a planted link could
# redirect root writes or deletions).
refuse_symlinks() {
    local path
    for path in "${DESTDIR}${LOCALSTATEDIR}/lib/soos" "${STATE_DIR}" "${SNAPSHOT_DIR}"; do
        if [[ -L "${path}" ]]; then
            echo "[ERROR] Refusing to use symlinked state path: ${path}" >&2
            exit 1
        fi
    done
}

# Relative paths (below the configuration directory) of every recorded entry.
list_entries() {
    local f name
    if [[ -d "${PAM_D}" ]]; then
        for f in "${PAM_D}"/*; do
            name="$(basename "${f}")"
            case "${name}" in
                *.soos-backup|.soos-*) continue ;;
            esac
            # Regular files and symlinks resolving to regular files only.
            [[ -f "${f}" ]] || continue
            echo "pam.d/${name}"
        done
    fi
    if [[ -f "${ETC_DIR}/nsswitch.conf" ]]; then
        echo "nsswitch.conf"
    fi
}

# Manifest of the CURRENT state, same format as SHA256SUMS. Fails (non-zero)
# if any entry cannot be hashed, so that a verification can never pass by error.
current_manifest() {
    local rel hash
    while IFS= read -r rel; do
        hash="$(sha256sum < "${ETC_DIR}/${rel}")" || return 1
        hash="${hash%% *}"
        [[ "${hash}" =~ ^[0-9a-f]{64}$ ]] || return 1
        printf '%s  %s\n' "${hash}" "${rel}"
    done < <(list_entries)
}

authselect_live() {
    [[ -z "${DESTDIR}" ]] && command -v authselect >/dev/null 2>&1
}

cmd_snapshot() {
    refuse_symlinks
    if [[ -f "${SNAPSHOT_DIR}/SHA256SUMS" ]]; then
        echo "[INFO]  Keeping existing pre-install PAM snapshot: ${SNAPSHOT_DIR}"
        return 0
    fi
    umask 077
    mkdir -p "${STATE_DIR}"
    chmod 0700 "${STATE_DIR}"
    local tmp rel
    tmp="$(mktemp -d "${STATE_DIR}/.pam-backup.XXXXXX")"
    mkdir -p "${tmp}/pam.d"
    while IFS= read -r rel; do
        # -L: store the content a symlinked stack (authselect) resolves to.
        cp -L --preserve=mode,timestamps "${ETC_DIR}/${rel}" "${tmp}/${rel}"
        chmod go-w "${tmp}/${rel}"
    done < <(list_entries)
    if ! current_manifest > "${tmp}/SHA256SUMS"; then
        rm -rf "${tmp}"
        echo "[ERROR] Unable to hash the PAM state; no snapshot recorded." >&2
        return 1
    fi
    if authselect_live; then
        authselect current --raw > "${tmp}/authselect.current" 2>/dev/null || true
    fi
    chmod 0700 "${tmp}"
    rm -rf "${SNAPSHOT_DIR}"
    mv "${tmp}" "${SNAPSHOT_DIR}"
    echo "[OK]    Recorded pre-install PAM state in ${SNAPSHOT_DIR}"
}

cmd_verify() {
    refuse_symlinks
    if [[ ! -f "${SNAPSHOT_DIR}/SHA256SUMS" ]]; then
        echo "[INFO]  No pre-install PAM snapshot at ${SNAPSHOT_DIR}"
        return 2
    fi
    # Compared in bash only (no diff/comm dependency: fedora:40 ships neither by
    # default); any failure counts as a difference (fail closed).
    local differences=0 current hash rel
    local -A recorded_hashes=() current_hashes=()
    while read -r hash rel; do
        [[ -n "${rel}" ]] || continue
        recorded_hashes["${rel}"]="${hash}"
    done < "${SNAPSHOT_DIR}/SHA256SUMS"
    if ! current="$(current_manifest)"; then
        echo "[WARN]  Unable to hash the current PAM state."
        return 1
    fi
    while read -r hash rel; do
        [[ -n "${rel}" ]] || continue
        current_hashes["${rel}"]="${hash}"
    done <<< "${current}"
    if [[ "${#recorded_hashes[@]}" -eq 0 ]]; then
        echo "[WARN]  Empty pre-install manifest: ${SNAPSHOT_DIR}/SHA256SUMS"
        return 1
    fi
    for rel in "${!recorded_hashes[@]}"; do
        if [[ -z "${current_hashes[${rel}]+set}" ]]; then
            echo "[WARN]  PAM state differs from the pre-install snapshot: removed ${rel}"
            differences=1
        elif [[ "${current_hashes[${rel}]}" != "${recorded_hashes[${rel}]}" ]]; then
            echo "[WARN]  PAM state differs from the pre-install snapshot: changed ${rel}"
            differences=1
        fi
    done
    for rel in "${!current_hashes[@]}"; do
        if [[ -z "${recorded_hashes[${rel}]+set}" ]]; then
            echo "[WARN]  PAM state differs from the pre-install snapshot: added ${rel}"
            differences=1
        fi
    done
    if authselect_live && [[ -f "${SNAPSHOT_DIR}/authselect.current" ]]; then
        local recorded current
        recorded="$(cat "${SNAPSHOT_DIR}/authselect.current")"
        current="$(authselect current --raw 2>/dev/null || true)"
        if [[ "${recorded}" != "${current}" ]]; then
            echo "[WARN]  authselect profile differs: recorded '${recorded}', current '${current}'"
            differences=1
        fi
    fi
    if [[ "${differences}" -eq 0 ]]; then
        echo "[OK]    PAM state identical to the pre-install snapshot."
        return 0
    fi
    echo "[WARN]  Pre-install copies are kept in ${SNAPSHOT_DIR}/pam.d"
    return 1
}

cmd_discard() {
    refuse_symlinks
    rm -rf "${SNAPSHOT_DIR}"
}

case "${COMMAND}" in
    snapshot) cmd_snapshot ;;
    verify)
        rc=0
        cmd_verify || rc=$?
        exit "${rc}"
        ;;
    discard) cmd_discard ;;
esac
exit 0
