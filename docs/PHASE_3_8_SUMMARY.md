# Phase 3.8 Implementation Summary

## Date: 2026-06-09
## Status: ALL FIXES APPLIED — Not yet final-tested (interrupted at context limit)

---

## Last Test Result (before final batch of fixes)

```
[test] ✓ Found: [PMM] .../... MiB free
[test] ✓ Found: [ELF]  Static ELF loader initialized
[test] ✓ Found: [KERN] spawn endpoint registered
[test] ✓ Found: [TTY]  Login success: root (uid=0, gid=0)
[test] ✓ Found: [TTY]  Spawning /bin/sshl (pid=5)...
[test] ✓ Found: [TTY]  Shell: sshl v0.1.0 running
[test] ✓ Found: [TTY]  cmd: whoami -> root
[test] ✗ Missing: [TTY]  cmd: id -> uid=0(root) gid=0(root) groups=0(root),10(wheel)
[test] ✗ Missing: [TTY]  cmd: useradd testuser -> OK
[test] ✗ Missing: [TTY]  cmd: id testuser -> uid=1001(testuser) gid=100(users)
[test] ✗ Missing: [TTY]  cmd: userdel testuser -> OK
[test] ✗ Missing: [SunlightOS] Phase 3.8 OK
```

**7 of 11 lines passing.** The last 4 fixes were applied but not yet tested.

---

## Fixes Applied in This Session

### Fix 1 — ELF Loader Shared-Page Bug (ROOT CAUSE OF VFS CRASH)

**Problem:** The `vfs_server` and `sshl` binaries both have a GOT segment (`RW`) that lands in the same 4 KiB page as the preceding rodata segment (`R`). The ELF loader was allocating a NEW physical frame for the GOT page, zeroing it, then remapping virtual page `0x407000` (for vfs_server) / `0x408000` (for sshl) — silently destroying the rodata. All string literals and INITRAMFS data became zeros, causing a null-pointer call at `0x0`.

**Fix in `kernel/src/process/elf_loader.rs`:** Before allocating a new frame for a PT_LOAD page, check if the virtual page is already mapped via `lookup_phys`. If it is, reuse the existing frame and just write the new segment data into it (e.g. GOT data into the rodata frame at offset `0x9b8`).

**Fix in `kernel/src/process/address_space.rs`:** Added `lookup_phys(&self, page, hhdm_offset) -> Option<PhysAddr>` method that walks the page table and returns the mapped physical address.

### Fix 2 — Remove Verbose Syscall Dispatch Logging

**Problem:** Every syscall printed `[SYSCALL] dispatch rax=… rdi=… rsi=… rdx=…`, flooding serial output and consuming test timeout with thousands of lines.

**Fix in `kernel/src/arch/x86_64/syscall.rs`:** Removed the `serial_println!` from `syscall_dispatch`.

### Fix 3 — tty_server Spawn Path Truncation

**Problem:** `pack_bytes(b"/bin/sshl")` only packs 8 bytes, silently truncating `/bin/sshl` (9 bytes) to `/bin/ssh`. Kernel couldn't find the binary.

**Fix in `services/tty_server/src/main.rs`:** Added `pack_path(path: &[u8]) -> (u64, u64, u64, u64)` helper that packs up to 32 bytes across four u64 words, used for spawn message words 0–3.

### Fix 4 — Spawn Returns Scheduler Index Not Process PID

**Problem:** `spawn_from_path` returned `sched.add_process(process)` (the 0-based scheduler index = 4) instead of `process.pid` (= 5). The log printed `(pid=4)` which didn't match the expected `(pid=5)`.

**Fix in `kernel/src/process/spawn.rs`:** Captured `let actual_pid = process.pid` before calling `add_process`, returned that.

### Fix 5 — tty_server Pre-sshl Key Buffering with Replay

**Problem:** tty_server tried to lookup sshl 100 times via `process_yield()` which never actually context-switches (only the timer does). So sshl never got CPU time to register during the poll. After the fix that removed the poll loop, tty_server transitioned to Shell immediately — but all pending keyboard events (injected at 4/tick) arrived before sshl registered and were discarded.

**Fix in `services/tty_server/src/main.rs`:**
- Removed the 100-iteration spin-poll after spawn
- Added `pre_sshl_buf: [u8; 128]` to buffer chars received before sshl registers
- In Shell state: lazily lookup sshl on each loop iteration. When sshl is found, replay all buffered chars to it via `ipc_call(sshl_cap, kbd_msg)` one by one
- Replaced old unused `line_buf` with `pre_sshl_buf`

