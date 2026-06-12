#!/bin/bash
# Test script to capture diagnostic output showing the process leak

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ISO_PATH="$PROJECT_ROOT/target/sunlightos.iso"
OUTPUT_LOG="$PROJECT_ROOT/leak_diagnostic_output.log"

if [ ! -f "$ISO_PATH" ]; then
    echo "Error: ISO not found at $ISO_PATH"
    exit 1
fi

echo "Starting SunlightOS with diagnostic output capture..."
echo "Commands to send: whoami, id, whoami, id, whoami"
echo "Expected output in: $OUTPUT_LOG"
echo ""

# Create a FIFO for input
mkfifo /tmp/qemu_input || true

# Start QEMU and capture output to both file and stdout
(
    # Send commands with delays
    sleep 3  # Wait for boot
    echo "whoami"
    sleep 1
    echo "id"
    sleep 1
    echo "whoami"
    sleep 1
    echo "id"
    sleep 1
    echo "whoami"
    sleep 2
) > /tmp/qemu_input &

QEMU_INPUT_PID=$!

timeout 20 qemu-system-x86_64 \
    -cdrom "$ISO_PATH" \
    -serial stdio \
    -display none \
    -m 256M \
    -smp 2 \
    -no-reboot \
    -no-shutdown \
    < /tmp/qemu_input 2>&1 | tee "$OUTPUT_LOG"

QEMU_EXIT=$?

wait $QEMU_INPUT_PID 2>/dev/null || true
rm /tmp/qemu_input 2>/dev/null || true

echo ""
echo "Test complete. Exit code: $QEMU_EXIT"
echo "Output saved to: $OUTPUT_LOG"
echo ""

# Extract and highlight diagnostic lines
echo "=== DIAGNOSTIC SUMMARY ==="
echo ""
echo "Process Lifecycle Events:"
grep "\[SCHED\] CREATED\|\[SCHED\] FINISHED" "$OUTPUT_LOG" | head -20 || true
echo ""
echo "Scheduler Diagnostics (every 1000 ticks):"
grep "\[SCHED-DIAG\]" "$OUTPUT_LOG" | head -10 || true
echo ""
echo "Memory Diagnostics:"
grep "\[PMM-DIAG\]" "$OUTPUT_LOG" | head -10 || true
echo ""
echo "Interrupt EOI Verification (sample):"
grep "\[IRQ0\]\|\[IRQ1\]" "$OUTPUT_LOG" | head -5 || true
echo ""
echo "=== END DIAGNOSTIC SUMMARY ==="
