# Phase 6.2: Process Execution, Kernel Pipes & Scrollback UI — Deep Engineering Plan

**Status:** Ready for implementation (Session 2)  
**Estimated effort:** 2 sessions (process exec + pipes = Session 2; scrollback UI = Session 3)  
**Foundation:** Phase 6.1 complete (VT100 grid, 16KB buffers, ANSI colors)

---

## PHASE 6.2.A: KERNEL PIPES — Priority 1

### Why First?
- Process execution depends on working pipes for stdio redirection
- Scrollback UI is independent (can ship anytime)
- Pipes are the bottleneck

### Current State (from exploration)
- **File:** `kernel/src/process/pipe.rs:4` — `PIPE_BUFFER_SIZE = 4096` bytes
- **Status:** Non-blocking stub (`read()/write()` return 0 immediately, no actual blocking)
- **Issue:** Ring buffer allocated but never connected to actual `Pipe` instances; no wakeup mechanism

### luxOS Reference (Inspection Notes)
- luxOS **does not** have working pipes (stub implementation like SunlightOS)
- Instead: luxOS uses **direct message passing** via IPC channels for parent→child communication
- **Key insight:** SunlightOS's microkernel IPC (8-word/64-byte messages) is the pipe equivalent
- **Lesson:** Don't copy luxOS pipes; instead, optimize SunlightOS's IPC for bulk data transfer

---

## PHASE 6.2.B: PROCESS EXECUTION — Priority 2 (depends on pipes)

### Current State
- **File:** `kernel/src/process/spawn.rs:21` — `spawn_from_path()` only supports embedded ELF (`include_bytes!`)
- **File:** `kernel/src/arch/x86_64/syscall.rs:541` — `sys_exec()` is a stub (returns `u64::MAX`)
- **File:** `kernel/src/process/fd_table.rs:85` — fd 0/1/2 initialized but hardcoded to serial, not pipes
- **Missing:** argv/envp marshalling, address space setup, privilege dropping

### luxOS Reference
- Process spawn: ELF loaded into fresh `AddressSpace`
- Stdin/stdout: routed to parent's pipe handles (if shell redirects)
- Privilege model: root → user on spawn (via `uid`/`gid` fields in `Process`)
- **Key insight:** Shell doesn't need to know about kernel details; just pass child_pid + pipe caps

### Implementation Order
1. **Step 1:** Implement real `sys_pipe()` — allocate `Pipe`, return (read_fd, write_fd)
2. **Step 2:** Implement fd duplication (`dup2()` or capability grants) — map process fd table entries to pipe ring buffers
3. **Step 3:** Implement `sys_read()` / `sys_write()` for pipes — blocking semantics on ring buffer
4. **Step 4:** Implement `sys_exec()` — load ELF, set up argv/envp, redirect stdin/stdout via fd table
5. **Step 5:** Wire shell to call `sys_exec()` + handle stdio redirection

---

## PHASE 6.2.C: SCROLLBACK UI — Priority 3 (independent)

### Current State
- **File:** `services/tty_server/src/main.rs:48` — `TERM_OUTPUT_MAX = 4096` (Phase 6.1 bump)
- `ShellTab.output` is a flat buffer; no history tracking beyond current screen
- **Keyboard input:** only handles ASCII keys, no special keys (PageUp/Down, arrows)

### Requirements
1. **Per-tab ring buffer** — store last N screen lines (suggest 256 lines = 256 KB per tab, 10 tabs = 2.5 MB)
2. **Viewport control** — track which row is at screen top (scrollback offset)
3. **Keyboard routing** — intercept PageUp/PageDown, scroll viewport
4. **Rendering** — adjust which lines are displayed based on viewport offset

### Files to Modify
- `services/tty_server/src/main.rs` — add scrollback ring buffer, viewport offset, keyboard handler
- `sunlight-tui/src/lib.rs` — adjust `render_terminal_grid()` to accept viewport offset, render from scrollback
- Keyboard event handler — add special-key decoding (requires `unpack_key_event` enhancement or new decoding)

---

## DETAILED IMPLEMENTATION GUIDE

### Phase 6.2A: Pipes (Session 2A — 2-3 hours)

#### Step 1: Real `sys_pipe()` Implementation
**File:** `kernel/src/process/pipe.rs`

