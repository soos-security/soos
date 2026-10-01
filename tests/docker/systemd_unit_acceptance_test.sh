#!/usr/bin/env bash
# =============================================================================
# tests/docker/systemd_unit_acceptance_test.sh — soos-daemon.service under a
# real systemd (PID 1) in Docker (GitHub #211, ONB-15; matrix IRP8, SUA1–SUA6)
# =============================================================================
# The textual unit contracts (installer_templates_contract, systemd_*_tests)
# cannot prove what systemd does with the unit. This harness boots ubuntu:24.04
# with systemd as PID 1, installs the freshly built release daemon and the
# shipped packaging/soos-daemon.service with scripts/install.sh (live install,
# exactly like an operator), and drives the unit with systemctl:
#
#   Part 4. `systemd-analyze verify` accepts the installed unit (byte-identical
#           to packaging/soos-daemon.service) without warnings, every hardening
#           directive is in effect, and the shipped values (Type=notify,
#           TimeoutStartSec=60, RestartSec=2, StartLimitBurst=5,
#           StartLimitIntervalSec=320) are what systemd loaded.
#   Part 1. No /var/lib/soos/models/manifest.toml: `systemctl start` leaves the
#           unit inactive with ConditionResult=no ("condition failed"), never
#           runs the daemon and never restarts it; after a reboot of the
#           container the enabled unit is skipped at boot the same way.
#   Part 3. A daemon that always fails to start (manifest present, corrupt
#           models): systemd restarts it, then stops after exactly
#           StartLimitBurst=5 runs inside StartLimitIntervalSec=320 ("Start
#           request repeated too quickly", ActiveState=failed, start-limit-hit);
#           a later `systemctl start` is refused without running the daemon.
#           The shipped timing values are used unchanged (no drop-in).
#   Part 2. Real models deployed by scripts/download_models.sh and the mock
#           camera: `systemctl start` returns only after READY=1 (Type=notify);
#           at that moment /run/soos/daemon.sock exists (0660 root:soos),
#           ActiveState=active, and scripts/wait_daemon_ready.sh reports a
#           healthy daemon. The start-to-ready time is measured
#           (ActiveEnterTimestamp - ExecMainStartTimestamp) and must stay well
#           under TimeoutStartSec=60.
#   Part 5. `systemctl stop` is clean: SIGTERM, graceful drain, exit 0 well
#           within TimeoutStopSec, socket removed, no SIGKILL.
#
# Model sources for parts 2 and 5 (--models):
#   auto      host if /var/lib/soos/models/manifest.toml exists on the host,
#             otherwise none (default)
#   host      bind-mount the host /var/lib/soos/models read-only, copy the
#             .onnx files into the container, then let download_models.sh
#             verify them (no network); models never enter the workspace
#   download  scripts/download_models.sh downloads them inside the container
#             (HTTPS, size-bounded, SHA-256 verified); used by CI
#   none      parts 2 and 5 are skipped with an explicit SKIPPED notice
#
# Isolation: the host is never modified. The release build runs in the
# tests/docker/Dockerfile.ubuntu image with Docker volumes for the target
# directory and the cargo registry (nothing is written into the workspace,
# which is mounted read-only). The systemd container runs --privileged with a
# private cgroup namespace and tmpfs /run; its V4L2 nodes are deleted before
# systemd starts and the daemon only uses the mock camera.
#
# Usage:
#   bash tests/docker/systemd_unit_acceptance_test.sh [--models auto|host|download|none]
#   bash tests/docker/systemd_unit_acceptance_test.sh --in-container <stage> --models <mode>
#   bash tests/docker/systemd_unit_acceptance_test.sh --help
# Environment: SOOS_DOCKER (docker binary), SOOS_SUA_TARGET_VOLUME,
# SOOS_SUA_REGISTRY_VOLUME, CARGO_BUILD_JOBS (forwarded to the build).
# =============================================================================

set -euo pipefail

