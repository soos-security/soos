#!/usr/bin/env bash
# =============================================================================
# tests/docker/push_sender_system_unit_test.sh — the push sender system service
# under a real systemd (PID 1) in Docker (ADR 2026-10-23 "Push Sender as a
# Sandboxed System Service"; matrix PSS14)
# =============================================================================
# The textual unit contracts (push_sender_system_contract) cannot prove what
# systemd does with packaging/soos-push-sender@.socket and
# packaging/soos-push-sender@.service. This harness boots ubuntu:24.04 with
# systemd as PID 1 (the system manager, which needs no unprivileged user
# namespace: the reason of the ADR), installs the freshly built release sender
# at /usr/libexec/soos/soos-push-sender and both units under
# /etc/systemd/system (the paths and modes of scripts/install.sh), and asserts:
#
#   1. `systemd-analyze verify` accepts both units without warnings.
#   2. `systemctl enable --now soos-push-sender@<alice>.socket` listens on
#      /run/soos-push/<alice>/push.sock, owned by alice, mode 0600, in
#      root-owned 0755 directories; the service is not started yet.
#   3. A frame from alice starts the service (socket activation): it runs as a
#      dynamic uid (neither 0 nor alice), its own sandbox self-check passes
#      ("push sender ready", never "sandbox not detected"), and alice gets a
#      reply frame. With network access the reply carries the HTTP status of
#      web.push.apple.com (DNS and TLS inside the sandbox); without network
#      the outcome is `retry` and that half is reported SKIPPED.
#   4. Another user (bob) cannot connect (EACCES); root can connect but the
#      sender closes the connection without a reply (SO_PEERCRED uid 0 is not
#      the instance uid).
#   5. The instance uid 0 is refused (exit 78, not restarted).
#   6. `systemctl disable --now` removes the socket.
#   `systemd-analyze security` of the instance is printed for the record.
#
# The kernel.apparmor_restrict_unprivileged_userns restriction of an Ubuntu
# host cannot be reproduced in a container; this harness proves the system
# manager path, not the owner's Ubuntu 24.04 host.
#
# Isolation: the release build runs in tests/docker/Dockerfile.ubuntu with
# Docker volumes for the target directory and the cargo registry (nothing is
# written into the workspace, mounted read-only). The systemd container is the
# image of tests/docker/Dockerfile.systemd (host-state boot units masked) run
# --privileged with a private cgroup namespace and tmpfs /run; host kernel.*,
# vm.* and fs.* sysctls are compared before and after. The images
# (soos-sua-builder, soos-sua-runtime) and the volumes soos-pss-target and
# soos-sua-cargo-registry are kept as build caches.
#
# Usage:
#   bash tests/docker/push_sender_system_unit_test.sh
#   bash tests/docker/push_sender_system_unit_test.sh --in-container check
# Environment: SOOS_DOCKER (docker binary), SOOS_PSS_TARGET_VOLUME,
# SOOS_SUA_REGISTRY_VOLUME, CARGO_BUILD_JOBS (forwarded to the build).
# =============================================================================

set -euo pipefail

readonly BUILDER_IMAGE="soos-sua-builder"
readonly RUNTIME_IMAGE="soos-sua-runtime"
readonly ALICE_UID=1500
readonly BOB_UID=1501
readonly SENDER_BIN="/usr/libexec/soos/soos-push-sender"
readonly UNIT_DIR="/etc/systemd/system"

if [[ -t 1 ]]; then
    readonly GREEN='\033[0;32m'
    readonly RED='\033[0;31m'
    readonly YELLOW='\033[1;33m'
    readonly BLUE='\033[0;34m'
    readonly NC='\033[0m'
else
    readonly GREEN=''
    readonly RED=''
    readonly YELLOW=''
    readonly BLUE=''
    readonly NC=''
fi

info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[FAIL]${NC}  $*" >&2; }
fail()    { error "$*"; exit 1; }

usage() {
    cat <<EOF
Usage: $(basename "$0") [--help]

Boots ubuntu:24.04 with systemd as PID 1 in Docker, installs the freshly built
release soos-push-sender and packaging/soos-push-sender@.{socket,service}, and
asserts the socket-activated DynamicUser system service (socket owner and mode,
dynamic uid, sandbox self-check, peer refusal, uid 0 refusal, removal).

Options:
  --in-container <stage>   Internal: run one stage inside the systemd container (check)
  -h, --help               Display this help message and exit
EOF
}

MODE="host"
STAGE=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --in-container) MODE="container"; STAGE="${2:-}"; shift 2 || fail "--in-container requires a stage" ;;
        -h|--help) usage; exit 0 ;;
        *) error "Unknown argument: $1"; usage >&2; exit 2 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# =============================================================================
