#!/usr/bin/env bash
# =============================================================================
# scripts/install_remote.sh — per-user installation of the soos remote companion
# =============================================================================
# Installs `soos-remote` (GitHub #339) and the optional Web Push sender `soos-push-sender`
# (ADR 2026-10-06) for the current user only: both binaries under ~/.local/bin, both user
# units under the systemd user directory and a configuration
# template (only when absent) that refuses to start until the owner fills in the
# Tailscale login. Never runs as root, never escalates privileges, never enables the
# units and never touches the Tailscale configuration: those steps are printed for the
# owner to run.
#
#   scripts/install_remote.sh              install or update
#   scripts/install_remote.sh --uninstall  stop the units, remove binaries and units, keep config
# =============================================================================

set -euo pipefail

if [[ "${EUID}" -eq 0 ]]; then
    echo "install_remote.sh: refusing to run as root; run it as the session owner" >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
BIN_DIR="${HOME}/.local/bin"
BIN_PATH="${BIN_DIR}/soos-remote"
UNIT_SRC="${REPO_ROOT}/packaging/soos-remote.service"
UNIT_PATH="${CONFIG_HOME}/systemd/user/soos-remote.service"
SENDER_BIN_PATH="${BIN_DIR}/soos-push-sender"
SENDER_UNIT_SRC="${REPO_ROOT}/packaging/soos-push-sender.service"
SENDER_UNIT_PATH="${CONFIG_HOME}/systemd/user/soos-push-sender.service"
CONFIG_DIR="${CONFIG_HOME}/soos"
CONFIG_PATH="${CONFIG_HOME}/soos/remote.toml"
RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
SOCKET_PATH="${RUNTIME_DIR}/soos-remote/remote.sock"

usage() {
    echo "usage: scripts/install_remote.sh [--uninstall]"
}

uninstall() {
    echo "Stopping and disabling the user units (if present)..."
    systemctl --user stop soos-remote.service 2>/dev/null || true
    systemctl --user disable soos-remote.service 2>/dev/null || true
    systemctl --user stop soos-push-sender.service 2>/dev/null || true
    systemctl --user disable soos-push-sender.service 2>/dev/null || true
    rm -f "${UNIT_PATH}" "${BIN_PATH}" "${SENDER_UNIT_PATH}" "${SENDER_BIN_PATH}"
    systemctl --user daemon-reload 2>/dev/null || true
    echo "Removed ${BIN_PATH}, ${UNIT_PATH}, ${SENDER_BIN_PATH} and ${SENDER_UNIT_PATH}."
    echo "The configuration ${CONFIG_PATH} and the stores next to it were kept."
}

