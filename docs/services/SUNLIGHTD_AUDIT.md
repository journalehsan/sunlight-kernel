# sunlightd Audit Report
## Date: 2026-06-14
## Audited by: Claude Code (read-only pass)

## Executive Summary
sunlightd does not exist in any form in the current source tree: no crate, no directory, no binary, no embed, and no references in Rust code. It appears only in aspirational planning documents (PHASE6_5_USERLAND_PLAN.md and PHASE6_5_USERLAND_EXPECTATIONS.md) as a future "init daemon" responsible for zram/swap. 

The kernel (kernel/src/main.rs) performs all early boot process creation directly via hardcoded ELF embeds and Process construction for PID 1 (sunlight-init) and four core services. sunlight-init (services/init) is a minimal capability nameserver only; it registers the kernel's special SPAWN_TOKEN under the name "spawn" and then loops forever serving REGISTER/LOOKUP. There is no user-space service supervisor, no manifest/config reading, no process monitoring, no restarts, and no daemon lifecycle management of any kind.

All 15 capabilities in the required matrix are MISSING (or at best "STUB (planning document only)").

## Boot Spawn Chain
Exact sequence from kernel/src/main.rs (all direct, no helper spawn functions for core services):

1. Kernel embeds (lines ~42-60):
   - `INIT_ELF_BYTES`: `../../target/x86_64-unknown-none/release/sunlight-init` (services/init, bin name sunlight-init)
   - `TIMER_SERVER_ELF_BYTES`: `sunlight-timer-server`
   - `VFS_SERVER_ELF_BYTES`: `sunlight-vfs-server`
   - `TTY_SERVER_ELF_BYTES`: `sunlight-tty-server`
   - `NET_SERVER_ELF_BYTES`: `net_server`
   - (Also sunshell, sunlight-utils, sunlight-net-utils — used only for later exec, not boot daemons)

2. Spawn calls (all in kernel/src/main.rs _start, using Process::new + elf_loader::load_elf + manual stack pages + init_context + sched add_process):
   - `[PROC] Spawning init (pid=1)...` (approx line 245): `Process::new(1, 0, "init", ...)`; passes `capability::SPAWN_TOKEN.0` as initial arg0. Entry from INIT_ELF_BYTES.
   - `[PROC] Spawning vfs_server (pid=3)...` (line 283): `Process::new(3, 0, "vfs_server", ...)`; additionally maps FAT_SHARE_VADDR. Entry from VFS_SERVER_ELF_BYTES.
   - `[PROC] Spawning timer_server (pid=2)...` (line 339): `Process::new(2, 0, "timer_server", ...)`; Entry from TIMER_SERVER_ELF_BYTES. (Note: pid assignment is explicit and not in code order.)
   - `[PROC] Spawning tty_server (pid=4)...` (line 376): `Process::new(4, 0, "tty_server", ...)`; maps framebuffer; passes fb args. Entry from TTY_SERVER_ELF_BYTES.
   - `[PROC] Spawning net_server (pid=5)...` (line 488): `Process::new(5, 0, "net_server", ...)`; Entry from NET_SERVER_ELF_BYTES.

3. After scheduler handoff, no further core daemon spawns in kernel boot path.
   - The "spawn" capability (special SPAWN_TOKEN 0xCAFEBABE_DEADBEEF) is passed to init, which registers it by name. Later user processes (e.g. tty_server) look up "spawn" and send SpawnMsg::SPAWN over it.
   - Kernel fast-path in arch/x86_64/syscall.rs:395 (if token == SPAWN_TOKEN) calls handle_spawn_call → process::spawn::spawn_from_path (kernel/src/process/spawn.rs:189), which only knows how to spawn the embedded sunshell/utils binaries by hard-coded path → ELF mapping. This is used for interactive shells and PATH-resolved applets, not for "services" or daemons.

No call to sunlightd, no path "sunlightd", no embed for it, and no user-space supervisor ever receives control to launch anything.

## Capability Matrix

