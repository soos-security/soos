#!/usr/bin/env bash
# =============================================================================
# scripts/android_build.sh — Build and test the soos native Android app
# =============================================================================
# GitHub #382, ADR 2026-10-13 "Native Android App for `soos-remote` (Kotlin and
# Jetpack Compose)". Needs a JDK 17 and the Android SDK (ANDROID_HOME); never
# installs either. The Gradle distribution is the one pinned in
# android/gradle/wrapper/gradle-wrapper.properties: it is downloaded over HTTPS
# and refused unless its SHA-256 equals distributionSha256Sum. No gradlew and no
# wrapper jar are committed.
#
# Usage: scripts/android_build.sh [verify|bootstrap|ci]
#   verify     (default) debug build, unit tests and lint with Gradle dependency
#              verification strict (every artifact must match
#              android/gradle/verification-metadata.xml)
#   bootstrap  same tasks, but (re)writes the SHA-256 verification metadata
#              (trust on first use: review the diff before committing it)
#   ci         verify, then the mandatory APK permission check; when the
#              metadata lists no component, writes it, records bootstrap=true in
#              $GITHUB_OUTPUT and FAILS so that an unverified build never passes
#
# Exit codes: 0 success, 1 failure (including a bootstrap run in ci mode),
#             2 usage error.
# =============================================================================

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
readonly ANDROID_DIR="${ROOT}/android"
readonly PROPS="${ANDROID_DIR}/gradle/wrapper/gradle-wrapper.properties"
readonly METADATA="${ANDROID_DIR}/gradle/verification-metadata.xml"
readonly CACHE="${SOOS_ANDROID_CACHE:-${HOME}/.cache/soos-android}"
readonly TASKS=(assembleDebug testDebugUnitTest lintDebug)
readonly BUILD_TOOLS_VERSION="35.0.0"
readonly APK="${ANDROID_DIR}/app/build/outputs/apk/debug/app-debug.apk"
# The only permissions the merged manifest may request: INTERNET, the
# signature-level receiver permission androidx.core declares for the app itself,
# and the two install-time (normal) permissions androidx.biometric 1.1.0 declares as
# a compile dependency of androidx.credentials 1.5.0 (owner decision 2026-10-15; the
# app never calls androidx.biometric, and neither permission grants data access).
readonly ALLOWED_PERMISSIONS=(
    "android.permission.INTERNET"
    "org.soos.remote.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION"
    "android.permission.USE_BIOMETRIC"
    "android.permission.USE_FINGERPRINT"
)

fail() {
    echo "[FAIL]  android_build: $*" >&2
    exit 1
}

MODE="${1:-verify}"
case "$MODE" in
    verify|bootstrap|ci) ;;
    *) echo "usage: scripts/android_build.sh [verify|bootstrap|ci]" >&2; exit 2 ;;
esac

property() {
    local key="$1" line
    line="$(grep -E "^${key}=" "$PROPS" | head -n 1)" || fail "${key} missing in ${PROPS}"
    # Java properties escape ':' as '\:'.
    printf '%s' "${line#*=}" | sed 's/\\:/:/g'
}

