#!/usr/bin/env bash
set -euo pipefail

# Build only the existing UI/client. Rendering engines are not linked here.
# Rust's bundled musl CRT/linker is sufficient; no MuPDF C toolchain is needed.
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "$script_dir/.." && pwd)"
cd "$project_root"
docview_target_dir=${CARGO_TARGET_DIR:-target}
features=()
if [[ -n "${DOCVIEW_FEATURES:-}" ]]; then features=(--features "$DOCVIEW_FEATURES"); fi
env -u CARGO_ENCODED_RUSTFLAGS RUSTFLAGS='-C relocation-model=static -C target-feature=+crt-static -C link-arg=-no-pie' \
    cargo build -p sunlight-docview --no-default-features --features std \
    --release --target x86_64-unknown-linux-musl "${features[@]}"
bash "$script_dir/stamp_helios_elf.sh" "$docview_target_dir/x86_64-unknown-linux-musl/release/sunlight-docview"