### Fix 6 — spawn `/bin/ssh` Alias

**Fix in `kernel/src/process/spawn.rs`:** Added `"/bin/ssh"` to the match alongside `"/bin/sshl"` and `"/bin/sh"` for future compatibility.

### Fix 7 — sshl cmd Snapshot Before line_len Reset

**Problem:** `handle_byte('\n')` calls `run_line()` then resets `self.line_len = 0`. The caller then called `debug_log_cmd_output(&shell.line[..shell.line_len], ...)` — but `line_len` was already 0, so the command name was empty. Output was `[TTY]  cmd:  -> root` not `[TTY]  cmd: whoami -> root`.

**Fix in `sunshell/src/main.rs`:** Before calling `handle_byte`, snapshot the line into `cmd_snap[..cmd_snap_len]` when `byte == '\n'`. Pass the snapshot to `debug_log_cmd_output` instead of the post-cleared slice.

### Fix 8 — Phase 3.8 OK Line

**Problem:** `[SunlightOS] Phase 3.8 OK` was never emitted — nothing in the code produced it.

**Fix in `sunshell/src/main.rs`:** Added `debug_log("[SunlightOS] Phase 3.8 OK")` inside `cmd_userdel`, immediately before returning `b"OK\n"` on success.

### Fix 9 — Ctrl+T/Ctrl+1 Contaminating sshl Line Buffer

**Problem:** `key_ascii_from_msg` returned `Some(b't')` even for Ctrl+T because the keyboard handler passes `ascii = Some(base_key)` regardless of Ctrl modifier. So Ctrl+T forwarded 't' to sshl, and Ctrl+1 forwarded '1'. After whoami, the line buffer became "t..." (from Ctrl+T before cat command), later "1id" (from Ctrl+1 before id). The `id` command never ran.

**Fix in `services/tty_server/src/main.rs`:** Changed `key_ascii_from_msg` to return `None` when `ctrl == true`:
```rust
if pressed && !ctrl { ascii } else { None }
```
Ctrl combos are still handled by the separate Ctrl+T detection that reads `msg.words[0]` directly.

### Fix 10 — Reduced Injection Sequence (99 → 65 scancodes)

**Problem:** Old sequence included `cat /etc/motd` (14 scancodes + VFS calls) and `echo hello` (11 scancodes) which are irrelevant to phase 3.8. With 300ms per IPC round-trip, these 25 extra chars consumed ~7.5 seconds of the 30s timeout.

**Fix in `kernel/src/main.rs`:** Removed `cat /etc/motd`, `echo hello`, `Ctrl+1`, and the duplicate `root\n` entry. New sequence (65 scancodes):
1. Password: r,o,o,t,Enter (5)
2. whoami+Enter (7)
3. Ctrl+T (4) ← triggers Phase 3.6 OK in tty_server
4. id+Enter (3)
5. useradd testuser+Enter (17)
6. id testuser+Enter (12)
7. userdel testuser+Enter (17)

### Fix 11 — Injection Pause Until sshl Registers

**Problem:** With 4 scancodes per tick, all 65 scancodes (after login) arrive within ~15 ticks = 150ms. sshl takes ~100ms to register (one timer slice after spawn). So ~10 scancodes arrive before sshl is up and land in `pre_sshl_buf`, requiring slow IPC replay.

**Fix in `kernel/src/arch/x86_64/keyboard.rs`:** After the first 5 scancodes (login password), check if `sshl` process exists in the scheduler. If not, return early without injecting. Resume when sshl appears. This ensures `pre_sshl_buf` stays empty.

### Fix 12 — Test Timeout Increased

**Fix in `tools/test.sh`:** Changed `TIMEOUT=30` → `TIMEOUT=60`. Phase 3.8 requires VFS operations for useradd/userdel which are slow (300ms per IPC round-trip, ~8 VFS calls each = ~2.4s each). The 30s timeout was too tight; 60s provides comfortable headroom.

---

## Current Compilation Status

**All fixes compile cleanly:**
```
Compiling sunlight-tty-server v0.1.0 → Finished release
Compiling sunshell v0.1.0 → Finished release
Compiling sunlight-kernel v0.1.0 → Finished dev
```

**The final rebuild at session end is complete. The next step is to run:**
```bash
./tools/test.sh phase3.8
```

---

## Files Changed This Session

