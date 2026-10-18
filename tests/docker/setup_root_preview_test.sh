#!/usr/bin/env bash
# =============================================================================
# tests/docker/setup_root_preview_test.sh — scripts/setup.sh camera view, daemon
# side, as REAL root under a real systemd (GitHub #400, matrix SET34)
# =============================================================================
# soos-daemon reads only /etc/soos/daemon.toml, so setup.sh creates the [preview]
# section there and nowhere else. The invariant harness
# (tests/invariants/src/setup_user_phase_contract.rs) runs setup.sh without root
# against a fake --sysconfdir, which can never reach that path. This harness
# runs it for real: it boots the pinned ubuntu:24.04 systemd image
# (tests/docker/Dockerfile.systemd, the image of the systemd-unit job) and runs
# the UNMODIFIED scripts/setup.sh as an unprivileged user, which escalates with
# the real sudo (environment reset) into its own --system-phase as root, with the
# default --sysconfdir /etc. The root half writes the real /etc/soos/daemon.toml
# and drives the shipped packaging/soos-daemon.service with the real systemctl,
# and its readiness gate is the real scripts/wait_daemon_ready.sh.
#
# Test doubles (container only; setup.sh has no test hook and none is added):
#   - /usr/libexec/soos/soos-daemon is a small Python stand-in run by the real
#     unit (Type=notify, full sandbox): it binds /run/soos/daemon.sock, sends
#     READY=1, answers "healthy" and records every start. A mode file read at
#     each start makes it refuse to start or report unhealthy, either only
#     while /etc/soos/daemon.toml exists, or always. No face pipeline runs.
#     Run by setup.sh as `soos-daemon --check-config --config <file>`, it only
#     records the call (never starts, never binds) and refuses in the mode
#     check-refuses.
#   - /usr/bin/soos-admin answers the status query of wait_daemon_ready.sh
#     over that socket and the camera report of the verify phase.
#   - setup.sh is copied byte for byte into a fake checkout whose install.sh
#     and install_remote.sh are stubs (the face install and the companion
#     build are not under test); wait_daemon_ready.sh and check_build_deps.sh
#     are the real ones. The build user's cargo is a stub, and `systemctl
#     --user` is emulated in the user half's PATH only (no user manager in the
#     container); the root half gets sudo's secure_path and the real systemctl.
#
# Cases (each one starts from a stopped, reset unit and a healthy daemon):
#   A1 /etc/soos and daemon.toml absent: daemon.toml created with exactly the
#      [preview] content, 0644 root:root, one link, no temporary file left,
#      /etc/soos created 0755; the daemon restarted (new invocation, started
#      with the file present); "[DONE] camera view (daemon)"; companion ran;
#      exit 0.
#   A2 Second run: "already done", file (inode, bytes) unchanged, no restart.
#   B  An administrator daemon.toml (0640) exists: never edited (bytes, mode,
#      owner, inode, mtime), the exact change printed, no restart; exit 0.
#   C1 The daemon starts but is not healthy with the file (readiness gate
#      fails), and recovers without it: file removed, /etc/soos created by the
#      run removed again, daemon healthy again, exit 1, companion never runs.
#   C2 The restart itself fails with the file, and recovers without it: same
#      as C1 (the /etc/soos directory existed before and is kept).
#   D  The daemon stays down: file removed, exit 70, companion never runs.
#   E1 /etc/soos/daemon.toml is a dangling symbolic link: refused and printed,
#      link kept, its target never created, no restart; exit 0.
#   E2 A symbolic link to an existing file: refused, the file never edited.
#   F1 soos-daemon --check-config refuses the created file (GitHub #441): the
#      file and the /etc/soos created by the run removed, no restart (same
#      invocation, never started with the file), daemon healthy, exit 1,
#      companion never runs.
#   F2 The created file is checked exactly once by the installed daemon, before
#      the restart on it; exit 0.
#
# Host isolation: the container runs --privileged (systemd and the unit sandbox
# need namespaces) with a private cgroup namespace and tmpfs /run, like the
# systemd-unit job; the image masks the boot units that would write host kernel
# or firmware state (asserted below) and the host kernel.*, vm.* and fs.*
# sysctls are compared before and after the run. --privileged still grants
# CAP_MKNOD: the image must never run untrusted code.
#
# Usage:
#   bash tests/docker/setup_root_preview_test.sh [--case <ID>]...  # host: runs Docker
#   bash tests/docker/setup_root_preview_test.sh --in-container [<ID>...]
#   bash tests/docker/setup_root_preview_test.sh --help
# Without --case every case runs (CI). Environment: SOOS_DOCKER (docker binary).
# =============================================================================

set -euo pipefail