```rust
// Add to Pipe struct
pub struct Pipe {
    buffer: [u8; PIPE_BUFFER_SIZE],
    read_pos: usize,
    write_pos: usize,
    data_len: usize,
    read_blocked: Vec<ProcessId>,  // processes waiting on read
    write_blocked: Vec<ProcessId>, // processes waiting on write
}

// Implement blocking semantics
impl Pipe {
    pub fn read_blocking(&mut self, pid: ProcessId, buf: &mut [u8]) -> Result<usize> {
        if self.data_len == 0 {
            // Block this process, add to read_blocked
            self.read_blocked.push(pid);
            return Err(EAGAIN);  // Kernel will reschedule
        }
        // Copy data, advance read_pos
        let n = (self.data_len).min(buf.len());
        // ... copy from ring buffer
        self.data_len -= n;
        // Wake any write_blocked processes
        for pid in self.write_blocked.drain(..) {
            reschedule(pid);
        }
        Ok(n)
    }
    
    pub fn write_blocking(&mut self, pid: ProcessId, buf: &[u8]) -> Result<usize> {
        let avail = PIPE_BUFFER_SIZE - self.data_len;
        if avail == 0 {
            self.write_blocked.push(pid);
            return Err(EAGAIN);
        }
        // Copy data, advance write_pos
        let n = buf.len().min(avail);
        // ... copy to ring buffer
        self.data_len += n;
        // Wake any read_blocked processes
        for pid in self.read_blocked.drain(..) {
            reschedule(pid);
        }
        Ok(n)
    }
}
```

**File:** `kernel/src/arch/x86_64/syscall.rs`

```rust
// Replace sys_pipe stub
fn sys_pipe(fd_table: &mut FdTable) -> u64 {
    // Allocate global pipe pool entry
    let pipe = Pipe::new();
    let pipe_id = PIPE_POOL.insert(pipe);
    
    // Allocate two fd table entries: read (fd=N) and write (fd=N+1)
    let read_fd = fd_table.alloc_entry(FileHandle::Pipe(pipe_id, READ));
    let write_fd = fd_table.alloc_entry(FileHandle::Pipe(pipe_id, WRITE));
    
    // Return as a u64: (read_fd << 32 | write_fd)
    ((read_fd as u64) << 32) | (write_fd as u64)
}

fn sys_read(fd: i32, buf: *mut u8, count: usize, pid: ProcessId) -> u64 {
    let entry = fd_table.get(fd as usize)?;
    match entry {
        FileHandle::Pipe(pipe_id, READ) => {
            let pipe = PIPE_POOL.get_mut(pipe_id)?;
            match pipe.read_blocking(pid, from_user_ptr(buf, count)) {
                Ok(n) => n as u64,
                Err(EAGAIN) => {
                    // Kernel reschedules pid automatically
                    0
                }
            }
        }
        _ => Err(EINVAL),
    }
}

fn sys_write(fd: i32, buf: *const u8, count: usize, pid: ProcessId) -> u64 {
    let entry = fd_table.get(fd as usize)?;
    match entry {
        FileHandle::Pipe(pipe_id, WRITE) => {
            let pipe = PIPE_POOL.get_mut(pipe_id)?;
            match pipe.write_blocking(pid, from_user_ptr(buf, count)) {
                Ok(n) => n as u64,
                Err(EAGAIN) => 0,
            }
        }
        _ => Err(EINVAL),
    }
}
```

**Verification:**
- Build: `cargo build --package sunlight-kernel`
- Run basic test: `./tools/test.sh` (check for pipe-related gates if any)
- Manual test (next phase): try `sysfetch | wc -l` in shell (once exec is ready)

---

#### Step 2: `sys_exec()` Implementation
**File:** `kernel/src/process/spawn.rs`

