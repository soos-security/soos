#!/usr/bin/env bash
# =============================================================================
# tests/docker/pam_rollback_test.sh — PAM Activation Order & Uninstall Rollback
# =============================================================================
# Validates on real distribution userlands (review findings ONB-03 / GitHub #161
# and ONB-08 / GitHub #166) that activating soos and then running
# scripts/uninstall.sh returns the PAM configuration to its exact pre-install
# state, and that the Debian password-failed hook is reachable on a wrong password.
#
# Debian/Ubuntu (ubuntu:24.04):
#   D0. A live scripts/install.sh (stub artifacts) that fails AFTER block 6a (model
#       digest mismatch) and one whose PAM snapshot helper fails part-way (cp
#       shim) both roll back everything: PAM state byte-identical, no
#       /var/lib/soos (hence no state/.pam-backup.* temp dir), no binaries,
#       no module, no 'soos' group.
#   D1. scripts/pam_snapshot.sh snapshot records the pre-install PAM state
#   D2. `pam-auth-update --package --enable soos soos-notify` succeeds (rc=0)
#   D3. Generated /etc/pam.d/common-auth order:
#         pam_soos.so timeout_ms=250 -> pam_unix.so -> pam_soos.so event=password-failed
#         -> pam_deny.so -> pam_permit.so
#       and the pam_unix success jump lands exactly on pam_permit.so (skips the hook)
#   D4. With the hook replaced by a pam_exec probe (same position and control):
#       wrong password -> probe reached and authentication rejected;
#       correct password -> probe NOT reached and authentication accepted.
#       Password login through common-auth works although pam_soos.so is absent.
#   D5. scripts/uninstall.sh restores every /etc/pam.d file (incl. a gdm-password
#       edited like `soos-admin gdm enable`) byte-for-byte (sha256), removes the
#       profiles and discards the verified snapshot; password login still works.
#
# Fedora (fedora:40):
#   F1. snapshot, profile install, `authselect select custom/soos with-faillock --force`
#   F2. scripts/uninstall.sh restores `authselect current --raw`, every /etc/pam.d
#       file and /etc/nsswitch.conf byte-for-byte (sha256); `authselect check` rc=0;
#       password login still works.
#
# Usage:
#   bash tests/docker/pam_rollback_test.sh                         # host: both distros via Docker
#   bash tests/docker/pam_rollback_test.sh --distro debian|fedora  # host: one distro
#   bash tests/docker/pam_rollback_test.sh --in-container debian|fedora
#   bash tests/docker/pam_rollback_test.sh --help
# =============================================================================

set -euo pipefail

readonly DEBIAN_IMAGE="${SOOS_DEBIAN_IMAGE:-ubuntu:24.04}"
readonly FEDORA_IMAGE="${SOOS_FEDORA_IMAGE:-fedora:40}"
readonly TEST_USER="soostest"
readonly TEST_PASS="password123"
readonly SNAPSHOT_DIR="/var/lib/soos/state/pam-backup"

if [[ -t 1 ]]; then
    readonly GREEN='\033[0;32m'
    readonly RED='\033[0;31m'
    readonly BLUE='\033[0;34m'
    readonly NC='\033[0m'
else
    readonly GREEN=''
    readonly RED=''
    readonly BLUE=''
    readonly NC=''
fi

info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
error()   { echo -e "${RED}[FAIL]${NC}  $*" >&2; }
fail()    { error "$*"; exit 1; }

usage() {
    cat <<EOF
Usage: $(basename "$0") [--distro debian|fedora|all] [--in-container debian|fedora] [--help]

Validates the Debian pam-auth-update stack order (password-failed hook before
pam_deny) and the byte-for-byte PAM rollback performed by scripts/uninstall.sh
on ${DEBIAN_IMAGE} and ${FEDORA_IMAGE} (override with SOOS_DEBIAN_IMAGE /
SOOS_FEDORA_IMAGE). The workspace is mounted read-only at /workspace.

Options:
  --distro <d>        Host mode: run only 'debian', 'fedora' or 'all' (default)
  --in-container <d>  Run the checks on the current (root) system
  -h, --help          Display this help message and exit
EOF
}