readonly UNIT="soos-daemon.service"
readonly MODELS_DIR="/var/lib/soos/models"
readonly MANIFEST="${MODELS_DIR}/manifest.toml"
readonly SOCKET="/run/soos/daemon.sock"
readonly HOST_MODELS_DIR="/var/lib/soos/models"
readonly IN_CONTAINER_HOST_MODELS="/host-models"
readonly BUILDER_IMAGE="soos-sua-builder"
readonly RUNTIME_IMAGE="soos-sua-runtime"
readonly SHIPPED_START_LIMIT_BURST=5
readonly SHIPPED_START_LIMIT_INTERVAL_S=320
# "Well under TimeoutStartSec=60": readiness must take less than half of it.
readonly MAX_READY_MS=30000
# The stop is bounded by one connection_timeout drain plus the runtime shutdown.
readonly MAX_STOP_MS=10000

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
Usage: $(basename "$0") [--models auto|host|download|none] [--help]

Boots ubuntu:24.04 with systemd as PID 1 in Docker, installs the freshly built
release soos-daemon and packaging/soos-daemon.service with scripts/install.sh,
and asserts the unit's behaviour under the real systemd manager (condition
failed without models, bounded restarts, Type=notify readiness, clean stop,
systemd-analyze verify). The host system is never modified.

Options:
  --models <mode>          Model source for the readiness and stop parts:
                           auto (default), host, download or none
  --in-container <stage>   Internal: run one stage inside the systemd container
                           (install, after-reboot)
  -h, --help               Display this help message and exit
EOF
}

MODE="host"
STAGE=""
MODELS="auto"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --models) MODELS="${2:-}"; shift 2 || fail "--models requires a value" ;;
        --in-container) MODE="container"; STAGE="${2:-}"; shift 2 || fail "--in-container requires a stage" ;;
        -h|--help) usage; exit 0 ;;
        *) error "Unknown argument: $1"; usage >&2; exit 2 ;;
    esac
done
case "${MODELS}" in
    auto|host|download|none) ;;
    *) fail "Unknown --models value '${MODELS}' (expected auto, host, download or none)" ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# =============================================================================
# Host mode: build, boot the systemd container, run the stages, clean up
# =============================================================================
if [[ "${MODE}" = "host" ]]; then
    DOCKER="${SOOS_DOCKER:-docker}"
    TARGET_VOLUME="${SOOS_SUA_TARGET_VOLUME:-soos-sua-target}"
    REGISTRY_VOLUME="${SOOS_SUA_REGISTRY_VOLUME:-soos-sua-cargo-registry}"
    CONTAINER="soos-sua-$$"
    command -v "${DOCKER}" >/dev/null 2>&1 || fail "Docker ('${DOCKER}') is not installed or not in PATH."
    "${DOCKER}" info >/dev/null 2>&1 || fail "Docker daemon is not accessible."

    if [[ "${MODELS}" = "auto" ]]; then
        if [[ -r "${HOST_MODELS_DIR}/manifest.toml" ]]; then
            MODELS="host"
        else
            MODELS="none"
        fi
        info "--models auto resolved to '${MODELS}'."
    fi
    if [[ "${MODELS}" = "host" && ! -r "${HOST_MODELS_DIR}/manifest.toml" ]]; then
        fail "--models host: ${HOST_MODELS_DIR}/manifest.toml is missing or unreadable on this host."
    fi

    # shellcheck disable=SC2329 # invoked by the EXIT trap below
    cleanup() {
        local rc=$?
        if [[ "${rc}" -ne 0 ]] && "${DOCKER}" inspect "${CONTAINER}" >/dev/null 2>&1; then
            error "Failure: last journal entries of ${UNIT} in the container:"
            "${DOCKER}" exec "${CONTAINER}" journalctl -u "${UNIT}" --no-pager -n 60 >&2 2>/dev/null || true
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

    # Always rebuild (a no-op when up to date): a stale binary left in the
    # target volume must never be the one under test (artifact freshness).
    info "Building the release daemon and soos-admin (cargo build --locked --release)..."
    "${DOCKER}" run --rm \
        -v "${WORKSPACE_ROOT}:/workspace:ro" \
        -v "${TARGET_VOLUME}:/target" \
        -v "${REGISTRY_VOLUME}:/usr/local/cargo/registry" \
        -e CARGO_TARGET_DIR=/target \
        -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}" \
        -w /workspace \
        "${BUILDER_IMAGE}" \
        cargo build --locked --release -p soos-daemon -p soos-admin-cli

    run_args=(
        --detach --name "${CONTAINER}"
        --privileged --cgroupns=private
        --tmpfs /run --tmpfs /run/lock
        -v "${WORKSPACE_ROOT}:/workspace:ro"
        -v "${TARGET_VOLUME}:/target:ro"
    )
    if [[ "${MODELS}" = "host" ]]; then
        run_args+=(-v "${HOST_MODELS_DIR}:${IN_CONTAINER_HOST_MODELS}:ro")
    fi
    info "Booting systemd as PID 1 in ${RUNTIME_IMAGE} (container ${CONTAINER})..."
    "${DOCKER}" run "${run_args[@]}" "${RUNTIME_IMAGE}" >/dev/null

    stage() {
        "${DOCKER}" exec "${CONTAINER}" \
            bash /workspace/tests/docker/systemd_unit_acceptance_test.sh \
            --in-container "$1" --models "${MODELS}"
    }

    stage install
    info "Rebooting the container (systemd shutdown, fresh /run, persistent /var)..."
    "${DOCKER}" restart --time 30 "${CONTAINER}" >/dev/null
    stage after-reboot

    success "systemd unit acceptance passed (models: ${MODELS})."
    exit 0
