#!/usr/bin/env bash
# =============================================================================
# tests/docker/system_onnxruntime_test.sh — offline distribution build against
# the system ONNX Runtime in a clean Arch container (GitHub #375, PR 1 of #361;
# ADR 2026-10-13 "Distribution Builds: System ONNX Runtime"; matrix SORT8, SORT10)
# =============================================================================
# Phases (host side; Docker without sudo):
#
#   1. Image (network): tests/docker/Dockerfile.system-ort with Rust = the
#      declared MSRV, read from `[workspace.package] rust-version` in Cargo.toml.
#   2. Fetch (network, separate step): `cargo fetch --locked` into a named
#      registry volume, and the attested models with scripts/download_models.sh
#      (HTTPS, size-bounded, SHA-256 verified) into a named models volume.
#      SOOS_SORT_MODELS=host uses the host /var/lib/soos/models read-only instead
#      (local runs; it must hold a manifest.toml).
#   3. Offline (`docker run --network none`, repository mounted read-only):
#      `--in-container` asserts the container has no network, no ort-sys
#      download cache and no download code in the resolved feature graph,
#      type-checks every target and feature at the MSRV, builds
#      the release workspace with `--no-default-features --features
#      system-onnxruntime`, checks that soos-daemon links the distribution
#      libonnxruntime and pam_soos.so does not, runs the real-model tests against
#      it, then removes the library and checks the daemon refuses to start.
#
# Usage:
#   bash tests/docker/system_onnxruntime_test.sh                 # host: runs Docker
#   bash tests/docker/system_onnxruntime_test.sh --in-container  # inside the image
#
# Environment (host mode):
#   SOOS_SORT_MODELS   download (default) or host
#   SOOS_SORT_KEEP     1 keeps the named volumes after the run (default: removed)
#   SOOS_SORT_JOBS     cargo build jobs in the offline container, 1..64 (default: 4)
#   SOOS_SORT_BUILD_ATTEMPTS  image build (pacman/rustup) attempts, 1..3 (default: 2)
# =============================================================================

set -euo pipefail

readonly IMAGE_TAG="soos-system-ort"
readonly DOCKERFILE="tests/docker/Dockerfile.system-ort"
# Per-run volume names: concurrent runs never share or delete each other's volumes.
readonly RUN_SUFFIX="${GITHUB_RUN_ID:-local}-$$"
readonly REGISTRY_VOLUME="soos-sort-cargo-registry-${RUN_SUFFIX}"
readonly TARGET_VOLUME="soos-sort-target-${RUN_SUFFIX}"
readonly MODELS_VOLUME="soos-sort-models-${RUN_SUFFIX}"
readonly HOST_MODELS_DIR="/var/lib/soos/models"
readonly SYSTEM_FEATURES=(--no-default-features --features system-onnxruntime)

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