MODE="host"
DISTRO="all"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --distro) DISTRO="${2:-}"; shift 2 ;;
        --in-container) MODE="container"; DISTRO="${2:-}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) error "Unknown argument: $1"; usage; exit 1 ;;
    esac
done
case "${DISTRO}" in
    debian|fedora) ;;
    all) [[ "${MODE}" = "host" ]] || fail "--in-container needs 'debian' or 'fedora'" ;;
    *) fail "Unknown distribution: '${DISTRO}'" ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# ---------------------------------------------------------------------------
# Host mode: delegate to ephemeral containers
# ---------------------------------------------------------------------------
if [[ "${MODE}" = "host" ]]; then
    command -v docker >/dev/null 2>&1 || fail "Docker is not installed or not in PATH."
    docker info >/dev/null 2>&1 || fail "Docker daemon is not accessible."
    run_in() {
        local image="$1" distro="$2"
        info "Running PAM rollback validation (${distro}) in ${image}..."
        docker run --rm \
            -v "${WORKSPACE_ROOT}:/workspace:ro" \
            "${image}" \
            bash /workspace/tests/docker/pam_rollback_test.sh --in-container "${distro}"
        success "PAM rollback validation (${distro}) passed in ${image}."
    }
    if [[ "${DISTRO}" = "all" || "${DISTRO}" = "debian" ]]; then
        run_in "${DEBIAN_IMAGE}" debian
    fi
    if [[ "${DISTRO}" = "all" || "${DISTRO}" = "fedora" ]]; then
        run_in "${FEDORA_IMAGE}" fedora
    fi
    exit 0
fi

# ---------------------------------------------------------------------------
# Container mode — shared helpers
# ---------------------------------------------------------------------------
[[ "$(id -u)" -eq 0 ]] || fail "--in-container requires root."

ensure_test_user() {
    if ! id -u "${TEST_USER}" >/dev/null 2>&1; then
        useradd -m -s /bin/bash "${TEST_USER}"
        echo "${TEST_USER}:${TEST_PASS}" | chpasswd
    fi
}