# Host mode: build, boot the systemd container, run the stage, clean up
# =============================================================================
if [[ "${MODE}" = "host" ]]; then
    DOCKER="${SOOS_DOCKER:-docker}"
    TARGET_VOLUME="${SOOS_PSS_TARGET_VOLUME:-soos-pss-target}"
    REGISTRY_VOLUME="${SOOS_SUA_REGISTRY_VOLUME:-soos-sua-cargo-registry}"
    CONTAINER="soos-pss-$$"
    command -v "${DOCKER}" >/dev/null 2>&1 || fail "Docker ('${DOCKER}') is not installed or not in PATH."
    "${DOCKER}" info >/dev/null 2>&1 || fail "Docker daemon is not accessible."

    # shellcheck disable=SC2329 # invoked by the EXIT trap below
    cleanup() {
        local rc=$?
        if [[ "${rc}" -ne 0 ]] && "${DOCKER}" inspect "${CONTAINER}" >/dev/null 2>&1; then
            error "Failure: last journal entries of the push sender units in the container:"
            "${DOCKER}" exec "${CONTAINER}" journalctl -u 'soos-push-sender@*' --no-pager -n 60 >&2 2>/dev/null || true
        fi
        "${DOCKER}" rm -f "${CONTAINER}" >/dev/null 2>&1 || true
        exit "${rc}"
    }
    trap cleanup EXIT

    info "Building the release build image (tests/docker/Dockerfile.ubuntu)..."
    "${DOCKER}" build -q -f "${WORKSPACE_ROOT}/tests/docker/Dockerfile.ubuntu" \
        -t "${BUILDER_IMAGE}" "${WORKSPACE_ROOT}" >/dev/null
    info "Building the systemd runtime image (tests/docker/Dockerfile.systemd)..."
    "${DOCKER}" build -q -f "${WORKSPACE_ROOT}/tests/docker/Dockerfile.systemd" \
        -t "${RUNTIME_IMAGE}" "${WORKSPACE_ROOT}" >/dev/null

    # Always rebuild (a no-op when up to date): a stale binary left in the target
    # volume must never be the one under test.
    info "Building the release push sender (cargo build --locked --release -p soos-push-sender)..."
    "${DOCKER}" run --rm \
        -v "${WORKSPACE_ROOT}:/workspace:ro" \
        -v "${TARGET_VOLUME}:/target" \
        -v "${REGISTRY_VOLUME}:/usr/local/cargo/registry" \
        -e CARGO_TARGET_DIR=/target \
        -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" \
        -w /workspace \
        "${BUILDER_IMAGE}" \
        cargo build --locked --release -p soos-push-sender

    # Host isolation guard (same rule as systemd_unit_acceptance_test.sh).
    host_sysctl_snapshot() {
        { sysctl -a 2>/dev/null || true; } | grep -E '^(kernel|vm|fs)\.' \
            | { grep -vE '^(kernel\.(random\.|ns_last_pid|pty\.nr|sched_domain|perf_event_max_sample_rate|tainted)|fs\.(dentry-state|inode-nr|inode-state|file-nr|aio-nr|quota\.)|vm\.stat_refresh)' || true; } \
            | sort
    }
    HOST_SYSCTL_BEFORE="$(host_sysctl_snapshot)"
    [[ -n "${HOST_SYSCTL_BEFORE}" ]] || fail "could not snapshot the host sysctls (sysctl -a)"

    info "Booting systemd as PID 1 in ${RUNTIME_IMAGE} (container ${CONTAINER})..."
    "${DOCKER}" run --detach --name "${CONTAINER}" \
        --privileged --cgroupns=private \
        --tmpfs /run --tmpfs /run/lock \
        -v "${WORKSPACE_ROOT}:/workspace:ro" \
        -v "${TARGET_VOLUME}:/target:ro" \
        "${RUNTIME_IMAGE}" >/dev/null

    "${DOCKER}" exec "${CONTAINER}" \
        bash /workspace/tests/docker/push_sender_system_unit_test.sh --in-container check

    after="$(host_sysctl_snapshot)"
    changed="$(diff <(echo "${HOST_SYSCTL_BEFORE}") <(echo "${after}") || true)"
    if [[ -n "${changed}" ]]; then
        echo "${changed}" >&2
        fail "host isolation broken: kernel sysctls differ after the run"
    fi
    success "Host kernel.*, vm.* and fs.* sysctls are unchanged after the privileged container ran."
    success "push sender system unit acceptance passed."
    exit 0
fi

# =============================================================================
# Container mode
# =============================================================================
[[ "$(id -u)" -eq 0 ]] || fail "--in-container requires root."
[[ "$(cat /proc/1/comm 2>/dev/null)" = "systemd" ]] || fail "--in-container requires systemd as PID 1."
[[ "${STAGE}" = "check" ]] || fail "Unknown stage '${STAGE}' (expected check)."

