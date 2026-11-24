#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
mode=canvas-sunapp
fixture_features=helios-wayland-fixture/sunapp-launcher,sunlight-display/helios_wayland_test
if [[ ${1:-} == --input ]]; then
    shift
    mode=canvas-sunapp-input
    fixture_features+=,helios-wayland-fixture/pointer-fixture
fi
accel=${1:-tcg}
build_only=false
if [[ $accel == --build-only ]]; then
    build_only=true
    accel=tcg
fi
case "$accel" in tcg|kvm) ;; *) exit 2 ;; esac
out=${HELIOS_WAYLAND_OUT:-"$PWD/target/helios-sunapp-sunlight-ui"}
run="$accel"
if [[ $mode == canvas-sunapp-input ]]; then
    out+=-input
    run+=-replay
fi
mkdir -p "$out/$run"
if [[ $accel == kvm && ! -e /dev/kvm ]]; then
    printf '%s\n' 'KVM unavailable: /dev/kvm does not exist' | tee "$out/$run/unavailable.txt"
    exit 1
fi
variant=canvas
build_variant=--canvas-elf
if [[ $mode == canvas-sunapp-input ]]; then
    variant=canvas-input
    build_variant=--canvas-input-elf
fi
(
    set -x
    build_out=$(mktemp -d "$out/client.XXXXXX")
    HELIOS_WAYLAND_OUT="$build_out" bash -x tools/test_helios_libwayland.sh "$build_variant" >"$out/client-build.log" 2>&1
    artifact="$build_out/$variant"
    elf="$artifact/client.elf"
    read -r digest _ <"$artifact/client.sha256"
    git rev-parse HEAD
    rustc -vV
    musl-gcc -v
    ld --version
    sha256sum Cargo.lock tools/tests/helios-libwayland-client.c sunlight-ui-wayland/src/lib.rs
    printf 'selected=%s expected=%s\n' "$elf" "$digest"
    sha256sum "$elf"
    printf '%s  %s\n' "$digest" "$elf" | sha256sum -c
    bundle="$out/SunlightCanvas.sunapp"
    mkdir -p "$bundle/Program"
    install -m644 SunlightCanvas.sunapp/Manifest.toml "$bundle/Manifest.toml"
    install -m755 "$elf" "$bundle/Program/sunlight-ui-wayland"
    printf '%s  %s\n' "$digest" "$bundle/Program/sunlight-ui-wayland" | sha256sum -c
    sha256sum "$bundle/Manifest.toml" "$bundle/Program/sunlight-ui-wayland"
    readelf -h -l -d "$elf"
    service_flags='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone'
    SUNLIGHT_CANVAS_EXPECTED_SHA256="$digest" RUSTFLAGS="$service_flags" cargo build -p helios-wayland-fixture -p sunlight-display \
        --release --features "$fixture_features"
    services="$PWD/target/x86_64-unknown-none/release"
    SUNLIGHT_WAYLAND_SERVER="$services/helios-wayland-fixture" \
    SUNLIGHT_CANVAS_LAUNCHER="$services/canvas-sunapp-launcher" \
    SUNLIGHT_CANVAS_BUNDLE_ELF="$bundle/Program/sunlight-ui-wayland" \
        cargo build -p sunlight-kernel --features helios_wayland_sunapp_test
    bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel "$out/guest.iso"
    sha256sum "$elf" "$bundle/Manifest.toml" "$bundle/Program/sunlight-ui-wayland" \
        target/iso_root/boot/sunlight-kernel.elf "$out/guest.iso" >"$out/artifacts.sha256"
) >"$out/build-install.log" 2>&1
if "$build_only"; then exit 0; fi
python3 tools/helios_wayland_gate.py "$out/guest.iso" "$out/$run" "$accel" "$mode"
sha256sum "$out/guest.iso" "$out/$run/"*.png >"$out/$run/SHA256SUMS"
if [[ $mode == canvas-sunapp-input ]]; then
    sha256sum "$out/SunlightCanvas.sunapp/Manifest.toml" \
        "$out/SunlightCanvas.sunapp/Program/sunlight-ui-wayland" >>"$out/$run/SHA256SUMS"
fi