# -----------------------------------------------------------------------------
# Host orchestration
# -----------------------------------------------------------------------------
host() {
    local root msrv models_source models_mount
    root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
    msrv="$(declared_msrv "${root}")"
    models_source="${SOOS_SORT_MODELS:-download}"
    local jobs="${SOOS_SORT_JOBS:-4}"
    local build_attempts="${SOOS_SORT_BUILD_ATTEMPTS:-2}"
    if ! [[ "${build_attempts}" =~ ^[0-9]+$ ]] || (( build_attempts < 1 || build_attempts > 3 )); then
        fail "SOOS_SORT_BUILD_ATTEMPTS must be an integer in 1..3 (got '${build_attempts}')"
    fi
    if ! [[ "${jobs}" =~ ^[0-9]+$ ]] || (( jobs < 1 || jobs > 64 )); then
        fail "SOOS_SORT_JOBS must be an integer in 1..64 (got '${jobs}')"
    fi
    command -v docker >/dev/null 2>&1 || fail "docker is required"

    cleanup() {
        if [[ "${SOOS_SORT_KEEP:-0}" != "1" ]]; then
            docker volume rm -f "${REGISTRY_VOLUME}" "${TARGET_VOLUME}" "${MODELS_VOLUME}" >/dev/null 2>&1 || true
        fi
    }
    trap cleanup EXIT

    info "Phase 1: image ${IMAGE_TAG} (Rust ${msrv}, onnxruntime-cpu from [extra])"
    # Bounded retry of the image build (pacman -Syu against live mirrors, rustup): a
    # transient mirror or CDN failure is retried; a persistent one still fails the run.
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

    info "Phase 2: fetch crates and models (network)"
    docker run --rm \
        -v "${root}:/workspace:ro" \
        -v "${REGISTRY_VOLUME}:/usr/local/cargo/registry" \
        -e "RUSTUP_TOOLCHAIN=${msrv}" \
        -w /workspace "${IMAGE_TAG}" \
        cargo fetch --locked
    case "${models_source}" in
        host)
            [[ -f "${HOST_MODELS_DIR}/manifest.toml" ]] \
                || fail "SOOS_SORT_MODELS=host but ${HOST_MODELS_DIR}/manifest.toml is missing"
            models_mount="${HOST_MODELS_DIR}:/models:ro"
            ;;
        download)
            docker run --rm \
                -v "${root}:/workspace:ro" \
                -v "${MODELS_VOLUME}:/models" \
                -w /workspace "${IMAGE_TAG}" \
                bash /workspace/scripts/download_models.sh --target-dir /models \
                    --manifest /workspace/models/manifest.toml
            models_mount="${MODELS_VOLUME}:/models:ro"
            ;;
        *)
            fail "SOOS_SORT_MODELS must be download or host (got '${models_source}')"
            ;;
    esac

    info "Phase 3: offline build and tests (--network none)"
    docker run --rm --network none \
        -v "${root}:/workspace:ro" \
        -v "${REGISTRY_VOLUME}:/usr/local/cargo/registry" \
        -v "${TARGET_VOLUME}:/target" \
        -v "${models_mount}" \
        -e "SOOS_MSRV=${msrv}" \
        -e "RUSTUP_TOOLCHAIN=${msrv}" \
        -e ORT_SKIP_DOWNLOAD=1 \
        -e CARGO_NET_OFFLINE=true \
        -e CARGO_TARGET_DIR=/target \
        -e "CARGO_BUILD_JOBS=${jobs}" \
        -e CARGO_TERM_COLOR=never \
        -e CARGO_PROFILE_DEV_DEBUG=0 \
        -e CARGO_PROFILE_TEST_DEBUG=0 \
        -w /workspace "${IMAGE_TAG}" \
        bash /workspace/tests/docker/system_onnxruntime_test.sh --in-container
    ok "System ONNX Runtime distribution build verified offline"
}

