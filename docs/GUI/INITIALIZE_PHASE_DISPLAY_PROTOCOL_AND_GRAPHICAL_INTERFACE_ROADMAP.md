# SunlightOS — Initialize Phase: Display Protocol & Graphical Interface Roadmap

**Status:** Planning / Design  
**Focus:** Foundational Sunlight Graphics Protocol (SGP), shared-memory windowing, minimal compositor, and the "Eyes Tracker" validation milestone.  
**Philosophy:** Keep the spirit of the existing microkernel — lightweight, event-driven, zero CPU waste when idle, register/IPC + mmap based.

This document is the authoritative plan for moving from the current TTY + framebuffer TUI world to a native graphical display stack.

---

## Current Foundation (What We Already Have)

- Clean Ring 3 `sunlight-mouse` driver:
  - Parses PS/2 3-byte packets.
  - Tracks **absolute** coordinates (clamped, Y-inverted).
  - Packs events as a single `u64`:
    ```
    word0 = abs_x (u16) | (abs_y (u16) << 16) | (buttons (u8) << 32)
    bits:
      [15:0]   = abs_x
      [31:16]  = abs_y
      [32]     = left
      [33]     = right
      [34]     = middle
      [63:35]  = reserved (0)
    ```
  - Delivers via `ipc_call` (label `0x2`) currently to `tty_server`.
- `sunlight-libc` with working `mmap` / `munmap`:
  - `MAP_SHARED | MAP_ANONYMOUS | MAP_PRIVATE`
  - `PROT_READ | PROT_WRITE`
- Mature synchronous register-based IPC (`IpcMsg`, `ipc_call`, `ipc_reply_and_try_recv`, `nameserver_register`/`lookup`, endpoints).
- Existing `ShmAlloc` / `ShmMap` / `ShmFree` syscalls (92-94) in the kernel.
- Framebuffer already provided by Limine and used by `sunlight-tui` (boot splash + current TTY rendering).
- No compositor / windowing system yet.

These pieces give us exactly the primitives needed for a zero-copy graphical stack.

---

## Core Architectural Decision: Blocking Event Listener

**Recommendation: Implement `SGP_EVENT_POLL` as a blocking call first.**

### Why Blocking Wins for SunlightOS

| Approach                    | CPU Waste | Latency | Animations | Fits Microkernel + Sync IPC | Verdict for Eyes Tracker |
|-----------------------------|-----------|---------|------------|-----------------------------|--------------------------|
| Blocking (sleep until input)| Zero      | Lowest  | Needs timeout later | Excellent                   | Ideal (recommended)     |
| Fixed-framerate loop        | High      | Up to 16 ms | Easy     | Poor (constant IPC)         | Avoid                   |
| Hybrid (timeout)            | Low       | Low     | Yes        | Good                        | Phase 2 evolution       |

**Blocking model fits the existing design perfectly:**

- The mouse driver already sends events asynchronously via IPC.
- The compositor can simply hold the client's thread on a wait list and reply when an event (or later, timeout) is ready.
- No busy loops, no extra timer infrastructure per client in the beginning.
- The CPU truly sleeps when nothing is happening — true embedded / RTOS spirit.

### SGP_EVENT_POLL Signature (Initial + Future)

**Phase 1 (Eyes Tracker):**
```
SGP_EVENT_POLL(window_id) -> packed_u64   // blocks until event available
```

**Later (when needed):**
```
SGP_EVENT_POLL(window_id, timeout_ms) -> Option<packed_u64>
```

`timeout_ms = 0` or special `TIMEOUT_INFINITE` for pure blocking.

The compositor implements this with its internal wait lists + (eventually) one-shot timers.

---

## Sunlight Graphics Protocol (SGP) — Minimal Register IPC

All calls use the existing 80-byte `IpcMsg` and fit in registers where possible.

**Nameserver name:** `display`

### Protocol Constants (proposed for `ipc/src/lib.rs`)

```rust
#[allow(non_snake_case)]
pub mod SgpMsg {
    // Client → Display
    pub const CREATE_WINDOW: u64 = 0xA101;
    pub const COMMIT_FRAME:  u64 = 0xA102;
    pub const EVENT_POLL:    u64 = 0xA103;
    pub const DESTROY_WINDOW: u64 = 0xA104;

    // Display → Client (replies / events)
    pub const REPLY:         u64 = 0xA1FF;
    pub const EVENT:         u64 = 0xA1FE;   // (future) pushed events if we add async later
}
```

### Message Layouts

**SGP_CREATE_WINDOW**
- Request:
  - `words[0]` = width (u32 low) | height (u32 high)   (or separate words)
  - `words[1]` = flags (bit 0 = resizable, etc.)
- Reply (`label = REPLY`):
  - `words[0]` = shm_token / identifier (or use capability in caps[0])
  - `words[1]` = window_id
  - `words[2]` = buffer size in bytes
  - `words[3]` = stride / pitch in bytes

