# SunlightOS — Utilities Integration Plan
## Loading sunlight-utils & sunlight-net-utils into Shell

---

## Current State

**Built but not integrated:**
- `sunlight-utils` v0.1 (ls, cat, cp, grep, find, etc.) — 20+ commands
- `sunlight-net-utils` v0.1 (ping, ifconfig, wget, curl, dig, etc.) — 8+ commands
- `sunshell` v0.2 (basic built-in commands only)
- `sunlight-tty` (login screen, mux)
- `services/vfs_server` (VFS with /etc/passwd, /etc/group)

**Issues to fix:**
1. Hardcoded `root` user in sunshell and TTY (ignores /etc/passwd)
2. No symlinks for command discovery
3. No PATH environment variable
4. No external command execution (`exec` family)
5. No process forking needed to run external commands

---

## Dependency Chain

```
1. User/Group Infrastructure (FIRST)
   └─ Load /etc/passwd properly (already exists from Phase 3)
   └─ Load /etc/group properly
   └─ Parse and return real user data

2. Binary Installation (DEPENDS ON: 1)
   └─ Copy sunlight-utils to RamFs
   └─ Copy sunlight-net-utils to RamFs
   └─ Create symlinks (/bin/ls → /usr/bin/sunlight-utils, etc.)

3. Shell Integration (DEPENDS ON: 2)
   └─ Set PATH environment variable
   └─ Implement command lookup in PATH
   └─ Implement fork() + exec() for external commands

4. Testing & Verification (DEPENDS ON: 3)
   └─ Verify users load from /etc/passwd
   └─ Run shell commands (ls, cat, ping, etc.)
   └─ Verify output and exit codes
```

---

## Phase 1: User/Group Infrastructure

### Goal
Properly load and parse /etc/passwd and /etc/group so users are not hardcoded.

### Implementation Steps

**1.1: Add user/group parsing to sunlight-fs**
- File: `sunlight-fs/src/users.rs` (NEW)
- Parse `/etc/passwd` format: `username:uid:gid:shell`
- Parse `/etc/group` format: `groupname:gid:members`
- Cache parsed users/groups on first read

**1.2: Update VFS to expose user lookup**
- File: `services/vfs_server/src/main.rs`
- Add IPC opcode for `GetUser(username)` → `UserInfo`
- Add IPC opcode for `GetGroup(groupname)` → `GroupInfo`

**1.3: Update sunshell to use real users**
- File: `sunshell/src/main.rs`
- Load user from VFS instead of hardcoding "root"
- Show actual username in prompt
- Return actual UID/GID for `id` command

**1.4: Update TTY login to use real users**
- File: `services/tty_server/src/main.rs`
- Load user list from VFS `/etc/passwd`
- Authenticate against actual users (plaintext OK for Phase 3)
- Return actual UID/GID after login

### Success Criteria
```
$ whoami
root

$ id
uid=0 gid=0 (from /etc/passwd, not hardcoded)

$ su user
password: user
$ whoami
user
```

---

## Phase 2: Binary Installation into RamFs

### Goal
Install sunlight-utils and sunlight-net-utils binaries and create command symlinks.

### Implementation Steps

**2.1: Add binary includes to kernel**
- File: `kernel/src/main.rs`
- Include compiled sunlight-utils binary
- Include compiled sunlight-net-utils binary

**2.2: Create RamFs entries for binaries**
- File: `kernel/src/main.rs` (RamFs init)
- Add `/usr/bin/sunlight-utils` → binary
- Add `/usr/bin/sunlight-net-utils` → binary

**2.3: Create symlinks for all commands**
- File: `sunlight-fs/src/ramfs.rs` (or new symlink support)
- Symlink `/bin/ls` → `/usr/bin/sunlight-utils`
- Symlink `/bin/cat` → `/usr/bin/sunlight-utils`
- Symlink `/bin/grep` → `/usr/bin/sunlight-utils`
- ... (all 28 commands)
- Symlink `/bin/ping` → `/usr/bin/sunlight-net-utils`
- Symlink `/bin/ifconfig` → `/usr/bin/sunlight-net-utils`
- ... (all network commands)

**2.4: Support symlinks in RamFs**
- File: `sunlight-fs/src/ramfs.rs`
- Add `RamEntry::Symlink(path, target)` variant
- Handle symlink traversal in `open()`
- Return actual file content when following symlinks

### Success Criteria
```
$ ls /bin
ls  cat  grep  find  ...  ping  ifconfig  wget  ...

$ ls -la /bin/ls
lrwxr-xr-x  1 root root  19 Jun 11 12:00 ls -> /usr/bin/sunlight-utils
```

---

## Phase 3: Shell Integration & Command Execution

