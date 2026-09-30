#!/usr/bin/env bash
# =============================================================================
# tests/docker/authselect_profile_test.sh — Fedora authselect Profile Activation
# =============================================================================
# Validates the custom authselect profile shipped in packaging/pam/fedora/soos
# on a real Fedora userland (GitHub #145 / review finding ONB-02):
#
#   A1. `authselect select custom/soos with-faillock --force` succeeds (rc=0)
#   A2. `authselect check` reports a valid configuration (rc=0)
#   A3. Generated /etc/pam.d/system-auth and password-auth contain no unresolved
#       template syntax and keep the order:
#         pam_faillock.so preauth -> pam_soos.so -> pam_unix.so
#         -> pam_faillock.so authfail -> pam_soos.so event=password-failed -> pam_deny.so
#       plus the account-phase pam_faillock.so line
#   A4. /etc/nsswitch.conf keeps every non-comment line of the base `local` profile
#   A5. Password authentication still works through the generated stacks
#       (correct password accepted, wrong password rejected) although pam_soos.so
#       is not installed ([success=done default=ignore] fallback)
#   A6. Activation without `with-faillock` also succeeds and emits no pam_faillock line
#   A7. Rollback: scripts/uninstall.sh restores the profile recorded in
#       /etc/soos/authselect.previous before removing /etc/authselect/custom/soos
#
# Usage:
#   bash tests/docker/authselect_profile_test.sh            # host: runs fedora:40 via Docker
#   bash tests/docker/authselect_profile_test.sh --in-container   # inside a Fedora/RHEL container
#   bash tests/docker/authselect_profile_test.sh --help
# =============================================================================

set -euo pipefail

readonly FEDORA_IMAGE="${SOOS_FEDORA_IMAGE:-fedora:40}"
readonly PROFILE_SRC_REL="packaging/pam/fedora/soos"
readonly PROFILE_DST="/etc/authselect/custom/soos"
readonly PREVIOUS_FILE="/etc/soos/authselect.previous"
readonly TEST_USER="soostest"
readonly TEST_PASS="password123"

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
Usage: $(basename "$0") [--in-container] [--help]

Validates activation, generated PAM/NSS configuration and rollback of the
soos custom authselect profile (packaging/pam/fedora/soos) on Fedora.

Without options the script starts an ephemeral ${FEDORA_IMAGE} container
(override with SOOS_FEDORA_IMAGE) with the workspace mounted at /workspace
and re-executes itself with --in-container.

Options:
  --in-container   Run the checks on the current (Fedora/RHEL, root) system
  -h, --help       Display this help message and exit
EOF
}

MODE="host"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --in-container) MODE="container"; shift ;;
        -h|--help) usage; exit 0 ;;
        *) error "Unknown argument: $1"; usage; exit 1 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# ---------------------------------------------------------------------------
# Host mode: delegate to an ephemeral Fedora container
# ---------------------------------------------------------------------------
if [[ "${MODE}" = "host" ]]; then
    command -v docker >/dev/null 2>&1 || fail "Docker is not installed or not in PATH."
    docker info >/dev/null 2>&1 || fail "Docker daemon is not accessible."
    info "Running authselect profile validation in ${FEDORA_IMAGE}..."
    docker run --rm \
        -v "${WORKSPACE_ROOT}:/workspace:ro" \
        "${FEDORA_IMAGE}" \
        bash /workspace/tests/docker/authselect_profile_test.sh --in-container
    success "authselect profile validation passed in ${FEDORA_IMAGE}."
    exit 0
fi

# ---------------------------------------------------------------------------
# Container mode
# ---------------------------------------------------------------------------
[[ "$(id -u)" -eq 0 ]] || fail "--in-container requires root."
PROFILE_SRC="${WORKSPACE_ROOT}/${PROFILE_SRC_REL}"
[[ -d "${PROFILE_SRC}" ]] || fail "Profile source directory missing: ${PROFILE_SRC}"

if ! command -v authselect >/dev/null 2>&1 || ! command -v pamtester >/dev/null 2>&1; then
    info "Installing authselect and pamtester..."
    dnf -q install -y authselect pamtester shadow-utils >/dev/null
fi

if ! id -u "${TEST_USER}" >/dev/null 2>&1; then
    useradd -m -s /bin/bash "${TEST_USER}"
    echo "${TEST_USER}:${TEST_PASS}" | chpasswd