| Capability | Status | Evidence (file:line) |
|---|---|---|
| sunlightd binary exists in workspace | MISSING | No crate in root Cargo.toml members; no services/sunlightd/; `find` for "sunlightd" in *.rs returns zero matches; only two .md files mention the name (planning docs). |
| sunlightd registered with nameserver | MISSING | services/init/src/main.rs only ever registers "spawn" (from kernel token) plus whatever other services self-register ("vfs", "time", "tty", "net", "timed"). No sunlightd code or registration call exists. |
| sunlightd reads a service manifest / config | MISSING | No manifest parsing code anywhere. grep for ".service", "ExecStart", "Restart=", "WantedBy" in *.rs returns zero results. No .service/.socket/.toml unit files under etc/ or services/ (only user/passwd data in sunlight-fs/etc/). |
| sunlightd can spawn a service process | MISSING | Core services are spawned exclusively by kernel main.rs direct paths. User spawns go through kernel's SPAWN_TOKEN fastpath or spawn_from_path (only for shells/utils). No sunlightd binary or spawn logic. |
| sunlightd monitors if a service is alive | MISSING | No sunlightd. Kernel tracks ProcessState::Finished and increments PROCESS_FINISHED (kernel/src/sched/mod.rs:520), but no daemon supervisor consumes this. Init never calls waitpid or observes children. |
| sunlightd restarts a crashed service | MISSING | No restart policy, no supervisor loop, no "Restart=on-failure" anywhere. kernel notes finished procs but performs no auto-restart of any service. |
| sunlightd has a `start` / `stop` / `status` IPC interface | MISSING | No opcodes, no message types, no endpoint for sunlightd. Existing messages are InitMsg, TimerMsg, TimeMsg, VfsMsg, KbdMsg, SpawnMsg, NetOp, etc. (ipc/src/lib.rs). |
| sunlightd responds to `sunlightctl` or equivalent CLI | MISSING | No sunlightctl binary, no client code, no CLI crate. sunshell and sunlight-utils are multi-call applets for interactive use, not service control. |
| sunlightd supports dependency ordering (A before B) | MISSING | No dependency graph, no WantedBy/After/Before, no unit file parser. Boot order is a hardcoded sequence in kernel/src/main.rs. |
| sunlightd supports socket activation | MISSING | No socket activation code or "socket.activate" references (case-insensitive grep returned nothing). No .socket files. |
| sunlightd supports `.service` unit file format | MISSING | Zero matches for any systemd unit syntax in source (see Step 8 greps). No parser, no stub parser. |
| sunlightd supports `.socket` unit file format | MISSING | Same as above — no support present or stubbed. |
| sunlightd supports `Restart=on-failure` semantics | MISSING | No Restart= handling, no policy enum, no restart-on-exit logic in any process supervisor (none exists). |
| sunlightd logs service stdout/stderr | MISSING | Services use debug_log (which goes to kernel serial). No collection, no per-service log buffering or IPC exposure of stdout/stderr by a supervisor. tty_server captures shell output for display but is not general. |
| sunlightd exposes service status via IPC | MISSING | No status query messages, no service table, no equivalent of "systemctl status". nameserver only does cap lookup. |

## Init Service Analysis
From complete read of services/init/src/main.rs (and its Cargo.toml naming it "sunlight-init", bin "sunlight-init"):

- **What it does after being spawned**: Immediately logs "[ init] SunlightOS init process started" and "[ init] Waiting for system services to register...". Creates an endpoint (ep = endpoint_create()), registers the passed spawn_token (if !=0) under name "spawn" via the internal registry. Then enters an infinite `loop { msg = ipc_recv(ep); ... ipc_reply_and_wait }` that only understands InitMsg::REGISTER (stores name→cap) and InitMsg::LOOKUP (returns GRANT+cap or DENY). All other labels return DENY. It implements the sole system nameserver / capability directory. It never exits.

- **Does it launch sunlightd?** No. It never calls any spawn syscall, never looks up paths, never execs or spawns additional processes. The only "spawn" action is passively publishing the kernel-provided SPAWN_TOKEN cap so that later processes (e.g. tty_server at login) can request new interactive shells.

- **Does it stay running or exit after spawning?** It stays running forever as the nameserver. It performs no spawns of its own.

- **Does it handle SIGCHLD / process exit reaping?** No. There is no signal handling, no waitpid loop, no observation of Finished processes, and no reaping code in init. (Kernel has ProcessState::Finished, note_process_finished, a Waitpid syscall number, and some wait_child support in PCB for shells, but init never uses any of it. Reaping is effectively kernel-internal via slot reuse in sched.)

- **Does it register the nameserver or does the kernel do that?** Init itself is the nameserver. The kernel only gives it the special SPAWN_TOKEN at startup (via set_initial_args) so init can publish it. Kernel never calls REGISTER itself for the nameserver endpoint; the INIT_NAMESERVER_ENDPOINT (0) is a well-known constant that user code binds to via endpoint_bind.

In short: sunlight-init == PID 1 nameserver only. It is not a service manager or equivalent to systemd. The prompt correctly distinguishes "kernel-space init (services/init or sunlight-init)" from the (non-existent) user-space sunlightd.

## Currently Supervised Services