readonly UNIT="soos-daemon.service"
readonly DAEMON_TOML="/etc/soos/daemon.toml"
readonly RUNTIME_IMAGE="soos-setup-root-runtime"
readonly TEST_USER="soostest"
readonly TEST_DIR="/var/lib/soos-setup-test"
readonly MODE_FILE="${TEST_DIR}/daemon-mode"
readonly START_LOG="/var/lib/soos/setup-test-daemon-starts.log"
readonly CHECKOUT="/opt/soos-setup-test/checkout"
readonly USER_BIN="/opt/soos-setup-test/user-bin"
readonly COMPANION_LOG="${TEST_DIR}/user/companion.log"
readonly LINK_TARGET="${TEST_DIR}/link-target.toml"
readonly LOGIN="owner.login@example.com"
readonly RP_ID="pc.tail1234.ts.net"
readonly CHECK_LOG="${TEST_DIR}/check-config.log"
readonly ALL_CASES=(A1 A2 B C1 C2 D E1 E2 F1 F2)

info() { echo "[INFO]  $*"; }
ok()   { echo "[OK]    $*"; }
fail() { echo "[FAIL]  $*" >&2; exit 1; }

usage() {
    sed -n '2,74p' "$0"
}

DOCKER=""
CONTAINER=""

# =============================================================================
# Host mode: build the image, boot systemd, run the cases, compare host sysctls
# =============================================================================
host_main() {
    local -a cases=()
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --case)
                [[ $# -ge 2 ]] || fail "--case requires a case id (${ALL_CASES[*]})"
                cases+=("$2")
                shift 2
                ;;
            -h|--help) usage; exit 0 ;;
            *) fail "unknown option: $1 (use --help)" ;;
        esac
    done
    local repo
    DOCKER="${SOOS_DOCKER:-docker}"
    command -v "${DOCKER}" >/dev/null 2>&1 || fail "docker is required"
    "${DOCKER}" info >/dev/null 2>&1 || fail "the Docker daemon is not accessible"
    repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
    CONTAINER="soos-setup-root-$$"

    # shellcheck disable=SC2329 # invoked by the EXIT trap below
    cleanup() {
        local rc=$?
        if [[ "${rc}" -ne 0 ]] && "${DOCKER}" inspect "${CONTAINER}" >/dev/null 2>&1; then
            echo "[FAIL]  last journal entries of ${UNIT} in the container:" >&2
            "${DOCKER}" exec "${CONTAINER}" journalctl -u "${UNIT}" --no-pager -n 40 >&2 2>/dev/null || true
        fi
        "${DOCKER}" rm -f "${CONTAINER}" >/dev/null 2>&1 || true
        exit "${rc}"
    }
    trap cleanup EXIT

    info "Building the systemd runtime image (tests/docker/Dockerfile.systemd, pinned digest)..."
    "${DOCKER}" build -q -f "${repo}/tests/docker/Dockerfile.systemd" -t "${RUNTIME_IMAGE}" \
        "${repo}" >/dev/null

    # Same host isolation guard as tests/docker/systemd_unit_acceptance_test.sh.
    host_sysctl_snapshot() {
        { sysctl -a 2>/dev/null || true; } | grep -E '^(kernel|vm|fs)\.' \
            | { grep -vE '^(kernel\.(random\.|ns_last_pid|pty\.nr|sched_domain|perf_event_max_sample_rate|tainted)|fs\.(dentry-state|inode-nr|inode-state|file-nr|aio-nr|quota\.)|vm\.stat_refresh)' || true; } \
            | sort
    }
    local before after changed
    before="$(host_sysctl_snapshot)"
    [[ -n "${before}" ]] || fail "could not snapshot the host sysctls (sysctl -a)"

    info "Booting systemd as PID 1 in ${RUNTIME_IMAGE} (container ${CONTAINER})..."
    "${DOCKER}" run --detach --name "${CONTAINER}" \
        --privileged --cgroupns=private \
        --tmpfs /run --tmpfs /run/lock \
        -v "${repo}:/workspace:ro" \
        "${RUNTIME_IMAGE}" >/dev/null
    "${DOCKER}" exec "${CONTAINER}" \
        bash /workspace/tests/docker/setup_root_preview_test.sh --in-container \
        ${cases[@]+"${cases[@]}"}

    after="$(host_sysctl_snapshot)"
    changed="$(diff <(echo "${before}") <(echo "${after}") || true)"
    if [[ -n "${changed}" ]]; then
        echo "${changed}" >&2
        fail "host isolation broken: kernel sysctls differ after the privileged container ran"
    fi
    ok "Host kernel.*, vm.* and fs.* sysctls are unchanged after the privileged container ran."
    echo "setup.sh root camera view test: all checks passed"
}

# =============================================================================
# Container mode — provisioning
# =============================================================================
now_ms() { echo $(( $(date +%s%N) / 1000000 )); }

