# SunlightOS QEMU Runner

The canonical host-side launcher is `tools/runs.sh`. `tools/run.sh` remains as
a compatibility shim that forwards to it.

## Available Entry Points

### 1. `tools/runs.sh` — Full-Featured Runner ⭐
Advanced runner with all options and help menu.

```bash
# Basic usage
./tools/runs.sh                 # GTK window (default)
./tools/runs.sh --help          # Show all options

# Display options
./tools/runs.sh --sdl           # SDL window
./tools/runs.sh --vnc           # VNC server on port 5900
./tools/runs.sh --curses        # Text-mode display
./tools/runs.sh --no-display    # Serial only
./tools/runs.sh --dual-gpu      # VGA std + explicit virtio-gpu-pci
./tools/runs.sh --resolution 1366x768

# Other options
./tools/runs.sh --memory 512    # Custom RAM size
./tools/runs.sh --gdb           # Wait for GDB on port 1234
./tools/runs.sh --screenshot    # Capture screenshot and exit
```

**Features:**
- Multiple display backends (GTK, SDL, VNC, curses)
- `--display MODE` switch plus convenience flags
- Old hardware-cursor runner mode via `--dual-gpu`
- VM-friendly default resolution policy for QEMU:
  `1366x768 -> 1280x800 -> 1280x720 -> 1024x768`
- Explicit QEMU resolution override via `--resolution WxH` or
  `SUNLIGHT_QEMU_RESOLUTION=WxH`
- Safety guardrails: resolution requests are ignored on non-QEMU hypervisors
  and when the selected QEMU video device lacks explicit `xres` / `yres`
- Custom memory configuration
- GDB debugging support
- Screenshot capture
- Comprehensive help menu

### 2. `tools/run.sh` — Compatibility Wrapper

```bash
./tools/run.sh --vnc
```

This simply forwards to `tools/runs.sh` so older commands still work.

## Recommended Usage

### For Development (with display)
```bash
./tools/runs.sh --sdl
```

The QEMU launcher now requests `1366x768` by default when the selected VirtIO
video device supports explicit `xres` / `yres` properties. That gives the
desktop, Start Menu, and apps more usable space without changing physical
hardware behavior.

### For Headless/SSH Environments
```bash
./tools/runs.sh --vnc
# Then connect with VNC viewer
```

### For Quick Tests
```bash
./tools/runs.sh --no-display
# Serial output shows all TUI log messages
```

### For Debugging
```bash
# Terminal 1: Start kernel with GDB wait
./tools/runs.sh --gdb --no-display

# Terminal 2: Connect GDB
gdb target/x86_64-unknown-none/debug/sunlight-kernel
(gdb) target remote :1234
(gdb) continue
```

## What You'll See

When running with a graphical display, the TUI shows:

```
┌─────────────────────────────────────────────────────────────┐
│  ☀  SunlightOS                v0.1.0 | Phase 2 | DEBUG     │  ← HEADER
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌── Boot Log ───────────────────────────────────────────┐ │
│  │ [PMM] Initializing...                                  │ │
│  │ [PMM] 236/251 MiB free                                 │ │
│  │ [PMM] OK                                               │ │  ← MAIN ZONE
│  │ [VMM] OK                                               │ │
│  │ [IDT] OK                                               │ │
│  │ [HEAP] OK                                              │ │
│  │ ... (scrolling log)                                    │ │
│  └────────────────────────────────────────────────────────┘ │
│                                                             │
│  Status: Initializing memory...                      ⟳     │
│  ▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░  60%                              │
│                                                             │
├─────────────────────────────────────────────────────────────┤
│  Status: OK              CPU: x86_64    RAM: 251 MiB      │  ← FOOTER
└─────────────────────────────────────────────────────────────┘
```

**Color coding:**
- 🟢 Green: OK messages
- 🔴 Red: Errors/panics
- 🟡 Yellow: Warnings
- ⚪ White: Info

## Build and Run Workflow

```bash
# 1. Build the kernel
./tools/build.sh

# 2. Run with TUI
./tools/runs.sh
```

## Documentation

See `docs/README_TUI.md` for detailed TUI documentation including:
- Feature descriptions
- Silent mode setup
- Technical implementation details
- Troubleshooting guide

## Common Issues

### Display fails in SSH/headless
Use VNC:
```bash
./tools/runs.sh --vnc
```

### "gtk initialization failed"
Your environment doesn't have X11/Wayland. Try:
```bash
./tools/runs.sh --sdl
# or
./tools/runs.sh --vnc
```

### Want to verify TUI works without display
```bash
./tools/runs.sh --no-display
```
All TUI log messages appear in serial output, so you can verify the TUI is updating correctly.

## Next Steps

After verifying the TUI works:
1. To enable silent mode, see `docs/README_TUI.md`
2. To customize colors, edit `sunlight-tui/src/layout.rs`
3. To modify layout, edit `sunlight-tui/src/splash.rs`
