#!/usr/bin/env bash
# Package the existing native runtime; never copy host libc into the sysroot.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
bash tools/build_native_std.sh build --manifest-path tools/native-c/Cargo.toml \
    --target-dir "$root/target/native-c" --release --locked
mkdir -p target/native-c/sysroot/lib
cp target/native-c/x86_64-sunlightos/release/libsunlight_native_c.a \
    target/native-c/sysroot/lib/libsunlight_native_c.a
# Keep one authoritative header tree, including when new headers are added.
ln -sfn "$root/sunlight-libc/include" target/native-c/sysroot/include
