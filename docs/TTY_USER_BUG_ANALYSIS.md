# TTY User Propagation Bug Analysis

## The Problem

When user logs in as "user" through TTY, the spawned shell still shows "root" for `whoami`/`id`.

## Root Cause

**The username is lost between login and shell spawn.**

### Current Flow (BROKEN)

```
services/tty_server/src/main.rs:
├─ TtyState::Login handles keyboard input
├─ login.handle_key_ascii(ascii)
├─ Returns LoginResult::Success { username, username_len, uid, gid }  ← HAS USERNAME ✅
│
└─ state = TtyState::Shell
   └─ spawn_tab(tabs, tab_count, ...)  ← USERNAME NOT PASSED ❌
      └─ Shell spawned without user context
         └─ sunshell hardcoded loads "root"
            └─ whoami shows "root" ❌
```

### Code Locations

**Where username IS available:**
```rust
// Line ~140 in services/tty_server/src/main.rs
LoginResult::Success { username, username_len, uid, gid } => {
    debug_log_login_success(&username[..username_len], uid, gid);
    // ⚠️ username is available here!
    state = TtyState::Shell;
    // ⚠️ but NOT passed to spawn_tab!
    if spawn_tab(tabs, tab_count, active_tab, next_shell_id, cap) { ... }
}
```

**Where username is NOT available:**
```rust
// Line ~341 in services/tty_server/src/main.rs
fn spawn_tab(
    tabs: &mut [ShellTab; MAX_TABS],
    tab_count: &mut usize,
    active_tab: &mut usize,
    next_shell_id: &mut u64,
    spawn_cap: CapabilityToken,
) -> bool {
    // ⚠️ No username parameter!
    let shell_id = *next_shell_id;
    let spawn_msg = IpcMsg::with_label(SpawnMsg::SPAWN)
        .word(0, pw0)  // Only shell path, no user info
        .word(1, pw1)
        .word(2, pw2)
        .word(3, pw3);
    // Shell spawned without user context
}
```

**Where hardcoding happens:**
```rust
// Line ~950+ in sunshell/src/main.rs
pub extern "C" fn _start(shell_id: u64) -> ! {
    ...
    let mut shell = Shell::new();
    shell.load_user_from_vfs(b"root");  // ⚠️ HARDCODED! Should be login user
    ...
}
```

## The Fix (3 Options)

### Option A: Pass username in shell_id encoding

Encode the UID in the shell_id:
- UID 0 (root) → shell_id starts at 0
- UID 1000 (user) → shell_id starts at 1000
- UID 2000 (other) → shell_id starts at 2000

**Pros:** Simple, no protocol change  
**Cons:** Limits to one shell per user

### Option B: Pass username through spawn message

Extend spawn message to include username:
```rust
let spawn_msg = IpcMsg::with_label(SpawnMsg::SPAWN)
    .word(0, pw0)         // shell path
    .word(1, pw1)
    .word(2, pw2)
    .word(3, pw3)
    .word(4, uid)         // NEW: add uid
    .word(5, gid);        // NEW: add gid
```

**Pros:** Clean, extensible  
**Cons:** Modifies spawn protocol (check if others depend on it)

### Option C: Use nameserver to pass context

Store login context in nameserver:
```rust
// After successful login
nameserver_grant("shell_context", shell_context_cap);

// In shell _start
let context_cap = nameserver_lookup("shell_context")?;
let user_info = ipc_call(context_cap, get_user_msg);
shell.load_user_from_vfs(user_info);
```

**Pros:** Robust, extensible  
**Cons:** More complex, more overhead

## Recommended Fix: Option B

Modify the spawn message to include UID/GID:

### Changes Required

1. **services/tty_server/src/main.rs**
   - Modify spawn_tab signature to accept `username: &[u8]`, `uid: u32`, `gid: u32`
   - Pass username/UID in spawn message words[4] and words[5]
   - Update all spawn_tab calls to pass user info

2. **sunshell/src/main.rs**
   - Modify _start to extract UID from words parameter (or use new protocol)
   - If UID is 1000, load "user" instead of "root"
   - Or better: load user by UID using VFS GETPWUID

3. **ipc/src/lib.rs** (maybe)
   - Verify SpawnMsg protocol documentation
   - Check if other code depends on specific message format

## Testing

After fix:
```bash
# Build
cargo build --workspace

# Run in QEMU
./tools/run.sh --build

# Test
Login: user
Password: user
$ whoami
user              # ✅ Should show "user" now
$ id
uid=1000(user) gid=1000(user)  # ✅ Should show user info
```

## Why This Matters

- User context is needed for permission enforcement
- File ownership tracking
- Group membership verification
- Future: setuid/setgid execution
- Multi-user session support

## Quick Fix (Temporary)

If Option B takes too long, quick workaround:
```rust
// In sunshell/src/main.rs _start()
shell.load_user_from_vfs(b"user");  // Change hardcoded "root" to "user"
```

This works for testing but doesn't support multiple users.

## Status

- [x] Root cause identified
- [x] Fix options documented
- [ ] Option B implementation
- [ ] Testing
- [ ] Verification
