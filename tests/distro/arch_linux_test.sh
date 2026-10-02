#!/usr/bin/env bash
# =============================================================================
# tests/distro/arch_linux_test.sh — Arch Linux Deployment Validation Test
# =============================================================================
# Sub-issue #32.3:
#   - Verify PKGBUILD build and installation via pacman
#   - Validate /etc/pam.d/system-auth integration via snippet
#   - Test swaylock and hyprlock Wayland screen locker integration
#   - Validate and document rollback procedure
#
# Usage:
#   bash tests/distro/arch_linux_test.sh [OPTIONS]
#
# Options:
#   --pkgbuild               Test installation via Arch PKGBUILD (.pkg.tar.zst)
#   --install-sh             Test installation via scripts/install.sh
#   --skip-build             Do not recompile; package the existing release artifacts
#   --dry-run                Simulate execution plan without modifying root filesystem
#   --allow-host-changes     Required for a live run: this script installs packages,
#                            rewrites PAM files and creates users on the host it runs
#                            on (run_distro_validation.sh passes it only inside a
#                            disposable Docker container, GitHub #163)
#   -h, --help               Display this help message and exit
# =============================================================================

set -euo pipefail

# Terminal Colors
if [[ -t 1 ]]; then
    readonly GREEN='\033[0;32m'
    readonly RED='\033[0;31m'
    readonly YELLOW='\033[1;33m'
    readonly BLUE='\033[0;34m'
    readonly BOLD='\033[1m'
    readonly NC='\033[0m'
else
    readonly GREEN=''
    readonly RED=''
    readonly YELLOW=''
    readonly BLUE=''
    readonly BOLD=''
    readonly NC=''
fi

info()    { echo -e "${BLUE}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[ERROR]${NC} $*" >&2; }

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

INSTALL_MODE="auto" # auto, pkgbuild, install-sh
SKIP_BUILD=false
DRY_RUN=false
ALLOW_HOST_CHANGES=false
TEST_USER="testuser"
TEST_PASS="password123"

usage() {
    cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Executes Arch Linux deployment, PKGBUILD packaging, system-auth integration,
swaylock and hyprlock screen locker testing, and rollback validation suite.

Options:
  --pkgbuild               Install via Arch Linux PKGBUILD package
  --install-sh             Install via scripts/install.sh
  --skip-build             Skip cargo build and use the existing release binaries
  --dry-run                Print execution plan without making root changes
  --allow-host-changes     Consent to a live run that modifies this host (packages,
                           PAM files, users); use it only in a disposable container
  -h, --help               Display this help message and exit

Security Invariants Tested:
  - Arch Linux PKGBUILD specification & pacman install invariants
  - Arch /etc/pam.d/system-auth snippet integration
  - Wayland screen locker integration (swaylock & hyprlock)
  - /var/lib/soos/{biometrics,evidence} mode 0700 (root:root)
  - /var/lib/soos/master.key mode 0600 (root:root, 32 bytes)
  - Nominal facial auth (Verdict::Allow -> PAM_SUCCESS, 0 password prompts)
  - Password fallback (Verdict::Deny / offline daemon -> password authentication)
  - Face login resets the pam_faillock tally; a locked account still fails
  - Safe rollback and uninstallation via pacman -R or uninstall.sh
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --pkgbuild)
            INSTALL_MODE="pkgbuild"
            shift
            ;;
        --install-sh)
            INSTALL_MODE="install-sh"
            shift
            ;;
        --skip-build)
            SKIP_BUILD=true
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --allow-host-changes)
            ALLOW_HOST_CHANGES=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            error "Unknown argument: $1"
            usage
            exit 1
            ;;
    esac
done

echo ""
info "==================================================================="
info "  SOOS — Arch Linux Deployment Validation (#32.3)"
info "==================================================================="
info "Workspace:    ${WORKSPACE_ROOT}"
info "Install Mode: ${INSTALL_MODE}"
info "Dry Run:      ${DRY_RUN}"
echo ""

if [[ "${DRY_RUN}" = true ]]; then
    info "[DRY RUN] Simulating Arch Linux deployment plan:"
    info "  1. Build / verify release binaries and PKGBUILD package"
    info "  2. Install via ${INSTALL_MODE} (pacman -U or scripts/install.sh)"
    info "  3. Verify filesystem permissions (biometrics: 0700, master.key: 0600)"
    info "  4. Verify /etc/pam.d/system-auth configuration and snippet"
    info "  5. Configure and test swaylock and hyprlock PAM services"
    info "  6. Test nominal screen unlocking via facial authentication (0 prompts)"
    info "  7. Test screen unlocking password fallback (valid succeeds, invalid rejected)"
    info "  7b. Face login resets the faillock tally; a locked account still fails"
    info "  8. Execute rollback procedure and restore clean system state"
    success "Dry run deployment validation completed successfully."
    exit 0
