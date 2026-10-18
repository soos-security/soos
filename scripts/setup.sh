#!/usr/bin/env bash
# =============================================================================
# scripts/setup.sh — One-command setup of soos from a source checkout
# =============================================================================
# GitHub #372 (PR 1 of #362); ADR 2026-10-13 "One-Command Setup `scripts/setup.sh`
# and Default-On Installer Profile"; Docs/PACKAGING_AND_PROVISIONING.md §3.0.
# Packages later ship this script as /usr/bin/soos-setup.
#
# Orchestrates the existing hardened scripts, never replaces them:
#   detect      distribution (os-release parsed, never sourced), package manager,
#               init, display manager, Tailscale version, session (local seat or
#               not), camera nodes, build user and target user
#   plan        every phase with "will run", "already done" or "skipped"
#   packages    missing packages of check_build_deps.sh --print-packages
#               build|models|gui, through the native package manager (root)
#   toolchain   scripts/install_rustup.sh when cargo is missing (build user)
#   build       cargo build --release --locked --workspace (build user, never root)
#   system      scripts/install.sh --start (root; its own journal and rollback)
#   access      soos-admin add-user <user> (root)
#   presence    --presence on|off: <sysconfdir>/soos/presence.disable (root)
#   verify      scripts/wait_daemon_ready.sh and soos-admin camera list (root)
#   enrollment  daemon stopped, soos-enroll enroll --username <user>, daemon
#               started again; from a local seat only (root)
#   state       <localstatedir>/lib/soos/state/setup.json, bounded, no secret (root)
#   preview     camera view, daemon side: <sysconfdir>/soos/daemon.toml [preview]
#               created only when the file is absent (root)
#   companion   phone companion as the session owner (never root): remote.toml
#               written with the owner's default-on features, scripts/install_remote.sh,
#               user units, verification; Tailscale steps are printed, never run
#
# Started as a normal user, it escalates with sudo (or doas) twice at most: once for
# the package manager, once to re-execute itself with --system-phase (sudo resets
# the environment, so everything the root half needs is passed as an argument).
# Started as root, it needs the invoking user (sudo/doas/pkexec) or --user, and
# builds as a normal user through runuser. --dry-run prints the plan, writes nothing.
#
# The phone companion (GitHub #400, PR 2 of #362; ADR 2026-10-15 "Setup User Phase:
# Phone Companion Profile With Default-On Features") runs after the root half, as the
# session owner, only when setup was started as that user. This version never
# activates PAM and never changes the Tailscale configuration (later PRs of #362).
# =============================================================================

set -euo pipefail
umask 022
export LC_ALL=C

readonly SETUP_STATE_FORMAT=1
readonly MAX_STATE_BYTES=4096
readonly READY_TIMEOUT_S=30
readonly CAMERA_TIMEOUT_S=15
readonly LOGINCTL_TIMEOUT_S=5
readonly MAX_USER_LEN=32
readonly QUERY_TIMEOUT_S=10

readonly EXIT_FAILURE=1
readonly EXIT_USAGE=2
readonly EXIT_INCOMPLETE=3
readonly EXIT_REFUSED=4
readonly EXIT_COMPANION=5
readonly EXIT_BUILD=40
readonly EXIT_NOT_READY=70

# Phone companion (user phase).
readonly MIN_TAILSCALE_VERSION="1.102.4"
readonly MAX_REMOTE_CONFIG_BYTES=16384
readonly MAX_DAEMON_TOML_SCAN_BYTES=65536
readonly MAX_LOGIN_LEN=254
readonly MAX_HOST_LEN=253
readonly COMPANION_SETTLE_S=2
readonly REMOTE_CONFIG_MARKER="# Managed by scripts/setup.sh (soos-setup): re-running setup rewrites this file."
# The only daemon configuration soos-daemon reads (crates/daemon/src/config.rs
# DEFAULT_CONFIG_PATH; packaging/soos-daemon.service passes no --config). The camera-view
# daemon side is written, and reported done, only when <sysconfdir>/soos/daemon.toml is
# this file; with another --sysconfdir it is a printed manual step.
readonly DAEMON_READ_CONFIG="/etc/soos/daemon.toml"
readonly DAEMON_PREVIEW_MARKER="# Written by scripts/setup.sh (soos-setup): live camera view of the phone companion."
# The boolean keys of remote.toml, in the order setup writes them (prerequisites first).
readonly COMPANION_KEYS=(allow_funnel allow_unlock password_alerts push_notifications
    offline_alerts camera_view camera_view_funnel)
declare -rA FEATURE_OPTION=([allow_funnel]=--funnel [allow_unlock]=--remote-unlock
    [password_alerts]=--password-alerts [push_notifications]=--push
    [offline_alerts]=--offline-alerts [camera_view]=--camera-view
    [camera_view_funnel]=--camera-view-funnel)
declare -rA FEATURE_LABEL=([allow_funnel]="Funnel" [allow_unlock]="remote unlock"
    [password_alerts]="password alerts" [push_notifications]="push"
    [offline_alerts]="offline alerts" [camera_view]="camera view"
    [camera_view_funnel]="camera view over Funnel")
# soos-remote refuses a key whose prerequisite is off (exit 78).
declare -rA FEATURE_PREREQS=([push_notifications]="password_alerts"
    [offline_alerts]="push_notifications" [camera_view_funnel]="camera_view allow_funnel")

if [[ -t 1 ]]; then
    readonly GREEN='\033[0;32m'
    readonly RED='\033[0;31m'
    readonly YELLOW='\033[1;33m'
    readonly BLUE='\033[0;34m'
    readonly NC='\033[0m'
else
    readonly GREEN=''
    readonly RED=''
    readonly YELLOW=''
    readonly BLUE=''
    readonly NC=''
fi

# Messages are printed with %s: variable content (os-release, paths) is never
# interpreted as escape sequences.
info()    { printf '%b[INFO]%b  %s\n' "${BLUE}" "${NC}" "$*"; }
success() { printf '%b[OK]%b    %s\n' "${GREEN}" "${NC}" "$*"; }
warn()    { printf '%b[WARN]%b  %s\n' "${YELLOW}" "${NC}" "$*"; }
error()   { printf '%b[ERROR]%b %s\n' "${RED}" "${NC}" "$*" >&2; }
detected() { printf '[DETECT] %s\n' "$*"; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
SCRIPT_PATH="${SCRIPT_DIR}/$(basename "${BASH_SOURCE[0]}")"

ASSUME_YES=false
DRY_RUN=false
TARGET_USER=""
ENROLL=true
PRESENCE=""
SKIP_PACKAGES=false
REINSTALL=false
DISTRO_OVERRIDE=""
PREFIX="/usr"
SYSCONFDIR="/etc"
LOCALSTATEDIR="/var"
RUNSTATEDIR="/run"
PAM_DIR=""
ARTIFACT_DIR_ARG=""
LOCAL_SEAT_ARG=""
SYSTEM_PHASE=false
NO_REMOTE=false
TS_LOGIN=""
RP_ID=""
COMPANION_OPTS=false
declare -A FEATURE_FLAG=()

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Installs soos from this checkout in one command: build dependencies, verified Rust
toolchain, release build, system install (scripts/install.sh --start), socket access
for the user, presence auto-unlock state, daemon health and camera check, and face
enrollment when run from a local seat. Re-running it is safe: finished phases are
reported as "already done". PAM is not activated by this version.

Options:
  -y, --yes                Non-interactive: do not ask for confirmation (required
                           when stdin or stdout is not a terminal)
  --dry-run                Detect the host and print the plan; change nothing
  --user <NAME>            Account to give socket access and to enroll (default:
                           the invoking user; required when started as root
                           without sudo, doas or pkexec)
  --no-enroll              Skip face enrollment (the command is printed)
  --presence <on|off>      Presence auto-unlock: off creates
                           <sysconfdir>/soos/presence.disable, on removes it;
                           without it the current state is kept and reported
                           (the daemon default is on)
  --skip-packages          Do not install missing packages (the command is printed)
  --reinstall              Run scripts/install.sh even when the install looks complete
  --distro <FAMILY>        Package family: debian, fedora or arch (default:
                           detected from os-release)
  --prefix <DIR>           Installation prefix (default: /usr)
  --sysconfdir <DIR>       Configuration directory (default: /etc)
  --localstatedir <DIR>    State directory (default: /var)
  --runstatedir <DIR>      Runtime directory (default: /run)
  --pam-dir <DIR>          PAM module directory (default: same rule as install.sh)

Phone companion (soos-remote; set up as the session owner after the system phase,
only when setup is started as that user, never as root). Every feature below is on
by default (owner decision). Funnel, remote unlock, push, offline alerts and the
camera view can be turned off later in soos-gui (Features), which also turns off
the features that need them. The failed-password alerts have no switch there: turn
them off with --password-alerts off, or set password_alerts, push_notifications and
offline_alerts to false in remote.toml. A feature already turned off in a
setup-managed remote.toml stays off unless its option says on:
  --no-remote              Do not set up the phone companion
  --tailscale-login <LOGIN>
                           Your Tailscale login (allowed_logins); asked
                           interactively when missing, never read from the
                           Tailscale client
  --rp-id <HOST>           This PC's Tailscale node name <pc>.<tailnet>.ts.net
                           (rp_id, the passkey domain); asked interactively when
                           missing. Without both, a --yes run skips the companion
                           and prints the steps
  --funnel <on|off>        Internet access through Tailscale Funnel (allow_funnel)
  --remote-unlock <on|off> Unlock from the phone with a passkey (allow_unlock)
  --password-alerts <on|off>
                           Failed-password alerts from the journal (password_alerts)
  --push <on|off>          Push notifications (push_notifications and the
                           soos-push-sender unit; needs password alerts)
  --offline-alerts <on|off>
                           Back-online notification (offline_alerts; needs push)
  --camera-view <on|off>   Live camera view (camera_view; the daemon [preview]
                           section is created when daemon.toml is absent)
  --camera-view-funnel <on|off>
                           Camera view over Funnel (camera_view_funnel; needs the
                           camera view and Funnel)
  Setup runs no Tailscale command except the read-only version check: the serve or
  funnel command and the passkey enrollment are printed for you to run.
  --system-phase           Internal: root half, run by the user half through sudo
  --artifact-dir <DIR>     Internal: release artifacts for --system-phase
  --local-seat <yes|no>    Internal: local-seat verdict of the user half for
                           --system-phase
  -h, --help               Display this help message and exit

The build always runs as a normal user (never root). Enrollment runs only from a
local seat (loginctl: Remote=no and a seat); from an SSH or remote session it is
skipped and the command to run at the PC is printed. soos-daemon owns the camera,
so setup stops it for the enrollment and starts it again on every exit path.

Exit codes:
  0   done (every phase done, already done or skipped)
  1   runtime failure (setup's own changes of this run were rolled back)
  2   usage or preflight error (nothing was changed)
  3   installed, but enrollment failed (nothing rolled back; retry command printed)
  4   system phase refused before any system change (packages, toolchain and
      build of this run are kept)
  5   installed, but the phone companion setup failed (its changes of this run
      were rolled back; the face install is kept)
  40  build failed (nothing installed)
  70  installed, but the daemon is not ready (nothing rolled back)
  129 hangup (HUP), 131 quit (QUIT): as 130 and 143
  130 interrupted (INT): setup's own changes of this run rolled back, daemon restarted
  143 terminated (TERM): same as 130
EOF
}

usage_error() {
    error "$*"
    usage >&2
    exit "${EXIT_USAGE}"
}

require_value() {
    if [[ $# -lt 2 || -z "$2" || "$2" == -* ]]; then
        usage_error "Option $1 requires a value"
    fi
}

absolute_dir() {
    local opt="$1" value="$2"
    if [[ "${value}" != /* || "${value}" == *$'\n'* ]]; then
        usage_error "${opt} needs an absolute path: '${value}'"
    fi
    while [[ "${value}" == */ && "${value}" != "/" ]]; do
        value="${value%/}"
    done
    printf '%s' "${value}"
}

valid_user_name() {
    local name="$1"
    [[ ${#name} -ge 1 && ${#name} -le ${MAX_USER_LEN} ]] || return 1
    [[ "${name}" =~ ^[a-z_][a-z0-9_-]*[$]?$ ]] || return 1
    [[ "${name}" != "root" ]]
}

# A Tailscale login (the Tailscale-User-Login value): a strict subset of what soos-remote
# accepts, so it can never break out of a TOML string.
valid_login() {
    local login="$1"
    [[ ${#login} -ge 1 && ${#login} -le ${MAX_LOGIN_LEN} ]] || return 1
    [[ "${login}" =~ ^[A-Za-z0-9._%+@-]+$ ]]
}

# The full node name <node>.<tailnet>.ts.net (lowercase; at least two labels before
# .ts.net, the rule of soos-remote rp_id).
valid_rp_id() {
    local host="$1" prefix label count=0
    [[ ${#host} -ge 1 && ${#host} -le ${MAX_HOST_LEN} && "${host}" == *.ts.net ]] || return 1
    prefix="${host%.ts.net}"
    [[ -n "${prefix}" && "${prefix}" != .* && "${prefix}" != *. && "${prefix}" != *..* ]] || return 1
    local -a labels=()
    IFS=. read -r -a labels <<< "${prefix}"
    for label in "${labels[@]}"; do
        [[ "${label}" =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]] || return 1
        count=$((count + 1))
    done
    [[ ${count} -ge 2 ]]
}

# The remote.toml key of a feature option.
feature_of_option() {
    local key
    for key in "${COMPANION_KEYS[@]}"; do
        if [[ "${FEATURE_OPTION[${key}]}" == "$1" ]]; then
            printf '%s' "${key}"
            return 0
        fi
    done
    return 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        -y|--yes)
            ASSUME_YES=true
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --user)
            require_value "$@"
            valid_user_name "$2" || usage_error "Invalid --user '$2': a local account name (not root, at most ${MAX_USER_LEN} characters)"
            TARGET_USER="$2"
            shift 2
            ;;
        --no-enroll)
            ENROLL=false
            shift
            ;;
        --presence)
            require_value "$@"
            case "$2" in
                on|off) PRESENCE="$2" ;;
                *) usage_error "--presence takes on or off: '$2'" ;;
            esac
            shift 2
            ;;
        --skip-packages)
            SKIP_PACKAGES=true
            shift
            ;;
        --reinstall)
            REINSTALL=true
            shift
            ;;
        --distro)
            require_value "$@"
            case "$2" in
                debian|fedora|arch) DISTRO_OVERRIDE="$2" ;;
                *) usage_error "--distro takes debian, fedora or arch: '$2'" ;;
            esac
            shift 2
            ;;
        --prefix|--sysconfdir|--localstatedir|--runstatedir|--pam-dir|--artifact-dir)
            require_value "$@"
            value="$(absolute_dir "$1" "$2")"
            case "$1" in
                --prefix) PREFIX="${value}" ;;
                --sysconfdir) SYSCONFDIR="${value}" ;;
                --localstatedir) LOCALSTATEDIR="${value}" ;;
                --runstatedir) RUNSTATEDIR="${value}" ;;
                --pam-dir) PAM_DIR="${value}" ;;
                --artifact-dir) ARTIFACT_DIR_ARG="${value}" ;;
            esac
            shift 2
            ;;
        --local-seat)
            require_value "$@"
            case "$2" in
                yes|no) LOCAL_SEAT_ARG="$2" ;;
                *) usage_error "--local-seat takes yes or no: '$2'" ;;
            esac
            shift 2
            ;;
        --system-phase)
            SYSTEM_PHASE=true
            shift
            ;;
        --no-remote)
            NO_REMOTE=true
            shift
            ;;
        --tailscale-login)
            require_value "$@"
            valid_login "$2" || usage_error "Invalid --tailscale-login: letters, digits and . _ % + @ - only, at most ${MAX_LOGIN_LEN} characters"
            TS_LOGIN="$2"
            COMPANION_OPTS=true
            shift 2
            ;;
        --rp-id)
            require_value "$@"
            value="${2,,}"
            value="${value%.}"
            valid_rp_id "${value}" || usage_error "Invalid --rp-id: the full node name <pc>.<tailnet>.ts.net, without scheme or port"
            RP_ID="${value}"
            COMPANION_OPTS=true
            shift 2
            ;;
        --funnel|--remote-unlock|--password-alerts|--push|--offline-alerts|--camera-view|--camera-view-funnel)
            require_value "$@"
            case "$2" in
                on|off) ;;
                *) usage_error "$1 takes on or off: '$2'" ;;
            esac
            FEATURE_FLAG[$(feature_of_option "$1")]="$2"
            COMPANION_OPTS=true
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage_error "Unknown option: $1"
            ;;
    esac
