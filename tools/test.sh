#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# --- Configuration ---
TIMEOUT=60
KERNEL_ELF="target/x86_64-unknown-none/debug/sunlight-kernel"
ISO_PATH="target/sunlightos.iso"
LIMINE_BRANCH="v8.x"
LIMINE_DIR="target/limine"

# Kernel flags: conservative, no SIMD assumptions (x86-64 baseline only).
# The kernel target (x86_64-unknown-none) already disables SSE/AVX via soft-float.
KERNEL_RUSTFLAGS="-C link-arg=-Tkernel/src/arch/x86_64/linker.ld -C relocation-model=static"

# Userspace baseline: x86-64-v2 (SSE3, SSSE3, SSE4.1, SSE4.2, POPCNT, CMPXCHG16B).
# v2 enables better code generation for userspace services without requiring AVX.
# v3 (AVX/AVX2) is runtime-only for selected apps until kernel adds XSAVE/YMM switching.
SERVICE_RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone"
TLS_RUSTFLAGS="$SERVICE_RUSTFLAGS --cfg aes_force_soft --cfg polyval_force_soft --cfg poly1305_force_soft --cfg chacha20_force_soft --cfg curve25519_dalek_backend=\"serial\""
BUILD_LOG=$(mktemp)
PHASE="${1:-phase3.0}"

