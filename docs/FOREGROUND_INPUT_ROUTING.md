# Foreground Process Input Routing Roadmap

## Problem Statement

Keyboard input is not routed to foreground processes. When you run long-running
interactive programs like `top` or `curl`, keyboard interrupts arrive at the
TTY server but are never forwarded to the process's stdin. This causes:

- Interactive programs can't receive input (e.g., pressing `q` to quit `top`)
- Users can't type commands while a foreground process runs
- The shell blocks completely until the foreground process exits
- Tab management breaks: you can't switch tabs or spawn new shells while a
  process runs in another tab
- Workaround: kill the app manually to regain control

## Current Architecture

```
Keyboard IRQ (IRQ1)
    ↓
Kernel keyboard handler
    ↓
TTY server IPC notification
    ↓
TTY server keyboard buffer
    ↓
[DEAD END - not forwarded to processes]
```

**What happens:**
- TTY server receives keyboard bytes via IPC from kernel
- In `Login` state: keys go to login form
- In `Shell` state: keys go to the shell prompt input buffer
- When a child process runs: keys still go to the parent shell's buffer, not
  to the child's stdin

**Why it fails:**
- No concept of "foreground process" in TTY server
- Process `fd 0` (stdin) is a placeholder `FileHandle(0)`, not connected to
  keyboard input
- Shell spawns child with pipe for stdout only, not stdin
- TTY doesn't know which process should receive keyboard input

## Required Architecture

```
Keyboard IRQ (IRQ1)
    ↓
Kernel keyboard handler
    ↓
TTY server IPC notification
    ↓
TTY server keyboard buffer
    ↓
TTY forwards to foreground_pid's stdin
    ↓
Process reads via sys_read(fd=0)
```

## Implementation Plan

### Phase 1: TTY Server Changes

**1.1 Add foreground process tracking**
- `services/tty_server/src/main.rs`: add `fg_pid: Option<u32>` to `ShellTab`
- When shell spawns a child, send IPC message to TTY: "set foreground pid=X"
- When child exits, send IPC message: "clear foreground"

**1.2 Add stdin buffer per tab**
- Create `stdin_buffer: VecDeque<u8>` in `ShellTab`
- When keyboard input arrives and `fg_pid.is_some()`, push bytes into
  `stdin_buffer` instead of shell prompt buffer

**1.3 Implement stdin read handler**
- TTY registers a new IPC operation: `TtyOp::ReadStdin { max_len }`
- When foreground process calls `read(0, ...)`, kernel routes it to TTY
- TTY drains `stdin_buffer` and returns bytes via IPC

**1.4 Add signal handling for Ctrl+C**
- Detect `0x03` (ETX) in keyboard stream
- Send `SIGINT` to `fg_pid` if present
- Requires kernel signal infrastructure (see Phase 3)

### Phase 2: Kernel Syscall Changes

**2.1 Wire stdin to TTY IPC**
- `kernel/src/process/fd_table.rs`: add new `FileHandle` variant for TTY stdin
- `kernel/src/arch/x86_64/syscall.rs`: in `sys_read`, detect TTY stdin handle
- Call TTY server via IPC: `ipc_call(tty_endpoint, TtyOp::ReadStdin)`
- Copy returned bytes to userspace buffer

**2.2 Create stdin handle on process spawn**
- When spawning a child for the shell, create `FileHandle::TtyStdin(tab_id)`
  for fd 0
- This connects the process's stdin to the TTY server's keyboard buffer

**2.3 Support blocking reads**
- If `stdin_buffer` is empty, TTY should block the IPC caller
- Use `ipc_reply_and_wait` pattern: don't reply until data is available
- Or return `EAGAIN` and let process poll (non-blocking mode)

### Phase 3: Shell Changes (sunshell)

**3.1 Notify TTY on child spawn**
- `sunshell/src/main.rs`: after `spawn_syscall()` succeeds, send IPC to TTY:
  `TtyOp::SetForeground { pid }`
