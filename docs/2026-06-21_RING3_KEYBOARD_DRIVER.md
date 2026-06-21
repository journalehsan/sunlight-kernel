# 🎉 First Ring-3 Driver: Keyboard Migrated Out of the Kernel

**Date:** 2026-06-21, 10:18 PM (at the office — and then home 😄)
**Milestone:** SunlightOS's **first true user-space (ring-3) device driver**.

---

## Why this matters

Until today the PS/2 keyboard lived **inside the kernel**: IRQ1 was handled in
ring 0, scancodes were translated to ASCII in kernel space, and key events were
injected straight into the TTY. That is exactly the kind of in-kernel device
logic that makes a kernel a *hybrid* rather than a *microkernel*.

We moved **all** of that policy into a standalone ring-3 process,
`services/sunlight-kbd`. The kernel now does the bare minimum a microkernel
should: acknowledge the interrupt, read the raw byte, and hand it off. Everything
else — scancode set 1 decoding, modifier tracking, ASCII translation — happens in
user space and is delivered to `tty_server` over normal IPC.

This is the proof-of-concept for the driver model. The same pattern now scales to
mouse, disk, and network drivers. **SunlightOS stays a microkernel.**

---

## The architecture

```
  hardware
     │ IRQ1
     ▼
┌─────────────────────────────────────────┐  ring 0 (kernel)
│ handle_irq1() (arch/x86_64/keyboard.rs)  │
│  • read raw scancode from port 0x60      │
│  • push into lock-free ring buffer       │
│  • notify_driver(): wake sunlight-kbd    │
│  • EOI to PIC                            │
└─────────────────────────────────────────┘
     │ syscall 112 (kbd_pop_scancode)   ▲ wake_pid + endpoint msg
     ▼                                  │
┌─────────────────────────────────────────┐  ring 3 (sunlight-kbd)
│  • pop raw scancodes                     │
│  • scancode-set-1 → ASCII + modifiers    │
│  • pack KEY_EVENT, ipc_call("tty")       │
└─────────────────────────────────────────┘
     │ IPC: KbdMsg::KEY_EVENT
     ▼
┌─────────────────────────────────────────┐  ring 3 (tty_server)
│  unpack_key_event → login / shell input  │
└─────────────────────────────────────────┘
```

The kernel keeps **no** keyboard policy. It only owns the IRQ stub, a small raw
ring buffer, and a syscall surface (110 register, 112 pop-scancode).

---

## The bugs we hunted (and squashed)

Getting the first ring-3 driver alive surfaced two stacked bugs. The keyboard was
completely dead and the symptom — an endless `[IRQ1]` storm with nothing on screen
— pointed everywhere *except* the actual causes.

### Bug 1 — `tty_server` never registered the name "tty"

`tty_server` printed `[TTY] Registered as 'tty'` but the line was **just a log
message** — the real `nameserver_register("tty", ep)` call was missing. The
ring-3 keyboard driver is the *first* code that ever needed to resolve "tty" (the
old in-kernel keyboard injected into the TTY directly), so the gap had sat latent
forever.

Result: `nameserver_lookup("tty")` returned `DENY` (confirmed by instrumentation:
`reply.label = 0x4`), and the driver spun in its lookup loop, never reaching its
key-processing loop.

**Fix:** add the actual `nameserver_register("tty", ep)` in `tty_server`.

### Bug 2 — endpoint **token** vs endpoint **id** mismatch

With the driver finally running, it blocked on `ipc_recv(my_endpoint)` waiting to
be woken for each keypress. But the kernel was queuing key events on the **wrong
endpoint**:

- `endpoint_create()` returns a **capability token** (e.g. `0x40000001CC6C80BF`),
  not the internal endpoint id.
- `kbd_register(my_endpoint.0 as u32)` stored that **token, truncated to u32**
  (`3429662911`) as `KBD_DRIVER_ENDPOINT`, and the kernel pushed key events onto a
  queue keyed by that phantom value.
- But `ipc_recv` resolves the token to the **real** endpoint id (`4`, per
  `[CAP] Created endpoint 4 for pid=6`) and reads from queue `4`.

Mismatch → events landed in a queue nobody reads → `ipc_recv` never returned → the
driver never looped back to `kbd_pop_scancode()`. `wake_pid` woke it, but it was
stuck inside `ipc_recv`'s WouldBlock retry loop.

**Fix:** the driver passes the **full u64 token** to `kbd_register`, and the
kernel's `sys_kbd_register` resolves it via
`CAP_BROKER.check(token, RECV_ONLY)` → real endpoint id (the same mapping
`ipc_recv` uses) before storing it.

> **General rule for every future ring-3 driver:** any kernel path that delivers
> to a user-space endpoint must resolve the capability token through
> `CAP_BROKER.check` — never use the raw token (or a truncation of it) as a queue
> key.

---

## Files touched

| File | Change |
|------|--------|
| `kernel/src/arch/x86_64/keyboard.rs` | reduced to a raw IRQ1 byte router (ring buffer + driver wake) |
| `kernel/src/arch/x86_64/syscall.rs` | `sys_kbd_register` resolves token → real endpoint id |
| `services/sunlight-kbd/` | **new** ring-3 keyboard driver (scancode decode + IPC) |
| `services/tty_server/src/main.rs` | real `nameserver_register("tty")`; consume `KEY_EVENT` |
| `services/init/src/main.rs`, `kernel/build.rs` | spawn + embed the new driver |

---

## Verified

```
[KBD] sunlight-kbd starting
[KBD] found tty, ready to process keys
[KBD] Registered user-space driver at endpoint 4
[KBD] registered with kernel IRQ1 router
```

Typing at the login screen works. First ring-3 driver: **alive.** 🟢

---

*Onward to the next driver — the pattern is proven and reusable.* 🚀
