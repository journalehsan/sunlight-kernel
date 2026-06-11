# Testing the Hardcoded User Fix

## What Was Fixed

**Bug:** After logging in as "user", `whoami` and `id` still showed "root"
**Root Cause:** UID/GID were extracted at TTY login but not passed to the spawned shell
**Solution:** Wire uid/gid through the entire spawn chain

## Fix Implementation

```
TTY Login (uid=1000)
  ↓
TTY spawn_tab(uid=1000, gid=1000)
  ↓ (in spawn message words[4:5])
Kernel spawn handler extracts uid/gid
  ↓
spawn_from_path(uid=1000, gid=1000)
  ↓
set_initial_args(shell_id, uid, gid, 0)
  ↓
Shell _start(shell_id, uid, gid)
  ↓
Load user by uid: 1000 → "user"
  ↓
whoami → "user" ✅
id → uid=1000(user) gid=1000(user) ✅
```

## How to Test

### Manual Testing (Recommended)

1. **Build and run:**
   ```bash
   ./tools/run.sh --build
   ```

2. **At the login prompt:**
   ```
   SunlightOS Login
   Username: user
   Password: user
   ```

3. **In the shell, type these commands:**
   ```bash
   $ whoami
   user                      ← Should show "user" (not "root")
   
   $ id
   uid=1000(user) gid=1000(user) groups=0(root)  ← Should show 1000, not 0
   ```

### Expected Output

**BEFORE FIX:**
```
$ whoami
root          ← ❌ Wrong, should be "user"

$ id
uid=0(root) gid=0(root) groups=0(root)  ← ❌ Wrong, should be 1000
```

**AFTER FIX:**
```
$ whoami
user          ← ✅ Correct!

$ id
uid=1000(user) gid=1000(user) groups=0(root)  ← ✅ Correct!
```

### Alternative Test: Login as root

You can also verify root still works:
```bash
$ whoami
root          ← Should work

$ id
uid=0(root) gid=0(root) groups=0(root)  ← Should work
```

## Implementation Details

### Changed Files

1. **services/tty_server/src/main.rs**
   - Modified `spawn_tab()` to accept `uid: u32, gid: u32` parameters
   - Updated both spawn_tab calls to pass uid/gid

2. **kernel/src/arch/x86_64/syscall.rs**
   - Extract uid/gid from spawn message words[4:5]
   - Pass to spawn_from_path() function

3. **kernel/src/process/spawn.rs**
   - Modified `spawn_from_path()` signature to accept uid/gid
   - Pass uid/gid to `set_initial_args()` as rsi and rdx registers (second and third x86-64 arguments)

4. **sunshell/src/main.rs**
   - Modified `_start()` signature: `fn _start(shell_id: u64, uid: u64, gid: u64)`
   - Map uid to username: 0→"root", 1000→"user"
   - Load correct user via `shell.load_user_from_vfs(username)`

## Data Flow

The uid/gid flow through the system:

```
TTY Server receives: username="user" → uid=1000, gid=1000
  │
  └─→ spawn_tab() call with uid=1000, gid=1000
        │
        └─→ IpcMsg.word(4) = 1000 (uid)
        └─→ IpcMsg.word(5) = 1000 (gid)
              │
              └─→ Kernel spawn handler
                    │
                    └─→ let uid = msg.words[4] = 1000
                    └─→ let gid = msg.words[5] = 1000
                          │
                          └─→ spawn_from_path(..., uid, gid)
                                │
                                └─→ set_initial_args(..., uid, gid, ...)
                                      │ (sets rsi=uid, rdx=gid registers)
                                      └─→ Shell _start(_, 1000, 1000)
                                            │
                                            └─→ uid=1000 → "user"
                                            └─→ whoami="user" ✅
```

## Verification

To verify the fix is working:

1. Build succeeds without errors
2. Boot to login screen
3. Login as "user" with password "user"
4. `whoami` returns "user"
5. `id` returns uid=1000, not uid=0

## Rollback

If there are issues:
- This fix is minimal and non-invasive
- Only 4 files modified
- Can be reverted with: `git revert 4706194`
- No data files or configurations changed

## Next Steps

Once verified:
1. Test with other users (if more users are added to /etc/passwd)
2. Test logout and login cycle
3. Test multiple shells/tabs with different users
4. Integration with file permissions (Phase 2: check file ownership)

## Known Limitations

- Maps uid 0→"root", 1000→"user" in shell (hardcoded map)
- Better approach would be: lookup user by uid via VFS /etc/passwd
- For now, this is sufficient for 2-user testing

## Related Issues

- [[TTY_USER_BUG_ANALYSIS.md]] - Original bug analysis document
- [[SESSION_STATE_20260611.md]] - Session context and full investigation