fi

# Non-comment, non-empty lines of a generated file.
effective_lines() { grep -v '^[[:space:]]*#' "$1" | sed '/^[[:space:]]*$/d'; }

# Line number of the first line containing $2 in file $1 (fails when absent).
line_of() {
    local n
    n="$(grep -n -F -- "$2" "$1" | head -n 1 | cut -d: -f1)"
    [[ -n "${n}" ]] || fail "$1: expected a line containing '$2'"
    echo "${n}"
}

# Primary soos rule relying on the module default deadline (GitHub #185): no
# timeout_ms= argument, or timeout_ms=1000 (DEFAULT_TIMEOUT_MS).
SOOS_PRIMARY_RE='\[success=done default=ignore\][[:space:]]+pam_soos\.so([[:space:]]+timeout_ms=1000)?[[:space:]]*$'

# Prints the 1-based line number of the primary soos rule in file $1 (fails if absent).
soos_primary_line_of() {
    local n
    n="$(grep -n -E -- "${SOOS_PRIMARY_RE}" "$1" | head -n 1 | cut -d: -f1)"
    [[ -n "${n}" ]] || fail "$1: expected the primary pam_soos.so rule (default timeout)"
    echo "${n}"
}

# Asserts correct password accepted and wrong password rejected for service $1.
# The faillock tally is reset first: pamtester runs only the auth phase, so the
# deliberate wrong-password attempts would otherwise lock the user (deny=3).
assert_password_auth() {
    local service="$1"
    faillock --user "${TEST_USER}" --reset >/dev/null 2>&1 || true
    if ! echo "${TEST_PASS}" | pamtester "${service}" "${TEST_USER}" authenticate >/dev/null 2>&1; then
        fail "${service}: correct password was rejected (all password logins would fail)"
    fi
    if echo "wrong-${TEST_PASS}" | pamtester "${service}" "${TEST_USER}" authenticate >/dev/null 2>&1; then
        fail "${service}: wrong password was accepted"
    fi
    success "${service}: correct password accepted, wrong password rejected."
}

# ---------------------------------------------------------------------------
# Baseline: the stock `local` profile with the same feature set
# ---------------------------------------------------------------------------
info "Selecting baseline profile 'local with-faillock'..."
authselect select local with-faillock --force >/dev/null
BASELINE_NSSWITCH="$(effective_lines /etc/nsswitch.conf)"
[[ -n "${BASELINE_NSSWITCH}" ]] || fail "baseline /etc/nsswitch.conf has no effective lines"
assert_password_auth system-auth