fi

# =============================================================================
# Container mode — helpers
# =============================================================================
[[ "$(id -u)" -eq 0 ]] || fail "--in-container requires root."
[[ "$(cat /proc/1/comm 2>/dev/null)" = "systemd" ]] || fail "--in-container requires systemd as PID 1."

prop() { systemctl show "${UNIT}" -p "$1" --value; }

# Matches stdin like `grep -q` but reads it to the end: an early exit would
# SIGPIPE the writer and fail the pipeline under pipefail.
has() { grep "$@" >/dev/null; }

expect_prop() {
    local name="$1" expected="$2" actual
    actual="$(prop "${name}")"
    [[ "${actual}" = "${expected}" ]] || fail "${UNIT}: ${name}='${actual}', expected '${expected}'"
}

now_ms() { echo $(( $(date +%s%N) / 1000000 )); }

journal_cursor() {
    journalctl --show-cursor -n 0 --no-pager 2>/dev/null | sed -n 's/^-- cursor: //p'
}

# Journal of the unit (manager and daemon lines) after cursor $1.
journal_since() {
    if [[ -n "$1" ]]; then
        journalctl -u "${UNIT}" --no-pager -o cat --after-cursor "$1"
    else
        journalctl -u "${UNIT}" --no-pager -o cat
    fi
}

# Daemon runs after cursor $1: one manager "Starting" line per executed start.
count_starts_since() {
    journal_since "$1" | grep -c '^Starting soos-daemon.service' || true
}