fi

# Explicit consent for live execution (GitHub #163): refuse before any command,
# trap or file operation so that an accidental run never touches the host.
if [[ "${ALLOW_HOST_CHANGES}" != true ]]; then
    error "Refusing a live run without --allow-host-changes: this suite installs packages,"
    error "rewrites PAM files and creates users. Run it through"
    error "tests/distro/run_distro_validation.sh (disposable Docker container) or use --dry-run."
    exit 2
fi

# Root check for live execution
if [[ "$(id -u)" -ne 0 ]]; then
    error "Live deployment testing requires root privileges (or use --dry-run)."
    exit 1
fi

cleanup() {
    info "Executing test cleanup..."
    # Stop the mock by PID first: minimal images (fedora:40) ship without pkill.
    if [[ -n "${MOCK_PID:-}" ]]; then
        kill "${MOCK_PID}" 2>/dev/null || true
        wait "${MOCK_PID}" 2>/dev/null || true
        MOCK_PID=""
    fi
    pkill -f "mock_daemon.py" 2>/dev/null || true
    rm -f /run/soos/daemon.sock
}
trap cleanup EXIT INT TERM

# ---------------------------------------------------------------------------
# Step 1: Package Build / Staging
# ---------------------------------------------------------------------------
cd "${WORKSPACE_ROOT}"

# Without --skip-build, always invoke cargo (a no-op when up to date) so a stale
# release artifact from the bind-mounted target/ is never packaged (GitHub #244).
if [[ "${SKIP_BUILD}" = false ]]; then
    # Bounded retry of the ort-sys ONNX Runtime download (GitHub #318).
    bash scripts/prefetch_onnxruntime.sh
    info "Building release artifacts (no-op when up to date)..."
    cargo build --locked --release --workspace
fi

# ---------------------------------------------------------------------------
# Step 2: Installation (PKGBUILD / pacman or scripts/install.sh)
# ---------------------------------------------------------------------------
if [[ "${INSTALL_MODE}" = "auto" || "${INSTALL_MODE}" = "pkgbuild" ]]; then
    if command -v pacman >/dev/null 2>&1; then
        info "Building Arch Linux PKGBUILD package..."
        bash scripts/build_arch.sh --skip-build
        PKG_FILE=$(ls -t target/packages/soos-*.pkg.tar.* | head -n 1)
        info "Installing package via pacman -U: ${PKG_FILE}..."
        pacman -U --noconfirm "${PKG_FILE}"
        INSTALL_MODE="pkgbuild"
        success "Arch package installed successfully via pacman."
    else
        warn "pacman not found, falling back to scripts/install.sh..."
        INSTALL_MODE="install-sh"
    fi
fi

if [[ "${INSTALL_MODE}" = "install-sh" ]]; then
    info "Installing via scripts/install.sh..."
    bash scripts/install.sh --skip-models --skip-systemd
    success "scripts/install.sh completed successfully."
fi

# ---------------------------------------------------------------------------
# Step 3: Filesystem Invariants & Permissions Verification
# ---------------------------------------------------------------------------
info "Verifying installed filesystem invariants..."
test -f "/usr/libexec/soos/soos-daemon" || { error "soos-daemon binary missing"; exit 1; }
test -f "/usr/bin/soos-admin" || { error "soos-admin binary missing"; exit 1; }
test -f "/usr/bin/soos-enroll" || { error "soos-enroll binary missing"; exit 1; }

PAM_TARGET=""
for candidate in "/usr/lib/security" "/lib/security"; do
    if [[ -f "${candidate}/pam_soos.so" ]]; then
        PAM_TARGET="${candidate}/pam_soos.so"
        break
    fi
done

if [[ -z "${PAM_TARGET}" ]]; then
    error "pam_soos.so module not found in system security directories."
    exit 1
fi
info "Located PAM module at ${PAM_TARGET}"

test -d "/var/lib/soos/biometrics" || { error "/var/lib/soos/biometrics missing"; exit 1; }
test -d "/var/lib/soos/evidence" || { error "/var/lib/soos/evidence missing"; exit 1; }
test -f "/var/lib/soos/master.key" || { error "/var/lib/soos/master.key missing"; exit 1; }

