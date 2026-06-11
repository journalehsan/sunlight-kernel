# Hardcoded User Bug Fix — Complete Summary

**Status:** ✅ FIXED AND READY FOR TESTING  
**Commits:** 2 new commits (4706194, 935eef2)  
**Build:** ✅ All packages compile successfully  
**Regressions:** ✅ None detected

---

## The Problem

When a user logged into SunlightOS, the shell would always show:
```bash
$ whoami
root          ← WRONG (should be "user" if that's who logged in)

$ id
uid=0(root) gid=0(root)  ← WRONG (should show logged-in user's uid)
```

This happened despite the TTY server successfully authenticating the user and getting their uid/gid from /etc/passwd.

---

## Root Cause Analysis

The uid/gid information **existed** but wasn't **propagated**:

1. ✅ TTY server received login credentials
2. ✅ TTY looked up user in /etc/passwd (got uid=1000, gid=1000 for "user")
3. ❌ TTY called spawn_tab() **without passing uid/gid**
4. ❌ Kernel's spawn handler **extracted uid/gid but ignored them**
5. ❌ Shell received only shell_id, defaulted to loading "root"

**Result:** Lost user context between login and shell creation.

---

## The Fix

Implemented complete uid/gid propagation chain:

```
Login successful (uid=1000)
     ↓
spawn_tab(uid=1000, gid=1000)
     ↓ [in spawn message words[4:5]]
kernel spawn_from_path(uid=1000, gid=1000)
     ↓ [in register arguments rsi=uid, rdx=gid]
shell _start(shell_id, uid, gid)
     ↓
Map uid → username ("user")
     ↓
Load user from /etc/passwd
     ↓
✅ whoami → "user"
✅ id → uid=1000(user) gid=1000(user)
```

### Files Modified

**1. services/tty_server/src/main.rs**
- `spawn_tab()` now accepts `uid: u32, gid: u32` parameters
- Updated 2 call sites to pass login uid/gid

**2. kernel/src/arch/x86_64/syscall.rs**
- `handle_spawn_call()` now uses uid/gid from message (was ignored)
- Passes them to `spawn_from_path()`

**3. kernel/src/process/spawn.rs**
- `spawn_from_path()` accepts and passes uid/gid to `set_initial_args()`
- Utilizes x86-64 register calling convention: rsi=uid, rdx=gid

**4. sunshell/src/main.rs**
- `_start()` now accepts uid/gid as parameters
- Maps uid → username: 0→"root", 1000→"user"
- Loads actual user from /etc/passwd instead of hardcoded "root"

---

## Testing

### Quick Manual Test

```bash
./tools/run.sh --build

# At login prompt:
Username: user
Password: user

# In shell:
$ whoami
user

$ id
uid=1000(user) gid=1000(user) groups=0(root)
```

**Expected:** Both commands show "user" / uid=1000  
**Before fix:** Both commands showed "root" / uid=0

### Comprehensive Test (see TEST_USER_FIX.md)
- Multiple user logins
- Root login verification
- Logout/login cycles
- Multiple shell tabs

---

## Architecture Notes

### What This Enables

✅ **Proper user context** in spawned shells  
✅ **Foundation for permissions** enforcement  
✅ **Multi-user support** (previously all shells were root)  
✅ **Security model** integrity (users can be enforced to specific permissions)

### What's Still Needed

⏳ **File ownership enforcement** (files owned by specific uids)  
⏳ **Permission checks** on file operations  
⏳ **setuid/setgid** execution bit support  
⏳ **Group membership** tracking  

### Design Decisions

- **Simple uid→username mapping**: Hardcoded 0→"root", 1000→"user"
  - Good for: Fast, no extra IPC, simple to understand
  - Better approach: Lookup by uid in /etc/passwd (future improvement)

- **Used x86-64 registers**: rsi=uid, rdx=gid
  - Good for: Minimal IPC changes, architectural standard
  - Alternative: Could use environment variables (future option)

---

## Git History

```
4706194 fix: pass uid/gid through spawn chain to fix hardcoded user bug
935eef2 docs: add testing guide for user uid/gid fix
```

---

## Verification Checklist

- [x] Code compiles without errors
- [x] All spawn_tab() calls updated
- [x] Kernel spawn handler extracts uid/gid
- [x] spawn_from_path passes uid/gid to registers
- [x] Shell _start accepts and uses uid/gid
- [x] No regressions in existing tests
- [ ] Manual testing in QEMU (pending)
- [ ] Multiple user logins verified (pending)
- [ ] File ownership checks next step (future)

---

## Known Limitations

1. **Hardcoded uid→username mapping**
   - Only 2 users: root (0) and user (1000)
   - Better: Dynamic lookup from /etc/passwd by uid

2. **No environment variables**
   - Could pass USER, UID, GID as env vars (future)
   - Currently just passed as register values

3. **No session tracking**
   - Shell doesn't track which session spawned it
   - Could be enhanced with session id tracking

---

## Impact Assessment

**Code quality:** ✅ Minimal, focused changes  
**Complexity:** ✅ Low (4 modified functions)  
**Risk:** ✅ Very low (read-only, no data mutations)  
**Reversibility:** ✅ Easy (single git revert)  
**Performance:** ✅ No impact  
**Security:** ✅ Improves security (proper user context)

---

## What's Next

1. **Immediate:** Manual testing in QEMU
2. **Short term:** File ownership enforcement
3. **Medium term:** Permission checks on file operations
4. **Long term:** Full POSIX user/group system

---

## References

- [[SESSION_STATE_20260611.md]] - Full session context
- [[TTY_USER_BUG_ANALYSIS.md]] - Original bug analysis
- [[TEST_USER_FIX.md]] - Testing guide
- Commit 4706194 - Implementation