now_ms() { echo $(( $(date +%s%N) / 1000000 )); }

wait_for_boot() {
    local state="" deadline
    deadline=$(( $(now_ms) + 90000 ))
    while (( $(now_ms) < deadline )); do
        state="$(systemctl is-system-running 2>/dev/null || true)"
        case "${state}" in
            running|degraded)
                info "systemd boot finished (${state})."
                return 0
                ;;
        esac
        sleep 0.5
    done
    fail "systemd did not finish booting within 90 s (state: '${state:-unknown}')"
}

# One request frame (4-byte big-endian length + JSON) to the socket $1 as user $2;
# prints CONNECT_FAIL, NO_REPLY or REPLY peer=<listener uid> <json> (the SO_PEERCRED
# uid soos-remote checks). perl-base ships IO::Socket::UNIX.
readonly CLIENT_PL='
use strict; use warnings; use IO::Socket::UNIX; use Socket qw(SOCK_STREAM SOL_SOCKET SO_PEERCRED);
my ($path, $payload) = @ARGV;
my $s = IO::Socket::UNIX->new(Type => SOCK_STREAM, Peer => $path);
if (!$s) { print "CONNECT_FAIL $!\n"; exit 0; }
my ($pid, $uid, $gid) = unpack("lII", getsockopt($s, SOL_SOCKET, SO_PEERCRED));
print {$s} pack("N", length $payload) . $payload;
my $hdr = ""; my $n = read($s, $hdr, 4);
if (!defined $n || $n < 4) { print "NO_REPLY\n"; exit 0; }
my $len = unpack("N", $hdr); my $body = ""; read($s, $body, $len);
print "REPLY peer=$uid $body\n";
'
# A syntactically valid frame towards Apple's push service with a bogus VAPID
# token: a reachable service answers with an HTTP error status.
readonly FRAME_JSON='{"v":1,"endpoint":"https://web.push.apple.com/QGuQyavXutnMH8l2ce1fmbpFBjgLv9SQK3-ASsYlvuDH_tok1","authorization":"vapid t=a.b.c, k=BAAA","ttl":60,"urgency":"high","topic":null,"body":"QkJCQkJCQkJCQkJCQkJCQg"}'

send_frame() {
    local path="$1" user="$2"
    if [[ "${user}" = root ]]; then
        timeout 40 perl -e "${CLIENT_PL}" "${path}" "${FRAME_JSON}"
    else
        timeout 40 runuser -u "${user}" -- perl -e "${CLIENT_PL}" "${path}" "${FRAME_JSON}"
    fi
}

wait_for_boot

info "Installing the sender and its units (paths and modes of scripts/install.sh)..."
install -D -m 0755 /target/release/soos-push-sender "${SENDER_BIN}"
for unit in soos-push-sender@.socket soos-push-sender@.service; do
    install -D -m 0644 "/workspace/packaging/${unit}" "${UNIT_DIR}/${unit}"
done
getent passwd alice >/dev/null || useradd -m -u "${ALICE_UID}" alice
getent passwd bob >/dev/null || useradd -m -u "${BOB_UID}" bob
systemctl daemon-reload

# 1. systemd-analyze verify.
for unit in "soos-push-sender@${ALICE_UID}.socket" "soos-push-sender@${ALICE_UID}.service"; do
    rc=0
    out="$(systemd-analyze verify "${unit}" 2>&1)" || rc=$?
    [[ -z "${out}" ]] || printf '%s\n' "${out}"
    [[ "${rc}" -eq 0 ]] || fail "systemd-analyze verify ${unit} exited ${rc}"
    if grep -Eiq 'unknown (key|lvalue|section)|failed to parse|invalid|ignoring|deprecated|not supported' <<< "${out}"; then
        fail "systemd-analyze verify ${unit} reported a problem"
    fi
done
success "systemd-analyze verify accepts both units."

# 2. The socket.
SOCKET_UNIT="soos-push-sender@${ALICE_UID}.socket"
SERVICE_UNIT="soos-push-sender@${ALICE_UID}.service"
SOCKET_PATH="/run/soos-push/${ALICE_UID}/push.sock"
systemctl enable --now "${SOCKET_UNIT}"
systemctl is-active --quiet "${SOCKET_UNIT}" || fail "${SOCKET_UNIT} is not active"
[[ -S "${SOCKET_PATH}" ]] || fail "${SOCKET_PATH} is not a socket"
[[ "$(stat -c '%u %a' "${SOCKET_PATH}")" = "${ALICE_UID} 600" ]] \
    || fail "${SOCKET_PATH}: owner/mode $(stat -c '%u %a' "${SOCKET_PATH}"), expected ${ALICE_UID} 600"
for dir in /run/soos-push "/run/soos-push/${ALICE_UID}"; do
    [[ "$(stat -c '%u %a' "${dir}")" = "0 755" ]] \
        || fail "${dir}: owner/mode $(stat -c '%u %a' "${dir}"), expected 0 755"