# Deterministic digest of the whole PAM state: type, link target and sha256 of
# the (dereferenced) content of every /etc/pam.d entry and /etc/nsswitch.conf,
# the pam-auth-update profile list and the selected authselect profile.
pam_state_digest() {
    local f
    for f in /etc/pam.d/* /etc/nsswitch.conf; do
        [[ -e "${f}" || -L "${f}" ]] || continue
        if [[ -L "${f}" ]]; then
            printf 'L %s -> %s ' "${f}" "$(readlink "${f}")"
        else
            printf 'F %s ' "${f}"
        fi
        if [[ -e "${f}" ]]; then
            sha256sum < "${f}" | cut -d' ' -f1
        else
            echo "dangling"
        fi
    done
    if [[ -d /usr/share/pam-configs ]]; then
        find /usr/share/pam-configs -maxdepth 1 -type f -printf 'pam-configs: %f\n' | sort
    fi
    if command -v authselect >/dev/null 2>&1; then
        echo "authselect: $(authselect current --raw 2>/dev/null || echo none)"
    fi
}

assert_same_state() {
    local label="$1" baseline="$2" current
    current="$(pam_state_digest)"
    if [[ "${current}" != "${baseline}" ]]; then
        error "${label}: PAM state differs from the pre-install baseline:"
        diff <(echo "${baseline}") <(echo "${current}") >&2 || true
        exit 1
    fi
    success "${label}: PAM state byte-identical to the pre-install baseline ($(echo "${baseline}" | wc -l) entries)."
}

# Asserts correct password accepted and wrong password rejected for service $1.
assert_password_auth() {
    local service="$1"
    if command -v faillock >/dev/null 2>&1; then
        faillock --user "${TEST_USER}" --reset >/dev/null 2>&1 || true
    fi
    if ! echo "${TEST_PASS}" | pamtester "${service}" "${TEST_USER}" authenticate >/dev/null 2>&1; then
        fail "${service}: correct password was rejected (password logins would fail)"
    fi
    if echo "wrong-${TEST_PASS}" | pamtester "${service}" "${TEST_USER}" authenticate >/dev/null 2>&1; then
        fail "${service}: wrong password was accepted"
    fi
    success "${service}: correct password accepted, wrong password rejected."
}

# Simulates `soos-admin gdm enable` (crates/admin-cli/src/gdm.rs): byte-exact
# backup, then insertion of the GDM line before '@include common-auth' (or first).
simulate_gdm_enable() {
    local file="/etc/pam.d/gdm-password"
    cp -p "${file}" "${file}.soos-backup"
    if grep -q '@include common-auth' "${file}"; then
        sed -i '0,/@include common-auth/s//auth  sufficient  pam_soos.so timeout_ms=2500\n@include common-auth/' "${file}"
    else
        sed -i '1i auth  sufficient  pam_soos.so timeout_ms=2500' "${file}"
    fi
    grep -q 'pam_soos.so timeout_ms=2500' "${file}" || fail "gdm fixture: soos line not inserted"
}

assert_no_soos_reference() {
    if grep -rlE '^[[:space:]]*-?(auth|account|password|session)[[:space:]].*pam_soos\.so' /etc/pam.d/ 2>/dev/null; then
        fail "$1: /etc/pam.d still references pam_soos.so (files listed above)"
    fi
    success "$1: no /etc/pam.d file references pam_soos.so."
}

# While soos is active the snapshot verification MUST report each edited file
# (exit 1): a verifier that cannot detect drift would bless any rollback.
# Usage: assert_verify_detects_drift <label> <rel-path>...
assert_verify_detects_drift() {
    local label="$1" rc=0 out rel
    shift
    out="$(bash "${WORKSPACE_ROOT}/scripts/pam_snapshot.sh" verify 2>&1)" || rc=$?
    [[ "${rc}" -eq 1 ]] || fail "${label}: pam_snapshot.sh verify returned ${rc} on an activated stack (expected 1)"
    for rel in "$@"; do
        grep -qF "changed ${rel}" <<< "${out}" \
            || fail "${label}: pam_snapshot.sh verify did not report 'changed ${rel}': ${out}"
    done
    success "${label}: snapshot verification reports the activated files ($*)."
}

# D0: drives a failing LIVE install through scripts/install.sh itself.
# Usage: assert_failed_install_rolls_back <label> <baseline> [PATH prefix]
assert_failed_install_rolls_back() {
    local label="$1" baseline="$2" path_prefix="${3:-}" stub=/tmp/soos-d0 rc=0 out
    rm -rf "${stub}"
    mkdir -p "${stub}/release"
    local name
    for name in soos-daemon soos-admin soos-enroll soos-gui libpam_soos.so; do
        printf 'fixture:%s\n' "${name}" > "${stub}/release/${name}"
        chmod 0755 "${stub}/release/${name}"
    done
    printf 'not-the-attested-model\n' > "${stub}/model.onnx"
    printf '[manifest]\nversion = "2.0.0"\n\n[models.bad_model]\nid = "bad_model"\nfilename = "bad_model.onnx"\nsha256 = "%s"\nlicense = "MIT"\nsource_url = "file://%s/model.onnx"\n' \
        "$(printf '%064d' 0)" "${stub}" > "${stub}/manifest.toml"
    out="$(PATH="${path_prefix:+${path_prefix}:}${PATH}" bash "${WORKSPACE_ROOT}/scripts/install.sh" \
        --artifact-dir "${stub}/release" --manifest "${stub}/manifest.toml" --skip-systemd 2>&1)" || rc=$?
    [[ "${rc}" -ne 0 ]] || fail "${label}: the failing install exited 0"
    grep -q "rolled back" <<< "${out}" || fail "${label}: no rollback reported: ${out}"
    [[ ! -e /var/lib/soos ]] || fail "${label}: /var/lib/soos left behind: $(find /var/lib/soos -maxdepth 3 | tr '\n' ' ')"
    local leftover
    for leftover in /run/soos /usr/libexec/soos /usr/bin/soos-admin /usr/bin/soos-enroll /usr/bin/soos-gui \
            /etc/systemd/system/soos-daemon.service /usr/share/pam-configs/soos /etc/pam.d/soos.snippet; do
        [[ ! -e "${leftover}" ]] || fail "${label}: ${leftover} left behind"
    done
    if find / -xdev -name pam_soos.so -path '*/security/*' 2>/dev/null | grep -q .; then
        fail "${label}: pam_soos.so left behind"
    fi
    ! getent group soos >/dev/null 2>&1 || fail "${label}: group 'soos' left behind"
    assert_same_state "${label}" "${baseline}"
    rm -rf "${stub}"
    success "${label}: failed live install rolled back completely (no state/.pam-backup.* left)."
}

run_uninstall() {
    bash "${WORKSPACE_ROOT}/scripts/uninstall.sh" --keep-data --skip-systemd \
        || fail "$1: scripts/uninstall.sh failed"
    [[ ! -e "${SNAPSHOT_DIR}" ]] || fail "$1: verified snapshot ${SNAPSHOT_DIR} was not discarded"
}

# Effective (non-comment) auth lines of a PAM file, one per line.
auth_lines() { grep -E '^[[:space:]]*auth[[:space:]]' "$1"; }

# 1-based index (among auth lines of $1) of the first auth line containing $2.
auth_index_of() {
    local n
    n="$(auth_lines "$1" | grep -n -F -- "$2" | head -n 1 | cut -d: -f1)"
    [[ -n "${n}" ]] || fail "$1: expected an auth line containing '$2'"
    echo "${n}"
}

# ---------------------------------------------------------------------------
# Debian / Ubuntu
# ---------------------------------------------------------------------------
run_debian() {
    command -v pam-auth-update >/dev/null 2>&1 || fail "pam-auth-update missing (libpam-runtime)"
    if ! command -v pamtester >/dev/null 2>&1; then
        info "Installing pamtester..."
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -qq >/dev/null
        apt-get install -y -qq pamtester >/dev/null
    fi
    ensure_test_user

    # Fixture: a GDM service as shipped by gdm3 (includes common-auth).
    printf '#%%PAM-1.0\nauth    requisite       pam_nologin.so\n@include common-auth\n@include common-account\n' \
        > /etc/pam.d/gdm-password
    chmod 0644 /etc/pam.d/gdm-password
    cat > /etc/pam.d/soos-login <<'EOF'
#%PAM-1.0
@include common-auth
EOF
    local baseline
    baseline="$(pam_state_digest)"
    assert_password_auth soos-login

    info "D0a: live scripts/install.sh failing after the PAM snapshot (model digest mismatch)"
    assert_failed_install_rolls_back D0a "${baseline}"
    info "D0b: live scripts/install.sh whose PAM snapshot helper fails part-way"
    local shim=/tmp/soos-d0-shim
    mkdir -p "${shim}"
    cat > "${shim}/cp" <<'EOF'
#!/bin/sh
for a in "$@"; do
    case "$a" in
        */.pam-backup.*/pam.d/*) echo "cp: simulated failure" >&2; exit 1 ;;
    esac
done
exec /usr/bin/cp "$@"
EOF
    chmod 0755 "${shim}/cp"
    assert_failed_install_rolls_back D0b "${baseline}" "${shim}"
    rm -rf "${shim}"

    info "D1: scripts/pam_snapshot.sh snapshot"
    bash "${WORKSPACE_ROOT}/scripts/pam_snapshot.sh" snapshot || fail "D1: snapshot failed"
    [[ -f "${SNAPSHOT_DIR}/SHA256SUMS" ]] || fail "D1: ${SNAPSHOT_DIR}/SHA256SUMS missing"
    [[ "$(stat -c '%a' "${SNAPSHOT_DIR}")" = "700" ]] || fail "D1: snapshot directory must be 0700"
    success "D1: pre-install PAM state recorded in ${SNAPSHOT_DIR}."

    info "D2: pam-auth-update --package --enable soos soos-notify"
    install -m 0644 "${WORKSPACE_ROOT}/packaging/pam/debian/soos" /usr/share/pam-configs/soos
    install -m 0644 "${WORKSPACE_ROOT}/packaging/pam/debian/soos-notify" /usr/share/pam-configs/soos-notify
    DEBIAN_FRONTEND=noninteractive pam-auth-update --package --enable soos soos-notify \
        || fail "D2: pam-auth-update failed"
    success "D2: profiles enabled."
    simulate_gdm_enable
    assert_verify_detects_drift D2 pam.d/common-auth pam.d/gdm-password

    local ca=/etc/pam.d/common-auth
    info "D3: generated ${ca}:"
    auth_lines "${ca}" | sed 's/^/        /'
    local soos unix_idx notify deny permit total jump
    soos="$(auth_index_of "${ca}" "pam_soos.so timeout_ms=250")"
    unix_idx="$(auth_index_of "${ca}" "pam_unix.so")"
    notify="$(auth_index_of "${ca}" "pam_soos.so event=password-failed timeout_ms=20")"
    deny="$(auth_index_of "${ca}" "pam_deny.so")"
    permit="$(auth_index_of "${ca}" "pam_permit.so")"
    [[ "${soos}" -lt "${unix_idx}" ]] || fail "D3: pam_soos must precede pam_unix"
    [[ "${unix_idx}" -lt "${notify}" ]] || fail "D3: password-failed hook must follow pam_unix"
    [[ "${notify}" -lt "${deny}" ]] \
        || fail "D3: password-failed hook (auth line ${notify}) must precede pam_deny (auth line ${deny}); it is unreachable on a wrong password"
    [[ "${deny}" -lt "${permit}" ]] || fail "D3: pam_deny must precede pam_permit"
    auth_lines "${ca}" | sed -n "${notify}p" | grep -qE '^[[:space:]]*auth[[:space:]]+\[default=ignore\][[:space:]]' \
        || fail "D3: the hook control must be [default=ignore]"
    jump="$(auth_lines "${ca}" | sed -n "${unix_idx}p" | sed -nE 's/.*success=([0-9]+).*/\1/p')"
    [[ -n "${jump}" ]] || fail "D3: pam_unix line has no numeric success jump"
    total=$((unix_idx + jump + 1))
    [[ "${total}" -eq "${permit}" ]] \
        || fail "D3: pam_unix success=${jump} lands on auth line ${total}, expected pam_permit (${permit})"
    success "D3: soos(${soos}) < unix(${unix_idx}, success=${jump} -> permit) < hook(${notify}) < deny(${deny}) < permit(${permit})"

    info "D4: reachability of the password-failed hook (pam_exec probe at the same position)"
    local marker=/run/soos-probe.hit
    cat > /usr/local/sbin/soos-probe <<EOF
#!/bin/sh
touch ${marker}
exit 0
EOF
    chmod 0755 /usr/local/sbin/soos-probe
    sed 's|pam_soos.so event=password-failed timeout_ms=20|pam_exec.so quiet /usr/local/sbin/soos-probe|' \
        "${ca}" > /etc/pam.d/soos-probe
    grep -q 'pam_exec.so quiet /usr/local/sbin/soos-probe' /etc/pam.d/soos-probe || fail "D4: probe not inserted"
    rm -f "${marker}"
    if echo "wrong-${TEST_PASS}" | pamtester soos-probe "${TEST_USER}" authenticate >/dev/null 2>&1; then
        fail "D4: wrong password accepted"
    fi
    [[ -f "${marker}" ]] || fail "D4: the password-failed hook is NOT reached on a wrong password"
    rm -f "${marker}"
    echo "${TEST_PASS}" | pamtester soos-probe "${TEST_USER}" authenticate >/dev/null 2>&1 \
        || fail "D4: correct password rejected"
    [[ ! -f "${marker}" ]] || fail "D4: the password-failed hook is reached after a SUCCESSFUL password"
    success "D4: hook reached on wrong password only; authentication outcome unchanged."
    rm -f /etc/pam.d/soos-probe /usr/local/sbin/soos-probe "${marker}"
    assert_password_auth soos-login

    info "D5: scripts/uninstall.sh"
    run_uninstall D5
    [[ ! -e /usr/share/pam-configs/soos && ! -e /usr/share/pam-configs/soos-notify ]] \
        || fail "D5: pam-auth-update profiles still installed"
    assert_no_soos_reference D5
    assert_same_state D5 "${baseline}"
    assert_password_auth soos-login
}