done

IS_ROOT=false
[[ "$(id -u)" == "0" ]] && IS_ROOT=true

if [[ "${SYSTEM_PHASE}" == true ]]; then
    if [[ "${IS_ROOT}" != true || -z "${TARGET_USER}" || -z "${ARTIFACT_DIR_ARG}" \
            || -z "${LOCAL_SEAT_ARG}" || "${DRY_RUN}" == true ]]; then
        usage_error "--system-phase is internal: it runs as root with --user, --artifact-dir and --local-seat, without --dry-run"
    fi
elif [[ -n "${ARTIFACT_DIR_ARG}" || -n "${LOCAL_SEAT_ARG}" ]]; then
    usage_error "--artifact-dir and --local-seat are internal to --system-phase"
fi

# Feature values: the option, else an explicit false of the setup-managed remote.toml
# (FILE_BOOL, filled by classify_remote_config), else on (owner default). A feature whose
# prerequisite resolves off is off too, unless its own option says on: then the run is
# refused before any change (soos-remote would refuse the file with exit 78).
declare -A FILE_BOOL=()
declare -A FEATURE_VALUE=()
declare -A FEATURE_NOTE=()
RESOLVE_CONFLICT=""
resolve_features() {
    local use_file="$1" key flag prereq value
    FEATURE_VALUE=()
    FEATURE_NOTE=()
    RESOLVE_CONFLICT=""
    for key in "${COMPANION_KEYS[@]}"; do
        flag="${FEATURE_FLAG[${key}]:-}"
        value=true
        if [[ "${flag}" == on ]]; then
            value=true
        elif [[ "${flag}" == off ]]; then
            value=false
            FEATURE_NOTE[${key}]="${FEATURE_OPTION[${key}]} off"
        elif [[ "${use_file}" == true && "${FILE_BOOL[${key}]:-}" == false ]]; then
            value=false
            FEATURE_NOTE[${key}]="kept off"
        fi
        for prereq in ${FEATURE_PREREQS[${key}]:-}; do
            [[ "${FEATURE_VALUE[${prereq}]}" == true ]] && continue
            if [[ "${flag}" == on ]]; then
                RESOLVE_CONFLICT="${FEATURE_OPTION[${key}]} on needs ${FEATURE_LABEL[${prereq}]}: add ${FEATURE_OPTION[${prereq}]} on"
                return 1
            fi
            if [[ "${value}" == true ]]; then
                value=false
                FEATURE_NOTE[${key}]="needs ${FEATURE_LABEL[${prereq}]}"
            fi
        done
        FEATURE_VALUE[${key}]="${value}"
    done
}

if [[ "${SYSTEM_PHASE}" == true ]]; then
    # The root half only receives the daemon side of the camera view.
    if [[ "${NO_REMOTE}" == true || -n "${TS_LOGIN}" || -n "${RP_ID}" ]]; then
        usage_error "--system-phase only takes --camera-view of the companion options"
    fi
    for key in "${!FEATURE_FLAG[@]}"; do
        [[ "${key}" == camera_view ]] || usage_error "--system-phase only takes --camera-view of the companion options"
    done
elif [[ "${NO_REMOTE}" == true && "${COMPANION_OPTS}" == true ]]; then
    usage_error "--no-remote cannot be combined with phone companion options"
elif ! resolve_features false; then
    usage_error "${RESOLVE_CONFLICT}"
fi

# Paths derived from the options (the same layout as scripts/install.sh).
PRESENCE_FLAG="${SYSCONFDIR}/soos/presence.disable"
KILL_SWITCH="${SYSCONFDIR}/soos/disabled"
STATE_DIR="${LOCALSTATEDIR}/lib/soos/state"
STATE_FILE="${STATE_DIR}/setup.json"
SOOS_STATE_ROOT="${LOCALSTATEDIR}/lib/soos"
BIOMETRICS_DIR="${SOOS_STATE_ROOT}/biometrics"
MODELS_DIR="${SOOS_STATE_ROOT}/models"
MASTER_KEY="${SOOS_STATE_ROOT}/master.key"
ADMIN_BIN="${PREFIX}/bin/soos-admin"
DAEMON_BIN="${PREFIX}/libexec/soos/soos-daemon"
ENROLL_BIN="${PREFIX}/bin/soos-enroll"
UNIT="soos-daemon.service"

# Runs a command bounded by coreutils timeout when it is available.
run_bounded() {
    local secs="$1"
    shift
    if command -v timeout >/dev/null 2>&1; then
        timeout --kill-after=2 "${secs}" "$@"
    else
        "$@"
    fi
}

unit_query() {
    command -v systemctl >/dev/null 2>&1 || return 1
    run_bounded "${QUERY_TIMEOUT_S}" systemctl "$1" --quiet "${UNIT}" 2>/dev/null
}

# =============================================================================
# Detection (read-only)
# =============================================================================
PREFLIGHT_ERRORS=0
preflight_fail() {
    error "$*"
    PREFLIGHT_ERRORS=$((PREFLIGHT_ERRORS + 1))
}

# Strips quotes and every non-printable character; bounds the length.
clean_value() {
    local v="$1"
    v="${v//[\"\']/}"
    v="${v//[^[:print:]]/}"
    printf '%s' "${v:0:80}"
}

OS_ID=""
OS_ID_LIKE=""
OS_PRETTY=""
# os-release is parsed line by line, never sourced or evaluated.
parse_os_release() {
    local file="$1" line
    while IFS= read -r line || [[ -n "${line}" ]]; do
        case "${line}" in
            ID=*) OS_ID="$(clean_value "${line#ID=}")" ;;
            ID_LIKE=*) OS_ID_LIKE="$(clean_value "${line#ID_LIKE=}")" ;;
            PRETTY_NAME=*) OS_PRETTY="$(clean_value "${line#PRETTY_NAME=}")" ;;
        esac
    done < "${file}"
}

# Package family of an os-release token (same table as check_build_deps.sh).
family_of_token() {
    case "$1" in
        debian|ubuntu|linuxmint|pop) echo debian ;;
        fedora|rhel|centos|rocky|almalinux) echo fedora ;;
        arch|archlinux|manjaro|endeavouros) echo arch ;;
        *) return 1 ;;
    esac
}

FAMILY=""
detect_family() {
    local file="" candidate token
    for candidate in "${SYSCONFDIR}/os-release" "/usr/lib/os-release"; do
        if [[ -f "${candidate}" ]]; then
            file="${candidate}"
            break
        fi
    done
    if [[ -n "${file}" ]]; then
        parse_os_release "${file}"
    fi
    if [[ -n "${DISTRO_OVERRIDE}" ]]; then
        FAMILY="${DISTRO_OVERRIDE}"
        return 0
    fi
    # read -a splits on blanks only: no pathname expansion.
    local -a tokens=()
    read -r -a tokens <<< "${OS_ID} ${OS_ID_LIKE}"
    for token in "${tokens[@]}"; do
        if FAMILY="$(family_of_token "${token}")"; then
            return 0
        fi
    done
    FAMILY=""
}

PKG_MANAGER=""
PKG_INSTALL_CMD=()
PKG_REFRESH_HINT=""
set_package_manager() {
    case "${FAMILY}" in
        debian)
            PKG_MANAGER="apt-get"
            PKG_INSTALL_CMD=(env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends)
            PKG_REFRESH_HINT="sudo apt-get update"
            ;;
        fedora)
            PKG_MANAGER="dnf"
            PKG_INSTALL_CMD=(dnf install -y)
            PKG_REFRESH_HINT="sudo dnf makecache"
            ;;
        arch)
            PKG_MANAGER="pacman"
            PKG_INSTALL_CMD=(pacman -S --needed --noconfirm)
            PKG_REFRESH_HINT="sudo pacman --sync --refresh --sysupgrade"
            ;;
    esac
}

# shellcheck disable=SC2016 # ${db:Status-Status} is a dpkg-query field, not a shell expansion.
package_installed() {
    local pkg="$1"
    case "${FAMILY}" in
        debian) [[ "$(dpkg-query -W -f='${db:Status-Status}' "${pkg}" 2>/dev/null)" == "installed" ]] ;;
        fedora) rpm -q --quiet "${pkg}" >/dev/null 2>&1 ;;
        arch) pacman -Q "${pkg}" >/dev/null 2>&1 ;;
        *) return 1 ;;
    esac
}

MISSING_PACKAGES=()
detect_packages() {
    local group list pkg query_tool
    case "${FAMILY}" in
        debian) query_tool="dpkg-query" ;;
        fedora) query_tool="rpm" ;;
        arch) query_tool="pacman" ;;
        *) return 0 ;;
    esac
    if ! command -v "${query_tool}" >/dev/null 2>&1; then
        preflight_fail "Package query tool not found for the ${FAMILY} family: ${query_tool}"
        return 0
    fi
    local -A seen=()
    local -a pkgs=()
    for group in build models gui; do
        if ! list="$(bash "${SCRIPT_DIR}/check_build_deps.sh" --distro "${FAMILY}" --print-packages "${group}")"; then
            preflight_fail "scripts/check_build_deps.sh could not list the ${group} packages for ${FAMILY}"
            return 0
        fi
        read -r -a pkgs <<< "${list}"
        for pkg in "${pkgs[@]}"; do
            [[ -n "${seen[${pkg}]:-}" ]] && continue
            seen[${pkg}]=1
            package_installed "${pkg}" || MISSING_PACKAGES+=("${pkg}")
        done
    done
}

# Classifies one logind session id: 0 local (Remote=no and a seat), 1 positively not
# local (logind answered Remote=yes, or no seat), 2 unknown (invalid id or the query
# failed, e.g. a terminal that is not a logind session).
session_is_local() {
    local session="$1" out="" line remote="" seat=""
    [[ "${session}" =~ ^[A-Za-z0-9]{1,32}$ ]] || return 2
    out="$(run_bounded "${LOGINCTL_TIMEOUT_S}" loginctl show-session "${session}" \
        --property=Remote --property=Seat 2>/dev/null)" || return 2
    while IFS= read -r line; do
        case "${line}" in
            Remote=*) remote="${line#Remote=}" ;;
            Seat=*) seat="${line#Seat=}" ;;
        esac
    done <<< "${out}"
    [[ -n "${remote}" ]] || return 2
    [[ "${remote}" == "no" && "${seat}" =~ ^[A-Za-z0-9_-]{1,64}$ ]] || return 1
    return 0
}

# Local-seat verdict (fail safe: any doubt means "not local"). The user half may
# fall back to the user's graphical session ONLY when logind does not know the
# shell's own session (a terminal started by the user service manager). A positive
# answer (Remote=yes, or no seat; e.g. an operator over SSH running `su - alice`
# while Alice sits at the desk) is final: remote. A root start never falls back
# (a root shell over SSH must not borrow the desk session).
LOCAL_SEAT=false
detect_local_seat() {
    local allow_display="$1" session="self" display="" line
    LOCAL_SEAT=false
    if [[ -n "${SSH_CONNECTION:-}" || -n "${SSH_CLIENT:-}" || -n "${SSH_TTY:-}" ]]; then
        return 0
    fi
    command -v loginctl >/dev/null 2>&1 || return 0
    [[ -n "${XDG_SESSION_ID:-}" ]] && session="${XDG_SESSION_ID}"
    local verdict=0
    session_is_local "${session}" || verdict=$?
    if [[ "${verdict}" -eq 0 ]]; then
        LOCAL_SEAT=true
        return 0
    fi
    # Positively remote or seatless: final. Only an unknown own session may fall back.
    [[ "${verdict}" -eq 2 && "${allow_display}" == true && -n "${TARGET_USER}" ]] || return 0
    line="$(run_bounded "${LOGINCTL_TIMEOUT_S}" loginctl show-user "${TARGET_USER}" \
        --property=Display 2>/dev/null)" || return 0
    display="${line#Display=}"
    if [[ -n "${display}" ]] && session_is_local "${display}"; then
        LOCAL_SEAT=true
    fi
    return 0
}