install_companion() {
    echo "Building soos-remote and soos-push-sender (release, locked)..."
    (cd "${REPO_ROOT}" && cargo build --release --locked -p soos-remote -p soos-push-sender)

    echo "Installing the binaries to ${BIN_DIR}..."
    install -Dm755 "${REPO_ROOT}/target/release/soos-remote" "${BIN_PATH}"
    install -Dm755 "${REPO_ROOT}/target/release/soos-push-sender" "${SENDER_BIN_PATH}"

    echo "Installing the user units to ${CONFIG_HOME}/systemd/user/..."
    install -Dm644 "${UNIT_SRC}" "${UNIT_PATH}"
    # packaging/soos-push-sender.service: installed, never enabled by this script.
    install -Dm644 "${SENDER_UNIT_SRC}" "${SENDER_UNIT_PATH}"

    if [[ ! -e "${CONFIG_PATH}" ]]; then
        echo "Writing the configuration template to ${CONFIG_PATH} (mode 0600)..."
        umask 077
        mkdir -p "${CONFIG_DIR}"
        cat > "${CONFIG_PATH}" <<'TEMPLATE'
# soos-remote configuration (GitHub #339). English only.
#
# Tailscale logins allowed to use the page (1..=8). The value is the Tailscale-User-Login
# header set by the Tailscale Serve proxy, usually the account e-mail. The service refuses to
# start while this list is empty.
allowed_logins = []

# Optional: DNS names accepted in the Host header (0..=4). When absent, any syntactically
# valid *.ts.net name is accepted.
# allowed_hosts = ["mypc.tail1234.ts.net"]

# Optional: logind polling interval while a stream is open, in ms (250..=10000).
# poll_interval_ms = 1000

# Optional: Unix socket path (default: $XDG_RUNTIME_DIR/soos-remote/remote.sock).
# socket_path = "/run/user/1000/soos-remote/remote.sock"

# Passkeys (Face ID, fingerprint or screen lock) and internet access through Tailscale Funnel
# (Docs/REMOTE_COMPANION.md section 2b). rp_id is the full node name; without it every
# remote unlock is refused.
# rp_id = "mypc.tail1234.ts.net"
# Accept requests from tailscale funnel (port 443 only; requires rp_id).
# allow_funnel = false
# Passkey store (default: remote-passkeys.json next to this file).
# credentials_path = "/srv/soos/remote-passkeys.json"

# Optional: remote unlock, always with a fresh passkey assertion (section 2a).
# allow_unlock = false

# Optional: failed-password alerts from the system journal (section 2c). Counts and classes
# only, never the typed password.
# password_alerts = false

# Optional: Web Push notifications of those alerts to the phone web app (section 2d; Android: section 2h).
# Requires password_alerts = true and rp_id, and the soos-push-sender user unit.
# push_notifications = false
# Contact of the VAPID key (default: https://<rp_id>); a mailto: address also works.
# vapid_subject = "mailto:you@example.com"
# What the phone lock screen shows: "detailed" (source, account, count) or "generic".
# push_previews = "detailed"

# Optional: live camera view on the phone (section 2f). Needs rp_id, a fresh passkey check per view,
# and, on the PC, the [preview] section of the daemon configuration must allow your uid and
# the remote view (an administrator change, printed by the installer). No recording.
# camera_view = false
# Also over Tailscale Funnel (requires camera_view and allow_funnel).
# camera_view_funnel = false
# Longest view in seconds (10..=300).
# camera_max_view_s = 120
# Frames per second (1..=10).
# camera_fps = 5
# Output width: 640 (source size) or 320 (half).
# camera_width = 640
# JPEG quality (50..=85).
# camera_quality = 70
# Battery level of the PC on the page (section 2g): level, charge state and mains only.
# battery_status = true
TEMPLATE
        chmod 0600 "${CONFIG_PATH}"
    else
        echo "Keeping the existing configuration ${CONFIG_PATH}."
    fi

    echo "Reloading the user manager..."
    systemctl --user daemon-reload

    echo
    echo "Next steps (run them yourself, no root needed):"
    echo "  1. Put your Tailscale login in ${CONFIG_PATH}: allowed_logins = [\"you@example.com\"]"
    echo "  2. Start the service: systemctl --user enable --now soos-remote"
    echo "  3. Publish the socket on your tailnet: tailscale serve --bg unix:${SOCKET_PATH}"
    echo "  4. Open https://<this-pc>.<tailnet>.ts.net on the phone and add it to the home screen."
    echo "  5. Optional passkeys and internet access (Docs/REMOTE_COMPANION.md section 2b):"
    echo "     set rp_id = \"<this-pc>.<tailnet>.ts.net\" in ${CONFIG_PATH}, restart the service,"
    echo "     run soos-remote enroll-code and register the phone's passkey over the tailnet,"
    echo "     then optionally set allow_funnel = true, restart, and publish on port 443 with:"
    echo "     tailscale funnel --bg unix:${SOCKET_PATH}"
    echo "  6. Optional push notifications on the phone (Docs/REMOTE_COMPANION.md section 2d):"
    echo "     set password_alerts = true and push_notifications = true, then run"
    echo "     systemctl --user enable --now soos-push-sender and restart soos-remote,"
    echo "     open the web app on the phone (the home-screen app on iPhone) and tap Enable notifications."
    echo "  7. Optional live camera view (Docs/REMOTE_COMPANION.md section 2f): set camera_view = true"
    echo "     in ${CONFIG_PATH} and restart soos-remote; an administrator must also add to"
    echo "     /etc/soos/daemon.toml and restart soos-daemon (this script never does it):"
    echo "       [preview]"
    echo "       enabled = true"
    echo "       allowed_uids = [$(id -u)]"
    echo "       remote_view = true"
    echo "Never use tailscale serve --http, nor tailscale funnel without rp_id and allow_funnel (see Docs/REMOTE_COMPANION.md)."
    warn_resolver_stub
}

# The push sender's sandbox hides /run (TemporaryFileSystem=/run:ro), so a resolv.conf that
# points into /run/systemd/resolve/ leaves it without DNS: every push would fail. Read-only
# check, prints a warning only; the system configuration is never changed.
warn_resolver_stub() {
    local target
    target="$(readlink -f /etc/resolv.conf 2>/dev/null || true)"
    if [[ "${target}" == /run/* ]]; then
        echo
        echo "WARNING: /etc/resolv.conf resolves to ${target}, which the soos-push-sender"
        echo "sandbox hides (/run is not visible to it). Push notifications would fail to resolve"
        echo "the push service. See Docs/REMOTE_COMPANION.md section 2d (Limits) before enabling"
        echo "push_notifications."
    fi
}

case "${1:-}" in
    "")
        install_companion
        ;;
    --uninstall)
        uninstall
        ;;
    -h|--help)
        usage
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
