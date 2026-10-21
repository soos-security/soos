#!/usr/bin/env bash
# =============================================================================
# scripts/remote_browser_tests.sh — Playwright bench of the soos-remote web app
# =============================================================================
# GitHub #464 (PR 2 of #355); ADR 2026-10-13 "Browser Support Policy for the
# `soos-remote` Web App", amendment of 2026-10-21; Docs/CI_CD_AND_SECURITY.md.
#
# Runs tests/browser on Chromium, Firefox and WebKit inside the official
# Playwright image, pinned by digest (the image carries Node and the browser
# builds of exactly this Playwright release). Two container runs:
#   1. `npm ci --ignore-scripts` of tests/browser/package-lock.json (integrity
#      hashes checked by npm; no lifecycle script runs, no browser download).
#      This is the only step with network, and it fetches pinned packages only.
#   2. `playwright test` with `--network none`, as the calling user, with every
#      capability dropped: the pages come from the loopback-only fixture server
#      of tests/fixtures (synthetic scenes, fixed clock), started inside the
#      container from the host binary, never from the owner's data.
# No secret, token or credential is read or passed. Dev-only: nothing here is
# built into a shipped artifact.
#
# Usage: scripts/remote_browser_tests.sh [--fixture-server <path>] [-- <playwright args>]
#   Without --fixture-server, the fixture server (the single example of the
#   soos-test-fixtures crate) is built with cargo (--locked). Extra arguments
#   go to `playwright test` (for example `-- --project=firefox`).
# Exit status: the status of `playwright test`; 2 for a usage or setup error.
# =============================================================================

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
readonly BENCH="$ROOT/tests/browser"
readonly ASSETS="$ROOT/crates/remote/assets"
# Playwright 1.64.0 on Ubuntu 24.04 (Node 24.21.0, npm 11.19.0, chromium-1248,
# firefox-1555, webkit-2370). Bump together with tests/browser/package.json and
# package-lock.json; the digest is the multi-architecture index digest.
readonly PLAYWRIGHT_IMAGE="mcr.microsoft.com/playwright:v1.64.0-noble@sha256:06a9939e57531807f8d5fd76ce44b53165ffb7d7501d87ab10e285c20b1e971f"
readonly CONTAINER_FIXTURE="/opt/soos/fixture-server"

usage_error() {
    echo "[FAIL]  remote_browser_tests: $*" >&2
    echo "usage: scripts/remote_browser_tests.sh [--fixture-server <path>] [-- <playwright args>]" >&2
    exit 2
}

fixture=""
playwright_args=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --fixture-server)
            [[ $# -ge 2 ]] || usage_error "--fixture-server needs a path"
            fixture="$2"
            shift 2
            ;;
        --)
            shift
            playwright_args=("$@")
            break
            ;;
        *)
            usage_error "unknown argument '$1'"
            ;;
    esac
done

command -v docker > /dev/null 2>&1 || usage_error "docker is required"
docker info > /dev/null 2>&1 || usage_error "the docker daemon is not reachable"

# The loopback fixture server is the single example of the soos-test-fixtures
# crate. RAC15 (tests/invariants readme_assets_contract) reserves its name, in
# scripts/ and .github/, to the two README capture scripts, so this script
# takes the executable from cargo's own build report and refuses to guess when
# the crate has any other number of examples.
if [[ -z "$fixture" ]]; then
    echo "[INFO]  building the loopback fixture server (soos-test-fixtures example)"
    report="$(cargo build --locked -p soos-test-fixtures --examples \
        --message-format=json-render-diagnostics)" \
        || usage_error "cargo could not build the soos-test-fixtures examples"
    built=()
    while IFS= read -r line; do
        if [[ -n "$line" ]]; then
            built+=("$line")
        fi
    done < <(printf '%s\n' "$report" \
        | grep -F '"reason":"compiler-artifact"' \
        | grep -F '"kind":["example"]' \
        | sed -n 's/.*"executable":"\([^"]*\)".*/\1/p')
    [[ ${#built[@]} -eq 1 ]] \
        || usage_error "expected one soos-test-fixtures example executable, cargo reported ${#built[@]}"
    fixture="${built[0]}"
fi
[[ -f "$fixture" && -x "$fixture" ]] || usage_error "fixture server '$fixture' is not an executable file"
fixture="$(cd "$(dirname "$fixture")" && pwd)/$(basename "$fixture")"

user_args=(--user "$(id -u):$(id -g)" -e HOME=/tmp -e npm_config_cache=/tmp/npm-cache)

echo "[INFO]  npm ci (pinned lockfile, no lifecycle scripts) in $PLAYWRIGHT_IMAGE"
docker run --rm "${user_args[@]}" \
    -e PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 \
    -v "$BENCH:/work/tests/browser" -w /work/tests/browser \
    "$PLAYWRIGHT_IMAGE" \
    npm ci --ignore-scripts --no-audit --no-fund --no-update-notifier

echo "[INFO]  playwright test (no network, loopback fixture server only)"
status=0
docker run --rm "${user_args[@]}" \
    --network none --init --shm-size=1g \
    --cap-drop ALL --security-opt no-new-privileges \
    -e CI=1 -e SOOS_FIXTURE_SERVER="$CONTAINER_FIXTURE" \
    -v "$fixture:$CONTAINER_FIXTURE:ro" \
    -v "$ASSETS:/work/crates/remote/assets:ro" \
    -v "$BENCH:/work/tests/browser" -w /work/tests/browser \
    "$PLAYWRIGHT_IMAGE" \
    npx --no-install playwright test "${playwright_args[@]}" || status=$?
exit "$status"
