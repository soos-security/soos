#!/usr/bin/env bash
# =============================================================================
# tests/docker/network_lock_pkcheck_test.sh — polkit network lock under a real
# polkitd (GitHub #365, matrix NLK10)
# =============================================================================
# Starts a system D-Bus and polkitd in the pinned ubuntu:24.04 image (with the
# distribution NetworkManager policy) and asks `pkcheck` for every locked
# NetworkManager action, as an unprivileged user, without and then with
# packaging/polkit/49-soos-network-lock.rules installed (root:root 0644):
#
#   P1. Baseline (distribution defaults) recorded for the six locked actions and
#       the controls.
#   P2. With the rule, every locked action is an authentication challenge
#       (exit 2) that is NOT retained (no polkit.retains_authorization_after_challenge):
#       AUTH_ADMIN on every request, never AUTH_ADMIN_KEEP, never YES, never NO.
#   P3. Controls (settings.modify.hostname, wifi.share.open, enable-disable-wwan)
#       give exactly the baseline result: other actions fall through.
#   P4. The rule changes at least one baseline result (it is really loaded).
#   P5. root stays authorized (exit 0): the lock never locks out repairs.
#
# Sentinel differential (plan evaluation round 1, F-2): a container-only rule
# /etc/polkit-1/rules.d/90-soos-test-sentinel.rules answers YES for every
# org.freedesktop.NetworkManager.* action, so any result not produced by the
# soos rule is "authorized":
#   S1. sentinel only: every NetworkManager action listed by pkaction is rc 0.
#   S2. sentinel + soos rule: each locked action is an unretained challenge
#       ("2 plain") and every other NetworkManager action stays rc 0 (exact
#       list, fall-through, no index-0 guard bug); root stays rc 0.
#
# Limit: a container has no seat, so polkit classifies the subject as
# non-local and applies `allow_any`. The rule never reads `subject`, so the
# result is the same for an active local session; that case is the owner
# hardware check NLK12.
#
# Usage:
#   bash tests/docker/network_lock_pkcheck_test.sh                 # host: runs Docker
#   bash tests/docker/network_lock_pkcheck_test.sh --in-container  # inside the image
# =============================================================================

set -euo pipefail

readonly IMAGE="${SOOS_DEBIAN_IMAGE:-ubuntu:24.04@sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3}"
readonly RULE_REL="packaging/polkit/49-soos-network-lock.rules"
readonly RULE_DEST="/etc/polkit-1/rules.d/49-soos-network-lock.rules"
readonly TEST_USER="soostest"
readonly NM="org.freedesktop.NetworkManager"
readonly LOCKED=(
    network-control
    enable-disable-wifi
    enable-disable-network
    settings.modify.system
    settings.modify.own
)
readonly SENTINEL="/etc/polkit-1/rules.d/90-soos-test-sentinel.rules"
readonly CONTROLS=(
    sleep-wake
    settings.modify.hostname
    wifi.share.open
    enable-disable-wwan
)

info() { echo "[INFO]  $*"; }
ok()   { echo "[OK]    $*"; }
fail() { echo "[FAIL]  $*" >&2; exit 1; }

# Prints "<exit code> <retained|plain>" of a pkcheck run by TEST_USER (or root).
check() {
    local action="$1" who="${2:-${TEST_USER}}" out rc
    if [[ "${who}" == "root" ]]; then
        set +e
        out="$(pkcheck --action-id "${NM}.${action}" --process $$ 2>&1)"
        rc=$?
        set -e
    else
        set +e
        out="$(su "${who}" -s /bin/bash -c \
            "pkcheck --action-id '${NM}.${action}' --process \$\$" 2>&1)"
        rc=$?
        set -e
    fi
    if grep -q 'retains_authorization_after_challenge=1' <<<"${out}"; then
        echo "${rc} retained"
    else
        echo "${rc} plain"
    fi
}

POLKITD_PID=""

# (Re)starts polkitd so that it loads the current rules directory.
start_polkitd() {
    if [[ -n "${POLKITD_PID}" ]]; then
        kill "${POLKITD_PID}" 2>/dev/null || true
        wait "${POLKITD_PID}" 2>/dev/null || true
    fi
    /usr/lib/polkit-1/polkitd --no-debug >/dev/null 2>&1 &
    POLKITD_PID=$!
    local i
    for i in $(seq 50); do
        if [[ "$(check network-control root)" == "0 plain" ]]; then
            return 0
        fi
        sleep 0.2
    done
    fail "polkitd did not answer within 10 s"
}

