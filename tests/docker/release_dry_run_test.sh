#!/usr/bin/env bash
# =============================================================================
# tests/docker/release_dry_run_test.sh — the release dry run is reproducible and
# its two tarballs alone build soos offline in a clean Arch container
# (GitHub #444, part of #361; ADR 2026-10-18 "Release Dry Run Publishes
# Nothing"; matrix RDR4, RDR5, RDR-S6)
# =============================================================================
# Phases (host side; Docker without sudo):
#
#   1. Reproducibility: scripts/release_dry_run.sh twice on HEAD, run 1 with the
#      fetch step, run 2 --offline, into two directories under target/; the
#      source tarball, the vendor tarball and SHA256SUMS must be byte-identical.
#   2. Image (network): tests/docker/Dockerfile.system-ort (the clean Arch image
#      of the system ONNX Runtime job) with Rust = the declared MSRV.
#   3. Offline build (`docker run --network none`): only the run-1 directory is
#      mounted (read-only) — never the repository, never a cargo registry —
#      and `--in-container` verifies SHA256SUMS, unpacks both tarballs, checks
#      there is no .git and no cargo registry, and builds the release workspace
#      with `--no-default-features --features system-onnxruntime` from the
#      vendored sources, then checks the link of soos-daemon and pam_soos.so.
#
# Everything produced is discarded at exit: nothing is tagged, signed,
# uploaded or published (no CI artifact either; the repository is public).
#
# Usage:
#   bash tests/docker/release_dry_run_test.sh                 # host: runs Docker
#   bash tests/docker/release_dry_run_test.sh --in-container  # inside the image
#
# Environment (host mode):
#   SOOS_RDR_KEEP            1 keeps the work directory under target/ (default: removed)
#   SOOS_RDR_JOBS            cargo build jobs in the offline container, 1..64 (default: 4)
#   SOOS_RDR_BUILD_ATTEMPTS  image build (pacman/rustup) attempts, 1..3 (default: 2)
# =============================================================================

set -euo pipefail
unset CDPATH

readonly IMAGE_TAG="soos-system-ort"
readonly DOCKERFILE="tests/docker/Dockerfile.system-ort"
readonly RUN_SUFFIX="${GITHUB_RUN_ID:-local}-$$"
readonly OUTPUTS=(SHA256SUMS)

info() { echo "[INFO]  $*"; }
ok()   { echo "[OK]    $*"; }
fail() { echo "[FAIL]  $*" >&2; exit 1; }

# Prints the declared MSRV as an exact release (1.92 -> 1.92.0).
declared_msrv() {
    local root="$1" value
    value="$(sed -n 's/^rust-version = "\([0-9][0-9.]*\)"$/\1/p' "${root}/Cargo.toml" | head -n 1)"
    [[ "${value}" =~ ^[0-9]+\.[0-9]+(\.[0-9]+)?$ ]] \
        || fail "Cargo.toml must declare [workspace.package] rust-version (got '${value}')"
    if [[ "${value}" =~ ^[0-9]+\.[0-9]+$ ]]; then
        value="${value}.0"
    fi
    echo "${value}"
}

WORK_DIR=""

cleanup() {
    if [[ -n "${WORK_DIR}" && "${SOOS_RDR_KEEP:-0}" != "1" ]]; then
        rm -rf -- "${WORK_DIR}"
    fi
}

