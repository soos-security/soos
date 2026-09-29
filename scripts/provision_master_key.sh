#!/bin/sh
# =============================================================================
# scripts/provision_master_key.sh — First-install master key provisioning
# =============================================================================
# Single source of truth for generating /var/lib/soos/master.key on the TARGET
# host. It is shipped as /usr/libexec/soos/provision-master-key and invoked by
# the package post-install scriptlets (debian/postinst, rpm %post,
# arch post_install) and by scripts/install.sh for a live (non-staged) install.
#
# Key material must never exist inside a package or a --destdir staging tree
# (GitHub #144 / ONB-01): every machine derives its own key at first install.
#
# Usage:
#   provision-master-key [--state-dir <DIR>]     (default: /var/lib/soos)
#
# Security invariants:
#   - The key is 32 bytes from the kernel CSPRNG (openssl rand or /dev/urandom).
#   - The file is created with mode 0600 from inception (umask 077 on a private
#     temporary file) and published atomically with a hard link, so a partially
#     written or world-readable key is never observable at the final path.
#   - An existing key is never overwritten; its mode is tightened to 0600.
#   - Symlinks and non-regular files at the key path are refused (fail closed).
#   - Output contains only paths and statuses, never key bytes.
# =============================================================================

set -eu

STATE_DIR="/var/lib/soos"

while [ $# -gt 0 ]; do
    case "$1" in
        --state-dir)
            [ $# -ge 2 ] || { echo "provision-master-key: --state-dir requires a value" >&2; exit 1; }
            STATE_DIR="$2"
            shift 2
            ;;
        -h|--help)
            echo "Usage: $(basename "$0") [--state-dir <DIR>]"
            exit 0
            ;;
        *)
            echo "provision-master-key: unknown option: $1" >&2
            exit 1
            ;;
    esac
done

STATE_DIR="${STATE_DIR%/}"
KEY_FILE="${STATE_DIR}/master.key"

# Refuse anything that is not a plain regular file at the key path.
if [ -L "${KEY_FILE}" ]; then
    echo "provision-master-key: refusing symlink at ${KEY_FILE}" >&2
    exit 1
fi
if [ -e "${KEY_FILE}" ] && [ ! -f "${KEY_FILE}" ]; then
    echo "provision-master-key: ${KEY_FILE} exists but is not a regular file" >&2
    exit 1
fi

# Existing key: keep it, only enforce the mode and ownership.
if [ -f "${KEY_FILE}" ]; then
    chmod 0600 "${KEY_FILE}"
    if [ "$(id -u)" -eq 0 ]; then
        chown root:root "${KEY_FILE}"
    fi
    echo "provision-master-key: master key already present at ${KEY_FILE} (mode 0600)."
    exit 0
fi

mkdir -p "${STATE_DIR}"

# Private temporary file in the same directory (same filesystem for the link).
TMP_FILE="${KEY_FILE}.tmp.$$"
cleanup() {
    rm -f "${TMP_FILE}"
}
trap cleanup EXIT INT TERM HUP

# Mode 0600 from inception: every file created below is 0600 at open(2) time.
umask 077

if command -v openssl >/dev/null 2>&1; then
    openssl rand 32 > "${TMP_FILE}"
else
    head -c 32 /dev/urandom > "${TMP_FILE}"
fi

# Verify the generator produced exactly 32 bytes before publishing the key.
KEY_LEN=$(wc -c < "${TMP_FILE}" | tr -d ' ')
if [ "${KEY_LEN}" != "32" ]; then
    echo "provision-master-key: generator produced ${KEY_LEN} bytes, expected 32" >&2
    exit 1
fi

chmod 0600 "${TMP_FILE}"
if [ "$(id -u)" -eq 0 ]; then
    chown root:root "${TMP_FILE}"
fi

# Atomic publish: ln(2) fails if the destination appeared meanwhile, so a key
# generated concurrently is never clobbered.
if ln "${TMP_FILE}" "${KEY_FILE}" 2>/dev/null; then
    echo "provision-master-key: master key generated at ${KEY_FILE} (mode 0600)."
elif [ -f "${KEY_FILE}" ] && [ ! -L "${KEY_FILE}" ]; then
    echo "provision-master-key: master key appeared concurrently at ${KEY_FILE}; keeping it."
else
    echo "provision-master-key: unable to publish ${KEY_FILE}" >&2
    exit 1
fi

exit 0
