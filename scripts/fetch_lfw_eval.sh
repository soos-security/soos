#!/usr/bin/env bash
# =============================================================================
# scripts/fetch_lfw_eval.sh — Bounded download of the public LFW benchmark
# =============================================================================
# Downloads the Labeled Faces in the Wild (LFW) original archive and the official
# 10-fold `pairs.txt` into an evaluation cache OUTSIDE the repository, verifies
# their SHA-256 digests, and extracts the archive. The data feeds the ignored
# real-face evaluation harness
# `crates/vision/tests/embedding_lfw_evaluation_tests.rs` (GitHub #278,
# walkthrough 160). Nothing downloaded here may ever be committed.
#
# Usage: scripts/fetch_lfw_eval.sh [CACHE_DIR]   (default: ~/.cache/soos-eval)
#
# Sources and digests are the ones pinned by scikit-learn
# (`sklearn/datasets/_lfw.py`, figshare mirror of vis-www.cs.umass.edu/lfw).
#
# Invariants:
#   - The cache directory must not be inside this git repository.
#   - curl is HTTPS-only, TLS >= 1.2, time- and size-bounded (--max-filesize).
#   - A file whose SHA-256 differs from the pinned digest is deleted and the
#     script fails; extraction only runs on a verified archive.
# =============================================================================

set -euo pipefail

readonly ARCHIVE_URL="https://ndownloader.figshare.com/files/5976018"
readonly ARCHIVE_SHA256="055f7d9c632d7370e6fb4afc7468d40f970c34a80d4c6f50ffec63f5a8d536c0"
readonly ARCHIVE_MAX_BYTES=268435456
readonly PAIRS_URL="https://ndownloader.figshare.com/files/5976006"
readonly PAIRS_SHA256="ea42330c62c92989f9d7c03237ed5d591365e89b3e649747777b70e692dc1592"
readonly PAIRS_MAX_BYTES=1048576

cache_dir="${1:-${HOME}/.cache/soos-eval}"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Resolve without creating anything, so a refused path is never created.
cache_dir="$(realpath --canonicalize-missing -- "${cache_dir}")"
repo_root="$(realpath -- "${repo_root}")"
case "${cache_dir}/" in
    "${repo_root}/"*)
        echo "[ERROR] the evaluation cache must live outside the repository: ${cache_dir}" >&2
        exit 1
        ;;
esac
mkdir -p "${cache_dir}"

fetch_verified() {
    local url="$1" digest="$2" max_bytes="$3" dest="$4"
    if [[ -f "${dest}" ]] && echo "${digest}  ${dest}" | sha256sum --check --status; then
        echo "[OK]    ${dest} already verified"
        return 0
    fi
    rm -f "${dest}"
    echo "[INFO]  downloading ${url}"
    curl --fail --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
        --max-time 1800 --max-filesize "${max_bytes}" --silent --show-error \
        --output "${dest}.part" "${url}"
    if ! echo "${digest}  ${dest}.part" | sha256sum --check --status; then
        rm -f "${dest}.part"
        echo "[ERROR] SHA-256 mismatch for ${url}" >&2
        return 1
    fi
    mv "${dest}.part" "${dest}"
    echo "[OK]    ${dest} verified"
}

fetch_verified "${PAIRS_URL}" "${PAIRS_SHA256}" "${PAIRS_MAX_BYTES}" "${cache_dir}/pairs.txt"
fetch_verified "${ARCHIVE_URL}" "${ARCHIVE_SHA256}" "${ARCHIVE_MAX_BYTES}" "${cache_dir}/lfw.tgz"

if [[ ! -d "${cache_dir}/lfw" ]]; then
    tar --extract --gzip --no-same-owner --no-same-permissions \
        --file "${cache_dir}/lfw.tgz" --directory "${cache_dir}"
fi
echo "[OK]    LFW ready: SOOS_EVAL_LFW_DIR=${cache_dir}/lfw SOOS_EVAL_LFW_PAIRS=${cache_dir}/pairs.txt"
