#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# shellcheck source=../vm_display_policy.sh
source "$PROJECT_ROOT/tools/vm_display_policy.sh"

assert_eq() {
    local expected="$1"
    local actual="$2"
    local label="$3"
    if [[ "$expected" != "$actual" ]]; then
        echo "[FAIL] $label"
        echo "  expected: $expected"
        echo "  actual:   $actual"
        exit 1
    fi
}

assert_true() {
    local label="$1"
    if ! "${@:2}" >/dev/null; then
        echo "[FAIL] $label"
        exit 1
    fi
}

assert_false() {
    local label="$1"
    if "${@:2}" >/dev/null; then
        echo "[FAIL] $label"
        exit 1
    fi
}

assert_eq "1366x768" \
    "$(sunlight_choose_preferred_mode "current" "1280x800" "1366x768" "1024x768")" \
    "choose 1366x768 when available"

assert_eq "1360x768" \
    "$(sunlight_choose_preferred_mode "current" "1360x768" "1280x800")" \
    "fallback to 1360x768 when 1366x768 is absent"

assert_eq "1280x800" \
    "$(sunlight_choose_preferred_mode "current" "1280x800" "1024x768")" \
    "fallback to 1280x800"

assert_eq "1280x720" \
    "$(sunlight_choose_preferred_mode "current" "1280x720" "1024x768")" \
    "fallback to 1280x720 when larger preferred modes are absent"

assert_eq "1024x768" \
    "$(sunlight_choose_preferred_mode "current" "1024x768")" \
    "fallback to 1024x768"

assert_eq "1600x900" \
    "$(sunlight_choose_preferred_mode "current" "1600x900" "800x600")" \
    "ranked fallback chooses practical widescreen mode"

assert_eq "current" \
    "$(sunlight_choose_preferred_mode "current" "2560x1440" "800x600")" \
    "fallback to current mode when candidates are suspicious"

assert_eq "current" \
    "$(sunlight_resolve_vm_display_mode "current" "hardware" "" "1366x768" "1280x800")" \
    "physical hardware keeps current mode"

assert_eq "current" \
    "$(sunlight_resolve_vm_display_mode "current" "qemu" "1600x900" "1366x768" "1280x800")" \
    "unsupported override falls back safely"

assert_eq "current" \
    "$(sunlight_resolve_vm_display_mode "current" "qemu" "bad-input" "1366x768" "1280x800")" \
    "invalid override falls back safely"

assert_eq "1280x720" \
    "$(sunlight_resolve_vm_display_mode "current" "qemu" "1280x720" "1366x768" "1280x720")" \
    "valid override wins when supported"

assert_true "qemu is treated as vm" sunlight_is_vm_environment "qemu"
assert_true "vmware is treated as vm" sunlight_is_vm_environment "vmware"
assert_false "hardware is not treated as vm" sunlight_is_vm_environment "hardware"

assert_true "valid resolution parses" sunlight_parse_resolution "1366x768"
assert_false "invalid resolution rejected" sunlight_parse_resolution "1366-768"

assert_eq "kvm" "$(sunlight_select_qemu_accel auto yes)" \
    "auto accelerator prefers usable KVM"
assert_eq "tcg" "$(sunlight_select_qemu_accel auto no)" \
    "auto accelerator falls back to TCG"
assert_eq "tcg" "$(sunlight_select_qemu_accel tcg yes)" \
    "explicit TCG stays TCG"
assert_false "explicit unavailable KVM fails" sunlight_select_qemu_accel kvm no

assert_eq "virtio-vga,edid=off,xres=1280,yres=720" \
    "$(sunlight_qemu_video_device_spec virtio-vga 1280 720 yes)" \
    "explicit QEMU mode disables EDID"
assert_eq "virtio-vga,xres=1280,yres=800" \
    "$(sunlight_qemu_video_device_spec virtio-vga 1280 800 no)" \
    "automatic QEMU mode retains EDID"
assert_eq "virtio-vga,ioeventfd=on,edid=off,xres=1280,yres=720" \
    "$(sunlight_qemu_video_device_spec virtio-vga 1280 720 yes yes)" \
    "KVM VirtIO mode enables ioeventfd"

echo "[PASS] vm display policy"
