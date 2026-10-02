#!/usr/bin/env bash
# =============================================================================
# scripts/prefetch_onnxruntime.sh — Bounded retry of the ONNX Runtime download
# =============================================================================
# The `ort-sys` build script downloads the prebuilt ONNX Runtime archive the
# first time it runs, verifies its SHA-256 and extracts it into its cache
# directory (ORT_CACHE_DIR, default ~/.cache/ort.pyke.io). A transient CDN or
# TLS error used to fail a whole CI job (GitHub #318). This script runs only
# that build script, `cargo check --locked -p ort` (same default features as
# every workspace crate, hence the same archive), and retries it a bounded
# number of times; every later cargo command then finds the extracted archive
# in the cache and downloads nothing.
#
# Environment:
#   SOOS_ORT_FETCH_ATTEMPTS  Attempts, 1..5 (default: 3)
#   SOOS_ORT_FETCH_DELAY_S   Base delay between attempts in seconds, 0..60
#                            (default: 15; attempt N waits N * delay)
#
# Exit codes: 0 fetched (or already cached), 1 every attempt failed,
#             2 invalid environment.
# =============================================================================

set -euo pipefail

readonly MAX_ATTEMPTS=5
readonly MAX_DELAY_S=60

ATTEMPTS="${SOOS_ORT_FETCH_ATTEMPTS:-3}"
DELAY_S="${SOOS_ORT_FETCH_DELAY_S:-15}"

if [[ ! "${ATTEMPTS}" =~ ^[0-9]+$ ]] || (( ATTEMPTS < 1 || ATTEMPTS > MAX_ATTEMPTS )); then
    echo "[ERROR] SOOS_ORT_FETCH_ATTEMPTS must be an integer in 1..${MAX_ATTEMPTS}: '${ATTEMPTS}'" >&2
    exit 2
fi
if [[ ! "${DELAY_S}" =~ ^[0-9]+$ ]] || (( DELAY_S > MAX_DELAY_S )); then
    echo "[ERROR] SOOS_ORT_FETCH_DELAY_S must be an integer in 0..${MAX_DELAY_S}: '${DELAY_S}'" >&2
    exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}/.."

for (( attempt = 1; attempt <= ATTEMPTS; attempt++ )); do
    echo "[INFO]  ONNX Runtime prefetch, attempt ${attempt}/${ATTEMPTS}: cargo check --locked -p ort"
    if cargo check --locked -p ort; then
        echo "[OK]    ONNX Runtime is available in the ort-sys cache."
        exit 0
    fi
    if (( attempt < ATTEMPTS )); then
        wait_s=$(( attempt * DELAY_S ))
        echo "[WARN]  ONNX Runtime prefetch failed; retrying in ${wait_s} s." >&2
        sleep "${wait_s}"
    fi
done

echo "[ERROR] ONNX Runtime prefetch failed after ${ATTEMPTS} attempt(s)." >&2
exit 1
