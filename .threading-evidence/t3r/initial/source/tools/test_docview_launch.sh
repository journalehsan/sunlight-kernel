#!/usr/bin/env bash
set -euo pipefail
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir/.."
# Default image: Helios viewer present, document renderer disabled. The native
# variant uses the artifacts prepared by tools/build_docserv.sh.
features=key_inject
if [[ "${DOCVIEW_BACKEND:-helios}" == native ]]; then
    features+=,native_document_viewer
fi
env SUNLIGHT_INJECT_PHASE=desktop_login \
    cargo build -p sunlight-kernel --no-default-features --features "$features"
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel \
    target/docview-launch-test.iso target/limine "$PWD"
if [[ "${DOCVIEW_LAUNCH_SKIP_BOOT:-0}" != 1 ]]; then
    python3 tools/docview_launch_gate.py
fi
