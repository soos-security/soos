#!/usr/bin/env bash
# =============================================================================
# scripts/check_build_deps.sh — Build / Runtime Dependency Preflight for soos
# =============================================================================
# Single source of truth for the per-distribution package lists required to
# build soos from source (GitHub #165 / ONB-07), and a read-only preflight that
# verifies the build host before `cargo build --release --locked --workspace`.
#
# Why each build package is needed (derived from the locked dependency graph):
#   - C toolchain + libstdc++ : linking, ONNX Runtime (ort-sys) static library
#   - PAM headers             : pam-bindings (crates/pam)
#   - libclang + clang        : bindgen, build-dependency of v4l2-sys-mit
#   - OpenSSL headers         : openssl-sys <- native-tls <- ureq, used by the
#                               ort-sys build script to download ONNX Runtime
#   - pkg-config              : locates OpenSSL and PAM for the build scripts
# The ort-sys build script downloads ONNX Runtime binaries over the network
# during `cargo build`; for offline builds point ORT_LIB_LOCATION at a local
# ONNX Runtime build instead.
#
# GUI runtime libraries are loaded with dlopen() by soos-gui (winit/glutin), so
# package managers cannot detect them automatically.
#
# Usage:
#   check_build_deps.sh [--distro <id>]                    # preflight (default)
#   check_build_deps.sh [--distro <id>] --print-packages <build|gui|models>
#
# Exit codes: 0 all dependencies present / list printed, 1 dependency missing,
#             2 usage error or unsupported distribution.
# =============================================================================

set -euo pipefail

DEBIAN_BUILD=(build-essential pkg-config libpam0g-dev libclang-dev clang libssl-dev)
DEBIAN_GUI=(libxkbcommon0 libwayland-client0 libwayland-egl1 libegl1 libgl1 libx11-6 libxcursor1 libxi6 libxrandr2)
DEBIAN_MODELS=(curl ca-certificates coreutils)

FEDORA_BUILD=(gcc gcc-c++ make pkgconf-pkg-config pam-devel clang-devel openssl-devel)
FEDORA_GUI=(libxkbcommon libwayland-client libwayland-egl mesa-libEGL mesa-libGL libX11 libXcursor libXi libXrandr)
FEDORA_MODELS=(curl ca-certificates coreutils)

ARCH_BUILD=(base-devel clang openssl pkgconf pam)
ARCH_GUI=(libxkbcommon wayland libglvnd libx11 libxcursor libxi libxrandr)
ARCH_MODELS=(curl ca-certificates coreutils)

DISTRO=""
PRINT_GROUP=""

usage() {
    cat <<EOF
Usage: $(basename "$0") [--distro <debian|ubuntu|fedora|rhel|arch>] [--print-packages <build|gui|models>]

Without --print-packages, checks that this host can build soos from source and
prints the package manager command for anything missing.

Options:
  --distro <id>              Override distribution detection (/etc/os-release)
  --print-packages <group>   Print the package list for a group and exit:
                               build  - build-time packages (Rust toolchain via rustup)
                               gui    - soos-gui runtime libraries (dlopen'ed)
                               models - tools used by scripts/download_models.sh
  -h, --help                 Display this help message
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --distro)
            [[ $# -ge 2 ]] || { usage >&2; exit 2; }
            DISTRO="$2"
            shift 2
            ;;
        --print-packages)
            [[ $# -ge 2 ]] || { usage >&2; exit 2; }
            PRINT_GROUP="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "check_build_deps: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

# Maps an os-release ID / ID_LIKE list to a package family.
family_of() {
    local candidate
    for candidate in "$@"; do
        case "${candidate}" in
            debian|ubuntu|linuxmint|pop) echo "debian"; return 0 ;;
            fedora|rhel|centos|rocky|almalinux) echo "fedora"; return 0 ;;
            arch|archlinux|manjaro|endeavouros) echo "arch"; return 0 ;;
        esac
    done
    return 1
}

FAMILY=""
if [[ -n "${DISTRO}" ]]; then
    if ! FAMILY="$(family_of "${DISTRO}")"; then
        echo "check_build_deps: unsupported distribution '${DISTRO}' (supported: debian, ubuntu, fedora, rhel, arch)" >&2
        exit 2
    fi
elif [[ -r /etc/os-release ]]; then
    os_id="$(sed -n 's/^ID=//p' /etc/os-release | tr -d '"')"
    os_like="$(sed -n 's/^ID_LIKE=//p' /etc/os-release | tr -d '"')"
    # shellcheck disable=SC2086 # ID_LIKE is a space-separated list.
    FAMILY="$(family_of "${os_id}" ${os_like})" || FAMILY=""
fi

packages_for() {
    local family="$1" group="$2"
    case "${family}:${group}" in
        debian:build)  echo "${DEBIAN_BUILD[*]}" ;;
        debian:gui)    echo "${DEBIAN_GUI[*]}" ;;
        debian:models) echo "${DEBIAN_MODELS[*]}" ;;
        fedora:build)  echo "${FEDORA_BUILD[*]}" ;;
        fedora:gui)    echo "${FEDORA_GUI[*]}" ;;
        fedora:models) echo "${FEDORA_MODELS[*]}" ;;
        arch:build)    echo "${ARCH_BUILD[*]}" ;;
        arch:gui)      echo "${ARCH_GUI[*]}" ;;
        arch:models)   echo "${ARCH_MODELS[*]}" ;;
        *) return 1 ;;
    esac
}