BIO_PERMS=$(stat -c "%a" "/var/lib/soos/biometrics" 2>/dev/null || stat -f "%OLp" "/var/lib/soos/biometrics")
if [[ "${BIO_PERMS}" != "700" ]]; then
    error "Invalid permissions on /var/lib/soos/biometrics: ${BIO_PERMS} (expected 700)"
    exit 1
fi

KEY_PERMS=$(stat -c "%a" "/var/lib/soos/master.key" 2>/dev/null || stat -f "%OLp" "/var/lib/soos/master.key")
if [[ "${KEY_PERMS}" != "600" ]]; then
    error "Invalid permissions on /var/lib/soos/master.key: ${KEY_PERMS} (expected 600)"
    exit 1
fi
success "All directory hierarchy and permission invariants verified."

# ---------------------------------------------------------------------------
# Step 4: Documented edit applied to the REAL stock /etc/pam.d/system-auth
# ---------------------------------------------------------------------------
# GitHub #309: the image keeps the pristine pambase system-auth (Dockerfile.arch);
# it is verified against the pambase mtree digest, the documented soos edit
# (Docs/DISTRIBUTION_DEPLOYMENT.md section 5.2, packaging/pam/arch/system-auth.snippet)
# is applied to it, and the result must equal packaging/pam/arch/system-auth.
info "Applying the documented soos edit to the stock pambase /etc/pam.d/system-auth..."
test -f "packaging/pam/arch/system-auth.snippet" || { error "Arch PAM snippet missing"; exit 1; }
STOCK_SYSTEM_AUTH="/usr/local/share/soos-test/system-auth.stock"
test -f "${STOCK_SYSTEM_AUTH}" || { error "${STOCK_SYSTEM_AUTH} missing (tests/docker/Dockerfile.arch keeps it)"; exit 1; }

PAMBASE_MTREE="$(find /var/lib/pacman/local -maxdepth 2 -path '/var/lib/pacman/local/pambase-*/mtree' | head -n 1)"
[[ -n "${PAMBASE_MTREE}" ]] || { error "pambase is not installed (no /var/lib/pacman/local/pambase-*/mtree)"; exit 1; }
STOCK_EXPECTED_SHA="$(zcat "${PAMBASE_MTREE}" | awk '$1 == "./etc/pam.d/system-auth"' \
    | sed -nE 's/.*sha256digest=([0-9a-f]{64}).*/\1/p')"
STOCK_ACTUAL_SHA="$(sha256sum "${STOCK_SYSTEM_AUTH}" | cut -d' ' -f1)"
if [[ -z "${STOCK_EXPECTED_SHA}" || "${STOCK_EXPECTED_SHA}" != "${STOCK_ACTUAL_SHA}" ]]; then
    error "The saved system-auth is not the stock pambase file (mtree ${STOCK_EXPECTED_SHA:-none}, file ${STOCK_ACTUAL_SHA})."
    exit 1
fi
success "Stock system-auth matches the pambase mtree digest (${PAMBASE_MTREE%/mtree})."

# The four-line edit of section 5.2: primary rule before pam_systemd_home.so, event
# rule right after pam_unix.so, both stock success jumps widened by one. Fails when
# the stock lines it anchors on are not found (pambase changed: review the edit).
apply_soos_arch_edit() {
    awk '
        /^-auth[ \t]+\[success=2 default=ignore\][ \t]+pam_systemd_home\.so/ && !home {
            print "auth  [success=4 default=ignore]  pam_soos.so"
            sub(/success=2/, "success=3")
            print
            home = 1
            next
        }
        /^auth[ \t]+\[success=1 default=bad\][ \t]+pam_unix\.so/ && !unix {
            sub(/success=1/, "success=2")
            print
            print "auth  optional                       pam_soos.so event=password-failed timeout_ms=20"
            unix = 1
            next
        }
        { print }
        END { if (!home || !unix) exit 1 }
    '
}

EDITED_SYSTEM_AUTH="$(mktemp)"
if ! apply_soos_arch_edit < "${STOCK_SYSTEM_AUTH}" > "${EDITED_SYSTEM_AUTH}"; then
    error "The documented edit does not apply to the stock pambase system-auth (anchors not found)."
    exit 1
fi
if ! diff -u "packaging/pam/arch/system-auth" "${EDITED_SYSTEM_AUTH}"; then
    error "packaging/pam/arch/system-auth differs from the stock stack with the documented edit (diff above)."
    exit 1