wait_for_boot() {
    local state="" deadline
    deadline=$(( $(now_ms) + 90000 ))
    while (( $(now_ms) < deadline )); do
        state="$(systemctl is-system-running 2>/dev/null || true)"
        case "${state}" in
            running|degraded) info "systemd boot finished (${state})."; return 0 ;;
        esac
        sleep 0.5
    done
    fail "systemd did not finish booting within 90 s (state: '${state:-unknown}')"
}

assert_host_state_units_masked() {
    local unit state
    for unit in systemd-sysctl.service systemd-modules-load.service systemd-binfmt.service \
            proc-sys-fs-binfmt_misc.automount proc-sys-fs-binfmt_misc.mount \
            systemd-rfkill.service systemd-rfkill.socket systemd-random-seed.service \
            systemd-tpm2-setup-early.service systemd-tpm2-setup.service; do
        state="$(systemctl is-enabled "${unit}" 2>/dev/null || true)"
        [[ "${state}" == masked || "${state}" == masked-runtime ]] \
            || fail "${unit} must be masked in the privileged container (is-enabled: '${state:-unknown}')"
    done
    ok "Host-state boot units are masked."
}

# Stand-in for soos-daemon, run by the shipped unit (Type=notify, full sandbox).
write_stub_daemon() {
    install -d -m 0755 /usr/libexec/soos
    cat > /usr/libexec/soos/soos-daemon <<'PY'
#!/usr/bin/python3
"""Test-only stand-in for soos-daemon (tests/docker/setup_root_preview_test.sh).

Records every start, then follows the mode of MODE_FILE: "ok" serves "healthy";
"refuse-with-config" / "unhealthy-with-config" refuse to start / answer
"unhealthy" while /etc/soos/daemon.toml exists; "down" never starts.
"""
import os
import signal
import socket
import sys

MODE_FILE = "/var/lib/soos-setup-test/daemon-mode"
START_LOG = "/var/lib/soos/setup-test-daemon-starts.log"
CONFIG = "/etc/soos/daemon.toml"
SOCKET_PATH = "/run/soos/daemon.sock"
CHECK_LOG = "/var/lib/soos-setup-test/check-config.log"


def check_config():
    """`--check-config [--config FILE]` (GitHub #441): records the call; never starts or binds."""
    path = CONFIG
    args = sys.argv[1:]
    if "--config" in args[:-1]:
        path = args[args.index("--config") + 1]
    try:
        with open(MODE_FILE, encoding="ascii") as handle:
            mode = handle.read().strip()
    except OSError:
        mode = "ok"
    try:
        with open(START_LOG, encoding="ascii") as log:
            present_starts = sum(1 for line in log if "daemon_toml=present" in line)
    except OSError:
        present_starts = 0
    with open(CHECK_LOG, "a", encoding="ascii") as log:
        log.write("check config=%s file=%s present_starts=%d\n" % (
            path, "present" if os.path.isfile(path) else "absent", present_starts))
    if mode == "check-refuses":
        print("soos-daemon: configuration refused: stub (mode check-refuses)", file=sys.stderr)
        return 78
    print("soos-daemon: configuration '%s' is valid" % path)
    return 0


def notify(message):
    address = os.environ.get("NOTIFY_SOCKET", "")
    if not address:
        return
    if address.startswith("@"):
        address = "\0" + address[1:]
    with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as sock:
        sock.connect(address)
        sock.sendall(message)


def main():
    if "--check-config" in sys.argv:
        return check_config()
    try:
        with open(MODE_FILE, encoding="ascii") as handle:
            mode = handle.read().strip()
    except OSError:
        mode = "ok"
    present = os.path.lexists(CONFIG)
    with open(START_LOG, "a", encoding="ascii") as log:
        log.write("start mode=%s daemon_toml=%s\n" % (mode, "present" if present else "absent"))
    if mode == "down" or (mode == "refuse-with-config" and present):
        print("stub soos-daemon: refusing to start (mode %s)" % mode, file=sys.stderr)
        return 1
    answer = b"unhealthy\n" if (mode == "unhealthy-with-config" and present) else b"healthy\n"
    if os.path.lexists(SOCKET_PATH):
        os.unlink(SOCKET_PATH)
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    server.bind(SOCKET_PATH)
    os.chmod(SOCKET_PATH, 0o660)
    server.listen(8)

    def stop(_signum, _frame):
        try:
            os.unlink(SOCKET_PATH)
        except OSError:
            pass
        sys.exit(0)

    signal.signal(signal.SIGTERM, stop)
    notify(b"READY=1")
    while True:
        conn, _ = server.accept()
        with conn:
            conn.sendall(answer)


if __name__ == "__main__":
    sys.exit(main())
PY
    chmod 0755 /usr/libexec/soos/soos-daemon
}

