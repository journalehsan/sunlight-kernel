# SunlightOS Graphical Boot TUI

Phase 2.5 adds a graphical boot TUI that renders directly to the framebuffer.

## Quick Start

### Build and Run
```bash
# Build the kernel with TUI
./tools/build.sh

# Run with the unified runner
./tools/runs.sh --help
```

## Display Options

### Option 1: Graphical Window (Recommended)
```bash
./tools/runs.sh             # Default: GTK window
./tools/runs.sh --sdl       # SDL window
```

**What you'll see:**
- **Header Bar** (48px): `☀ SunlightOS  v0.1.0 | Phase 2 | DEBUG`
- **Main Zone**: Scrolling log panel with colored messages:
  - Green for "OK" messages
  - Red for errors/panics
  - Yellow for warnings
  - White for info
- **Footer Bar** (32px): `Status: OK  CPU: x86_64  RAM: 251 MiB`
- **Progress Bar**: Shows 0-100% boot progress
- **Animated Spinner**: Rotates during operations

### Option 2: VNC Server
```bash
./tools/runs.sh --vnc

# In another terminal:
vncviewer localhost:5900
```

### Option 3: Text Mode (Curses)
```bash
./tools/runs.sh --curses
```

### Option 4: Serial Only
```bash
./tools/runs.sh --no-display
```
The TUI still renders to the framebuffer, but you won't see it.

## TUI Features

### Debug Mode (Default)
Shows a scrolling log panel with all boot messages:
```
┌── Boot Log ────────────────────────────────────────┐
│ [PMM] Initializing...                              │
│ [PMM] 236/251 MiB free                             │
│ [PMM] OK                                           │
│ [VMM] OK                                           │
│ [IDT] OK                                           │
│ [HEAP] OK                                          │
│ [SYSCALL] OK                                       │
│ [CAP] Capability broker initialized                │
│ [IPC] IPC bus initialized                          │
│ [PROC] init pid=1                                  │
│ [PROC] timer_server pid=2                          │
│ [SunlightOS] Phase 2 OK                           │
└────────────────────────────────────────────────────┘

Status: SunlightOS ready                          ⟳
▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓  100%
```

### Silent Mode (Feature Flag)
Centered branding with minimal output:
```
                  ☀  (sun logo)

                 SunlightOS
             Lightweight. Secure.

          ─────────────────────────────
             Initializing memory...
          ▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░  60%
          ─────────────────────────────

          ✓ Physical memory manager
          ✓ Virtual memory manager
          ⟳ Loading capability broker...
```

**Enable silent mode:**
```bash
# Compile-time
cargo build --package sunlight-kernel --features sunlight-tui/shutup-mode

# Or edit kernel/src/main.rs and change:
sunlight_tui::BootMode::Debug
# to:
sunlight_tui::BootMode::Silent
```

## Advanced Options

```bash
# Custom memory size
./tools/runs.sh --memory 512

# Enable GDB debugging (waits for GDB on port 1234)
./tools/runs.sh --gdb

# Multiple options
./tools/runs.sh --sdl --memory 512 --debug
```

## Technical Details

### Implementation
- **Pure Rust, `no_std`** — no heap allocations
- **Zero external dependencies** — only uses `core`
- **No floating-point math** — fixed-point Q10 arithmetic
- **Initialized before PMM** — works before kernel heap exists

### Architecture
```
sunlight-tui/
├── framebuffer.rs  — Raw pixel operations
├── font.rs         — 8×16 bitmap font (1520 bytes)
├── draw.rs         — Primitives (rect, progress, spinner)
├── fmt.rs          — No-alloc formatting
├── layout.rs       — Three-zone layout
├── splash.rs       — Main API
└── modes/
    ├── debug.rs    — Scrolling log (64-line buffer)
    └── silent.rs   — Centered branding
```

### Color Palette
- **Background**: `#000000` (black)
- **Surface**: `#111111` (header/footer)
- **Accent**: `#E8820C` (SunlightOS orange)
- **Text**: `#EEEEEE` (white)
- **Success**: `#44CC44` (green)
- **Error**: `#FF4444` (red)
- **Warning**: `#FFAA00` (yellow)

## Troubleshooting

### "No framebuffer" error
The Limine bootloader provides the framebuffer. If you see this error,
ensure `limine.cfg` requests a framebuffer:

```
PROTOCOL=limine
RESOLUTION=1024x768
```

### Display initialization fails
This is common in headless/SSH environments. Use VNC:
```bash
./tools/runs.sh --vnc
# Connect with: vncviewer localhost:5900
```

### Serial output only
If you can't get a graphical display but want to verify the TUI is working,
check the serial output — all log messages that appear in the TUI are also
printed to serial.

## Files

- `tools/runs.sh` — Canonical runner with all options
- `tools/run.sh` — Compatibility shim to `runs.sh`

## Verification

The TUI updates at each boot phase:
1. **0%** — TUI initialized (before PMM)
2. **10%** — Physical memory manager
3. **20%** — Virtual memory manager
4. **30%** — Interrupt descriptor table
5. **40%** — Kernel heap
6. **50%** — System calls
7. **60%** — Capability broker
8. **70%** — IPC bus
9. **80%** — Init process
10. **90%** — Timer server
11. **100%** — Ready

All messages appear in both the graphical TUI and serial output.

## Login Background

The framebuffer login screen displays an optional static background image
behind the login card:

- **VFS path:** `/usr/share/sunlightos/backgrounds/login-background.tga`
- **Format:** TGA type 2 (uncompressed true-color, 24 bpp)
- **Rendering:** aspect-fill (preserve ratio, fill screen, crop if needed)
- **Overlay:** ~40 % translucent dark overlay for UI readability
- **Fallback:** plain dark background on decode failure
- The image is embedded in the `tty_server` binary at compile time.