- Track child PID and wait for exit

**3.2 Notify TTY on child exit**
- After `waitpid` or detecting child exit, send IPC:
  `TtyOp::ClearForeground`
- Resume normal shell prompt input

**3.3 Add job control commands**
- Implement `Ctrl+Z` to suspend foreground process (requires `SIGSTOP`)
- Implement `bg` and `fg` builtins for job management
- Track background jobs table

### Phase 4: Signal Infrastructure (future)

**4.1 Kernel signal support**
- Add signal mask, pending signals, and signal handlers to `Process` struct
- Implement `sys_kill`, `sys_signal`, `sys_sigaction` syscalls
- Deliver signals on syscall exit or timer interrupt

**4.2 Default signal handlers**
- `SIGINT` (Ctrl+C): terminate process
- `SIGSTOP` (Ctrl+Z): suspend process
- `SIGCONT`: resume process
- `SIGTERM`: graceful termination

**4.3 TTY signal delivery**
- When TTY detects Ctrl+C, call kernel: `kill(fg_pid, SIGINT)`
- When TTY detects Ctrl+Z, call kernel: `kill(fg_pid, SIGSTOP)`

### Phase 5: Tab Independence

**5.1 Per-tab stdin buffers**
- Each `ShellTab` has independent `fg_pid` and `stdin_buffer`
- Switching tabs doesn't affect other tabs' foreground processes

**5.2 Tab switch protocol**
- When user presses `F1`-`F12` to switch tabs, update TTY's `active_tab`
- Keyboard input routes to `tabs[active_tab].stdin_buffer` only

**5.3 Background process output**
- Background processes (not foreground) can still write to stdout
- Output is buffered or displayed in background until tab is active

### Phase 6: Process Lifecycle Integration

**6.1 Auto-cleanup on process exit**
- Kernel notifies TTY when foreground process exits
- TTY clears `fg_pid` and `stdin_buffer` for that tab

**6.2 Orphaned process handling**
- If shell exits while child runs, reparent child to `init`
- TTY clears foreground association for that tab

## Testing Plan

1. **Basic stdin forwarding**: Run `cat`, type text, see it echoed back
2. **Interactive quit**: Run `top`, press `q`, verify early exit
3. **Ctrl+C**: Run long sleep, press Ctrl+C, verify termination
4. **Tab switching**: Run `top` in tab 1, switch to tab 2, type commands, verify tab 1 still runs
5. **Multiple tabs**: Run `top` in 3 tabs simultaneously, verify independence
6. **Pipe stdin**: Test `echo "hello" | cat`, verify stdin comes from pipe not keyboard

## Migration Notes

- Existing processes with placeholder `FileHandle(0)` stdin will continue to
  get `EAGAIN` on read attempts (backward compatible)
- New processes spawned via shell will get `FileHandle::TtyStdin` and work
  properly
- `sunlight-top` auto-exit workaround can be removed once keyboard input works

## Priority

**High**: Phases 1-3 (enables basic interactive programs)  
**Medium**: Phase 4 (enables Ctrl+C and job control)  
**Low**: Phases 5-6 (multi-tab robustness)

## Estimated Effort

- Phase 1: 4-6 hours (TTY server refactor)
- Phase 2: 3-4 hours (kernel syscall plumbing)
- Phase 3: 2-3 hours (shell IPC integration)
- Phase 4: 8-12 hours (signal infrastructure from scratch)
- Phase 5: 2-3 hours (tab independence)
- Phase 6: 1-2 hours (cleanup logic)

**Total**: ~20-30 hours for full implementation

## References

- Current stdin placeholder: `kernel/src/process/fd_table.rs:128`
- Keyboard IRQ handler: `kernel/src/arch/x86_64/keyboard.rs`
- TTY keyboard handling: `services/tty_server/src/main.rs:194`
- Shell spawn logic: `sunshell/src/main.rs` (pipe creation)
- Syscall read handler: `kernel/src/arch/x86_64/syscall.rs:1412`