# -----------------------------------------------------------------------------
# In-container offline phase
# -----------------------------------------------------------------------------
in_container() {
    # Destructive steps below (pacman -Rdd): run only inside the soos image, as its root user.
    if [[ "${SOOS_SORT_CONTAINER:-}" != "1" || ! -f /.dockerenv || "${EUID}" -ne 0 ]]; then
        fail "--in-container runs only inside the ${IMAGE_TAG} image (never on a host)"
    fi
    local msrv="${SOOS_MSRV:?SOOS_MSRV is required}"
    local target="${CARGO_TARGET_DIR:?CARGO_TARGET_DIR is required}"
    local cache="${HOME}/.cache/ort.pyke.io"

    cd /workspace

    # Network really disabled: only the loopback interface exists.
    local ifaces
    ifaces="$(ls /sys/class/net)"
    [[ "${ifaces}" == "lo" ]] || fail "network interfaces present: ${ifaces//$'\n'/ }"
    ok "No network interface besides lo"

    [[ "${ORT_SKIP_DOWNLOAD:-}" == "1" ]] || fail "ORT_SKIP_DOWNLOAD=1 is required"
    [[ -z "${ORT_LIB_LOCATION:-}" && -z "${ORT_LIB_PATH:-}" ]] \
        || fail "ORT_LIB_LOCATION/ORT_LIB_PATH must be unset (pkg-config only)"
    [[ -z "${LIBONNXRUNTIME_NO_PKG_CONFIG:-}" ]] \
        || fail "LIBONNXRUNTIME_NO_PKG_CONFIG must be unset (the system build needs pkg-config)"
    [[ -z "${PKG_CONFIG_PATH:-}" && -z "${PKG_CONFIG_LIBDIR:-}" && -z "${PKG_CONFIG_SYSROOT_DIR:-}" ]] \
        || fail "PKG_CONFIG_PATH/LIBDIR/SYSROOT_DIR must be unset (the image's .pc files only)"
    [[ ! -e "${cache}" ]] || fail "${cache} exists before the build"

    rustc --version | grep -q "^rustc ${msrv} " || fail "rustc is not ${msrv}: $(rustc --version)"
    ok "$(rustc --version) (declared MSRV)"
    info "System ONNX Runtime: $(pacman -Q onnxruntime-cpu), pkg-config $(pkg-config --modversion libonnxruntime)"

    # Resolved feature graph of the system build: pkg-config + api-17, no download code.
    local graph
    graph="$(cargo tree --locked --offline -e features -i ort-sys --workspace "${SYSTEM_FEATURES[@]}")"
    if grep -q 'download-binaries' <<<"${graph}"; then
        fail "the system build graph enables download-binaries"
    fi
    if grep -q 'api-18' <<<"${graph}"; then
        fail "the system build graph requires an API level above 17"
    fi
    grep -q 'ort-sys feature "pkg-config"' <<<"${graph}" || fail "pkg-config not enabled"
    ok "ort-sys resolves to pkg-config + api-17 without download-binaries"

    # MSRV coverage of every target and feature (no linking: deterministic offline).
    info "Type-check of every target and feature at the MSRV ${msrv}"
    cargo check --locked --offline --workspace --all-targets --all-features
    ok "Workspace type-checks at the MSRV with every target and feature"

    info "cargo build --locked --offline --release --workspace ${SYSTEM_FEATURES[*]}"
    cargo build --locked --offline --release --workspace --no-default-features --features system-onnxruntime
    ok "Release workspace built offline against the system ONNX Runtime"

    local daemon="${target}/release/soos-daemon"
    local pam_so="${target}/release/libpam_soos.so"
    readelf -d "${daemon}" | grep -q 'NEEDED.*\[libonnxruntime\.so\.1\]' \
        || fail "soos-daemon does not link libonnxruntime.so.1 dynamically"
    if readelf -d "${daemon}" | grep -qE 'RPATH|RUNPATH'; then
        fail "soos-daemon carries an RPATH/RUNPATH (the root daemon must use the system search path)"
    fi
    local resolved
    resolved="$(ldd "${daemon}" | awk '/libonnxruntime\.so/ {print $3}')"
    [[ "${resolved}" == "/usr/lib/libonnxruntime.so.1" ]] \
        || fail "libonnxruntime resolves to '${resolved}'"
    pacman -Qo "$(readlink -f "${resolved}")" | grep -q 'onnxruntime-cpu' \
        || fail "${resolved} is not owned by onnxruntime-cpu"
    ok "soos-daemon links ${resolved} (onnxruntime-cpu)"
    if readelf -d "${pam_so}" | grep -q 'libonnxruntime'; then
        fail "libpam_soos.so must never link ONNX Runtime"
    fi
    ok "libpam_soos.so does not link ONNX Runtime"

    # Real-model tests against the system library. Every suite of soos-inference-ort runs;
    # the shipped-model suites then run again with the models required (the evaluation
    # suite of the retired ArcFace model skips cleanly without it).
    export SOOS_MODELS_DIR=/models
    cargo test --locked --offline "${SYSTEM_FEATURES[@]}" -p soos-inference-ort --all-targets
    SOOS_REQUIRE_REAL_MODELS=1 cargo test --locked --offline "${SYSTEM_FEATURES[@]}" -p soos-inference-ort \
        --test system_onnxruntime_tests --test pad_real_model_tests --test embedding_real_model_tests \
        --test scrfd_real_model_tests --test scrfd_hot_path_real_model_tests \
        --test pad_output_contract_tests --test pad_input_range_tests
    SOOS_REQUIRE_REAL_MODELS=1 cargo test --locked --offline "${SYSTEM_FEATURES[@]}" -p soos-vision \
        --test pad_crop_real_model_tests
    SOOS_REQUIRE_REAL_MODELS=1 cargo test --locked --offline "${SYSTEM_FEATURES[@]}" -p soos-daemon \
        --test pad_nonface_pipeline_real_model_tests --test system_onnxruntime_startup_tests
    ok "Real-model tests pass against the system ONNX Runtime"

    [[ ! -e "${cache}" ]] || fail "${cache} was created: something downloaded ONNX Runtime"
    ok "No ort-sys download cache was created"

    # Fail closed: without the library the daemon cannot start (the loader refuses it
    # before main), so the socket never exists and pam_soos.so returns PAM_IGNORE.
    pacman -Rdd --noconfirm onnxruntime-cpu >/dev/null
    local out rc
    set +e
    out="$("${daemon}" --version 2>&1)"
    rc=$?
    set -e
    [[ "${rc}" -ne 0 ]] || fail "soos-daemon started without libonnxruntime (exit 0)"
    grep -q 'libonnxruntime' <<<"${out}" || fail "unexpected failure without the library: ${out}"
    ok "Without libonnxruntime the daemon refuses to start (exit ${rc})"
}

case "${1:-}" in
    --in-container) in_container ;;
    "") host ;;
    *) fail "usage: $0 [--in-container]" ;;
esac
