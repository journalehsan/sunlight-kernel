#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

out="$PWD/target/boot-resource-prototype"
mkdir -p "$out"
vfs_flags="-C link-arg=-T$PWD/services/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone"
RUSTFLAGS="$vfs_flags" cargo build -p sunlight-vfs-server --release --features boot-resources \
    --target-dir "$out/vfs"
cargo run -p sunlight-fs --example pack_boot_resources --features boot-resources \
    --release --target x86_64-unknown-linux-gnu -- "$out/resources.bin"
SUNLIGHT_BOOT_RESOURCE_VFS="$out/vfs/x86_64-unknown-none/release/sunlight-vfs-server" \
    cargo build -p sunlight-kernel --release --features boot_resources,native_document_viewer \
    --target-dir "$out/kernel"
SUNLIGHT_BOOT_RESOURCE_MODULE="$out/resources.bin" tools/make_hybrid_iso.sh \
    "$out/kernel/x86_64-unknown-none/release/sunlight-kernel" "$out/sunlightos.iso"
