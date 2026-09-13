#!/usr/bin/env bash
# =============================================================================
# scripts/sync_issue.sh — Dual-Sync Wrapper for Local BACKLOG.md & GitHub Issues
# =============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
python3 "${SCRIPT_DIR}/sync_issue.py" "$@"
