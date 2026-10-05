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
#   3. 'soos-admin --format json --socket-path <sock> status' exits 0, i.e.
#      the daemon reports "is_healthy": true. Right after a (re)start the
#      camera is still starting and the status is unhealthy, so the query is
#      repeated every 0.5 s within the same --timeout bound (GitHub #329).
#      Every attempt is bounded by 'timeout --kill-after=1 5' when coreutils
#      'timeout' supports it. Only the final attempt's report is printed: the
#      healthy one on success, the last completed one on timeout. On timeout
#      the error also shows the last non-empty stderr of 'soos-admin status'
#      (at most 1024 characters, control characters printed as '?'). An exit
#      status of 126 or 127 (soos-admin not executable / not found) stops at
#      once with exit 1 (GitHub #331).
#
# Bound: --timeout bounds the socket wait and the
# start of the last status attempt; the worst case is about --timeout + 7.5 s
# (1 s SECONDS granularity, 0.5 s poll, one 5 s attempt plus 1 s kill grace),
# i.e. about 37.5 s with the default. Without coreutils 'timeout --kill-after'
# an attempt is not bounded.
#
# Options:
#   --socket <PATH>     Daemon socket (default: /run/soos/daemon.sock)
#   --manifest <PATH>   Deployed models manifest (default: /var/lib/soos/models/manifest.toml)
#   --admin <PATH>      soos-admin binary (default: soos-admin from PATH)
#   --timeout <SECS>    Maximum wait for the socket and a healthy status,
#                       1..300 (default: 30)
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
readonly POLL_INTERVAL_S=0.5
readonly ATTEMPT_TIMEOUT_S=5
readonly ADMIN_STDERR_TAIL_MAX=1024
# Bytes of one attempt's stderr ever held in memory (only the last ones are kept).
readonly ADMIN_STDERR_CAPTURE_BYTES=4096

