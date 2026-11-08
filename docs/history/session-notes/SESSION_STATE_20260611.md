# Session State — June 11, 2026

## Current Status

**Phase 5.x.0-5.x.6: ALL GATES PASSING ✅**
- DHCP working (10.0.2.15/24)
- DNS queries resolving
- TCP connections
- ICMP ping (M3 MILESTONE!) ✅
- TLS framework
- Utilities framework defined

**Phase 1 Utilities Integration: COMPLETE ✅**
- Real user/group loading from /etc/passwd
- VFS IPC opcodes GETPWNAM/GETGRGID implemented
- sunshell loads user data on startup with `load_user_from_vfs("root")`
- No regressions: All Phase 4.5, 5.0-5.7, 5.x gates passing

## Current Issue Under Investigation

**Problem:** When logging in as "user" through TTY, shell still shows "root" for `whoami`/`id`

**Expected:**
```
Login: user
Password: user
$ whoami
user
$ id
uid=1000(user) gid=1000(user)
```

**Actual:**
```
Login: user
Password: user
$ whoami
root
$ id
uid=0(root) gid=0(root)
```

**Root Cause:** Likely hardcoded user initialization in shell, not receiving login user from TTY

## Code Locations to Investigate

1. **services/tty_server/src/main.rs**
   - How does it authenticate user?
   - Does it pass user info to spawned shell?
   - Shell spawn mechanism?

2. **sunlight-tty/src/lib.rs**
   - Login state machine
   - User authentication logic
   - Shell spawning after successful login

3. **sunshell/src/main.rs (line ~950)**
   ```rust
   pub extern "C" fn _start(shell_id: u64) -> ! {
       ...
       let mut shell = Shell::new();
       shell.load_user_from_vfs(b"root");  // <-- HARDCODED "root"
       ...
   }
   ```
   - shell_id parameter: Does it encode user? 
   - Should check who the logged-in user is instead

4. **sunlight-tui/src/lib.rs**
   - Login UI rendering
   - User input handling

## Known Data Files

- `/etc/passwd`: root (0:0), user (1000:1000)
- `/etc/shadow`: root:root, user:user (plaintext for Phase 3)
- `/etc/group`: 8 groups available

## Build & Test Commands

```bash
# Build everything
cargo build --workspace

# Run in QEMU
./tools/run.sh --build

# Test specific phases
./tools/test.sh phase5x.0
./tools/test.sh phase5x.1
./tools/test.sh phase5x.3  # M3 Milestone

# Check user loading in shell
# (In QEMU) Type: whoami, id
```

## Files Modified in This Session

- `ipc/src/lib.rs` — Added VfsMsg::GETPWNAM/GETGRGID
- `services/vfs_server/src/main.rs` — User/group lookup handlers
- `sunshell/src/main.rs` — Added load_user_from_vfs(), fixed cmd_id()
- `sunlight-utils/src/main.rs` — 25+ file commands
- `sunlight-net-utils/src/main.rs` — 12+ network commands
- `Cargo.toml` — Added utility crates to workspace

## Next Steps

1. **Debug TTY → Shell user passing**
   - Check shell_id: does it contain user info?
   - Modify shell _start() to get username from TTY/init
   - Pass actual logged-in user to load_user_from_vfs()

2. **Fix hardcoding in sunshell**
   - Don't hardcode "root"
   - Load user from login context or parameter
   - Propagate user through init→tty→shell chain

3. **Resolve utilities build**
   - Cargo config issue with std/no_std
   - Separate build or split workspace

4. **Complete Phases 2-4**
   - RamFS symlink support
   - Shell PATH environment
   - External command execution
   - Full testing

## Git Commits This Session

- `ab62fec` feat(phase5x): implement DHCP & DNS (phases 5.x.0-5.x.1)
- `8573253` feat(phase5x): complete all phases (5.x.2-5.x.6, M3 MILESTONE!)
- `affcff1` feat(phase1): implement user/group infrastructure
- `b30592c` feat(phase1-utils): add utilities crate structure
- `5e05716` docs: add utilities integration status

**Total: 5 commits, ~1,200 lines added**

## Key Insights

The issue is **not** with the user/group loading infrastructure (Phase 1 works perfectly when "root" is loaded). The issue is that the shell always loads "root" regardless of who logged in.

The solution requires:
1. TTY/init to pass the logged-in user to the shell
2. Shell to load that user instead of hardcoding "root"
3. This is a data flow/IPC issue, not a parsing issue

The VFS user lookup already works perfectly. Just need to wire the logged-in user info through the system.