# Stand-in for soos-admin: the status query of wait_daemon_ready.sh and the camera report.
write_stub_admin() {
    cat > /usr/bin/soos-admin <<'SH'
#!/usr/bin/env bash
# Test-only stand-in for soos-admin (tests/docker/setup_root_preview_test.sh).
set -euo pipefail
sock=""
args=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --socket-path) sock="$2"; shift 2 ;;
        --format|--config) shift 2 ;;
        *) args+=("$1"); shift ;;
    esac
done
case "${args[*]}" in
    status)
        python3 - "${sock}" <<'PY'
import socket
import sys
try:
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
        sock.settimeout(2)
        sock.connect(sys.argv[1])
        answer = sock.recv(64)
except OSError as err:
    print("stub soos-admin: %s" % err, file=sys.stderr)
    sys.exit(1)
if answer != b"healthy\n":
    print('{"is_healthy": false}')
    print("stub soos-admin: daemon unhealthy", file=sys.stderr)
    sys.exit(1)
print('{"is_healthy": true}')
PY
        ;;
    "camera list") echo "No camera selected (setup root preview test stand-in)." ;;
    *) echo "stub soos-admin: unexpected command: ${args[*]}" >&2; exit 64 ;;
esac
SH
    chmod 0755 /usr/bin/soos-admin
}

write_fake_checkout() {
    install -d -m 0755 "${CHECKOUT}/scripts" "${USER_BIN}"
    local name
    for name in setup.sh wait_daemon_ready.sh check_build_deps.sh; do
        install -m 0755 "/workspace/scripts/${name}" "${CHECKOUT}/scripts/${name}"
    done
    # The face install is not under test: the unit and the stand-ins are already in place.
    cat > "${CHECKOUT}/scripts/install.sh" <<SH
#!/usr/bin/env bash
printf 'install.sh stub ran as uid %s\n' "\$(id -u)" >> "${TEST_DIR}/install.log"
SH
    # Same contract as the real install_remote.sh: never root, installs two binaries
    # and two user units, never enables a unit.
    cat > "${CHECKOUT}/scripts/install_remote.sh" <<SH
#!/usr/bin/env bash
set -euo pipefail
[[ "\$(id -u)" != 0 ]] || exit 1
printf 'install_remote.sh ran as uid %s\n' "\$(id -u)" >> "${COMPANION_LOG}"
conf="\${XDG_CONFIG_HOME:-\${HOME}/.config}"
install -d -m 0755 "\${HOME}/.local/bin" "\${conf}/systemd/user"
for bin in soos-remote soos-push-sender; do
    printf '#!/bin/sh\nexit 0\n' > "\${HOME}/.local/bin/\${bin}"
    chmod 0755 "\${HOME}/.local/bin/\${bin}"
    printf '[Service]\nExecStart=%s\n' "\${HOME}/.local/bin/\${bin}" > "\${conf}/systemd/user/\${bin}.service"
done
SH
    chmod 0755 "${CHECKOUT}/scripts/install.sh" "${CHECKOUT}/scripts/install_remote.sh"
    # `systemctl --user` emulation for the user half only (no user manager here).
    cat > "${USER_BIN}/systemctl" <<SH
#!/usr/bin/env bash
[[ "\${1:-}" == "--user" ]] || exec /usr/bin/systemctl "\$@"
shift
state="${TEST_DIR}/user/units"
mkdir -p "\${state}"
verb="\${1:-}"
shift || true
now=0
units=()
for a in "\$@"; do
    case "\${a}" in
        --now) now=1 ;;
        --quiet|-q) ;;
        *) units+=("\${a}") ;;
    esac
done
rc=0
for u in "\${units[@]}"; do
    case "\${verb}" in
        is-enabled) [[ -e "\${state}/\${u}.enabled" ]] || rc=1 ;;
        is-active) [[ -e "\${state}/\${u}.active" ]] || rc=3 ;;
        enable) touch "\${state}/\${u}.enabled"; (( now )) && touch "\${state}/\${u}.active" ;;
        disable) rm -f "\${state}/\${u}.enabled"; (( now )) && rm -f "\${state}/\${u}.active" ;;
        start|restart) touch "\${state}/\${u}.active" ;;
        stop) rm -f "\${state}/\${u}.active" ;;
        try-restart) ;;
        *) rc=1 ;;
    esac
done
exit "\${rc}"
SH
    chmod 0755 "${USER_BIN}/systemctl"
    cmp -s /workspace/scripts/setup.sh "${CHECKOUT}/scripts/setup.sh" \
        || fail "the setup.sh under test differs from the checkout"
}

