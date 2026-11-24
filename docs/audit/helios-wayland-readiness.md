# Helios Wayland readiness and cross-ABI buffer audit

Source review of the working tree at `1e5204285`, including pre-existing
local changes. **Buffer fixture verdict: blocked.** No guest fixture was built
or run. Source findings below do not constitute runtime results.

## Existing graphics route

The [workspace manifest](../../Cargo.toml) includes the kernel, Helios
compatibility crate, native UI and display service. The default build target is
`x86_64-unknown-none` ([config](../../.cargo/config.toml#L4)); Helios selects a
Linux syscall personality for an ELF marked `HLNX01`
([ELF loader](../../sunlight-elf/src/lib.rs#L18),
[`exec_into_process`](../../kernel/src/process/spawn.rs#L142),
[`translate_syscall`](../../compat-linux/src/lib.rs#L375)).

```text
Current: native UI → SGP IPC → sunlight-display → framebuffer / GPU scanout
Proposal: Helios client → AF_UNIX + SCM_RIGHTS → guest bridge → SGP → same output
```

[Init](../../services/init/src/main.rs#L29) starts the input and display
services. [Display `_start`](../../services/sunlight-display/src/main.rs#L8095)
registers `display_server` and discovers the framebuffer. The display is a
native userspace process and owns window state, focus, composition and output.
The keyboard and mouse services produce input; the display routes it through
[`EVENT_POLL`](../../services/sunlight-display/src/main.rs#L9365).
[`Window::connect_with_flags`](../../sunlight-ui/app.rs#L396) creates an SGP
window; [`commit`](../../sunlight-ui/app.rs#L745) publishes pixels.
[`redraw_scene`](../../services/sunlight-display/src/main.rs#L5075) composites
them and [`present_back_buffer`](../../services/sunlight-display/src/main.rs#L3653)
reaches the Limine, VMware SVGA or VirtIO GPU backend.

**Inferred design:** a separate guest Wayland service can own its listening
socket and dispatch loop and act as an SGP client. Map one `xdg_toplevel` to one
native window, copy each validated `wl_shm` commit into its SGP surface, and
translate SGP focus, resize and input in the opposite direction. A single host
window containing a nested desktop would also require layout and focus policy
inside the bridge. An in-process adapter would instead put the listener and
protocol dispatch into the display's [IPC loop](../../services/sunlight-display/src/main.rs#L8514);
no socket/IPC multiplexing for that design was found. The separate service
keeps the existing compositor path intact. Smithay guest buildability is
unverified. Searches of manifests, lockfile, display/UI/IPC code, tools and
documentation found no guest Smithay server or Wayland protocol entrypoint.
[`runs.sh`](../../tools/runs.sh#L267) uses the host display environment for
QEMU; a Linux-host compositor test does not validate the guest.

## Buffer transport: code checkpoints

| Checkpoint | Confirmed in source |
| --- | --- |
| Helios named listener | `socket(41)`, `bind(49)`, `listen(50)`, `accept(43)` / `accept4(288)` translate through [`translate_syscall`](../../compat-linux/src/lib.rs#L375) to [`sys_linux_socket`](../../kernel/src/arch/x86_64/syscall.rs#L9197), [`sys_linux_bind`](../../kernel/src/arch/x86_64/syscall.rs#L9249), [`sys_linux_listen`](../../kernel/src/arch/x86_64/syscall.rs#L9274) and [`sys_linux_accept`](../../kernel/src/arch/x86_64/syscall.rs#L9371). [`sys_linux_connect`](../../kernel/src/arch/x86_64/syscall.rs#L9301) looks up a listener and enqueues one end of a pipe-backed pair. Accept installs a descriptor for that end. |
| Native receiving endpoint | Native [`EndpointCreate`](../../kernel/src/arch/x86_64/syscall.rs#L2707) and `EndpointBind` operate on IPC capabilities, not socket FDs; receive uses native IPC. Native [`NativeSocketPair`](../../ipc/src/lib.rs#L105) creates two already connected FDs through the same [`create_socketpair_impl`](../../kernel/src/arch/x86_64/syscall.rs#L9499) as Linux socketpair. It cannot bind, listen, or accept a named connection. The native public ABI has no named socket lifecycle interface in the inspected syscall definitions and userspace wrappers. The [internal handler numbers 1050–1058](../../kernel/src/arch/x86_64/syscall.rs#L1317) are matched without a personality guard, so native raw dispatch appears able to enter those handlers. This is a source observation, not a supported native ABI or a tested route. Shared byte-stream backing is present; the supported named native listener is missing. |
| Standard FD handoff | Linux x86-64 `sendmsg(46)` and `recvmsg(47)` have no arms in [`translate_syscall`](../../compat-linux/src/lib.rs#L375). Its default is `SHIM_ENOSYS`; [`syscall_dispatch`](../../kernel/src/arch/x86_64/syscall.rs#L1010) translates this to internal 1005, which [returns Linux errno 38](../../kernel/src/arch/x86_64/syscall.rs#L1261). No `msghdr` is consumed and no FD is retained, queued, or installed. Searches of kernel, compat, IPC and native libc found no native `recvmsg` or `SCM_RIGHTS` implementation. `sendto`/`recvfrom` cover bytes, not ancillary data. This blocks even a Linux-to-Linux version of the requested test. |
| FD to shared pages | [`sys_linux_mmap`](../../kernel/src/arch/x86_64/syscall.rs#L10029) calls [`translate_mmap_flags`](../../compat-linux/src/lib.rs#L485), which rejects `MAP_SHARED` and requires private anonymous flags. The native [`sys_mmap` wrapper](../../kernel/src/arch/x86_64/syscall.rs#L12340) passes FD and offset to [`process::mmap::sys_mmap`](../../kernel/src/process/mmap.rs#L217), which rejects FD != -1 or nonzero offset before any backing lookup. A regular file FD therefore does not work either. [`sys_shm_alloc` and `sys_shm_map`](../../kernel/src/arch/x86_64/syscall.rs#L13050) can map the same pages by capability ([`map_shared_page`](../../kernel/src/memory/shared.rs#L223)); the [tagged Helios native calls](../../kernel/src/arch/x86_64/syscall.rs#L672) can reach that interface. That backing is not exposed as an FD here, and passing its token over IPC would test a different contract. |
| Connection authority | [`unix_socket::bind`](../../kernel/src/process/unix_socket.rs#L91) stores a pathname byte string in the global pool and checks for duplicates. [`find_listener`](../../kernel/src/process/unix_socket.rs#L140) checks path equality and listening state; `sys_linux_connect` checks the caller's socket and queue availability but does not check peer UID, directory permissions, or session identity. A process that can invoke these operations and knows the name can attempt to connect. VFS permissions on the runtime directory and `WAYLAND_DISPLAY` do not enforce access for this implementation. Native IPC endpoint rights are separate. |

## Minimal fixture design (pending prerequisites)

First expose a supported native listener and receive interface over the same
transport, with a connection authorization policy checked in the kernel. Add
the bounded ancillary FD path and an FD-backed shared mapping contract with
independent descriptor and mapping lifetimes. Evaluate a small shared-region
FD or suitable file backing before choosing any allocator API;
`memfd_create` is not a prerequisite in itself. Existing candidates are
`compat-linux/src/lib.rs`, `kernel/src/arch/x86_64/syscall.rs`,
`kernel/src/process/{unix_socket,fd_table,mmap}.rs`, native syscall wrappers
and, if reusing its backing, `kernel/src/memory/shared.rs`. No production
changes or fixture sources were added in this audit.

1. Run two separate processes in the guest: an `HLNX01` static Linux sender
   and a native receiver. Receiver creates, binds, listens and accepts; sender
   connects. Test that an unauthorized process cannot connect even when it
   knows the name.
2. Allocate one FD-backed page. Map it shared in the sender; write pattern A.
   Sender issues `sendmsg` with one control byte and exactly one
   `SOL_SOCKET/SCM_RIGHTS` descriptor. Receiver checks ancillary type,
   descriptor count and truncation status, maps the installed FD, and checks A.
3. Receiver acknowledges A. Sender writes pattern B, then sends a fixed
   coordination byte only. Receiver checks B by reading its *existing*
   mapping. Sender closes its FD, unmaps its view and exits after confirmation.
   Receiver maps afresh using its still-open FD and verifies B; then closes
   that FD and verifies the mapping remains readable until unmap.
4. Negative case: send one valid FD together with one invalid FD. Require an
   error with no partial delivery, no new receiver FD, and unchanged backing
   references after cleanup. Use fixture-only accounting for descriptors and
   object references; a failed syscall return alone does not rule out a leak.

Expected milestones are listener authorized, FD received, initial pattern
matched, mutation observed, sender gone with receiver reference valid, and
error cleanup verified. The first diagnostic is native listener/accept;
if that is present, check for `ENOSYS` on the first ancillary call, then
`EINVAL` at shared mapping. No expected milestone was observed in a guest.

## Follow-on milestones

1. Show a changing pattern from a guest Wayland client in a real SGP window,
   correlating configure/ack and a `wl_shm` commit with visual output. Check
   ELF startup, then the transport, then SGP presentation.
2. Run a verified guest Chromium binary outside SunApp with
   `--ozone-platform=wayland`, test input and resize, and verify renderer
   sandbox enforcement independently. The current [ELF parser](../../sunlight-elf/src/lib.rs#L212)
   rejects `PT_INTERP`; the ordinary dynamically linked Linux browser is
   development-host-only here. Guest dependencies/helper processes, Wayland
   connectivity, and sandbox support are separate gates. No guest Chromium
   executable or sandbox compatibility was found.
3. Package and repeat stage 2 through the launcher. The local
   [Btop bundle](../../Btop.sunapp/Manifest.toml) resolves via
   [`terminal_bundle_entry`](../../sunlight-libc/src/sun_exec.rs#L704) to
   `/Applications/Btop.sunapp/Program/btop`; the desktop route opens a PTY
   terminal and [the shell spawns it](../../sunshell/src/main.rs#L1453).
   This is neither a GUI runtime route nor an app sandbox. There is no
   bundle-specific cwd; the [kernel starts at root](../../kernel/src/process/mod.rs#L684)
   and [can inherit parent cwd on the VFS-routed spawn path](../../kernel/src/process/spawn.rs#L1000).
   A future GUI route should pass session-owned guest `XDG_RUNTIME_DIR` and
   `WAYLAND_DISPLAY` via [`launch_with_environment`](../../sunlight-libc/src/sun_exec.rs#L511),
   preserving credentials, entry validation and existing terminal/Chronos
   handling. Compare executable, argv, cwd, identity and environment to the
   passing standalone run first.

After the buffer prerequisite passes, the smallest display PR is one static
client and one guest bridge mapping one toplevel to one SGP window, with
visual proof of changing pixels and window removal on disconnect. Smithay,
Wayland/SGP implementation, Chromium and SunApp changes are outside this
buffer audit.

## Commands and result

- Checked source using `rg` and `sed` in the files linked above; the
  `SCM_RIGHTS|sendmsg|recvmsg` Rust source search returned no matches.
- `command -v qemu-system-x86_64 rustc` found both launchers. No build or
  QEMU boot command was run: the target contract is absent in source, so an
  ordinary boot would not exercise it.
- Result log: **blocked**; no guest serial output, no FD handoff, and no
  cross-process mapping observation. No Linux-to-Linux substitute was run.
