#!/usr/bin/env bash
# =============================================================================
# crates/admin-cli/tests/network_lock_uninstall_script_test.sh — scripts/uninstall.sh
# network lock hardening against a temporary root (GitHub #370, matrix NLK15, NLK17)
# =============================================================================
# Drives the real `soos-admin` binary and scripts/uninstall.sh --destdir against
# staging roots created with mktemp; never touches the host's /etc and never
# needs root. Run by crates/admin-cli/tests/network_lock_shell_tests.rs:
#
#   network_lock_uninstall_script_test.sh <path-to-soos-admin> <repository-root>
# =============================================================================

set -euo pipefail

BIN="${1:?usage: network_lock_uninstall_script_test.sh <soos-admin> <repo-root>}"
REPO="${2:?usage: network_lock_uninstall_script_test.sh <soos-admin> <repo-root>}"

RULES_NAME="49-soos-network-lock.rules"
RFKILL_NAME="72-soos-network-lock-rfkill.rules"
RULES_REL="etc/polkit-1/rules.d/${RULES_NAME}"
RFKILL_REL="etc/udev/rules.d/${RFKILL_NAME}"

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

run_uninstall() {
    bash "${REPO}/scripts/uninstall.sh" --destdir "$1" --keep-data --skip-systemd 2>&1
}

# Pids no process can have: PID_MAX_LIMIT (pids are below pid_max <= 4194304) and above.
DEAD=4194304
DEAD2=4194305

# --- U1: a symlinked ancestor of the polkit rules directory is refused -------
ROOT="$(new_root u1)"
"${BIN}" network-lock install --rfkill --root "${ROOT}" >/dev/null || fail "U1 install failed"
mv "${ROOT}/etc/polkit-1" "${STAGE}/u1-real-polkit-1"
ln -s "${STAGE}/u1-real-polkit-1" "${ROOT}/etc/polkit-1"
out="$(run_uninstall "${ROOT}")" || fail "U1 scripts/uninstall.sh exited non-zero: ${out}"
[[ -f "${STAGE}/u1-real-polkit-1/rules.d/${RULES_NAME}" ]] \
    || fail "U1 the rule was removed through a symlinked ancestor"
grep -q "symbolic link" <<<"${out}" || fail "U1 no warning about the symbolic link: ${out}"
[[ ! -e "${ROOT}/${RFKILL_REL}" ]] || fail "U1 the rfkill rule on a clean path was kept"
ok "U1 a symlinked etc/polkit-1 keeps the rule and its target"

# --- U2: a symlinked etc/udev is refused as well ------------------------------
ROOT="$(new_root u2)"
"${BIN}" network-lock install --rfkill --root "${ROOT}" >/dev/null || fail "U2 install failed"
mv "${ROOT}/etc/udev" "${STAGE}/u2-real-udev"
ln -s "${STAGE}/u2-real-udev" "${ROOT}/etc/udev"
out="$(run_uninstall "${ROOT}")" || fail "U2 scripts/uninstall.sh exited non-zero: ${out}"
[[ -f "${STAGE}/u2-real-udev/rules.d/${RFKILL_NAME}" ]] \
    || fail "U2 the rfkill rule was removed through a symlinked ancestor"
[[ ! -e "${ROOT}/${RULES_REL}" ]] || fail "U2 the polkit rule on a clean path was kept"
ok "U2 a symlinked etc/udev keeps the rfkill rule and its target"

# --- U3: a symlinked etc is refused for both files ----------------------------
ROOT="$(new_root u3)"
"${BIN}" network-lock install --rfkill --root "${ROOT}" >/dev/null || fail "U3 install failed"
mv "${ROOT}/etc" "${STAGE}/u3-real-etc"
ln -s "${STAGE}/u3-real-etc" "${ROOT}/etc"
run_uninstall "${ROOT}" >/dev/null || fail "U3 scripts/uninstall.sh exited non-zero"
[[ -f "${STAGE}/u3-real-etc/polkit-1/rules.d/${RULES_NAME}" ]] \
    || fail "U3 the polkit rule was removed through a symlinked etc"
[[ -f "${STAGE}/u3-real-etc/udev/rules.d/${RFKILL_NAME}" ]] \
    || fail "U3 the rfkill rule was removed through a symlinked etc"
ok "U3 a symlinked etc keeps both rules"

# --- U4: stale temp files of a dead process are removed, others kept ----------
ROOT="$(new_root u4)"
"${BIN}" network-lock install --rfkill --root "${ROOT}" >/dev/null || fail "U4 install failed"
RULES_DIR="${ROOT}/etc/polkit-1/rules.d"
RFKILL_DIR="${ROOT}/etc/udev/rules.d"
echo half > "${RULES_DIR}/.${RULES_NAME}.${DEAD}.tmp"
echo half > "${RFKILL_DIR}/.${RFKILL_NAME}.${DEAD}.tmp"
echo keep > "${RULES_DIR}/.${RULES_NAME}.$$.tmp"          # live pid (this shell)
echo keep > "${RULES_DIR}/.${RULES_NAME}.${DEAD}x.tmp"    # near name
echo keep > "${RULES_DIR}/.10-other.rules.${DEAD}.tmp"    # another file's name
echo keep > "${RULES_DIR}/.${RULES_NAME}.0${DEAD}.tmp"    # leading zero
echo keep > "${RULES_DIR}/.${RULES_NAME}.2147483648.tmp"  # above i32::MAX
echo outside > "${STAGE}/u4-outside"
ln -s "${STAGE}/u4-outside" "${RFKILL_DIR}/.${RFKILL_NAME}.${DEAD2}.tmp"
run_uninstall "${ROOT}" >/dev/null || fail "U4 scripts/uninstall.sh exited non-zero"
[[ ! -e "${RULES_DIR}/.${RULES_NAME}.${DEAD}.tmp" ]] || fail "U4 stale polkit temp file kept"
[[ ! -e "${RFKILL_DIR}/.${RFKILL_NAME}.${DEAD}.tmp" ]] || fail "U4 stale udev temp file kept"
[[ -f "${RULES_DIR}/.${RULES_NAME}.$$.tmp" ]] || fail "U4 temp file of a live process removed"
[[ -f "${RULES_DIR}/.${RULES_NAME}.${DEAD}x.tmp" ]] || fail "U4 near name removed"
[[ -f "${RULES_DIR}/.10-other.rules.${DEAD}.tmp" ]] || fail "U4 another file's temp name removed"
[[ -f "${RULES_DIR}/.${RULES_NAME}.0${DEAD}.tmp" ]] || fail "U4 leading-zero name removed"
[[ -f "${RULES_DIR}/.${RULES_NAME}.2147483648.tmp" ]] || fail "U4 pid above i32::MAX removed"
[[ "$(cat "${STAGE}/u4-outside")" == "outside" ]] || fail "U4 symlink target modified"
[[ -n "$(find "${RFKILL_DIR}" -maxdepth 1 -type l)" ]] || fail "U4 symlink with a temp name removed"
[[ ! -e "${ROOT}/${RULES_REL}" && ! -e "${ROOT}/${RFKILL_REL}" ]] || fail "U4 rule files kept"
ok "U4 stale temp files of a dead process removed; live, near-name and symlink entries kept"

echo "network lock uninstall script test: all checks passed"
