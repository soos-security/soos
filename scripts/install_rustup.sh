#!/usr/bin/env bash
# =============================================================================
# install_rustup.sh — Verified, pinned Rust toolchain bootstrap for soos
# =============================================================================
#
# Replaces `curl https://sh.rustup.rs | sh` (GitHub #260, ONB-16):
#   1. downloads `rustup-init` of a pinned rustup release for the host triple
#      from the versioned archive on static.rust-lang.org (HTTPS / TLS 1.2+);
#   2. verifies it against the SHA-256 digest committed below BEFORE it is made
#      executable; a mismatch aborts and nothing is executed;
#   3. runs it with `--profile minimal` and the exact toolchain release pinned by
#      rust-toolchain.toml (floating channels such as `stable` are refused).
#
# Used by the sandbox Dockerfiles (COPY + RUN) and by developers on a fresh host:
#   ./scripts/install_rustup.sh                          # installs 1.98.1
#   ./scripts/install_rustup.sh --default-toolchain 1.98.1
#
# Bumping: change RUSTUP_VERSION and both digests together (take them from
# https://static.rust-lang.org/rustup/archive/<version>/<triple>/rustup-init.sha256
# and cross-check a local `sha256sum` of the downloaded binary). The toolchain
# pin follows rust-toolchain.toml (tests/invariants/src/rustup_bootstrap_contract.rs).
#
# Environment passed through to rustup-init: CARGO_HOME, RUSTUP_HOME,
# RUSTUP_INIT_SKIP_PATH_CHECK.
#
# Exit codes: 0 installed, 1 download / verification / install failure,
#             2 usage error (bad argument, floating channel).
# =============================================================================

set -euo pipefail

RUSTUP_VERSION="1.29.1"
RUSTUP_INIT_SHA256_X86_64="dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71"
RUSTUP_INIT_SHA256_AARCH64="15f6e4ce9f583b929c996c91562bad6d4454f3281de858b02cdfdef615fac433"
PINNED_TOOLCHAIN="1.98.1"

ARCHIVE_BASE_URL="https://static.rust-lang.org/rustup/archive"

usage() {
    cat <<EOF
Usage: $(basename "$0") [--default-toolchain <x.y.z>]

Downloads rustup-init ${RUSTUP_VERSION}, verifies its pinned SHA-256 digest and
installs the exact Rust release (default ${PINNED_TOOLCHAIN}, from rust-toolchain.toml).
EOF
}

die() {
    echo "[install_rustup] ERROR: $*" >&2
    exit 1
}

usage_error() {
    echo "[install_rustup] ERROR: $*" >&2
    usage >&2
    exit 2
}

toolchain="${PINNED_TOOLCHAIN}"
while [ $# -gt 0 ]; do
    case "$1" in
        --default-toolchain)
            [ $# -ge 2 ] || usage_error "--default-toolchain needs a value"
            toolchain="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage_error "unknown argument: $1"
            ;;
    esac
done

# Only an exact release: a floating channel (stable, beta, nightly) or a partial
# version would make the bootstrap non-reproducible.
if ! [[ "${toolchain}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    usage_error "--default-toolchain must be an exact release x.y.z (got '${toolchain}')"
fi
if [ "${toolchain}" != "${PINNED_TOOLCHAIN}" ]; then
    echo "[install_rustup] WARNING: installing ${toolchain}; rust-toolchain.toml pins ${PINNED_TOOLCHAIN}" >&2
fi

arch="$(uname -m)"
case "${arch}" in
    x86_64|amd64)
        triple="x86_64-unknown-linux-gnu"
        expected="${RUSTUP_INIT_SHA256_X86_64}"
        ;;
    aarch64|arm64)
        triple="aarch64-unknown-linux-gnu"
        expected="${RUSTUP_INIT_SHA256_AARCH64}"
        ;;
    *)
        die "unsupported architecture '${arch}': no pinned rustup-init digest (supported: x86_64, aarch64)"
        ;;
esac

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required"

workdir="$(mktemp -d "${TMPDIR:-/tmp}/soos-rustup.XXXXXXXX")"
cleanup() {
    rm -rf "${workdir}"
}
trap cleanup EXIT

url="${ARCHIVE_BASE_URL}/${RUSTUP_VERSION}/${triple}/rustup-init"
echo "[install_rustup] Downloading rustup-init ${RUSTUP_VERSION} (${triple})"
curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    --max-time 300 -o "${workdir}/rustup-init" "${url}" \
    || die "download failed: ${url}"

actual="$(sha256sum "${workdir}/rustup-init" | cut -d' ' -f1)"
if [ "${actual}" != "${expected}" ]; then
    die "rustup-init checksum mismatch (expected ${expected}, got ${actual}); refusing to execute it"
fi
echo "[install_rustup] rustup-init SHA-256 verified"

chmod 0755 "${workdir}/rustup-init"
"${workdir}/rustup-init" -y --profile minimal \
    --default-toolchain "${toolchain}" \
    || die "rustup-init failed"
echo "[install_rustup] Rust ${toolchain} installed"