URL="$(property distributionUrl)"
SHA="$(property distributionSha256Sum)"
[[ "$URL" =~ ^https://services\.gradle\.org/distributions/gradle-[0-9]+\.[0-9]+(\.[0-9]+)?-bin\.zip$ ]] \
    || fail "distributionUrl is not a pinned services.gradle.org -bin.zip"
[[ "$SHA" =~ ^[0-9a-f]{64}$ ]] || fail "distributionSha256Sum is not a SHA-256"

ZIP_NAME="${URL##*/}"
VERSION_DIR="${ZIP_NAME%-bin.zip}"
DIST_DIR="${CACHE}/${SHA}"
ZIP="${DIST_DIR}/${ZIP_NAME}"
GRADLE="${DIST_DIR}/${VERSION_DIR}/bin/gradle"

mkdir -p "$DIST_DIR"
if [[ ! -x "$GRADLE" ]]; then
    if [[ ! -f "$ZIP" ]] || ! echo "${SHA}  ${ZIP}" | sha256sum --check --status; then
        rm -f "$ZIP" "${ZIP}.part"
        curl --fail --silent --show-error --location \
            --proto '=https' --proto-redir '=https' --max-redirs 5 --tlsv1.2 \
            --connect-timeout 20 --max-time 600 --retry 3 \
            --output "${ZIP}.part" "$URL"
        mv "${ZIP}.part" "$ZIP"
    fi
    if ! echo "${SHA}  ${ZIP}" | sha256sum --check --status; then
        rm -f "$ZIP"
        fail "Gradle distribution SHA-256 mismatch (file deleted)"
    fi
    rm -rf "${DIST_DIR:?}/${VERSION_DIR}"
    unzip -q "$ZIP" -d "$DIST_DIR"
fi
[[ -x "$GRADLE" ]] || fail "Gradle executable missing after unpacking"

cd "$ANDROID_DIR"

run_strict() {
    "$GRADLE" --no-daemon --stacktrace --dependency-verification strict "${TASKS[@]}"
}

run_bootstrap() {
    "$GRADLE" --no-daemon --stacktrace --write-verification-metadata sha256 "${TASKS[@]}"
}

check_permissions() {
    local aapt2="${ANDROID_HOME:-}/build-tools/${BUILD_TOOLS_VERSION}/aapt2"
    [[ -n "${ANDROID_HOME:-}" && -x "$aapt2" ]] || fail "aapt2 ${BUILD_TOOLS_VERSION} not found under ANDROID_HOME"
    [[ -f "$APK" ]] || fail "debug APK missing: ${APK}"
    local dump line name allowed permission found=0
    # aapt2 prints a <uses-permission> request as `uses-permission: name='X'` and a
    # <uses-permission-sdk-23> request as `uses-permission-sdk-23: name='X'` (either may be
    # followed by attributes such as maxSdkVersion). Both reach the allowlist; any other
    # `uses-permission*` line, indented or not, fails closed.
    local re="^uses-permission(-sdk-23)?: name='([^']+)'"
    dump="$("$aapt2" dump permissions "$APK")" || fail "aapt2 dump permissions failed"
    while IFS= read -r line; do
        [[ "$line" =~ ^[[:space:]]*uses-permission ]] || continue
        [[ "$line" =~ $re ]] || fail "unrecognized aapt2 permission line: ${line}"
        name="${BASH_REMATCH[2]}"
        allowed=0
        for permission in "${ALLOWED_PERMISSIONS[@]}"; do
            if [[ "$name" == "$permission" ]]; then
                allowed=1
            fi
        done
        [[ "$allowed" -eq 1 ]] || fail "unexpected permission in the APK: ${name}"
        if [[ "$name" == "android.permission.INTERNET" ]]; then
            found=1
        fi
    done <<< "$dump"
    [[ "$found" -eq 1 ]] || fail "android.permission.INTERNET not found (unexpected aapt2 output)"
    echo "[OK]    android_build: APK permissions are INTERNET plus the allowlisted library permissions"
}

case "$MODE" in
    verify)
        run_strict
        ;;
    bootstrap)
        run_bootstrap
        echo "[WARN]  android_build: verification metadata rewritten; review and commit android/gradle/verification-metadata.xml"
        ;;
    ci)
        if ! grep -q '<component ' "$METADATA"; then
            run_bootstrap
            if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
                echo "bootstrap=true" >> "$GITHUB_OUTPUT"
            fi
            echo "::error::Gradle dependency verification metadata is empty: this bootstrap run generated it (artifact android-verification-metadata). Review it, commit it on the branch and re-run; an unverified build never passes."
            exit 1
        fi
        run_strict
        check_permissions
        ;;
esac
