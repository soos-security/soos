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
# The browser first opens the fixture page /readme/tour-seen, which marks the
# web app's first-run feature tour as seen and loads / (GitHub #448). A second
# run in the same profile takes the screenshot of /readme/capture: index.html
# with the page clock frozen at the fixture's fixed capture instant, while the
# server builds every body from that same instant (--clock fixed), so every
# scene and theme shows the same date and times. Every browser run has a pinned time zone
# and locale (BROWSER_ENV, --lang=en-US, --accept-lang=en-US; GitHub #458).
# Every browser run is bounded: it runs in the background and await_browser waits at most
# BROWSER_TIMEOUT_S seconds (coreutils timeout), then stops it (SIGTERM, SIGKILL after
# BROWSER_KILL_AFTER_S seconds) and fails the whole script with status 124.
# Every browser run is its own process group (job control on for the background start), and
# the whole group (renderer, GPU, zygote and utility children) is ended after the run, on a
# timeout and from the exit trap; INT and TERM exit through that trap, so no browser process
# outlives the script.
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
# Pinned time zone and locale of every browser run, so clock times (12/24 h) and month names
# never depend on the capture host (GitHub #458); --lang/--accept-lang below pin the browser's
# own locale and Accept-Language the same way.
BROWSER_ENV=(TZ=UTC LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 LANGUAGE=en_US)
# Upper bound of one browser run. A normal run ends within seconds (SETTLE_MS of virtual time),
# but headless Chrome was seen to hang without ever writing the screenshot. Four runs per
# scene, each at most BROWSER_TIMEOUT_S + BROWSER_KILL_AFTER_S seconds, stay within the
# fixture server's 240 s lifetime (--serve-ms 240000): 4 x (45 + 5) = 200 s.
BROWSER_TIMEOUT_S=45
BROWSER_KILL_AFTER_S=5

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
browser_pid=""
watch_pid=""

# Ends whatever is left of the process group $1 of a browser run: Chrome forks renderer, GPU,
# zygote and utility children that can outlive its main process. SIGTERM, then SIGKILL when a
# member is still there BROWSER_KILL_AFTER_S seconds later; a no-op once the group is empty.
# Called only after the main process (the group leader) has been reaped.
end_browser_group() {
    local pgid="$1" tick=0
    kill -TERM -- "-$pgid" 2> /dev/null || return 0
    while kill -0 -- "-$pgid" 2> /dev/null; do
        if [[ "$tick" -ge $((BROWSER_KILL_AFTER_S * 5)) ]]; then
            kill -KILL -- "-$pgid" 2> /dev/null || true
            return 0
        fi
        sleep 0.2
        tick=$((tick + 1))
    done
}

# Stops the browser run in the background, if any: SIGTERM to its whole process group, then
# SIGKILL when the main process is still running BROWSER_KILL_AFTER_S seconds later, and ends
# every child left in the group. The plain pid is only a fallback.
stop_browser() {
    if [[ -z "$browser_pid" ]]; then
        return 0
    fi
    kill -TERM -- "-$browser_pid" 2> /dev/null || kill -TERM "$browser_pid" 2> /dev/null || true
    if ! timeout "${BROWSER_KILL_AFTER_S}s" tail --pid="$browser_pid" --sleep-interval=0.2 -f /dev/null; then
        kill -KILL -- "-$browser_pid" 2> /dev/null || kill -KILL "$browser_pid" 2> /dev/null || true
    fi
    wait "$browser_pid" 2> /dev/null || true
    end_browser_group "$browser_pid"
    browser_pid=""
}

# Waits for the browser run started in the background ($browser_pid) for at most
# BROWSER_TIMEOUT_S seconds. On expiry the run is stopped and the script fails loudly with
# status 124 (a killed Chrome may itself exit 0); a failing run fails the script too.
# Job control was switched on (set -m) only so that the run got its own process group; it is
# switched off again first. The timeout watcher runs in the background and is awaited with the
# wait builtin, which INT and TERM interrupt at once (the run no longer shares the terminal's
# process group, so it no longer gets the terminal's Ctrl-C itself).
await_browser() {
    set +m
    local label="$1"
    timeout "${BROWSER_TIMEOUT_S}s" tail --pid="$browser_pid" --sleep-interval=0.2 -f /dev/null &
    watch_pid=$!
    if ! wait "$watch_pid"; then
        watch_pid=""
        echo "capture_readme_web.sh: the browser run ($label) did not finish within ${BROWSER_TIMEOUT_S} s; stopping it" >&2
        stop_browser
        exit 124
    fi
    watch_pid=""
    local status=0
    wait "$browser_pid" || status=$?
    end_browser_group "$browser_pid"
    browser_pid=""
    if [[ "$status" -ne 0 ]]; then
        echo "capture_readme_web.sh: the browser run ($label) failed with status $status" >&2
        exit "$status"
    fi
}

cleanup() {
    if [[ -n "$watch_pid" ]]; then
        kill "$watch_pid" 2> /dev/null || true
        wait "$watch_pid" 2> /dev/null || true
    fi
    stop_browser
    if [[ -n "$server_pid" ]]; then
        kill "$server_pid" 2> /dev/null || true
        wait "$server_pid" 2> /dev/null || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

mkdir -p "$OUT"
for scene in "${SCENES[@]}"; do
    portfile="$work/port-$scene"
    cargo run --locked -q -p soos-test-fixtures --example readme_web_fixture_server -- \
        --scene "$scene" --assets "$ROOT/crates/remote/assets" --serve-ms 240000 --clock fixed > "$portfile" &
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
            --lang=en-US --accept-lang=en-US --virtual-time-budget="$SETTLE_MS")
        if [[ "$variant" == "dark" ]]; then
            flags+=(--force-dark-mode --blink-settings=preferredColorScheme=0)
        else
            flags+=(--blink-settings=preferredColorScheme=1)
        fi
        # The fixture page marks the first-run feature tour as seen in this throw-away
        # profile, then loads / (GitHub #448), so the tour never covers the capture.
        # BROWSER_ENV (TZ=UTC, en_US locale) keeps the capture host's time zone and locale out
        # of the clock times and dates shown.
        # Each run starts in the background and is bounded by await_browser. Job control is on
        # for the start only (await_browser switches it off), so the run's pid is the process
        # group id of Chrome and of every child it forks; stdin is /dev/null because a job
        # control background job keeps the terminal otherwise.
        set -m
        env "${BROWSER_ENV[@]}" "$browser" "${flags[@]}" "http://127.0.0.1:$port/readme/tour-seen" < /dev/null > /dev/null 2>&1 &
        browser_pid=$!
        await_browser "tour-seen page, $scene, $variant"
        # Same profile, page clock frozen at the fixed capture instant: the screenshot.
        set -m
        env "${BROWSER_ENV[@]}" "$browser" "${flags[@]}" --screenshot="$raw" \
            "http://127.0.0.1:$port/readme/capture" < /dev/null > /dev/null 2>&1 &
        browser_pid=$!
        await_browser "screenshot, $scene, $variant"
        cargo run --locked -q -p soos-test-fixtures --example readme_web_fixture_server -- \
            --strip-png "$raw" "$OUT/web-$scene-$variant.png"
        echo "==> $OUT/web-$scene-$variant.png"
    done
    kill "$server_pid" 2> /dev/null || true
    wait "$server_pid" 2> /dev/null || true
    server_pid=""
done