fi
success "Stock system-auth + documented edit == packaging/pam/arch/system-auth."

SYNTHETIC_SYSTEM_AUTH_BACKUP="$(mktemp)"
cp -p /etc/pam.d/system-auth "${SYNTHETIC_SYSTEM_AUTH_BACKUP}"
install -m 0644 "${EDITED_SYSTEM_AUTH}" /etc/pam.d/system-auth
rm -f "${EDITED_SYSTEM_AUTH}"

# ---------------------------------------------------------------------------
# Step 5: Screen Locker Integration (swaylock / hyprlock)
# ---------------------------------------------------------------------------
info "Configuring Wayland screen locker PAM stacks (swaylock, hyprlock) on the edited system-auth..."

# Arch swaylock and hyprlock include system-auth.
cat << 'EOF' > /etc/pam.d/test-swaylock
#%PAM-1.0
auth include system-auth
account include system-auth
EOF

cat << 'EOF' > /etc/pam.d/test-hyprlock
#%PAM-1.0
auth include system-auth
account include system-auth
EOF

# Ensure test user exists
if ! id -u "${TEST_USER}" >/dev/null 2>&1; then
    useradd -m -s /bin/bash "${TEST_USER}"
    echo "${TEST_USER}:${TEST_PASS}" | chpasswd
fi

# Add user to soos system group
soos-admin add-user "${TEST_USER}" || usermod -aG soos "${TEST_USER}"

# Enroll through the real CLI with the mock camera and synthetic inference
# (GitHub #168). soos-biometric-store writes <uid>.cbor.enc, encrypted with
# /var/lib/soos/master.key; a hand-made file would prove nothing.
TEST_UID="$(id -u "${TEST_USER}")"
ENROLLED_TEMPLATE="/var/lib/soos/biometrics/${TEST_UID}.cbor.enc"
soos-enroll --mock enroll --username "${TEST_USER}" --yes \
    || { error "soos-enroll --mock enroll failed for '${TEST_USER}'"; exit 1; }
test -f "${ENROLLED_TEMPLATE}" || { error "Enrollment did not create ${ENROLLED_TEMPLATE}"; exit 1; }
TEMPLATE_MODES="$(stat -c '%a %U:%G' "${ENROLLED_TEMPLATE}")"
[[ "${TEMPLATE_MODES}" == "600 root:root" ]] \
    || { error "${ENROLLED_TEMPLATE} is '${TEMPLATE_MODES}', expected '600 root:root'"; exit 1; }
soos-enroll --mock verify --username "${TEST_USER}" \
    || { error "soos-enroll --mock verify rejected the freshly enrolled template"; exit 1; }
success "User '${TEST_USER}' enrolled via soos-enroll --mock: ${ENROLLED_TEMPLATE} (600 root:root)."

# Compile pam_test_runner if needed
if [[ ! -x "/usr/local/bin/pam_test_runner" ]]; then
    gcc -O2 tests/docker/pam_test_runner.c -lpam -o /usr/local/bin/pam_test_runner
    chmod 755 /usr/local/bin/pam_test_runner
fi

# Socket directory invariant (GitHub #168): /run/soos is 0750 root:soos and the
# socket 0660 root:soos, exactly as the daemon and the packages create them.
# The PAM host (pam_test_runner) runs as root, like sudo/login/gdm.
install -d -m 0750 -o root -g soos /run/soos

assert_socket_modes() {
    local dir_modes sock_modes
    dir_modes="$(stat -c '%a %U:%G' /run/soos)"
    [[ "${dir_modes}" == "750 root:soos" ]] \
        || { error "/run/soos is '${dir_modes}', expected '750 root:soos'"; exit 1; }
    [[ -S /run/soos/daemon.sock ]] || { error "/run/soos/daemon.sock is not a socket"; exit 1; }
    sock_modes="$(stat -c '%a %U:%G' /run/soos/daemon.sock)"
    [[ "${sock_modes}" == "660 root:soos" ]] \
        || { error "/run/soos/daemon.sock is '${sock_modes}', expected '660 root:soos'"; exit 1; }
}

EVENTS_LOG="$(mktemp)"
start_mock_daemon() {
    : > "${EVENTS_LOG}"
    python3 tests/docker/mock_daemon.py --stamps monotonic --socket /run/soos/daemon.sock \
        --record "${EVENTS_LOG}" "$@" &
    MOCK_PID=$!
    for _ in $(seq 1 50); do
        [[ -S /run/soos/daemon.sock ]] && break
        sleep 0.1
    done
    assert_socket_modes
}