case "$PHASE" in
    phase2.6)
        EXPECTED_FILE="tools/tests/phase2_6.expected"
        FINAL_MARKER="[SunlightOS] Phase 2.6 OK"
        PASS_LABEL="Phase 2.6"
        NEED_DISK=false
        ;;
    phase2b1)
        EXPECTED_FILE="tools/tests/phase2b1.expected"
        FINAL_MARKER="[TTY]  exit: dirname /tmp/path/name/// -> 0"
        PASS_LABEL="Phase 2B.1 utilities"
        NEED_DISK=false
        ;;
    phase3.0)
        EXPECTED_FILE="tools/tests/phase3_0.expected"
        FINAL_MARKER="[SunlightOS] Phase 3.0 OK"
        PASS_LABEL="Phase 3.0"
        NEED_DISK=false
        ;;
    phase3.5)
        EXPECTED_FILE="tools/tests/phase3_5.expected"
        FINAL_MARKER="[SunlightOS] Phase 3.5 OK"
        PASS_LABEL="Phase 3.5"
        NEED_DISK=true
        ;;
    phase3.75)
        EXPECTED_FILE="tools/tests/wiseowl_phase3_75.expected"
        FINAL_MARKER="[WISEOWL-3.75] native gate PASS"
        PASS_LABEL="Wise Owl Phase 3.75 native"
        NEED_DISK=true
        TIMEOUT=90
        ;;
    phase3.875)
        EXPECTED_FILE="tools/tests/wiseowl_phase3_875.expected"
        FINAL_MARKER="[WISEOWL-3.875] FINAL PASS"
        PASS_LABEL="Wise Owl Phase 3.875 native"
        NEED_DISK=true
        TIMEOUT=180
        ;;
    phase3.6)
        EXPECTED_FILE="tools/tests/phase3_6.expected"
        FINAL_MARKER="[SunlightOS] Phase 3.6 OK"
        PASS_LABEL="Phase 3.6"
        NEED_DISK=false
        ;;
    sunlightd)
        EXPECTED_FILE="tools/tests/sunlightd.expected"
        FINAL_MARKER="[SunlightOS] sunlightd OK"
        PASS_LABEL="sunlightd"
        NEED_DISK=false
        ;;
    phase3.7)
        EXPECTED_FILE="tools/tests/phase3_7.expected"
        FINAL_MARKER="[SunlightOS] Phase 3.7 OK"
        PASS_LABEL="Phase 3.7"
        NEED_DISK=false
        ;;
    phase3.8)
        EXPECTED_FILE="tools/tests/phase3_8.expected"
        FINAL_MARKER="[SunlightOS] Phase 3.8 OK"
        PASS_LABEL="Phase 3.8"
        NEED_DISK=false
        ;;
    phase3.9)
        EXPECTED_FILE="tools/tests/phase3_9.expected"
        FINAL_MARKER="[TTY]  hostnamectl invoked"
        PASS_LABEL="Phase 3.9"
        NEED_DISK=false
        ;;
    phase4.5)
        EXPECTED_FILE="tools/tests/phase4_5.expected"
        FINAL_MARKER="[SunlightOS] Ring 3 Expansion OK"
        PASS_LABEL="Ring 3 Expansion"
        NEED_DISK=false
        ;;
    phase5.0)
        EXPECTED_FILE="tools/tests/phase5_0.expected"
        FINAL_MARKER="[NET]  virtio-net OK"
        PASS_LABEL="Developer Build"
        NEED_DISK=false
        ;;
    phase5.1)
        EXPECTED_FILE="tools/tests/phase5_1.expected"
        FINAL_MARKER="[NET]  Interface: eth0 MAC="
        PASS_LABEL="Userland Growth"
        NEED_DISK=false
        ;;
    phase5.2)
        EXPECTED_FILE="tools/tests/phase5_2.expected"
        FINAL_MARKER="[DHCP] OK"
        PASS_LABEL="Phase 5.2"
        NEED_DISK=false
        ;;
    phase5.3)
        EXPECTED_FILE="tools/tests/phase5_3.expected"
        FINAL_MARKER="[NET]  NetOp handlers registered"
        PASS_LABEL="Phase 5.3"
        NEED_DISK=false
        ;;
    phase5.4)
        EXPECTED_FILE="tools/tests/phase5_4.expected"
        FINAL_MARKER="[NET]  Linux process socket syscalls ready"
        PASS_LABEL="Phase 5.4"
        NEED_DISK=false
        ;;
    phase5.5)
        EXPECTED_FILE="tools/tests/phase5_5.expected"
        FINAL_MARKER="[TLS]  Handshake OK: google.com"
        PASS_LABEL="Phase 5.5"
        NEED_DISK=false
        ;;
    phase5.6)
        EXPECTED_FILE="tools/tests/phase5_6.expected"
        FINAL_MARKER="[BTRFS] Mounted /data read-only"
        PASS_LABEL="Phase 5.6"
        NEED_DISK=true
        ;;
    phase5.7)
        EXPECTED_FILE="tools/tests/phase5_7.expected"
        FINAL_MARKER="[SunlightOS] Post-Phase Stabilization OK"
        PASS_LABEL="Stabilization and Hardening"
        NEED_DISK=true
        ;;
    phase5x.0)
        EXPECTED_FILE="tools/tests/phase5x_0.expected"
        FINAL_MARKER="[DHCP] OK"
        PASS_LABEL="Phase 5.x.0"
        NEED_DISK=false
        ;;
    phase5x.1)
        EXPECTED_FILE="tools/tests/phase5x_1.expected"
        FINAL_MARKER="[DNS]  OK"
        PASS_LABEL="Phase 5.x.1"
        NEED_DISK=false
        ;;
    phase5x.2)
        EXPECTED_FILE="tools/tests/phase5x_2.expected"
        FINAL_MARKER="[TCP]  OK"
        PASS_LABEL="Phase 5.x.2"
        NEED_DISK=false
        ;;
    phase5x.3)
        EXPECTED_FILE="tools/tests/phase5x_3.expected"
        FINAL_MARKER="[M3]   ping 8.8.8.8: SUCCESS"
        PASS_LABEL="Phase 5.x.3"
        NEED_DISK=false
        ;;
    phase5x.4)
        EXPECTED_FILE="tools/tests/phase5x_4.expected"
        FINAL_MARKER="[TLS]  Handshake OK"
        PASS_LABEL="Phase 5.x.4"
        NEED_DISK=false
        ;;
    phase5x.5)
        EXPECTED_FILE="tools/tests/phase5x_5.expected"
        FINAL_MARKER="[UTIL] OK"
        PASS_LABEL="Phase 5.x.5"
        NEED_DISK=false
        ;;
    phase5x.6)
        EXPECTED_FILE="tools/tests/phase5x_6.expected"
        FINAL_MARKER="[NET]  OK"
        PASS_LABEL="Phase 5.x.6"
        NEED_DISK=false
        ;;
    dns_hosts)
        EXPECTED_FILE="tools/tests/dns_hosts.expected"
        FINAL_MARKER="[DNS] /etc/hosts loaded (hosts + hardcoded resolver active)"
        PASS_LABEL="dns_hosts"
        NEED_DISK=false
        ;;
    phase6.5.1)
        EXPECTED_FILE="tools/tests/phase6_5_1.expected"
        FINAL_MARKER="[TTY]  sysfetch invoked"
        PASS_LABEL="Phase 6.5.1"
        NEED_DISK=false
        ;;
    phase6.5.3)
        EXPECTED_FILE="tools/tests/phase6_5_3.expected"
        FINAL_MARKER="[EXEC] ls exit=0"
        PASS_LABEL="Phase 6.5.3"
        NEED_DISK=true
        ;;
    phase6.5.utils)
        EXPECTED_FILE="tools/tests/phase6_5_utils.expected"
        FINAL_MARKER="[TTY]  exit: expand /tests/expand-tabs -> 0"
        PASS_LABEL="Phase 6.5 utilities"
        NEED_DISK=false
        ;;
    phase2b4)
        EXPECTED_FILE="tools/tests/phase2b4.expected"
        FINAL_MARKER="[TTY]  exit: comm /tests/comm-a /tests/comm-b -> 0"
        PASS_LABEL="Phase 2B.4 utilities"
        NEED_DISK=false
        ;;
    phase2b5)
        EXPECTED_FILE="tools/tests/phase2b5.expected"
        FINAL_MARKER="[TTY]  exit: tee /tmp/phase2b7-tee -> 0"
        PASS_LABEL="Phase 2B.7A utilities"
        NEED_DISK=false
        ;;
    phase_shm)
        EXPECTED_FILE="tools/tests/phase_shm.expected"
        FINAL_MARKER="[SHM]  Shared memory grant: PASSED"
        PASS_LABEL="Shared Memory Grant"
        NEED_DISK=false
        ;;
    phase_sec)
        EXPECTED_FILE="tools/tests/phase_sec.expected"
        FINAL_MARKER="[SEC]  Security hardening: PASSED"
        PASS_LABEL="Security Hardening"
        NEED_DISK=false
        ;;
    phase0.9)
        EXPECTED_FILE="tools/tests/phase0_9.expected"
        FINAL_MARKER="[ENTROPY] secure source=virtio-rng conditioner=ChaCha20 readiness=ready"
        PASS_LABEL="Phase 0.9 Secure Randomness"
        NEED_DISK=false
        ;;
    mm2b)
        EXPECTED_FILE="tools/tests/mm2b.expected"
        FINAL_MARKER="[MM-2B] PMM accounting returned to baseline: OK"
        PASS_LABEL="MM-2B 12-core TLB Shootdown"
        NEED_DISK=false
        ;;
    mm2d)
        EXPECTED_FILE="tools/tests/mm2d.expected"
        FINAL_MARKER="[MM-2D] focused munmap/shootdown gate: OK"
        PASS_LABEL="MM-2D Anonymous Munmap"
        NEED_DISK=false
        ;;
    mm2e)
        EXPECTED_FILE="tools/tests/mm2e.expected"
        FINAL_MARKER="[MM-2E] focused mprotect/permission shootdown gate: OK"
        PASS_LABEL="MM-2E Anonymous Mprotect"
        NEED_DISK=false
        ;;
    swap1)
        EXPECTED_FILE="tools/tests/swap1.expected"
        FINAL_MARKER="[SWAP-1] focused pressure gate: OK"
        PASS_LABEL="SWAP-1 Multi-pool Pressure"
        NEED_DISK=false
        ;;
    top)
        EXPECTED_FILE="tools/tests/top.expected"
        FINAL_MARKER="[TOP] rendering"
        PASS_LABEL="top"
        NEED_DISK=false
        ;;
    memory-accounting)
        EXPECTED_FILE="tools/tests/memory_accounting.expected"
        FINAL_MARKER="[MEMORY-ACCOUNTING] FINAL PASS"
        PASS_LABEL="Physical Memory Accounting Phase 1"
        NEED_DISK=false
        TIMEOUT=90
        ;;
    tzctl)
        EXPECTED_FILE="tools/tests/tzctl.expected"
        FINAL_MARKER="[TTY]  cmd: tzctl get -> Active: Asia/Tehran"
        PASS_LABEL="tzctl"
        NEED_DISK=false
        ;;
    session-foundation)
        EXPECTED_FILE="tools/tests/session_foundation.expected"
        FINAL_MARKER="[SESSION-FOUNDATION] FINAL PASS"
        PASS_LABEL="Session Foundation"
        NEED_DISK=false
        TIMEOUT=180
        ;;
    session-configuration)
        EXPECTED_FILE="tools/tests/session_configuration.expected"
        FINAL_MARKER="[SESSION-CONFIG] FINAL PASS"
        PASS_LABEL="Session Configuration"
        NEED_DISK=false
        TIMEOUT=360
        ;;
    welcome-wizard)
        EXPECTED_FILE="tools/tests/welcome_wizard.expected"
        FINAL_MARKER="[WELCOME-WIZARD] FINAL PASS"
        PASS_LABEL="Welcome Wizard"
        NEED_DISK=false
        TIMEOUT=360
        ;;
    *)
        echo "[test] Unsupported gate '$PHASE'. Supported: phase0.9 phase2.6 phase2b1 phase3.0 phase3.5 phase3.6 phase3.7 phase3.8 phase3.9 phase4.5 phase5.0 phase5.1 phase5.2 phase5.3 phase5.4 phase5.5 phase5.6 phase5.7 phase5x.0 phase5x.1 phase5x.2 phase5x.3 phase5x.4 phase5x.5 phase5x.6 dns_hosts phase6.5.1 phase6.5.3 phase6.5.utils phase_shm phase_sec mm2b mm2d mm2e swap1 session-foundation session-configuration welcome-wizard sunlightd top tzctl memory-accounting"
        exit 2
        ;;