in_container() {
    local ws="$1"
    info "Installing polkitd, network-manager (policy file only) and dbus..."
    apt-get update -qq >/dev/null
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
        polkitd network-manager dbus >/dev/null 2>&1 || fail "package installation failed"
    [[ -f /usr/share/polkit-1/actions/${NM}.policy ]] || fail "NetworkManager policy missing"
    useradd -m "${TEST_USER}"
    mkdir -p /run/dbus
    dbus-daemon --system --fork 2>/dev/null || fail "system bus did not start"

    declare -A base
    start_polkitd
    local a
    for a in "${LOCKED[@]}" "${CONTROLS[@]}"; do
        base[${a}]="$(check "${a}")"
        info "P1 baseline ${a}: ${base[${a}]}"
    done

    install -o root -g root -m 0644 "${ws}/${RULE_REL}" "${RULE_DEST}"
    start_polkitd

    local changed=0 now
    for a in "${LOCKED[@]}"; do
        now="$(check "${a}")"
        [[ "${now}" == "2 plain" ]] \
            || fail "P2 ${a}: expected an unretained challenge (2 plain), got '${now}'"
        [[ "${now}" != "${base[${a}]}" ]] && changed=$((changed + 1))
    done
    ok "P2 every locked action asks for administrator authentication on every request"

    for a in "${CONTROLS[@]}"; do
        now="$(check "${a}")"
        [[ "${now}" == "${base[${a}]}" ]] \
            || fail "P3 ${a}: rule changed an unrelated action ('${base[${a}]}' -> '${now}')"
    done
    ok "P3 other actions fall through to the distribution default"

    [[ "${changed}" -ge 1 ]] || fail "P4 the rule changed no baseline result (not loaded?)"
    ok "P4 the rule changed ${changed} baseline result(s)"

    [[ "$(check network-control root)" == "0 plain" ]] || fail "P5 root lost the action"
    ok "P5 root stays authorized"

    local all_ids=() id suffix locked
    mapfile -t all_ids < <(pkaction | grep "^${NM}\." | sort)
    [[ "${#all_ids[@]}" -ge 10 ]] || fail "pkaction listed only ${#all_ids[@]} NetworkManager actions"
    cat > "${SENTINEL}" <<'RULE'
// Test-only sentinel (tests/docker/network_lock_pkcheck_test.sh): sorts after the soos rule.
polkit.addRule(function (action, subject) {
    if (action.id.indexOf("org.freedesktop.NetworkManager.") === 0) {
        return polkit.Result.YES;
    }
});
RULE
    chmod 0644 "${SENTINEL}"

    rm -f "${RULE_DEST}"
    start_polkitd
    for id in "${all_ids[@]}"; do
        suffix="${id#"${NM}".}"
        now="$(check "${suffix}")"
        [[ "${now}" == "0 plain" ]] || fail "S1 sentinel not reached for ${id} ('${now}')"
    done
    ok "S1 the sentinel authorizes all ${#all_ids[@]} NetworkManager actions"

    install -o root -g root -m 0644 "${ws}/${RULE_REL}" "${RULE_DEST}"
    start_polkitd
    for id in "${all_ids[@]}"; do
        suffix="${id#"${NM}".}"
        locked=false
        for a in "${LOCKED[@]}"; do
            [[ "${a}" == "${suffix}" ]] && locked=true
        done
        now="$(check "${suffix}")"
        if [[ "${locked}" == true ]]; then
            [[ "${now}" == "2 plain" ]] || fail "S2 ${id}: expected '2 plain', got '${now}'"
        else
            [[ "${now}" == "0 plain" ]] || fail "S2 ${id}: not locked but got '${now}' (fall-through broken)"
        fi
    done
    for a in "${LOCKED[@]}"; do
        printf '%s\n' "${all_ids[@]}" | grep -qx "${NM}.${a}" \
            || fail "S2 locked action ${NM}.${a} is not defined by the installed policy"
    done
    [[ "$(check network-control root)" == "0 plain" ]] || fail "S2 root lost the action"
    ok "S2 exactly the ${#LOCKED[@]} locked actions ask for an administrator; the rest fall through"
    echo "network lock pkcheck test: all checks passed"
}

case "${1:-}" in
    --in-container)
        in_container /workspace
        ;;
    -h|--help)
        sed -n '2,30p' "$0"
        ;;
    "")
        command -v docker >/dev/null 2>&1 || fail "docker is required"
        repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
        info "Running in ${IMAGE}"
        docker run --rm -v "${repo}:/workspace:ro" "${IMAGE}" \
            bash /workspace/tests/docker/network_lock_pkcheck_test.sh --in-container
        ;;
    *)
        fail "unknown option: $1 (use --help)"
        ;;
esac