install_command_for() {
    local family="$1"
    shift
    case "${family}" in
        debian) echo "sudo apt-get install -y --no-install-recommends $*" ;;
        fedora) echo "sudo dnf install -y $*" ;;
        arch)   echo "sudo pacman -S --needed $*" ;;
    esac
}

if [[ -n "${PRINT_GROUP}" ]]; then
    if [[ -z "${FAMILY}" ]]; then
        echo "check_build_deps: cannot detect the distribution; pass --distro <id>" >&2
        exit 2
    fi
    if ! packages_for "${FAMILY}" "${PRINT_GROUP}"; then
        echo "check_build_deps: unknown package group '${PRINT_GROUP}' (build, gui, models)" >&2
        exit 2
    fi
    exit 0
fi

# -----------------------------------------------------------------------------
# Read-only build host preflight
# -----------------------------------------------------------------------------
MISSING=()

for tool in cargo rustc cc pkg-config; do
    if ! command -v "${tool}" >/dev/null 2>&1; then
        MISSING+=("${tool}")
    fi
done

if command -v pkg-config >/dev/null 2>&1 && ! pkg-config --exists openssl 2>/dev/null; then
    MISSING+=("OpenSSL development headers (pkg-config openssl)")
fi

if [[ ! -f /usr/include/security/pam_appl.h ]]; then
    MISSING+=("PAM development headers (security/pam_appl.h)")
fi

libclang_found=false
libclang_globs=(
    "${LIBCLANG_PATH:-/nonexistent}/libclang*.so*"
    "/usr/lib/libclang*.so*"
    "/usr/lib64/libclang*.so*"
    "/usr/lib/*-linux-gnu/libclang*.so*"
    "/usr/lib/llvm-*/lib/libclang*.so*"
    "/usr/lib64/llvm*/lib/libclang*.so*"
)
for pattern in "${libclang_globs[@]}"; do
    if compgen -G "${pattern}" >/dev/null 2>&1; then
        libclang_found=true
        break
    fi
done
if [[ "${libclang_found}" = false ]]; then
    MISSING+=("libclang shared library (bindgen; set LIBCLANG_PATH if installed elsewhere)")
fi

if [[ ${#MISSING[@]} -eq 0 ]]; then
    echo "check_build_deps: all build dependencies are present."
    if [[ -z "${ORT_LIB_LOCATION:-}" ]]; then
        echo "check_build_deps: note: ort-sys downloads ONNX Runtime during the build (network required; set ORT_LIB_LOCATION for offline builds)."
    fi
    exit 0
fi

echo "check_build_deps: missing build dependencies:" >&2
for item in "${MISSING[@]}"; do
    echo "  - ${item}" >&2
done
if [[ -n "${FAMILY}" ]]; then
    # shellcheck disable=SC2046 # Intentional word splitting of the package list.
    echo "Install them with: $(install_command_for "${FAMILY}" $(packages_for "${FAMILY}" build))" >&2
else
    echo "Unknown distribution: see Docs/PACKAGING_AND_PROVISIONING.md for the package lists." >&2
fi
case " ${MISSING[*]} " in
    *" cargo "*|*" rustc "*)
        echo "Rust toolchain: install rustup from https://rustup.rs (rust-toolchain.toml selects the channel)." >&2
        ;;
esac
exit 1
