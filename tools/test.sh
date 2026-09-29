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
    wiseowl-identity-phase-a)
        EXPECTED_FILE="tools/tests/wiseowl_identity_phase_a.expected"
        FINAL_MARKER="[WISEOWL-IDENTITY-A] native gate PASS"
        PASS_LABEL="Wise Owl Identity Phase A"
        NEED_DISK=true
        TIMEOUT=120
        ;;
    wiseowl-identity-phase-b)
        EXPECTED_FILE="tools/tests/wiseowl_identity_phase_b.expected"
        FINAL_MARKER="[WISEOWL-IDENTITY-B] native gate PASS"
        PASS_LABEL="Wise Owl Identity Phase B propagation"
        NEED_DISK=true
        TIMEOUT=120
        ;;
    wiseowl-identity-phase-c)
        EXPECTED_FILE="tools/tests/wiseowl_identity_phase_c.expected"
        FINAL_MARKER="[WISEOWL-IDENTITY-C] replacement ready"
        PASS_LABEL="Wise Owl Identity Phase C activation lifecycle"
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
    helios-proven-tier1)
        EXPECTED_FILE="tools/tests/helios_proven_tier1.expected"
        FINAL_MARKER="LINUX-PROBE TIER1 PASS"
        PASS_LABEL="Helios Proven Tier 1"
        NEED_DISK=false
        TIMEOUT=150
        ;;
    helios-thread-probe)
        EXPECTED_FILE="tools/tests/helios_thread_probe.expected"
        FINAL_MARKER="THREAD_PROBE PASS"
        PASS_LABEL="Helios Linux thread probe"
        NEED_DISK=false
        TIMEOUT=150
        ;;
    helios-io-probe)
        EXPECTED_FILE="tools/tests/helios_io_probe.expected"
        FINAL_MARKER="IO_SOCKET PASS"
        PASS_LABEL="Helios Linux nonblocking I/O probe"
        NEED_DISK=false
        TIMEOUT=150
        ;;
    helios-open-largefile-probe)
        EXPECTED_FILE="tools/tests/helios_open_largefile_probe.expected"
        FINAL_MARKER="OPEN_LARGEFILE PASS"
        PASS_LABEL="Helios Linux O_LARGEFILE open probe"
        NEED_DISK=false
        TIMEOUT=150
        ;;
    helios-note-regression)
        EXPECTED_FILE="tools/tests/helios_note_regression.expected"
        FINAL_MARKER="[HELIOS-NOTE] interactive-ready"
        PASS_LABEL="Helios Note regression"
        NEED_DISK=false
        TIMEOUT=120
        ;;
    helios-static-runtime)
        EXPECTED_FILE="tools/tests/helios_static_runtime.expected"
        FINAL_MARKER="[HELIOS-NOTE] interactive-ready"
        PASS_LABEL="Helios static runtime"
        NEED_DISK=false
        TIMEOUT=180
        ;;
    yazi-baseline)
        EXPECTED_FILE="tools/tests/yazi_baseline.expected"
        FINAL_MARKER="[ELF] header rejected: NotStaticExecutable"
        PASS_LABEL="Yazi v26.9.1 stock ELF baseline"
        NEED_DISK=false
        TIMEOUT=120
        ;;
    yazi-phase1)
        EXPECTED_FILE="tools/tests/yazi_phase1.expected"
        # The gate observes both the first frame and an injected arrow key
        # arriving through the foreground TTY stdin path.
        FINAL_MARKER="[HELIOS-YAZI] arrow-down read from tty"
        PASS_LABEL="Yazi v26.9.1 Phase 1 runtime"
        NEED_DISK=false
        TIMEOUT=180
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
    wiseowl-phase4a)
        EXPECTED_FILE="tools/tests/wiseowl_phase4a.expected"
        FINAL_MARKER="[WISEOWL-BRAIN] FINAL PASS"
        PASS_LABEL="Wise Owl Phase 4A Brain Foundation"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    wiseowl-phase4b)
        EXPECTED_FILE="tools/tests/wiseowl_phase4b.expected"
        FINAL_MARKER="[WISEOWL-BRAIN] FINAL PASS"
        PASS_LABEL="Wise Owl Phase 4B Brain Foundation"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    wiseowl-foundation-v1)
        EXPECTED_FILE="tools/tests/wiseowl_foundation_v1.expected"
        FINAL_MARKER="[WISEOWL-BRAIN] WELCOME_INTEGRATION PASS"
        PASS_LABEL="Wise Owl Brain Foundation v1"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    wiseowl-executor-v1)
        EXPECTED_FILE="tools/tests/wiseowl_executor_v1.expected"
        FINAL_MARKER="EXECUTION_RESULT PASS"
        PASS_LABEL="Wise Owl Trusted Action Executor v1"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    wiseowl-planner-v1)
        EXPECTED_FILE="tools/tests/wiseowl_planner_v1.expected"
        FINAL_MARKER="[WISEOWL-PLANNER] EXECUTION_RESULT PASS"
        PASS_LABEL="Wise Owl Bounded Action Planner v1"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    wiseowl-coordinator-v1)
        EXPECTED_FILE="tools/tests/wiseowl_coordinator_v1.expected"
        FINAL_MARKER="[WISEOWL-COORD] COMPLETE PASS"
        PASS_LABEL="Wise Owl Conversational Action Coordinator v1"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    wiseowl-outcome-observer-v1)
        EXPECTED_FILE="tools/tests/wiseowl_outcome_observer_v1.expected"
        FINAL_MARKER="[WISEOWL-OUTCOME] COMPLETE PASS"
        PASS_LABEL="Wise Owl Action Outcome Observer v1"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    wiseowl-action-receipt-v1)
        EXPECTED_FILE="tools/tests/wiseowl_action_receipt_v1.expected"
        FINAL_MARKER="[WISEOWL-RECEIPT] COMPLETE PASS"
        PASS_LABEL="Wise Owl Action Receipt Ledger v1"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    wiseowl-graphical-console-v1)
        EXPECTED_FILE="tools/tests/wiseowl_graphical_console_v1.expected"
        FINAL_MARKER="[WISEOWL-GUI] WINDOW_CREATED PASS"
        PASS_LABEL="Wise Owl Graphical Console v1"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    wiseowl-gui-conversation-v1)
        EXPECTED_FILE="tools/tests/wiseowl_gui_conversation_v1.expected"
        FINAL_MARKER="[WISEOWL-GUI-CHAT] COMPLETE PASS"
        PASS_LABEL="Wise Owl GUI Conversation v1"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    wiseowl-gui-bridge-foundation-v1)
        EXPECTED_FILE="tools/tests/wiseowl_gui_bridge_foundation_v1.expected"
        FINAL_MARKER="[WISEOWL-GUI-BRIDGE] COMPLETE PASS"
        PASS_LABEL="Wise Owl GUI Bridge Foundation v1"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    wiseowl-trusted-session-readiness-v1)
        EXPECTED_FILE="tools/tests/wiseowl_trusted_session_readiness_v1.expected"
        FINAL_MARKER="[WISEOWL-TRUST] COMPLETE PASS"
        PASS_LABEL="Wise Owl Trusted Session Attestation and Readiness Evidence v1"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    wiseowl-gui-live-action-activation-v1)
        EXPECTED_FILE="tools/tests/wiseowl_gui_live_action_activation_v1.expected"
        FINAL_MARKER="[WISEOWL-GUI-ACTIVE] COMPLETE PASS"
        PASS_LABEL="Wise Owl GUI Live Action Activation v1"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    wiseowl-delegated-session-lifecycle-ipc-v1)
        EXPECTED_FILE="tools/tests/wiseowl_delegated_session_lifecycle_ipc_v1.expected"
        FINAL_MARKER="[WISEOWL-DELEGATION] COMPLETE PASS"
        PASS_LABEL="Wise Owl Delegated Session Authority and Lifecycle IPC v1"
        NEED_DISK=true
        TIMEOUT=360
        ;;
    audio)
        EXPECTED_FILE="tools/tests/audio.expected"
        FINAL_MARKER="[AUDIOD] test-tone done"
        PASS_LABEL="Audio playback foundation"
        NEED_DISK=false
        TIMEOUT=90
        ;;
    *)
        echo "[test] Unsupported gate '$PHASE'. See tools/tests for supported gates."
        exit 2
        ;;