| Service name | Spawned by | Registered as | Status |
|---|---|---|---|
| sunlight-init (binary: sunlight-init) | kernel (kernel/src/main.rs ~245, direct embed + Process::new(1,...)) | "spawn" (the kernel SPAWN_TOKEN cap, registered by init itself) | REAL (PID 1 nameserver) |
| timer_server (binary: sunlight-timer-server) | kernel (kernel/src/main.rs ~339, direct embed + Process::new(2,...)) | "time" (self-register in timer_server/src/main.rs:18) | REAL (PID 2) |
| vfs_server (binary: sunlight-vfs-server) | kernel (kernel/src/main.rs ~283, direct embed + Process::new(3,...)) + extra FAT share mapping | "vfs" (self-register in vfs_server/src/main.rs:172) | REAL (PID 3) |
| tty_server (binary: sunlight-tty-server) | kernel (kernel/src/main.rs ~376, direct embed + Process::new(4,...)) + fb mapping + args | "tty" (self-register in tty_server/src/main.rs:175) | REAL (PID 4; also spawns user shells on demand) |
| net_server (binary: net_server) | kernel (kernel/src/main.rs ~488, direct embed + Process::new(5,...)) | "net" (self-register in net_server/src/main.rs:67) | REAL (PID 5) |
| timed (binary: timed) | nowhere at boot (present in workspace + Cargo.toml members, self-registers "timed" if launched) | "timed" (services/timed/src/main.rs:73) | MISSING from boot chain (never embedded or spawned by kernel or init) |
| sunlightd | nowhere | nowhere | MISSING (does not exist) |
| sunshell / sshl (and utils) | on-demand only (tty_server via SpawnMsg over "spawn" cap; kernel spawn_from_path) | per-tab name like "sshl<N>" (registered by the spawned shell itself) | REAL (user interactive processes, not boot-supervised daemons) |

No other services are launched at boot. install_sunlightos is an interactive installer binary, not auto-started.

## IPC Contract
**NO IPC INTERFACE — sunlightd is either a stub or operates without external control.**

No sunlightd-specific opcodes, message layouts, or endpoints exist in ipc/src/lib.rs or kernel. The only related "init" IPC is the nameserver protocol used by every service:

- InitMsg (ipc/src/lib.rs:120):
  - REGISTER = 1 : words[0] = name_to_u64(name), words[1] = endpoint_id (sent to init cap)
  - LOOKUP = 2 : words[0] = name_to_u64(name) → GRANT (words[0]=cap) or DENY
  - GRANT = 3, DENY = 4 (replies)

- IpcMsg is the fixed 80-byte struct (label, badge, word_count, cap_count, words[8], caps[2]).

- SpawnMsg (used by tty_server and kernel fastpath): SPAWN=1, REPLY=2, ERROR=3. Path packed in first 4 words; uid/gid in 4/5; reply carries pid.

- No "sunlightctl", no control client, no status or lifecycle messages.

All real services self-register with the init nameserver after creating their own endpoint and then serve their own protocol (VfsMsg, TimerMsg, TimeMsg, NetOp, KbdMsg, etc.).

## systemd Unit File Compatibility
Completely absent — no parsing, no stubs, no recognition of the syntax.

Exact command outputs (Step 8):
- `grep -r "\.service" . --include="*.rs" -l` → (no output)
- `grep -r "Restart=" . --include="*.rs"` → (no output)
- `grep -r "ExecStart" . --include="*.rs"` → (no output)
- `grep -r "WantedBy" . --include="*.rs"` → (no output)
- `grep -ri "socket.activate" . --include="*.rs"` → (no output)

No .service or .socket files exist in the tree (find for them under etc/ or services/ only surfaced unrelated Cargo/.conf files via path globbing). No fstab-style or toml service manifests are parsed for supervision (VFS does parse a simple /etc/fstab for mounts only).

## cargo check Result
```
warning: creating a mutable reference to mutable static
   --> services/tty_server/src/main.rs:21:9
    |
21 |         HEAP.as_mut_ptr().add(aligned)
   |         ^^^^^^^^^^^^^^^^^ mutable reference to mutable static
   |
   = note: mutable references to mutable statics are dangerous; it's undefined behavior if any other pointer to the static is used or if any other reference is created for the static while the mutable reference lives
   = note: for more information, see <https://doc.rust-lang.org/edition-guide/rust-2024/static-mut-references.html>

warning: creating a mutable reference to mutable static
   --> services/tty_server/src/main.rs:487:15
    |
487 |         match &mut GRID_CACHE {
   |               ^^^^^^^^^^^^^^^ mutable reference to mutable static
   |
   = note: mutable references to mutable statics are dangerous; it's undefined behavior if any other pointer to the static is used or if any other reference is created for the static while the mutable reference lives
   = note: for more information, see <https://doc.rust-lang.org/edition-guide/rust-2024/static-mut-references.html>
help: use `&raw mut` instead to create a raw pointer
    |
487 |         match &raw mut GRID_CACHE {
   |                +++

warning: creating a mutable reference to mutable static
   --> services/tty_server/src/main.rs:505:17
    |
505 |                 GRID_CACHE.as_mut().unwrap().as_mut()
   |                 ^^^^^^^^^^^^^^^^^^^ mutable reference to mutable static
   |
   = note: mutable references to mutable statics are dangerous; it's undefined behavior if any other pointer to the static is used or if any other reference is created for the static while the mutable reference lives
   = note: for more information, see <https://doc.rust-lang.org/edition-guide/rust-2024/static-mut-references.html>

warning: `sunlight-tty-server` (bin "sunlight-tty-server") generated 6 warnings
    Finished `dev` profile [optimized + debuginfo] target(s) in 0.20s
```
(No errors. Warnings are pre-existing in tty_server bump allocator / grid cache and unrelated to any sunlightd code, which does not exist.)