esac

mapfile -t EXPECTED < <(grep -Ev '^[[:space:]]*($|#)' "$EXPECTED_FILE")

# --- Step 1: Build service binaries first ---
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-init --release >"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-timer-server --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-swapd --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-kbd --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-mouse --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-usb-mouse --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-deviced --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-networkd --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-resolved --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-powerd --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-thermald --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-vfs-server --release >>"$BUILD_LOG" 2>&1
if [[ "$PHASE" == "session-foundation" ]]; then
    SUNLIGHT_INJECT_PHASE=session_foundation RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tty-server --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "session-configuration" ]]; then
    SUNLIGHT_INJECT_PHASE=session_configuration RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tty-server --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "welcome-wizard" ]]; then
    SUNLIGHT_INJECT_PHASE=welcome_wizard RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tty-server --release >>"$BUILD_LOG" 2>&1
else
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tty-server --release >>"$BUILD_LOG" 2>&1
fi
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package pty_server --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-net-server --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package timezone_service --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-timed --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tz --features tzutils --bin tzutils --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package rand_service --release >>"$BUILD_LOG" 2>&1
if [[ "$PHASE" == "session-configuration" ]]; then
    SUNLIGHT_INJECT_PHASE=session_configuration RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-sessiond --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "welcome-wizard" ]]; then
    SUNLIGHT_INJECT_PHASE=welcome_wizard RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-sessiond --release >>"$BUILD_LOG" 2>&1
