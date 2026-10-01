#!/usr/bin/env bash
# =============================================================================
# scripts/camera_report.sh — Informational camera selection report
# =============================================================================
# Prints 'soos-admin camera list' for the installed daemon configuration, so the
# operator sees which V4L2 node soos-daemon would open and why (GitHub #287).
# Used by 'scripts/install.sh' after the install is committed.
#
# The report is metadata only (VIDIOC_QUERYCAP / ENUM_FMT / ENUM_FRAMESIZES; no
# frame is captured) and purely informational: whatever happens (no camera,
# missing binary, probe hang), this helper exits 0. The probe is bounded by
# --timeout when coreutils 'timeout' is available.
#
# Options:
#   --admin <PATH>      soos-admin binary (default: soos-admin from PATH)
#   --config <PATH>     Daemon configuration (default: /etc/soos/daemon.toml)
#   --timeout <SECS>    Maximum run time, 1..300 (default: 15)
#   -h, --help          Display this help message
#
# Exit codes: 0 always (2 only for a usage error).
# =============================================================================

set -uo pipefail

ADMIN_BIN="soos-admin"
CONFIG_PATH="/etc/soos/daemon.toml"
TIMEOUT_S=15
readonly MAX_TIMEOUT_S=300

usage() {
    cat <<EOF
Usage: $(basename "$0") [--admin PATH] [--config PATH] [--timeout SECS]

Prints 'soos-admin camera list --config PATH' (informational, always exits 0).
EOF
}

usage_error() {
    echo "[ERROR] $*" >&2
    usage >&2
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --admin|--config|--timeout)
            [[ $# -ge 2 ]] || usage_error "Option $1 requires a value"
            case "$1" in
                --admin) ADMIN_BIN="$2" ;;
                --config) CONFIG_PATH="$2" ;;
                --timeout) TIMEOUT_S="$2" ;;
            esac
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage_error "Unknown option: $1"
            ;;
    esac
done

if [[ ! "${TIMEOUT_S}" =~ ^[0-9]{1,3}$ ]] || (( 10#${TIMEOUT_S} < 1 || 10#${TIMEOUT_S} > MAX_TIMEOUT_S )); then
    usage_error "--timeout must be an integer between 1 and ${MAX_TIMEOUT_S} seconds: '${TIMEOUT_S}'"
fi
TIMEOUT_S=$((10#${TIMEOUT_S}))

if ! command -v "${ADMIN_BIN}" >/dev/null 2>&1; then
    echo "[WARN]  ${ADMIN_BIN} not found; camera report skipped (run: sudo soos-admin camera list)"
    exit 0
fi

RC=0
if command -v timeout >/dev/null 2>&1; then
    timeout --kill-after=2 "${TIMEOUT_S}" "${ADMIN_BIN}" camera list --config "${CONFIG_PATH}" || RC=$?
else
    "${ADMIN_BIN}" camera list --config "${CONFIG_PATH}" || RC=$?
fi

case "${RC}" in
    0) ;;
    1)
        echo "[WARN]  No camera would be selected; check the camera connection and"
        echo "        [pipeline] camera_device / sensor_preference in ${CONFIG_PATH}"
        ;;
    124|137)
        echo "[WARN]  Camera report timed out after ${TIMEOUT_S} s (a V4L2 node did not answer)"
        ;;
    *)
        echo "[WARN]  Camera report failed (exit ${RC}); run: sudo soos-admin camera list"
        ;;
esac
exit 0
