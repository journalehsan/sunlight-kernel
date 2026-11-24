#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
out="$PWD/target/helios-wayland-lib"
build="$out/build"
mkdir -p "$build"

(
    cd "$build"
    if [[ ! -d libffi-3.4.8 ]]; then
        curl -fsSL https://github.com/libffi/libffi/releases/download/v3.4.8/libffi-3.4.8.tar.gz | tar xz
    fi
    if [[ ! -d wayland-1.26.0 ]]; then
        curl -fsSL https://codeload.github.com/wayland-mirror/wayland/tar.gz/refs/tags/1.26.0 | tar xz
    fi
    if [[ ! -f prefix/lib/libffi.a ]]; then
        (
            cd libffi-3.4.8
            CC=musl-gcc ./configure --prefix="$build/prefix" --libdir="$build/prefix/lib" \
                --disable-shared --disable-exec-static-tramp
            make -j"$(nproc)"
            make install
        ) >ffi.log 2>&1
    fi
    cat >musl.ini <<EOF
[binaries]
c = 'musl-gcc'
pkg-config = 'pkg-config'
[host_machine]
system = 'linux'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'
[properties]
pkg_config_libdir = '$build/prefix/lib/pkgconfig'
EOF
    printf "[built-in options]\npkg_config_path = ['%s']\n" \
        "$(pkg-config --variable=pcfiledir wayland-scanner)" >native.ini
    meson setup wl wayland-1.26.0 --native-file native.ini --cross-file musl.ini \
        -Ddefault_library=static -Dscanner=false -Ddocumentation=false -Dtests=false \
        -Ddtd_validation=false >wl.log 2>&1
    ninja -C wl src/libwayland-client.a >>wl.log 2>&1
)
xml="$(pkg-config --variable=pkgdatadir wayland-protocols)/stable/xdg-shell/xdg-shell.xml"
wayland-scanner client-header "$xml" "$build/xdg-shell-client.h"
wayland-scanner private-code "$xml" "$build/xdg-shell.c"
musl-gcc -static -no-pie -O2 -Wall -Wextra -Werror \
    -I"$build" -I"$build/wl/src" -I"$build/wayland-1.26.0/src" \
    tools/tests/helios-libwayland-client.c "$build/xdg-shell.c" \
    "$build/wl/src/libwayland-client.a" "$build/prefix/lib/libffi.a" -pthread -o "$out/client.elf"
bash tools/stamp_helios_elf.sh "$out/client.elf"

service_flags='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone'
RUSTFLAGS="$service_flags" cargo build -p helios-wayland-fixture -p sunlight-display \
    --release --features sunlight-display/helios_wayland_test >"$out/service-build.log" 2>&1
SUNLIGHT_WAYLAND_SERVER="$PWD/target/x86_64-unknown-none/release/helios-wayland-fixture" \
SUNLIGHT_WAYLAND_CLIENT="$out/client.elf" \
    cargo build -p sunlight-kernel --features helios_wayland_test >"$out/kernel-build.log" 2>&1
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel \
    "$out/guest.iso" >"$out/pack.log" 2>&1
status=0
for accel in "${@:-kvm}"; do
    python3 tools/helios_wayland_gate.py "$out/guest.iso" "$out/$accel" "$accel" libwayland || status=1
done
exit "$status"
