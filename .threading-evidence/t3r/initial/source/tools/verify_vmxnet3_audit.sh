#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

PASS=0
FAIL=0

pass()  { echo -e "${GREEN}[PASS]${NC} $1"; PASS=$((PASS+1)); }
fail()  { echo -e "${RED}[FAIL]${NC} $1"; FAIL=$((FAIL+1)); }
warn()  { echo -e "${YELLOW}[WARN]${NC} $1"; }

KERNEL_ELF="target/x86_64-unknown-none/debug/sunlight-kernel"
ISO_PATH="target/sunlightos.iso"
EXPECTED_MARKER="SUNLIGHT_VMXNET3_BUILD_20260711-AUDIT"
EXPECTED_STAGE_MARKER="target/x86_64-unknown-none/debug"

echo "=== VMXNET3 Audit Artifact Verification ==="
echo ""

# 1. Kernel binary exists
if [ -f "$KERNEL_ELF" ]; then
    pass "Kernel binary exists: $KERNEL_ELF"
    SIZE=$(stat -c%s "$KERNEL_ELF" 2>/dev/null || stat -f%z "$KERNEL_ELF" 2>/dev/null || echo "?")
    echo "       size=$SIZE bytes"
else
    fail "Kernel binary MISSING: $KERNEL_ELF"
fi

# 2. VMXNET3 marker present in kernel binary
MARKER_FOUND=0
if command -v strings &>/dev/null; then
    if strings "$KERNEL_ELF" | grep -qF "$EXPECTED_MARKER"; then
        MARKER_FOUND=1
    fi
fi
# Fallback: use nm to find the marker symbol
if [ "$MARKER_FOUND" -eq 0 ] && command -v nm &>/dev/null; then
    if nm "$KERNEL_ELF" 2>/dev/null | grep -q "SUNLIGHT_VMXNET3_BUILD_MARKER"; then
        MARKER_FOUND=1
    fi
fi
# Second fallback: use readelf
if [ "$MARKER_FOUND" -eq 0 ] && command -v readelf &>/dev/null; then
    if readelf -p .rodata "$KERNEL_ELF" 2>/dev/null | grep -qF "$EXPECTED_MARKER"; then
        MARKER_FOUND=1
    fi
fi
if [ "$MARKER_FOUND" -eq 1 ]; then
    pass "VMXNET3 marker found in kernel binary: $EXPECTED_MARKER"
else
    fail "VMXNET3 marker NOT found in kernel binary: $EXPECTED_MARKER"
fi

# 3. nm symbol check for probe marker
if command -v nm &>/dev/null; then
    if nm "$KERNEL_ELF" | grep -q "sunlight_vmxnet3_probe_marker"; then
        pass "sunlight_vmxnet3_probe_marker symbol found in kernel binary"
    else
        fail "sunlight_vmxnet3_probe_marker symbol NOT found in kernel binary"
    fi
elif command -v llvm-nm &>/dev/null; then
    if llvm-nm "$KERNEL_ELF" | grep -q "sunlight_vmxnet3_probe_marker"; then
        pass "sunlight_vmxnet3_probe_marker symbol found in kernel binary"
    else
        fail "sunlight_vmxnet3_probe_marker symbol NOT found in kernel binary"
    fi
else
    warn "Neither 'nm' nor 'llvm-nm' available; skipping symbol check"
fi

# 4. ISO exists
if [ -f "$ISO_PATH" ]; then
    pass "ISO exists: $ISO_PATH"
else
    fail "ISO MISSING: $ISO_PATH"
fi

# 5. Freshness: kernel ELF vs ISO (ISO must be newer or equal)
if [ -f "$KERNEL_ELF" ] && [ -f "$ISO_PATH" ]; then
    KERNEL_TS=$(stat -c%Y "$KERNEL_ELF" 2>/dev/null || stat -f%m "$KERNEL_ELF" 2>/dev/null || echo "0")
    ISO_TS=$(stat -c%Y "$ISO_PATH" 2>/dev/null || stat -f%m "$ISO_PATH" 2>/dev/null || echo "0")
    if [ "$ISO_TS" -ge "$KERNEL_TS" ]; then
        pass "ISO is not older than kernel (ISO: $ISO_TS, kernel: $KERNEL_TS)"
    else
        fail "ISO is OLDER than kernel (ISO: $ISO_TS, kernel: $KERNEL_TS) — image may contain stale kernel"
    fi
fi

# 6. Build log marker check
if [ -d "$EXPECTED_STAGE_MARKER" ]; then
    LIB_FILE="$EXPECTED_STAGE_MARKER/libsunlight_net*.rlib"
    if ls $LIB_FILE 2>/dev/null | head -1 >/dev/null; then
        LIB=$(ls $LIB_FILE 2>/dev/null | head -1)
        if command -v strings &>/dev/null; then
            if strings "$LIB" | grep -qF "$EXPECTED_MARKER" 2>/dev/null; then
                pass "VMXNET3 marker found in sunlight-net rlib"
            else
                warn "VMXNET3 marker not found in sunlight-net rlib (may be LTO-stripped)"
            fi
        fi
    else
        warn "sunlight-net rlib not found (may use different profile path)"
    fi
fi

echo ""
echo "=== Summary ==="
echo -e "Passed: ${GREEN}$PASS${NC}"
if [ "$FAIL" -gt 0 ]; then
    echo -e "Failed: ${RED}$FAIL${NC}"
fi

if [ "$FAIL" -gt 0 ]; then
    echo ""
    echo -e "${RED}VMXNET3 AUDIT VERIFICATION FAILED${NC}"
    exit 1
else
    echo -e "${GREEN}VMXNET3 AUDIT VERIFICATION PASSED${NC}"
    exit 0
fi