## Gap Analysis — What Must Be Built
Ordered from most critical (boot integration + basic supervision) to least:

1. No sunlightd crate/binary at all (must be added to workspace, built as x86_64-unknown-none user binary like the other services, embedded or loadable via VFS, and launched by kernel or by init).
2. No launch of sunlightd in the boot chain (kernel main.rs spawns only the five hardcoded services; init never spawns anything).
3. No service manifest / unit / config reader (no parser for toml, json, or systemd .service/.socket syntax; no representation of units, dependencies, or exec lines).
4. No supervisor state machine or loop inside a long-lived PID (no tracking of desired vs actual processes, no start/stop/restart operations).
5. No monitoring / liveness (no waitpid consumption for service children, no heartbeat/notify protocol, no use of kernel's Finished state by any user-space entity for services).
6. No restart semantics (no Restart=, no on-failure / always / never policy, no backoff).
7. No IPC control surface (no opcodes for start/stop/status/reload; no sunlightctl or equivalent; nameserver is only for cap lookup).
8. No dependency ordering / activation (no graph, no socket activation, no "WantedBy", no ordering of vfs before net etc. beyond kernel hardcode).
9. No stdout/stderr capture + logging per service (current debug_log is global serial; no per-unit log ring or query interface).
10. No exposure of service status / properties over IPC (clients cannot ask "is X running?").
11. No integration with existing SpawnMsg / capability model for supervised (vs. ad-hoc) spawns.
12. (Aspirational per docs) No zram/swap generator role, no virtio usage from user space under sunlightd.

The current architecture has a working nameserver (init) + direct kernel boot spawns + an on-demand spawn endpoint for interactive binaries. sunlightd would need to become a new PID (probably spawned by init after core services) that then takes over higher-level daemon lifecycle using the existing VFS, spawn cap, and IPC primitives.

## Recommended Implementation Phases
Split work to keep `./tools/test.sh` passing at each gate and to allow incremental boot changes:

- **sunlightd-phase0 / foundation (pre-boot impact)**: Add `services/sunlightd/` crate (Cargo.toml + stub main.rs that registers "sunlightd" with nameserver and enters an IPC loop). Add it to root workspace. Add a minimal embed in kernel (or load via VFS later). Update kernel boot to spawn it as e.g. pid=6 after net_server. Ensure `cargo check --workspace` and existing test gates still pass. (No real supervision yet.)

- **sunlightd-phase1 / minimal supervision**: Implement a trivial manifest (e.g. simple array of {name, path, restart?} in code or a tiny /etc/sunlight/services.toml read via VFS). sunlightd spawns listed services via the "spawn" cap (or direct Spawn syscall once wired). Basic start on boot. Add restart on exit (naive: always respawn Finished children it owns). Emit serial markers for test.sh gates.

- **sunlightd-phase2 / IPC + CLI surface**: Define SunlightdMsg (or extend) with START/STOP/STATUS/RESTART opcodes + simple request/reply using IpcMsg. Implement a sunlightctl (or add commands to sunshell / a new sunlight-utils applet) that looks up "sunlightd" and drives the interface. Expose service list + state.

- **sunlightd-phase3 / systemd-compat + policies**: Parse a subset of .service (at minimum ExecStart, Restart=, Type=simple). Add WantedBy / ordering (after=). Support socket activation stub (or real once sockets are richer). Add per-service stdout/stderr capture (pipe or shm) + queryable logs.

- **sunlightd-phase4 / production + swap role**: Full dependency solver, proper reaping + SIGCHLD-equivalent notifications, backoff, failure counting, integration with zram/virtio per the Phase 6.5 plan expectations (sunlightd as the zram allocator). Comprehensive status, enable/disable, and integration tests.

Each phase must keep the QEMU boot gate (`./tools/test.sh`) green and add new expected serial markers only when intentionally changing observable boot output. Use the existing pattern of phase* expected files in tools/tests/.

All work must remain #![no_std] + panic=abort for services, follow four-space Rust 2021 style, and avoid introducing warnings.

---

**End of read-only audit.** No code was written or modified during this pass. All findings are based on exhaustive file reads and the exact commands specified in the prompt.