```rust
// Replace sys_exec stub
pub fn sys_exec(
    path: &[u8],
    argv: &[*const u8],  // array of pointers to arg strings
    envp: &[*const u8],  // array of pointers to env vars
    current_pid: ProcessId,
) -> Result<u64> {
    // 1. Load ELF from VFS (path)
    let elf_bytes = vfs_read_file(path)?;
    let elf = ElfParser::parse(&elf_bytes)?;
    
    // 2. Get current process's address space
    let proc = PROCESS_TABLE.get_mut(current_pid)?;
    
    // 3. Tear down old address space, build new one
    proc.address_space = AddressSpace::new();
    
    // 4. Load ELF segments into new address space
    for segment in elf.load_segments() {
        let vaddr = segment.vaddr();
        let memsz = segment.memsz();
        let filesz = segment.filesz();
        
        // Allocate pages, copy file data
        let pages = alloc_pages((memsz + 4095) / 4096);
        copy_from_user(&segment.data[..filesz], pages as *mut u8);
        
        proc.address_space.map(vaddr, pages, memsz, segment.perms());
    }
    
    // 5. Setup stack with argv/envp (x86_64 calling convention)
    let stack_top = setup_stack(argv, envp, &mut proc.address_space)?;
    
    // 6. Set entry point and initial stack pointer
    proc.regs.rip = elf.entry_point() as u64;
    proc.regs.rsp = stack_top;
    
    // 7. Preserve current uid/gid (don't privilege-drop yet; shell handles that)
    // proc.uid, proc.gid unchanged
    
    Ok(elf.entry_point() as u64)
}

// Helper: marshal argv/envp onto stack
fn setup_stack(argv: &[*const u8], envp: &[*const u8], as: &mut AddressSpace) -> Result<u64> {
    let stack_base = 0x7fff_0000u64;  // typical user stack base
    let stack_size = 0x1000;  // 4KB stack
    as.map(stack_base, alloc_pages(1), stack_size, RW);
    
    let mut sp = stack_base + stack_size - 8;  // 8-byte aligned
    
    // Push envp array (null-terminated)
    for &env_ptr in envp {
        copy_to_user(sp as *mut *const u8, &env_ptr);
        sp -= 8;
    }
    copy_to_user(sp as *mut *const u8, &(0 as *const u8));  // NULL terminator
    sp -= 8;
    let envp_addr = sp;
    
    // Push argv array (null-terminated)
    for &arg_ptr in argv {
        copy_to_user(sp as *mut *const u8, &arg_ptr);
        sp -= 8;
    }
    copy_to_user(sp as *mut *const u8, &(0 as *const u8));  // NULL terminator
    sp -= 8;
    let argv_addr = sp;
    
    // Push argc
    copy_to_user(sp as *mut usize, &(argv.len()));
    sp -= 8;
    
    // RDI=argc, RSI=argv, RDX=envp (x86_64 SysV ABI)
    // (set in process regs before returning)
    
    Ok(sp)
}
```

**File:** `kernel/src/arch/x86_64/syscall.rs`

```rust
// Update syscall dispatch to call sys_exec
31 => {  // EXEC
    let path_ptr = regs.rdi as *const u8;
    let argv_ptr = regs.rsi as *const *const u8;
    let envp_ptr = regs.rdx as *const *const u8;
    
    match sys_exec(from_user_slice(path_ptr), from_user_array(argv_ptr), from_user_array(envp_ptr), current_pid) {
        Ok(entry) => entry,
        Err(e) => (-(e as i64)) as u64,
    }
}
```

**Verification:**
- Build: `cargo build --package sunlight-kernel`
- Test in session 2B

---

### Phase 6.2B: Scrollback UI (Session 3 — 2-3 hours)

#### File: `services/tty_server/src/main.rs`

Add to `ShellTab`:

```rust
struct ShellTab {
    // ... existing fields ...
    
    // Scrollback ring buffer: last N lines of output
    scrollback: Vec<Vec<u8>>,  // Vec of lines
    scrollback_max: usize,      // cap at 256 lines
    viewport_offset: usize,     // which line is at screen top (0 = latest)
}

impl ShellTab {
    fn new() -> Self {
        Self {
            // ... existing init ...
            scrollback: Vec::with_capacity(256),
            scrollback_max: 256,
            viewport_offset: 0,
        }
    }
    
    // Called when output buffer is full/wraps; save line to scrollback
    fn save_to_scrollback(&mut self, line: &[u8]) {
        if self.scrollback.len() >= self.scrollback_max {
            self.scrollback.remove(0);  // drop oldest
        }
        self.scrollback.push(line.to_vec());
        self.viewport_offset = 0;  // reset viewport to latest
    }
    
    // PageUp: scroll up in history
    fn scroll_up(&mut self) {
        if self.viewport_offset < self.scrollback.len().saturating_sub(1) {
            self.viewport_offset += 1;
        }
    }
    
    // PageDown: scroll down toward latest
    fn scroll_down(&mut self) {
        self.viewport_offset = self.viewport_offset.saturating_sub(1);
    }
    
    // Get the line to display at screen row N
    fn get_display_line(&self, screen_row: usize) -> &[u8] {
        let history_row = self.scrollback.len().saturating_sub(self.viewport_offset + screen_row + 1);
        if history_row < self.scrollback.len() {
            &self.scrollback[history_row]
        } else {
            &[]  // blank line if no history
        }
    }
}
```

#### File: Keyboard Handler

In main loop, when handling `KBD_LABEL`:

```rust
// After unpacking key event
match ascii {
    Some(0x21) if shift => {  // Shift+! = PageUp (depends on keyboard layout)
        if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
            tab.scroll_up();
        }
        needs_render = true;
    }
    Some(0x3F) if shift => {  // Shift+? = PageDown
        if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
            tab.scroll_down();
        }
        needs_render = true;
    }
    // ... existing key handling ...
}
```

#### File: `sunlight-tui/src/lib.rs`

Update `render_terminal_grid()` signature:

```rust
pub unsafe fn render_terminal_grid(
    fb_addr: *mut u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    tab_count: usize,
    active_tab: usize,
    cols: usize,
    rows: usize,
    cells: &[TermCell],
    cursor_row: usize,
    cursor_col: usize,
    input_line: &[u8],
    prompt: &[u8],
    viewport_offset: usize,  // NEW: scrollback position
) {
    // ... existing chrome rendering ...
    
    // When rendering grid lines:
    for screen_row in 0..rows {
        let history_row = screen_row + viewport_offset;
        let cells_for_row = if history_row < cells.len() {
            &cells[history_row * cols..(history_row + 1) * cols]
        } else {
            &[]
        };
        
        // Draw cells_for_row to screen
        for (col, cell) in cells_for_row.iter().enumerate() {
            // ... existing cell rendering ...
        }
    }
    
    // Draw scrollback indicator (e.g., "↑ 42 lines back ↓" in footer)
    if viewport_offset > 0 {
        let indicator = b"[SCROLLBACK]";
        tty_draw_line(&mut fb, MARGIN, footer_text_y, indicator, layout::palette::TEXT_DIM, 1);
    }
}
```

**Verification:**
- Build: `cargo build --package sunlight-tty-server`
- Test boot: `./tools/run.sh --build --screenshot`
- Manual test: run a long command, press PageUp (keyboard intercept TBD based on your layout)

---

## Testing Strategy

### Session 2 (Pipes + Exec)
1. Build and verify no compiler errors
2. Add basic unit tests in `kernel/src/process/pipe.rs` (test ring buffer without blocking)
3. Run `./tools/test.sh` (check existing gates don't break)
4. Manual test: `sysfetch | head -5` in shell (once shell calls sys_exec)

### Session 3 (Scrollback)
1. Build and verify no compiler errors
2. Manually test: run a 100-line command, press PageUp to scroll
3. Verify PageDown returns to latest
4. Verify rendering doesn't crash on edge cases (empty scrollback, etc.)

---

## Files to Modify (Checklist)

**Pipes & Exec:**
- [ ] `kernel/src/process/pipe.rs` — blocking ring buffer + wakeup
- [ ] `kernel/src/process/spawn.rs` — real sys_exec with ELF loader + stack setup
- [ ] `kernel/src/arch/x86_64/syscall.rs` — sys_pipe, sys_read, sys_write implementations
- [ ] `kernel/src/process/fd_table.rs` — extend FileHandle enum for pipes

**Scrollback UI:**
- [ ] `services/tty_server/src/main.rs` — add scrollback ring buffer, viewport offset, keyboard handler
- [ ] `sunlight-tui/src/lib.rs` — update render_terminal_grid to accept viewport_offset
- [ ] Keyboard handler — intercept PageUp/PageDown and map to scroll_up/scroll_down

---

## Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| Pipes block indefinitely on full buffer | Implement timeout + force-wake in scheduler |
| Stack overflow during exec | Validate stack size in setup_stack(), use guard pages |
| Viewport offset causes OOB access | Add bounds checks in get_display_line() |
| Keyboard event decoding fails | Start with PageUp=a, PageDown=z for testing; refine later |

---

## Success Criteria

**Phase 6.2A (Pipes):**
- ✅ `sysfetch | wc -l` runs without kernel panic
- ✅ Output of piped command matches expected (e.g., 30 lines)
- ✅ No deadlock (processes unblock correctly)

**Phase 6.2B (Exec):**
- ✅ Shell can spawn external binaries (not just built-in commands)
- ✅ Argv/envp passed correctly (child sees correct args)
- ✅ Stdio redirection works (child's write goes to parent's read end)

**Phase 6.2C (Scrollback):**
- ✅ PageUp scrolls up, PageDown scrolls down
- ✅ Oldest lines are discarded after 256 lines
- ✅ Viewport resets to latest after command execution

---

## Next After Phase 6.2

**Phase 6.3:** ioctl/termios support (enable htop, interactive TUI apps)  
**Phase 6.4:** Multi-pane support (split terminals, tmux-like)  
**Phase 7.0:** Window Manager + X11 compatibility layer

---

**Ready to execute in Session 2. Start with Phase 6.2A (Pipes). Good luck!**