# Number of PasswordFailed events the recording mock daemon received.
password_failed_events() {
    sleep 0.3
    grep -c '^event kind=password-failed' "${EVENTS_LOG}" || true
}

# Current pam_faillock tally of the test user (one line per recorded failure).
faillock_tally() {
    faillock --user "${TEST_USER}" | grep -cE '^[0-9]{4}-[0-9]{2}-[0-9]{2}' || true
}

assert_tally() {
    local expected="$1" label="$2" tally
    tally="$(faillock_tally)"
    if [[ "${tally}" != "${expected}" ]]; then
        error "${label}: faillock tally is ${tally}, expected ${expected}"
        faillock --user "${TEST_USER}" >&2 || true
        exit 1
    fi
}

assert_events() {
    local expected="$1" label="$2" count
    count="$(password_failed_events)"
    if [[ "${count}" != "${expected}" ]]; then
        error "${label}: ${count} PasswordFailed event(s) received, expected ${expected}"
        cat "${EVENTS_LOG}" >&2
        exit 1
    fi
}

# pam_faillock deny limit: the last "deny = N" of /etc/security/faillock.conf,
# else the pam_faillock built-in default of 3.
faillock_deny_limit() {
    local deny
    deny="$(sed -nE 's/^[[:space:]]*deny[[:space:]]*=[[:space:]]*([0-9]+)[[:space:]]*$/\1/p' \
        /etc/security/faillock.conf 2>/dev/null | tail -n 1)"
    printf '%s' "${deny:-3}"
}

# Runs <count> wrong-password logins through the edited stack (daemon absent).
wrong_passwords() {
    local count="$1" i
    for (( i = 0; i < count; i++ )); do
        if /usr/local/bin/pam_test_runner test-swaylock "${TEST_USER}" "wrong_password" 2>/dev/null; then
            error "Security Invariant Violation: Invalid password accepted by swaylock!"
            exit 1
        fi
    done
}

faillock --user "${TEST_USER}" --reset

# ---------------------------------------------------------------------------
# Step 6: PAM flows through the edited stock stack (swaylock & hyprlock)
# ---------------------------------------------------------------------------
# 6a/6b. Nominal facial unlock: Allow -> success, no event, no faillock entry.
for locker in test-swaylock test-hyprlock; do
    info "Testing nominal facial unlock for ${locker}..."
    start_mock_daemon --mode allow
    if /usr/local/bin/pam_test_runner "${locker}" "${TEST_USER}"; then
        success "${locker} authenticated via facial verification without password prompt."
    else
        error "${locker} failed nominal facial authentication."
        exit 1
    fi
    assert_events 0 "${locker} face Allow"
    assert_tally 0 "${locker} face Allow"
    cleanup
done

# 6c. Password fallback with the daemon absent: success, faillock tally 0.
info "Testing screen locker password fallback (daemon absent)..."
if /usr/local/bin/pam_test_runner test-swaylock "${TEST_USER}" "${TEST_PASS}"; then
    success "swaylock cleanly fell back to password authentication."
else
    error "swaylock rejected valid password during fallback (daemon absent)."
    exit 1
fi
assert_tally 0 "valid password, daemon absent"
success "Valid password with the daemon absent: faillock tally 0."

# 6d. Face Deny + valid password: success, NO PasswordFailed event, tally 0.
info "Testing valid password after a face Deny (recording daemon)..."
start_mock_daemon --mode deny
if ! /usr/local/bin/pam_test_runner test-swaylock "${TEST_USER}" "${TEST_PASS}"; then
    error "swaylock rejected the valid password after a face Deny."
    exit 1
fi
assert_events 0 "valid password after face Deny"
assert_tally 0 "valid password after face Deny"
success "Valid password: accepted, no PasswordFailed event, faillock tally 0."

# 6e. Wrong password: failure, exactly one PasswordFailed event, tally 1.
info "Testing screen locker rejection of an incorrect password..."
if /usr/local/bin/pam_test_runner test-swaylock "${TEST_USER}" "wrong_password" 2>/dev/null; then
    error "Security Invariant Violation: Invalid password accepted by swaylock!"
    exit 1
fi
assert_events 1 "wrong password"
assert_tally 1 "wrong password"
success "Wrong password: rejected, exactly one PasswordFailed event, faillock tally 1."
cleanup
faillock --user "${TEST_USER}" --reset

