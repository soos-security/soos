#!/usr/bin/env bash
# =============================================================================
# crates/admin-cli/tests/network_lock_install_test.sh — network lock install,
# status and uninstall against a temporary root (GitHub #365, matrix NLK2/NLK3)
# =============================================================================
# Drives the real `soos-admin` binary and scripts/uninstall.sh against a staging
# root created with mktemp; never touches the host's /etc and never needs root.
# Run by crates/admin-cli/tests/network_lock_shell_tests.rs:
#
#   network_lock_install_test.sh <path-to-soos-admin> <repository-root>
# =============================================================================

set -euo pipefail

BIN="${1:?usage: network_lock_install_test.sh <soos-admin> <repo-root>}"
REPO="${2:?usage: network_lock_install_test.sh <soos-admin> <repo-root>}"

RULES_SRC="${REPO}/packaging/polkit/49-soos-network-lock.rules"
RFKILL_SRC="${REPO}/packaging/udev/72-soos-network-lock-rfkill.rules"
RULES_REL="etc/polkit-1/rules.d/49-soos-network-lock.rules"
RFKILL_REL="etc/udev/rules.d/72-soos-network-lock-rfkill.rules"

STAGE="$(mktemp -d)"
trap 'rm -rf -- "${STAGE}"' EXIT

fail() { echo "[FAIL] $*" >&2; exit 1; }
ok()   { echo "[OK]   $*"; }

new_root() {
    local root="${STAGE}/$1"
    (umask 022 && mkdir -p "${root}/etc/polkit-1/rules.d" "${root}/etc/udev/rules.d")
    chmod 0755 "${root}" "${root}/etc" "${root}/etc/polkit-1" "${root}/etc/polkit-1/rules.d" \
        "${root}/etc/udev" "${root}/etc/udev/rules.d"
    echo "${root}"
}

# --- S1: install writes the shipped rule, 0644, atomically --------------------
ROOT="$(new_root s1)"
"${BIN}" network-lock install --root "${ROOT}" >/dev/null || fail "S1 install exited non-zero"
cmp -s "${RULES_SRC}" "${ROOT}/${RULES_REL}" || fail "S1 installed rule differs from ${RULES_SRC}"
[[ "$(stat -c '%a' "${ROOT}/${RULES_REL}")" == "644" ]] || fail "S1 mode is not 0644"
[[ ! -e "${ROOT}/${RFKILL_REL}" ]] || fail "S1 rfkill rule installed without --rfkill"
leftover="$(find "${ROOT}/etc" -name '.*' -type f | wc -l)"
[[ "${leftover}" -eq 0 ]] || fail "S1 temporary file left behind"
ok "S1 install writes the shipped rule with mode 0644 and no temporary file"

# --- S2: status of a caller-owned staging copy reports insecure_permissions ----
set +e
status_out="$("${BIN}" network-lock status --root "${ROOT}")"
status_rc=$?
set -e
if [[ "$(id -u)" -eq 0 ]]; then
    [[ "${status_rc}" -eq 0 ]] || fail "S2 root-owned rule: status exited ${status_rc}"
    grep -q "installed" <<<"${status_out}" || fail "S2 status does not report installed"
else
    [[ "${status_rc}" -eq 1 ]] || fail "S2 caller-owned rule: status exited ${status_rc}, expected 1"
    grep -q "insecure_permissions" <<<"${status_out}" \
        || fail "S2 a rule not owned by root is not reported as insecure_permissions"
fi
json="$("${BIN}" --format json network-lock status --root "${ROOT}" || true)"
grep -q '"rules"' <<<"${json}" || fail "S2 --format json prints no JSON report"
ok "S2 status checks the owner (exit ${status_rc})"

# --- S3: --rfkill adds the udev rule; a plain install removes it again --------
"${BIN}" network-lock install --rfkill --root "${ROOT}" >/dev/null || fail "S3 install --rfkill failed"
cmp -s "${RFKILL_SRC}" "${ROOT}/${RFKILL_REL}" || fail "S3 installed udev rule differs"
[[ "$(stat -c '%a' "${ROOT}/${RFKILL_REL}")" == "644" ]] || fail "S3 udev rule mode is not 0644"
"${BIN}" network-lock install --root "${ROOT}" >/dev/null || fail "S3 plain re-install failed"
[[ ! -e "${ROOT}/${RFKILL_REL}" ]] || fail "S3 plain install kept the rfkill rule"
ok "S3 the rfkill rule follows the --rfkill option"

# --- S4: a symlink at the target name is refused, its target untouched --------
ROOT="$(new_root s4)"
echo "keep me" > "${STAGE}/outside"
ln -s "${STAGE}/outside" "${ROOT}/${RULES_REL}"
if "${BIN}" network-lock install --root "${ROOT}" >/dev/null 2>&1; then
    fail "S4 install wrote through a symlink"
fi
[[ "$(cat "${STAGE}/outside")" == "keep me" ]] || fail "S4 symlink target modified"
[[ -L "${ROOT}/${RULES_REL}" ]] || fail "S4 symlink replaced"
ok "S4 symlink target refused"

# --- S5: a symlinked rules directory is refused --------------------------------
ROOT="$(new_root s5)"
mkdir -p "${STAGE}/elsewhere"
rmdir "${ROOT}/etc/polkit-1/rules.d"
ln -s "${STAGE}/elsewhere" "${ROOT}/etc/polkit-1/rules.d"
if "${BIN}" network-lock install --root "${ROOT}" >/dev/null 2>&1; then
    fail "S5 install wrote through a symlinked directory"
fi
[[ -z "$(ls -A "${STAGE}/elsewhere")" ]] || fail "S5 file written through the symlinked directory"
ok "S5 symlinked directory refused"

# --- S6: uninstall removes both files and is idempotent -----------------------
ROOT="$(new_root s6)"
"${BIN}" network-lock install --rfkill --root "${ROOT}" >/dev/null || fail "S6 install failed"
"${BIN}" network-lock uninstall --root "${ROOT}" >/dev/null || fail "S6 uninstall failed"
[[ ! -e "${ROOT}/${RULES_REL}" && ! -e "${ROOT}/${RFKILL_REL}" ]] || fail "S6 files left after uninstall"
"${BIN}" network-lock uninstall --root "${ROOT}" >/dev/null || fail "S6 second uninstall failed"
ok "S6 uninstall removes both files"

# --- S7: scripts/uninstall.sh removes both files from a destdir ---------------
ROOT="$(new_root s7)"
"${BIN}" network-lock install --rfkill --root "${ROOT}" >/dev/null || fail "S7 install failed"
bash "${REPO}/scripts/uninstall.sh" --destdir "${ROOT}" --keep-data --skip-systemd >/dev/null 2>&1 \
    || fail "S7 scripts/uninstall.sh exited non-zero"
[[ ! -e "${ROOT}/${RULES_REL}" ]] || fail "S7 scripts/uninstall.sh kept the polkit rule"
[[ ! -e "${ROOT}/${RFKILL_REL}" ]] || fail "S7 scripts/uninstall.sh kept the rfkill rule"
ok "S7 scripts/uninstall.sh removes the network lock"

# --- S8: the real root is refused without root privileges ---------------------
if [[ "$(id -u)" -ne 0 ]]; then
    if "${BIN}" network-lock install >/dev/null 2>&1; then
        fail "S8 install on / succeeded without root"
    fi
    if "${BIN}" network-lock uninstall >/dev/null 2>&1; then
        fail "S8 uninstall on / succeeded without root"
    fi
    ok "S8 install and uninstall on / need root"
fi

echo "network lock shell test: all checks passed"