else
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-sessiond --release >>"$BUILD_LOG" 2>&1
fi
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-sessionctl --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-startup-fixture --release >>"$BUILD_LOG" 2>&1
if [[ "$PHASE" == "welcome-wizard" ]]; then
    SUNLIGHT_INJECT_PHASE=welcome_wizard RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-welcome --bin welcome --release >>"$BUILD_LOG" 2>&1
else
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-welcome --bin welcome --release >>"$BUILD_LOG" 2>&1
fi
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlightd --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-niced --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-gcd --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlightctl --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-uac --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-sm --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-kv --features sunlightos --no-default-features --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-kvctl --features sunlightos --no-default-features --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$TLS_RUSTFLAGS" cargo build --package sunlight-tls --features sunlightos --no-default-features --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package certificatectl --features sunlightos --no-default-features --release >>"$BUILD_LOG" 2>&1
# sunshell (includes localectl builtin + pulls in support libs e.g. sunlight-locale, sunlight-tz)
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunshell --features sunlight --no-default-features --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-utils --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-net-utils --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-top --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package memoryctl --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-fetch --features sunlightos --no-default-features --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-sunsay --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-zoxide --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-dict --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-hangman --release >>"$BUILD_LOG" 2>&1

RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package cpu-utils --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-display --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package mezzo --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package mezzoctl --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package eyes --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-runner --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sun-exec --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sun-open --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-terminal --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-chronos --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tasks --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-vortex-shell --release >>"$BUILD_LOG" 2>&1
if [[ "$PHASE" != "mm2b" && "$PHASE" != "mm2d" && "$PHASE" != "swap1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-bench --release >>"$BUILD_LOG" 2>&1
fi
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-calculator --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-widget-gallery --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-silicon-echoes --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-files --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-light-lens --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-edit --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-writer --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-calendar --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-reminders --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-devices --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package rappid-rabbit --features dom --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-api-lab --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-dialogd --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-control-panel --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-thumbd --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-clipd --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-clipman --release >>"$BUILD_LOG" 2>&1
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-memory --bin wiseowl-memoryd --bin wiseowl-memoryctl --features sunlightos --no-default-features --release >>"$BUILD_LOG" 2>&1
if [[ "$PHASE" == "phase3.75" || "$PHASE" == "phase3.875" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-memorydb --bin wiseowl-memorydb --bin wiseowl-memorydbctl --features sunlightos,phase375-test --no-default-features --release >>"$BUILD_LOG" 2>&1
else
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-memorydb --bin wiseowl-memorydb --bin wiseowl-memorydbctl --features sunlightos --no-default-features --release >>"$BUILD_LOG" 2>&1
fi
if [[ "$PHASE" == "phase3.75" || "$PHASE" == "phase3.875" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-index --bin wiseowl-indexd --bin wiseowl-indexctl --features sunlightos,phase375-test --no-default-features --release >>"$BUILD_LOG" 2>&1
else
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-index --bin wiseowl-indexd --bin wiseowl-indexctl --features sunlightos --no-default-features --release >>"$BUILD_LOG" 2>&1
fi
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-emoji-picker --release >>"$BUILD_LOG" 2>&1
# --- Step 1b: Create FAT32 disk image (phase3.5+) ---
if [[ "$NEED_DISK" == "true" ]]; then
    bash tools/disk.sh >>"$BUILD_LOG" 2>&1
fi

# --- Step 2: Build kernel ---
KERNEL_FEATURES=""
if [[ "$PHASE" == "phase2b1" || "$PHASE" == "phase3.6" || "$PHASE" == "phase3.7" || "$PHASE" == "phase3.8" || "$PHASE" == "phase3.9" || "$PHASE" == "phase3.75" || "$PHASE" == "phase3.875" || "$PHASE" == "phase6.5.1" || "$PHASE" == "phase6.5.3" || "$PHASE" == "phase6.5.utils" || "$PHASE" == "phase2b4" || "$PHASE" == "phase2b5" || "$PHASE" == "top" || "$PHASE" == "tzctl" || "$PHASE" == "session-foundation" || "$PHASE" == "session-configuration" || "$PHASE" == "welcome-wizard" ]]; then
    KERNEL_FEATURES="--features key_inject"
elif [[ "$PHASE" == "phase_sec" ]]; then
    KERNEL_FEATURES="--features mm2a_test_injection"
elif [[ "$PHASE" == "mm2b" ]]; then
    KERNEL_FEATURES="--features mm2b_smp_test"
elif [[ "$PHASE" == "mm2d" ]]; then
    KERNEL_FEATURES="--features mm2d_munmap_test"
elif [[ "$PHASE" == "mm2e" ]]; then
    KERNEL_FEATURES="--features mm2e_mprotect_test"
elif [[ "$PHASE" == "swap1" ]]; then
    KERNEL_FEATURES="--features swap1_test"
elif [[ "$PHASE" == "memory-accounting" ]]; then
    KERNEL_FEATURES="--features memory_accounting_test"
fi
EXTRA_ENV=()
if [[ "$PHASE" == "phase2b1" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=phase2b1)
elif [[ "$PHASE" == "phase3.9" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=phase3.9)
elif [[ "$PHASE" == "phase3.75" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=wiseowl3.75)
elif [[ "$PHASE" == "phase3.875" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=wiseowl3.875)
elif [[ "$PHASE" == "phase6.5.1" ]]; then
    # Reuse the phase3.9 key sequence — it logs in and types sysfetch
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=phase3.9)
elif [[ "$PHASE" == "phase6.5.3" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=phase6.5.3)
elif [[ "$PHASE" == "phase6.5.utils" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=phase6.5.utils)
elif [[ "$PHASE" == "phase2b4" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=phase2b4)
elif [[ "$PHASE" == "phase2b5" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=phase2b5)
elif [[ "$PHASE" == "top" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=top)
elif [[ "$PHASE" == "tzctl" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=tzctl)
elif [[ "$PHASE" == "session-foundation" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=session_foundation)
elif [[ "$PHASE" == "session-configuration" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=session_configuration)
elif [[ "$PHASE" == "welcome-wizard" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=welcome_wizard)
elif [[ "$PHASE" == "phase4.5" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=phase4.5)
elif [[ "$PHASE" == phase5* || "$PHASE" == phase5x* || "$PHASE" == "dns_hosts" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE="$PHASE")
fi
touch kernel/src/main.rs
env "${EXTRA_ENV[@]}" cargo build --package sunlight-kernel $KERNEL_FEATURES >>"$BUILD_LOG" 2>&1

# --- Step 3–5: Hybrid ISO (BIOS + UEFI) via shared helper ---
LIMINE_BRANCH="$LIMINE_BRANCH" "$SCRIPT_DIR/make_hybrid_iso.sh" \
    "$KERNEL_ELF" "$ISO_PATH" "$PROJECT_ROOT/$LIMINE_DIR" "$PROJECT_ROOT" \
    >>"$BUILD_LOG" 2>&1

# --- Step 6: Launch QEMU with timeout ---
KVM_FLAGS=""
if [[ -r /dev/kvm && -w /dev/kvm ]]; then
    KVM_FLAGS="-enable-kvm"
fi

QEMU_OUTPUT=$(mktemp)
trap "rm -f $QEMU_OUTPUT $BUILD_LOG" EXIT

# Extra QEMU flags for phases that need a virtio-blk disk
DISK_FLAGS=""
if [[ "$NEED_DISK" == "true" && -f "target/test.img" ]]; then
    DISK_FLAGS="-drive id=hd0,file=target/test.img,if=none,format=raw -device virtio-blk-pci,disable-modern=on,drive=hd0"
fi

# Extra QEMU flags for Phase 5 networking (virtio-net). Always add for phase5* so PCI scan + driver init succeed.
NET_FLAGS=""
if [[ "$PHASE" == phase5* || "$PHASE" == phase5x* ]]; then
    NET_FLAGS="-netdev user,id=net0 -device virtio-net-pci,netdev=net0,disable-modern=on"
fi

set +e
QEMU_SMP="${SUNLIGHT_TEST_CPUS:-2}"
if [[ "$PHASE" == "mm2b" ]]; then
    QEMU_SMP=12
elif [[ "$PHASE" == "mm2d" ]]; then
    QEMU_SMP=4
elif [[ "$PHASE" == "mm2e" ]]; then
    QEMU_SMP=4
elif [[ "$PHASE" == "swap1" ]]; then
    QEMU_SMP=4
fi
qemu-system-x86_64 \
    -cdrom "$ISO_PATH" \
    -serial file:"$QEMU_OUTPUT" \
    -display none \
    -m 1024M \
    -smp "$QEMU_SMP" \
    $KVM_FLAGS \
    -device virtio-rng-pci,disable-modern=on \
    -device qemu-xhci,id=xhci -device usb-mouse,bus=xhci.0 \
    $DISK_FLAGS \
    $NET_FLAGS \
    -no-reboot \
    -no-shutdown >>"$BUILD_LOG" 2>&1 &
QEMU_PID=$!

# Wait up to TIMEOUT seconds, checking if QEMU is still running
for ((i=0; i<TIMEOUT; i++)); do
    if ! kill -0 $QEMU_PID 2>/dev/null; then
        break
    fi
    # Check if the final runtime milestone is present (early exit on success).
    if grep -Fq "$FINAL_MARKER" "$QEMU_OUTPUT" 2>/dev/null \
        && { [[ "$PHASE" == "mm2b" ]] || grep -Fq "[timer] 100 ticks elapsed" "$QEMU_OUTPUT" 2>/dev/null; }; then
        sleep 1
        break
    fi
    sleep 1
done

# If still running, kill it
if kill -0 $QEMU_PID 2>/dev/null; then
    kill -TERM $QEMU_PID 2>/dev/null || true
    sleep 1
    kill -KILL $QEMU_PID 2>/dev/null || true
fi

wait $QEMU_PID 2>/dev/null
QEMU_EXIT=$?
set -e

if [[ "$PHASE" == "phase3.75" ]]; then
    cp "$QEMU_OUTPUT" target/wiseowl-phase375-serial.log
fi
if [[ "$PHASE" == "phase3.875" ]]; then
    cp "$QEMU_OUTPUT" target/wiseowl-phase3875-serial.log
fi

ALL_FOUND=true
PMM_LINE=$(grep -E '^\[PMM\] [0-9]+/[0-9]+ MiB free$' "$QEMU_OUTPUT" | head -n1 || true)
if [[ -n "$PMM_LINE" ]]; then
    :
else
    ALL_FOUND=false
fi

for expected in "${EXPECTED[@]}"; do
    if ! grep -Fq "$expected" "$QEMU_OUTPUT"; then
        ALL_FOUND=false
    fi
done

if [[ "$ALL_FOUND" == true ]]; then
    echo "══════════════════════════════════════"
    echo "  SunlightOS — ${PASS_LABEL} Boot Gate"
    echo "══════════════════════════════════════"
    if [[ -n "$PMM_LINE" ]]; then
        echo "$PMM_LINE"
    fi
    for expected in "${EXPECTED[@]}"; do
        echo "$expected"
    done
    echo "══════════════════════════════════════"
    echo "✓ ${PASS_LABEL} gate PASSED"
    exit 0
else
    echo "[test] --- build and tool output ---"
    cat "$BUILD_LOG"
    echo "[test] -----------------------------"
    echo ""
    echo "[test] --- QEMU serial output ---"
    cat "$QEMU_OUTPUT"
    echo "[test] --------------------------"
    echo ""

    if [[ -n "$PMM_LINE" ]]; then
        echo "[test] ✓ Found: [PMM] .../... MiB free"
    else
        echo "[test] ✗ Missing: [PMM] .../... MiB free"
    fi

    for expected in "${EXPECTED[@]}"; do
        if grep -Fq "$expected" "$QEMU_OUTPUT"; then
            echo "[test] ✓ Found: $expected"
        else
            echo "[test] ✗ Missing: $expected"
        fi
    done
    echo "[test] ✗ ${PASS_LABEL} gate FAILED"
    exit 1
fi