### Goal
Make sunshell execute external commands from PATH.

### Implementation Steps

**3.1: Add PATH environment variable**
- File: `sunshell/src/main.rs`
- Set default PATH: `/bin:/usr/bin`
- Pass PATH to child processes

**3.2: Implement command lookup**
- File: `sunshell/src/builtins/mod.rs`
- New function: `find_command_in_path(cmd) → Option<String>`
- Search PATH directories via VFS
- Return full path to executable

**3.3: Implement external command execution**
- File: `sunshell/src/main.rs`
- Check if command is builtin (whoami, id, echo, etc.)
- If not builtin: call `find_command_in_path()`
- If found: fork() child process
- In child: load binary via ELF loader, exec() it
- In parent: wait for child exit, return exit code

**3.4: Update command parsing**
- File: `sunshell/src/parser.rs`
- Support argument passing: `ls -la /root`
- Support pipe: `cat file.txt | grep pattern`
- Support redirect: `ls > output.txt`

### Success Criteria
```
$ ls
bin  boot  etc  home  root  usr  var

$ cat /etc/motd
Welcome to SunlightOS

$ grep root /etc/passwd
root:0:0:/bin/sh

$ ping 8.8.8.8
64 bytes from 8.8.8.8: icmp_seq=0 time=20ms
```

---

## Phase 4: Testing & Verification

### Goal
Verify all components work together correctly.

### Test Cases

**4.1: User/Group Tests**
- [ ] Login as `root` with hardcoded password
- [ ] `whoami` returns "root"
- [ ] `id` shows uid=0 gid=0
- [ ] Create additional users in /etc/passwd and test login

**4.2: Binary Location Tests**
- [ ] `/bin/ls` exists and is executable
- [ ] All symlinks resolve correctly
- [ ] Running `/bin/ls` without PATH works

**4.3: Shell Command Tests**
- [ ] `ls /` lists directories
- [ ] `cat /etc/motd` displays welcome
- [ ] `grep pattern /etc/passwd` finds users
- [ ] `ping 8.8.8.8` sends ICMP requests
- [ ] `ifconfig` shows network interface
- [ ] `wget` downloads files
- [ ] `find / -name "*.txt"` finds text files

**4.4: Edge Cases**
- [ ] Command not found → error message
- [ ] Wrong argument count → error handling
- [ ] Non-existent file → ENOENT
- [ ] Permission denied → correct error
- [ ] Invalid user login → authentication failure

---

## Implementation Order

```
Logical Order (respecting dependencies):

1. USER/GROUP INFRASTRUCTURE
   1.1 Parse /etc/passwd in sunlight-fs
   1.2 Add user lookup IPC opcodes
   1.3 Fix sunshell to load real users
   1.4 Fix TTY login to use real users

2. BINARY INSTALLATION
   2.1 Include binary ELFs in kernel
   2.2 Create RamFs entries
   2.3 Add symlink support to RamFs
   2.4 Create all command symlinks

3. SHELL INTEGRATION
   3.1 Add PATH environment variable
   3.2 Implement PATH lookup
   3.3 Implement fork/exec for external commands
   3.4 Update command parser for arguments/pipes

4. TESTING
   4.1-4.4 Run test suite and verify all works
```

---

## Quick Summary

| Phase | What | Time Est. | Blocker |
|-------|------|-----------|---------|
| 1 | User/group loading | 30min | None |
| 2 | Binary installation + symlinks | 45min | Phase 1 ✓ |
| 3 | Shell PATH + fork/exec | 60min | Phase 2 ✓ |
| 4 | Testing + verification | 30min | Phase 3 ✓ |

**Total: ~2.5 hours for complete integration**

---

## Expected Final Experience

```bash
$ login
SunlightOS Login
Username: root
Password: ••••
Welcome to SunlightOS

root@sunlightos:~# whoami
root

root@sunlightos:~# id
uid=0(root) gid=0(root) groups=0(root)

root@sunlightos:~# ls /bin
cat  cp  find  grep  ls  mkdir  mv  ping  rm  wget  ...

root@sunlightos:~# cat /etc/motd
Welcome to SunlightOS

root@sunlightos:~# ping google.com
PING google.com (142.250.185.46) 56 bytes of data
64 bytes from 142.250.185.46: icmp_seq=0 time=20ms
...

root@sunlightos:~# find / -name "*.txt"
/etc/motd
/boot/HELLO.TXT
...
```

---

## Notes

- Phase 1 is critical: everything else depends on proper user loading
- Phase 2 needs RamFs symlink support: currently missing, must add
- Phase 3 requires fork() and exec(): already exist from Phase 4, just wire them
- Testing must verify both builtin and external commands work