provision() {
    info "Installing python3-minimal and sudo..."
    apt-get update -qq >/dev/null
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
        python3-minimal sudo >/dev/null 2>&1 || fail "package installation failed"

    groupadd --system soos
    useradd -m -s /bin/bash "${TEST_USER}"
    usermod -aG soos "${TEST_USER}"
    TEST_UID="$(id -u "${TEST_USER}")"
    printf '%s ALL=(root) NOPASSWD: ALL\n' "${TEST_USER}" > /etc/sudoers.d/soos-setup-test
    chmod 0440 /etc/sudoers.d/soos-setup-test
    visudo -cq || fail "sudoers drop-in rejected"

    install -d -m 0755 "${TEST_DIR}"
    install -d -m 0755 -o "${TEST_USER}" -g "${TEST_USER}" "${TEST_DIR}/user"
    install -d -m 0755 /var/lib/soos /var/lib/soos/models
    install -m 0644 /workspace/models/manifest.toml /var/lib/soos/models/manifest.toml
    install -m 0644 /workspace/packaging/soos-daemon.service "/etc/systemd/system/${UNIT}"
    write_stub_daemon
    write_stub_admin
    write_fake_checkout
    # The build user's cargo: setup's build step only needs the release directory.
    install -d -m 0755 -o "${TEST_USER}" -g "${TEST_USER}" "/home/${TEST_USER}/.cargo" "/home/${TEST_USER}/.cargo/bin"
    cat > "/home/${TEST_USER}/.cargo/bin/cargo" <<'SH'
#!/usr/bin/env bash
mkdir -p "${CARGO_TARGET_DIR:?}/release"
SH
    chown "${TEST_USER}:${TEST_USER}" "/home/${TEST_USER}/.cargo/bin/cargo"
    chmod 0755 "/home/${TEST_USER}/.cargo/bin/cargo"
    install -d -m 0700 -o "${TEST_USER}" -g "${TEST_USER}" "/run/user/${TEST_UID}"
    systemctl daemon-reload
    systemctl enable --quiet "${UNIT}"
    ok "Provisioned: ${TEST_USER} (uid ${TEST_UID}) in group soos, shipped ${UNIT} with the stand-in daemon."
}

# =============================================================================
# Container mode — helpers
# =============================================================================
TEST_UID=""
OUT=""
RC=0

invocation() { systemctl show -p InvocationID --value "${UNIT}"; }
last_start() { tail -n 1 "${START_LOG}"; }

daemon_ready() {
    bash /workspace/scripts/wait_daemon_ready.sh --socket /run/soos/daemon.sock \
        --manifest /var/lib/soos/models/manifest.toml --admin /usr/bin/soos-admin --timeout 30 >/dev/null
}

# The [preview] content the root half must write, spelled out independently of setup.sh.
expected_daemon_toml() {
    printf '%s\n' \
        "# Written by scripts/setup.sh (soos-setup): live camera view of the phone companion." \
        "# Docs/DAEMON.md section 1.5; edit or remove it as an administrator." \
        "[preview]" \
        "enabled = true" \
        "allowed_uids = [${TEST_UID}]" \
        "remote_view = true"
}

# Fresh case: unit stopped and reset, no daemon.toml, no companion state; /etc/soos as
# asked; a healthy daemon started, then the case's daemon mode set.
reset_case() {
    local etc_soos="$1" mode="$2"
    systemctl stop "${UNIT}" >/dev/null 2>&1 || true
    systemctl reset-failed "${UNIT}" >/dev/null 2>&1 || true
    : > "${START_LOG}"
    rm -rf /etc/soos "${LINK_TARGET}" "/home/${TEST_USER}/.config" "/home/${TEST_USER}/.local" \
        "/home/${TEST_USER}/target" "${TEST_DIR}/user/units" "${COMPANION_LOG}"
    if [[ "${etc_soos}" == present ]]; then
        install -d -m 0755 /etc/soos
    fi
    echo ok > "${MODE_FILE}"
    systemctl start "${UNIT}" || fail "the stand-in daemon did not start"
    daemon_ready || fail "the stand-in daemon is not ready before the case"
    echo "${mode}" > "${MODE_FILE}"
}

# Runs the unmodified setup.sh as the test user (user half; real sudo for the root half).
run_setup() {
    RC=0
    OUT="$(runuser -u "${TEST_USER}" -- env -i \
        HOME="/home/${TEST_USER}" USER="${TEST_USER}" LOGNAME="${TEST_USER}" LC_ALL=C \
        PATH="${USER_BIN}:/usr/local/bin:/usr/bin:/bin" \
        XDG_RUNTIME_DIR="/run/user/${TEST_UID}" CARGO_TARGET_DIR="/home/${TEST_USER}/target" \
        bash "${CHECKOUT}/scripts/setup.sh" --yes --skip-packages --no-enroll \
        --tailscale-login "${LOGIN}" --rp-id "${RP_ID}" --camera-view on \
        < /dev/null 2>&1)" || RC=$?
}

