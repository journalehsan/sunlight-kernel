# SunlightOS QEMU Runner Scripts

Created multiple scripts in `tools/` to run SunlightOS with the graphical TUI in various display modes.

## Available Scripts

### 1. `tools/run.sh` — Full-Featured Runner ⭐
Advanced runner with all options and help menu.

```bash
# Basic usage
./tools/run.sh                  # GTK window (default)
./tools/run.sh --help           # Show all options

# Display options
./tools/run.sh --sdl            # SDL window
./tools/run.sh --vnc            # VNC server on port 5900
./tools/run.sh --curses         # Text-mode display
./tools/run.sh --no-display     # Serial only

# Other options
./tools/run.sh --memory 512     # Custom RAM size
./tools/run.sh --gdb            # Wait for GDB on port 1234
./tools/run.sh --screenshot     # Capture screenshot and exit
```

**Features:**
- Multiple display backends (GTK, SDL, VNC, curses)
- Custom memory configuration
- GDB debugging support
- Screenshot capture
- Comprehensive help menu

### 2. `tools/run-simple.sh` — Auto-Detect Display
Tries multiple display backends automatically.

```bash
./tools/run-simple.sh
```

**How it works:**
1. Tries SDL
2. Falls back to GTK
3. Falls back to Cocoa (macOS)
4. Falls back to Curses
5. Falls back to serial-only

Great for quick testing when you don't know which display backend works.

### 3. `tools/run-gui.sh` — GTK Only
Launches with GTK window (requires X11/Wayland).

```bash
./tools/run-gui.sh
```

### 4. `tools/run-vnc.sh` — VNC Only
Launches VNC server on port 5900.

```bash
./tools/run-vnc.sh

# In another terminal:
vncviewer localhost:5900
```

Perfect for headless/SSH environments.

### 5. `tools/run-screenshot.sh` — Screenshot Capture
Attempts to capture a screenshot of the boot TUI.

```bash
./tools/run-screenshot.sh
```

**Note:** May require additional permissions for QEMU monitor access.

## Recommended Usage

### For Development (with display)
```bash
./tools/run.sh --sdl
# or
./tools/run-simple.sh
```

### For Headless/SSH Environments
```bash
./tools/run.sh --vnc
# Then connect with VNC viewer
```

### For Quick Tests
```bash
./tools/run.sh --no-display
# Serial output shows all TUI log messages
```

### For Debugging
```bash
# Terminal 1: Start kernel with GDB wait
./tools/run.sh --gdb --no-display

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
./tools/run-simple.sh

# Or use specific display
./tools/run.sh --sdl
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
./tools/run.sh --vnc
```

### "gtk initialization failed"
Your environment doesn't have X11/Wayland. Try:
```bash
./tools/run.sh --sdl
# or
./tools/run.sh --vnc
```

### Want to verify TUI works without display
```bash
./tools/run.sh --no-display
```
All TUI log messages appear in serial output, so you can verify the TUI is updating correctly.

## All Scripts Summary

| Script | Purpose | Best For |
|--------|---------|----------|
| `run.sh` | Full-featured with options | Development, debugging |
| `run-simple.sh` | Auto-detect display | Quick testing |
| `run-gui.sh` | GTK only | Desktop with X11 |
| `run-vnc.sh` | VNC only | Headless/SSH |
| `run-screenshot.sh` | Screenshot capture | Documentation |

## Next Steps

After verifying the TUI works:
1. To enable silent mode, see `docs/README_TUI.md`
2. To customize colors, edit `sunlight-tui/src/layout.rs`
3. To modify layout, edit `sunlight-tui/src/splash.rs`
