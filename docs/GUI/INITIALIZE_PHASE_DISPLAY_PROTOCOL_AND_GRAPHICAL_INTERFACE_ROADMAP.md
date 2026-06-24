# SunlightOS — Initialize Phase: Display Protocol & Graphical Interface Roadmap

**Status:** Planning / Design / Audit updated  
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

## Phase 1 Audit Findings

This audit answers the three gating questions for the display protocol work using the current kernel/userspace implementation.

### 1. Capability Passing

**Answer:** Yes, register IPC already carries up to two capability tokens per message, but shared-page transfer is currently just token handoff, not broker-mediated rights derivation.

- `ipc::IpcMsg` has `cap_count` plus `caps[0..2)`, and the userspace/kernel ABI transports them in `r13`/`r14`.
- `ipc_call`, `ipc_recv`, and reply paths preserve those fields end-to-end.
- `shm_alloc()` returns a `CapabilityToken`, and another process can successfully call `shm_map(token)` after receiving that token.
- What does **not** exist yet is a separate "derive a narrower grant for process B" flow for shared memory. The token itself is the authority. If process A shares it, process B can map it until the owner frees it or exits.

**Implication for the GUI plan:** We can bootstrap window backing storage without new kernel IPC features, but the security model is currently "whoever learns the token may map the page." That is acceptable for an early single-client prototype, but not the final shape for multi-client GUI isolation.

### 2. Mapping Mechanisms

**Answer:** `mmap` does not currently map fd-backed objects at all.

- `kernel/src/process/mmap.rs` only accepts `MAP_ANONYMOUS`.
- The `fd` parameter is ignored by the native `sys_mmap` path.
- There is no current VFS/kernel path where an anonymous memory object behind an fd is resolved and mapped.
- The real shared-memory mechanism today is `ShmAlloc` + `ShmMap`, not `mmap(fd=...)`.

**Implication for the GUI plan:** The initial display protocol should use `ShmAlloc`/`ShmMap` directly. The roadmap should not assume that a returned identifier can be turned into a normal `mmap` call yet.

### 3. Size Limits

**Answer:** Current shared memory is page-granular, while anonymous `mmap` already supports arbitrary lengths.

- `SYS_MAP_TELEMETRY` maps exactly one read-only telemetry page at a fixed user address.
- `ShmAlloc` / `ShmMap` currently operate on **one 4 KiB page per token**.
- Anonymous `mmap` can map arbitrary lengths by allocating however many pages are needed, but that memory is private to the process, not shared with another process.

**Implication for the GUI plan:** A real framebuffer-sized shared window buffer needs either:
1. a new multi-page shared-region primitive, or
2. a Phase 1 prototype constrained to a very small single-page surface.

For any practical `320x240x4` or `400x300x4` window, single-page shared memory is insufficient.

### Audit Summary

The display stack does **not** need new IPC transport primitives first. It **does** need a better shared-memory object model before a useful graphical client can exist. The next plan should therefore treat "multi-page shared backing store" as the real first blocker, ahead of compositor policy.

---

## Transition to SHM Objects

The audit points to one concrete architectural step: move from page tokens to kernel-managed SHM objects.

### Target Contract

**Kernel object shape**
- `SYS_SHM_CREATE(size, flags)` allocates a shared-memory object and returns a capability token or fd-like handle.
- The object owns a set of physical pages, its total length, and its lifetime.
- The object can be mapped into multiple processes at the same physical backing.

**Userspace flow**
1. Process A calls `shm_create(size, flags)`.
2. Process A maps it locally with `shm_map(token, prot)`.
3. Process A transfers the handle through IPC.
4. Process B maps the same object into its own address space with the same handle.

This is the right abstraction for display buffers because a practical framebuffer needs multiple pages. A 1080p 32-bit buffer is about 8.3 MiB, so a single-page grant is not enough except for tiny prototypes.

### What Changes In The Plan

- The GUI plan should stop treating `ShmAlloc` / `ShmMap` as the end state.
- Single-page token sharing remains useful as an early primitive, but it should be framed as a stepping stone.
- The display service should consume SHM objects, not anonymous process-private mappings.
- The first kernel milestone is no longer "can we share one page?" but "can we create, map, and transfer a multi-page SHM object safely?"

### Milestone Ordering

The first visible GUI milestone is a single window with two eyes following the mouse cursor.

1. Audit the current shared-page capability implementation.
2. If it already supports multi-page shared mappings, reuse it.
3. If it is page-limited or service-specific, generalize it into SHM objects.
4. Add the display buffer protocol using SHM capabilities.
5. Build the eyes demo on top of one shared pixel buffer.

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
  - `caps[0]` = shared-memory handle
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
Use the kernel's shared-memory object handle as the buffer authority. The display service should return that handle in `caps[0]`, and the client should map it with `shm_map(handle, prot)` once the kernel SHM-object path exists. If a transitional token-only path is retained briefly, it should be documented as a compatibility shim rather than the model the GUI stack is built around.

