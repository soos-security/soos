#!/usr/bin/env bash
# =============================================================================
# scripts/release_dry_run.sh — reproducible source and vendor tarballs, built
# locally and never published (GitHub #444, part of #361; ADR 2026-10-18
# "Release Dry Run Publishes Nothing"; matrix RDR1-RDR6, RDR-S1-RDR-S8)
# =============================================================================
# Builds, from the committed HEAD only:
#
#   soos-<version>.tar.gz          git archive of HEAD (prefix soos-<version>/),
#                                  every mtime = commit time, gzip -n
#   soos-<version>-vendor.tar.zst  cargo vendor --locked of that tree plus the
#                                  vendored-sources .cargo/config.toml, as a
#                                  deterministic tar (sorted names, owner 0/0,
#                                  mtime = commit time, fixed zstd level)
#   SHA256SUMS                     sha256sum of both files
#
# into target/release-dry-run/ (or --out-dir). <version> is the `version` of
# [workspace.package] in HEAD:Cargo.toml; SOURCE_DATE_EPOCH is the commit time
# (git log -1 --format=%ct HEAD). Two runs on the same commit give the same
# bytes. Unpacked into one directory, the two tarballs build offline:
#
#   ORT_SKIP_DOWNLOAD=1 cargo build --locked --offline --release --workspace \
#       --no-default-features --features system-onnxruntime
#
# The only network access is `cargo fetch --locked` (skipped with --offline).
# This script never tags, signs, pushes, uploads or publishes anything: those
# steps are owner-only (Docs/RELEASING.md).
#
# Usage:
#   scripts/release_dry_run.sh [--dry-run] [--offline] [--out-dir DIR]
#   scripts/release_dry_run.sh --check-vendor VENDOR_DIR CARGO_LOCK
#   scripts/release_dry_run.sh --help
#
# Exit codes: 0 success, 1 failure, 2 usage error.
# =============================================================================

set -euo pipefail
umask 022
export LC_ALL=C
export TZ=UTC
# Each of these changes the produced bytes; CDPATH would make `cd` print.
unset TAR_OPTIONS GZIP ZSTD_CLEVEL ZSTD_NBTHREADS CDPATH

readonly ZSTD_LEVEL=12
readonly GZIP_LEVEL=9
# The only online step (skipped with --offline).
readonly FETCH_COMMAND=(cargo fetch --locked)
readonly CRATES_IO_SOURCE="registry+https://github.com/rust-lang/crates.io-index"
readonly EXIT_FAILURE=1
readonly EXIT_USAGE=2

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT

info() { echo "[INFO]  $*"; }
ok()   { echo "[OK]    $*"; }
plan() { echo "[PLAN]  $*"; }
warn() { echo "[WARN]  $*" >&2; }
fail() { echo "[FAIL]  $*" >&2; exit "${EXIT_FAILURE}"; }
usage_error() { echo "[FAIL]  $*" >&2; echo "Run with --help for usage." >&2; exit "${EXIT_USAGE}"; }

usage() {
    cat <<'EOF'
Usage:
  scripts/release_dry_run.sh [--dry-run] [--offline] [--out-dir DIR]
  scripts/release_dry_run.sh --check-vendor VENDOR_DIR CARGO_LOCK
  scripts/release_dry_run.sh --help

Builds soos-<version>.tar.gz (git archive of HEAD), soos-<version>-vendor.tar.zst
(cargo vendor --locked plus .cargo/config.toml) and SHA256SUMS, reproducibly,
into target/release-dry-run/. Nothing is tagged, signed, uploaded or published.

Options:
  --dry-run       Print the plan; write, fetch and build nothing.
  --offline       Skip the fetch step (every locked crate must already be in
                  the local cargo cache).
  --out-dir DIR   Output directory (default: target/release-dry-run of the
                  repository). Only the three output files are replaced in it.
  --check-vendor VENDOR_DIR CARGO_LOCK
                  Only compare the .cargo-checksum.json package checksum of every
                  vendored crate with the Cargo.lock checksums, then exit.
  --help          Show this help.

Exit codes: 0 success, 1 failure, 2 usage error.
EOF
}

# The vendored-sources configuration printed by `cargo vendor` for a crates.io-only lock file.
vendor_config() {
    printf '%s\n' \
        '[source.crates-io]' \
        'replace-with = "vendored-sources"' \
        '' \
        '[source.vendored-sources]' \
        'directory = "vendor"'
}