**SGP_COMMIT_FRAME**
- Request:
  - `words[0]` = window_id
  - (optional) `words[1]` = damage rect or full-damage flag
- Reply: simple `REPLY` (ack)

**SGP_EVENT_POLL** (blocking initially)
- Request:
  - `words[0]` = window_id
  - `words[1]` = timeout_ms (0 or special value = infinite for first impl)
- Reply:
  - `label = REPLY`
  - `words[0]` = packed event u64 (same format as mouse today) **or** 0 if no event (timeout case)

**Event packing is deliberately the same as the mouse driver** so routing is trivial.

**SGP_DESTROY_WINDOW**
- `words[0]` = window_id

**shm_token strategy (initial):**
Use the kernel's existing `ShmAlloc`/`ShmMap` mechanism. The display service allocates a page (or multiple) and grants the client a mapping capability or returns an identifier that the client can turn into a `mmap` call. For the absolute minimum, the display service can do the allocation internally and reply with enough info for the client to `mmap` the same physical pages (MAP_SHARED semantics).

---

## Client Event Loop Contract (Idiomatic Pattern)

This becomes the canonical way to write graphical SunlightOS applications:

```rust
// Typical single-threaded graphical client
let (shm_token, window_id, stride) = sgp_create_window(320, 240);

let buffer = unsafe {
    // Map the shared region using libc mmap or direct ShmMap
    mmap(...) as *mut u32   // ARGB or RGBx pixels
};

loop {
    let event = sgp_event_poll(window_id, TIMEOUT_INFINITE);  // blocks

    update_state_from_event(event);   // mouse position, buttons, future keys

    // Zero-copy draw into the shared buffer
    render_scene(buffer, stride, current_state);

    sgp_commit_frame(window_id);
}
```

**Key properties:**
- One window per client to start (simplifies focus + routing).
- Blocking `EVENT_POLL` means the process does zero work between input.
- `COMMIT_FRAME` tells the compositor "this buffer is now valid for this window".
- The client never polls the mouse itself — it receives authoritative state from the display service.

---

## Improved 4-Phase Roadmap

### Phase 1: Shared Memory Foundation + Basic SGP Skeleton

**Goals**
- A new `sunlight-display` (or `sunlight-wm`) service that:
  - Registers as `display`.
  - Can allocate window backing storage (via mmap + MAP_ANONYMOUS or kernel Shm*).
  - Implements `SGP_CREATE_WINDOW` + `SGP_COMMIT_FRAME` (no-op initially) + `SGP_EVENT_POLL` (blocking stub).
- Kernel or display service obtains the real Limine framebuffer.
- Prove that a client can map a buffer, write pixels, and commit.

**Deliverables**
- New crate `services/sunlight-display` (or `sunlight-wm`).
- Protocol constants added to `ipc/`.
- Minimal `CREATE` / `COMMIT` / `POLL` handling (POLL can just block until we wire input).
- Client-side helpers (probably in a small `sunlight-gui` lib or directly in first app).
- Documentation of the shared memory grant mechanism used.

**Gate**
- A test program creates a window, writes a solid color or simple pattern to the SHM buffer, calls COMMIT, and the serial log shows the commit was received.
- `./tools/test.sh` still passes.

### Phase 2: Input Routing + Blocking EVENT_POLL (Eyes Tracker Prep)

**Goals**
- Modify input path so `sunlight-mouse` (and later keyboard) events are delivered to the display service instead of (or in addition to) tty_server.
- Or: have the display service register for "raw" input and perform focus routing itself.
- Implement true blocking `EVENT_POLL`:
  - Display service keeps per-window waiters.
  - When a mouse event arrives for the focused window, it replies to the waiting client's `ipc_reply_and_wait` / similar.
- Route the packed `u64` mouse event through to the waiting client unchanged (or lightly wrapped).

**Deliverables**
- Display service becomes the owner of the current mouse coordinate space.
- Working blocking receive for at least one client.
- Focus concept (the single window that gets mouse events).

**Gate**
- A client blocked in `EVENT_POLL` wakes exactly when the mouse moves.
- Serial logs show clean "event delivered" + client processing.

### Phase 3: Eyes Tracker — The First Real Graphical Client

**The canonical validation application.**

**Requirements**
- Small fixed-size window (e.g. 400×300).
- Two static white circles ("eyes").
- On every mouse event:
  1. Read the packed `(abs_x, abs_y, buttons)`.
  2. For each eye, compute pupil position (simple vector math from eye center toward mouse, clamped to eye radius).
  3. Draw black filled pupils.
  4. (Optionally) clear previous frame or use double-buffering in the same buffer.
- Call `SGP_COMMIT_FRAME`.

**Non-goals for this milestone**
- Window decorations, title bars, multiple windows, dragging, keyboard input to the app.

**Success criteria**
- Moving the mouse in QEMU causes the pupils to track the cursor smoothly with imperceptible lag.
- No CPU spin when the mouse is still (use `top`/`sysinfo` or just observe).
- The client binary runs as a normal user-space program launched from shell or init.

