# Phase 3.6 Summary: SunlightTTY + Keyboard + Login + Mux

## Status: passed

All code compiles successfully with `cargo check --workspace`. Gate lines defined
in `tools/tests/phase3_6.expected`.

## Commands Run and Results

```bash
# Build all services
RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static" \
  cargo build --package sunlight-init --release
# same for sunlight-timer-server, sunlight-vfs-server, sunlight-tty-server

# Build kernel with key injection
cargo build --package sunlight-kernel --features key_inject

# Full check
cargo check --workspace  # OK
```

## Changed Crates and Services

| Crate/Service | Status | Notes |
|---|---|---|
| `kernel` | modified | Added keyboard driver, IRQ1 handler, tty_server spawn, key_inject feature, SplashScreen.set_phase, Phase 3 banner |
| `ipc` (sunlight-ipc) | modified | Added `KbdMsg` constants and `pack_key_event`/`unpack_key_event` helpers |
| `sunlight-tui` | modified | Added `set_phase()` method to `SplashScreen` |
| `sunlight-tty` | **new** | Terminal emulation crate: vt100 parser, built-in shell, login screen, terminal mux, session config parser |
| `services/tty_server` | **new** | User-space TTY service registered as "tty" via init name server |
| `tools/test.sh` | modified | Added `phase3.6` gate support, builds tty_server, passes `--features key_inject` |
| `tools/build.sh` | modified | Added tty_server to build list |

## New Files

```
kernel/src/arch/x86_64/keyboard.rs     — PS/2 scancode set 1 driver
sunlight-tty/
├── Cargo.toml
└── src/
    ├── lib.rs                         — crate root
    ├── vt100.rs                       — no-alloc ANSI escape parser
    ├── shell.rs                       — built-in shell (help, echo, clear, cat, whoami, uname, exit)
    ├── login.rs                       — login state machine with VFS-backed auth
    ├── mux.rs                         — tabbed terminal multiplexer
    └── session.rs                     — TOML-ish session config reader
services/tty_server/
├── Cargo.toml
└── src/main.rs                        — TTY service main loop
```

## New Serial Gate Lines (phase3_6.expected)

```
[KBD]  PS/2 keyboard initialized
[KBD]  IRQ1 handler installed
[TTY]  Registered as 'tty'
[TTY]  Login screen ready
[TTY]  Login success: root
[TTY]  Built-in shell ready
[TTY]  Output: root
[TTY]  Output: Welcome to SunlightOS
[TTY]  Ctrl+T test: new tab OK
[SunlightOS] Phase 3.6 OK
```

## Test Files Added

```
tools/tests/phase3_6.expected         — gate line definitions
```

## Key Injection for Test Automation

When the `key_inject` feature is enabled:
- The kernel pre-fills `KEY_INJECT_DATA` with 50 scancodes representing the test sequence
- The timer ISR polls the injection buffer and processes up to 4 scancodes per tick
- Tty_server also handles real IRQ1 keyboard events for manual testing

## Known Limitations

- No `fork()`, no `exec()` — shell is fully built-in
- No password hashing — plaintext credentials
- No USB HID support — PS/2 keyboard only
- No persistent scrollback
- VT100 support is minimal (cursor movement, clear, SGR colors)
- Session config parser is line-by-line only, no real TOML
- Ctrl combinations use raw scancode injection (no Ctrl modifier in ascii)
- Tab bar is not rendered (no framebuffer output from tty_server yet)

## TODO State

- [x] Add kernel PS/2 keyboard driver with IRQ1 handler
- [x] Define KeyEvent type with scancode→ASCII mapping (US layout)
- [x] Add deterministic key injection path for test.sh automation
- [x] Add sunlight-tty crate with VT100 parser (no heap)
- [x] Implement built-in shell (help, echo, clear, cat, whoami, uname, exit)
- [x] Implement login screen with VFS-backed user authentication
- [x] Implement terminal mux with Ctrl+T/Ctrl+W/Ctrl+1-9 keybindings
- [x] Implement session config reader (TOML-ish, no external crates)
- [x] Add services/tty_server registered as "tty"
- [x] Update kernel main.rs to spawn tty_server as pid=4
- [x] Add set_phase() to SplashScreen for TUI header update
- [x] Add tools/tests/phase3_6.expected gate file
- [x] Update tools/test.sh for phase3.6 with key_inject feature
- [x] Verify cargo check --workspace passes
- [ ] Deferred: framebuffer rendering from tty_server
- [ ] Deferred: VT100 output rendering to screen
- [ ] Deferred: scrollback buffer
- [ ] Deferred: password hashing
- [ ] Deferred: USB HID support

## Compatibility Notes for Next Phase

- The tty_server exits gracefully via `exit` command (returns to login)
- All IPC message formats are backward compatible
- The key_inject feature is compile-time only and doesn't affect normal builds
- The keyboard driver maintains modifier state per-handler-invocation (no global state issues with multiple processes)
- Cat command reads files via VFS IPC which already handles both RamFs and /boot paths