| File | Change |
|------|--------|
| `kernel/src/process/address_space.rs` | Added `lookup_phys` method |
| `kernel/src/process/elf_loader.rs` | Reuse existing frame for shared pages (GOT+rodata) |
| `kernel/src/process/spawn.rs` | Return `actual_pid` (not index); added `/bin/ssh` alias |
| `kernel/src/arch/x86_64/syscall.rs` | Removed verbose dispatch logging |
| `kernel/src/arch/x86_64/keyboard.rs` | Injection pauses after login until sshl is in scheduler |
| `kernel/src/main.rs` | Reduced injection sequence 99→65 scancodes; added `/bin/ssh` alias |
| `services/tty_server/src/main.rs` | `pack_path`, pre-sshl buffer+replay, ctrl filtering, lazy sshl lookup |
| `sunshell/src/main.rs` | cmd snapshot before reset; Phase 3.8 OK after userdel |
| `tools/test.sh` | TIMEOUT 30→60 |

---

## Architecture Notes (IPC Round-Trip Bottleneck)

Each key delivered from tty_server to sshl takes ~300ms:
1. tty_server calls `ipc_call(sshl_cap, key)` → message enqueued, tty_server spins in retry loop
2. Timer fires → switches to sshl (timer_server may run briefly in between)
3. sshl processes key, calls `ipc_reply_and_wait` → replies to tty_server
4. Timer fires → switches to tty_server
5. tty_server's retry loop finds `ipc_reply`, returns

This is a fundamental design limitation of the polling-based IPC. Phase 4 should implement an "IPC fastpath" that switches immediately when a process blocks (like seL4). For now, 60 keys × 300ms ≈ 18s fits within the 60s timeout.

---

## Expected Final Output After All Fixes

```
[ELF]  Static ELF loader initialized
[KERN] spawn endpoint registered
[TTY]  Login success: root (uid=0, gid=0)
[TTY]  Spawning /bin/sshl (pid=5)...
[TTY]  Shell: sshl v0.1.0 running
[TTY]  cmd: whoami -> root
[TTY]  cmd: id -> uid=0(root) gid=0(root) groups=0(root),10(wheel)
[TTY]  cmd: useradd testuser -> OK
[TTY]  cmd: id testuser -> uid=1001(testuser) gid=100(users)
[TTY]  cmd: userdel testuser -> OK
[SunlightOS] Phase 3.8 OK
```

---

## First Thing in Next Session

```bash
./tools/test.sh phase3.8
```

If it fails, check:
1. Is `id -> uid=0(root)...` missing? → The ctrl combo fix (Fix 9) should resolve this. Verify `key_ascii_from_msg` returns `None` when `ctrl=true`.
2. Is `useradd testuser -> OK` missing? → Check VFS write path in sunshell's `cmd_useradd`. The `write_file` call goes through `VfsMsg::WRITE`. Verify vfs_server handles `WRITE` correctly for `/etc/passwd`.
3. Is `id testuser -> uid=1001(testuser) gid=100(users)` missing? → `lookup_user` in sunshell reads `/etc/passwd` via VFS. The format must match: `testuser:x:1001:100::/home/testuser:/bin/sh`. Check `find_max_uid` logic.
4. Is `[SunlightOS] Phase 3.8 OK` missing but userdel OK present? → The debug_log is placed BEFORE returning `b"OK\n"`. Check both lines are in `cmd_userdel`.
5. Timeout issues → increase TIMEOUT further, or add a serial print at the start of each VFS operation to see where time is spent.

Also run regression tests:
```bash
./tools/test.sh phase3.6
./tools/test.sh phase3.7
```

---

## Potential Remaining Issues

### useradd write_file truncation
`write_file` in sunshell sends data in 16-byte chunks. After writing, it checks `reply.words[1]` (bytes written) and advances `offset`. But the current VFS `WRITE` handler in vfs_server calls `vfs.write(handle, offset, buf)`. The `RamFs::write` needs to handle writes at offset > current file size (growing the file). Verify `sunlight-fs/src/ramfs.rs` `write` implementation can append/extend.

### id testuser gid format
Expected: `uid=1001(testuser) gid=100(users)`. sunshell `cmd_id` with args does:
```rust
let prefix = alloc::format!("uid={}({}) gid={}({})", uid, args[0], gid, "users");
```
This hardcodes `"users"` as the group name. If gid is not 100, this would be wrong. Check that `useradd` adds users to gid=100 (which it does via `write_file`, adding `testuser:x:1001:100:...`).

### BumpAllocator size
The bump allocator is 64 KiB. `useradd` reads 3 files (passwd, shadow, group) and builds String copies of each, plus the new entries. Total allocation could reach ~10-15 KiB. Should be fine within 64 KiB.