**Implementation notes**
- Start with immediate composition inside the display service (on `COMMIT_FRAME`, copy the client's SHM region into the on-screen framebuffer at the window's current location + draw cursor on top).
- Keep the compositor itself in a simple poll/reply loop on its endpoint + the mouse source.

### Phase 4: Progressive Decoration & Basic Window Shell

Layer features one at a time, each independently testable:

1. **Hardware / software cursor layer**
   - Display service maintains a cursor bitmap.
   - Renders it last, right before flush to physical framebuffer.
   - Cursor remains responsive even if client is slow.

2. **Simple window chrome**
   - Display draws a 1-pixel border + small top bar around each client's buffer.
   - Store per-window position + size.

3. **Basic input routing for multiple windows**
   - Hit-testing mouse position against window rects.
   - Only the topmost / focused window receives events.

4. **Dragging**
   - When mouse is down over a window's title bar region, treat subsequent moves as "move this window".
   - Update offsets in the compositor; no client involvement needed.

5. **Expose / repaint**
   - When a window is uncovered, send a synthetic event (or the client simply gets the next mouse move and can choose to redraw).

---

## Compositor Synchronization Strategy

**Start simple — Immediate Composition:**

- Client calls `SGP_COMMIT_FRAME(window_id)`.
- Compositor immediately:
  1. Copies the client's SHM buffer region for that window into the backbuffer at the window's (x,y) offset.
  2. Overlays the cursor.
  3. Flushes to the physical Limine framebuffer (or page-flips if we add double-buffering later).
  4. Replies to the client.

This serializes client render + composition. Perfect for Eyes Tracker (single window).

**Planned upgrade path (documented but deferred):**

- Mark windows "dirty".
- Use a VBlank / timer driven composite pass.
- Only copy dirty regions.
- Enables tear-free multi-window updates and future animations.

---

## Eyes Tracker — Minimal Specification

**Window size:** 320×240 or 400×300 recommended.

**Visual elements:**
- Two eyes (white circles with thin black outline).
- Two pupils (small black circles).
- Pupil movement constrained inside each eye.
- Background can be a solid color or very light grid.

**Behavior:**
- Pupils follow mouse position.
- No requirement to handle buttons yet (but the packed data is available).

**Build & launch:**
- Should be buildable as a normal crate under `services/` or as a standalone example.
- Launched like any other user binary once the display service is running.

---

## Non-Goals (This Initialize Phase)

- Full widget toolkit
- Wayland/X11 compatibility
- Hardware acceleration (we are pure software)
- Complex input (multi-touch, tablets)
- Client-side decorations (we own them in the compositor)
- Animations beyond what a timeout on POLL can provide

These come after the Eyes Tracker is solid.

---

## Implementation Recommendations

1. Put the protocol definition in `ipc/src/lib.rs` under `SgpMsg` (matching `RandMsg`, `SmMsg` style).
2. Create `services/sunlight-display` following the exact structure of `sunlight-mouse`, `sunlight-kbd`, `timezone_service`.
3. Use the same `ipc_reply_and_try_recv` + yield pattern the compositor will use while it also waits on other sources (new client connections, etc.).
4. Make the mouse driver configurable or add a second delivery path so tty_server and the display service can coexist during transition.
5. Keep the first client extremely small — no dependencies on complex UI libs.

---

## Future Evolution (After Eyes Tracker)

- Add `timeout_ms` parameter to `EVENT_POLL`.
- Add keyboard events (reuse/extend existing `KbdMsg`).
- Add `CONFIGURE_WINDOW` / move / resize protocol.
- Damage rectangles for efficiency.
- Proper back-buffer + front-buffer + atomic present.
- VBlank-driven compositor tick.
- Multiple clients + stacking order + focus model.
- Basic window manager policy (click to focus, etc.).

---

## References & Related Documents

- `docs/2026-06-22_RING3_MOUSE_DRIVER.md` — source of the packed mouse event format
- `sunlight-libc/src/mman.rs` — mmap constants and wrappers
- `ipc/src/lib.rs` — IpcMsg, RandMsg, existing service protocols, Shm syscalls
- `services/sunlight-mouse/src/main.rs` — reference implementation of a clean input driver
- `services/tty_server/src/main.rs` — current input routing example
- `docs/README_TUI.md` and Phase 2.5 documents — current framebuffer usage
- `tools/run-gui.sh` — convenient way to launch with graphical output

---

## Summary — The Minimalist Path

1. Blocking `EVENT_POLL` + shared framebuffer pages via mmap.
2. Tiny protocol (3 calls).
3. Single validating client ("Eyes Tracker").
4. Immediate composition first.
5. Everything else layered strictly after the core loop is proven.

This keeps SunlightOS feeling like a clean embedded OS even when it has a windowing system: the machine is asleep unless the user (or an event) gives it a reason to wake up.

The Eyes Tracker will be the moment we know the architecture is correct.