---

## Client Event Loop Contract (Idiomatic Pattern)

This becomes the canonical way to write graphical SunlightOS applications:

```rust
// Typical single-threaded graphical client
let (shm_handle, window_id, stride) = sgp_create_window(320, 240);

let buffer = unsafe {
    // Target SHM-object path: map the shared object with the libc wrapper.
    shm_map(shm_handle, PROT_READ | PROT_WRITE).unwrap() as *mut u32
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

## Revised Roadmap

### Phase 1: Shared-Page Capability Audit

**Goals**
- Audit the current shared-page capability implementation.
- Determine whether it already supports multi-page shared mappings.
- Classify the current design as reusable, page-limited, or service-specific.
- Document the current delegation and cleanup semantics.

**Deliverables**
- Audit notes captured in this roadmap.
- A clear reuse or generalization decision.
- A concrete list of kernel changes needed, if any.

**Gate**
- We can state whether the existing shared-page design is sufficient for the GUI path.
- We know whether the next step is reuse or SHM-object generalization.

### Phase 2: Kernel SHM Objects

**Goals**
- Only if Phase 1 proves the current design is page-limited or service-specific:
  generalize it into kernel-managed SHM objects.
- Implement a kernel-side `ShmObject` that owns a multi-page backing store.
- Back each object with a `Vec<PhysFrame>` sized to the requested region.
- Make the capability table hold a reference-counted `ShmRegion(Arc<ShmObject>)` entry.
- Include rights in the ABI from the beginning, even if initial enforcement is simple.
- Support mapping the same object into multiple processes.
- Keep revocation and cleanup deterministic when capability references drop to zero.
- Replace or repurpose the existing single-page SHM syscalls with the new object-based ABI.

**Deliverables**
- Kernel SHM object type and lifetime tracking.
- Capability integration for `ShmRegion`.
- Syscall ABI for `SYS_SHM_CREATE` and `SYS_SHM_MAP`.
- Rights constants and propagation through the ABI.
- Tests for multi-process mapping of the same buffer and automatic cleanup.

**Gate**
- Process A and Process B can map the same SHM object and observe writes from one another.
- The kernel can revoke or clean up the object without leaking physical pages.
- Dropping the last capability reference frees the backing frames automatically.
- The implementation can express read-only and read/write roles from the start, even if v0 enforcement is permissive.

### Kernel Spec

**ShmObject structure**
- `size: usize`
- `frames: Vec<PhysFrame>`
- The frames do not need to be physically contiguous.
- The object is logically contiguous when mapped into a process.

**Lifetime**
- The object is owned by reference-counted capability state.
- When the last reference is dropped, the object frees its frames back to the allocator.

**Capability integration**
- Add a capability variant for shared memory regions: `ShmRegion(Arc<ShmObject>)`.
- The token passed through IPC is a handle to the same reference-counted object.
- Capability transfer should clone the reference, not copy the frames.

**Rights**
- `pub const SHM_READ: u64 = 1 << 0;`
- `pub const SHM_WRITE: u64 = 1 << 1;`
- `pub const SHM_TRANSFER: u64 = 1 << 2;`
- v0 may allow any holder to map a token, but the types and ABI must already express read/write roles.
- v1 should be able to restrict clients to read/write while giving the display service read-only access.

**Syscall ABI**
- `SYS_SHM_CREATE` (`92`): `rdi = size` in bytes, `rsi = flags`.
- `SYS_SHM_MAP` (`93`): `rdi = cap_token_id`, `rsi = prot_flags`.
- `SYS_SHM_CREATE` allocates the frames, wraps them in an `Arc<ShmObject>`, records the requested rights, and returns a capability token to the caller.
- `SYS_SHM_MAP` validates the capability, finds a free virtual range large enough for the object, and maps the frames into the current process with the requested protection flags.
- For v0, `SYS_SHM_MAP` may permit any valid token holder to map; the rights fields still need to exist for the later isolation pass.

**Libc surface**
- `sunlight-libc/src/sys.rs` should expose `SYS_SHM_CREATE = 92` and `SYS_SHM_MAP = 93`.
- `sunlight-libc/src/lib.rs` should expose `shm_create(size, flags)` and `shm_map(token, prot)`.
- The API should stay small enough for the window manager and first graphical client to use directly.

**Required test**
- `shm-writer` or `shm-test-client` creates a 128 KiB SHM object, maps it read/write, writes `SUNLIGHT_SHM_OK` at offset `4096`, and sends the token over IPC.
- `shm-reader` or `shm-test-server` receives the token, maps it read-only, reads the same offset, and replies OK only if the bytes match.
- The final step is that dropping the last capability reference frees the frames automatically.

### Phase 3: Basic SGP Skeleton on Top of Real SHM

**Goals**
- Do not start SGP until the Phase 2 SHM test passes.
- Add the display buffer protocol using SHM capabilities.
- Add protocol constants and client helpers for `CREATE_WINDOW`, `COMMIT_FRAME`, and `EVENT_POLL`.
- Build `sunlight-display` with:
  - nameserver registration,
  - one-window-per-client state,
  - shared backing-store allocation from SHM objects,
  - `COMMIT_FRAME` acknowledgement,
  - a blocking `EVENT_POLL` stub.
- Kernel or display service obtains the real Limine framebuffer.
- Prove that a client can map a shared buffer, write pixels, and commit.

**Deliverables**
- New crate `services/sunlight-display` (or `sunlight-wm`).
- Protocol constants added to `ipc/`.
- Minimal `CREATE` / `COMMIT` / `POLL` handling.
- Client-side helpers for SHM creation, transfer, and `shm_map`.

**Gate**
- A test program creates a window, writes a solid color or simple pattern to the shared buffer, calls `COMMIT_FRAME`, and the serial log shows the commit was received.
- `./tools/test.sh` still passes.

### Phase 4: Input Routing + Blocking EVENT_POLL

**Goals**
- Modify input path so `sunlight-mouse` events are delivered to the display service instead of directly to `tty_server`, or add a transitional fan-out path.
- Implement true blocking `EVENT_POLL`:
  - Display service keeps per-window waiters.
  - When a mouse event arrives for the focused window, it replies to the waiting client's blocked call.
- Route the packed `u64` mouse event through to the waiting client unchanged at first.
- Add the first focus rule for a single active window.

**Deliverables**
- Display service becomes the owner of the current mouse coordinate space.
- Working blocking receive for at least one client.
- Focus concept for the single-window prototype.

**Gate**
- A client blocked in `EVENT_POLL` wakes exactly when the mouse moves.
- Serial logs show clean "event delivered" + client processing.

### Phase 5: Eyes Demo — First Window

**The canonical validation application.**

**Requirements**
- Small fixed-size window (e.g. 400×300).
- Two eyes that follow mouse cursor movement.
- On every mouse event:
  1. Read the packed `(abs_x, abs_y, buttons)`.
  2. For each eye, compute pupil position (simple vector math from eye center toward mouse, clamped to eye radius).
  3. Draw black filled pupils.
  4. (Optionally) clear previous frame or use double-buffering in the same buffer.
- Call `SGP_COMMIT_FRAME`.

**Non-goals for this milestone**
- Window decorations, title bars, multiple windows, dragging, keyboard input to the app.

**Success criteria**
- Moving the mouse in QEMU causes the two eyes to track the cursor smoothly with imperceptible lag.
- No CPU spin when the mouse is still (use `top`/`sysinfo` or just observe).
- The client binary runs as a normal user-space program launched from shell or init.

**Implementation notes**
- Start with immediate composition inside the display service (on `COMMIT_FRAME`, copy the client's SHM region into the on-screen framebuffer at the window's current location + draw cursor on top).
- Keep the compositor itself in a simple poll/reply loop on its endpoint + the mouse source.

### Phase 6: Progressive Decoration & Basic Window Shell

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

1. Treat SHM objects as the first engineering task, not as an implementation detail under `CREATE_WINDOW`.
2. Put the protocol definition in `ipc/src/lib.rs` under `SgpMsg` once the SHM contract is decided.
3. Add the libc wrappers for `shm_create` and `shm_map` alongside the kernel ABI change.
4. Create `services/sunlight-display` following the exact structure of existing small services.
5. Use the same `ipc_reply_and_try_recv` + yield pattern the compositor will use while it also waits on other sources.
6. Keep the first client extremely small — no dependencies on complex UI libs.

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
- `kernel/src/process/mmap.rs` — current anonymous-only mmap implementation
- `kernel/src/memory/shared.rs` — current single-page shared-memory implementation
- `kernel/src/capability/mod.rs` — shared-page token minting and revocation
- `kernel/src/arch/x86_64/syscall.rs` — `ShmAlloc` / `ShmMap` / `MapTelemetry` syscall handlers
- `ipc/src/lib.rs` — IpcMsg, capability transport, and Shm syscalls
- `services/sunlight-mouse/src/main.rs` — reference implementation of a clean input driver
- `services/tty_server/src/main.rs` — current input routing example
- `docs/README_TUI.md` and Phase 2.5 documents — current framebuffer usage
- `tools/run-gui.sh` — convenient way to launch with graphical output

---

## Summary — The Minimalist Path

1. Close the shared-memory gap first: framebuffer-sized shared regions.
2. Keep IPC simple: existing register IPC plus capability slots are enough.
3. Build the tiny protocol after the SHM contract is real.
4. Validate with a single client ("Eyes Tracker").
5. Layer composition and window-management policy only after the core loop is proven.

This keeps SunlightOS feeling like a clean embedded OS even when it has a windowing system: the machine is asleep unless the user (or an event) gives it a reason to wake up.

The Eyes Tracker will be the moment we know the architecture is correct.
