#!/usr/bin/env bash
# Canonical entry point for build/run (see AGENTS.md / dual-firmware boot).
# Delegates to tools/runs.sh so BIOS and UEFI share one implementation.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$SCRIPT_DIR/tools/runs.sh" "$@"