camera_node_count() {
    local n=0 node
    for node in /dev/video*; do
        if [[ -c "${node}" ]]; then
            n=$((n + 1))
        fi
    done
    printf '%s' "${n}"
}

display_manager() {
    local link="${SYSCONFDIR}/systemd/system/display-manager.service" target name
    if [[ -L "${link}" ]]; then
        target="$(readlink -- "${link}")" || target=""
        name="$(clean_value "$(basename -- "${target}" .service)")"
        printf '%s' "${name:-unknown}"
    else
        printf 'none'
    fi
}

tailscale_version() {
    local out=""
    if ! command -v tailscale >/dev/null 2>&1; then
        printf 'not installed'
        return 0
    fi
    # Read-only: `tailscale version` is the only Tailscale command setup ever runs.
    # The whole (short) output is read, then cut to its first line: an early-closing
    # pipe reader would turn a SIGPIPE into a lost version under pipefail.
    out="$(run_bounded "${QUERY_TIMEOUT_S}" tailscale version 2>/dev/null)" || out=""
    out="$(clean_value "${out%%$'\n'*}")"
    printf '%s' "${out:-unknown}"
}

# The user who invoked a root shell through sudo, doas or pkexec (same rule as
# scripts/install.sh, GitHub #308), or nothing.
invoking_user() {
    local name=""
    if [[ -n "${SUDO_USER:-}" ]]; then
        name="${SUDO_USER}"
    elif [[ -n "${DOAS_USER:-}" ]]; then
        name="${DOAS_USER}"
    elif [[ -n "${PKEXEC_UID:-}" && "${PKEXEC_UID}" =~ ^[0-9]+$ ]]; then
        name="$(run_bounded "${QUERY_TIMEOUT_S}" getent passwd "${PKEXEC_UID}" | cut -d: -f1)" || name=""
    fi
    [[ "${name}" == "root" ]] && name=""
    printf '%s' "${name}"
}

