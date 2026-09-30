#!/usr/bin/env bash
# =============================================================================
# tests/docker/pam_case_lib.sh — Shared helpers for the Docker PAM matrix
# =============================================================================
# Sourced by tests/docker/test_suite.sh (GitHub #189, review TCI-06). Every helper
# returns non-zero on a failed expectation; callers exit 1 on it. Nothing here is
# a bypass: the helpers only measure and assert.
#
#   pam_stack_timeout_ms <pam file>   budget of the first pam_soos.so auth line that
#                                     is not the password-failed hook, with the module's
#                                     own default (1000 ms) and clamp (10..5000 ms)
#   deadline_bound_ms <timeout_ms>    largest accepted elapsed time for one PAM run
#   timeout_mock_delay_ms <timeout_ms> mock daemon delay, always above the bound
#   now_ms                            monotonic-enough wall clock in milliseconds
#   assert_elapsed_within_deadline <label> <elapsed_ms> <timeout_ms>
#   assert_no_facial_authorization <service> <label>
#   assert_password_fallback <service> <label>
# =============================================================================

# Module defaults (crates/pam/src/config.rs: DEFAULT_/MIN_/MAX_TIMEOUT_MS).
readonly PAM_CASE_DEFAULT_TIMEOUT_MS=1000
readonly PAM_CASE_MIN_TIMEOUT_MS=10
readonly PAM_CASE_MAX_TIMEOUT_MS=5000
# Tolerance above the module budget for one pam_test_runner run in a container:
# process start, dlopen of the PAM stack, pam_unix password hashing and scheduler
# noise on shared CI runners. The mock delay always exceeds budget + tolerance, so
# a module that waits for the daemon instead of its deadline is still detected.
readonly PAM_CASE_DEADLINE_TOLERANCE_MS=1000
readonly PAM_CASE_MOCK_DELAY_MARGIN_MS=1500

pam_stack_timeout_ms() {
    local file="$1"
    local line
    line="$(grep -E '^[[:space:]]*-?auth[[:space:]].*pam_soos\.so' "${file}" \
        | grep -v 'event=' | head -n 1 || true)"
    if [[ -z "${line}" ]]; then
        echo "pam_stack_timeout_ms: no pam_soos.so auth line in ${file}" >&2
        return 1
    fi
    local value
    value="$(printf '%s\n' "${line}" | grep -oE '(^|[[:space:]])timeout_ms=[0-9]+' \
        | head -n 1 | sed 's/.*timeout_ms=//' || true)"
    if [[ -z "${value}" ]]; then
        echo "${PAM_CASE_DEFAULT_TIMEOUT_MS}"
        return 0
    fi
    # Strip leading zeros so bash never parses the value as octal.
    value="$((10#${value}))"
    if (( value < PAM_CASE_MIN_TIMEOUT_MS )); then
        value="${PAM_CASE_MIN_TIMEOUT_MS}"
    elif (( value > PAM_CASE_MAX_TIMEOUT_MS )); then
        value="${PAM_CASE_MAX_TIMEOUT_MS}"
    fi
    echo "${value}"
}

deadline_bound_ms() {
    echo "$(( $1 + PAM_CASE_DEADLINE_TOLERANCE_MS ))"
}

timeout_mock_delay_ms() {
    echo "$(( $1 + PAM_CASE_DEADLINE_TOLERANCE_MS + PAM_CASE_MOCK_DELAY_MARGIN_MS ))"
}

now_ms() {
    echo "$(( $(date +%s%N) / 1000000 ))"
}

assert_elapsed_within_deadline() {
    local label="$1" elapsed="$2" timeout="$3"
    local bound
    bound="$(deadline_bound_ms "${timeout}")"
    if (( elapsed > bound )); then
        echo "[FAIL]  ${label}: PAM run took ${elapsed} ms, above timeout_ms=${timeout} + ${PAM_CASE_DEADLINE_TOLERANCE_MS} ms tolerance (${bound} ms): the module did not honor its deadline." >&2
        return 1
    fi
    echo "[OK]    ${label}: PAM run took ${elapsed} ms (bound ${bound} ms for timeout_ms=${timeout})."
}

# A run WITHOUT password must fail: pam_soos.so returned PAM_IGNORE, so pam_unix
# reached its prompt and the non-interactive conversation refused it.
assert_no_facial_authorization() {
    local service="$1" label="$2"
    if /usr/local/bin/pam_test_runner "${service}" testuser 2>/dev/null; then
        echo "[FAIL]  ${label}: ${service} authenticated WITHOUT a password (the verdict was honored as an authorization)." >&2
        return 1
    fi
    echo "[OK]    ${label}: no facial authorization on ${service} (password prompt reached)."
}

# The valid password is accepted and a wrong password is rejected.
assert_password_fallback() {
    local service="$1" label="$2"
    if ! /usr/local/bin/pam_test_runner "${service}" testuser password123; then
        echo "[FAIL]  ${label}: ${service} rejected the valid password." >&2
        return 1
    fi
    if /usr/local/bin/pam_test_runner "${service}" testuser wrong_password 2>/dev/null; then
        echo "[FAIL]  ${label}: ${service} accepted a wrong password!" >&2
        return 1
    fi
    echo "[OK]    ${label}: ${service} accepted the valid password and rejected a wrong one."
}