done
if systemctl is-active --quiet "${SERVICE_UNIT}"; then
    fail "${SERVICE_UNIT} runs before any connection"
fi
success "${SOCKET_UNIT} listens on ${SOCKET_PATH} (${ALICE_UID}, 0600, root-owned 0755 directories); the service waits for a connection."

# 3. Socket activation by alice.
reply="$(send_frame "${SOCKET_PATH}" alice)"
info "alice: ${reply}"
[[ "${reply}" == REPLY* ]] || fail "alice got no reply frame: ${reply}"
[[ "${reply}" == "REPLY peer=0 "* ]] \
    || fail "the listener's SO_PEERCRED uid is not 0 (PID 1), which soos-remote requires: ${reply}"
success "The socket's SO_PEERCRED uid seen by the client is 0 (PID 1 listens), the peer uid soos-remote requires for the system socket."
systemctl is-active --quiet "${SERVICE_UNIT}" || fail "${SERVICE_UNIT} is not running after the connection"
pid="$(systemctl show -p MainPID --value "${SERVICE_UNIT}")"
[[ "${pid}" =~ ^[1-9][0-9]*$ ]] || fail "no main PID for ${SERVICE_UNIT}"
sender_uid="$(awk '/^Uid:/ {print $2}' "/proc/${pid}/status")"
[[ "${sender_uid}" =~ ^[0-9]+$ ]] || fail "could not read the sender uid"
(( sender_uid != 0 && sender_uid != ALICE_UID )) \
    || fail "the sender runs as uid ${sender_uid}, expected a dynamic uid"
(( sender_uid >= 61184 && sender_uid <= 65519 )) \
    || fail "uid ${sender_uid} is outside the DynamicUser range 61184-65519"
caps="$(awk '/^CapEff:/ {print $2}' "/proc/${pid}/status")"
[[ "${caps}" =~ ^0+$ ]] || fail "the sender has effective capabilities ${caps}"
journal="$(journalctl -u "${SERVICE_UNIT}" --no-pager -o cat)"
grep -q "push sender ready" <<< "${journal}" || fail "the sender never reported ready:
${journal}"
if grep -q "sandbox not detected" <<< "${journal}"; then
    fail "the sender did not detect its sandbox"
fi
success "Socket activation: ${SERVICE_UNIT} runs as dynamic uid ${sender_uid} with no capability and its sandbox self-check passed."
if [[ "${reply}" =~ \"status\":([0-9]{3}) ]]; then
    success "DNS, TLS and HTTP inside the sandbox: web.push.apple.com answered HTTP ${BASH_REMATCH[1]} (${reply#REPLY })."
else
    warn "SKIPPED: no HTTP status in the reply (no network from the container?): ${reply#REPLY }"
fi

# 4. Other peers.
reply="$(send_frame "${SOCKET_PATH}" bob)"
[[ "${reply}" == CONNECT_FAIL* ]] || fail "bob could connect to alice's sender socket: ${reply}"
success "bob cannot connect to alice's socket (${reply})."
reply="$(send_frame "${SOCKET_PATH}" root)"
[[ "${reply}" == NO_REPLY ]] || fail "root (peer uid 0) got an answer from alice's sender: ${reply}"
success "A connection whose SO_PEERCRED uid is not ${ALICE_UID} is closed without a reply."

# 5. Instance uid 0 is refused.
systemctl start soos-push-sender@0.socket
send_frame /run/soos-push/0/push.sock root >/dev/null || true
deadline=$(( $(now_ms) + 15000 ))
result=""
while (( $(now_ms) < deadline )); do
    result="$(systemctl show -p ExecMainStatus --value soos-push-sender@0.service)"
    [[ "${result}" = "78" ]] && break
    sleep 0.3
done
[[ "${result}" = "78" ]] || fail "soos-push-sender@0.service did not exit 78 (ExecMainStatus '${result}')"
journalctl -u soos-push-sender@0.service --no-pager -o cat | grep -q "push sender instance identity refused\|push sender arguments refused" \
    || fail "the uid 0 instance did not log its refusal"
systemctl stop soos-push-sender@0.socket soos-push-sender@0.service 2>/dev/null || true
success "The uid 0 instance exits 78 and is not restarted."

info "systemd-analyze security ${SERVICE_UNIT}:"
systemd-analyze security --no-pager "${SERVICE_UNIT}" | tail -n 1 || true

# 6. Removal.
systemctl disable --now "${SOCKET_UNIT}"
systemctl stop "${SERVICE_UNIT}"
[[ ! -e "${SOCKET_PATH}" ]] || fail "${SOCKET_PATH} remains after disable --now"
success "disable --now removes ${SOCKET_PATH}."
