#!/usr/bin/env bash
# =============================================================================
# scripts/install_remote.sh — per-user installation of the soos remote companion
# =============================================================================
# Installs `soos-remote` (GitHub #339) for the current user only: the binary under
# ~/.local/bin, the user unit under the systemd user directory and a configuration
# template (only when absent) that refuses to start until the owner fills in the
# Tailscale login. Never runs as root, never escalates privileges, never enables the
# unit and never touches the Tailscale configuration: those steps are printed for the
# owner to run.
#
#   scripts/install_remote.sh              install or update
#   scripts/install_remote.sh --uninstall  stop the unit, remove binary and unit, keep config
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
CONFIG_DIR="${CONFIG_HOME}/soos"
CONFIG_PATH="${CONFIG_HOME}/soos/remote.toml"
RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
SOCKET_PATH="${RUNTIME_DIR}/soos-remote/remote.sock"

usage() {
    echo "usage: scripts/install_remote.sh [--uninstall]"
}

uninstall() {
    echo "Stopping and disabling the user unit (if present)..."
    systemctl --user stop soos-remote.service 2>/dev/null || true
    systemctl --user disable soos-remote.service 2>/dev/null || true
    rm -f "${UNIT_PATH}" "${BIN_PATH}"
    systemctl --user daemon-reload 2>/dev/null || true
    echo "Removed ${BIN_PATH} and ${UNIT_PATH}."
    echo "The configuration ${CONFIG_PATH} was kept."
}

install_companion() {
    echo "Building soos-remote (release, locked)..."
    (cd "${REPO_ROOT}" && cargo build --release --locked -p soos-remote)

    echo "Installing the binary to ${BIN_PATH}..."
    install -Dm755 "${REPO_ROOT}/target/release/soos-remote" "${BIN_PATH}"

    echo "Installing the user unit to ${UNIT_PATH}..."
    install -Dm644 "${UNIT_SRC}" "${UNIT_PATH}"

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
    echo "Never use tailscale serve --http or tailscale funnel for this socket (see Docs/REMOTE_COMPANION.md)."
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