case_fail() {
    printf '%s\n' "----- setup.sh output (exit ${RC}) -----" "${OUT}" "-----" >&2
    fail "$1"
}

expect_rc() {
    [[ "${RC}" -eq "$1" ]] || case_fail "$2: expected exit $1, got ${RC}"
}

expect_out() {
    grep -qF -- "$1" <<< "${OUT}" || case_fail "$2: output lacks: $1"
}

expect_no_out() {
    if grep -qF -- "$1" <<< "${OUT}"; then
        case_fail "$2: output must not contain: $1"
    fi
}

no_temp_files() {
    local left=""
    if [[ -d /etc/soos ]]; then
        left="$(find /etc/soos -maxdepth 1 -name '.daemon.toml.*' -print -quit)"
    fi
    [[ -z "${left}" ]] || case_fail "$1: temporary file left behind: ${left}"
}

companion_ran() {
    [[ -s "${COMPANION_LOG}" && -f "/home/${TEST_USER}/.config/soos/remote.toml" ]] \
        || case_fail "$1: the phone companion did not run"
    expect_out "[DONE] companion: soos-remote.service soos-push-sender.service running" "$1"
}

companion_not_run() {
    [[ ! -e "${COMPANION_LOG}" && ! -e "/home/${TEST_USER}/.config/soos/remote.toml" ]] \
        || case_fail "$1: the phone companion ran after a failed system phase"
    expect_out "[SKIP] companion: the system phase did not complete (exit ${RC})" "$1"
}

file_identity() { stat -c '%i %a %U:%G %h %Y' -- "$1"; }

# =============================================================================
# Container mode — cases
# =============================================================================
case_a1() {
    local id="A1" before
    reset_case absent ok
    before="$(invocation)"
    run_setup
    expect_rc 0 "${id}"
    expect_out "[DONE] camera view (daemon): ${DAEMON_TOML} created (preview for uid ${TEST_UID}, remote view on); ${UNIT} restarted" "${id}"
    [[ -f "${DAEMON_TOML}" && ! -L "${DAEMON_TOML}" ]] || case_fail "${id}: ${DAEMON_TOML} is not a regular file"
    cmp -s "${DAEMON_TOML}" <(expected_daemon_toml) \
        || case_fail "${id}: ${DAEMON_TOML} content differs: $(cat "${DAEMON_TOML}")"
    [[ "$(stat -c '%a %U:%G %h' "${DAEMON_TOML}")" == "644 root:root 1" ]] \
        || case_fail "${id}: ${DAEMON_TOML} must be 0644 root:root with one link: $(stat -c '%a %U:%G %h' "${DAEMON_TOML}")"
    [[ "$(stat -c '%a %U:%G' /etc/soos)" == "755 root:root" ]] \
        || case_fail "${id}: /etc/soos must be created 0755 root:root: $(stat -c '%a %U:%G' /etc/soos)"
    no_temp_files "${id}"
    [[ "$(invocation)" != "${before}" ]] || case_fail "${id}: ${UNIT} was not restarted"
    [[ "$(last_start)" == "start mode=ok daemon_toml=present" ]] \
        || case_fail "${id}: the daemon was not restarted on the new file (last start: $(last_start))"
    systemctl is-active --quiet "${UNIT}" || case_fail "${id}: ${UNIT} is not active"
    companion_ran "${id}"
    ok "A1 absent: created with exactly the [preview] section (0644 root:root, hard link, no temporary file), daemon restarted on it, step done, companion ran, exit 0"
}

case_a2() {
    local id="A2" before identity
    # Continues from A1 (no reset): the second run changes nothing.
    [[ -f "${DAEMON_TOML}" ]] || fail "${id} needs the file of A1 (run A1 first)"
    echo ok > "${MODE_FILE}"
    before="$(invocation)"
    identity="$(file_identity "${DAEMON_TOML}")"
    run_setup
    expect_rc 0 "${id}"
    expect_out "[DONE] camera view (daemon): already done (${DAEMON_TOML} written by setup)" "${id}"
    [[ "$(file_identity "${DAEMON_TOML}")" == "${identity}" ]] || case_fail "${id}: ${DAEMON_TOML} changed"
    cmp -s "${DAEMON_TOML}" <(expected_daemon_toml) || case_fail "${id}: content changed"
    [[ "$(invocation)" == "${before}" ]] || case_fail "${id}: ${UNIT} was restarted"
    no_temp_files "${id}"
    ok "A2 second run: already done, file and daemon untouched, exit 0"
}