# ---------------------------------------------------------------------------
# A1/A2: activation with the documented command
# ---------------------------------------------------------------------------
info "Installing profile into ${PROFILE_DST}..."
rm -rf "${PROFILE_DST}"
mkdir -p "${PROFILE_DST}"
cp "${PROFILE_SRC}"/* "${PROFILE_DST}/"

info "A1: authselect select custom/soos with-faillock --force"
if ! authselect select custom/soos with-faillock --force; then
    fail "A1: documented activation command failed"
fi
success "A1: profile activated."

info "A2: authselect check"
authselect check || fail "A2: authselect check reported an invalid configuration"
CURRENT="$(authselect current --raw)"
[[ "${CURRENT}" = "custom/soos with-faillock" ]] \
    || fail "A2: unexpected 'authselect current --raw' output: '${CURRENT}'"
success "A2: configuration valid, current profile: ${CURRENT}"

# ---------------------------------------------------------------------------
# A3: generated stacks
# ---------------------------------------------------------------------------
for stack in system-auth password-auth; do
    file="/etc/pam.d/${stack}"
    info "A3: inspecting generated ${file}"
    if grep -q '{' "${file}"; then
        fail "A3: ${file} contains unresolved template syntax: $(grep -n '{' "${file}" | head -n 3)"
    fi
    preauth="$(line_of "${file}" "pam_faillock.so preauth")"
    soos="$(soos_primary_line_of "${file}")"
    unix_line="$(line_of "${file}" "pam_unix.so")"
    authfail="$(line_of "${file}" "pam_faillock.so authfail")"
    event="$(line_of "${file}" "pam_soos.so event=password-failed timeout_ms=20")"
    deny="$(line_of "${file}" "pam_deny.so")"
    [[ "${preauth}" -lt "${soos}" ]] || fail "A3: ${file}: faillock preauth must precede pam_soos"
    [[ "${soos}" -lt "${unix_line}" ]] || fail "A3: ${file}: pam_soos must precede pam_unix"
    [[ "${unix_line}" -lt "${authfail}" ]] || fail "A3: ${file}: faillock authfail must follow pam_unix"
    [[ "${authfail}" -lt "${event}" ]] || fail "A3: ${file}: password-failed event must follow faillock authfail"
    [[ "${event}" -lt "${deny}" ]] || fail "A3: ${file}: pam_deny must close the auth stack"
    grep -qE '^account[[:space:]]+required[[:space:]]+pam_faillock\.so' "${file}" \
        || fail "A3: ${file}: account-phase pam_faillock.so missing"
    faillock_count="$(grep -c 'pam_faillock.so' "${file}")"
    [[ "${faillock_count}" -eq 3 ]] || fail "A3: ${file}: expected 3 pam_faillock lines, found ${faillock_count}"
    success "A3: ${file}: faillock preauth(${preauth}) < soos(${soos}) < unix(${unix_line}) < authfail(${authfail}) < event(${event}) < deny(${deny})"
done

# ---------------------------------------------------------------------------
# A4: nsswitch.conf preserved
# ---------------------------------------------------------------------------
info "A4: comparing /etc/nsswitch.conf with the baseline"
CUSTOM_NSSWITCH="$(effective_lines /etc/nsswitch.conf)"
if [[ "${CUSTOM_NSSWITCH}" != "${BASELINE_NSSWITCH}" ]]; then
    error "A4: /etc/nsswitch.conf differs from the baseline profile:"
    diff <(echo "${BASELINE_NSSWITCH}") <(echo "${CUSTOM_NSSWITCH}") >&2 || true
    exit 1
fi
success "A4: /etc/nsswitch.conf identical to the baseline ($(echo "${BASELINE_NSSWITCH}" | wc -l) lines)."

# ---------------------------------------------------------------------------
# A5: password fallback through the generated stacks
# ---------------------------------------------------------------------------
info "A5: password authentication through generated stacks (pam_soos.so absent)"
assert_password_auth system-auth
assert_password_auth password-auth

# ---------------------------------------------------------------------------
# A6: activation without with-faillock
# ---------------------------------------------------------------------------
info "A6: authselect select custom/soos --force (no optional feature)"
authselect select custom/soos --force >/dev/null || fail "A6: activation without features failed"
authselect check || fail "A6: authselect check failed without features"
if grep -q 'pam_faillock.so' /etc/pam.d/system-auth; then
    fail "A6: pam_faillock emitted although with-faillock was not selected"
fi
grep -q -E -- "${SOOS_PRIMARY_RE}" /etc/pam.d/system-auth || fail "A6: pam_soos line missing"
assert_password_auth system-auth
success "A6: profile valid without optional features."

# ---------------------------------------------------------------------------
# A7: rollback through scripts/uninstall.sh
# ---------------------------------------------------------------------------
info "A7: rollback via scripts/uninstall.sh --keep-data --skip-systemd"
authselect select custom/soos with-faillock --force >/dev/null
mkdir -p "$(dirname "${PREVIOUS_FILE}")"
printf '%s\n' "local with-faillock" > "${PREVIOUS_FILE}"
bash "${WORKSPACE_ROOT}/scripts/uninstall.sh" --keep-data --skip-systemd
CURRENT="$(authselect current --raw)"
[[ "${CURRENT}" = "local with-faillock" ]] \
    || fail "A7: uninstall did not restore the recorded profile (current: '${CURRENT}')"
[[ ! -d "${PROFILE_DST}" ]] || fail "A7: ${PROFILE_DST} still present after uninstall"
[[ ! -f "${PREVIOUS_FILE}" ]] || fail "A7: ${PREVIOUS_FILE} still present after uninstall"
authselect check || fail "A7: configuration invalid after rollback"
ROLLED_BACK_NSSWITCH="$(effective_lines /etc/nsswitch.conf)"
[[ "${ROLLED_BACK_NSSWITCH}" = "${BASELINE_NSSWITCH}" ]] || fail "A7: nsswitch.conf differs after rollback"
assert_password_auth system-auth
success "A7: previous profile restored and custom profile removed."

echo ""
success "==================================================================="
success "  Fedora authselect profile validation succeeded (A1-A7)"
success "==================================================================="
exit 0