# -----------------------------------------------------------------------------
# Vendor check: the vendored crate set equals Cargo.lock, compared by checksum.
# -----------------------------------------------------------------------------
check_vendor() {
    local vendor="$1" lock="$2"
    [[ -d "${vendor}" && ! -L "${vendor}" ]] || fail "vendor directory ${vendor} is missing"
    [[ -f "${lock}" ]] || fail "lock file ${lock} is missing"

    local sources bad_sources registry_count
    sources="$(sed -n 's/^source = "\(.*\)"$/\1/p' "${lock}")"
    bad_sources="$(grep -v -x -F "${CRATES_IO_SOURCE}" <<<"${sources}" | grep -v '^$' || true)"
    [[ -z "${bad_sources}" ]] \
        || fail "Cargo.lock has a source other than crates.io: $(head -n 1 <<<"${bad_sources}")"
    registry_count="$(grep -c -x -F "source = \"${CRATES_IO_SOURCE}\"" "${lock}" || true)"

    local expected expected_count malformed
    expected="$(sed -n 's/^checksum = "\(.*\)"$/\1/p' "${lock}" | sort)"
    malformed="$(grep -v -E '^[0-9a-f]{64}$' <<<"${expected}" | grep -v '^$' || true)"
    [[ -z "${malformed}" ]] || fail "Cargo.lock has a malformed checksum: $(head -n 1 <<<"${malformed}")"
    expected_count="$(grep -c -E '^[0-9a-f]{64}$' <<<"${expected}" || true)"
    [[ "${expected_count}" == "${registry_count}" ]] \
        || fail "Cargo.lock has ${registry_count} crates.io packages but ${expected_count} checksums"

    local entry name json packages actual=""
    local -i vendored=0
    while IFS= read -r -d '' entry; do
        name="${entry##*/}"
        [[ -d "${entry}" && ! -L "${entry}" ]] || fail "vendor entry ${name} is not a directory"
        json="${entry}/.cargo-checksum.json"
        [[ -f "${json}" && ! -L "${json}" ]] || fail "vendor entry ${name} has no .cargo-checksum.json"
        packages="$(grep -o -E '"package":"[0-9a-f]{64}"' "${json}" || true)"
        [[ -n "${packages}" && "$(wc -l <<<"${packages}")" -eq 1 ]] \
            || fail "vendor entry ${name} must hold exactly one package checksum"
        actual+="$(grep -o -E '[0-9a-f]{64}' <<<"${packages}")"$'\n'
        vendored+=1
    done < <(find "${vendor}" -mindepth 1 -maxdepth 1 -print0)
    actual="$(sort <<<"${actual%$'\n'}")"

    if [[ "${actual}" != "${expected}" ]]; then
        local first
        first="$(diff <(echo "${expected}") <(echo "${actual}") | grep -E '^[<>] ' | head -n 1 || true)"
        fail "vendored crates differ from Cargo.lock (${vendored} vendored, ${expected_count} locked; first difference: ${first:-count})"
    fi
    ok "${vendored} vendored crates match Cargo.lock by package checksum"
}

