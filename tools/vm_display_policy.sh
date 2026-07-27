#!/usr/bin/env bash

# VM display resolution policy shared by host-side launchers.
# This does not mode-switch physical hardware; it only selects a conservative
# preferred VM resolution when the hypervisor backend exposes an explicit
# width/height request interface.

readonly SUNLIGHT_VM_PREFERRED_MODES=(
    "1366x768"
    "1360x768"
    "1280x800"
    "1280x720"
    "1440x900"
    "1024x768"
)

readonly SUNLIGHT_VM_AUTO_MAX_W=1920
readonly SUNLIGHT_VM_AUTO_MAX_H=1080

sunlight_parse_resolution() {
    local spec="${1:-}"
    if [[ "$spec" =~ ^([0-9]{3,5})x([0-9]{3,5})$ ]]; then
        printf '%s %s\n' "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}"
        return 0
    fi
    return 1
}

sunlight_is_vm_environment() {
    case "${1:-}" in
        qemu|kvm|vmware|virtualbox|vbox)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

sunlight_resolution_in_list() {
    local wanted="${1:-}"
    shift || true

    local mode
    for mode in "$@"; do
        if [[ "$mode" == "$wanted" ]]; then
            return 0
        fi
    done
    return 1
}

sunlight_choose_preferred_mode() {
    local fallback="${1:-}"
    shift || true

    local preferred
    for preferred in "${SUNLIGHT_VM_PREFERRED_MODES[@]}"; do
        if sunlight_resolution_in_list "$preferred" "$@"; then
            printf '%s\n' "$preferred"
            return 0
        fi
    done

    sunlight_choose_ranked_mode "$fallback" "$@"
}

sunlight_abs_diff() {
    local a="$1"
    local b="$2"
    if (( a >= b )); then
        printf '%s\n' "$((a - b))"
    else
        printf '%s\n' "$((b - a))"
    fi
}

sunlight_mode_score() {
    local mode="${1:-}"
    local w h
    if ! read -r w h <<<"$(sunlight_parse_resolution "$mode")"; then
        printf '%s\n' "-999999"
        return 0
    fi

    local score=0
    if (( w > SUNLIGHT_VM_AUTO_MAX_W || h > SUNLIGHT_VM_AUTO_MAX_H )); then
        score=$((score - 2000))
    fi

    if (( w >= 1280 )); then
        score=$((score + 600))
    elif (( w >= 1024 )); then
        score=$((score + 250))
    else
        score=$((score - 400))
    fi

    if (( h >= 720 )); then
        score=$((score + 600))
    else
        score=$((score - 400))
    fi

    if (( w < 1024 || h < 768 )); then
        score=$((score - 250))
    fi

    local area=$((w * h))
    if (( area > 1500000 )); then
        area=1500000
    fi
    score=$((score + area / 10000))

    local penalty_16_9 penalty_16_10 penalty
    penalty_16_9="$(sunlight_abs_diff "$((w * 9))" "$((h * 16))")"
    penalty_16_10="$(sunlight_abs_diff "$((w * 10))" "$((h * 16))")"
    if (( penalty_16_9 < penalty_16_10 )); then
        penalty="$penalty_16_9"
    else
        penalty="$penalty_16_10"
    fi
    score=$((score - penalty / 12))

    printf '%s\n' "$score"
}

sunlight_choose_ranked_mode() {
    local fallback="${1:-}"
    shift || true

    local best_mode=""
    local best_score="-999999"
    local mode score
    for mode in "$@"; do
        score="$(sunlight_mode_score "$mode")"
        if (( score > best_score )); then
            best_score="$score"
            best_mode="$mode"
        fi
    done

    local fallback_score
    fallback_score="$(sunlight_mode_score "$fallback")"
    if [[ -n "$best_mode" ]] && (( best_score > 0 && best_score > fallback_score )); then
        printf '%s\n' "$best_mode"
        return 0
    fi

    printf '%s\n' "$fallback"
}

sunlight_resolve_vm_display_mode() {
    local current_mode="${1:-}"
    local environment_hint="${2:-}"
    local override_mode="${3:-}"
    shift 3 || true

    if ! sunlight_is_vm_environment "$environment_hint"; then
        printf '%s\n' "$current_mode"
        return 0
    fi

    if [[ -n "$override_mode" ]]; then
        if sunlight_parse_resolution "$override_mode" >/dev/null \
            && sunlight_resolution_in_list "$override_mode" "$@"; then
            printf '%s\n' "$override_mode"
            return 0
        fi
        printf '%s\n' "$current_mode"
        return 0
    fi

    sunlight_choose_preferred_mode "$current_mode" "$@"
}

sunlight_qemu_device_supports_resolution() {
    local device="${1:-}"
    qemu-system-x86_64 -device "$device,help" 2>/dev/null \
        | grep -q "xres=<uint32>"
}

# Resolve the QEMU accelerator without letting an "auto" launch silently look
# like KVM while it is actually using TCG.  The caller performs the host probe
# and passes "yes" only when /dev/kvm is accessible and QEMU advertises KVM.
sunlight_select_qemu_accel() {
    local requested="${1:-auto}"
    local kvm_usable="${2:-no}"

    case "$requested" in
        auto)
            if [[ "$kvm_usable" == "yes" ]]; then
                printf '%s\n' kvm
            else
                printf '%s\n' tcg
            fi
            ;;
        kvm)
            [[ "$kvm_usable" == "yes" ]] || return 1
            printf '%s\n' kvm
            ;;
        tcg)
            printf '%s\n' tcg
            ;;
        *)
            return 2
            ;;
    esac
}

# An explicit resolution is a fixed guest policy, not a suggestion to the SDL
# window.  Disabling EDID prevents QEMU's UI size from replacing xres/yres.
# Auto-selected modes retain EDID so future resize support can opt into it.
sunlight_qemu_video_device_spec() {
    local device="$1"
    local width="${2:-}"
    local height="${3:-}"
    local pin_mode="${4:-no}"
    local ioeventfd="${5:-no}"
    local spec="$device"

    if [[ "$ioeventfd" == "yes" ]]; then
        spec+=",ioeventfd=on"
    fi

    if [[ -z "$width" || -z "$height" ]]; then
        printf '%s\n' "$spec"
    elif [[ "$pin_mode" == "yes" ]]; then
        printf '%s,edid=off,xres=%s,yres=%s\n' "$spec" "$width" "$height"
    else
        printf '%s,xres=%s,yres=%s\n' "$spec" "$width" "$height"
    fi
}