esac

# Allow short diagnostic runs without weakening the phase's default gate.
if [[ -n "${SUNLIGHT_TEST_TIMEOUT:-}" ]]; then
    [[ "$SUNLIGHT_TEST_TIMEOUT" =~ ^[1-9][0-9]*$ ]] || exit 2
    TIMEOUT="$SUNLIGHT_TEST_TIMEOUT"
fi

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
if [[ "$PHASE" == "audio" ]]; then
    SUNLIGHT_INJECT_AUDIO_TEST=1 RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-audiod --release >>"$BUILD_LOG" 2>&1
else
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-audiod --release >>"$BUILD_LOG" 2>&1
fi
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-vfs-server --release >>"$BUILD_LOG" 2>&1
if [[ "$PHASE" == "session-foundation" ]]; then
    SUNLIGHT_INJECT_PHASE=session_foundation RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tty-server --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "session-configuration" ]]; then
    SUNLIGHT_INJECT_PHASE=session_configuration RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tty-server --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "welcome-wizard" ]]; then
    SUNLIGHT_INJECT_PHASE=welcome_wizard RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-tty-server --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-phase4a" || "$PHASE" == "wiseowl-phase4b" || "$PHASE" == "wiseowl-foundation-v1" ]]; then
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
elif [[ "$PHASE" == "wiseowl-delegated-session-lifecycle-ipc-v1" ]]; then
    SUNLIGHT_INJECT_PHASE=wiseowl_delegated_session_lifecycle_ipc_v1 RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-sessiond --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-phase4a" || "$PHASE" == "wiseowl-phase4b" || "$PHASE" == "wiseowl-foundation-v1" ]]; then
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
if [[ "$PHASE" == "wiseowl-graphical-console-v1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-console --bin wiseowl --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-gui-conversation-v1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-console --bin wiseowl --features conversation-v1-test --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-delegated-session-lifecycle-ipc-v1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-console --bin wiseowl --features delegated-session-lifecycle-ipc-v1-test --release >>"$BUILD_LOG" 2>&1
fi
if [[ "$PHASE" == "wiseowl-identity-phase-c" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlightd --features identity-phase-c-test --release >>"$BUILD_LOG" 2>&1
else
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlightd --release >>"$BUILD_LOG" 2>&1
fi
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
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package melody-mina --release >>"$BUILD_LOG" 2>&1
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
if [[ "$PHASE" == "wiseowl-identity-phase-c" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-memory --bin wiseowl-memoryd --bin wiseowl-memoryctl --features sunlightos,identity-phase-b-test,identity-phase-c-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-identity-phase-b" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-memory --bin wiseowl-memoryd --bin wiseowl-memoryctl --features sunlightos,identity-phase-b-test --no-default-features --release >>"$BUILD_LOG" 2>&1
else
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-memory --bin wiseowl-memoryd --bin wiseowl-memoryctl --features sunlightos --no-default-features --release >>"$BUILD_LOG" 2>&1
fi
if [[ "$PHASE" == "wiseowl-identity-phase-a" ]]; then
    # The Phase A gate injects one stop after a durable staged ROOT so the
    # next native boot exercises staged recovery on the same state volume.
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-memorydb --bin wiseowl-memorydb --bin wiseowl-memorydbctl --features sunlightos,identity-phase-a-test,identity-phase-a-fault-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-identity-phase-b" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-memorydb --bin wiseowl-memorydb --bin wiseowl-memorydbctl --features sunlightos,identity-phase-b-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-identity-phase-c" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-memorydb --bin wiseowl-memorydb --bin wiseowl-memorydbctl --features sunlightos,identity-phase-b-test,identity-phase-c-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "phase3.75" || "$PHASE" == "phase3.875" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-memorydb --bin wiseowl-memorydb --bin wiseowl-memorydbctl --features sunlightos,phase375-test --no-default-features --release >>"$BUILD_LOG" 2>&1
else
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-memorydb --bin wiseowl-memorydb --bin wiseowl-memorydbctl --features sunlightos --no-default-features --release >>"$BUILD_LOG" 2>&1
fi
if [[ "$PHASE" == "wiseowl-identity-phase-b" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-index --bin wiseowl-indexd --bin wiseowl-indexctl --features sunlightos,identity-phase-b-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-identity-phase-c" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-index --bin wiseowl-indexd --bin wiseowl-indexctl --features sunlightos,identity-phase-b-test,identity-phase-c-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "phase3.75" || "$PHASE" == "phase3.875" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-index --bin wiseowl-indexd --bin wiseowl-indexctl --features sunlightos,phase375-test --no-default-features --release >>"$BUILD_LOG" 2>&1
else
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-index --bin wiseowl-indexd --bin wiseowl-indexctl --features sunlightos --no-default-features --release >>"$BUILD_LOG" 2>&1
fi
if [[ "$PHASE" == "wiseowl-executor-v1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-brain --bin wiseowl-braind --bin wiseowl-brainctl --features sunlightos,executor-v1-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-planner-v1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-brain --bin wiseowl-braind --bin wiseowl-brainctl --features sunlightos,planner-v1-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-coordinator-v1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-brain --bin wiseowl-braind --bin wiseowl-brainctl --features sunlightos,coordinator-v1-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-outcome-observer-v1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-brain --bin wiseowl-braind --bin wiseowl-brainctl --features sunlightos,outcome-observer-v1-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-action-receipt-v1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-brain --bin wiseowl-braind --bin wiseowl-brainctl --features sunlightos,action-receipt-v1-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-gui-bridge-foundation-v1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-brain --bin wiseowl-braind --bin wiseowl-brainctl --features sunlightos,gui-bridge-foundation-v1-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-trusted-session-readiness-v1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-brain --bin wiseowl-braind --bin wiseowl-brainctl --features sunlightos,trusted-session-readiness-v1-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-gui-live-action-activation-v1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-brain --bin wiseowl-braind --bin wiseowl-brainctl --features sunlightos,gui-live-action-activation-v1-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-delegated-session-lifecycle-ipc-v1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-brain --bin wiseowl-braind --bin wiseowl-brainctl --features sunlightos,delegated-session-lifecycle-ipc-v1-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-identity-phase-c" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-brain --bin wiseowl-braind --bin wiseowl-brainctl --features sunlightos,identity-phase-b-test,identity-phase-c-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-identity-phase-b" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-brain --bin wiseowl-braind --bin wiseowl-brainctl --features sunlightos,identity-phase-b-test --no-default-features --release >>"$BUILD_LOG" 2>&1
elif [[ "$PHASE" == "wiseowl-phase4a" || "$PHASE" == "wiseowl-phase4b" || "$PHASE" == "wiseowl-foundation-v1" ]]; then
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-brain --bin wiseowl-braind --bin wiseowl-brainctl --features sunlightos,phase4a-test --no-default-features --release >>"$BUILD_LOG" 2>&1
else
    RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package wiseowl-brain --bin wiseowl-braind --bin wiseowl-brainctl --features sunlightos --no-default-features --release >>"$BUILD_LOG" 2>&1
fi
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build --package sunlight-emoji-picker --release >>"$BUILD_LOG" 2>&1
# --- Step 1b: Create the FAT32 volume required by this gate. ---
if [[ "$PHASE" == "wiseowl-identity-phase-a" ]]; then
    mkdir -p target/wiseowl-identity-phase-a-evidence
    : >target/wiseowl-identity-phase-a-evidence/identity-fingerprint.txt
fi
if [[ "$NEED_DISK" == "true" ]]; then
    if [[ "$PHASE" == wiseowl-* || "$PHASE" == "phase3.75" || "$PHASE" == "phase3.875" ]]; then
        if [[ "$PHASE" == "wiseowl-identity-phase-b" \
            && -s target/wiseowl-identity-phase-a-evidence/identity-fingerprint.txt \
            && -f target/state-test.img ]]; then
            echo "reusing Phase A state image for Phase B" >>"$BUILD_LOG"
        else
            bash tools/state-disk.sh target/state-test.img >>"$BUILD_LOG" 2>&1
        fi
    else
        bash tools/disk.sh >>"$BUILD_LOG" 2>&1
    fi
fi

# --- Step 2: Build kernel ---
if [[ "$PHASE" == "helios-proven-tier1" || "$PHASE" == "helios-static-runtime" || "$PHASE" == "helios-thread-probe" || "$PHASE" == "helios-io-probe" ]]; then
    "$SCRIPT_DIR/build_helios_probes.sh" >>"$BUILD_LOG" 2>&1
fi
if [[ "$PHASE" == "helios-note-regression" || "$PHASE" == "helios-static-runtime" ]]; then
    RUSTFLAGS="-C relocation-model=static -C target-feature=+crt-static -C link-arg=-no-pie" \
        cargo build --package helios-note --release --target x86_64-unknown-linux-musl >>"$BUILD_LOG" 2>&1
    bash "$SCRIPT_DIR/stamp_helios_elf.sh" target/x86_64-unknown-linux-musl/release/helios-note >>"$BUILD_LOG" 2>&1
fi
if [[ "$PHASE" == "yazi-baseline" || "$PHASE" == "yazi-phase1" ]]; then
    bash "$SCRIPT_DIR/build_yazi.sh" >>"$BUILD_LOG" 2>&1
fi
KERNEL_FEATURES=""
if [[ "$PHASE" == "helios-open-largefile-probe" || "$PHASE" == "helios-proven-tier1" || "$PHASE" == "helios-thread-probe" || "$PHASE" == "helios-io-probe" || "$PHASE" == "helios-note-regression" || "$PHASE" == "helios-static-runtime" || "$PHASE" == "yazi-baseline" || "$PHASE" == "yazi-phase1" || "$PHASE" == "phase2b1" || "$PHASE" == "phase3.6" || "$PHASE" == "phase3.7" || "$PHASE" == "phase3.8" || "$PHASE" == "phase3.9" || "$PHASE" == "phase3.75" || "$PHASE" == "phase3.875" || "$PHASE" == "wiseowl-identity-phase-a" || "$PHASE" == "phase6.5.1" || "$PHASE" == "phase6.5.3" || "$PHASE" == "phase6.5.utils" || "$PHASE" == "phase2b4" || "$PHASE" == "phase2b5" || "$PHASE" == "top" || "$PHASE" == "tzctl" || "$PHASE" == "session-foundation" || "$PHASE" == "session-configuration" || "$PHASE" == "welcome-wizard" || "$PHASE" == "wiseowl-phase4a" || "$PHASE" == "wiseowl-phase4b" || "$PHASE" == "wiseowl-foundation-v1" || "$PHASE" == "wiseowl-executor-v1" || "$PHASE" == "wiseowl-planner-v1" || "$PHASE" == "wiseowl-coordinator-v1" || "$PHASE" == "wiseowl-outcome-observer-v1" || "$PHASE" == "wiseowl-action-receipt-v1" || "$PHASE" == "wiseowl-graphical-console-v1" || "$PHASE" == "wiseowl-gui-conversation-v1" || "$PHASE" == "wiseowl-gui-bridge-foundation-v1" || "$PHASE" == "wiseowl-trusted-session-readiness-v1" || "$PHASE" == "wiseowl-delegated-session-lifecycle-ipc-v1" ]]; then
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
elif [[ "$PHASE" == "wiseowl-graphical-console-v1" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=wiseowl_graphical_console_v1)
elif [[ "$PHASE" == "wiseowl-gui-conversation-v1" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=wiseowl_gui_conversation_v1)
elif [[ "$PHASE" == "wiseowl-delegated-session-lifecycle-ipc-v1" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=wiseowl_delegated_session_lifecycle_ipc_v1)
elif [[ "$PHASE" == "wiseowl-phase4a" || "$PHASE" == "wiseowl-phase4b" || "$PHASE" == "wiseowl-foundation-v1" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=welcome_wizard)
elif [[ "$PHASE" == "phase4.5" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=phase4.5)
elif [[ "$PHASE" == "helios-proven-tier1" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=helios-proven-tier1)
elif [[ "$PHASE" == "helios-thread-probe" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=helios-thread-probe)
elif [[ "$PHASE" == "helios-io-probe" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=helios-io-probe)
elif [[ "$PHASE" == "helios-open-largefile-probe" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=helios-open-largefile-probe)
elif [[ "$PHASE" == "helios-note-regression" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=helios-note-regression)
elif [[ "$PHASE" == "helios-static-runtime" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=helios-static-runtime)
elif [[ "$PHASE" == "yazi-baseline" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=yazi-baseline)
elif [[ "$PHASE" == "yazi-phase1" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE=yazi-phase1)
elif [[ "$PHASE" == phase5* || "$PHASE" == phase5x* || "$PHASE" == "dns_hosts" ]]; then
    EXTRA_ENV+=(SUNLIGHT_INJECT_PHASE="$PHASE")
fi
touch kernel/src/main.rs
touch "$PROJECT_ROOT/services/sunlightd/src/main.rs"
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
QEMU_OUTPUT_FIRST="$QEMU_OUTPUT"
QEMU_OUTPUT_SECOND=""
QEMU_OUTPUT_THIRD=""
QEMU_OUTPUT_DETACHED=""
QEMU_OUTPUT_RESTORED=""
ROOT_FIRST=""
ROOT_SECOND=""
ROOT_THIRD=""
ROOT_RESTORED=""
trap 'rm -f "$QEMU_OUTPUT_FIRST" "$QEMU_OUTPUT" "$BUILD_LOG" "${QEMU_OUTPUT_SECOND:-}" "${QEMU_OUTPUT_THIRD:-}" "${QEMU_OUTPUT_DETACHED:-}" "${QEMU_OUTPUT_RESTORED:-}" "${ROOT_FIRST:-}" "${ROOT_SECOND:-}" "${ROOT_THIRD:-}" "${ROOT_RESTORED:-}"' EXIT

# Extra QEMU flags for phases that need a virtio-blk disk
DISK_FLAGS=""
if [[ "$NEED_DISK" == "true" ]]; then
    if [[ "$PHASE" == wiseowl-* || "$PHASE" == "phase3.75" || "$PHASE" == "phase3.875" ]]; then
        DISK_FLAGS="-drive id=hd0,file=target/state-test.img,if=none,format=raw,cache=writeback -device virtio-blk-pci,disable-modern=on,drive=hd0"
    elif [[ -f "target/test.img" ]]; then
        DISK_FLAGS="-drive id=hd0,file=target/test.img,if=none,format=raw -device virtio-blk-pci,disable-modern=on,drive=hd0"
    fi
fi

# Extra QEMU flags for Phase 5 networking (virtio-net). Always add for phase5* so PCI scan + driver init succeed.
NET_FLAGS=""
if [[ "$PHASE" == phase5* || "$PHASE" == phase5x* ]]; then
    NET_FLAGS="-netdev user,id=net0 -device virtio-net-pci,netdev=net0,disable-modern=on"
fi

# Audio hardware is opt-in so existing gates do not depend on a host backend.
AUDIO_FLAGS=""
if [[ "$PHASE" == "audio" ]]; then
    AUDIO_FLAGS="-audiodev none,id=snd0 -device intel-hda -device hda-output,audiodev=snd0"
fi

set +e
QEMU_SMP="${SUNLIGHT_TEST_CPUS:-2}"
QEMU_MEMORY_MB="${SUNLIGHT_TEST_MEMORY_MB:-1024}"
if [[ "$PHASE" == "yazi-phase1" && -z "${SUNLIGHT_TEST_MEMORY_MB:-}" ]]; then
    QEMU_MEMORY_MB=4096
fi
if [[ "$PHASE" == "yazi-phase1" && -z "${SUNLIGHT_TEST_CPUS:-}" ]]; then
    QEMU_SMP=16
fi
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
    -m "${QEMU_MEMORY_MB}M" \
    -smp "$QEMU_SMP" \
    $KVM_FLAGS \
    -device virtio-rng-pci,disable-modern=on \
    -device qemu-xhci,id=xhci -device usb-mouse,bus=xhci.0 \
    $DISK_FLAGS \
    $NET_FLAGS \
    $AUDIO_FLAGS \
    -no-reboot \
    -no-shutdown >>"$BUILD_LOG" 2>&1 &
QEMU_PID=$!

# Linux workers share the executable name but have distinct TIDs. Only the
# spawned thread-group leader ending is a terminated Yazi application.
yazi_leader_finished() {
    local leader_pid
    leader_pid=$(sed -n 's/^\[SYSCALL\] spawn: \/bin\/yazi pid=\([0-9][0-9]*\) ppid=.*/\1/p' "$QEMU_OUTPUT" | head -n1)
    [[ -n "$leader_pid" ]] && grep -Fq "process_mark_finished pid=$leader_pid name='yazi'" "$QEMU_OUTPUT"
}

INITIAL_MARKER="$FINAL_MARKER"
if [[ "$PHASE" == "wiseowl-identity-phase-a" ]]; then
    INITIAL_MARKER="[WISEOWL-IDENTITY-A] injected crash after durable staged ROOT"
fi

# Wait up to TIMEOUT seconds, checking if QEMU is still running
for ((i=0; i<TIMEOUT; i++)); do
    if ! kill -0 $QEMU_PID 2>/dev/null; then
        break
    fi
    if [[ "$PHASE" == "yazi-phase1" ]] && yazi_leader_finished; then
        # An unexpected exit, including one after the key was read, fails the
        # interactive observation window.
        sleep 1
        break
    fi
    # Check if the final runtime milestone is present (early exit on success).
    if [[ "$PHASE" == "wiseowl-identity-phase-b" ]] \
        && { grep -Eq '\[WISEOWL-INDEX\] (persistent identity bound |identity status unavailable)' "$QEMU_OUTPUT" 2>/dev/null; } \
        && { grep -Eq '\[WISEOWL-BRAIN\] (persistent identity bound |identity status unavailable)' "$QEMU_OUTPUT" 2>/dev/null; } \
        && { grep -Eq '\[WISEOWL\] (persistent identity available |identity status unavailable)' "$QEMU_OUTPUT" 2>/dev/null; }; then
        sleep 1
        break
    fi
    if grep -Fq "$INITIAL_MARKER" "$QEMU_OUTPUT" 2>/dev/null \
        && { [[ "$PHASE" == "mm2b" ]] || grep -Fq "[timer] 100 ticks elapsed" "$QEMU_OUTPUT" 2>/dev/null; }; then
        if [[ "$PHASE" == "yazi-phase1" ]]; then
            # Keep observing after input for late worker and TTY failures.
            sleep 10
        fi
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

# Phase A keeps one state image across independent VM boots. Compare the full
# durable ROOT bytes after each boot, then boot without the volume and ensure
# the initramfs /state mount-point cannot produce a temporary identity.
if [[ "$PHASE" == "wiseowl-identity-phase-a" ]]; then
    mkdir -p target/wiseowl-identity-phase-a-evidence
    cp "$QEMU_OUTPUT" target/wiseowl-identity-phase-a-evidence/boot-1-serial.log
    printf '%s\n' \
        'Interruption marker: AfterRootFlush, after ROOT file and stage-directory sync.' \
        'Harness boundary: wait for marker, send SIGTERM to QEMU, wait one second, then SIGKILL only if still alive.' \
        'State-disk probe: mdir/mcopy run after the QEMU process exits.' \
        >target/wiseowl-identity-phase-a-evidence/boot-1-stop-boundary.txt
    if ! command -v mcopy >/dev/null 2>&1; then
        echo "[test] mcopy is required for the Phase A state-volume reboot check" >&2
        exit 1
    fi

    ROOT_FIRST=$(mktemp)
    ROOT_SECOND=$(mktemp)
    if command -v mdir >/dev/null 2>&1; then
        mdir -i target/state-test.img ::/WISEOWL-MEMORYDB/IDENTITY.STAGE \
            >target/wiseowl-identity-phase-a-evidence/boot-1-staged-directory.txt 2>&1 || true
    fi
    if ! grep -Fq "$INITIAL_MARKER" "$QEMU_OUTPUT"; then
        echo "[test] boot 1 missed the post-file-sync interruption marker" >&2
        cat "$QEMU_OUTPUT"
        exit 1
    fi
    if grep -Fq "[WISEOWL-DB] identity loaded:" "$QEMU_OUTPUT"; then
        echo "[test] boot 1 continued past the intended pre-publication interruption" >&2
        cat "$QEMU_OUTPUT"
        exit 1
    fi
    if ! mcopy -i target/state-test.img ::/WISEOWL-MEMORYDB/IDENTITY.STAGE/ROOT "$ROOT_FIRST" >/dev/null 2>&1; then
        echo "[test] boot 1 staged ROOT is not readable from the stopped state disk" >&2
        cat target/wiseowl-identity-phase-a-evidence/boot-1-staged-directory.txt 2>/dev/null || true
        cat "$QEMU_OUTPUT"
        exit 1
    fi
    FIRST_IDENTITY=$(od -An -j 12 -N4 -tx1 "$ROOT_FIRST" | tr -d ' \n' | tr '[:lower:]' '[:upper:]')
    echo "[test] boot 1 durably staged identity ROOT ($FIRST_IDENTITY), then stopped before publication"

    QEMU_OUTPUT_SECOND=$(mktemp)
    qemu-system-x86_64 \
        -cdrom "$ISO_PATH" \
        -serial file:"$QEMU_OUTPUT_SECOND" \
        -display none \
        -m 1024M \
        -smp "$QEMU_SMP" \
        $KVM_FLAGS \
        -device virtio-rng-pci,disable-modern=on \
        -device qemu-xhci,id=xhci -device usb-mouse,bus=xhci.0 \
        $DISK_FLAGS \
        -no-reboot \
        -no-shutdown >>"$BUILD_LOG" 2>&1 &
    QEMU_PID=$!
    for ((i=0; i<TIMEOUT; i++)); do
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then
            break
        fi
        if grep -Fq "$FINAL_MARKER" "$QEMU_OUTPUT_SECOND" 2>/dev/null; then
            sleep 1
            break
        fi
        sleep 1
    done
    if kill -0 "$QEMU_PID" 2>/dev/null; then
        kill -TERM "$QEMU_PID" 2>/dev/null || true
        sleep 1
        kill -KILL "$QEMU_PID" 2>/dev/null || true
    fi
    wait "$QEMU_PID" 2>/dev/null || true
    cp "$QEMU_OUTPUT_SECOND" target/wiseowl-identity-phase-a-evidence/boot-2-serial.log

    if ! grep -Fq "$FINAL_MARKER" "$QEMU_OUTPUT_SECOND" || \
        ! grep -Fq "identity creation recovered from staged state" "$QEMU_OUTPUT_SECOND" || \
        ! mcopy -i target/state-test.img ::/WISEOWL-MEMORYDB/IDENTITY/ROOT "$ROOT_SECOND" >/dev/null 2>&1; then
        echo "[test] boot 2 did not reload identity from the same state image" >&2
        cat "$QEMU_OUTPUT_SECOND"
        exit 1
    fi
    STATE_MOUNT_LINE=$(grep -nF '[VFS] FAT volume mounted at /state' "$QEMU_OUTPUT_SECOND" | head -n1 | cut -d: -f1)
    MEMORYDB_START_LINE=$(grep -nF '[WISEOWL-DB] starting wiseowl-memorydb' "$QEMU_OUTPUT_SECOND" | head -n1 | cut -d: -f1)
    if [[ -z "$STATE_MOUNT_LINE" || -z "$MEMORYDB_START_LINE" || "$STATE_MOUNT_LINE" -ge "$MEMORYDB_START_LINE" ]]; then
        echo "[test] persistent /state was not mounted before wiseowl-memorydb startup" >&2
        cat "$QEMU_OUTPUT_SECOND"
        exit 1
    fi
    SECOND_IDENTITY=$(sed -n 's/^.*identity loaded: \([0-9A-F]\{8\}\)$/\1/p' "$QEMU_OUTPUT_SECOND" | head -n1)
    if [[ "$FIRST_IDENTITY" != "$SECOND_IDENTITY" ]] || ! cmp -s "$ROOT_FIRST" "$ROOT_SECOND"; then
        echo "[test] staged identity ROOT changed during native recovery" >&2
        echo "boot 1 fingerprint: ${FIRST_IDENTITY:-missing}"
        echo "boot 2 fingerprint: ${SECOND_IDENTITY:-missing}"
        exit 1
    fi
    if ! grep -Fq "lineage sequence=1 continuity_generation=1 genesis=Created" "$QEMU_OUTPUT_SECOND"; then
        echo "[test] recovered identity did not report genesis lineage values" >&2
        cat "$QEMU_OUTPUT_SECOND"
        exit 1
    fi
    echo "[test] boot 2 recovered the staged identity and committed genesis ($SECOND_IDENTITY)"

    ROOT_THIRD=$(mktemp)
    QEMU_OUTPUT_THIRD=$(mktemp)
    qemu-system-x86_64 \
        -cdrom "$ISO_PATH" \
        -serial file:"$QEMU_OUTPUT_THIRD" \
        -display none \
        -m 1024M \
        -smp "$QEMU_SMP" \
        $KVM_FLAGS \
        -device virtio-rng-pci,disable-modern=on \
        -device qemu-xhci,id=xhci -device usb-mouse,bus=xhci.0 \
        $DISK_FLAGS \
        -no-reboot \
        -no-shutdown >>"$BUILD_LOG" 2>&1 &
    QEMU_PID=$!
    for ((i=0; i<TIMEOUT; i++)); do
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then
            break
        fi
        if grep -Fq "$FINAL_MARKER" "$QEMU_OUTPUT_THIRD" 2>/dev/null; then
            sleep 1
            break
        fi
        sleep 1
    done
    if kill -0 "$QEMU_PID" 2>/dev/null; then
        kill -TERM "$QEMU_PID" 2>/dev/null || true
        sleep 1
        kill -KILL "$QEMU_PID" 2>/dev/null || true
    fi
    wait "$QEMU_PID" 2>/dev/null || true
    cp "$QEMU_OUTPUT_THIRD" target/wiseowl-identity-phase-a-evidence/boot-3-serial.log
    if ! grep -Fq "$FINAL_MARKER" "$QEMU_OUTPUT_THIRD" || \
        ! mcopy -i target/state-test.img ::/WISEOWL-MEMORYDB/IDENTITY/ROOT "$ROOT_THIRD" >/dev/null 2>&1 || \
        ! cmp -s "$ROOT_SECOND" "$ROOT_THIRD"; then
        echo "[test] committed identity did not survive a second native reboot" >&2
        cat "$QEMU_OUTPUT_THIRD"
        exit 1
    fi
    THIRD_IDENTITY=$(sed -n 's/^.*identity loaded: \([0-9A-F]\{8\}\)$/\1/p' "$QEMU_OUTPUT_THIRD" | head -n1)
    if [[ "$THIRD_IDENTITY" != "$SECOND_IDENTITY" ]]; then
        echo "[test] identity fingerprint changed on the third boot" >&2
        exit 1
    fi
    echo "[test] committed identity survived another independent VM boot ($THIRD_IDENTITY)"
    QEMU_OUTPUT="$QEMU_OUTPUT_SECOND"

    QEMU_OUTPUT_DETACHED=$(mktemp)
    qemu-system-x86_64 \
        -cdrom "$ISO_PATH" \
        -serial file:"$QEMU_OUTPUT_DETACHED" \
        -display none \
        -m 1024M \
        -smp "$QEMU_SMP" \
        $KVM_FLAGS \
        -device virtio-rng-pci,disable-modern=on \
        -device qemu-xhci,id=xhci -device usb-mouse,bus=xhci.0 \
        -no-reboot \
        -no-shutdown >>"$BUILD_LOG" 2>&1 &
    QEMU_PID=$!
    DETACHED_MARKER="[WISEOWL-DB] persistent /state unavailable; identity startup suspended"
    for ((i=0; i<TIMEOUT; i++)); do
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then
            break
        fi
        if grep -Fq "$DETACHED_MARKER" "$QEMU_OUTPUT_DETACHED" 2>/dev/null; then
            sleep 1
            break
        fi
        sleep 1
    done
    if kill -0 "$QEMU_PID" 2>/dev/null; then
        kill -TERM "$QEMU_PID" 2>/dev/null || true
        sleep 1
        kill -KILL "$QEMU_PID" 2>/dev/null || true
    fi
    wait "$QEMU_PID" 2>/dev/null || true
    cp "$QEMU_OUTPUT_DETACHED" target/wiseowl-identity-phase-a-evidence/boot-without-state-serial.log
    if ! grep -Fq "$DETACHED_MARKER" "$QEMU_OUTPUT_DETACHED" || \
        grep -Fq "[WISEOWL-DB] identity loaded:" "$QEMU_OUTPUT_DETACHED" || \
        grep -Fq "[WISEOWL-DB] registered" "$QEMU_OUTPUT_DETACHED"; then
        echo "[test] detached state volume did not fail closed explicitly" >&2
        cat "$QEMU_OUTPUT_DETACHED"
        exit 1
    fi
    echo "[test] detached state volume did not create or publish a replacement identity"

    ROOT_RESTORED=$(mktemp)
    QEMU_OUTPUT_RESTORED=$(mktemp)
    qemu-system-x86_64 \
        -cdrom "$ISO_PATH" \
        -serial file:"$QEMU_OUTPUT_RESTORED" \
        -display none \
        -m 1024M \
        -smp "$QEMU_SMP" \
        $KVM_FLAGS \
        -device virtio-rng-pci,disable-modern=on \
        -device qemu-xhci,id=xhci -device usb-mouse,bus=xhci.0 \
        $DISK_FLAGS \
        -no-reboot \
        -no-shutdown >>"$BUILD_LOG" 2>&1 &
    QEMU_PID=$!
    for ((i=0; i<TIMEOUT; i++)); do
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then
            break
        fi
        if grep -Fq "$FINAL_MARKER" "$QEMU_OUTPUT_RESTORED" 2>/dev/null; then
            sleep 1
            break
        fi
        sleep 1
    done
    if kill -0 "$QEMU_PID" 2>/dev/null; then
        kill -TERM "$QEMU_PID" 2>/dev/null || true
        sleep 1
        kill -KILL "$QEMU_PID" 2>/dev/null || true
    fi
    wait "$QEMU_PID" 2>/dev/null || true
    cp "$QEMU_OUTPUT_RESTORED" target/wiseowl-identity-phase-a-evidence/boot-with-state-restored-serial.log
    if ! grep -Fq "$FINAL_MARKER" "$QEMU_OUTPUT_RESTORED" || \
        ! mcopy -i target/state-test.img ::/WISEOWL-MEMORYDB/IDENTITY/ROOT "$ROOT_RESTORED" >/dev/null 2>&1 || \
        ! cmp -s "$ROOT_SECOND" "$ROOT_RESTORED"; then
        echo "[test] original state volume did not restore the unchanged committed identity" >&2
        cat "$QEMU_OUTPUT_RESTORED"
        exit 1
    fi
    RESTORED_IDENTITY=$(sed -n 's/^.*identity loaded: \([0-9A-F]\{8\}\)$/\1/p' "$QEMU_OUTPUT_RESTORED" | head -n1)
    if [[ "$RESTORED_IDENTITY" != "$SECOND_IDENTITY" ]] || \
        ! grep -Fq "lineage sequence=1 continuity_generation=1" "$QEMU_OUTPUT_RESTORED"; then
        echo "[test] original state volume returned with changed identity or continuity" >&2
        cat "$QEMU_OUTPUT_RESTORED"
        exit 1
    fi
    echo "[test] original state volume restored unchanged identity ($RESTORED_IDENTITY)"
    printf '%s\n' "$FIRST_IDENTITY" >target/wiseowl-identity-phase-a-evidence/identity-fingerprint.txt
fi

if [[ "$PHASE" == "wiseowl-identity-phase-b" || "$PHASE" == "wiseowl-identity-phase-c" ]]; then
    IDENTITY_EVIDENCE_DIR="target/$PHASE-evidence"
    mkdir -p "$IDENTITY_EVIDENCE_DIR"
    cp "$QEMU_OUTPUT" "$IDENTITY_EVIDENCE_DIR/boot-1-serial.log"
    DB_STATUS=$(sed -n 's/^.*\[WISEOWL-DB\] identity status Ready fingerprint=\([0-9A-F]\{8\}\).*$/\1/p' "$QEMU_OUTPUT" | head -n1)
    MOUNT_LINE=$(grep -nF '[VFS] FAT volume mounted at /state' "$QEMU_OUTPUT" | head -n1 | cut -d: -f1)
    DB_START_LINE=$(grep -nF '[WISEOWL-DB] starting wiseowl-memorydb' "$QEMU_OUTPUT" | head -n1 | cut -d: -f1)
    DB_LOAD_LINE=$(grep -nF '[WISEOWL-DB] identity loaded:' "$QEMU_OUTPUT" | head -n1 | cut -d: -f1)
    DB_READY_LINE=$(grep -nF '[WISEOWL-DB] identity status Ready fingerprint=' "$QEMU_OUTPUT" | head -n1 | cut -d: -f1)
    PHASE_A_STATUS=""
    if [[ "$PHASE" == "wiseowl-identity-phase-b" \
        && -s target/wiseowl-identity-phase-a-evidence/identity-fingerprint.txt ]]; then
        PHASE_A_STATUS=$(cat target/wiseowl-identity-phase-a-evidence/identity-fingerprint.txt)
    fi
    if [[ -z "$DB_STATUS" || -z "$MOUNT_LINE" || -z "$DB_START_LINE" || -z "$DB_LOAD_LINE" || -z "$DB_READY_LINE" ]] \
        || (( MOUNT_LINE >= DB_START_LINE || DB_START_LINE >= DB_LOAD_LINE || DB_LOAD_LINE >= DB_READY_LINE )) \
        || { [[ -n "$PHASE_A_STATUS" ]] && [[ "$DB_STATUS" != "$PHASE_A_STATUS" ]]; } \
        || ! grep -Fq "[WISEOWL-INDEX] persistent identity bound $DB_STATUS" "$QEMU_OUTPUT" \
        || ! grep -Fq "[WISEOWL-BRAIN] persistent identity bound $DB_STATUS" "$QEMU_OUTPUT" \
        || ! grep -Fq "[WISEOWL] persistent identity available $DB_STATUS" "$QEMU_OUTPUT" \
        || ! grep -Fq 'continuity_generation=1' "$QEMU_OUTPUT"; then
        echo '[test] native Phase B identity propagation or startup order failed' >&2
        cat "$QEMU_OUTPUT"
        exit 1
    fi
    for consumer in '[WISEOWL-INDEX] persistent identity bound' '[WISEOWL-BRAIN] persistent identity bound' '[WISEOWL] persistent identity available'; do
        CONSUMER_LINE=$(grep -nF "$consumer" "$QEMU_OUTPUT" | head -n1 | cut -d: -f1)
        if [[ -z "$CONSUMER_LINE" ]] || (( CONSUMER_LINE <= DB_READY_LINE )); then
            echo "[test] consumer bound before MemoryDB status readiness: $consumer" >&2
            cat "$QEMU_OUTPUT"
            exit 1
        fi
    done
    if [[ -n "$PHASE_A_STATUS" ]]; then
        printf 'state_image=target/state-test.img\nphase_a_fingerprint=%s\nphase_b_fingerprint=%s\n' \
            "$PHASE_A_STATUS" "$DB_STATUS" \
        >"$IDENTITY_EVIDENCE_DIR/state-volume-reuse.txt"
    else
        printf '%s\n' 'state_image=target/state-test.img' 'source=standalone Phase B run' \
            >"$IDENTITY_EVIDENCE_DIR/state-volume-reuse.txt"
    fi
    if [[ "$PHASE" == "wiseowl-identity-phase-c" ]]; then
        ACCEPTED_COUNT=$(grep -Fc '[WISEOWL-IDENTITY-C] writer accepted' "$QEMU_OUTPUT" || true)
        REJECTED_COUNT=$(grep -Fc '[WISEOWL-IDENTITY-C] writer rejected' "$QEMU_OUTPUT" || true)
        ACTIVE_COUNT=$(grep -Fc '[WISEOWL-ACTIVATION] state=Active' "$QEMU_OUTPUT" || true)
        ACCEPTED_GENERATIONS=$(sed -n 's/^.*\[WISEOWL-IDENTITY-C\] writer accepted pid=[0-9]* generation=\([0-9]*\).*$/\1/p' "$QEMU_OUTPUT" | sort -u | wc -l)
        FIRST_ACTIVATION=$(sed -n 's/^.*\[WISEOWL-ACTIVATION\] state=Activating activation=\([0-9A-F]\{8\}\).*$/\1/p' "$QEMU_OUTPUT" | head -n1)
        RESTART_ACTIVATION=$(sed -n 's/^.*\[WISEOWL-ACTIVATION\] state=RecoveringLocal activation=\([0-9A-F]\{8\}\).*$/\1/p' "$QEMU_OUTPUT" | head -n1)
        BOOT_EPOCH_COUNT=$(sed -n 's/^.*\[WISEOWL-IDENTITY-C\] boot epoch=\([0-9A-F]\{8\}\).*$/\1/p' "$QEMU_OUTPUT" | sort -u | wc -l)
        INSTALL_FP_COUNT=$(sed -n 's/^.*\[WISEOWL-IDENTITY-C\] installation=\([0-9A-F]\{8\}\).*$/\1/p' "$QEMU_OUTPUT" | sort -u | wc -l)
        if [[ "$ACCEPTED_COUNT" != 2 || "$ACCEPTED_GENERATIONS" != 2 || "$REJECTED_COUNT" != 1 || "$ACTIVE_COUNT" != 2 \
            || "$BOOT_EPOCH_COUNT" != 1 || "$INSTALL_FP_COUNT" != 1 || -z "$FIRST_ACTIVATION" \
            || "$FIRST_ACTIVATION" != "$RESTART_ACTIVATION" ]] \
            || ! grep -Fq '[WISEOWL-IDENTITY-C] duplicate stopped' "$QEMU_OUTPUT" \
            || ! grep -Fq '[WISEOWL-IDENTITY-C] MemoryDB-only restart old_pid=' "$QEMU_OUTPUT" \
            || ! grep -Fq '[WISEOWL-IDENTITY-C] consumers initially active' "$QEMU_OUTPUT" \
            || ! grep -Fq '[WISEOWL-IDENTITY-C] consumers paused while MemoryDB absent' "$QEMU_OUTPUT" \
            || ! grep -Fq '[WISEOWL-IDENTITY-C] brain probe mode=GenericDegraded' "$QEMU_OUTPUT" \
            || ! grep -Fq '[WISEOWL-IDENTITY-C] memory probe durable=0 ram=1' "$QEMU_OUTPUT" \
            || ! grep -Fq '[WISEOWL-IDENTITY-C] memory RAM session available while durable blocked' "$QEMU_OUTPUT" \
            || ! grep -Fq '[WISEOWL-IDENTITY-C] replacement ready' "$QEMU_OUTPUT"; then
            echo '[test] native Phase C duplicate/restart gate failed' >&2
            cat "$QEMU_OUTPUT"
            exit 1
        fi
        printf 'accepted_writers=%s\nrejected_writers=%s\nsame_boot_activation=%s\n' \
            "$ACCEPTED_COUNT" "$REJECTED_COUNT" "$FIRST_ACTIVATION" \
            >"$IDENTITY_EVIDENCE_DIR/writer-restart-comparison.txt"
        rg '\[WISEOWL-IDENTITY-C\] (writer accepted|writer rejected|installation=|boot epoch=|index health|brain probe|memory probe|memory RAM|consumers|replacement ready)' \
            "$QEMU_OUTPUT" >"$IDENTITY_EVIDENCE_DIR/process-endpoint-transitions.txt"
    fi
    echo '[WISEOWL-IDENTITY-B] native gate PASS' >>"$QEMU_OUTPUT"

    # Phase C native reboot gate: the first QEMU was stopped after Active, so
    # this is an unclean system stop. The next boot must keep Identity and
    # Installation, recover locally, and publish a different ActivationId.
    LOCAL_FIRST=$(mktemp)
    LOCAL_SECOND=$(mktemp)
    ROOT_FIRST_C=$(mktemp)
    ROOT_SECOND_C=$(mktemp)
    LINEAGE_FIRST_C=$(mktemp)
    LINEAGE_SECOND_C=$(mktemp)
    HEAD_FIRST_C=$(mktemp)
    HEAD_SECOND_C=$(mktemp)
    mcopy -i target/state-test.img ::/WISEOWL-MEMORYDB/IDENTITY/LOCAL "$LOCAL_FIRST" >/dev/null 2>&1
    mcopy -i target/state-test.img ::/WISEOWL-MEMORYDB/IDENTITY/ROOT "$ROOT_FIRST_C" >/dev/null 2>&1
    mcopy -i target/state-test.img ::/WISEOWL-MEMORYDB/IDENTITY/LINEAGE "$LINEAGE_FIRST_C" >/dev/null 2>&1
    mcopy -i target/state-test.img ::/WISEOWL-MEMORYDB/IDENTITY/HEAD "$HEAD_FIRST_C" >/dev/null 2>&1
    QEMU_OUTPUT_C=$(mktemp)
    qemu-system-x86_64 \
        -cdrom "$ISO_PATH" \
        -serial file:"$QEMU_OUTPUT_C" \
        -display none \
        -m 1024M \
        -smp "$QEMU_SMP" \
        $KVM_FLAGS \
        -device virtio-rng-pci,disable-modern=on \
        -device qemu-xhci,id=xhci -device usb-mouse,bus=xhci.0 \
        $DISK_FLAGS \
        -no-reboot \
        -no-shutdown >>"$BUILD_LOG" 2>&1 &
    QEMU_PID=$!
    for ((i=0; i<TIMEOUT; i++)); do
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then break; fi
        if grep -Fq '[WISEOWL-ACTIVATION] state=Active' "$QEMU_OUTPUT_C" 2>/dev/null \
            && grep -Fq '[WISEOWL-DB] identity status Ready fingerprint=' "$QEMU_OUTPUT_C" 2>/dev/null \
            && grep -Fq '[WISEOWL-INDEX] persistent identity bound' "$QEMU_OUTPUT_C" 2>/dev/null \
            && grep -Fq '[WISEOWL-BRAIN] persistent identity bound' "$QEMU_OUTPUT_C" 2>/dev/null \
            && grep -Fq '[WISEOWL] persistent identity available' "$QEMU_OUTPUT_C" 2>/dev/null; then
            sleep 1
            break
        fi
        sleep 1
    done
    if kill -0 "$QEMU_PID" 2>/dev/null; then
        kill -TERM "$QEMU_PID" 2>/dev/null || true
        sleep 1
        kill -KILL "$QEMU_PID" 2>/dev/null || true
    fi
    wait "$QEMU_PID" 2>/dev/null || true
    cp "$QEMU_OUTPUT_C" "$IDENTITY_EVIDENCE_DIR/boot-2-unclean-recovery-serial.log"
    if ! grep -Fq '[WISEOWL-ACTIVATION] state=RecoveringLocal' "$QEMU_OUTPUT_C" \
        || ! grep -Fq '[WISEOWL-ACTIVATION] state=Active' "$QEMU_OUTPUT_C" \
        || ! grep -Fq 'continuity_generation=1' "$QEMU_OUTPUT_C"; then
        echo '[test] unclean reboot did not recover local activation' >&2
        cp "$BUILD_LOG" "$IDENTITY_EVIDENCE_DIR/build-output-failure.log"
        tail -n 30 "$BUILD_LOG" >&2
        cat "$QEMU_OUTPUT_C"
        exit 1
    fi
    mcopy -i target/state-test.img ::/WISEOWL-MEMORYDB/IDENTITY/LOCAL "$LOCAL_SECOND" >/dev/null 2>&1
    mcopy -i target/state-test.img ::/WISEOWL-MEMORYDB/IDENTITY/ROOT "$ROOT_SECOND_C" >/dev/null 2>&1
    mcopy -i target/state-test.img ::/WISEOWL-MEMORYDB/IDENTITY/LINEAGE "$LINEAGE_SECOND_C" >/dev/null 2>&1
    mcopy -i target/state-test.img ::/WISEOWL-MEMORYDB/IDENTITY/HEAD "$HEAD_SECOND_C" >/dev/null 2>&1
    INSTALL_FIRST=$(od -An -tx1 -j 40 -N 16 "$LOCAL_FIRST" | tr -d ' \n')
    INSTALL_SECOND=$(od -An -tx1 -j 40 -N 16 "$LOCAL_SECOND" | tr -d ' \n')
    ACTIVATION_FIRST=$(od -An -tx1 -j 56 -N 16 "$LOCAL_FIRST" | tr -d ' \n')
    ACTIVATION_SECOND=$(od -An -tx1 -j 56 -N 16 "$LOCAL_SECOND" | tr -d ' \n')
    BOOT_EPOCH_FIRST=$(od -An -tx1 -j 80 -N 16 "$LOCAL_FIRST" | tr -d ' \n')
    BOOT_EPOCH_SECOND=$(od -An -tx1 -j 80 -N 16 "$LOCAL_SECOND" | tr -d ' \n')
    if [[ "$INSTALL_FIRST" != "$INSTALL_SECOND" || "$ACTIVATION_FIRST" == "$ACTIVATION_SECOND" ]] \
        || [[ "$BOOT_EPOCH_FIRST" == "$BOOT_EPOCH_SECOND" ]] \
        || ! cmp -s "$ROOT_FIRST_C" "$ROOT_SECOND_C" \
        || ! cmp -s "$LINEAGE_FIRST_C" "$LINEAGE_SECOND_C" \
        || ! cmp -s "$HEAD_FIRST_C" "$HEAD_SECOND_C"; then
        echo '[test] reboot changed identity/installation or retained activation/changed lineage' >&2
        exit 1
    fi
    cp "$LOCAL_FIRST" "$IDENTITY_EVIDENCE_DIR/boot-1-LOCAL.bin"
    cp "$LOCAL_SECOND" "$IDENTITY_EVIDENCE_DIR/boot-2-LOCAL.bin"
    printf 'ROOT_sha256=%s\nLINEAGE_sha256=%s\nHEAD_sha256=%s\nstate_volume_sha256=%s\n' \
        "$(sha256sum "$ROOT_SECOND_C" | cut -d' ' -f1)" \
        "$(sha256sum "$LINEAGE_SECOND_C" | cut -d' ' -f1)" \
        "$(sha256sum "$HEAD_SECOND_C" | cut -d' ' -f1)" \
        "$(sha256sum target/state-test.img | cut -d' ' -f1)" \
        >"$IDENTITY_EVIDENCE_DIR/global-state-hashes.txt"
    printf 'installation_id=%s\nboot_1_activation_id=%s\nboot_2_activation_id=%s\nboot_1_epoch=%s\nboot_2_epoch=%s\n' \
        "$INSTALL_SECOND" "$ACTIVATION_FIRST" "$ACTIVATION_SECOND" "$BOOT_EPOCH_FIRST" "$BOOT_EPOCH_SECOND" \
        >"$IDENTITY_EVIDENCE_DIR/activation-comparison.txt"
    echo "[test] unclean reboot recovered same installation with a new activation ($INSTALL_SECOND)"
fi

# Preserve the raw serial evidence when requested, before the EXIT trap removes it.
# Example: SUNLIGHT_TEST_SERIAL_LOG=target/ipc-serial.log ./tools/test.sh phase2.6
if [[ -n "${SUNLIGHT_TEST_SERIAL_LOG:-}" ]]; then
    cp "$QEMU_OUTPUT" "$SUNLIGHT_TEST_SERIAL_LOG"
fi

if [[ "$PHASE" == "phase3.75" ]]; then
    cp "$QEMU_OUTPUT" target/wiseowl-phase375-serial.log
fi
if [[ "$PHASE" == "phase3.875" ]]; then
    cp "$QEMU_OUTPUT" target/wiseowl-phase3875-serial.log
fi
if [[ "$PHASE" == "wiseowl-graphical-console-v1" ]]; then
    cp "$QEMU_OUTPUT" target/wiseowl-graphical-console-v1-serial.log
fi
if [[ "$PHASE" == "wiseowl-gui-conversation-v1" ]]; then
    cp "$QEMU_OUTPUT" target/wiseowl-gui-conversation-v1-serial.log
fi
if [[ "$PHASE" == "wiseowl-gui-bridge-foundation-v1" ]]; then
    cp "$QEMU_OUTPUT" target/wiseowl-gui-bridge-foundation-v1-serial.log
fi
if [[ "$PHASE" == "wiseowl-trusted-session-readiness-v1" ]]; then
    cp "$QEMU_OUTPUT" target/wiseowl-trusted-session-readiness-v1-serial.log
fi
if [[ "$PHASE" == "phase3.0" ]]; then
    cp "$QEMU_OUTPUT" target/timekeeping-phase3-serial.log
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

# A launch marker alone is not a Yazi runtime smoke pass: Tokio can panic
# immediately after exec when a required Linux primitive is unavailable.
if [[ "$PHASE" == "yazi-phase1" ]] && \
    { yazi_leader_finished \
      || grep -Eq "Failed building the Runtime|OS can't spawn worker thread|thread 'main' .* panicked|\[SYSCALL\] mmap failed .*error=NoMemory|process_mark_finished pid=[0-9]+ name='yazi'.*code=([1-9][0-9]*)" "$QEMU_OUTPUT"; }; then
    ALL_FOUND=false
fi
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
