#!/bin/bash
# Test script to stress-test SunlightOS with many commands
# Run with increased memory (1GB) to test if freeze is memory-related

set -e

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ISO_PATH="$PROJECT_ROOT/target/sunlightos.iso"
OUTPUT_LOG="$PROJECT_ROOT/stress_test_output.log"

if [ ! -f "$ISO_PATH" ]; then
    echo "Error: ISO not found at $ISO_PATH"
    exit 1
fi

echo "=== SunlightOS Memory Stress Test ==="
echo "Memory: 1024 MiB (4x default)"
echo "Test: Running 20+ commands to trigger freeze"
echo "Output: $OUTPUT_LOG"
echo ""

# Create a FIFO for input
mkfifo /tmp/qemu_stress_input || true

# Start QEMU and capture output to both file and stdout
(
    # Wait for boot to login screen
    sleep 5

    # Run a sequence of commands
    for i in {1..25}; do
        echo "whoami  # Iteration $i/25"
        sleep 0.5
        echo "id  # Check groups"
        sleep 0.5
        echo "pwd  # Print working directory"
        sleep 0.5
    done

    # Try a few more diagnostic commands
    echo "uptime  # Check system uptime"
    sleep 1

    sleep 2
) > /tmp/qemu_stress_input &

QEMU_INPUT_PID=$!

timeout 60 qemu-system-x86_64 \
    -cdrom "$ISO_PATH" \
    -serial stdio \
    -display none \
    -m 1024M \
    -smp 2 \
    -no-reboot \
    -no-shutdown \
    < /tmp/qemu_stress_input 2>&1 | tee "$OUTPUT_LOG"

QEMU_EXIT=$?

wait $QEMU_INPUT_PID 2>/dev/null || true
rm /tmp/qemu_stress_input 2>/dev/null || true

echo ""
echo "=== Test Complete ==="
echo "Exit code: $QEMU_EXIT"
echo "Output saved to: $OUTPUT_LOG"
echo ""

# Count successful commands
WHOAMI_COUNT=$(grep -c "^root$" "$OUTPUT_LOG" 2>/dev/null || echo 0)
echo "Commands that completed: ~$WHOAMI_COUNT 'whoami' calls"
echo ""

# Check for diagnostic messages
echo "=== Memory Diagnostics ==="
grep "\[PMM-DIAG\]" "$OUTPUT_LOG" | tail -5 || echo "No PMM diagnostics found"
echo ""
echo "=== Scheduler Diagnostics ==="
grep "\[SCHED-DIAG\]" "$OUTPUT_LOG" | tail -5 || echo "No SCHED diagnostics found"
echo ""

# Look for the freeze point
echo "=== Last 20 lines before freeze ==="
tail -20 "$OUTPUT_LOG"