# -----------------------------------------------------------------------------
# Host orchestration
# -----------------------------------------------------------------------------
host() {
    local root msrv work
    root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
    msrv="$(declared_msrv "${root}")"
    local jobs="${SOOS_RDR_JOBS:-4}"
    local build_attempts="${SOOS_RDR_BUILD_ATTEMPTS:-2}"
    if ! [[ "${build_attempts}" =~ ^[0-9]+$ ]] || (( build_attempts < 1 || build_attempts > 3 )); then
        fail "SOOS_RDR_BUILD_ATTEMPTS must be an integer in 1..3 (got '${build_attempts}')"
    fi
    if ! [[ "${jobs}" =~ ^[0-9]+$ ]] || (( jobs < 1 || jobs > 64 )); then
        fail "SOOS_RDR_JOBS must be an integer in 1..64 (got '${jobs}')"
    fi
    command -v docker >/dev/null 2>&1 || fail "docker is required"
    command -v cmp >/dev/null 2>&1 || fail "cmp is required"

    work="${root}/target/release-dry-run-harness-${RUN_SUFFIX}"
    [[ ! -e "${work}" ]] || fail "${work} already exists"
    # Global: the EXIT trap runs after host() returns, when its locals are gone.
    WORK_DIR="${work}"
    trap cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    mkdir -p "${work}"

    info "Phase 1: two release dry runs of HEAD (run 2 offline)"
    "${root}/scripts/release_dry_run.sh" --out-dir "${work}/run1"
    "${root}/scripts/release_dry_run.sh" --offline --out-dir "${work}/run2"
    local listing1 listing2 name
    listing1="$(cd "${work}/run1" && ls -A)"
    listing2="$(cd "${work}/run2" && ls -A)"
    [[ "${listing1}" == "${listing2}" ]] || fail "the two runs produced different file sets"
    [[ "$(wc -l <<<"${listing1}")" -eq 3 ]] || fail "expected exactly three outputs: ${listing1//$'\n'/ }"
    local -a outputs=("${OUTPUTS[@]}")
    while IFS= read -r name; do
        [[ "${name}" == "SHA256SUMS" ]] || outputs+=("${name}")
    done < <(awk '{print $2}' "${work}/run1/SHA256SUMS")
    (( ${#outputs[@]} == 3 )) || fail "SHA256SUMS must list exactly two tarballs"
    for name in "${outputs[@]}"; do
        cmp "${work}/run1/${name}" "${work}/run2/${name}" \
            || fail "${name} differs between the two runs (not reproducible)"
        ok "${name} is byte-identical across the two runs"
    done

    info "Phase 2: image ${IMAGE_TAG} (Rust ${msrv}, onnxruntime-cpu from [extra])"
    local attempt
    for (( attempt = 1; attempt <= build_attempts; attempt++ )); do
        if docker build --pull=false --build-arg "SOOS_MSRV=${msrv}" -f "${root}/${DOCKERFILE}" \
            -t "${IMAGE_TAG}" "${root}"; then
            break
        fi
        if (( attempt == build_attempts )); then
            fail "image build failed after ${build_attempts} attempt(s)"
        fi
        info "Image build attempt ${attempt}/${build_attempts} failed; retrying in $(( attempt * 30 )) s"
        sleep "$(( attempt * 30 ))"
    done

    info "Phase 3: offline build from the two tarballs only (--network none)"
    docker run --rm --network none \
        -v "${work}/run1:/release:ro" \
        -v "${root}/tests/docker/release_dry_run_test.sh:/opt/soos-rdr/release_dry_run_test.sh:ro" \
        -e "SOOS_MSRV=${msrv}" \
        -e "RUSTUP_TOOLCHAIN=${msrv}" \
        -e ORT_SKIP_DOWNLOAD=1 \
        -e CARGO_NET_OFFLINE=true \
        -e CARGO_HOME=/build/cargo-home \
        -e CARGO_TARGET_DIR=/build/target \
        -e "CARGO_BUILD_JOBS=${jobs}" \
        -e CARGO_TERM_COLOR=never \
        -e CARGO_PROFILE_RELEASE_DEBUG=0 \
        -w / "${IMAGE_TAG}" \
        bash /opt/soos-rdr/release_dry_run_test.sh --in-container
    ok "The release dry run is reproducible and its tarballs build soos offline"
}

# -----------------------------------------------------------------------------
# In-container offline phase
# -----------------------------------------------------------------------------
in_container() {
    if [[ "${SOOS_SORT_CONTAINER:-}" != "1" || ! -f /.dockerenv || "${EUID}" -ne 0 ]]; then
        fail "--in-container runs only inside the ${IMAGE_TAG} image (never on a host)"
    fi
    local msrv="${SOOS_MSRV:?SOOS_MSRV is required}"
    local target="${CARGO_TARGET_DIR:?CARGO_TARGET_DIR is required}"
    local cache="${HOME}/.cache/ort.pyke.io"

    local ifaces
    ifaces="$(ls /sys/class/net)"
    [[ "${ifaces}" == "lo" ]] || fail "network interfaces present: ${ifaces//$'\n'/ }"
    ok "No network interface besides lo"

    [[ "${ORT_SKIP_DOWNLOAD:-}" == "1" ]] || fail "ORT_SKIP_DOWNLOAD=1 is required"
    [[ -z "${LIBONNXRUNTIME_NO_PKG_CONFIG:-}" && -z "${ORT_LIB_LOCATION:-}" ]] \
        || fail "LIBONNXRUNTIME_NO_PKG_CONFIG/ORT_LIB_LOCATION must be unset (pkg-config only)"
    [[ "${CARGO_HOME:-}" == "/build/cargo-home" && ! -e "${CARGO_HOME}" ]] \
        || fail "CARGO_HOME must be a fresh, empty /build/cargo-home"
    [[ ! -e /usr/local/cargo/registry && ! -e /usr/local/cargo/git ]] \
        || fail "the image must hold no cargo registry or git cache"
    [[ ! -e "${cache}" ]] || fail "${cache} exists before the build"
    [[ ! -e /workspace/Cargo.toml ]] || fail "the repository must not be mounted"
    rustc --version | grep -q "^rustc ${msrv} " || fail "rustc is not ${msrv}: $(rustc --version)"
    ok "$(rustc --version); no cargo registry, no repository"

    cd /release
    sha256sum -c --strict SHA256SUMS
    [[ "$(wc -l <SHA256SUMS)" -eq 2 ]] || fail "SHA256SUMS must list exactly two files"
    local source_tarball vendor_tarball
    source_tarball="$(awk '$2 ~ /^soos-[^/]*\.tar\.gz$/ {print $2}' SHA256SUMS)"
    vendor_tarball="$(awk '$2 ~ /^soos-[^/]*-vendor\.tar\.zst$/ {print $2}' SHA256SUMS)"
    [[ -n "${source_tarball}" && -n "${vendor_tarball}" ]] || fail "unexpected SHA256SUMS entries"
    local prefix="${source_tarball%.tar.gz}"
    [[ "${vendor_tarball}" == "${prefix}-vendor.tar.zst" ]] || fail "tarball versions differ"
    ok "SHA256SUMS verifies ${source_tarball} and ${vendor_tarball}"

    mkdir -p /build/src
    tar -xzf "/release/${source_tarball}" -C /build/src
    tar --zstd -xf "/release/${vendor_tarball}" -C /build/src
    local tree="/build/src/${prefix}"
    [[ -f "${tree}/Cargo.lock" && -d "${tree}/vendor" ]] || fail "unpacked tree is incomplete"
    if [[ -n "$(find /build/src -name .git -print -quit)" ]]; then
        fail "the unpacked tree must not contain .git"
    fi
    grep -q '^replace-with = "vendored-sources"$' "${tree}/.cargo/config.toml" \
        && grep -q '^directory = "vendor"$' "${tree}/.cargo/config.toml" \
        || fail "${tree}/.cargo/config.toml does not select the vendored sources"
    ok "Unpacked ${prefix}: no .git, vendored sources configured"

    cd "${tree}"
    info "cargo build --locked --offline --release --workspace --no-default-features --features system-onnxruntime"
    cargo build --locked --offline --release --workspace --no-default-features --features system-onnxruntime
    ok "Release workspace built offline from the two tarballs"

    local daemon="${target}/release/soos-daemon"
    local pam_so="${target}/release/libpam_soos.so"
    [[ -x "${daemon}" && -f "${pam_so}" ]] || fail "soos-daemon or libpam_soos.so is missing"
    readelf -d "${daemon}" | grep -q 'NEEDED.*\[libonnxruntime\.so\.1\]' \
        || fail "soos-daemon does not link libonnxruntime.so.1 dynamically"
    if readelf -d "${pam_so}" | grep -q 'libonnxruntime'; then
        fail "libpam_soos.so must never link ONNX Runtime"
    fi
    ok "soos-daemon links libonnxruntime.so.1; libpam_soos.so does not"

    [[ ! -e "${cache}" ]] || fail "${cache} was created: something downloaded ONNX Runtime"
    if [[ -n "$(find "${CARGO_HOME}" -path '*/registry/cache/*' -name '*.crate' -print -quit 2>/dev/null)" ]]; then
        fail "cargo downloaded crates instead of using the vendored sources"
    fi
    ok "No download: no ort-sys cache, no registry crate"
}

case "${1:-}" in
    --in-container) in_container ;;
    "") host ;;
    *) fail "usage: $0 [--in-container]" ;;
esac