# -----------------------------------------------------------------------------
# Inputs from HEAD
# -----------------------------------------------------------------------------
workspace_version() {
    local manifest version
    manifest="$(git -C "${ROOT}" show HEAD:Cargo.toml)" || fail "cannot read HEAD:Cargo.toml"
    version="$(awk '
        /^[[:space:]]*\[/ { inside = ($0 ~ /^[[:space:]]*\[workspace\.package\][[:space:]]*$/); next }
        inside && /^[[:space:]]*version[[:space:]]*=/ {
            sub(/^[^=]*=[[:space:]]*"/, ""); sub(/".*$/, ""); print; exit
        }' <<<"${manifest}")"
    [[ "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] \
        || fail "HEAD:Cargo.toml must declare a [workspace.package] version (got '${version}')"
    echo "${version}"
}

preflight() {
    local tool
    for tool in cargo tar gzip zstd sha256sum find sort; do
        command -v "${tool}" >/dev/null 2>&1 || fail "${tool} is required"
    done
    tar --version 2>/dev/null | head -n 1 | grep -q 'GNU tar' || fail "GNU tar is required (--sort=name)"
}

# -----------------------------------------------------------------------------
# Steps
# -----------------------------------------------------------------------------
build_source_tarball() {
    local listing
    git -C "${ROOT}" -c tar.umask=0022 archive --format=tar --prefix="${PREFIX}/" HEAD \
        | gzip -n -9 >"${STAGE}/${SOURCE_TARBALL}"
    listing="$(tar -tzf "${STAGE}/${SOURCE_TARBALL}")"
    if grep -q -F -e "${PREFIX}/.cargo/" -e "${PREFIX}/vendor/" <<<"${listing}"; then
        fail "the source tree must not carry .cargo/ or vendor/"
    fi
    ok "${SOURCE_TARBALL} (git archive of ${COMMIT}, gzip -${GZIP_LEVEL} -n)"
}

unpack_source_tree() {
    mkdir "${STAGE}/tree"
    tar -xzf "${STAGE}/${SOURCE_TARBALL}" -C "${STAGE}/tree"
    TREE="${STAGE}/tree/${PREFIX}"
    [[ -f "${TREE}/Cargo.lock" ]] || fail "the archived tree has no Cargo.lock"
}

fetch_crates() {
    if [[ "${OFFLINE}" == "1" ]]; then
        info "Fetch skipped (--offline): the local cargo cache must hold every locked crate"
        return
    fi
    info "Fetch (network): ${FETCH_COMMAND[*]}"
    (cd "${TREE}" && "${FETCH_COMMAND[@]}")
}

vendor_crates() {
    local snippet="${STAGE}/vendor-config.toml"
    info "Vendor (offline): cargo vendor --locked --offline"
    (cd "${TREE}" && CARGO_NET_OFFLINE=true cargo vendor --locked --offline vendor >"${snippet}")
    if [[ ! -s "${snippet}" ]]; then
        # cargo prints nothing and creates no directory when nothing needs vendoring.
        grep -q '^source = ' "${TREE}/Cargo.lock" && fail "cargo vendor printed no configuration"
        mkdir -p "${TREE}/vendor"
    elif [[ "$(cat "${snippet}")" != "$(vendor_config)" ]]; then
        fail "unexpected cargo vendor configuration: $(tr '\n' ' ' <"${snippet}")"
    fi
    mkdir "${TREE}/.cargo"
    vendor_config >"${TREE}/.cargo/config.toml"
}

build_vendor_tarball() {
    tar -C "${STAGE}/tree" \
        --sort=name \
        --format=gnu \
        --owner=0 --group=0 --numeric-owner \
        --mtime="@${SOURCE_DATE_EPOCH}" \
        --mode=u+rwX,go+rX,go-w \
        --hard-dereference \
        -cf - "${PREFIX}/.cargo/config.toml" "${PREFIX}/vendor" \
        | zstd "-${ZSTD_LEVEL}" -T1 -q -c >"${STAGE}/${VENDOR_TARBALL}"
    ok "${VENDOR_TARBALL} (sorted, owner 0/0, mtime @${SOURCE_DATE_EPOCH}, zstd -${ZSTD_LEVEL})"
}

write_checksums() {
    (cd "${STAGE}" && sha256sum "${SOURCE_TARBALL}" "${VENDOR_TARBALL}" >SHA256SUMS)
}

move_outputs() {
    local name
    for name in "${SOURCE_TARBALL}" "${VENDOR_TARBALL}" SHA256SUMS; do
        mv -f "${STAGE}/${name}" "${OUT_DIR}/${name}"
    done
}

print_plan() {
    plan "commit: ${COMMIT}"
    plan "version: ${VERSION} ([workspace.package] of HEAD:Cargo.toml)"
    plan "SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH} (commit time)"
    plan "output: ${OUT_DIR}"
    plan "file: ${SOURCE_TARBALL}"
    plan "file: ${VENDOR_TARBALL}"
    plan "file: SHA256SUMS"
    plan "step 1: git archive --format=tar --prefix=${PREFIX}/ HEAD | gzip -n -${GZIP_LEVEL}"
    if [[ "${OFFLINE}" == "1" ]]; then
        plan "step 2: ${FETCH_COMMAND[*]} (skipped: --offline)"
    else
        plan "step 2: ${FETCH_COMMAND[*]} (the only network access)"
    fi
    plan "step 3: cargo vendor --locked --offline vendor, plus .cargo/config.toml"
    plan "step 4: --check-vendor (every .cargo-checksum.json package checksum against Cargo.lock)"
    plan "step 5: tar --sort=name --owner=0 --group=0 --numeric-owner --mtime=@${SOURCE_DATE_EPOCH} | zstd -${ZSTD_LEVEL}"
    plan "step 6: sha256sum > SHA256SUMS"
    plan "nothing is tagged, signed, uploaded or published"
}

cleanup() {
    if [[ -n "${STAGE:-}" && -d "${STAGE}" ]]; then
        rm -rf -- "${STAGE}"
    fi
}

main() {
    command -v git >/dev/null 2>&1 || fail "git is required"
    COMMIT="$(git -C "${ROOT}" rev-parse --verify HEAD)" || fail "no HEAD commit"
    VERSION="$(workspace_version)"
    SOURCE_DATE_EPOCH="$(git -C "${ROOT}" log -1 --format=%ct HEAD)"
    [[ "${SOURCE_DATE_EPOCH}" =~ ^[0-9]+$ ]] || fail "invalid commit time '${SOURCE_DATE_EPOCH}'"
    export SOURCE_DATE_EPOCH
    PREFIX="soos-${VERSION}"
    SOURCE_TARBALL="${PREFIX}.tar.gz"
    VENDOR_TARBALL="${PREFIX}-vendor.tar.zst"
    OUT_DIR="${OUT_DIR_ARG:-${ROOT}/target/release-dry-run}"

    if [[ -n "$(git -C "${ROOT}" status --porcelain --untracked-files=normal)" ]]; then
        warn "the work tree has uncommitted changes: only the committed HEAD ${COMMIT} is packed"
    fi

    if [[ "${DRY_RUN}" == "1" ]]; then
        print_plan
        return
    fi

    preflight
    [[ ! -e "${OUT_DIR}" || -d "${OUT_DIR}" ]] || fail "${OUT_DIR} exists and is not a directory"
    mkdir -p -- "${OUT_DIR}"
    OUT_DIR="$(cd "${OUT_DIR}" && pwd)"
    trap cleanup EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    STAGE="$(mktemp -d "${OUT_DIR}/.staging.XXXXXX")"
    info "Release dry run of ${COMMIT} (version ${VERSION}, SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH})"

    build_source_tarball
    unpack_source_tree
    fetch_crates
    vendor_crates
    check_vendor "${TREE}/vendor" "${TREE}/Cargo.lock"
    build_vendor_tarball
    write_checksums
    move_outputs
    ok "Outputs in ${OUT_DIR} (nothing was tagged, signed, uploaded or published):"
    sed 's/^/        /' "${OUT_DIR}/SHA256SUMS"
}

# -----------------------------------------------------------------------------
# Arguments
# -----------------------------------------------------------------------------
DRY_RUN=0
OFFLINE=0
OUT_DIR_ARG=""
OUT_DIR_SET=0
CHECK_VENDOR=0
STAGE=""

if [[ "${1:-}" == "--check-vendor" ]]; then
    (( $# == 3 )) || usage_error "--check-vendor takes exactly VENDOR_DIR and CARGO_LOCK"
    [[ -d "$2" ]] || usage_error "--check-vendor: ${2} is not a directory"
    [[ -f "$3" ]] || usage_error "--check-vendor: ${3} is not a file"
    CHECK_VENDOR=1
fi

if [[ "${CHECK_VENDOR}" == "1" ]]; then
    check_vendor "$2" "$3"
    exit 0
fi

while (( $# > 0 )); do
    case "$1" in
        --help|-h)
            usage
            exit 0
            ;;
        --dry-run)
            [[ "${DRY_RUN}" == "0" ]] || usage_error "--dry-run given twice"
            DRY_RUN=1
            ;;
        --offline)
            [[ "${OFFLINE}" == "0" ]] || usage_error "--offline given twice"
            OFFLINE=1
            ;;
        --out-dir)
            [[ "${OUT_DIR_SET}" == "0" ]] || usage_error "--out-dir given twice"
            (( $# >= 2 )) || usage_error "--out-dir needs a directory"
            [[ -n "$2" && "$2" != -* ]] || usage_error "--out-dir needs a directory (got '${2}')"
            [[ "$2" != "/" ]] || usage_error "--out-dir must not be /"
            OUT_DIR_ARG="$2"
            OUT_DIR_SET=1
            shift
            ;;
        --check-vendor)
            usage_error "--check-vendor must be used alone"
            ;;
        *)
            usage_error "unknown argument '$1'"
            ;;
    esac
    shift
done

main