wait_for_boot() {
    # The manager socket appears shortly after PID 1 starts: poll, bounded.
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

# Part 4 — systemd-analyze verify, hardening directives in effect, shipped values.
part4_verify_and_shipped_values() {
    local installed="/etc/systemd/system/${UNIT}" out rc=0
    cmp -s /workspace/packaging/soos-daemon.service "${installed}" \
        || fail "${installed} differs from packaging/soos-daemon.service"
    out="$(systemd-analyze verify "${installed}" 2>&1)" || rc=$?
    [[ -z "${out}" ]] || printf '%s\n' "${out}"
    [[ "${rc}" -eq 0 ]] || fail "systemd-analyze verify ${UNIT} exited ${rc}"
    if grep -Eiq 'unknown (key|lvalue|section)|failed to parse|invalid|ignoring|deprecated|not supported' <<< "${out}"; then
        fail "systemd-analyze verify reported a rejected or ignored directive (see above)"
    fi
    success "Part 4: systemd-analyze verify ${UNIT}: exit 0, no unknown, ignored or invalid directive."

    # A directive systemd does not understand is ignored with a warning and keeps
    # its default, so the loaded values prove each hardening option took effect.
    local -a expected=(
        "Type=notify" "NotifyAccess=main" "TimeoutStartUSec=1min" "RestartUSec=2s"
        "Restart=on-failure" "StartLimitBurst=${SHIPPED_START_LIMIT_BURST}"
        "StartLimitIntervalUSec=5min 20s" "User=root" "Group=soos" "UMask=0077"
        "NoNewPrivileges=yes" "PrivateTmp=yes" "ProtectHome=yes" "ProtectSystem=strict"
        "DevicePolicy=closed" "RestrictAddressFamilies=AF_UNIX" "LockPersonality=yes"
        "MemoryDenyWriteExecute=yes" "RestrictSUIDSGID=yes" "SystemCallArchitectures=native"
        "SystemCallErrorNumber=1" "ProtectKernelTunables=yes" "ProtectKernelModules=yes"
        "ProtectKernelLogs=yes" "ProtectControlGroups=yes" "ProtectClock=yes"
        "ProtectHostname=yes" "RestrictNamespaces=yes" "RestrictRealtime=yes"
        "PrivateNetwork=yes" "RuntimeDirectoryMode=0750" "StateDirectoryMode=0755"
        "AmbientCapabilities=" "ExecMainPID=0"
    )
    local pair
    for pair in "${expected[@]}"; do
        expect_prop "${pair%%=*}" "${pair#*=}"
    done
    local caps
    caps="$(prop CapabilityBoundingSet | tr ' ' '\n' | sort | tr '\n' ' ')"
    [[ "${caps}" = "cap_chown cap_dac_override cap_fowner cap_ipc_lock " ]] \
        || fail "CapabilityBoundingSet='${caps}', expected cap_chown cap_dac_override cap_fowner cap_ipc_lock"
    prop DeviceAllow | has 'char-video4linux rw' || fail "DeviceAllow does not contain 'char-video4linux rw'"
    prop SystemCallFilter | has -w 'mlockall' || fail "SystemCallFilter=@system-service was not loaded"
    prop IPAddressDeny | has -E '0\.0\.0\.0/0|any' || fail "IPAddressDeny=any was not loaded"
    prop Before | has -w 'display-manager.service' || fail "Before= does not contain display-manager.service"
    success "Part 4: every hardening directive is in effect and the shipped values are unchanged (Type=notify, TimeoutStartSec=60, RestartSec=2, StartLimitBurst=5, StartLimitIntervalSec=320)."
    info "systemd-analyze security: $(systemd-analyze security "${UNIT}" --no-pager 2>/dev/null | tail -n 1)"
}

# Part 1 — no models: condition failed, never executed, never restarted.
part1_condition_failed_without_models() {
    [[ ! -e "${MANIFEST}" ]] || fail "precondition: ${MANIFEST} must not exist"
    [[ "$(systemctl is-enabled "${UNIT}")" = "enabled" ]] || fail "scripts/install.sh did not enable ${UNIT}"
    local cursor rc=0
    cursor="$(journal_cursor)"
    systemctl start "${UNIT}" || rc=$?
    [[ "${rc}" -eq 0 ]] || fail "systemctl start with an unmet condition exited ${rc} (expected 0: a skipped start is not a failure)"
    expect_prop ConditionResult "no"
    expect_prop ActiveState "inactive"
    expect_prop SubState "dead"
    expect_prop Result "success"
    # Three RestartSec periods: a restart loop would have run by now.
    sleep 6
    expect_prop ConditionResult "no"
    expect_prop ActiveState "inactive"
    expect_prop NRestarts "0"
    expect_prop ExecMainPID "0"
    expect_prop ExecMainStartTimestampMonotonic "0"
    journal_since "${cursor}" | has "skipped because of an unmet condition check (ConditionPathExists=${MANIFEST})" \
        || fail "the journal does not report the unmet ConditionPathExists=${MANIFEST}"
    [[ "$(count_starts_since "${cursor}")" -eq 0 ]] || fail "the daemon was executed although the condition failed"
    # systemctl status exits 3 for an inactive unit: capture, then match.
    local status_text
    status_text="$(systemctl status "${UNIT}" --no-pager 2>&1 || true)"
    grep -q "start condition unmet" <<< "${status_text}" \
        || fail "systemctl status does not show 'start condition unmet'"
    success "Part 1: without ${MANIFEST}, systemctl start leaves ${UNIT} inactive with ConditionResult=no (condition failed), 0 executions, 0 restarts after 6 s."
}

# Part 1 (boot) — the enabled unit is skipped at boot the same way.
part1_condition_failed_at_boot() {
    [[ "$(systemctl is-enabled "${UNIT}")" = "enabled" ]] || fail "${UNIT} is not enabled after the reboot"
    expect_prop ConditionResult "no"
    [[ -n "$(prop ConditionTimestamp)" ]] || fail "the condition of ${UNIT} was not evaluated at boot"
    expect_prop ActiveState "inactive"
    expect_prop NRestarts "0"
    expect_prop ExecMainPID "0"
    journalctl -b -u "${UNIT}" --no-pager -o cat \
        | has "skipped because of an unmet condition check (ConditionPathExists=${MANIFEST})" \
        || fail "the boot journal does not report the unmet condition"
    [[ "$(journalctl -b -u "${UNIT}" --no-pager -o cat | grep -c '^Starting soos-daemon.service' || true)" -eq 0 ]] \
        || fail "the daemon was executed at boot although the condition failed"
    success "Part 1 (boot): the enabled unit was skipped at boot (multi-user.target) with ConditionResult=no, no execution, no restart."
}

# Part 3 — a start that always fails ends in start-limit-hit after 5 runs.
part3_start_limit_hit() {
    install -d -m 0755 "${MODELS_DIR}"
    install -m 0644 /workspace/models/manifest.toml "${MANIFEST}"
    local f
    for f in scrfd_500m_kps.onnx minifasnet_v2_80x80.onnx arcface_w600k_mbf.onnx; do
        printf 'corrupt model for the start-limit test\n' > "${MODELS_DIR}/${f}"
        chmod 0644 "${MODELS_DIR}/${f}"
    done
    systemctl reset-failed "${UNIT}" 2>/dev/null || true

    local cursor rc=0 t0 elapsed_ms deadline
    cursor="$(journal_cursor)"
    t0="$(now_ms)"
    systemctl start "${UNIT}" 2>/dev/null || rc=$?
    [[ "${rc}" -ne 0 ]] || fail "systemctl start of a daemon that exits before READY=1 returned 0"
    # Each failed run takes well under a second plus RestartSec=2; bounded wait.
    deadline=$(( t0 + 120000 ))
    while :; do
        if [[ "$(prop ActiveState)" = "failed" ]] \
                && journal_since "${cursor}" | has 'Start request repeated too quickly'; then
            break
        fi
        (( $(now_ms) < deadline )) || fail "${UNIT} did not hit its start limit within 120 s (ActiveState=$(prop ActiveState), NRestarts=$(prop NRestarts))"
        sleep 0.5
    done
    elapsed_ms=$(( $(now_ms) - t0 ))

    local starts result
    starts="$(count_starts_since "${cursor}")"
    result="$(prop Result)"
    [[ "${starts}" -eq "${SHIPPED_START_LIMIT_BURST}" ]] \
        || fail "the daemon ran ${starts} times, expected exactly StartLimitBurst=${SHIPPED_START_LIMIT_BURST}"
    expect_prop NRestarts "${SHIPPED_START_LIMIT_BURST}"
    expect_prop SubState "failed"
    (( elapsed_ms < SHIPPED_START_LIMIT_INTERVAL_S * 1000 )) \
        || fail "the start limit was reached after ${elapsed_ms} ms, not within StartLimitIntervalSec=${SHIPPED_START_LIMIT_INTERVAL_S}"
    journal_since "${cursor}" | has 'refusing to start daemon (fail-closed)' \
        || fail "the daemon did not report its fail-closed refusal to start"
    # systemd keeps the first failure result of the run ('exit-code') and logs the
    # start-limit-hit; it reports Result=start-limit-hit when no earlier failure is recorded.
    case "${result}" in
        start-limit-hit|exit-code) ;;
        *) fail "unexpected Result='${result}' after the start limit was hit" ;;
    esac

    # The limit holds: a manual start is refused and does not run the daemon.
    local cursor2 rc2=0
    cursor2="$(journal_cursor)"
    systemctl start "${UNIT}" 2>/dev/null || rc2=$?
    [[ "${rc2}" -ne 0 ]] || fail "a manual start after start-limit-hit was accepted"
    journal_since "${cursor2}" | has 'Start request repeated too quickly' \
        || fail "a manual start after start-limit-hit was not refused by the start limit"
    [[ "$(count_starts_since "${cursor2}")" -eq 0 ]] || fail "a manual start after start-limit-hit ran the daemon"
    expect_prop ActiveState "failed"
    success "Part 3: a daemon that always fails ran exactly ${starts} times (NRestarts=${SHIPPED_START_LIMIT_BURST}), then systemd logged 'Start request repeated too quickly' (start-limit-hit, Result=${result}) after ${elapsed_ms} ms < StartLimitIntervalSec=${SHIPPED_START_LIMIT_INTERVAL_S} s; a later start is refused without running it."

    systemctl reset-failed "${UNIT}"
    rm -f "${MODELS_DIR}"/*.onnx "${MANIFEST}"
}

# Deploys the attested models with scripts/download_models.sh; returns 1 when skipped.
deploy_models() {
    case "${MODELS}" in
        none)
            return 1
            ;;
        host)
            [[ -r "${IN_CONTAINER_HOST_MODELS}/manifest.toml" ]] || fail "host models are not mounted at ${IN_CONTAINER_HOST_MODELS}"
            install -d -m 0755 "${MODELS_DIR}"
            local f
            for f in "${IN_CONTAINER_HOST_MODELS}"/*.onnx; do
                install -m 0644 "${f}" "${MODELS_DIR}/$(basename "${f}")"
            done
            info "Host models copied; scripts/download_models.sh verifies them against models/manifest.toml."
            ;;
        download)
            info "Downloading the attested models with scripts/download_models.sh (HTTPS, size-bounded, SHA-256 verified)..."
            ;;
    esac
    bash /workspace/scripts/download_models.sh --target-dir "${MODELS_DIR}" \
        --manifest /workspace/models/manifest.toml
    [[ -f "${MANIFEST}" ]] || fail "download_models.sh did not deploy ${MANIFEST}"
}

# Part 2 — Type=notify readiness with real models and the mock camera.
part2_notify_readiness() {
    install -d -m 0755 /etc/soos
    printf '%s\n' '# systemd acceptance test: no camera in the container' \
        '[pipeline]' 'use_mock_camera = true' > /etc/soos/daemon.toml
    chmod 0644 /etc/soos/daemon.toml

    local cursor rc=0 t0 wall_ms
    cursor="$(journal_cursor)"
    t0="$(now_ms)"
    systemctl start "${UNIT}" || rc=$?
    wall_ms=$(( $(now_ms) - t0 ))
    [[ "${rc}" -eq 0 ]] || fail "systemctl start with deployed models exited ${rc}"

    # Checked immediately after systemctl start returned: with Type=notify this
    # is the moment systemd received READY=1.
    [[ -S "${SOCKET}" ]] || fail "${SOCKET} does not exist when systemctl start returned (READY=1 sent before bind?)"
    local sock_meta dir_meta
    sock_meta="$(stat -c '%a %U:%G' "${SOCKET}")"
    dir_meta="$(stat -c '%a %U:%G' /run/soos)"
    [[ "${sock_meta}" = "660 root:soos" ]] || fail "${SOCKET} is '${sock_meta}', expected '660 root:soos'"
    [[ "${dir_meta}" = "750 root:soos" ]] || fail "/run/soos is '${dir_meta}', expected '750 root:soos'"
    expect_prop ActiveState "active"
    expect_prop SubState "running"
    expect_prop NRestarts "0"
    [[ "$(prop MainPID)" -gt 0 ]] || fail "MainPID is 0 while the unit is active"

    local log
    log="$(journal_since "${cursor}")"
    grep -q 'Reported readiness to systemd' <<< "${log}" \
        || fail "the daemon did not report READY=1 to the systemd notify socket"
    local listening_line ready_line started_line
    listening_line="$(grep -n 'soos-daemon initialized and listening' <<< "${log}" | head -n 1 | cut -d: -f1)"
    ready_line="$(grep -n 'Reported readiness to systemd' <<< "${log}" | head -n 1 | cut -d: -f1)"
    started_line="$(grep -n '^Started soos-daemon.service' <<< "${log}" | head -n 1 | cut -d: -f1)"
    [[ -n "${listening_line}" && -n "${ready_line}" && -n "${started_line}" ]] \
        || fail "missing listening / readiness / Started lines in the journal"
    (( listening_line < ready_line && ready_line < started_line )) \
        || fail "journal order is not socket bound -> READY=1 -> Started (lines ${listening_line}, ${ready_line}, ${started_line})"

    local exec_us active_us ready_ms
    exec_us="$(prop ExecMainStartTimestampMonotonic)"
    active_us="$(prop ActiveEnterTimestampMonotonic)"
    (( exec_us > 0 && active_us > exec_us )) || fail "invalid start timestamps (exec ${exec_us}, active ${active_us})"
    ready_ms=$(( (active_us - exec_us) / 1000 ))
    (( ready_ms < MAX_READY_MS )) \
        || fail "start-to-ready took ${ready_ms} ms, not well under TimeoutStartSec=60 (limit ${MAX_READY_MS} ms)"
    success "Part 2: Type=notify — systemctl start returned after READY=1 (${wall_ms} ms wall); ${SOCKET} was already bound (${sock_meta}, /run/soos ${dir_meta}); journal order bind -> READY=1 -> Started; ActiveState=active."
    success "Part 2: start-to-ready ${ready_ms} ms (ExecMainStartTimestamp -> ActiveEnterTimestamp) on $(nproc) CPU(s) of '$(sed -n 's/^model name[[:space:]]*: //p' /proc/cpuinfo | head -n 1)', TimeoutStartSec=60 s."

    local status_json
    status_json="$(bash /workspace/scripts/wait_daemon_ready.sh --socket "${SOCKET}" \
        --manifest "${MANIFEST}" --admin /usr/bin/soos-admin --timeout 10)" \
        || fail "scripts/wait_daemon_ready.sh did not report the daemon ready"
    printf '%s\n' "${status_json}"
    local key
    for key in '"is_healthy": true' '"socket_ready": true' '"models_verified": true' \
            '"camera_ready": true' '"systemd_active_state": "active"'; do
        grep -qF "${key}" <<< "${status_json}" || fail "soos-admin status JSON lacks ${key}"
    done
    success "Part 2: soos-admin --format json status reports a healthy daemon (is_healthy, socket_ready, models_verified, camera_ready, systemd active)."
}

# Part 5 — clean stop.
part5_clean_stop() {
    local cursor rc=0 t0 stop_ms
    cursor="$(journal_cursor)"
    t0="$(now_ms)"
    systemctl stop "${UNIT}" || rc=$?
    stop_ms=$(( $(now_ms) - t0 ))
    [[ "${rc}" -eq 0 ]] || fail "systemctl stop exited ${rc}"
    expect_prop ActiveState "inactive"
    expect_prop Result "success"
    expect_prop ExecMainCode "1"
    expect_prop ExecMainStatus "0"
    [[ ! -e "${SOCKET}" ]] || fail "${SOCKET} still exists after the stop"
    local log
    log="$(journal_since "${cursor}")"
    grep -q 'Received SIGTERM signal; shutting down gracefully' <<< "${log}" || fail "the daemon did not log the SIGTERM shutdown"
    grep -q 'soos-daemon terminated cleanly' <<< "${log}" || fail "the daemon did not finish its graceful drain"
    if grep -Eq 'timed out|Killing process|SIGKILL' <<< "${log}"; then
        fail "the stop needed a timeout or SIGKILL"
    fi
    (( stop_ms < MAX_STOP_MS )) || fail "the stop took ${stop_ms} ms (limit ${MAX_STOP_MS} ms, TimeoutStopSec=$(prop TimeoutStopUSec))"
    success "Part 5: systemctl stop took ${stop_ms} ms (TimeoutStopSec=$(prop TimeoutStopUSec)): SIGTERM, graceful drain, exit status 0, Result=success, socket removed, no SIGKILL."
}

case "${STAGE}" in
    install)
        wait_for_boot
        info "Live install with scripts/install.sh (fresh release artifacts, no models yet)..."
        bash /workspace/scripts/install.sh --artifact-dir /target/release --allow-missing \
            --skip-models --distro none
        part4_verify_and_shipped_values
        part1_condition_failed_without_models
        ;;
    after-reboot)
        wait_for_boot
        part1_condition_failed_at_boot
        part3_start_limit_hit
        if deploy_models; then
            part2_notify_readiness
            part5_clean_stop
        else
            warn "SKIPPED: parts 2 (Type=notify readiness) and 5 (clean stop) need the real models (--models host|download)."
        fi
        ;;
    *)
        fail "Unknown stage '${STAGE}' (expected install or after-reboot)"
        ;;
esac
