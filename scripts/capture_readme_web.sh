#!/usr/bin/env bash
# =============================================================================
# scripts/capture_readme_web.sh — optional README captures of the phone web app
# =============================================================================
# GitHub #381 (PR 1 of #356); ADR 2026-10-13 "README Assets Are Generated From
# Synthetic Data, Never Hand-Edited"; Docs/README_ASSETS.md.
#
# Serves crates/remote/assets and canned, identity-free JSON from the
# loopback-only fixture server (tests/fixtures/examples/readme_web_fixture_server.rs;
# never the real companion), opens it in an already installed headless
# Chrome/Chromium with a throw-away profile, and writes
# Docs/assets/screenshots/web-<scene>-<light|dark>.png at 390x844 CSS pixels,
# device scale factor 2, with every PNG metadata chunk stripped.
#
# Optional and never a CI requirement: nothing is installed or downloaded.
# The browser binary is taken from $CHROME or found on PATH. Run
# scripts/capture_readme_assets.sh afterwards (or with --web) to refresh
# Docs/assets/MANIFEST.toml.
#
# Usage: scripts/capture_readme_web.sh
# =============================================================================

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/Docs/assets/screenshots"
SCENES=(unlocked locked-alerts)
VIEWPORT="390,844"
SCALE=2
SETTLE_MS=4000

browser="${CHROME:-}"
if [[ -z "$browser" ]]; then
    for candidate in chromium chromium-browser google-chrome-stable google-chrome; do
        if command -v "$candidate" > /dev/null 2>&1; then
            browser="$(command -v "$candidate")"
            break
        fi
    done
fi
if [[ -z "$browser" ]]; then
    echo "capture_readme_web.sh: no Chrome/Chromium found (set CHROME=...); skipping" >&2
    exit 3
fi

cd "$ROOT"
cargo build --locked -q -p soos-test-fixtures --example readme_web_fixture_server

work="$(mktemp -d)"
server_pid=""
cleanup() {
    if [[ -n "$server_pid" ]]; then
        kill "$server_pid" 2> /dev/null || true
        wait "$server_pid" 2> /dev/null || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT

mkdir -p "$OUT"
for scene in "${SCENES[@]}"; do
    portfile="$work/port-$scene"
    cargo run --locked -q -p soos-test-fixtures --example readme_web_fixture_server -- \
        --scene "$scene" --assets "$ROOT/crates/remote/assets" --serve-ms 60000 > "$portfile" &
    server_pid=$!
    port=""
    for _ in $(seq 1 100); do
        port="$(sed -n 's/^PORT=\([0-9]\{1,5\}\)$/\1/p' "$portfile" 2> /dev/null || true)"
        [[ -n "$port" ]] && break
        sleep 0.1
    done
    if [[ -z "$port" ]]; then
        echo "capture_readme_web.sh: the fixture server did not start" >&2
        exit 1
    fi
    for variant in light dark; do
        profile="$(mktemp -d "$work/profile.XXXXXX")"
        raw="$work/$scene-$variant.png"
        flags=(--headless=new --disable-gpu --no-first-run --no-default-browser-check
            --disable-extensions --disable-sync --disable-background-networking
            --disable-component-update --hide-scrollbars --user-data-dir="$profile"
            --window-size="$VIEWPORT" --force-device-scale-factor="$SCALE"
            --virtual-time-budget="$SETTLE_MS" --screenshot="$raw")
        if [[ "$variant" == "dark" ]]; then
            flags+=(--force-dark-mode --blink-settings=preferredColorScheme=0)
        else
            flags+=(--blink-settings=preferredColorScheme=1)
        fi
        "$browser" "${flags[@]}" "http://127.0.0.1:$port/" > /dev/null 2>&1
        cargo run --locked -q -p soos-test-fixtures --example readme_web_fixture_server -- \
            --strip-png "$raw" "$OUT/web-$scene-$variant.png"
        echo "==> $OUT/web-$scene-$variant.png"
    done
    kill "$server_pid" 2> /dev/null || true
    wait "$server_pid" 2> /dev/null || true
    server_pid=""
done