usage() {
    cat <<EOF
Usage: $(basename "$0") [--socket PATH] [--manifest PATH] [--admin PATH] [--timeout SECS]

Waits (bounded) until soos-daemon reports healthy, then prints 'soos-admin status' as JSON.
--timeout (1..300, default 30) bounds the socket wait and the start of the last status
attempt; the worst case is about --timeout + 7.5 s (1 s SECONDS granularity, 0.5 s poll,
one 5 s attempt plus 1 s kill grace). Without coreutils 'timeout --kill-after' an attempt
is not bounded.
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

# One bound covers the socket wait and the health poll. Only SECONDS deltas
# are used (SECONDS may be inherited from the environment; never 'date').
start=${SECONDS}

# 2. Bounded wait for the socket (two polls per second).
polls=$((TIMEOUT_S * 2))
while [[ ! -S "${SOCKET_PATH}" ]]; do
    if (( polls <= 0 )); then
        echo "[ERROR] soos-daemon is not ready: no socket at ${SOCKET_PATH} after ${TIMEOUT_S} s." >&2
        echo "        Inspect: systemctl status soos-daemon.service; journalctl -u soos-daemon.service -n 50" >&2
        exit 1
    fi
    polls=$((polls - 1))
    sleep "${POLL_INTERVAL_S}"
done

# 3. Poll the status until the daemon reports healthy (bounded).
# Each attempt is bounded by coreutils 'timeout' (whole process group, no
# --foreground) once a capability probe shows it supports --kill-after;
# otherwise the attempt runs unwrapped (degraded mode).
attempt_prefix=()
if command -v timeout >/dev/null 2>&1 \
    && timeout --kill-after=1 "${ATTEMPT_TIMEOUT_S}" true >/dev/null 2>&1; then
    attempt_prefix=(timeout --kill-after=1 "${ATTEMPT_TIMEOUT_S}")
else
    echo "[WARN] coreutils 'timeout --kill-after' is unavailable: status attempts are not individually bounded." >&2
fi

# Keeps the last ADMIN_STDERR_TAIL_MAX characters of a stderr text.
bounded_tail() {
    local text="$1"
    if (( ${#text} > ADMIN_STDERR_TAIL_MAX )); then
        text="${text:${#text}-ADMIN_STDERR_TAIL_MAX}"
    fi
    printf '%s' "${text}"
}

# Prints a stderr tail on stderr, each line indented by 8 spaces, every control
# character (except the line breaks) replaced by '?'; never interpreted.
print_tail() {
    local text="$1" line
    while [[ "${text}" == *$'\n' ]]; do
        text="${text%$'\n'}"
    done
    [[ -n "${text}" ]] || return 0
    while IFS= read -r line; do
        printf '        %s\n' "${line//[[:cntrl:]]/?}" >&2
    done <<< "${text}"
}

q_admin="$(printf '%q' "${ADMIN_BIN}")"
last_report=""
have_report=0
last_error=""
while true; do
    # One attempt: stdout, stderr and exit status captured separately, in memory
    # ("<stderr>\037<status>\037<stdout>", parsed from the right). Stderr passes through
    # 'tail -c', so at most ADMIN_STDERR_CAPTURE_BYTES of it are ever held. A report that
    # contains a raw \037 is not valid soos-admin JSON: it is dropped.
    captured="$(
        {
            attempt_rc=0
            out="$(
                {
                    { ${attempt_prefix[@]+"${attempt_prefix[@]}"} "${ADMIN_BIN}" --format json --socket-path "${SOCKET_PATH}" status 2>&1 1>&5 5>&-; } \
                        | tail -c "${ADMIN_STDERR_CAPTURE_BYTES}" >&3
                    exit "${PIPESTATUS[0]}"
                } 5>&1
            )" || attempt_rc=$?
            if [[ "${out}" == *$'\037'* ]]; then
                out=""
                (( attempt_rc != 0 )) || attempt_rc=1
            fi
            printf '\037%s\037%s' "${attempt_rc}" "${out}"
        } 3>&1
    )"
    report="${captured##*$'\037'}"
    rest="${captured%$'\037'*}"
    rc="${rest##*$'\037'}"
    err="${rest%$'\037'*}"
    [[ "${rc}" =~ ^[0-9]+$ ]] || rc=1
    if (( rc == 0 )); then
        printf '%s\n' "${report}"
        exit 0
    fi
    if (( rc == 126 || rc == 127 )); then
        if (( rc == 126 )); then
            reason="found but not executable"
        else
            reason="not found"
        fi
        printf '%s\n' "[ERROR] Cannot run '${q_admin}' (exit ${rc}: ${reason}); readiness cannot be checked." >&2
        printf '%s\n' "        Pass --admin <PATH> with the installed soos-admin binary." >&2
        print_tail "$(bounded_tail "${err}")"
        exit 1
    fi
    # A killed attempt (124: timed out, 137: SIGKILL) contributes no output.
    if (( rc == 124 || rc == 137 )); then
        last_error="status attempt killed after ${ATTEMPT_TIMEOUT_S} s (no answer)"
    else
        if [[ -n "${report}" ]]; then
            last_report="${report}"
            have_report=1
        fi
        if [[ -n "${err}" ]]; then
            last_error="$(bounded_tail "${err}")"
        fi
    fi
    if (( SECONDS - start >= TIMEOUT_S + 1 )); then
        break
    fi
    sleep "${POLL_INTERVAL_S}"
done

if (( have_report == 1 )); then
    printf '%s\n' "${last_report}"
fi
echo "[ERROR] soos-daemon did not report healthy within ${TIMEOUT_S} s ('${ADMIN_BIN} status' kept failing)." >&2
if [[ -n "${last_error}" ]]; then
    printf '%s\n' "        Last '${q_admin} status' error:" >&2
    print_tail "${last_error}"
fi
echo "        Inspect: soos-admin status; journalctl -u soos-daemon.service -n 50" >&2
exit 1