# Prints "<uid>:<home>" of an account, or fails.
account_info() {
    local name="$1" entry uid home
    entry="$(run_bounded "${QUERY_TIMEOUT_S}" getent passwd "${name}" 2>/dev/null)" || return 1
    uid="$(id -u -- "${name}" 2>/dev/null)" || return 1
    home="$(printf '%s' "${entry}" | cut -d: -f6)"
    [[ "${uid}" =~ ^[0-9]+$ && "${home}" == /* ]] || return 1
    printf '%s:%s' "${uid}" "${home}"
}

TARGET_UID=""
resolve_target_user() {
    if [[ -z "${TARGET_USER}" ]]; then
        if [[ "${IS_ROOT}" == true ]]; then
            TARGET_USER="$(invoking_user)"
            if [[ -z "${TARGET_USER}" ]]; then
                preflight_fail "Started as root without an invoking user (no SUDO_USER, DOAS_USER or PKEXEC_UID): pass --user <name>, or run setup as that user."
                return 0
            fi
        else
            TARGET_USER="$(id -un)"
        fi
        if ! valid_user_name "${TARGET_USER}"; then
            preflight_fail "The target account '${TARGET_USER}' is not a valid non-root account name: pass --user <name>."
            TARGET_USER=""
            return 0
        fi
    fi
    local info=""
    if ! info="$(account_info "${TARGET_USER}")"; then
        preflight_fail "Unknown account: ${TARGET_USER}"
        return 0
    fi
    TARGET_UID="${info%%:*}"
    if [[ "${TARGET_UID}" == "0" ]]; then
        preflight_fail "The target account must not be root (uid 0): ${TARGET_USER}"
        TARGET_UID=""
    fi
}

# Build user: the current user when not root; when root, the invoking user, else the
# owner of the checkout. Never root.
BUILD_USER=""
BUILD_UID=""
BUILD_HOME=""
resolve_build_user() {
    local info=""
    if [[ "${IS_ROOT}" != true ]]; then
        BUILD_USER="$(id -un)"
    else
        BUILD_USER="$(invoking_user)"
        if [[ -z "${BUILD_USER}" ]]; then
            BUILD_USER="$(stat -c '%U' -- "${WORKSPACE_ROOT}" 2>/dev/null)" || BUILD_USER=""
        fi
        if ! command -v runuser >/dev/null 2>&1; then
            preflight_fail "runuser is required to build as a normal user when setup runs as root."
        fi
    fi
    if [[ -z "${BUILD_USER}" || "${BUILD_USER}" == "root" ]] || ! info="$(account_info "${BUILD_USER}")"; then
        preflight_fail "No normal account to build with (the build never runs as root): run setup as the checkout owner, or through sudo from that account."
        BUILD_USER=""
        return 0
    fi
    BUILD_UID="${info%%:*}"
    BUILD_HOME="${info#*:}"
    if [[ "${BUILD_UID}" == "0" ]]; then
        preflight_fail "The build never runs as root."
        BUILD_USER=""
    fi
}

# The cargo target directory as cargo sees it from the checkout (same rule as
# scripts/install.sh resolve_cargo_target_dir, GitHub #331).
resolve_target_dir() {
    local dir="${CARGO_TARGET_DIR:-}"
    if [[ -z "${dir}" ]]; then
        printf '%s' "${WORKSPACE_ROOT}/target"
    elif [[ "${dir}" == /* ]]; then
        printf '%s' "${dir}"
    else
        printf '%s' "${WORKSPACE_ROOT}/${dir}"
    fi
}

# Read-only (same rule as install.sh check_build_target_dir): every entry of the
# target directory must belong to the build user, or cargo fails late.
check_target_dir_owner() {
    local hit=""
    [[ -n "${BUILD_UID}" && -d "${TARGET_DIR}" ]] || return 0
    local q_target
    q_target="$(printf '%q' "${TARGET_DIR}")"
    hit="$(find -H "${TARGET_DIR}" ! -uid "${BUILD_UID}" -print -quit 2>/dev/null)" || hit=""
    if [[ -n "${hit}" ]]; then
        preflight_fail "The cargo target directory ${q_target} holds files not owned by the build user '${BUILD_USER}' (first: $(printf '%q' "${hit}")). Fix it with: sudo chown -R ${BUILD_USER}: ${q_target}"
        return 0
    fi
    hit="$(find -H "${TARGET_DIR}" \( \( -type d ! -perm -u+rwx \) -o \( ! -type d ! -type l ! -perm -u+rw \) \) -print -quit 2>/dev/null)" || hit=""
    if [[ -n "${hit}" ]]; then
        preflight_fail "The cargo target directory ${q_target} holds an entry the build user '${BUILD_USER}' cannot read or write (first: $(printf '%q' "${hit}")). Fix it with: chmod -R u+rwX ${q_target}"
    fi
}

# PAM module directory with the rule of scripts/install.sh (live install).
resolve_pam_dir() {
    local candidate
    if [[ -n "${PAM_DIR}" ]]; then
        return 0
    fi
    for candidate in /lib/x86_64-linux-gnu/security /lib/aarch64-linux-gnu/security \
            /usr/lib64/security /lib64/security /usr/lib/security /lib/security; do
        if [[ -d "${candidate}" ]]; then
            PAM_DIR="${candidate}"
            return 0
        fi
    done
    if [[ -d /usr/lib64 ]]; then
        PAM_DIR="/usr/lib64/security"
    else
        PAM_DIR="/lib/security"
    fi
}

# The PAM template install.sh writes for the family is in place.
pam_template_present() {
    local file
    case "${FAMILY}" in
        arch)
            cmp -s -- "${WORKSPACE_ROOT}/packaging/pam/arch/system-auth.snippet" \
                "${PREFIX}/share/soos/pam/system-auth.snippet"
            ;;
        fedora)
            for file in "${WORKSPACE_ROOT}/packaging/pam/fedora/soos/"*; do
                [[ -f "${file}" ]] || continue
                cmp -s -- "${file}" "${SYSCONFDIR}/authselect/custom/soos/$(basename -- "${file}")" || return 1
            done
            ;;
        debian)
            cmp -s -- "${WORKSPACE_ROOT}/packaging/pam/debian/soos" /usr/share/pam-configs/soos \
                && cmp -s -- "${WORKSPACE_ROOT}/packaging/pam/debian/soos-notify" /usr/share/pam-configs/soos-notify
            ;;
        *) return 1 ;;
    esac
}

# True when every file install.sh installs equals its source, the key and the group
# exist, the deployed models verify, and the unit is enabled and active.
system_up_to_date() {
    local artifacts="$1" name
    [[ -d "${artifacts}" ]] || return 1
    cmp -s -- "${artifacts}/soos-daemon" "${PREFIX}/libexec/soos/soos-daemon" || return 1
    for name in soos-admin soos-enroll soos-gui; do
        cmp -s -- "${artifacts}/${name}" "${PREFIX}/bin/${name}" || return 1
    done
    cmp -s -- "${artifacts}/libpam_soos.so" "${PAM_DIR}/pam_soos.so" || return 1
    cmp -s -- "${WORKSPACE_ROOT}/scripts/provision_master_key.sh" \
        "${PREFIX}/libexec/soos/provision-master-key" || return 1
    cmp -s -- "${WORKSPACE_ROOT}/packaging/soos-daemon.service" \
        "${SYSCONFDIR}/systemd/system/${UNIT}" || return 1
    cmp -s -- "${WORKSPACE_ROOT}/models/manifest.toml" "${MODELS_DIR}/manifest.toml" || return 1
    [[ -e "${MASTER_KEY}" ]] || return 1
    run_bounded "${QUERY_TIMEOUT_S}" getent group soos >/dev/null 2>&1 || return 1
    pam_template_present || return 1
    bash "${SCRIPT_DIR}/download_models.sh" --check-only \
        --manifest "${MODELS_DIR}/manifest.toml" --target-dir "${MODELS_DIR}" >/dev/null 2>&1 || return 1
    unit_query is-enabled || return 1
    unit_query is-active
}

user_in_group() {
    local groups
    groups="$(id -nG -- "${TARGET_USER}" 2>/dev/null)" || return 1
    [[ " ${groups} " == *" soos "* ]]
}

cargo_path() {
    if [[ -x "${BUILD_HOME}/.cargo/bin/cargo" ]]; then
        printf '%s' "${BUILD_HOME}/.cargo/bin/cargo"
    else
        command -v cargo 2>/dev/null
    fi
}

template_path() {
    printf '%s' "${BIOMETRICS_DIR}/${TARGET_UID}.cbor.enc"
}

enroll_command() {
    printf 'sudo systemctl stop soos-daemon && sudo soos-enroll enroll --username %s; sudo systemctl start soos-daemon' \
        "${TARGET_USER}"
}

presence_report() {
    if [[ -e "${KILL_SWITCH}" || -L "${KILL_SWITCH}" ]]; then
        printf 'everything off (%s present: face PAM and presence auto-unlock stopped)' "${KILL_SWITCH}"
    elif [[ -e "${PRESENCE_FLAG}" || -L "${PRESENCE_FLAG}" ]]; then
        printf 'auto-unlock off (%s present; --presence on enables it)' "${PRESENCE_FLAG}"
    else
        printf 'auto-unlock on (daemon default; --presence off disables it)'
    fi
}

# =============================================================================
# Phone companion: detection, renders and plan (read-only)
# =============================================================================
DAEMON_TOML="${SYSCONFDIR}/soos/daemon.toml"
CAMERA_DAEMON="off"
COMPANION_RUN=false
COMPANION_SKIP_SHORT=""
COMPANION_SKIP=""
CONFIG_HOME=""
CONFIG_DIR=""
REMOTE_CONFIG=""
REMOTE_BACKUP=""
BIN_DIR=""
USER_UNIT_DIR=""
REMOTE_SOCKET=""
CONFIG_STATE=""
CONFIG_ACTION=""
CONFIG_FINGERPRINT=""
FILE_LOGIN=""
FILE_RP_ID=""
COMPANION_LOGIN=""
COMPANION_RP_ID=""
UNITS_CURRENT=false
REMOTE_UNIT_KEPT_OFF=false
SENDER_UNIT_KEPT_OFF=false
DAEMON_PREVIEW_STATE=""

# The daemon.toml setup creates when none exists (camera view, daemon side).
render_daemon_preview() {
    printf '%s\n# Docs/DAEMON.md section 1.5; edit or remove it as an administrator.\n[preview]\nenabled = true\nallowed_uids = [%s]\nremote_view = true\n' \
        "${DAEMON_PREVIEW_MARKER}" "${TARGET_UID}"
}

# The setup-managed remote.toml for the resolved features (no secret: the login is an
# account name and the node name is public once Funnel is on).
render_remote_config() {
    local key
    printf '%s\n' "${REMOTE_CONFIG_MARKER}"
    printf '%s\n' "# A feature turned off later (for example in the soos-gui feature tour) stays off until"
    printf '%s\n' "# setup runs with its --<feature> on flag. Reference: Docs/REMOTE_COMPANION.md section 5."
    printf 'allowed_logins = ["%s"]\n' "${COMPANION_LOGIN}"
    printf 'rp_id = "%s"\n' "${COMPANION_RP_ID}"
    for key in "${COMPANION_KEYS[@]}"; do
        printf '%s = %s\n' "${key}" "${FEATURE_VALUE[${key}]}"
    done
}

# Read-only check of an existing daemon.toml: its [preview] table holds exactly
# enabled = true, remote_view = true and allowed_uids listing the target uid.
daemon_toml_allows_view() {
    local line table="" enabled=false remote=false uid_ok=false item size
    [[ -f "${DAEMON_TOML}" && -r "${DAEMON_TOML}" ]] || return 1
    size="$(head -c $((MAX_DAEMON_TOML_SCAN_BYTES + 1)) -- "${DAEMON_TOML}" | wc -c)"
    (( size <= MAX_DAEMON_TOML_SCAN_BYTES )) || return 1
    local -a items=()
    while IFS= read -r line || [[ -n "${line}" ]]; do
        line="${line#"${line%%[![:space:]]*}"}"
        line="${line%"${line##*[![:space:]]}"}"
        if [[ "${line}" =~ ^\[([A-Za-z0-9_.-]+)\]$ ]]; then
            table="${BASH_REMATCH[1]}"
            continue
        elif [[ "${line}" == \[* ]]; then
            # Any other header ([[array]], quoted or spaced names) ends the [preview] table.
            table=""
            continue
        fi
        [[ "${table}" == preview ]] || continue
        case "${line}" in
            "enabled = true") enabled=true ;;
            "remote_view = true") remote=true ;;
        esac
        if [[ "${line}" =~ ^allowed_uids\ =\ \[([0-9,\ ]*)\]$ ]]; then
            IFS=', ' read -r -a items <<< "${BASH_REMATCH[1]}"
            for item in "${items[@]}"; do
                [[ "${item}" == "${TARGET_UID}" ]] && uid_ok=true
            done
        fi
    done < <(head -c "${MAX_DAEMON_TOML_SCAN_BYTES}" -- "${DAEMON_TOML}")
    [[ "${enabled}" == true && "${remote}" == true && "${uid_ok}" == true ]]
}

# elsewhere | absent | done | allows | unreadable | manual | link (read-only). "elsewhere":
# <sysconfdir>/soos/daemon.toml is not the file soos-daemon reads; it is never read or
# written then.
daemon_preview_state() {
    if [[ "${DAEMON_TOML}" != "${DAEMON_READ_CONFIG}" ]]; then
        printf 'elsewhere'
    elif [[ -L "${DAEMON_TOML}" ]]; then
        printf 'link'
    elif [[ ! -e "${DAEMON_TOML}" ]]; then
        printf 'absent'
    elif [[ ! -r "${DAEMON_TOML}" ]]; then
        printf 'unreadable'
    elif cmp -s -- "${DAEMON_TOML}" <(render_daemon_preview); then
        printf 'done'
    elif daemon_toml_allows_view; then
        printf 'allows'
    else
        printf 'manual'
    fi
}

daemon_preview_hint() {
    printf 'as an administrator add to %s: [preview] enabled = true, allowed_uids = [%s], remote_view = true; then: sudo systemctl restart soos-daemon' \
        "${DAEMON_READ_CONFIG}" "${TARGET_UID}"
}

# Classifies remote.toml: absent, template (the untouched install_remote.sh template),
# managed (setup's marker and strict grammar; FILE_LOGIN, FILE_RP_ID, FILE_BOOL) or user.
classify_remote_config() {
    local line key value inner size n=0 managed=true template=true empty_logins=0
    local -A seen=()
    CONFIG_STATE="absent"
    FILE_LOGIN=""
    FILE_RP_ID=""
    FILE_BOOL=()
    [[ -e "${REMOTE_CONFIG}" ]] || return 0
    size="$(head -c $((MAX_REMOTE_CONFIG_BYTES + 1)) -- "${REMOTE_CONFIG}" | wc -c)"
    if (( size > MAX_REMOTE_CONFIG_BYTES )); then
        CONFIG_STATE="user"
        return 0
    fi
    while IFS= read -r line || [[ -n "${line}" ]]; do
        n=$((n + 1))
        if [[ ${n} -eq 1 && "${line}" != "${REMOTE_CONFIG_MARKER}" ]]; then
            managed=false
        fi
        [[ -z "${line}" || "${line}" == \#* ]] && continue
        if [[ "${line}" == "allowed_logins = []" ]]; then
            empty_logins=$((empty_logins + 1))
        else
            template=false
        fi
        [[ "${managed}" == true ]] || continue
        if [[ ! "${line}" =~ ^([a-z_]+)\ =\ (.+)$ ]]; then
            managed=false
            continue
        fi
        key="${BASH_REMATCH[1]}"
        value="${BASH_REMATCH[2]}"
        if [[ -n "${seen[${key}]:-}" ]]; then
            managed=false
            continue
        fi
        seen[${key}]=1
        case "${key}" in
            allowed_logins)
                # BASH_REMATCH is copied first: the validators run their own matches.
                inner=""
                [[ "${value}" =~ ^\[\"([^\"]+)\"\]$ ]] && inner="${BASH_REMATCH[1]}"
                if [[ -n "${inner}" ]] && valid_login "${inner}"; then
                    FILE_LOGIN="${inner}"
                else
                    managed=false
                fi
                ;;
            rp_id)
                inner=""
                [[ "${value}" =~ ^\"([^\"]+)\"$ ]] && inner="${BASH_REMATCH[1]}"
                if [[ -n "${inner}" ]] && valid_rp_id "${inner}"; then
                    FILE_RP_ID="${inner}"
                else
                    managed=false
                fi
                ;;
            allow_funnel|allow_unlock|password_alerts|push_notifications|offline_alerts|camera_view|camera_view_funnel)
                if [[ "${value}" == true || "${value}" == false ]]; then
                    FILE_BOOL[${key}]="${value}"
                else
                    managed=false
                fi
                ;;
            *) managed=false ;;
        esac
    done < <(head -c "${MAX_REMOTE_CONFIG_BYTES}" -- "${REMOTE_CONFIG}")
    if [[ "${managed}" == true && -n "${FILE_LOGIN}" && -n "${FILE_RP_ID}" ]]; then
        CONFIG_STATE="managed"
        return 0
    fi
    FILE_LOGIN=""
    FILE_RP_ID=""
    FILE_BOOL=()
    if [[ "${template}" == true && ${empty_logins} -eq 1 ]]; then
        CONFIG_STATE="template"
    else
        CONFIG_STATE="user"
    fi
}

# Identity of remote.toml at plan time (re-checked before the write).
config_fingerprint() {
    if [[ -e "${REMOTE_CONFIG}" || -L "${REMOTE_CONFIG}" ]]; then
        cksum < "${REMOTE_CONFIG}"
    else
        printf 'absent'
    fi
}

user_unit_query() {
    run_bounded "${QUERY_TIMEOUT_S}" systemctl --user "$1" --quiet "$2" 2>/dev/null
}

companion_units_current() {
    user_unit_query is-enabled soos-remote.service || return 1
    user_unit_query is-active soos-remote.service || return 1
    if [[ "${FEATURE_VALUE[push_notifications]}" == true && "${SENDER_UNIT_KEPT_OFF}" != true ]]; then
        user_unit_query is-enabled soos-push-sender.service || return 1
        user_unit_query is-active soos-push-sender.service || return 1
    fi
}

features_summary() {
    local key out="" item
    for key in "${COMPANION_KEYS[@]}"; do
        item="${FEATURE_LABEL[${key}]} "
        if [[ "${FEATURE_VALUE[${key}]}" == true ]]; then
            item+="on"
        else
            item+="off"
        fi
        [[ -n "${FEATURE_NOTE[${key}]:-}" ]] && item+=" (${FEATURE_NOTE[${key}]})"
        out+="${out:+, }${item}"
    done
    printf '%s' "${out}"
}

companion_skip() {
    COMPANION_SKIP_SHORT="$1"
    COMPANION_SKIP="$2"
    if [[ "${COMPANION_OPTS}" == true ]]; then
        preflight_fail "The phone companion options cannot be applied: $2"
    fi
}

# Decides whether the companion runs and with what; asks the login and node name in an
# interactive run. Never writes.
resolve_companion() {
    local answer=""
    if [[ "${NO_REMOTE}" == true ]]; then
        companion_skip "--no-remote" "--no-remote given"
        return 0
    fi
    if [[ "${IS_ROOT}" == true ]]; then
        companion_skip "started as root" "it runs as the session owner: at the PC, re-run ./scripts/setup.sh as ${TARGET_USER:-the user} without sudo"
        return 0
    fi
    if [[ -z "${TARGET_USER}" || -z "${TARGET_UID}" || "${TARGET_USER}" != "${BUILD_USER}" ]]; then
        companion_skip "another user" "it runs as the session owner: run ./scripts/setup.sh as ${TARGET_USER:-the user}"
        return 0
    fi
    if [[ "${HOME:-}" != /* ]]; then
        companion_skip "no HOME" "HOME is not an absolute path"
        return 0
    fi
    if [[ -n "${XDG_CONFIG_HOME:-}" && "${XDG_CONFIG_HOME}" != /* ]]; then
        companion_skip "XDG_CONFIG_HOME" "XDG_CONFIG_HOME is not an absolute path"
        return 0
    fi
    if [[ "${XDG_RUNTIME_DIR:-}" != /* || ! -d "${XDG_RUNTIME_DIR}" || ! -O "${XDG_RUNTIME_DIR}" ]]; then
        companion_skip "no user manager" "XDG_RUNTIME_DIR is not this user's runtime directory (log in to a session of ${TARGET_USER} and re-run setup)"
        return 0
    fi
    CONFIG_HOME="${XDG_CONFIG_HOME:-${HOME}/.config}"
    CONFIG_DIR="${CONFIG_HOME}/soos"
    REMOTE_CONFIG="${CONFIG_DIR}/remote.toml"
    REMOTE_BACKUP="${REMOTE_CONFIG}.soos-setup.bak"
    BIN_DIR="${HOME}/.local/bin"
    USER_UNIT_DIR="${CONFIG_HOME}/systemd/user"
    REMOTE_SOCKET="${XDG_RUNTIME_DIR}/soos-remote/remote.sock"
    if [[ -e "${CONFIG_HOME}" && ! ( -d "${CONFIG_HOME}" && -O "${CONFIG_HOME}" ) && ! -L "${CONFIG_HOME}" ]]; then
        companion_skip "unsafe path" "${CONFIG_HOME} is not a directory owned by ${TARGET_USER}"
        return 0
    fi
    local path
    for path in "${CONFIG_HOME}" "${CONFIG_DIR}" "${REMOTE_CONFIG}" "${REMOTE_BACKUP}"; do
        if [[ -L "${path}" ]]; then
            companion_skip "unsafe path" "${path} is a symbolic link; setup never writes through it"
            return 0
        fi
    done
    if [[ -e "${CONFIG_DIR}" && ! ( -d "${CONFIG_DIR}" && -O "${CONFIG_DIR}" ) ]]; then
        companion_skip "unsafe path" "${CONFIG_DIR} is not a directory owned by ${TARGET_USER}"
        return 0
    fi
    local perms=""
    if [[ -d "${CONFIG_DIR}" ]]; then
        perms="$(stat -c '%a' -- "${CONFIG_DIR}")" || perms=""
        if [[ ! "${perms}" =~ ^[0-7]{3,4}$ ]] || (( (8#${perms} & 8#022) != 0 )); then
            companion_skip "unsafe path" "${CONFIG_DIR} is group- or world-writable (mode ${perms}): chmod 0700 ${CONFIG_DIR}"
            return 0
        fi
    fi
    if [[ -e "${REMOTE_BACKUP}" && ! ( -f "${REMOTE_BACKUP}" && -O "${REMOTE_BACKUP}" ) ]]; then
        companion_skip "unsafe path" "${REMOTE_BACKUP} is not a regular file owned by ${TARGET_USER}"
        return 0
    fi
    if [[ -e "${REMOTE_CONFIG}" && ! ( -f "${REMOTE_CONFIG}" && -O "${REMOTE_CONFIG}" && -r "${REMOTE_CONFIG}" ) ]]; then
        companion_skip "unsafe path" "${REMOTE_CONFIG} is not a readable regular file owned by ${TARGET_USER}"
        return 0
    fi
    classify_remote_config
    CONFIG_FINGERPRINT="$(config_fingerprint)"
    if [[ "${CONFIG_STATE}" == user ]]; then
        if [[ "${COMPANION_OPTS}" == true ]]; then
            preflight_fail "${REMOTE_CONFIG} was not written by setup, which never edits it: move it aside (mv ${REMOTE_CONFIG} ${REMOTE_CONFIG}.mine) to let setup write it, or edit it yourself."
            return 0
        fi
        COMPANION_RUN=true
        CONFIG_ACTION="kept"
        CAMERA_DAEMON="off"
        resolve_features false || true
        return 0
    fi
    COMPANION_LOGIN="${TS_LOGIN:-${FILE_LOGIN}}"
    COMPANION_RP_ID="${RP_ID:-${FILE_RP_ID}}"
    INTERACTIVE=false
    if [[ "${ASSUME_YES}" != true && "${DRY_RUN}" != true && -t 0 && -t 1 ]]; then
        INTERACTIVE=true
    fi
    if [[ "${INTERACTIVE}" == true && -z "${COMPANION_LOGIN}" ]]; then
        read -r -p "Tailscale login for the phone companion (for example you@example.com; empty: skip the companion): " answer || answer=""
        if [[ -n "${answer}" ]]; then
            valid_login "${answer}" || usage_error "Invalid Tailscale login"
            COMPANION_LOGIN="${answer}"
        fi
    fi
    if [[ "${INTERACTIVE}" == true && -n "${COMPANION_LOGIN}" && -z "${COMPANION_RP_ID}" ]]; then
        read -r -p "This PC's Tailscale node name <pc>.<tailnet>.ts.net (empty: skip the companion): " answer || answer=""
        answer="${answer,,}"
        answer="${answer%.}"
        if [[ -n "${answer}" ]]; then
            valid_rp_id "${answer}" || usage_error "Invalid node name: <pc>.<tailnet>.ts.net"
            COMPANION_RP_ID="${answer}"
        fi
    fi
    if [[ -z "${COMPANION_LOGIN}" || -z "${COMPANION_RP_ID}" ]]; then
        companion_skip "needs the Tailscale login and node name" "re-run with --tailscale-login <you@example.com> --rp-id <pc>.<tailnet>.ts.net (Docs/REMOTE_COMPANION.md section 4)"
        return 0
    fi
    if ! resolve_features "$([[ "${CONFIG_STATE}" == managed ]] && printf true || printf false)"; then
        preflight_fail "${RESOLVE_CONFLICT} (${REMOTE_CONFIG} keeps it off)"
        return 0
    fi
    case "${CONFIG_STATE}" in
        absent) CONFIG_ACTION="create" ;;
        template) CONFIG_ACTION="replace" ;;
        managed)
            if cmp -s -- "${REMOTE_CONFIG}" <(render_remote_config); then
                CONFIG_ACTION="already done"
            else
                CONFIG_ACTION="rewrite"
            fi
            ;;
    esac
    CAMERA_DAEMON="off"
    if [[ "${FEATURE_VALUE[camera_view]}" == true ]]; then
        CAMERA_DAEMON="on"
        DAEMON_PREVIEW_STATE="$(daemon_preview_state)"
    fi
    # A unit the owner disabled later (soos-gui companion switch) stays disabled: setup
    # enables the units only for a configuration it writes for the first time.
    if [[ "${CONFIG_STATE}" == managed ]]; then
        user_unit_query is-enabled soos-remote.service || REMOTE_UNIT_KEPT_OFF=true
        # The sender only when push was already on in that file: switching push on now
        # enables it like a first write.
        if [[ "${FEATURE_VALUE[push_notifications]}" == true && "${FILE_BOOL[push_notifications]:-}" == true ]]; then
            user_unit_query is-enabled soos-push-sender.service || SENDER_UNIT_KEPT_OFF=true
        fi
    fi
    if [[ "${CONFIG_ACTION}" == "already done" ]] && companion_units_current; then
        UNITS_CURRENT=true
    fi
    COMPANION_RUN=true
}

print_companion_plan() {
    if [[ "${COMPANION_RUN}" != true ]]; then
        plan companion "skipped (${COMPANION_SKIP_SHORT})" "${COMPANION_SKIP}"
        return 0
    fi
    plan companion "will run" "scripts/install_remote.sh as ${TARGET_USER} (its own build of soos-remote and soos-push-sender, a second compile on the first run; binaries in ${BIN_DIR}, user units; unchanged files are reported as already up to date)"
    case "${CONFIG_ACTION}" in
        kept)
            plan "companion config" "kept" "${REMOTE_CONFIG} was not written by setup, which never edits it (the keys setup would write are printed at the end)"
            plan "camera view (daemon)" "skipped" "the configuration is not managed by setup"
            plan "companion units" "left as they are" "configuration not managed by setup (running units are restarted after a binary update)"
            ;;
        *)
            local what=""
            case "${CONFIG_ACTION}" in
                create) what="create ${REMOTE_CONFIG} (mode 0600)" ;;
                replace) what="replace the untouched install_remote.sh template ${REMOTE_CONFIG} (backup ${REMOTE_BACKUP})" ;;
                rewrite) what="rewrite ${REMOTE_CONFIG} (backup ${REMOTE_BACKUP})" ;;
                *) what="${REMOTE_CONFIG}" ;;
            esac
            local status="will run"
            [[ "${CONFIG_ACTION}" == "already done" ]] && status="already done"
            plan "companion config" "${status}" "${what}; login given, node ${COMPANION_RP_ID}: $(features_summary)"
            print_daemon_preview_plan
            if [[ "${REMOTE_UNIT_KEPT_OFF}" == true ]]; then
                plan "companion units" "kept off" "soos-remote.service is not enabled; setup only enables it for a configuration it writes for the first time (systemctl --user enable --now soos-remote.service)"
            elif [[ "${UNITS_CURRENT}" == true ]]; then
                plan "companion units" "already done" "soos-remote.service$([[ "${FEATURE_VALUE[push_notifications]}" == true ]] && printf ' and soos-push-sender.service') enabled and running"
            else
                plan "companion units" "will run" "validate the configuration, then enable and start soos-remote.service$([[ "${FEATURE_VALUE[push_notifications]}" == true ]] && printf ' and soos-push-sender.service') (systemctl --user)"
            fi
            ;;
    esac
    plan "tailscale" "printed only" "setup runs no tailscale serve or funnel command; the steps are printed at the end"
}

print_daemon_preview_plan() {
    if [[ "${CAMERA_DAEMON}" != on ]]; then
        plan "camera view (daemon)" "skipped" "camera view off"
        return 0
    fi
    case "${DAEMON_PREVIEW_STATE}" in
        absent) plan "camera view (daemon)" "will run" "create ${DAEMON_TOML} ([preview] enabled, allowed_uids = [${TARGET_UID}], remote_view = true) and restart soos-daemon" ;;
        done) plan "camera view (daemon)" "already done" "${DAEMON_TOML} was written by setup" ;;
        allows) plan "camera view (daemon)" "already done" "${DAEMON_TOML} allows the view" ;;
        unreadable) plan "camera view (daemon)" "checked at run time (needs root)" "${DAEMON_TOML} is not readable by ${TARGET_USER}" ;;
        link) plan "camera view (daemon)" "skipped" "${DAEMON_TOML} is a symbolic link; setup never follows it" ;;
        elsewhere) plan "camera view (daemon)" "manual (not applied)" "soos-daemon reads ${DAEMON_READ_CONFIG}, not ${DAEMON_TOML} (--sysconfdir), so setup writes nothing: $(daemon_preview_hint)" ;;
        *) plan "camera view (daemon)" "manual" "${DAEMON_TOML} exists and setup never edits it: $(daemon_preview_hint)" ;;
    esac
}

# =============================================================================
# State file (bounded, strict grammar, no secret)
# =============================================================================
PREV_GROUP_ADDED=false
PREV_FLAG_CREATED=false
STATE_REFUSE_CODE="${EXIT_USAGE}"

state_invalid() {
    error "Refusing the setup state file ${STATE_FILE}: $*"
    error "Inspect it, or remove it to start over: sudo rm ${STATE_FILE} (it records what setup did, never a secret)."
    exit "${STATE_REFUSE_CODE}"
}

read_state() {
    PREV_GROUP_ADDED=false
    PREV_FLAG_CREATED=false
    [[ -L "${STATE_DIR}" ]] && state_invalid "its directory is a symbolic link"
    [[ -L "${STATE_FILE}" ]] && state_invalid "it is a symbolic link"
    [[ -e "${STATE_FILE}" ]] || return 0
    [[ -f "${STATE_FILE}" ]] || state_invalid "it is not a regular file"
    [[ -O "${STATE_FILE}" ]] || state_invalid "it is not owned by the user running setup's root half"
    local size perms
    size="$(head -c $((MAX_STATE_BYTES + 1)) -- "${STATE_FILE}" | wc -c)"
    (( size <= MAX_STATE_BYTES )) || state_invalid "it is larger than ${MAX_STATE_BYTES} bytes"
    perms="$(stat -c '%a' -- "${STATE_FILE}")"
    [[ "${perms}" =~ ^[0-7]{3,4}$ ]] || state_invalid "its mode cannot be read"
    (( (8#${perms} & 8#022) == 0 )) || state_invalid "it is group- or world-writable (mode ${perms})"
    local line key value stored
    while IFS= read -r line || [[ -n "${line}" ]]; do
        [[ "${line}" == "{" || "${line}" == "}" ]] && continue
        if [[ ! "${line}" =~ ^\ \ \"([a-z_]+)\":\ ([^,]+),?$ ]]; then
            state_invalid "unexpected line"
        fi
        key="${BASH_REMATCH[1]}"
        value="${BASH_REMATCH[2]}"
        case "${key}" in
            format) [[ "${value}" == "${SETUP_STATE_FORMAT}" ]] || state_invalid "unsupported format" ;;
            tool) [[ "${value}" == '"soos-setup"' ]] || state_invalid "unexpected tool" ;;
            profile) [[ "${value}" == '"face"' ]] || state_invalid "unexpected profile" ;;
            user)
                [[ "${value}" =~ ^\"([a-z0-9_$-]{1,32})\"$ ]] || state_invalid "invalid user"
                stored="${BASH_REMATCH[1]}"
                valid_user_name "${stored}" || state_invalid "invalid user"
                if [[ "${stored}" != "${TARGET_USER}" ]]; then
                    state_invalid "it records the user ${stored}, not ${TARGET_USER} (format ${SETUP_STATE_FORMAT} holds one user)"
                fi
                ;;
            presence) [[ "${value}" == '"on"' || "${value}" == '"off"' ]] || state_invalid "invalid presence" ;;
            enrolled) [[ "${value}" == true || "${value}" == false ]] || state_invalid "invalid enrolled" ;;
            group_added_by_setup)
                [[ "${value}" == true || "${value}" == false ]] || state_invalid "invalid ${key}"
                [[ "${value}" == true ]] && PREV_GROUP_ADDED=true
                ;;
            presence_flag_created_by_setup)
                [[ "${value}" == true || "${value}" == false ]] || state_invalid "invalid ${key}"
                [[ "${value}" == true ]] && PREV_FLAG_CREATED=true
                ;;
            *) state_invalid "unknown key" ;;
        esac
    done < <(head -c "${MAX_STATE_BYTES}" -- "${STATE_FILE}")
    return 0
}

# Read-only root checks of the system half (symbolic links are never followed).
root_preflight() {
    local path
    read_state
    for path in "${SYSCONFDIR}/soos" "${PRESENCE_FLAG}"; do
        if [[ -L "${path}" ]]; then
            error "${path} is a symbolic link; refusing to follow it"
            exit "${STATE_REFUSE_CODE}"
        fi
    done
}

GROUP_ADDED_NOW=false
FLAG_CREATED_NOW=false

write_state() {
    local presence="on" enrolled=false group=false flag=false content tmp
    [[ -e "${PRESENCE_FLAG}" || -L "${PRESENCE_FLAG}" ]] && presence="off"
    [[ -n "${TARGET_UID}" && -e "$(template_path)" ]] && enrolled=true
    [[ "${PREV_GROUP_ADDED}" == true || "${GROUP_ADDED_NOW}" == true ]] && group=true
    [[ "${PREV_FLAG_CREATED}" == true || "${FLAG_CREATED_NOW}" == true ]] && flag=true
    content="$(printf '{\n  "format": %s,\n  "tool": "soos-setup",\n  "profile": "face",\n  "user": "%s",\n  "group_added_by_setup": %s,\n  "presence_flag_created_by_setup": %s,\n  "presence": "%s",\n  "enrolled": %s\n}' \
        "${SETUP_STATE_FORMAT}" "${TARGET_USER}" "${group}" "${flag}" "${presence}" "${enrolled}")"
    content+=$'\n'
    if [[ -L "${STATE_DIR}" || -L "${STATE_FILE}" ]]; then
        error "The state path is a symbolic link: ${STATE_FILE}"
        return 1
    fi
    if [[ ! -d "${STATE_DIR}" ]]; then
        install -d -m 0700 -- "${STATE_DIR}" || return 1
    fi
    if [[ -f "${STATE_FILE}" ]] && cmp -s -- "${STATE_FILE}" <(printf '%s' "${content}"); then
        printf '[DONE] state: already up to date (%s)\n' "${STATE_FILE}"
        return 0
    fi
    tmp="$(umask 077 && mktemp "${STATE_DIR}/.setup.json.XXXXXX")" || return 1
    if ! { printf '%s' "${content}" > "${tmp}" && chmod 0600 -- "${tmp}" && mv -f -- "${tmp}" "${STATE_FILE}"; }; then
        rm -f -- "${tmp}"
        return 1
    fi
    printf '[DONE] state: %s\n' "${STATE_FILE}"
}

# =============================================================================
# Root half: system, access, presence, verify, enrollment, state
# =============================================================================
JOURNAL=()
ROLLBACK_ARMED=false
DAEMON_STOPPED=false

rollback_journal() {
    set +e
    local i
    for ((i = ${#JOURNAL[@]} - 1; i >= 0; i--)); do
        case "${JOURNAL[i]}" in
            group_added)
                if command -v gpasswd >/dev/null 2>&1; then
                    gpasswd -d "${TARGET_USER}" soos >/dev/null 2>&1
                elif command -v deluser >/dev/null 2>&1; then
                    deluser "${TARGET_USER}" soos >/dev/null 2>&1
                fi
                ;;
            presence_flag_created) rm -f -- "${PRESENCE_FLAG}" ;;
            presence_flag_removed) (set -C; : > "${PRESENCE_FLAG}") 2>/dev/null && chmod 0644 -- "${PRESENCE_FLAG}" ;;
            soos_conf_dir_created) rmdir -- "${SYSCONFDIR}/soos" 2>/dev/null ;;
            daemon_toml_created)
                rm -f -- "${DAEMON_TOML}"
                run_bounded "${QUERY_TIMEOUT_S}" systemctl try-restart "${UNIT}" >/dev/null 2>&1
                ;;
        esac
    done
    JOURNAL=()
    set -e
}

fail_and_rollback() {
    local phase="$1"
    trap - HUP INT QUIT TERM
    printf '[FAIL] %s\n' "${phase}"
    rollback_journal
    ROLLBACK_ARMED=false
    error "Setup FAILED in phase ${phase}; setup's own changes of this run were rolled back."
    error "The soos install itself (scripts/install.sh) is kept; remove it with: sudo ./scripts/uninstall.sh --keep-data"
    exit "${EXIT_FAILURE}"
}

# shellcheck disable=SC2329 # Invoked by the HUP/INT/QUIT/TERM traps.
on_signal() {
    local rc="$1"
    trap - HUP INT QUIT TERM
    if [[ "${ROLLBACK_ARMED}" == true ]]; then
        rollback_journal
        error "Interrupted; setup's own changes of this run were rolled back."
    fi
    exit "${rc}"
}

# EXIT trap of the root half: the daemon stopped for the enrollment is always started
# again, whatever the exit path.
# shellcheck disable=SC2329 # Invoked by the EXIT trap.
on_exit_system() {
    local rc=$?
    if [[ "${DAEMON_STOPPED}" == true ]]; then
        DAEMON_STOPPED=false
        if ! run_bounded "${QUERY_TIMEOUT_S}" systemctl start "${UNIT}"; then
            error "Could not start ${UNIT} again: sudo systemctl start soos-daemon"
            [[ "${rc}" -eq 0 || "${rc}" -eq "${EXIT_INCOMPLETE}" ]] && rc="${EXIT_NOT_READY}"
        fi
    fi
    exit "${rc}"
}

phase_system() {
    local rc=0
    local -a args=(--start --artifact-dir "${ARTIFACT_DIR}" --prefix "${PREFIX}"
        --sysconfdir "${SYSCONFDIR}" --localstatedir "${LOCALSTATEDIR}" --runstatedir "${RUNSTATEDIR}"
        --pam-dir "${PAM_DIR}" --distro "${FAMILY}")
    if [[ "${REINSTALL}" != true ]] && system_up_to_date "${ARTIFACT_DIR}"; then
        SYSTEM_SKIPPED=true
        printf '[DONE] system: already done (installed files equal the build, models verified, unit enabled and active)\n'
        return 0
    fi
    info "Running scripts/install.sh --start (transactional; rolls back its own changes on failure)"
    bash "${SCRIPT_DIR}/install.sh" "${args[@]}" || rc=$?
    return "${rc}"
}

phase_access() {
    if user_in_group; then
        printf '[DONE] access: already done (%s is in the soos group)\n' "${TARGET_USER}"
        return 0
    fi
    "${ADMIN_BIN}" add-user "${TARGET_USER}" || return 1
    JOURNAL+=(group_added)
    GROUP_ADDED_NOW=true
    printf '[DONE] access: %s added to the soos group\n' "${TARGET_USER}"
}

ensure_soos_conf_dir() {
    local dir="${SYSCONFDIR}/soos"
    if [[ -L "${dir}" ]]; then
        error "${dir} is a symbolic link"
        return 1
    fi
    if [[ -e "${dir}" && ! -d "${dir}" ]]; then
        error "${dir} exists and is not a directory"
        return 1
    fi
    if [[ ! -d "${dir}" ]]; then
        install -d -m 0755 -- "${dir}" || return 1
        JOURNAL+=(soos_conf_dir_created)
    fi
}

phase_presence() {
    if [[ -L "${PRESENCE_FLAG}" ]]; then
        error "${PRESENCE_FLAG} is a symbolic link; refusing to follow it"
        return 1
    fi
    case "${PRESENCE}" in
        off)
            if [[ -e "${PRESENCE_FLAG}" ]]; then
                printf '[DONE] presence: already done (auto-unlock off)\n'
                return 0
            fi
            ensure_soos_conf_dir || return 1
            (set -C; : > "${PRESENCE_FLAG}") || return 1
            JOURNAL+=(presence_flag_created)
            FLAG_CREATED_NOW=true
            chmod 0644 -- "${PRESENCE_FLAG}" || return 1
            printf '[DONE] presence: auto-unlock turned off (%s)\n' "${PRESENCE_FLAG}"
            ;;
        on)
            if [[ ! -e "${PRESENCE_FLAG}" ]]; then
                printf '[DONE] presence: already done (auto-unlock on)\n'
                return 0
            fi
            rm -f -- "${PRESENCE_FLAG}" || return 1
            JOURNAL+=(presence_flag_removed)
            printf '[DONE] presence: auto-unlock turned on (%s removed)\n' "${PRESENCE_FLAG}"
            ;;
        *)
            printf '[DONE] presence: %s\n' "$(presence_report)"
            ;;
    esac
}

# Camera view, daemon side (only with --camera-view on from the user half): creates
# daemon.toml with the [preview] section when the file is absent; an existing file is
# never edited (the administrator change is printed).
phase_preview() {
    local tmp="" state
    [[ "${CAMERA_DAEMON}" == on ]] || return 0
    state="$(daemon_preview_state)"
    case "${state}" in
        done)
            printf '[DONE] camera view (daemon): already done (%s written by setup)\n' "${DAEMON_TOML}"
            return 0
            ;;
        allows)
            printf '[DONE] camera view (daemon): already done (%s allows the view)\n' "${DAEMON_TOML}"
            return 0
            ;;
        link)
            printf '[SKIP] camera view (daemon): %s is a symbolic link; setup never follows it\n' "${DAEMON_TOML}"
            return 0
            ;;
        elsewhere)
            printf '[SKIP] camera view (daemon): not applied: soos-daemon reads %s, not %s; %s\n' \
                "${DAEMON_READ_CONFIG}" "${DAEMON_TOML}" "$(daemon_preview_hint)"
            return 0
            ;;
        absent) ;;
        *)
            printf '[SKIP] camera view (daemon): %s exists and setup never edits it: %s\n' \
                "${DAEMON_TOML}" "$(daemon_preview_hint)"
            return 0
            ;;
    esac
    ensure_soos_conf_dir || return 1
    tmp="$(mktemp "${SYSCONFDIR}/soos/.daemon.toml.XXXXXX")" || return 1
    # A hard link never replaces an existing file: created only when still absent.
    if ! { render_daemon_preview > "${tmp}" && chmod 0644 -- "${tmp}" && ln -- "${tmp}" "${DAEMON_TOML}"; }; then
        rm -f -- "${tmp}"
        return 1
    fi
    rm -f -- "${tmp}"
    JOURNAL+=(daemon_toml_created)
    # GitHub #441: the installed daemon validates the new file with its own loader before any
    # restart; a refused file is removed again and the daemon keeps running untouched.
    if ! run_bounded "${QUERY_TIMEOUT_S}" "${DAEMON_BIN}" --check-config --config "${DAEMON_TOML}"; then
        printf '[FAIL] camera view (daemon): soos-daemon --check-config refused %s (removed again; %s not restarted)\n' "${DAEMON_TOML}" "${UNIT}"
        rm -f -- "${DAEMON_TOML}"
        unset 'JOURNAL[${#JOURNAL[@]}-1]'
        return 1
    fi
    info "Restarting ${UNIT} to load the [preview] section of ${DAEMON_TOML}"
    if ! run_bounded "${QUERY_TIMEOUT_S}" systemctl restart "${UNIT}" || ! daemon_ready; then
        printf '[FAIL] camera view (daemon): %s did not become ready with %s (removed again)\n' "${UNIT}" "${DAEMON_TOML}"
        rm -f -- "${DAEMON_TOML}"
        unset 'JOURNAL[${#JOURNAL[@]}-1]'
        if ! run_bounded "${QUERY_TIMEOUT_S}" systemctl restart "${UNIT}" || ! daemon_ready; then
            PREVIEW_DAEMON_DOWN=true
        fi
        return 1
    fi
    printf '[DONE] camera view (daemon): %s created (preview for uid %s, remote view on); %s restarted\n' \
        "${DAEMON_TOML}" "${TARGET_UID}" "${UNIT}"
}

daemon_ready() {
    bash "${SCRIPT_DIR}/wait_daemon_ready.sh" \
        --socket "${RUNSTATEDIR}/soos/daemon.sock" \
        --manifest "${MODELS_DIR}/manifest.toml" \
        --admin "${ADMIN_BIN}" \
        --timeout "${READY_TIMEOUT_S}"
}

CAMERA_OK=false
phase_verify() {
    if ! daemon_ready; then
        printf '[FAIL] verify: the daemon is not ready (journalctl -u soos-daemon.service)\n'
        if [[ "${SYSTEM_SKIPPED}" == true ]]; then
            error "The install looked complete but the daemon is not ready: re-run setup with --reinstall."
        fi
        return 1
    fi
    info "Camera report (metadata only, no frame captured):"
    if run_bounded "${CAMERA_TIMEOUT_S}" "${ADMIN_BIN}" camera list --config "${SYSCONFDIR}/soos/daemon.toml"; then
        CAMERA_OK=true
        printf '[DONE] verify: daemon healthy, camera selected\n'
    else
        warn "no camera selected: check the camera connection and [pipeline] camera_device in ${SYSCONFDIR}/soos/daemon.toml"
        printf '[DONE] verify: daemon healthy, no camera selected\n'
    fi
}

ENROLL_FAILED=false
phase_enrollment() {
    local reason="" rc=0
    if [[ "${ENROLL}" != true ]]; then
        reason="--no-enroll"
    elif [[ -e "$(template_path)" ]]; then
        printf '[DONE] enrollment: already done (%s has a template)\n' "${TARGET_USER}"
        return 0
    elif [[ "${LOCAL_SEAT}" != true ]]; then
        reason="remote session or no local seat"
    elif [[ "${CAMERA_OK}" != true ]]; then
        reason="no camera"
    fi
    if [[ -n "${reason}" ]]; then
        printf '[SKIP] enrollment: %s - run at the PC, in front of the camera: %s\n' "${reason}" "$(enroll_command)"
        if [[ "${reason}" == "remote session or no local seat" && "${SYSTEM_PHASE}" != true ]]; then
            info "Started as root, setup only trusts the session of this shell: at the PC, re-run ./scripts/setup.sh without sudo from the desktop session."
        fi
        return 0
    fi
    # soos-daemon is the exclusive owner of the camera: stop it for the enrollment;
    # the EXIT trap starts it again on every exit path.
    info "Stopping ${UNIT} for the enrollment (it owns the camera); it is started again afterwards."
    DAEMON_STOPPED=true
    run_bounded "${QUERY_TIMEOUT_S}" systemctl stop "${UNIT}" || rc=$?
    if [[ "${rc}" -eq 0 ]]; then
        # Without a terminal the run was confirmed with --yes: soos-enroll then saves the
        # new template without its own [y/N] prompt (only reached when none exists).
        local -a confirm=()
        [[ -t 0 ]] || confirm=(--yes)
        info "Enrolling ${TARGET_USER}: a face is captured now, at this PC; look at the camera."
        "${ENROLL_BIN}" --biometrics-dir "${BIOMETRICS_DIR}" --key-file "${MASTER_KEY}" \
            --models-dir "${MODELS_DIR}" enroll --username "${TARGET_USER}" "${confirm[@]}" || rc=$?
    fi
    # The stop marker is cleared only once the unit was started again, so a signal
    # before that point still leaves the restart to the EXIT trap.
    if run_bounded "${QUERY_TIMEOUT_S}" systemctl start "${UNIT}"; then
        DAEMON_STOPPED=false
        if ! daemon_ready; then
            printf '[FAIL] enrollment: %s did not become ready; see journalctl -u soos-daemon.service\n' "${UNIT}"
            NOT_READY_AFTER_ENROLL=true
        fi
    else
        printf '[FAIL] enrollment: %s did not start again; see journalctl -u soos-daemon.service\n' "${UNIT}"
        NOT_READY_AFTER_ENROLL=true
    fi
    if [[ "${rc}" -eq 0 ]]; then
        printf '[DONE] enrollment: %s enrolled\n' "${TARGET_USER}"
    else
        ENROLL_FAILED=true
        printf '[FAIL] enrollment: retry at the PC, in front of the camera: %s\n' "$(enroll_command)"
    fi
}

print_summary() {
    echo ""
    info "Summary:"
    if [[ "${GROUP_ADDED_NOW}" == true ]]; then
        info "  ${TARGET_USER} was added to the soos group: log out and back in for it to apply."
    fi
    info "  Presence: $(presence_report)"
    info "  PAM is not activated by this version: see Docs/DISTRIBUTION_DEPLOYMENT.md (test before activating)."
    if [[ -n "${COMPANION_SKIP}" ]]; then
        info "  Phone companion: skipped (${COMPANION_SKIP})."
    fi
    info "  Rescue: sudo touch ${KILL_SWITCH}   Remove: sudo ./scripts/uninstall.sh --keep-data"
}

finish_with_state() {
    local rc="$1"
    trap - HUP INT QUIT TERM
    ROLLBACK_ARMED=false
    if ! write_state; then
        printf '[FAIL] state\n'
        error "Could not write ${STATE_FILE}; the changes of this run are kept (setup can be re-run)."
        exit "${EXIT_FAILURE}"
    fi
    print_summary
    exit "${rc}"
}

SYSTEM_SKIPPED=false
NOT_READY_AFTER_ENROLL=false
PREVIEW_DAEMON_DOWN=false
run_system_phase() {
    STATE_REFUSE_CODE="${EXIT_REFUSED}"
    [[ "${SYSTEM_PHASE}" != true ]] && STATE_REFUSE_CODE="${EXIT_USAGE}"
    root_preflight
    trap 'on_signal 129' HUP
    trap 'on_signal 130' INT
    trap 'on_signal 131' QUIT
    trap 'on_signal 143' TERM
    trap on_exit_system EXIT

    local rc=0
    phase_system || rc=$?
    if [[ "${rc}" -eq "${EXIT_NOT_READY}" ]]; then
        printf '[FAIL] system: installed, but the daemon is not ready (exit 70; nothing rolled back)\n'
        finish_with_state "${EXIT_NOT_READY}"
    elif [[ "${rc}" -ne 0 ]]; then
        printf '[FAIL] system\n'
        error "scripts/install.sh failed (exit ${rc}): install.sh rolled back its own changes; setup changed nothing else on the system."
        exit "${EXIT_FAILURE}"
    fi

    ROLLBACK_ARMED=true
    phase_access || fail_and_rollback "access"
    phase_presence || fail_and_rollback "presence"
    if ! phase_preview; then
        if [[ "${PREVIEW_DAEMON_DOWN}" == true ]]; then
            printf '[FAIL] camera view (daemon): %s is not ready even without %s (exit 70; nothing else rolled back)\n' "${UNIT}" "${DAEMON_TOML}"
            finish_with_state "${EXIT_NOT_READY}"
        fi
        fail_and_rollback "camera view (daemon)"
    fi
    if ! phase_verify; then
        finish_with_state "${EXIT_NOT_READY}"
    fi
    phase_enrollment
    if [[ "${NOT_READY_AFTER_ENROLL}" == true ]]; then
        finish_with_state "${EXIT_NOT_READY}"
    elif [[ "${ENROLL_FAILED}" == true ]]; then
        finish_with_state "${EXIT_INCOMPLETE}"
    fi
    finish_with_state 0
}

if [[ "${SYSTEM_PHASE}" == true ]]; then
    detect_family
    [[ -z "${FAMILY}" ]] && usage_error "--system-phase needs a supported distribution (or --distro)"
    resolve_target_user
    if [[ "${PREFLIGHT_ERRORS}" -gt 0 ]]; then
        exit "${EXIT_REFUSED}"
    fi
    resolve_pam_dir
    ARTIFACT_DIR="${ARTIFACT_DIR_ARG}"
    CAMERA_DAEMON="${FEATURE_FLAG[camera_view]:-off}"
    LOCAL_SEAT=false
    [[ "${LOCAL_SEAT_ARG}" == "yes" ]] && LOCAL_SEAT=true
    run_system_phase
fi

# =============================================================================
# User half: detect, plan, confirm, packages, toolchain, build, escalate
# =============================================================================
detect_family
if [[ -z "${FAMILY}" ]]; then
    preflight_fail "Unsupported distribution '${OS_PRETTY:-unknown}' (ID=${OS_ID:-none}): supported families are debian, fedora and arch; pass --distro <family> to choose one."
else
    set_package_manager
fi
INIT="none"
if [[ -d "${RUNSTATEDIR}/systemd/system" ]]; then
    INIT="systemd"
else
    preflight_fail "systemd is required (${RUNSTATEDIR}/systemd/system not found)."
fi
resolve_target_user
resolve_build_user
resolve_pam_dir
if [[ "${IS_ROOT}" == true ]]; then
    detect_local_seat false
else
    detect_local_seat true
fi
CAMERA_NODES="$(camera_node_count)"
DISPLAY_MANAGER="$(display_manager)"
TAILSCALE_VERSION="$(tailscale_version)"
if [[ -n "${FAMILY}" ]]; then
    detect_packages
fi
TARGET_DIR="$(resolve_target_dir)"
ARTIFACT_DIR="${TARGET_DIR}/release"
check_target_dir_owner

ESCALATE=()
if [[ "${IS_ROOT}" != true ]]; then
    if command -v sudo >/dev/null 2>&1; then
        ESCALATE=(sudo --)
    elif command -v doas >/dev/null 2>&1; then
        ESCALATE=(doas --)
    elif [[ "${DRY_RUN}" != true ]]; then
        preflight_fail "Root is needed for the system phase, but neither sudo nor doas was found: run this script as root with --user <name>."
    fi
fi
if [[ "${DRY_RUN}" != true && "${ASSUME_YES}" != true && ! ( -t 0 && -t 1 ) ]]; then
    preflight_fail "Not running on a terminal: pass --yes to run non-interactively (or --dry-run to only print the plan)."
fi

detected "distribution: ${OS_PRETTY:-unknown} (family: ${FAMILY:-unsupported})"
detected "package manager: ${PKG_MANAGER:-none}"
detected "init: ${INIT}"
detected "display manager: ${DISPLAY_MANAGER}"
detected "tailscale: ${TAILSCALE_VERSION} (read only; setup never runs serve or funnel)"
if [[ "${LOCAL_SEAT}" == true ]]; then
    detected "session: local seat"
else
    detected "session: remote or no local seat"
fi
detected "camera: ${CAMERA_NODES} V4L2 node(s) under /dev"
detected "user: ${TARGET_USER:-unknown} (build user: ${BUILD_USER:-none})"

# Started as root, the root-only checks run now, before anything changes.
if [[ "${IS_ROOT}" == true && "${PREFLIGHT_ERRORS}" -eq 0 ]]; then
    root_preflight
fi

# Phone companion: decided now (read-only; may ask the login and node name).
if [[ "${PREFLIGHT_ERRORS}" -eq 0 ]]; then
    resolve_companion
fi

if [[ "${PREFLIGHT_ERRORS}" -gt 0 ]]; then
    error "Preflight failed with ${PREFLIGHT_ERRORS} error(s); nothing was changed."
    exit "${EXIT_USAGE}"
fi

plan() { printf '[PLAN] %s: %s - %s\n' "$1" "$2" "$3"; }

PKG_COMMAND="${PKG_INSTALL_CMD[*]} ${MISSING_PACKAGES[*]}"
if [[ "${IS_ROOT}" != true ]]; then
    PKG_COMMAND="${ESCALATE[0]:-sudo} ${PKG_COMMAND}"
fi
if [[ ${#MISSING_PACKAGES[@]} -eq 0 ]]; then
    plan packages "already done" "every package of check_build_deps.sh (build, models, gui) is installed"
elif [[ "${SKIP_PACKAGES}" == true ]]; then
    plan packages "skipped (--skip-packages)" "missing: ${MISSING_PACKAGES[*]}; install them with: ${PKG_COMMAND}"
else
    plan packages "will run" "${PKG_COMMAND}"
fi
CARGO_FOUND="$(cargo_path)" || CARGO_FOUND=""
if [[ -n "${CARGO_FOUND}" ]]; then
    plan toolchain "already done" "cargo found: ${CARGO_FOUND}"
    if [[ ! -x "${BUILD_HOME}/.cargo/bin/rustup" ]] && ! command -v rustup >/dev/null 2>&1; then
        warn "cargo without rustup ignores the toolchain pinned by rust-toolchain.toml; scripts/install_rustup.sh is recommended."
    fi
else
    plan toolchain "will run" "scripts/install_rustup.sh as ${BUILD_USER} (verified rustup-init, pinned toolchain)"
fi
plan build "will run" "cargo build --release --locked --workspace as ${BUILD_USER} (incremental)"
if [[ "${REINSTALL}" == true ]]; then
    plan system "will run" "scripts/install.sh --start (--reinstall)"
elif system_up_to_date "${ARTIFACT_DIR}"; then
    plan system "already done" "installed files equal the current build, models verified, unit enabled and active (checked again after the build)"
else
    plan system "will run" "scripts/install.sh --start"
fi
if user_in_group; then
    plan access "already done" "${TARGET_USER} is in the soos group"
else
    plan access "will run" "soos-admin add-user ${TARGET_USER}"
fi
case "${PRESENCE}" in
    off)
        if [[ -e "${PRESENCE_FLAG}" ]]; then
            plan presence "already done" "auto-unlock off"
        else
            plan presence "will run" "create ${PRESENCE_FLAG}"
        fi
        ;;
    on)
        if [[ -e "${PRESENCE_FLAG}" ]]; then
            plan presence "will run" "remove ${PRESENCE_FLAG}"
        else
            plan presence "already done" "auto-unlock on"
        fi
        ;;
    *)
        plan presence "already done" "$(presence_report)"
        ;;
esac
plan verify "will run" "daemon health (wait_daemon_ready.sh) and camera report"
if [[ "${ENROLL}" != true ]]; then
    plan enrollment "skipped (--no-enroll)" "$(enroll_command)"
elif [[ -n "${TARGET_UID}" && -e "$(template_path)" ]]; then
    plan enrollment "already done" "${TARGET_USER} has a template"
elif [[ "${LOCAL_SEAT}" != true ]]; then
    plan enrollment "skipped (remote session or no local seat)" "run at the PC: $(enroll_command)"
elif [[ -d "${BIOMETRICS_DIR}" && ! ( -r "${BIOMETRICS_DIR}" && -x "${BIOMETRICS_DIR}" ) ]]; then
    plan enrollment "checked at run time (needs root)" "soos-enroll enroll --username ${TARGET_USER}, daemon stopped meanwhile"
else
    plan enrollment "will run" "soos-enroll enroll --username ${TARGET_USER}, daemon stopped meanwhile (needs a camera)"
fi
plan state "will run" "${STATE_FILE} (opt-ins and results only, never a secret)"
print_companion_plan
plan later "not in this version" "PAM activation (Docs/DISTRIBUTION_DEPLOYMENT.md)"

if [[ "${DRY_RUN}" == true ]]; then
    success "Dry run complete: nothing was changed."
    exit 0
fi

if [[ "${ASSUME_YES}" != true ]]; then
    answer=""
    read -r -p "Proceed with this plan? [y/N] " answer || answer=""
    case "${answer}" in
        y|Y|yes|YES) ;;
        *)
            info "Aborted by the user; nothing was changed."
            exit "${EXIT_FAILURE}"
            ;;
    esac
fi

# Runs a command as root: directly, or through the escalation helper.
as_root() {
    if [[ "${IS_ROOT}" == true ]]; then
        "$@"
    else
        "${ESCALATE[@]}" "$@"
    fi
}

# Runs a command as the build user (never root): directly, or through runuser with
# a cleared environment and an explicit allow-list, so that root's CARGO_HOME,
# RUSTUP_HOME, RUSTFLAGS or any other variable never reaches the user build.
as_build_user() {
    if [[ "${IS_ROOT}" == true ]]; then
        local -a keep=(HOME="${BUILD_HOME}" USER="${BUILD_USER}" LOGNAME="${BUILD_USER}"
            PATH="${BUILD_HOME}/.cargo/bin:${PATH}")
        local name
        for name in LANG LC_ALL TERM http_proxy https_proxy no_proxy HTTP_PROXY HTTPS_PROXY NO_PROXY; do
            if [[ -n "${!name:-}" ]]; then
                keep+=("${name}=${!name}")
            fi
        done
        runuser -u "${BUILD_USER}" -- env -i "${keep[@]}" "$@"
    else
        env PATH="${BUILD_HOME}/.cargo/bin:${PATH}" "$@"
    fi
}

# =============================================================================
# User phase: the phone companion, as the session owner (never root)
# =============================================================================
COMPANION_JOURNAL=()
COMPANION_ARMED=false
COMPANION_INSTALLED_NOW=false
CONFIG_CHANGED=false

rollback_companion() {
    set +e
    local i entry restored=false tmp
    local -a restart_units=()
    for ((i = ${#COMPANION_JOURNAL[@]} - 1; i >= 0; i--)); do
        entry="${COMPANION_JOURNAL[i]}"
        case "${entry}" in
            unit_enabled:*)
                run_bounded "${QUERY_TIMEOUT_S}" systemctl --user disable --now "${entry#unit_enabled:}" >/dev/null 2>&1
                ;;
            unit_started:*)
                run_bounded "${QUERY_TIMEOUT_S}" systemctl --user stop "${entry#unit_started:}" >/dev/null 2>&1
                ;;
            unit_restarted:*)
                restart_units+=("${entry#unit_restarted:}")
                ;;
            config_dir_created)
                rmdir -- "${CONFIG_DIR}" 2>/dev/null
                ;;
            config_created)
                rm -f -- "${REMOTE_CONFIG}"
                restored=true
                ;;
            config_replaced)
                tmp="$(umask 077 && mktemp "${CONFIG_DIR}/.remote.toml.XXXXXX")"
                if [[ -n "${tmp}" ]] && cat -- "${REMOTE_BACKUP}" > "${tmp}" && chmod 0600 -- "${tmp}"; then
                    mv -f -- "${tmp}" "${REMOTE_CONFIG}"
                else
                    rm -f -- "${tmp}"
                    error "Could not restore ${REMOTE_CONFIG}; the previous version is ${REMOTE_BACKUP}"
                fi
                restored=true
                ;;
        esac
    done
    # Units that ran before this run run again, on the restored configuration.
    for entry in "${restart_units[@]}"; do
        run_bounded "${QUERY_TIMEOUT_S}" systemctl --user restart "${entry}" >/dev/null 2>&1
    done
    if [[ "${restored}" == true && ${#restart_units[@]} -eq 0 ]]; then
        run_bounded "${QUERY_TIMEOUT_S}" systemctl --user try-restart soos-remote.service >/dev/null 2>&1
    fi
    COMPANION_JOURNAL=()
    set -e
}

# shellcheck disable=SC2329 # Invoked by the HUP/INT/QUIT/TERM traps of the user phase.
on_companion_signal() {
    local rc="$1"
    trap - HUP INT QUIT TERM
    if [[ "${COMPANION_ARMED}" == true ]]; then
        rollback_companion
        error "Interrupted; the phone companion changes of this run were rolled back."
    fi
    exit "${rc}"
}

# Writes the managed remote.toml: 0600 in a 0700 directory, temporary file and rename,
# the previous file kept as a 0600 backup.
write_remote_config() {
    local tmp
    if [[ ! -d "${CONFIG_DIR}" ]]; then
        (umask 077 && mkdir -p -- "${CONFIG_DIR}") || return 1
        COMPANION_JOURNAL+=(config_dir_created)
    fi
    if [[ -L "${CONFIG_DIR}" || -L "${REMOTE_CONFIG}" || -L "${REMOTE_BACKUP}" ]]; then
        error "A symbolic link appeared under ${CONFIG_DIR}; refusing to write through it"
        return 1
    fi
    if [[ -e "${REMOTE_CONFIG}" ]]; then
        tmp="$(umask 077 && mktemp "${CONFIG_DIR}/.remote.toml.bak.XXXXXX")" || return 1
        if ! { cat -- "${REMOTE_CONFIG}" > "${tmp}" && chmod 0600 -- "${tmp}" && mv -f -- "${tmp}" "${REMOTE_BACKUP}"; }; then
            rm -f -- "${tmp}"
            return 1
        fi
        COMPANION_JOURNAL+=(config_replaced)
    else
        COMPANION_JOURNAL+=(config_created)
    fi
    tmp="$(umask 077 && mktemp "${CONFIG_DIR}/.remote.toml.XXXXXX")" || return 1
    if ! { render_remote_config > "${tmp}" && chmod 0600 -- "${tmp}" && mv -f -- "${tmp}" "${REMOTE_CONFIG}"; }; then
        rm -f -- "${tmp}"
        return 1
    fi
    CONFIG_CHANGED=true
}

# Enables and starts a user unit, or starts / restarts it when needed.
UNITS_STARTED_NOW=false
companion_unit() {
    local unit="$1" restart="$2"
    if ! user_unit_query is-enabled "${unit}"; then
        # Journaled first: a failed enable --now may leave the unit enabled.
        COMPANION_JOURNAL+=("unit_enabled:${unit}")
        UNITS_STARTED_NOW=true
        run_bounded "${QUERY_TIMEOUT_S}" systemctl --user enable --now "${unit}" || return 1
    elif ! user_unit_query is-active "${unit}"; then
        # Inactive before this run: a rollback stops it again.
        COMPANION_JOURNAL+=("unit_started:${unit}")
        UNITS_STARTED_NOW=true
        run_bounded "${QUERY_TIMEOUT_S}" systemctl --user start "${unit}" || return 1
    elif [[ "${restart}" == true ]]; then
        # Active before this run: a rollback restarts it on the restored configuration.
        COMPANION_JOURNAL+=("unit_restarted:${unit}")
        UNITS_STARTED_NOW=true
        run_bounded "${QUERY_TIMEOUT_S}" systemctl --user restart "${unit}" || return 1
    fi
}

# Checksums of the four files install_remote.sh installs ("absent" when missing).
installed_fingerprint() {
    local file
    for file in "${BIN_DIR}/soos-remote" "${BIN_DIR}/soos-push-sender" \
            "${USER_UNIT_DIR}/soos-remote.service" "${USER_UNIT_DIR}/soos-push-sender.service"; do
        if [[ -f "${file}" ]]; then
            cksum < "${file}"
        else
            printf 'absent\n'
        fi
    done
}

companion_failed() {
    local step="$1"
    printf '[FAIL] companion: %s\n' "${step}"
    rollback_companion
    COMPANION_ARMED=false
    trap - HUP INT QUIT TERM
    error "The phone companion setup FAILED (${step}); its changes of this run were rolled back. The face install is kept."
    if [[ "${COMPANION_INSTALLED_NOW}" == true ]]; then
        error "The companion binaries and units installed by scripts/install_remote.sh are kept; remove them with: scripts/install_remote.sh --uninstall"
    fi
    if [[ "${CAMERA_DAEMON}" == on && "${DAEMON_PREVIEW_STATE}" == absent ]]; then
        error "The [preview] section the root half created in ${DAEMON_TOML} is kept (an administrator file; remove it by hand if unwanted)."
    fi
    return "${EXIT_COMPANION}"
}

run_companion_phase() {
    local push="${FEATURE_VALUE[push_notifications]:-false}"
    COMPANION_JOURNAL=()
    COMPANION_ARMED=true
    trap 'on_companion_signal 129' HUP
    trap 'on_companion_signal 130' INT
    trap 'on_companion_signal 131' QUIT
    trap 'on_companion_signal 143' TERM

    if [[ "$(config_fingerprint)" != "${CONFIG_FINGERPRINT}" ]]; then
        companion_failed "${REMOTE_CONFIG} changed during setup; re-run setup"
        return
    fi
    case "${CONFIG_ACTION}" in
        create|replace|rewrite)
            write_remote_config || { companion_failed "writing ${REMOTE_CONFIG}"; return; }
            printf '[DONE] companion config: %s written (mode 0600)\n' "${REMOTE_CONFIG}"
            ;;
        "already done") printf '[DONE] companion config: already done (%s)\n' "${REMOTE_CONFIG}" ;;
        kept) printf '[DONE] companion config: kept (%s was not written by setup)\n' "${REMOTE_CONFIG}" ;;
    esac
    # install_remote.sh builds its own two packages (their feature set differs from the
    # workspace build), so the decision is taken on the installed files, before and after.
    local before after
    before="$(installed_fingerprint)"
    info "Running scripts/install_remote.sh (as ${TARGET_USER}; incremental build; it never enables a unit)"
    if ! (cd "${WORKSPACE_ROOT}" && as_build_user CARGO_TARGET_DIR="${TARGET_DIR}" \
            bash "${SCRIPT_DIR}/install_remote.sh" >/dev/null); then
        companion_failed "scripts/install_remote.sh failed (run it directly to see its output)"
        return
    fi
    after="$(installed_fingerprint)"
    if [[ "${before}" != "${after}" ]]; then
        COMPANION_INSTALLED_NOW=true
        printf '[DONE] companion: installed in %s\n' "${BIN_DIR}"
    else
        printf '[DONE] companion: already up to date (installed files unchanged)\n'
    fi
    if [[ "${CONFIG_ACTION}" == kept ]]; then
        if [[ "${COMPANION_INSTALLED_NOW}" == true ]]; then
            run_bounded "${QUERY_TIMEOUT_S}" systemctl --user try-restart soos-remote.service soos-push-sender.service \
                || { companion_failed "systemctl --user try-restart"; return; }
        fi
        COMPANION_ARMED=false
        trap - HUP INT QUIT TERM
        printf '[DONE] companion units: left as they are (configuration not managed by setup)\n'
        return 0
    fi
    # soos-remote validates the whole configuration (exit 78 on any refusal) without
    # starting the service; its output (no login) is discarded.
    local vrc=0
    run_bounded "${QUERY_TIMEOUT_S}" "${BIN_DIR}/soos-remote" --config "${REMOTE_CONFIG}" passkeys list >/dev/null || vrc=$?
    if [[ "${vrc}" -eq 78 ]]; then
        companion_failed "soos-remote refused the configuration ${REMOTE_CONFIG} (exit 78)"
        return
    elif [[ "${vrc}" -ne 0 ]]; then
        companion_failed "soos-remote could not check its passkey store next to ${REMOTE_CONFIG} (exit ${vrc}; soos-remote passkeys list shows the reason)"
        return
    fi
    local -a running=()
    if [[ "${REMOTE_UNIT_KEPT_OFF}" == true ]]; then
        printf '[SKIP] companion units: soos-remote.service is not enabled; setup only enables it for a configuration it writes for the first time: systemctl --user enable --now soos-remote.service\n'
    else
        companion_unit soos-remote.service "$([[ "${CONFIG_CHANGED}" == true || "${COMPANION_INSTALLED_NOW}" == true ]] && printf true || printf false)" \
            || { companion_failed "starting soos-remote.service"; return; }
        running+=(soos-remote.service)
    fi
    if [[ "${push}" == true && "${SENDER_UNIT_KEPT_OFF}" == true ]]; then
        printf '[SKIP] companion units: soos-push-sender.service is not enabled; setup only enables it for a configuration it writes for the first time or when push is switched on: systemctl --user enable --now soos-push-sender.service\n'
    elif [[ "${push}" == true ]]; then
        companion_unit soos-push-sender.service "${COMPANION_INSTALLED_NOW}" \
            || { companion_failed "starting soos-push-sender.service"; return; }
        running+=(soos-push-sender.service)
    fi
    # Type=exec: a configuration refusal (exit 78) shows only after the start returned.
    if [[ "${UNITS_STARTED_NOW}" == true ]]; then
        sleep "${COMPANION_SETTLE_S}"
    fi
    local unit
    for unit in "${running[@]}"; do
        if ! user_unit_query is-active "${unit}"; then
            companion_failed "${unit} is not running (journalctl --user -u ${unit%.service})"
            return
        fi
    done
    COMPANION_ARMED=false
    trap - HUP INT QUIT TERM
    if [[ ${#running[@]} -gt 0 ]]; then
        printf '[DONE] companion: %s running\n' "${running[*]}"
    fi
}

# Version a.b.c of the detected Tailscale client is at least MIN_TAILSCALE_VERSION.
tailscale_recent_enough() {
    local have="$1" min="${MIN_TAILSCALE_VERSION}" i
    [[ "${have}" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+) ]] || return 1
    local -a a=("${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "${BASH_REMATCH[3]}")
    [[ "${min}" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || return 1
    local -a b=("${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}" "${BASH_REMATCH[3]}")
    for i in 0 1 2; do
        (( 10#${a[i]} > 10#${b[i]} )) && return 0
        (( 10#${a[i]} < 10#${b[i]} )) && return 1
    done
    return 0
}

# Guided steps: printed for the owner, never run by setup.
print_companion_guide() {
    local groups="" group journal_group="" target=""
    echo ""
    info "Phone companion:"
    if [[ "${CONFIG_ACTION}" == kept ]]; then
        info "  ${REMOTE_CONFIG} is yours: setup did not edit it. The owner defaults it would write are:"
        info "  allowed_logins, rp_id, allow_funnel = true, allow_unlock = true, password_alerts = true,"
        info "  push_notifications = true, offline_alerts = true, camera_view = true, camera_view_funnel = true"
        info "  (Docs/REMOTE_COMPANION.md section 5), then: systemctl --user enable --now soos-remote"
        return 0
    fi
    info "  Next steps (setup never runs them):"
    if [[ "${TAILSCALE_VERSION}" == "not installed" ]]; then
        warn "  Tailscale is not installed: install it from the official instructions (https://tailscale.com/download/linux) and log in to your tailnet."
    elif ! tailscale_recent_enough "${TAILSCALE_VERSION}"; then
        warn "  Tailscale ${TAILSCALE_VERSION} is older than ${MIN_TAILSCALE_VERSION} (or unknown): upgrade it first; older releases were not verified to strip forged identity headers."
    fi
    if [[ "${FEATURE_VALUE[allow_funnel]}" == true ]]; then
        info "  1. Publish the companion on the internet (no VPN on the phone):"
        info "       tailscale funnel --bg unix:${REMOTE_SOCKET}"
        info "     Funnel must be allowed for this node by the funnel attribute of your tailnet policy (Tailscale admin console)."
    else
        info "  1. Publish the companion on your tailnet:"
        info "       tailscale serve --bg unix:${REMOTE_SOCKET}"
    fi
    info "  2. On the phone open https://${COMPANION_RP_ID} and add it to the home screen."
    info "  3. Register the phone's passkey over the tailnet: at this PC run soos-remote enroll-code and type the code on the phone (it is shown only on this PC)."
    if [[ "${FEATURE_VALUE[push_notifications]}" != true ]] && user_unit_query is-enabled soos-push-sender.service; then
        info "  Push is off: soos-push-sender.service stays enabled but idle; stop it with: systemctl --user disable --now soos-push-sender.service"
    fi
    if [[ "${FEATURE_VALUE[push_notifications]}" == true ]]; then
        info "  4. Push: in the web app tap Enable notifications."
        target="$(readlink -f /etc/resolv.conf 2>/dev/null)" || target=""
        if [[ "${target}" == /run/* ]]; then
            warn "  /etc/resolv.conf resolves to ${target}, which the soos-push-sender sandbox hides: push notifications are NOT delivered until you add the resolver drop-in of Docs/REMOTE_COMPANION.md section 2d (setup never adds it)."
        fi
    fi
    if [[ "${FEATURE_VALUE[password_alerts]}" == true ]]; then
        groups="$(id -nG -- "${TARGET_USER}" 2>/dev/null)" || groups=""
        for group in wheel adm systemd-journal; do
            if [[ " ${groups} " == *" ${group} "* ]]; then
                journal_group="${group}"
                break
            fi
        done
        if [[ -n "${journal_group}" ]]; then
            info "  Password alerts: ${TARGET_USER} can read the system journal (group ${journal_group})."
        else
            warn "  Password alerts need read access to the system journal: as an administrator run 'sudo usermod -aG systemd-journal ${TARGET_USER}', then log out and back in (setup never changes groups)."
        fi
    fi
    if [[ "${CAMERA_DAEMON}" == on && ( "${DAEMON_PREVIEW_STATE}" == manual || "${DAEMON_PREVIEW_STATE}" == link || "${DAEMON_PREVIEW_STATE}" == elsewhere ) ]]; then
        warn "  Camera view: $(daemon_preview_hint)"
    fi
    info "  Optional: 'loginctl enable-linger ${TARGET_USER}' keeps the companion answering while you are logged out (setup never enables it)."
    info "  Funnel, remote unlock, push, offline alerts and the camera view can be turned off later in soos-gui (Features); the failed-password alerts have no switch there: run setup again with --password-alerts off. Remove the companion with scripts/install_remote.sh --uninstall."
}

# --- packages (root; never removed by a rollback) ---
if [[ ${#MISSING_PACKAGES[@]} -gt 0 && "${SKIP_PACKAGES}" != true ]]; then
    info "Installing missing packages: ${PKG_COMMAND}"
    if ! as_root "${PKG_INSTALL_CMD[@]}" "${MISSING_PACKAGES[@]}"; then
        printf '[FAIL] packages\n'
        error "The package install failed; nothing else was changed. If the package index is stale, refresh it first (${PKG_REFRESH_HINT}) and re-run setup."
        exit "${EXIT_FAILURE}"
    fi
    printf '[DONE] packages: installed %s (kept on any later failure)\n' "${MISSING_PACKAGES[*]}"
fi

# --- toolchain (build user; never removed by a rollback) ---
if [[ -z "${CARGO_FOUND}" ]]; then
    info "Installing the pinned Rust toolchain with scripts/install_rustup.sh (verified rustup-init)"
    if ! as_build_user bash "${SCRIPT_DIR}/install_rustup.sh"; then
        printf '[FAIL] toolchain\n'
        exit "${EXIT_FAILURE}"
    fi
    printf '[DONE] toolchain\n'
fi

# --- build (build user, never root; explicit PATH, no login shell) ---
info "Building: cargo build --release --locked --workspace (as ${BUILD_USER})"
if ! (cd "${WORKSPACE_ROOT}" && as_build_user CARGO_TARGET_DIR="${TARGET_DIR}" SOOS_BINDIR="${PREFIX}/bin" \
        cargo build --release --locked --workspace); then
    printf '[FAIL] build\n'
    error "The release build failed; nothing was installed."
    exit "${EXIT_BUILD}"
fi
printf '[DONE] build: %s\n' "${ARTIFACT_DIR}"

# --- root half ---
if [[ "${IS_ROOT}" == true ]]; then
    run_system_phase
fi
SEAT_ARG="no"
[[ "${LOCAL_SEAT}" == true ]] && SEAT_ARG="yes"
SYSTEM_ARGS=(--system-phase --yes --user "${TARGET_USER}" --artifact-dir "${ARTIFACT_DIR}"
    --local-seat "${SEAT_ARG}" --distro "${FAMILY}" --pam-dir "${PAM_DIR}"
    --prefix "${PREFIX}" --sysconfdir "${SYSCONFDIR}" --localstatedir "${LOCALSTATEDIR}"
    --runstatedir "${RUNSTATEDIR}")
[[ -n "${PRESENCE}" ]] && SYSTEM_ARGS+=(--presence "${PRESENCE}")
[[ "${ENROLL}" != true ]] && SYSTEM_ARGS+=(--no-enroll)
[[ "${REINSTALL}" == true ]] && SYSTEM_ARGS+=(--reinstall)
# The root half resolves the uid of the [preview] section itself; only the decision is passed.
if [[ "${CAMERA_DAEMON}" == on && "${DAEMON_PREVIEW_STATE}" != elsewhere ]]; then
    SYSTEM_ARGS+=(--camera-view on)
fi
info "Escalating once for the system phase: ${ESCALATE[0]} bash scripts/setup.sh --system-phase ..."
rc=0
as_root bash "${SCRIPT_PATH}" "${SYSTEM_ARGS[@]}" || rc=$?

# --- user phase: the phone companion (as this user, never root) ---
# Runs after a complete root half (0) or a failed enrollment (3, which stays the exit
# code); any other exit of the root half skips it.
if [[ "${COMPANION_RUN}" != true ]]; then
    # The root half's summary cannot know it: the reason is repeated after the long run.
    info "Phone companion: skipped (${COMPANION_SKIP})."
    exit "${rc}"
fi
if [[ "${rc}" -ne 0 && "${rc}" -ne "${EXIT_INCOMPLETE}" ]]; then
    printf '[SKIP] companion: the system phase did not complete (exit %s); re-run setup once it is fixed\n' "${rc}"
    exit "${rc}"
fi
crc=0
run_companion_phase || crc=$?
if [[ "${crc}" -eq 0 ]]; then
    print_companion_guide
elif [[ "${rc}" -eq 0 ]]; then
    rc="${crc}"
fi
exit "${rc}"
