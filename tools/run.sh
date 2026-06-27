#!/usr/bin/env bash
# Compatibility wrapper. Prefer tools/runs.sh.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/runs.sh" "$@"