case_b() {
    local id="B" before identity mine
    reset_case present ok
    mine=$'# administrator file\n[pipeline]\ncamera_device = "/dev/video0"\n'
    printf '%s' "${mine}" > "${DAEMON_TOML}"
    chmod 0640 "${DAEMON_TOML}"
    touch -d '2026-01-02 03:04:05' "${DAEMON_TOML}"
    before="$(invocation)"
    identity="$(file_identity "${DAEMON_TOML}")"
    run_setup
    expect_rc 0 "${id}"
    expect_out "[SKIP] camera view (daemon): ${DAEMON_TOML} exists and setup never edits it: as an administrator add to ${DAEMON_TOML}: [preview] enabled = true, allowed_uids = [${TEST_UID}], remote_view = true; then: sudo systemctl restart soos-daemon" "${id}"
    expect_no_out "[DONE] camera view (daemon)" "${id}"
    [[ "$(file_identity "${DAEMON_TOML}")" == "${identity}" ]] \
        || case_fail "${id}: ${DAEMON_TOML} inode, mode, owner, links or mtime changed"
    cmp -s "${DAEMON_TOML}" <(printf '%s' "${mine}") || case_fail "${id}: ${DAEMON_TOML} was edited"
    [[ "$(invocation)" == "${before}" ]] || case_fail "${id}: ${UNIT} was restarted"
    no_temp_files "${id}"
    companion_ran "${id}"
    ok "B existing: never edited (bytes, inode, mode, owner, mtime), change printed, no restart, exit 0"
}

# C1/C2: the daemon is not ready with the file and recovers without it.
case_c() {
    local id="$1" mode="$2" etc_soos="$3"
    reset_case "${etc_soos}" "${mode}"
    run_setup
    expect_rc 1 "${id}"
    expect_out "[FAIL] camera view (daemon): ${UNIT} did not become ready with ${DAEMON_TOML} (removed again)" "${id}"
    expect_out "Setup FAILED in phase camera view (daemon)" "${id}"
    [[ ! -e "${DAEMON_TOML}" && ! -L "${DAEMON_TOML}" ]] || case_fail "${id}: ${DAEMON_TOML} was not removed"
    no_temp_files "${id}"
    if [[ "${etc_soos}" == absent ]]; then
        [[ ! -e /etc/soos ]] || case_fail "${id}: /etc/soos created by the run was not removed"
    else
        [[ -d /etc/soos ]] || case_fail "${id}: the existing /etc/soos was removed"
    fi
    grep -qx "start mode=${mode} daemon_toml=present" "${START_LOG}" \
        || case_fail "${id}: the daemon was never restarted with the file"
    [[ "$(last_start)" == "start mode=${mode} daemon_toml=absent" ]] \
        || case_fail "${id}: the daemon was not restarted without the file (last start: $(last_start))"
    systemctl is-active --quiet "${UNIT}" || case_fail "${id}: ${UNIT} did not recover"
    daemon_ready || case_fail "${id}: ${UNIT} is not healthy after the rollback"
    companion_not_run "${id}"
}

case_c1() {
    case_c C1 unhealthy-with-config absent
    ok "C1 readiness gate failed with the file: removed, /etc/soos removed again, daemon healthy without it, exit 1, companion never ran"
}

case_c2() {
    case_c C2 refuse-with-config present
    ok "C2 restart failed with the file: removed, existing /etc/soos kept, daemon healthy without it, exit 1, companion never ran"
}

case_d() {
    local id="D"
    reset_case present down
    run_setup
    expect_rc 70 "${id}"
    expect_out "[FAIL] camera view (daemon): ${UNIT} did not become ready with ${DAEMON_TOML} (removed again)" "${id}"
    expect_out "[FAIL] camera view (daemon): ${UNIT} is not ready even without ${DAEMON_TOML} (exit 70; nothing else rolled back)" "${id}"
    [[ ! -e "${DAEMON_TOML}" && ! -L "${DAEMON_TOML}" ]] || case_fail "${id}: ${DAEMON_TOML} was not removed"
    no_temp_files "${id}"
    if systemctl is-active --quiet "${UNIT}"; then
        case_fail "${id}: ${UNIT} is unexpectedly active"
    fi
    companion_not_run "${id}"
    ok "D daemon stays down: file removed, exit 70, companion never ran"
}