# ---------------------------------------------------------------------------
# Fedora
# ---------------------------------------------------------------------------
run_fedora() {
    if ! command -v authselect >/dev/null 2>&1 || ! command -v pamtester >/dev/null 2>&1; then
        info "Installing authselect and pamtester..."
        dnf -q install -y authselect pamtester shadow-utils >/dev/null
    fi
    ensure_test_user
    authselect select local with-faillock --force >/dev/null
    printf '#%%PAM-1.0\nauth     substack       password-auth\naccount  include        password-auth\n' \
        > /etc/pam.d/gdm-password
    chmod 0644 /etc/pam.d/gdm-password
    local baseline
    baseline="$(pam_state_digest)"
    assert_password_auth system-auth

    info "F1: snapshot, profile install and activation"
    bash "${WORKSPACE_ROOT}/scripts/pam_snapshot.sh" snapshot || fail "F1: snapshot failed"
    [[ -f "${SNAPSHOT_DIR}/SHA256SUMS" ]] || fail "F1: ${SNAPSHOT_DIR}/SHA256SUMS missing"
    mkdir -p /etc/authselect/custom/soos /etc/soos
    cp "${WORKSPACE_ROOT}"/packaging/pam/fedora/soos/* /etc/authselect/custom/soos/
    authselect current --raw > /etc/soos/authselect.previous
    authselect select custom/soos with-faillock --force >/dev/null || fail "F1: activation failed"
    authselect check || fail "F1: authselect check failed"
    grep -q 'pam_soos.so timeout_ms=250' /etc/pam.d/system-auth || fail "F1: soos line not generated"
    simulate_gdm_enable
    assert_verify_detects_drift F1 pam.d/system-auth pam.d/password-auth pam.d/gdm-password
    assert_password_auth system-auth
    success "F1: custom/soos with-faillock active."

    info "F2: scripts/uninstall.sh"
    run_uninstall F2
    authselect check || fail "F2: authselect check failed after rollback"
    [[ ! -d /etc/authselect/custom/soos ]] || fail "F2: custom profile still present"
    assert_no_soos_reference F2
    assert_same_state F2 "${baseline}"
    assert_password_auth system-auth
}

case "${DISTRO}" in
    debian) run_debian ;;
    fedora) run_fedora ;;
esac

echo ""
success "==================================================================="
success "  PAM activation order and rollback validation succeeded (${DISTRO})"
success "==================================================================="
exit 0
