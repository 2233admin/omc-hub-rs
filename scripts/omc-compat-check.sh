#!/usr/bin/env bash
# omc-compat-check.sh — Detect OMC plugin changes that might break omc-hub-rs
# Run after `omc update` to check for breaking changes.
#
# Usage:
#   ./scripts/omc-compat-check.sh              # check + report
#   ./scripts/omc-compat-check.sh --snapshot   # save current state as baseline
#   ./scripts/omc-compat-check.sh --auto       # snapshot if no baseline, else check

set -euo pipefail

# Use Python for all extraction — avoids bash/Windows path hell
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "${SCRIPT_DIR}/omc-compat-check.py" "$@"
