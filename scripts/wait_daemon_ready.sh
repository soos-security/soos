#!/usr/bin/env bash
# =============================================================================
# scripts/wait_daemon_ready.sh — Bounded soos-daemon readiness probe
# =============================================================================
# Waits until soos-daemon is ready after a (first) start, then prints the
# machine-readable status (GitHub #211). Used by 'scripts/install.sh --start'
# and by operators after 'systemctl start soos-daemon'.
#
# Readiness means, in order:
#   1. the models manifest is deployed (otherwise ConditionPathExists= in
#      soos-daemon.service skips the start: fail immediately, never wait);
#   2. the daemon socket exists and is a Unix socket (polled every 0.5 s, for
#      at most --timeout seconds);
#   3. 'soos-admin --format json --socket-path <sock> status' succeeds.
#
# Options:
#   --socket <PATH>     Daemon socket (default: /run/soos/daemon.sock)
#   --manifest <PATH>   Deployed models manifest (default: /var/lib/soos/models/manifest.toml)
#   --admin <PATH>      soos-admin binary (default: soos-admin from PATH)
#   --timeout <SECS>    Maximum wait for the socket, 1..300 (default: 30)
#   -h, --help          Display this help message
#
# Exit codes: 0 ready, 1 not ready, 2 usage error.
# =============================================================================

set -euo pipefail

SOCKET_PATH="/run/soos/daemon.sock"
MANIFEST_PATH="/var/lib/soos/models/manifest.toml"
ADMIN_BIN="soos-admin"
TIMEOUT_S=30
readonly MAX_TIMEOUT_S=300

usage() {
    cat <<EOF
Usage: $(basename "$0") [--socket PATH] [--manifest PATH] [--admin PATH] [--timeout SECS]

Waits (bounded) until soos-daemon answers, then prints 'soos-admin status' as JSON.
Exit codes: 0 ready, 1 not ready, 2 usage error.
EOF
}

usage_error() {
    echo "[ERROR] $*" >&2
    usage >&2
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --socket|--manifest|--admin|--timeout)
            [[ $# -ge 2 ]] || usage_error "Option $1 requires a value"
            case "$1" in
                --socket) SOCKET_PATH="$2" ;;
                --manifest) MANIFEST_PATH="$2" ;;
                --admin) ADMIN_BIN="$2" ;;
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
[[ -n "${SOCKET_PATH}" && -n "${MANIFEST_PATH}" && -n "${ADMIN_BIN}" ]] \
    || usage_error "--socket, --manifest and --admin must not be empty"

# 1. Models deployed? Without them the unit condition skips the start.
if [[ ! -f "${MANIFEST_PATH}" ]]; then
    echo "[ERROR] Models are not deployed (${MANIFEST_PATH} is missing): soos-daemon.service will not start" >&2
    echo "        (ConditionPathExists). Deploy them with: sudo scripts/download_models.sh" >&2
    exit 1
fi

# 2. Bounded wait for the socket (two polls per second).
polls=$((TIMEOUT_S * 2))
while [[ ! -S "${SOCKET_PATH}" ]]; do
    if (( polls <= 0 )); then
        echo "[ERROR] soos-daemon is not ready: no socket at ${SOCKET_PATH} after ${TIMEOUT_S} s." >&2
        echo "        Inspect: systemctl status soos-daemon.service; journalctl -u soos-daemon.service -n 50" >&2
        exit 1
    fi
    polls=$((polls - 1))
    sleep 0.5
done

# 3. The daemon answers a status query.
if ! "${ADMIN_BIN}" --format json --socket-path "${SOCKET_PATH}" status; then
    echo "[ERROR] soos-daemon socket exists but '${ADMIN_BIN} status' failed." >&2
    echo "        Inspect: journalctl -u soos-daemon.service -n 50" >&2
    exit 1
fi
exit 0