# 6f. Wrong password with the daemon absent: still rejected (no bypass).
if /usr/local/bin/pam_test_runner test-swaylock "${TEST_USER}" "wrong_password" 2>/dev/null; then
    error "Security Invariant Violation: Invalid password accepted with the daemon absent!"
    exit 1
fi
success "swaylock rejected the invalid password with the daemon absent."
faillock --user "${TEST_USER}" --reset

# 6g/6h (GitHub #318): a face match is a successful login for pam_faillock. The
# primary rule jumps to pam_permit.so, so pam_env.so and pam_faillock.so authsucc
# run exactly as after a correct password: the tally is reset, while a locked
# account still fails (preauth and authsucc both refuse it).
FAILLOCK_DENY="$(faillock_deny_limit)"
if [[ ! "${FAILLOCK_DENY}" =~ ^[0-9]+$ ]] || (( FAILLOCK_DENY < 2 )); then
    error "pam_faillock deny limit '${FAILLOCK_DENY}' is unusable (need an integer >= 2)."
    exit 1
fi
BELOW_DENY=$(( FAILLOCK_DENY - 1 ))
info "pam_faillock deny limit: ${FAILLOCK_DENY} (/etc/security/faillock.conf or built-in default)."

# 6g. N wrong passwords (N = deny - 1, not locked), then face Allow: success, tally 0.
info "Testing that a face login resets ${BELOW_DENY} recorded password failure(s)..."
wrong_passwords "${BELOW_DENY}"
assert_tally "${BELOW_DENY}" "${BELOW_DENY} wrong password(s) before the face login"
start_mock_daemon --mode allow
if ! /usr/local/bin/pam_test_runner test-swaylock "${TEST_USER}"; then
    error "swaylock rejected the face Allow after ${BELOW_DENY} wrong password(s) (account not locked)."
    exit 1
fi
assert_events 0 "face Allow after ${BELOW_DENY} wrong password(s)"
assert_tally 0 "face Allow after ${BELOW_DENY} wrong password(s)"
cleanup
success "Face Allow after ${BELOW_DENY} wrong password(s): accepted, faillock tally reset to 0."

# 6h. deny wrong passwords (locked), then face Allow: still rejected, tally kept.
info "Testing that a locked account still fails with a face Allow..."
faillock --user "${TEST_USER}" --reset
wrong_passwords "${FAILLOCK_DENY}"
assert_tally "${FAILLOCK_DENY}" "${FAILLOCK_DENY} wrong passwords (locked account)"
start_mock_daemon --mode allow
if /usr/local/bin/pam_test_runner test-swaylock "${TEST_USER}" 2>/dev/null; then
    error "Security Invariant Violation: a face Allow unlocked an account locked by pam_faillock!"
    exit 1
fi
assert_events 0 "face Allow on a locked account"
assert_tally "${FAILLOCK_DENY}" "face Allow on a locked account"
cleanup
success "Locked account: face Allow rejected, faillock tally kept at ${FAILLOCK_DENY}."
faillock --user "${TEST_USER}" --reset

# Restore the image's synthetic system-auth (used by tests/docker/test_suite.sh).
install -m 0644 "${SYNTHETIC_SYSTEM_AUTH_BACKUP}" /etc/pam.d/system-auth
rm -f "${SYNTHETIC_SYSTEM_AUTH_BACKUP}" "${EVENTS_LOG}"

# ---------------------------------------------------------------------------
# Step 7: Rollback Procedure Verification
# ---------------------------------------------------------------------------
info "Validating safe uninstallation and rollback procedure..."
if [[ "${INSTALL_MODE}" = "pkgbuild" ]] && command -v pacman >/dev/null 2>&1; then
    info "Removing package via pacman -R soos..."
    pacman -R --noconfirm soos
    test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after pacman -R"; exit 1; }
    success "pacman -R rollback removed binaries cleanly."
else
    info "Executing rollback via scripts/uninstall.sh --keep-data..."
    bash scripts/uninstall.sh --keep-data --skip-systemd
    test ! -f "/usr/bin/soos-admin" || { error "soos-admin still present after uninstall"; exit 1; }
    success "scripts/uninstall.sh rollback completed cleanly."
fi

# Clean up test files
rm -f /etc/pam.d/test-swaylock /etc/pam.d/test-hyprlock "${ENROLLED_TEMPLATE}"

echo ""
success "==================================================================="
success "  Arch Linux Deployment Validation Succeeded (100%)"
success "==================================================================="
exit 0