case_e() {
    local id="$1" existing="$2" before target_identity=""
    reset_case present ok
    if [[ "${existing}" == true ]]; then
        printf '[pipeline]\n' > "${LINK_TARGET}"
        chmod 0644 "${LINK_TARGET}"
        target_identity="$(file_identity "${LINK_TARGET}")"
    fi
    ln -s "${LINK_TARGET}" "${DAEMON_TOML}"
    before="$(invocation)"
    run_setup
    expect_rc 0 "${id}"
    expect_out "[SKIP] camera view (daemon): ${DAEMON_TOML} is a symbolic link; setup never follows it" "${id}"
    expect_no_out "[DONE] camera view (daemon)" "${id}"
    [[ -L "${DAEMON_TOML}" && "$(readlink "${DAEMON_TOML}")" == "${LINK_TARGET}" ]] \
        || case_fail "${id}: the symbolic link was replaced or removed"
    if [[ "${existing}" == true ]]; then
        if [[ "$(file_identity "${LINK_TARGET}")" != "${target_identity}" ]] \
                || ! cmp -s "${LINK_TARGET}" <(printf '[pipeline]\n'); then
            case_fail "${id}: the link target was edited"
        fi
    else
        [[ ! -e "${LINK_TARGET}" ]] || case_fail "${id}: the dangling link target was created"
    fi
    [[ "$(invocation)" == "${before}" ]] || case_fail "${id}: ${UNIT} was restarted"
    no_temp_files "${id}"
    companion_ran "${id}"
}

case_e1() {
    case_e E1 false
    ok "E1 dangling symbolic link: refused and printed, link kept, target never created, no restart, exit 0"
}

case_e2() {
    case_e E2 true
    ok "E2 symbolic link to a file: refused and printed, target never edited, no restart, exit 0"
}

# F1: soos-daemon --check-config refuses the created file: removed again, never restarted on.
case_f1() {
    local id="F1" before
    reset_case absent check-refuses
    : > "${CHECK_LOG}"
    before="$(invocation)"
    run_setup
    expect_rc 1 "${id}"
    expect_out "[FAIL] camera view (daemon): soos-daemon --check-config refused ${DAEMON_TOML} (removed again; ${UNIT} not restarted)" "${id}"
    expect_out "Setup FAILED in phase camera view (daemon)" "${id}"
    grep -qx "check config=${DAEMON_TOML} file=present present_starts=0" "${CHECK_LOG}" \
        || case_fail "${id}: the created file was not checked: $(cat "${CHECK_LOG}")"
    [[ ! -e "${DAEMON_TOML}" && ! -L "${DAEMON_TOML}" ]] || case_fail "${id}: ${DAEMON_TOML} was not removed"
    [[ ! -e /etc/soos ]] || case_fail "${id}: /etc/soos created by the run was not removed"
    no_temp_files "${id}"
    [[ "$(invocation)" == "${before}" ]] || case_fail "${id}: ${UNIT} was restarted"
    if grep -q "daemon_toml=present" "${START_LOG}"; then
        case_fail "${id}: the daemon was started with the refused file"
    fi
    systemctl is-active --quiet "${UNIT}" || case_fail "${id}: ${UNIT} is not active"
    daemon_ready || case_fail "${id}: ${UNIT} is not healthy"
    companion_not_run "${id}"
    ok "F1 check refused: file and /etc/soos removed, no restart, daemon healthy, exit 1, companion never ran"
}

# F2: the created file is checked exactly once, before the daemon is restarted on it.
case_f2() {
    local id="F2"
    reset_case absent ok
    : > "${CHECK_LOG}"
    run_setup
    expect_rc 0 "${id}"
    [[ "$(cat "${CHECK_LOG}")" == "check config=${DAEMON_TOML} file=present present_starts=0" ]] \
        || case_fail "${id}: expected one check of the created file before any start with it: $(cat "${CHECK_LOG}")"
    grep -qx "start mode=ok daemon_toml=present" "${START_LOG}" \
        || case_fail "${id}: the daemon was not restarted on the checked file"
    expect_out "[DONE] camera view (daemon): ${DAEMON_TOML} created" "${id}"
    companion_ran "${id}"
    ok "F2 check passed: one check of the created file before the restart, exit 0"
}

in_container() {
    [[ "$(id -u)" -eq 0 ]] || fail "--in-container requires root"
    [[ -f /.dockerenv && "$(cat /proc/1/comm 2>/dev/null)" == systemd ]] \
        || fail "--in-container only runs inside the Docker systemd container"
    local -a cases=("$@")
    [[ ${#cases[@]} -gt 0 ]] || cases=("${ALL_CASES[@]}")
    local c k known
    for c in "${cases[@]}"; do
        known=false
        for k in "${ALL_CASES[@]}"; do
            [[ "${c}" == "${k}" ]] && known=true
        done
        [[ "${known}" == true ]] || fail "unknown case '${c}' (known: ${ALL_CASES[*]})"
    done
    wait_for_boot
    assert_host_state_units_masked
    provision
    for c in "${cases[@]}"; do
        info "Case ${c}..."
        "case_${c,,}"
    done
    echo "setup.sh root camera view cases passed: ${cases[*]}"
}

case "${1:-}" in
    --in-container)
        shift
        in_container "$@"
        ;;
    *)
        host_main "$@"
        ;;
esac
