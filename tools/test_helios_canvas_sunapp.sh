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
case "$accel" in tcg|kvm) ;; *) exit 2 ;; esac
out="$PWD/target/helios-sunapp-sunlight-ui"
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
elf="$PWD/target/helios-wayland-lib/canvas/client.elf"
digest=577ac203166af6171f5ae2789a4a9da9d39ae40dc6d801188288703a8995ba9e
if [[ $mode == canvas-sunapp-input ]]; then
    elf="$PWD/target/helios-wayland-lib/canvas-input/client.elf"
    bash -x tools/test_helios_libwayland.sh --canvas-input-elf >"$out/client-build.log" 2>&1
else
    printf '%s  %s\n' "$digest" "$elf" | sha256sum -c
fi
(
    set -x
    bundle="$out/SunlightCanvas.sunapp"
    mkdir -p "$bundle/Program"
    install -m644 SunlightCanvas.sunapp/Manifest.toml "$bundle/Manifest.toml"
    install -m755 "$elf" "$bundle/Program/sunlight-ui-wayland"
    cmp "$elf" "$bundle/Program/sunlight-ui-wayland"
    sha256sum "$bundle/Manifest.toml" "$bundle/Program/sunlight-ui-wayland"
    readelf -h -l -d "$elf"
    service_flags='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone'
    RUSTFLAGS="$service_flags" cargo build -p helios-wayland-fixture -p sunlight-display \
        --release --features "$fixture_features"
    services="$PWD/target/x86_64-unknown-none/release"
    SUNLIGHT_WAYLAND_SERVER="$services/helios-wayland-fixture" \
    SUNLIGHT_CANVAS_LAUNCHER="$services/canvas-sunapp-launcher" \
    SUNLIGHT_CANVAS_BUNDLE_ELF="$bundle/Program/sunlight-ui-wayland" \
        cargo build -p sunlight-kernel --features helios_wayland_sunapp_test
    bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel "$out/guest.iso"
) >"$out/build-install.log" 2>&1
python3 tools/helios_wayland_gate.py "$out/guest.iso" "$out/$run" "$accel" "$mode"
sha256sum "$elf" "$out/guest.iso" "$out/$run/"*.png >"$out/$run/SHA256SUMS"
if [[ $mode == canvas-sunapp-input ]]; then
    sha256sum "$out/SunlightCanvas.sunapp/Manifest.toml" \
        "$out/SunlightCanvas.sunapp/Program/sunlight-ui-wayland" >>"$out/$run/SHA256SUMS"
fi
